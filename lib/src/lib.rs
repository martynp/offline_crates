use std::collections::HashMap;
use std::io::{Error, Result};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use futures::TryStreamExt;
use glob::{glob, Paths};
use progress_bar::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

pub mod structures;

use crate::structures::*;

pub async fn process_crate_definition(glob: Paths, expected: usize) -> Vec<CrateData> {
    let mut task_channels = Vec::new();
    let mut join_handles = Vec::new();

    let (collect_tx, mut collect_rx) = tokio::sync::mpsc::channel::<CrateData>(100);

    let collector = tokio::task::spawn(async move {
        let mut crates = Vec::new();
        loop {
            if let Some(c) = collect_rx.recv().await {
                crates.push(c);
            } else {
                return crates;
            }
        }
    });

    for _ in 0..4 {
        let collect_tx = collect_tx.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<PathBuf>(100);
        task_channels.push(tx);
        join_handles.push(Some(tokio::task::spawn(async move {
            while let Some(file) = rx.recv().await {
                let content = std::fs::read_to_string(&file).unwrap_or("".into());
                for line in content.lines() {
                    if let Ok(version) = serde_json::from_str::<CrateData>(line) {
                        if !version.yanked {
                            collect_tx
                                .send(version)
                                .await
                                .expect("Unable to send command message to worker thread");
                        }
                    }
                }
            }
        })));
    }

    tokio::task::spawn(async move {
        init_progress_bar_with_eta(expected);
        set_progress_bar_action("Processing", Color::Blue, Style::Bold);

        let mut count = 0;
        let mut channel_index = 0;
        for path in glob {
            task_channels[channel_index]
                .send(path.unwrap())
                .await
                .unwrap();
            channel_index += 1;
            if channel_index >= task_channels.len() {
                channel_index = 0;
            }
            count += 1;
            if count % 1000 == 0 {
                set_progress_bar_progress(count);
            }
        }

        set_progress_bar_progress(expected);
        finalize_progress_bar();

        drop(task_channels);
    })
    .await
    .expect("Failed to joing crate definition tasker thread");

    for handle in &mut join_handles {
        if let Some(handle) = handle.take() {
            handle
                .await
                .expect("Failed to join crate processing thread");
        }
    }
    drop(collect_tx);

    collector
        .await
        .expect("Failed to join crate definiton collecting thread")
}

/// Download the crates we know about.
///
/// This job is split over multiple tasks. A large number of downloader tasks is started with
/// an MPSC channel set up to each one.
///
/// The MPSC channel can hold up to 100 crates to download, the downloader tasks run through
/// those tasked downloads until all the downloads are complete.
///
/// A separate task is continuous refilling those channels with new files to download. This
/// separate task also handles updating the terminal progress bar.
pub async fn download_crates(
    git_repository: &Path,
    location: &Path,
    limit: i32,
    search_path: &Vec<String>,
    move_matches: bool,
    crates: Vec<CrateData>,
) -> Result<()> {
    let number_of_crates = crates.len();

    // Work out the download server to use for the downloads
    let download = if let Ok(content) = std::fs::read_to_string(git_repository.join("config.json"))
    {
        let config: RepoConfig = serde_json::from_str(&content).unwrap();
        config.dl
    } else {
        log::error!("Was unable to open config.json in git repository...");
        return Err(Error::last_os_error());
    };
    log::info!("Using {}", &download);

    let mut task_channels = Vec::new();
    let mut join_handles = Vec::new();

    for _ in 0..40 {
        let location = location.to_path_buf();
        let download = download.to_owned();
        let search_paths = search_path.to_owned();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CrateData>(100);
        task_channels.push(tx);
        join_handles.push(Some(tokio::task::spawn(async move {
            while let Some(data) = rx.recv().await {
                let download_url = format!("{}/{}/{}/download", &download, data.name, data.vers);
                let file_path = location.join(path_to_crate(&data));

                if file_path.exists()
                    && sha256_compare(&file_path, &data.cksum)
                        .await
                        .expect("Failed opening file for sha256 comparison")
                {
                    continue;
                } else if let Some(path) = search(&search_paths, &data).await {
                    tokio::fs::create_dir_all(
                        file_path.parent().expect("File did not have parent"),
                    )
                    .await
                    .unwrap();
                    if move_matches {
                        tokio::fs::rename(path, file_path).await.unwrap();
                    } else {
                        tokio::fs::copy(path, file_path).await.unwrap();
                    }
                    continue;
                } else {
                    tokio::fs::create_dir_all(
                        file_path.parent().expect("File did not have parent"),
                    )
                    .await
                    .expect("Unable to create download directory for crate");
                    let response = reqwest::get(download_url).await.unwrap();

                    let mut dest = tokio::fs::File::create(&file_path).await.unwrap();
                    let mut stream = response.bytes_stream();

                    while let Ok(chunk) = stream.try_next().await {
                        if let Some(chunk) = chunk {
                            dest.write_all(&chunk).await.unwrap();
                        } else {
                            break;
                        }
                    }
                }
            }
        })));
    }

    tokio::task::spawn(async move {
        init_progress_bar_with_eta(number_of_crates);
        set_progress_bar_action("Downloading", Color::Blue, Style::Bold);

        let mut count = 0;
        let mut ecount = 0;
        let mut channel_index = 0;
        for c in crates {
            'send_command: loop {
                let result = task_channels[channel_index]
                    .send_timeout(c.to_owned(), std::time::Duration::from_millis(100))
                    .await;

                // Alway increment to the next channel
                channel_index += 1;
                if channel_index >= task_channels.len() {
                    channel_index = 0;
                }

                // If it is an error we stay in the loop to retry this crate on a different downloader
                if result.is_err() {
                    ecount += 1;
                    if ecount >= task_channels.len() {
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                        ecount = 0;
                    }
                    continue 'send_command;
                } else {
                    ecount = 0;
                }

                break 'send_command;
            }
            count += 1;
            set_progress_bar_progress(count);

            if limit > 0 && count > limit as usize {
                log::warn!("Exiting early due to download limit");
                break;
            }
        }

        set_progress_bar_progress(number_of_crates);
        finalize_progress_bar();

        drop(task_channels);
    })
    .await
    .expect("Failed to join download monitoring thread");

    for handle in &mut join_handles {
        if let Some(handle) = handle.take() {
            handle.await.expect("Failed to join download thread");
        }
    }

    Ok(())
}

pub async fn process_existing_crates_list(
    existing: &Option<PathBuf>,
    crates: Vec<CrateData>,
) -> Vec<CrateData> {
    let existing: HashMap<String, String> = if let Some(path) = existing {
        log::info!("Processing existing crates list");
        if let Ok(content) = tokio::fs::read_to_string(path).await {
            content
                .lines()
                .map(|l| {
                    let (checksum, path) = l.split_at(64);
                    let checksum = checksum.to_string();
                    let filename = PathBuf::from_str(path.trim_start())
                        .unwrap()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
                    (checksum, filename)
                })
                .collect()
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    if !existing.is_empty() {
        log::info!("Checking for existing crates");
        let to_process = crates.len();
        init_progress_bar_with_eta(to_process);
        set_progress_bar_action("Checking", Color::Blue, Style::Bold);
        let mut count = 0;
        let new_crates: Vec<CrateData> = crates
            .into_iter()
            .filter(|c| {
                count += 1;
                if count % 1000 == 0 {
                    set_progress_bar_progress(count);
                }
                if let Some(found) = existing.get(&c.cksum) {
                    if format!("{}-{}.crate", c.name, c.vers) != *found {
                        log::warn!("Name mismatch {} != {}", c.name, found);
                    }
                    return false;
                }
                true
            })
            .collect();
        set_progress_bar_progress(to_process);
        finalize_progress_bar();
        let removed = to_process - new_crates.len();
        if removed > 0 {
            log::info!("Removed {removed} existing crates");
        }
        new_crates
    } else {
        crates
    }
}

/// From a given search path and the loaded crate data, copy any missing crates to the
/// specified store location
///
pub async fn copy_missing_crates(
    search_paths: &[String],
    store_location: &Path,
    crates: &[CrateData],
) -> Result<usize> {
    let mut count = 0;
    let mut copied = 0;
    let to_process = crates.len();

    let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let semaphore = Arc::new(Semaphore::new(10));

    init_progress_bar_with_eta(to_process);
    set_progress_bar_action("Checking", Color::Blue, Style::Bold);
    for data in crates {
        count += 1;
        if count % 1000 == 0 {
            set_progress_bar_progress(count);
        }
        let relative_file_path = path_to_crate(data);
        let path_on_disk = store_location.join(relative_file_path);

        if !path_on_disk.exists() {
            //    if let Some(path) = search(search_paths, data) {
            //        std::fs::create_dir_all(file_path.parent().expect("File did not have parent"))?;
            //        std::fs::copy(path, store_location.join(file_path))?;
            //        copied += 1;
            //    }

            // Spin until there is a semaphore to grab
            while semaphore.available_permits() == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            let semaphore = semaphore.clone();
            let search_paths = search_paths.to_vec();
            let data = data.clone();
            let handle = tokio::task::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                if let Some(path) = search(&search_paths, &data).await {
                    tokio::fs::create_dir_all(
                        path_on_disk.parent().expect("File did not have parent"),
                    )
                    .await
                    .expect("Create dir failed");
                    tokio::fs::copy(path, path_on_disk)
                        .await
                        .expect("Copy failed");
                }
            });
            handles.push(handle);
            copied += 1;
        }
    }
    set_progress_bar_progress(to_process);
    finalize_progress_bar();

    for handle in handles {
        let _ = handle.await;
    }

    semaphore.close();

    Ok(copied)
}

pub fn path_to_crate(data: &CrateData) -> PathBuf {
    match data.name.len() {
        1 => PathBuf::from_str(&format!(
            "./1/{}/{}-{}.crate",
            data.name, data.name, data.vers
        ))
        .expect("Failed to create path from single digit name"),
        2 => PathBuf::from_str(&format!(
            "./2/{}/{}-{}.crate",
            data.name, data.name, data.vers
        ))
        .expect("Failed to create path from double digit name"),
        3 => {
            let first = &data.name[0..2];
            PathBuf::from_str(&format!(
                "./3/{}/{}/{}-{}.crate",
                first, data.name, data.name, data.vers
            ))
            .expect("Failed to create path from tripple digit name")
        }
        _ => {
            let first = &data.name[0..2];
            let second = &data.name[2..4];
            PathBuf::from_str(&format!(
                "./{}/{}/{}/{}-{}.crate",
                first, second, data.name, data.name, data.vers
            ))
            .expect("Failed to create path from name")
        }
    }
}

async fn sha256_compare(file_path: &PathBuf, checksum: &str) -> Result<bool> {
    use sha2::Digest;
    let file = tokio::fs::File::open(file_path).await?;
    let mut hasher = sha2::Sha256::new();

    // Use tokio's async copy to read the file and update the hasher
    let mut file = tokio::io::BufReader::new(file);
    let mut buffer = vec![0; 8192]; // 8KB buffer
    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break; // EOF
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    Ok(hash[..] == hex::decode(checksum).expect("sha256 checksum incorrect"))
}

async fn search(search_path: &Vec<String>, data: &CrateData) -> Option<PathBuf> {
    for path in search_path {
        let pattern = format!("{}/**/{}-{}.crate", path, data.name, data.vers);
        if let Ok(potential_matches) = glob(&pattern) {
            for potential_match in potential_matches.flatten() {
                if sha256_compare(&potential_match, &data.cksum)
                    .await
                    .expect("sha256 compare failed")
                {
                    return Some(potential_match);
                }
            }
        }
    }

    None
}

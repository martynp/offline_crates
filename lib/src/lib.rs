use std::collections::HashMap;
use std::io::{Error, Result};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use futures::TryStreamExt;
use glob::{glob, Paths};
use progress_bar::*;
use tokio::io::AsyncWriteExt;

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
                for version in std::fs::read_to_string(&file)
                    .unwrap_or("".into())
                    .lines()
                    .map(|line| serde_json::from_str::<CrateData>(line).ok())
                {
                    if let Some(version) = version {
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

pub async fn download_crates(
    git_repository: &Path,
    location: &Path,
    limit: i32,
    search_path: &Vec<String>,
    crates: Vec<CrateData>,
) -> Result<()> {
    let number_of_crates = crates.len();

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

    for _ in 0..4 {
        let location = location.to_path_buf();
        let download = download.to_owned();
        let search_paths = search_path.to_owned();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CrateData>(100);
        task_channels.push(tx);
        join_handles.push(Some(tokio::task::spawn(async move {
            let mut downloaded = 0;
            while let Some(data) = rx.recv().await {
                let download_url = format!("{}/{}/{}/download", &download, data.name, data.vers);
                let file_path = location.join(path_to_crate(&data));

                if file_path.exists()
                    && sha256_compare(&file_path, &data.cksum)
                        .expect("Failed opening file for sha256 comparison")
                {
                    continue;
                } else if let Some(path) = search(&search_paths, &data) {
                    tokio::fs::copy(path, file_path).await.unwrap();
                    continue;
                } else {
                    std::fs::create_dir_all(file_path.parent().expect("File did not have parent"))
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

                downloaded += 1;
                if limit > 0 && downloaded > limit {
                    break;
                }
            }
        })));
    }

    tokio::task::spawn(async move {
        init_progress_bar_with_eta(number_of_crates);
        set_progress_bar_action("Downloading", Color::Blue, Style::Bold);

        let mut count = 0;
        let mut channel_index = 0;
        for c in crates {
            if task_channels[channel_index]
                .send(c.to_owned())
                .await
                .is_err()
            {
                break;
            }
            channel_index += 1;
            if channel_index >= task_channels.len() {
                channel_index = 0;
            }
            count += 1;
            set_progress_bar_progress(count);
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
                        log::warn!("Name missmatch {} != {}", c.name, found);
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
            log::info!("Removed {} existing crates", removed);
        }
        new_crates
    } else {
        crates
    }
}

pub fn path_to_crate(data: &CrateData) -> PathBuf {
    match data.name.len() {
        1 => PathBuf::from_str(&format!("./1/{}-{}.crate", data.name, data.vers))
            .expect("Failed to create path from single digit name"),
        2 => PathBuf::from_str(&format!("./2/{}-{}.crate", data.name, data.vers))
            .expect("Failed to create path from double digit name"),
        3 => {
            let first = &data.name[0..2];
            PathBuf::from_str(&format!("./3/{}/{}-{}.crate", first, data.name, data.vers))
                .expect("Failed to create path from tripple digit name")
        }
        _ => {
            let first = &data.name[0..2];
            let second = &data.name[2..4];
            PathBuf::from_str(&format!(
                "./{}/{}/{}-{}.crate",
                first, second, data.name, data.vers
            ))
            .expect("Failed to create path from name")
        }
    }
}

fn sha256_compare(file_path: &PathBuf, checksum: &str) -> Result<bool> {
    use sha2::Digest;
    let mut file = std::fs::File::open(file_path)?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let hash = hasher.finalize();
    Ok(hash[..] == hex::decode(checksum).expect("sha256 checksum incorrect"))
}

fn search(search_path: &Vec<String>, data: &CrateData) -> Option<PathBuf> {
    for path in search_path {
        let pattern = format!("{}/**/{}-{}.crate", path, data.name, data.vers);
        if let Ok(mut potential_matchs) = glob(&pattern) {
            return potential_matchs
                .find(|c| {
                    sha256_compare(c.as_ref().expect("Unexpected glob error..."), &data.cksum)
                        .expect("sha256 compare failed")
                })
                .map(|c| c.expect("Unexpected glob error..."));
        }
    }

    None
}

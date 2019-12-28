use rustc_serialize::json::Json;

use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use progress_bar::color::{Color, Style};
use progress_bar::progress_bar::ProgressBar;

use sha2::{Digest, Sha256};

use serde::{Deserialize, Serialize};

extern crate reqwest;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Create {
    pub name: String,
    pub version: String,
    pub checksum: String,
    pub file_path: String,
    pub relative_path: String,
}

/// Extract file names from crates-io.index repository
///
/// Arguments
///
/// * `dir` - Path to the root of the gir repository
/// * `files` - Vector to store found files
///
pub fn walk_repo(dir: &Path, base_dir: &str, files: &mut Vec<String>) -> std::io::Result<()> {
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if path.ends_with(".git") == false {
                    walk_repo(&path, base_dir, files)?;
                }
            } else {
                if path.ends_with("config.json") == false {
                    let path_str = path.strip_prefix(base_dir).unwrap().to_str().unwrap();
                    files.push(String::from(path_str));
                }
            }
        }
    }
    Ok(())
}

/// Verify the store against the index by looking at checksums
///
/// Arguments
///
/// * `creates` - Vector of creates to verify
/// * `threads` - Number of threads to use for the sha256 verification
///
/// Returns
///
/// * Vector<Crate> - Vector of crates that need to be downloaded
///
pub fn verify_store(
    creates: &mut Vec<Create>,
    threads: usize,
) -> std::io::Result<Vec<Create>> {
    let mut progress_bar = ProgressBar::new(creates.len());
    progress_bar.set_action("Verifying", Color::Blue, Style::Bold);

    // Handles for the verifier threads
    let mut handles = Vec::new();

    // Communications from the main thread to the verifier thread
    let mut to_thread = Vec::new();

    // Communication from the verifier thread to the main thread
    let (sender, receiver) = mpsc::channel();

    let (to_missing_collator, missing_collator_rx) = mpsc::channel();
    let missing_creates = thread::spawn(move || {
        let mut progress_bar = ProgressBar::new(0);
        let mut missing = Vec::new();
        loop {
            let msg: (&str, Create) = missing_collator_rx.recv().unwrap();
            if msg.0 == "missing" {
                missing.push(msg.1);
            } else if msg.0 == "invalid" {
                progress_bar.print_info(
                    "Failure",
                    &format!(
                        "{}-{} - Checksum incorrect, downloading crate again",
                        msg.1.name, msg.1.version
                    ),
                    Color::Red,
                    Style::Bold,
                );
                missing.push(msg.1);
            } else if msg.0 == "exit" {
                break;
            }
        }
        return missing;
    });

    // Create all the threads
    for i in 0..threads {
        // Generate a MPSC for this thread
        let (msg, thread_rx) = mpsc::channel();

        // Store the object to communicate with the thread
        to_thread.push(msg);

        // Clone a sender for this thread
        let sender_n = sender.clone();
        let to_missing_collator_n = to_missing_collator.clone();

        // Create the thread and push the handler to the vector store
        handles.push(thread::spawn(move || {
            // Wait for all the threads to be created before they report in to the main thread for
            // tasking
            thread::sleep(Duration::from_millis(100));

            // Tell the main thread we are waiting for tasking
            sender_n.send(i).unwrap();

            loop {
                // Block while waiting for tasking
                let msg: (&str, Create) = thread_rx.recv().unwrap();
                if msg.0 == "exit" {
                    break;
                } else {
                    if sha256_compare_file(&msg.1.file_path, &msg.1.checksum).unwrap() == false {
                        to_missing_collator_n.send(("invalid", msg.1.clone())).unwrap();
                    }
                    sender_n.send(i).unwrap();
                }
            }
        }));
    }

    for create in creates {
        let path_to_create = Path::new(&create.file_path);
        if path_to_create.is_file() {
            // Wait for a thread to become free
            let msg = receiver.recv().unwrap();

            // Message the selected thread with the required information
            to_thread[msg].send(("data", create.clone())).unwrap();
        } else {
            to_missing_collator.send(("missing", create.clone())).unwrap();
        }
        progress_bar.inc();
    }

    // Signal all the threads to stop
    for th in to_thread {
        th.send(("exit", Create::default())).unwrap();
    }

    // Join all verifiers
    for handle in handles {
        handle.join().unwrap();
    }

    // Signal and join the thread collating missing pakages
    to_missing_collator
        .send(("exit", Create::default()))
        .unwrap();
    let missing = missing_creates.join().unwrap();

    progress_bar.print_info("Verify", "Complete", Color::Green, Style::Bold);
    println!("");

    println!("{:?} creates to download", missing.len());

    Ok(missing)
}

/// Download creates given in vector of Creates
///
/// Arguments
///
/// * `creates` - Vector of creates to download
/// * `threads` - Number of simultaneous downloads
///
pub fn download_creates(creates: &Vec<Create>, threads: usize) -> Result<(), std::io::Error> {
    // Progress bar for user updates
    let mut progress_bar = ProgressBar::new(creates.len());
    progress_bar.set_action("Downloading", Color::Blue, Style::Bold);

    // Handles for the download threads
    let mut handles = Vec::new();

    // Communications from the main thread to the download thread
    let mut to_thread = Vec::new();

    // Communication from the download thread to the main thread
    let (sender, receiver) = mpsc::channel();

    // Create all the threads
    for i in 0..threads {
        // Generate a MPSC for this thread
        let (msg, thread_rx) = mpsc::channel();

        // Store the object to communicate with the thread
        to_thread.push(msg);

        // Clone a sender for this thread
        let sender_n = sender.clone();

        // Create the thread and push the handler to the vector store
        handles.push(thread::spawn(move || {
            // Wait for all the threads to be created before they report in to the main thread for
            // tasking
            thread::sleep(Duration::from_millis(100));

            // Tell the main thread we are waiting for tasking
            sender_n.send(i).unwrap();

            loop {
                // Block while waiting for tasking
                let b: (String, String, String) = thread_rx.recv().unwrap();
                if b.0 == "exit" {
                    break;
                } else {
                    download_create(b.0, b.1, b.2).unwrap();
                    sender_n.send(i).unwrap();
                }
            }
        }));
    }

    // Download each create
    for create in creates {
        // Make sure we have somewhere for the file to be downloaded to
        let dir_path = Path::new(&create.file_path).parent().unwrap();
        std::fs::create_dir_all(dir_path).unwrap();

        // Calculate the URL to be downloaded
        let target = format!(
            "https://crates.io/api/v1/crates/{}/{}/download",
            create.name, create.version
        );

        // Wait for a thread to become free
        let msg = receiver.recv().unwrap();

        // Get the local path and checksum for this create
        let file_path = String::from(dir_path.to_str().unwrap());
        let checksum = String::from(&create.checksum);

        // Message the selected thread with the requried information
        to_thread[msg].send((target, file_path, checksum)).unwrap();

        progress_bar.inc();
    }

    for th in to_thread {
        th.send((String::from("exit"), String::from(""), String::from("")))
            .unwrap();
    }

    for handle in handles {
        handle.join().unwrap();
    }

    progress_bar.print_info("Download", "Complete", Color::Green, Style::Bold);
    println!("");

    Ok(())
}

/// Download an individual create
///
/// Arguments
///
/// * `target` - String with URL of file to be downloaded
/// * `file_path` - String to local file
/// * `checksum` - String of expected SHA256
///
fn download_create(
    target: String,
    file_path: String,
    checksum: String,
) -> Result<(), std::io::Error> {
    let file_path = Path::new(&file_path);

    let mut response = reqwest::get(&target).unwrap();

    // TODO: unwrap_or should be an error...
    let dest_path = {
        let fname = response
            .url()
            .path_segments()
            .and_then(|segments| segments.last())
            .and_then(|name| if name.is_empty() { None } else { Some(name) })
            .unwrap_or("tmp.bin");

        file_path.join(fname)
    };

    let mut dest = std::fs::File::create(dest_path.to_str().unwrap()).unwrap();
    std::io::copy(&mut response, &mut dest)?;

    let checksum_check = sha256_compare_file(dest_path.to_str().unwrap(), &checksum)?;
    if checksum_check == false {
        // TODO: This should be a progress_bar call
        println!("Failed downloading {}", target);
    }

    Ok(())
}

/// Get data for each version of each create in the list of files
///
/// Arguments
///
/// * `files` - Vector of files containing JSON data to be parsed
///
pub fn get_create_info(
    files: &mut Vec<String>,
    creates: &mut Vec<Create>,
    git_repo: &str,
    file_store: &str,
) -> Result<(), std::io::Error> {
    let mut progress_bar = ProgressBar::new(files.len());
    progress_bar.set_action("Parsing", Color::Blue, Style::Bold);
    // Each entry in files is in the format ./crates.io-index/[a]/[b]/[create]
    for create in files {
        // Use the of the create String to get the file we want to open
        let file_path: String = format!("{}/{}", git_repo, &create);

        // Create a path to the parent of the file_folder
        let sub_folder = Path::new(&create)
            .parent()
            .unwrap()
            .to_str()
            .expect("Failed determine folder");

        // Open the file and use the BufReader to read it line by line
        let file = File::open(file_path)?;
        let reader = BufReader::new(file);

        // Reading each line in the file, parse its JSON and extract the list of files to be
        // downloaded
        for line in reader.lines() {
            // Create the JSON parser
            let json = Json::from_str(&line.unwrap()).unwrap();

            // Extract the data we want
            let name = json
                .find_path(&["name"])
                .unwrap()
                .to_string()
                .trim_matches('"')
                .to_string();
            let version = json
                .find_path(&["vers"])
                .unwrap()
                .to_string()
                .trim_matches('"')
                .to_string();
            let checksum = json
                .find_path(&["cksum"])
                .unwrap()
                .to_string()
                .trim_matches('"')
                .to_string();

            // Infer the file path for this version of this crate
            let relative_path = format!("{}/{}/{}-{}.crate", sub_folder, name, name, version);
            let file_path = format!("{}/{}", file_store, relative_path);

            // Fill a Create struct with the extracted data
            let create = Create {
                name: name,
                version: version,
                checksum: checksum,
                file_path: file_path,
                relative_path: relative_path,
            };
            creates.push(create);
        }
        progress_bar.inc();
    }
    progress_bar.print_info("Parsing", "Complete", Color::Green, Style::Bold);
    println!("");
    println!("There are {:?} creates to get", creates.len());

    Ok(())
}

fn sha256_compare_file(file_path: &str, checksum: &str) -> Result<bool, std::io::Error> {
    let mut file = File::open(file_path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let hash = hasher.result();
    let hash: Vec<u8> = hash.iter().cloned().collect();
    match hash == hex::decode(checksum).unwrap() {
        true => Ok(true),
        false => Ok(false),
    }
}

/// Check for duplicate version numbers
///
/// Arguments
///
/// * `name` - Name of create
/// * `version` - Version of create
/// * `checksum` - Checksum of create
/// * `creates` - Vector of creates to search
///
pub fn check_duplicate_version(
    name: String,
    version: String,
    checksum: String,
    creates: &Vec<Create>,
) -> Result<bool, std::io::Error> {
    for candidate in creates.clone() {
        if candidate.name == name && candidate.version == version {
            if candidate.checksum != checksum {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

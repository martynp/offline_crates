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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
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
    let offset = base_dir.len() + 1; // +1 for the first '/'
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
                    let path_str = path.to_str().unwrap();
                    files.push(String::from(&path_str[offset..]));
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
/// * `packages` - Vector of packages to verify
/// * `missing` - Vector of packages to be filled with missing items
///
pub fn verify_store(
    packages: &mut Vec<Package>,
    missing: &mut Vec<Package>,
) -> std::io::Result<()> {
    let mut progress_bar = ProgressBar::new(packages.len());
    progress_bar.set_action("Verifying", Color::Blue, Style::Bold);

    for package in packages {
        let path_to_package = Path::new(&package.file_path);
        if path_to_package.is_file() {
            if sha256_compare_file(&package.file_path, &package.checksum)? == false {
                progress_bar.print_info(
                    "Failure",
                    &format!(
                        "{}-{} - Checksum incorrect, downloading crate again",
                        package.name, package.version
                    ),
                    Color::Red,
                    Style::Bold,
                );
                missing.push(package.clone());
            }
        } else {
            missing.push(package.clone());
        }
        progress_bar.inc();
    }

    progress_bar.print_info("Verify", "Complete", Color::Green, Style::Bold);
    println!("");

    println!("{:?} packages to download", missing.len());

    Ok(())
}

/// Download packages given in vector of Packages
///
/// Arguments
///
/// * `packages` - Vector of packages to download
/// * `threads` - Number of simultaneous downloads
///
pub fn download_packages(packages: &Vec<Package>, threads: usize) -> Result<(), std::io::Error> {
    // Progress bar for user updates
    let mut progress_bar = ProgressBar::new(packages.len());
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
                    download_package(b.0, b.1, b.2).unwrap();
                    sender_n.send(i).unwrap();
                }
            }
        }));
    }

    // Download each package
    for package in packages {
        // Make sure we have somewhere for the file to be downloaded to
        let dir_path = Path::new(&package.file_path).parent().unwrap();
        std::fs::create_dir_all(dir_path).unwrap();

        // Calculate the URL to be downloaded
        let target = format!(
            "https://crates.io/api/v1/crates/{}/{}/download",
            package.name, package.version
        );

        // Wait for a thread to become free
        let msg = receiver.recv().unwrap();

        // Get the local path and checksum for this package
        let file_path = String::from(dir_path.to_str().unwrap());
        let checksum = String::from(&package.checksum);

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

/// Download an individual package
///
/// Arguments
///
/// * `target` - String with URL of file to be downloaded
/// * `file_path` - String to local file
/// * `checksum` - String of expected SHA256
///
fn download_package(
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

/// Get data for each version of each package in the list of files
///
/// Arguments
///
/// * `files` - Vector of files containing JSON data to be parsed
///
pub fn get_package_info(
    files: &mut Vec<String>,
    packages: &mut Vec<Package>,
    git_repo: &str,
    file_store: &str,
) -> Result<(), std::io::Error> {
    let mut progress_bar = ProgressBar::new(files.len());
    progress_bar.set_action("Parsing", Color::Blue, Style::Bold);

    // Each entry in files is in the format ./crates.io-index/[a]/[b]/[package]
    for package in files {
        // Use the of the package String to get the file we want to open
        let file_path: String = format!("{}/{}", git_repo, &package);

        // Create a path to the parent of the file_folder
        let sub_folder = Path::new(&package)
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

            // Fill a Package struct with the extracted data
            let package = Package {
                name: name,
                version: version,
                checksum: checksum,
                file_path: file_path,
                relative_path: relative_path,
            };
            packages.push(package);
        }
        progress_bar.inc();
    }
    progress_bar.print_info("Parsing", "Complete", Color::Green, Style::Bold);
    println!("");
    println!("There are {:?} packages to get", packages.len());

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
/// * `name` - Name of package
/// * `version` - Version of package
/// * `checksum` - Checksum of package
/// * `packages` - Vector of packages to search
///
pub fn check_duplicate_version(
    name: String,
    version: String,
    checksum: String,
    packages: &Vec<Package>,
) -> Result<(bool), std::io::Error> {
    for candidate in packages.clone() {
        if candidate.name == name && candidate.version == version {
            if candidate.checksum != checksum {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

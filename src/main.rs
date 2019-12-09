
use git2::Repository;
use rustc_serialize::json::{Json, ToJson};

use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use progress_bar::color::{Color, Style};
use progress_bar::progress_bar::ProgressBar;

use sha2::{Digest, Sha256};


extern crate reqwest;
extern crate clap;

use clap::{Arg, App};

const CRATES_URL: &str = "https://github.com/rust-lang/crates.io-index";

#[derive(Debug, Clone)]
struct Package {
    name: String,
    version: String,
    checksum: String,
    file_path: String,
    relative_path: String,
}


/// Equivalent to git pull
///
/// Arguments
///
/// * `path` - Path to repository to fast-forward
///
fn fast_forward(path: &Path) -> Result<(), git2::Error> {
    let repo = Repository::open(path)?;

    repo.find_remote("origin")?.fetch(&["master"], None, None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
    let analysis = repo.merge_analysis(&[&fetch_commit])?;
    if analysis.0.is_up_to_date() {
        Ok(())
    } else if analysis.0.is_fast_forward() {
        let refname = format!("refs/heads/{}", "master");
        let mut reference = repo.find_reference(&refname)?;
        reference.set_target(fetch_commit.id(), "Fast-Forward")?;
        repo.set_head(&refname)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
    } else {
        Err(git2::Error::from_str("Fast-forward only!"))
    }
}

/// Extract file names from crates-io.index repository
///
/// Arguments
///
/// * `dir` - Path to the root of the gir repository
/// * `files` - Vector to store found files
///
fn walk_repo(dir: &Path, base_dir: &str, files: &mut Vec<String>) -> std::io::Result<()> {
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

fn main() {

    let mut progress_bar = ProgressBar::new(0);

    // Using clap to parse command line options
    let matches = App::new("Crates.io Mirror")
        .version("1.0")
        .author("Martyn P")
        .arg(Arg::with_name("index")
             .short("i")
             .long("index")
             .help("Location for crates.io-index")
             .takes_value(true))
        .arg(Arg::with_name("store")
             .short("s")
             .long("store")
             .help("Location for create file store")
             .takes_value(true))
        .arg(Arg::with_name("diff")
             .short("d")
             .long("diff")
             .help("Create list of new packages")
             .takes_value(true))
        .get_matches();

    // Extract the command line arguments
    let git_path = matches.value_of("index").unwrap_or("./crates.io-index");
    let mut store_path = matches.value_of("store").unwrap_or("./crates");

    let repo_dir = Path::new(git_path);
    let store_dir = Path::new(store_path);

    // Check to see if the git repository for the crates exists
    if repo_dir.exists() && repo_dir.is_dir() {

        progress_bar.print_info(
            "Info", 
            &format!("Index directory exists, updating..."),
            Color::Green,
            Style::Bold
        );
        println!("");

        // If it does then try and open
        let _repo = match Repository::open(repo_dir) {
            Ok(repo) => repo,
            Err(e) => panic!("Failed to open: {}", e),
        };
        // If we got a repo, then reset any changes (may not be needed)
        _repo
            .reset(
                &_repo.revparse_single("HEAD").unwrap(),
                git2::ResetType::Hard,
                None,
            )
            .unwrap();
        // Fast-forward any changes
        if let Err(e) = fast_forward(repo_dir) {
            panic!("Failed to pull: {}", e)
        }
    // If directory does not exist, then clone it!
    } else {

        progress_bar.print_info(
            "Info", 
            &format!("Index directory does not exist, cloning..."),
            Color::Green,
            Style::Bold
        );
        println!("");

        let _repo = match Repository::clone(CRATES_URL, git_path) {
            Ok(repo) => repo,
            Err(e) => panic!("Failed to clone: {}", e),
        };
    }

    // Get the file list from the repository
    let mut files = Vec::new();
    walk_repo(&repo_dir, &git_path, &mut files).unwrap();


    let mut new_file = Vec::new();
    for _ in 0..1000 {
        new_file.push(files.pop().unwrap());
    }
    let mut files = new_file;

    let mut packages: Vec<Package> = Vec::new();
    get_package_info(&mut files, &mut packages, git_path, &mut store_path).unwrap();

    /*
    let mut progress_bar = ProgressBar::new(packages.len());
    progress_bar.set_action("Duplicates", Color::Blue, Style::Bold);
    for package in packages.clone() {
        for candidate in packages.clone() {
            if candidate.name == package.name && candidate.version == package.version {
                if candidate.checksum != package.checksum {
                    progress_bar.print_info("Failure", &format!("{}-{} - duplicate found", package.name, package.version), Color::Red, Style::Bold);
                }
            }
        }
        progress_bar.inc();
    }
    progress_bar.print_info("Duplicates", "Complete", Color::Green, Style::Bold);
    println!("");
    */

    let mut missing_files: Vec<Package> = Vec::new();
    verify_store(&mut packages, &mut missing_files).unwrap();

    if matches.is_present("diff") {
        let diff_path = matches.value_of("diff").unwrap();
        let file = File::create(&diff_path).unwrap();
        let mut file = std::io::LineWriter::new(file);
        for package in missing_files.clone() {
            file.write_all(format!("{}\n", package.relative_path).as_bytes()).unwrap();
        }
    }

    download_packages(&mut missing_files, 20).unwrap();
}

fn verify_store(packages: &mut Vec<Package>, missing: &mut Vec<Package>) -> std::io::Result<()> {

    let mut progress_bar = ProgressBar::new(packages.len());
    progress_bar.set_action("Verifying", Color::Blue, Style::Bold);

    for package in packages {
        let path_to_package = Path::new(&package.file_path);
        if path_to_package.is_file() {
            if sha256_compare_file(&package.file_path, &package.checksum)? == false {
                progress_bar.print_info("Failure", &format!("{}-{} - Checksum incorrect, downloading crate again", package.name, package.version), Color::Red, Style::Bold);
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
fn download_packages(packages: &Vec<Package>, threads: usize) -> Result<(), std::io::Error> {
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
               //     println!("Downloading {} to {}", b.0, b.1);
                    download_package(b.0, b.1, b.2).unwrap();
                    sender_n.send(i).unwrap();
                }
            }
        }));
    }

    for package in packages {

        let dir_path = Path::new(&package.file_path).parent().unwrap();
        std::fs::create_dir_all(dir_path).unwrap();
        let dir_path = String::from(dir_path.to_str().unwrap());

        let target = format!(
            "https://crates.io/api/v1/crates/{}/{}/download",
            package.name, package.version
        );

        let msg = receiver.recv().unwrap();

        let checksum = String::from(&package.checksum);

        to_thread[msg].send((target, dir_path, checksum)).unwrap();

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

fn download_package(
    target: String,
    dir_path: String,
    checksum: String,
) -> Result<(), std::io::Error> {
    let dir_path = Path::new(&dir_path);

    let mut response = reqwest::get(&target).unwrap();

    let dest_path = {
        let fname = response
            .url()
            .path_segments()
            .and_then(|segments| segments.last())
            .and_then(|name| if name.is_empty() { None } else { Some(name) })
            .unwrap_or("tmp.bin");

        dir_path.join(fname)
    };

    let mut dest = std::fs::File::create(dest_path.to_str().unwrap()).unwrap();
    std::io::copy(&mut response, &mut dest)?;

    let checksum_check = sha256_compare_file(dest_path.to_str().unwrap(), &checksum)?;
    if checksum_check == false {
        println!("Failed downloading {}", target);
//        panic!("Checksum failed");
    }
    //println!("{:?}", checksum_check);

    //        Err(std::io::Error::new(std::io::ErrorKind::Other, "Checksum failed"));

    Ok(())
}

/// Get data for each version of each package in the list of files
///
/// Arguments
///
/// * `files` - Vector of files containing JSON data to be parsed
///
fn get_package_info(
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

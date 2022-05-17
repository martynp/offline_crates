// TODO: Add diff file generator
// TODO: ADd apache 2
// TODO: Only download files not in diff

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

use clap::{App, Arg};
use git2::Repository;
use log;
use progress_bar::color::{Color, Style};
use progress_bar::progress_bar::ProgressBar;

mod repository;

const CRATES_URL: &str = "https://github.com/rust-lang/crates.io-index";
const CRATES_BRANCH: &str = "master";

/// Equivalent to git pull
///
/// https://stackoverflow.com/a/58778350
///
/// Arguments
///
/// * `path` - Path to repository to fast-forward
///
fn fast_forward(path: &Path, branch: &str) -> Result<(), git2::Error> {
    let repo = Repository::open(path)?;

    repo.find_remote("origin")?.fetch(&[branch], None, None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
    let analysis = repo.merge_analysis(&[&fetch_commit])?;
    if analysis.0.is_up_to_date() {
        Ok(())
    } else if analysis.0.is_fast_forward() {
        let refname = format!("refs/heads/{}", branch);
        let mut reference = repo.find_reference(&refname)?;
        reference.set_target(fetch_commit.id(), "Fast-Forward")?;
        repo.set_head(&refname)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
    } else {
        Err(git2::Error::from_str("Fast-forward only!"))
    }
}

fn main() {
    // Using clap to parse command line options
    let cmd = App::new("Crates.io Mirror")
        .version("1.0")
        .author("Martyn P")
        .arg(
            Arg::with_name("index")
                .short("i")
                .long("index")
                .help("Location for crates.io-index (default is ./crates.io-index)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("store")
                .short("s")
                .long("store")
                .help("Location for crate file store (default is ./crates)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("existing")
                .short("e")
                .long("existing")
                .help("List of existing files (these wont be download unless files have different sha256 checksum - see help for format)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("verify")
                .short("v")
                .long("verify")
                .help("If set the files in the file store will be verified against the checksum on record, this is required when crates have been re-issued.")
                .takes_value(false),
        )
        .get_matches();

    // Extract the command line arguments
    let git_path = cmd.value_of("index").unwrap_or("./crates.io-index");
    let store_path = cmd.value_of("store").unwrap_or("./crates");

    // Check for incompatible settings
    if cmd.is_present("verify") && cmd.is_present("existing") {
        panic!("--verify and --existing cannot be used together");
    }

    // Confirm if existing is set, that the path is valid, otherwise it is a long wait
    // before the user finds out
    if cmd.is_present("existing") {
        let existing_path = Path::new(cmd.value_of("existing").unwrap());
        if !existing_path.is_file() {
            panic!("File given for --existing does not exist");
        }
    }

    let repo_dir = Path::new(git_path);

    // Check to see if the git repository for the crates exists
    if repo_dir.exists() && repo_dir.is_dir() {
        //TODO: Check to see if dir is a repo...

        println!("Index directory exists, updating...");

        // If it does then try and open
        let _repo = match Repository::open(repo_dir) {
            Ok(repo) => repo,
            Err(e) => panic!("Failed to open: {}", e),
        };
        // Fast-forward any changes
        if let Err(e) = fast_forward(repo_dir, CRATES_BRANCH) {
            panic!("Failed to pull: {}", e)
        }

    // If directory does not exist, then clone it!
    } else {
        println!("Index directory does not exist, cloning (this may take a while)...");

        let _repo = match Repository::clone(CRATES_URL, git_path) {
            Ok(repo) => repo,
            Err(e) => panic!("Failed to clone: {}", e),
        };
    }

    // Get the file list from the repository
    let mut files = Vec::new();
    repository::walk_repo(&repo_dir, &git_path, &mut files).unwrap();

    // Get metadata from the index repository
    let mut crates_in_repo: Vec<repository::Crate> = Vec::new();
    repository::get_crate_info(&files, &mut crates_in_repo, &git_path, &store_path).unwrap();

    // Verify the store
    if cmd.is_present("verify") {
        crates_in_repo = repository::verify_store(&crates_in_repo, 10).unwrap();
    }

    // Process an "existing" file

    if cmd.is_present("existing") {
        let existing_path = Path::new(cmd.value_of("existing").unwrap());
        if !existing_path.is_file() {
            panic!("File given for --existing does not exist");
        }
        // Get all the lines out of the file
        let file = File::open(existing_path).unwrap();
        let reader = std::io::BufReader::new(file);
        // Line format is
        // [sha256] [relative path]
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();

        let mut progress_bar = ProgressBar::new(lines.len());
        progress_bar.set_action("Processing", Color::Blue, Style::Bold);

        let mut removed_items: usize = 0;
        for line in &lines {
            progress_bar.inc();
            let parts = line.split(" ").collect::<Vec<&str>>();
            let checksum = parts[0];
            let mut path = String::from(parts[2]);
            if path.starts_with("./") {
                // Remove the ./ at the start
                path.remove(0);
                path.remove(0);
            }
            let path = std::path::Path::new(store_path).join(std::path::Path::new(&path));

            let mut to_remove: usize = 0;
            for (index, c) in crates_in_repo.iter().enumerate() {
                if c.file_path == path.to_str().unwrap() {
                    if c.checksum == checksum {
                        to_remove = index;
                        break;
                    }
                }
            }
            crates_in_repo.remove(to_remove);
            removed_items += 1;
        }
        println!(
            "Removed {} items based on existing files list ({} entries)",
            removed_items,
            lines.len(),
        );
    }

    // Do the deed with 20 threads
    repository::download_crates(&mut crates_in_repo, 20).unwrap();
}

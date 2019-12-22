use git2::Repository;

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

use progress_bar::color::{Color, Style};
use progress_bar::progress_bar::ProgressBar;

use repository::*;

extern crate clap;
extern crate reqwest;

use clap::{App, Arg};

const CRATES_URL: &str = "https://github.com/rust-lang/crates.io-index";

/// Equivalent to git pull
///
/// https://stackoverflow.com/a/58778350
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

fn main() {
    // Using clap to parse command line options
    let matches = App::new("Crates.io Mirror")
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
                .help("Location for create file store (default is ./crates)")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("diff")
                .short("d")
                .long("diff")
                .help("Create list of new packages")
                .takes_value(true),
        )
        .get_matches();

    // Extract the command line arguments
    let git_path = matches.value_of("index").unwrap_or("./crates.io-index");
    let mut store_path = matches.value_of("store").unwrap_or("./crates");

    let repo_dir = Path::new(git_path);

    // Check to see if the git repository for the crates exists
    if repo_dir.exists() && repo_dir.is_dir() {
        let mut progress_bar = ProgressBar::new(0);
        progress_bar.print_info(
            "Info",
            &format!("Index directory exists, updating..."),
            Color::Green,
            Style::Bold,
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
        let mut progress_bar = ProgressBar::new(0);
        progress_bar.print_info(
            "Info",
            &format!("Index directory does not exist, cloning..."),
            Color::Green,
            Style::Bold,
        );
        println!("");

        let _repo = match Repository::clone(CRATES_URL, git_path) {
            Ok(repo) => repo,
            Err(e) => panic!("Failed to clone: {}", e),
        };
    }

    // Get the file list from the repository
    let mut files = Vec::new();
    repository::walk_repo(&repo_dir, &git_path, &mut files).unwrap();

    // Get metadata from the index repository
    let mut packages: Vec<Package> = Vec::new();
    repository::get_package_info(&mut files, &mut packages, git_path, &mut store_path).unwrap();

    // Verify the store
    let mut missing_files: Vec<Package> = repository::verify_store(&mut packages, 10).unwrap();

    // If a diff is required, save it
    if matches.is_present("diff") {
        let diff_path = matches.value_of("diff").unwrap();
        let file = File::create(&diff_path).unwrap();
        let mut file = std::io::LineWriter::new(file);
        for package in missing_files.clone() {
            file.write_all(format!("{}\n", package.relative_path).as_bytes())
                .unwrap();
        }
    }

    // Do the deed with 20 threads
    repository::download_packages(&mut missing_files, 20).unwrap();
}

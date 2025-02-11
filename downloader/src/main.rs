use std::io::{Error, ErrorKind, Result};
use std::path::PathBuf;

use clap::Parser;
use glob::{glob, Paths};

/// Crates Repository Downloader
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Repository to mirror
    #[arg(
        short,
        long,
        default_value = "https://github.com/rust-lang/crates.io-index"
    )]
    repository: String,

    /// Branch name
    #[arg(short, long, default_value = "master")]
    branch: String,

    /// Limit number of crates to process (for debug)
    #[arg(long, default_value_t = -1)]
    limit: i32,

    /// Location to store files
    #[arg(short, long)]
    location: PathBuf,

    /// Git repository, if specified this repository will be reset and updated
    #[arg(short, long)]
    git_repository: PathBuf,

    /// Optional search path for existing crates
    #[arg(short, long)]
    search_path: Vec<String>,

    /// Optional input containing sha256 checksums of existing files
    #[arg(short, long)]
    existing: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!(
        "Updating git repository {}@{}",
        args.repository,
        args.branch
    );
    update_git_repository(&args)?;

    log::info!("Collecting metadata...");
    let glob_search = &format!("{}/**/*", args.git_repository.to_string_lossy());
    let crate_definitions: Paths = glob(glob_search).expect("Location glob search failed");
    let to_process = crate_definitions.count();
    log::info!("Found {} potential crate definitions", to_process);
    let crate_definitions: Paths = glob(glob_search).expect("Location glob search failed");

    log::info!("Processing crate definitions");
    let crates = lib::process_crate_definition(crate_definitions, to_process).await;
    log::info!("Found {} crate permutations", crates.len());

    let crates = lib::process_existing_crates_list(&args.existing, crates).await;

    log::info!("Downloading {} crates", crates.len());
    lib::download_crates(
        &args.git_repository,
        &args.location,
        args.limit,
        &args.search_path,
        crates,
    )
    .await?;

    Ok(())
}

pub fn update_git_repository(args: &crate::Args) -> Result<()> {
    use git2::Repository;

    // User has defined a pre-existing repository to use, or a blank folder to use
    let repo = match Repository::open(args.git_repository.clone()) {
        Ok(r) => {
            // If it did open, is it actually the correct repository?
            log::info!(
                "Opened repository located at {}",
                args.git_repository.to_string_lossy()
            );
            r
        }
        Err(_) => {
            log::info!(
                "Git repository {} was empty, cloning from {}",
                args.git_repository.to_string_lossy(),
                &args.repository
            );
            Repository::clone(&args.repository, args.git_repository.clone())
                .map_err(|e| Error::new(ErrorKind::Other, e))?
        }
    };

    log::info!("Checking out branch {}", &args.branch);
    let (object, _) = repo
        .revparse_ext(&args.branch)
        .map_err(|e| Error::new(ErrorKind::Other, e))?;

    repo.checkout_tree(&object, None)
        .map_err(|e| Error::new(ErrorKind::Other, e))?;

    Ok(())
}

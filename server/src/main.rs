use std::path::PathBuf;

use clap::Parser;
use glob::{glob, Paths};
use rocket::fs::NamedFile;
use rocket::State;

use lib::structures::*;

struct CrateState {
    crates: Vec<CrateData>,
}

#[macro_use]
extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    "Offline Crates Mirror"
}

#[get("/api/v1/crates/<name>/<version>/download")]
async fn api(
    location: &State<PathBuf>,
    state: &State<CrateState>,
    name: &str,
    version: &str,
) -> Option<NamedFile> {
    let file = state
        .crates
        .iter()
        .find(|c| c.name == name && c.vers == version)
        .map(lib::path_to_crate)?;
    NamedFile::open(location.join(&file)).await.ok()
}

/// Crates Server Downloader
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Limit number of crates to process (for debug)
    #[arg(long, default_value_t = -1)]
    limit: i32,

    /// Location to store files
    #[arg(short, long)]
    location: PathBuf,

    /// Git repository, if specified this repository will be reset and updated
    #[arg(short, long)]
    git_repository: PathBuf,
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Collecting metadata...");
    let glob_search = &format!("{}/**/*", args.git_repository.to_string_lossy());
    let crate_definitions = glob(glob_search).expect("Location glob search failed");
    let to_process = crate_definitions.count();
    log::info!("Found {} potential crate definitions", to_process);
    let crate_definitions: Paths = glob(glob_search).expect("Location glob search failed");

    log::info!("Processing crate definitions");
    let crates = lib::process_crate_definition(crate_definitions, to_process).await;
    log::info!("Found {} crate permutations", crates.len());

    let _rocket = rocket::build()
        .manage(args.location)
        .manage(CrateState { crates })
        .mount("/", routes![index, api])
        .ignite()
        .await?
        .launch()
        .await?;

    Ok(())
}

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

#[get("/config.json")]
async fn config_json(config: &State<String>) -> String {
    config.to_string()
}

/// Offline Crates Server
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
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

    let config = rocket::tokio::fs::read_to_string(args.git_repository.join("config.json"))
        .await
        .expect("Failed to read config.json file from git registry");

    let _rocket = rocket::build()
        .manage(args.location)
        .manage(CrateState { crates })
        .manage(config)
        .mount("/", routes![index, api, config_json])
        .ignite()
        .await?
        .launch()
        .await?;

    Ok(())
}

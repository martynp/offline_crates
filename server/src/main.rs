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
    locations: &State<Locations>,
    state: &State<CrateState>,
    name: &str,
    version: &str,
) -> Option<NamedFile> {
    let file = state
        .crates
        .iter()
        .find(|c| c.name == name && c.vers == version)
        .map(lib::path_to_crate)?;
    NamedFile::open(locations.store.join(&file)).await.ok()
}

#[get("/<first>/<name>")]
async fn sparse_short(
    locations: &State<Locations>,
    first: &str,
    name: &str,
) -> Option<NamedFile> {

    if !matches!(first, "1" | "2" | "3") {
        return None;
    }

    let path = locations
        .git_repository
        .join(format!("{first}/{name}"));

    let canonical_path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return None,
    };

    if !canonical_path.starts_with(&locations.git_repository) {
        return None;
    }

    NamedFile::open(canonical_path).await.ok()
}

#[get("/<first>/<second>/<name>")]
async fn sparse(
    locations: &State<Locations>,
    first: &str,
    second: &str,
    name: &str,
) -> Option<NamedFile> {
    let path = locations
        .git_repository
        .join(format!("{first}/{second}/{name}"));

    let canonical_path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return None,
    };

    if !canonical_path.starts_with(&locations.git_repository) {
        return None;
    }

    NamedFile::open(canonical_path).await.ok()
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
    store: PathBuf,

    /// Git repository location on disk
    #[arg(short, long)]
    git_repository: PathBuf,

    /// Optional search path for existing crates
    #[arg(long)]
    search_path: Vec<String>,
}

struct Locations {
    store: PathBuf,
    git_repository: PathBuf,
}

#[allow(clippy::result_large_err)]
#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Collecting metadata...");
    let glob_search = &format!("{}/**/*", args.git_repository.to_string_lossy());
    let crate_definitions = glob(glob_search).expect("Location glob search failed");
    let to_process = crate_definitions.count();
    log::info!("Found {to_process} potential crate definitions");
    let crate_definitions: Paths = glob(glob_search).expect("Location glob search failed");

    log::info!("Processing crate definitions");
    let crates = lib::process_crate_definition(crate_definitions, to_process).await;
    log::info!("Found {} crate permutations", crates.len());

    if !args.search_path.is_empty() {
        log::info!("Copying missing crates from search paths");
        let copied = lib::copy_missing_crates(&args.search_path, &args.store, &crates)
            .await
            .expect("Failed to copy missing crates");
        log::info!("Copied {copied} crates from search paths");
    }

    let config = rocket::tokio::fs::read_to_string(args.git_repository.join("config.json"))
        .await
        .expect("Failed to read config.json file from git registry");

    let _rocket = rocket::build()
        .manage(Locations {
            store: args.store,
            git_repository: args.git_repository,
        })
        .manage(CrateState { crates })
        .manage(config)
        .mount("/", routes![index, api, config_json, sparse, sparse_short])
        .ignite()
        .await?
        .launch()
        .await?;

    Ok(())
}

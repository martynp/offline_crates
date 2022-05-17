use std::path::Path;

use rocket::fs::NamedFile;

use rocket::State;

use clap::{App, Arg};

mod repository;

struct CrateState {
    crates: Vec<repository::Crate>,
}

#[macro_use]
extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    // TODO: See landing page ticket...
    "Hello, world!"
}

#[get("/api/v1/crates/<name>/<version>/download")]
async fn api(crates: &State<CrateState>, name: String, version: String) -> Option<NamedFile> {
    let mut file_path: String = String::from("");
    let mut found: bool = false;

    // Iterate over the creates vector to look for a match, update the file_path string and found bool
    // if there is a match
    for create in &crates.crates {
        if create.name == name && create.version == version {
            file_path = create.file_path.clone();
            found = true;
            break;
        }
    }

    NamedFile::open(&file_path).await.ok()
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    // Using clap to parse command line options
    let cmd_args = App::new("Crates.io Mirror Server")
        .version("1.0")
        .author("Martyn P")
        .arg(
            Arg::with_name("index")
                .short("i")
                .long("index")
                .help("Location for crates.io-index")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("store")
                .short("s")
                .long("store")
                .help("Location for create file store")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("cache")
                .short("c")
                .long("cache")
                .help("Cache file")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("create_cache")
                .long("create_cache")
                .help("Create cache file"),
        )
        .get_matches();

    // Creates are going to be read from file, or created from index repository
    let mut crates: Vec<repository::Crate> = Vec::new();

    // Extract the command line arguments
    let git_path = cmd_args.value_of("index").unwrap();
    let store_path = cmd_args.value_of("store").unwrap();
    let repo_dir = Path::new(git_path);

    // Find all the meta files in the repo and extract crate data
    let mut files = Vec::new();
    repository::walk_repo(&repo_dir, &git_path, &mut files).unwrap();
    repository::get_crate_info(&mut files, &mut crates, git_path, store_path).unwrap();

    // Start the server...

    let _rocket = rocket::build()
        .manage(CrateState { crates: crates })
        .mount("/", routes![index, api])
        .ignite()
        .await?
        .launch()
        .await?;

    Ok(())
}

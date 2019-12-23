#![feature(proc_macro_hygiene, decl_macro)]

use rocket::response::NamedFile;
use rocket::response::status::NotFound;
use rocket::State;

use repository::*;

use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::Path;

use clap::{Arg, App};

struct PackageState {
    packages : Vec<repository::Package>,
}

#[macro_use] extern crate rocket;

#[get("/")]
fn index() -> &'static str {
    // TODO: See landing page ticket...
    "Hello, world!"
}

#[get("/api/v1/crates/<name>/<version>/download")]
fn api(packages: State<PackageState>, name: String, version: String) -> Result<NamedFile, NotFound<String>>{

    let mut file_path : String = String::from("");
    let mut found : bool = false;

    // Iterate over the packages vector to look for a match, update the file_path string and found bool
    // if there is a match
    for package in &packages.packages {
        if package.name == name && package.version == version {
            file_path = package.file_path.clone();
            found = true;
            break;
        }
    }

    if found == false {
        // Crete is not in index if found is still false
        Err(NotFound(String::from("Crate/version not found in index")))
    } else {
        // Create in index, but may not exists on disk...
        NamedFile::open(&file_path).map_err(|_| NotFound(String::from("File not found")))
    }
}



fn main() -> Result<(), std::io::Error> {

    // Using clap to parse command line options
    let cmd_args = App::new("Crates.io Mirror Server")
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
        .arg(Arg::with_name("cache")
             .short("c")
             .long("cache")
             .help("Cache file")
             .takes_value(true))
        .arg(Arg::with_name("create_cache")
             .long("create_cache")
             .help("Create cache file"))
        .get_matches();


    // Packages are going to be read from file, or created from index repository
    let mut packages: Vec<Package> = Vec::new();

    // --cache without --create_cache means load the cache from file and run
    if cmd_args.is_present("cache") && cmd_args.is_present("create_cache") == false {

        // Pull the command line argument and attempt to open for reading
        let cache_file = cmd_args.value_of("cache").unwrap();
        let fp = File::open(cache_file)?;

        // There is one JSON per lane so use the BufReader to get a line at a time
        let reader = BufReader::new(fp);

        // Iterate over and use serde to deserialize the JSON
        for line in reader.lines() {
            let deserialized : Package = serde_json::from_str(&line.unwrap()).unwrap();
            packages.push(deserialized);
        }

    // Not using a cache, or creating a new one
    } else {

        // Extract the command line arguments
        let git_path = cmd_args.value_of("index").unwrap();
        let mut store_path = cmd_args.value_of("store").unwrap();
        let repo_dir = Path::new(git_path);

        // Find all the meta files in the repo and extract crate data
        let mut files = Vec::new();
        repository::walk_repo(&repo_dir, &git_path, &mut files).unwrap();
        repository::get_package_info(&mut files, &mut packages, git_path, &mut store_path).unwrap();

        // If create path is present, store the data in the given cache file
        if cmd_args.is_present("create_cache") {
            let cache_file = cmd_args.value_of("cache").expect("Cache path is required to create cache");
            let mut fp = File::create(cache_file)?;
            for package in packages {
                let json_str = serde_json::to_string(&package).unwrap();
                fp.write(json_str.as_bytes()).unwrap();
                fp.write(b"\n").unwrap();
            }
            std::process::exit(-1);
        }

    }

    // Start the server...
    rocket::ignite()
        .manage(PackageState { packages : packages })
        .mount("/", routes![index, api])
        .launch();

    Ok(())
}


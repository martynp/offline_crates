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
    "Hello, world!"
}

#[get("/api/v1/crates/<name>/<version>/download")]
fn api(packages: State<PackageState>, name: String, version: String) -> Result<NamedFile, NotFound<String>>{
    println!("{}/{}", name, version);
    let mut file_path : String = String::from("");

    for package in packages.packages.clone() {
        if package.name == name && package.version == version {
            file_path = package.file_path;
        }
    }
    println!("{}", file_path);

    NamedFile::open(&file_path).map_err(|_| NotFound(format!("File not found")))
}



fn main() -> Result<(), std::io::Error> {

    // Using clap to parse command line options
    let matches = App::new("Crates.io Mirror Server")
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

    // Extract the command line arguments
    // For later...
    let git_path = matches.value_of("index").unwrap();
    let mut store_path = matches.value_of("store").unwrap();

    let mut packages: Vec<Package> = Vec::new();

    if matches.is_present("cache") && matches.is_present("create_cache") == false {

        let cache_file = matches.value_of("cache").unwrap();
        let fp = File::open(cache_file)?;

        let reader = BufReader::new(fp);

        for line in reader.lines() {
            let deserialized : Package = serde_json::from_str(&line.unwrap()).unwrap();
            packages.push(deserialized);
        }

    } else {


        let repo_dir = Path::new(git_path);

        let mut files = Vec::new();
        repository::walk_repo(&repo_dir, &git_path, &mut files).unwrap();

        repository::get_package_info(&mut files, &mut packages, git_path, &mut store_path).unwrap();

        if matches.is_present("create_cache") {
            let cache_file = matches.value_of("cache").expect("Cache path is required to create cache");
            let mut fp = File::create(cache_file)?;
            for package in packages {
                let json_str = serde_json::to_string(&package).unwrap();
                fp.write(json_str.as_bytes()).unwrap();
                fp.write(b"\n").unwrap();
            }
            std::process::exit(-1);
        }

    }


    rocket::ignite()
        .manage(PackageState { packages : packages })
        .mount("/", routes![index, api])
        .launch();

    Ok(())
}


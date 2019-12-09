#![feature(proc_macro_hygiene, decl_macro)]
use rocket::response::NamedFile;
use rocket::response::status::NotFound;
use rocket::State;
use rustc_serialize::json::{Json, ToJson};

use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::Path;

use progress_bar::color::{Color, Style};
use progress_bar::progress_bar::ProgressBar;


struct PackageState {
    packages : Vec<Package>,
}

#[macro_use] extern crate rocket;
#[derive(Debug, Clone)]
struct Package {
    name: String,
    version: String,
    checksum: String,
    file_path: String,
    relative_path: String,
}


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



fn main() {

    let git_path = "/home/martyn/virtual_machines/crates/crates.io-index";
    let mut store_path = "/home/martyn/virtual_machines/crates/crates-mirror/crates";

    let repo_dir = Path::new(git_path);

    let mut files = Vec::new();
    walk_repo(&repo_dir, &git_path, &mut files).unwrap();

    let mut packages: Vec<Package> = Vec::new();
    get_package_info(&mut files, &mut packages, git_path, &mut store_path).unwrap();




    rocket::ignite()
        .manage(PackageState { packages : packages })
        .mount("/", routes![index, api])
        .launch();
}

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



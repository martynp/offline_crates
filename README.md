# Complete offline mirror of crates.io

This repository contains applications to create and update and serve a local copy of the crates.io crate repository. There are three parts to the crates.io service:

1) crates.io-index - the github hosted repository containing the metadata for all crates on crates.io.
2) File store - the place where cargo goes to get the crate it is looking for, served via a REST API.
3) Local configuration to point at the alternative file source.

In order to create the offline mirror you need to mirror the index, create the offline file store and then process the API.

# Creating the initial mirror

Local machines / projects are directed to a hosted git repository, the repository contains the metadata for all crates and a config.json file which cargo uses to formulate the correct URL to download dependencies - the config.json in the index needs to be modified to use alternative file stores, for example when using a local server:

```
{
  "dl": "http://localhost:8000/api/v1/crates",
  "api": "http://localhost:8000"
}
```

In order to have an offline repository there needs to be local access to https://github.com/rust-lang/crates.io-index. There are many ways to go about this, for example using Gogs (https://hub.docker.com/r/gogs/gogs/).

Tldr: Mirror https://github.com/rust-lang/crates.io-index and change config.json to point to your local API and download server.

# Creating the file store

Using cargo to build the offline_crates application (i.e. `cargo build --release`) the application has the following options:

* -i / --index
* -s / --store
* -e / --existing
* -v / --verify

`index` should point to a directory which already contains a clone of the crates.io-index repository, if the directory does not exist then the official index will be cloned from github. The default location is ./crates.io-index.

The `store` is the path to the location for the local file store. The default location is ./crates.

For example:

```
cargo run -- --index /data/crates.io-index --store /data/store
```

The `existing` is a list of files which already exist in the index, and their sha256. The following command will create the list of files:

``` bash
find . -name "*.crate" | xargs sha256sum > existing.files
```

This list can be generated from the offline store and moved to the online location, or the list of files can be tracked at the online location and each set of additional files added to the list of existing files.

If the existing files list is given, any matching crates will not be downloaded.

Tldr: `cargo run -- --index /data/crates.io-index --store /data/store`, copy /data/store to offline location. Create a list of files using `find . -name "*.crate" | xargs sha256sum > existing.files`

# Running the server

The server is included as a sub-project in ./offline_crates_server. Using cargo to build the offline_crates_server and run it manually, or use the build script in ./packaging:

``` bash
cd packaging
source build_docker.sh
```

This will create `offline_crates_server_image.tar.gz` which can be copied to the offline system.

The offline_crate_server executables has the following options:

* -i / --index
* -s / --store

Index and store are the same as above, for consistency clone from the index from the local mirror.


Run the server using:

```
offline_crates_server --index /data/crates.io-index --store /data/store
```

or,

```
docker run -d -it --restart=unless-stopped --env ROCKET_ADDRESS=0.0.0.0 --env ROCKET_PORT=8000 --name mirror -p 8000:8000 -v /data/crates.io-index:/index -v /data/store:/store offline_crates_server --index /index --store /store
```

The server will take a few seconds to scan all the files in the index before starting.


# Local configuration

Finally set the local configuration to look at the local mirror.

Cargo looks for its configuration in a number of locations. This includes the ./.cargo/config for each project and $HOME/.cargo/config for all of a users projects.

The following can be added to `$HOME/.cargo/config` files to configure which repository Cargo uses for its index:

```
[source]

[source.mirror]
registry = "http://localhost:8080/crates.io-index.git"

[source.crates-io]
replace-with = "mirror"
```

# Updating

To update the repo run the application you can run the application against the previous store - this will update the index and check to see that all the downloaded crates are correct (--verify) then only download new or changed crates.

Alternatively the `--existing` option can be used, which will mean only new/changed files are downloaded. Those files can then be moved to the offline repo.

A new list of 'existing' files will need to be generated and appended to the old list - or the list can be generated from the offline mirror directly.

In both instances the index will need to be updated on the offline mirror, and the config.json changed to reflect the local configuration.


# TODO

* Make all of the above simpler!
  * Package executables
  * Docker container for server
  * Simple homepage for server allowing searching - the offline repository might be a bit behind crates-io.index so users will need to know which versions of a crate are available.
  * Testing...
  * Refactoring...

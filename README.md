

# Complete offline mirror of crates.or

This repository contains applications to create and serve a local copy of the crates.io crate repository. There are three parts to the crates.io service.

1) crates.io-index - the github hosted repository containing the metadata for all crates on crates.io.
2) File store - the place where cargo goes to get the crate it is looking for, served via a REST API.
3) Local configuration to point at the alternative file source.

In order to create the offline mirror you need to mirror the index, create the offline file store and then process the API.

# Mirroring the index

Local machines / projects are directed to a hosted git repository, the repository contains the metadata for all crates and a config.json file which cargo uses to formulate the correct URL to download dependencies - the config.json needs to be modified to use alternative file stores, for example when using a local server:

```
{
  "dl": "http://localhost:8000/api/v1/crates",
  "api": "http://localhost:8000"
}
```

In order to have an offline repository there needs to be local access to https://github.com/rust-lang/crates.io-index. There are many ways to go about this, for example using Gogs (https://hub.docker.com/r/gogs/gogs/).

When developing with an internet connection it is possible to fork the original repository and then change the config.json as required.

# Creating the file store

Using cargo to build the offline\_crates application (i.e. `cargo build --release`) the ./target/release/offline\_crates application has the following options:

* -i / --index
* -s / --store
* -d / --diff

Index should point to a directory which already contains a clone of the crates.io-index repository, if the directory does not exist then the official index will be cloned from github which is probably not what you want. The default location is ./crates.io-index.

The store is the path to the location for the local file store. The default location is ./crates.

The diff option will fill the given file with the relative paths to newly added files. This can be used to track changes to the repository making it possible to identify new files for transfer to offline systems.

For example:

```
./target/release/offline_crates --index ~/offline/crates.io/index --store ~/offline/crates
```

# Running the server

The server is included as a sub-project in ./offline\_crates\_server. Using cargo to build the offline\_crates_\server (`cargo build --release`), the ./target/release/offline\_crates\_server has the following options:

* -i / --index
* -s / --store
* -c / --cache
* --create\_cache

Index and store are the same as above.

When the server starts it will parse all the files in the index to extract all the information required about the crates in the file store. This can take a long time. Therefore it is possible to run the application with a cache of the metadata, the cache is created:

```
./target/release/offline_crates_server --cache ./cache.json --index ~/offline/crates.io/index --store ~/offline/crates --create_cache
```

The server is then started using:

```
./target/release/offline_crates_server --cache ./cache.json
```

Without the cache the server is started using:

```
./target/release/offline_crates_server --index ~/offline/crates.io/index --store ~/offline/crates
```

# Local configuration

Cargo looks for its configuration in a number of locations. This includes the ./.cargo/config for each project and $HOME/.cargo/config for all of a users projects.

The following can be added to those files to configure which repository Cargo uses for its index:

```
[source]

[source.mirror]
registry = "http://localhost:8080/crates.io-index.git"

[source.crates-io]
replace-with = "mirror"
```

# TODO

* Make all of the above simpler!
  * Package executables
  * Docker container for server
  * Simple homepage for server allowing searching - the offline repository might be a bit behind crates-io.index so users will need to know which versions of a crate are available.
  * Testing...
  * Refactoring...

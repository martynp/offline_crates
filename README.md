# Offline Crates Repository Mirror (offline crates.io mirror)

This tool enables you to create an offline mirror of the crates repository.

The crate meta-data and crates themselves are downloaded and then hosted using a file server. Minor modifications to the end users local cargo configuration are required to point at the locally hosted repository.


## Downloading crates files

To download a mirror of the default crates.io repository:

``` bash
cargo run -p downloader -- --location ./files --git-repository ./git
```

This will clone the crate repository's git repo to `./git` and download the files to `./files`.

The following additional parameters can be used:

- `--existing` set a list of files which already exist in the offline index and their sha256. The following command will create the list of files:

  ``` bash
  find . -name "*.crate" | xargs sha256sum > existing.files
  ```

  This list can be generated from the offline store and moved to the online location, or the list of files can be tracked at the online location and each set of additional files added to the list of existing files.

  If the existing files list is given, any matching crates will not be downloaded.

- `--search-path` defines a local path to look for crate files, if found the file will be copied in to the specified file location. The file location should not be given as a search path, files existing in the file location will not be re-downloaded.


## Server Configuration

In the offline environment Cargo needs to be directed to a git repository where additional configuration information will be retrieved.

A copy of the git repository used to download the crates files should be transferred to the offline environment and the `config.json` file changed to reflect the offline addressing, for example:

```
{
  "dl": "http://localhost:8000/api/v1/crates",
  "api": "http://localhost:8000"
}
```

This change must be committed to the default branch.

The server can be started using the following command:

``` bash 
cargo run -p server -- --location /mnt/crates --git-repository http://localhost:8080/crates.io-index.git
```

The following additional parameters can be used:

- `--search-path` will look for missing crate files and copy them in to the given file store


## Local configuration

Cargo looks for its configuration in a number of locations. This includes the `./.cargo/config.toml` for each project and `$HOME/.cargo/config.toml` for all of a users projects.

The following can be added to `$HOME/.cargo/config.toml` files to configure which default repository Cargo uses for its index:

```
[source]

[source.mirror]
registry = "sparse+http://localhost:8000/"

[source.crates-io]
replace-with = "mirror"
```

If using the git based crates registry, the `registry` value should point to the HTTP path of the git repository index.

# Docker

Each release is also packaged in to a single docker image, hosted on docker hub (https://hub.docker.com/repository/docker/martynp/offline-crates/). The `download` and `server` binaries are available within the image:

```
# Download / update local mirror
docker run --rm -it -u $(id -u):$(id -g) -v [path/to/location]:/files -v [path/to/git]:/git martynp/offline-crates:latest downloader -l /files -g /git

# Start server on port 8000
docker run --rm -it -u $(id -u):$(id -g) -p 8000:8000 -v [path/to/location]:/files -v [path/to/git]:/git martynp/offline-crates:latest server -l /files -g /git
```

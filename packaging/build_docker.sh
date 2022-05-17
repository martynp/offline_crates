#!/bin/sh

pushd ../offline_crates_server
cargo clean
cargo build --release
popd 

cp ../offline_crates_server/target/release/offline_crates_server ./

docker build . -t offline_crates_server:latest
docker save offline_crates_server:latest | gzip offline_crates_server_image.tar.gz

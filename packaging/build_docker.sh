#!/bin/sh

cp ../offline_crates_server/target/release/offline_crates_server ./

docker build . -t offline_crates_server:latest
docker save offline_crates_server:latest -o offline_crates_server_image.tar

FROM ubuntu:latest
RUN apt-get update && apt-get install -y \
  openssl \
&& rm -rf /var/lib/apt/lists/*
COPY ./offline_crates_server /


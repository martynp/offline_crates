FROM rust:1.87.0-slim-bookworm AS builder

RUN apt-get update \
  && apt-get install build-essential --no-install-recommends -y

COPY . /workspace/
RUN cd /workspace/ && cargo build --release

FROM debian:bookworm-slim

COPY --from=builder /workspace/target/release/downloader /usr/bin
COPY --from=builder /workspace/target/release/server /usr/bin

ENV ROCKET_LOG=warn

CMD server --help
FROM rust:1.50 as builder
RUN apt-get update && apt-get install -y libssl-dev

COPY . /tmp
WORKDIR /tmp
RUN cargo build --release

FROM ubuntu:18.04
RUN apt-get update && apt-get install -y libssl-dev
COPY --from=builder /tmp/target/release/graphgate /usr/bin/graphgate

ENTRYPOINT [ "graphgate" ]

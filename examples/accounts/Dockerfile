FROM rust:alpine as builder
RUN apk add musl-dev

COPY . /tmp
WORKDIR /tmp

RUN cargo --version
RUN cargo build --release

FROM alpine as base
COPY --from=builder /tmp/target/release/federation-accounts /usr/bin/federation-accounts

ENTRYPOINT [ "federation-accounts" ]

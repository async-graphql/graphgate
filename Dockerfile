###
# Builder
###
FROM rust:latest as builder

RUN rustup target add x86_64-unknown-linux-musl
RUN apt update && apt install -y musl-tools musl-dev
RUN update-ca-certificates

ENV USER=graphgate
ENV UID=10001


RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

WORKDIR /graphgate

COPY ./ .

RUN cargo build --target x86_64-unknown-linux-musl --release

###
# Final Image
###

FROM scratch

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /graphgate

COPY --from=builder /graphgate/target/x86_64-unknown-linux-musl/release/graphgate ./

USER graphgate:graphgate

ENTRYPOINT [ "/graphgate/graphgate" ]

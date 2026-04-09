FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev pkgconf
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release && \
    ./target/release/benchv2 --help > /dev/null

FROM alpine:3.23
RUN apk add --no-cache ca-certificates curl
COPY --from=builder /app/target/release/benchv2 /usr/local/bin/benchv2
ENTRYPOINT ["benchv2"]

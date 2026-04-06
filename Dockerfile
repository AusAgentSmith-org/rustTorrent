# syntax=docker/dockerfile:1
# Multi-stage build: compile rtbit with webui from source, output minimal scratch image.

FROM rust:alpine AS builder

RUN apk update && apk add clang lld npm pkgconf musl-dev openssl-dev openssl-libs-static curl

COPY . /src/
WORKDIR /src/

ENV OPENSSL_STATIC=1

# Remove desktop workspace member (excluded by .dockerignore, needs glib)
RUN sed -i '/"desktop\/src-tauri"/d' Cargo.toml

RUN --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/usr/local/cargo/registry/cache \
    --mount=type=cache,target=/usr/local/cargo/registry/index \
    cargo build --profile release-github && \
    cp target/release-github/rtbit /bin/rtbit

# ---

FROM lscr.io/linuxserver/baseimage-alpine:3.23

COPY --from=builder /bin/rtbit /bin/rtbit
COPY root/ /

RUN mkdir -p /home/rtbit/db /home/rtbit/cache /home/rtbit/downloads /home/rtbit/completed

WORKDIR /home/rtbit

ENV HOME=/home/rtbit
ENV XDG_DATA_HOME=/home/rtbit/db
ENV XDG_CACHE_HOME=/home/rtbit/cache
ENV RTBIT_HTTP_API_LISTEN_ADDR=0.0.0.0:3030
ENV RTBIT_LISTEN_PORT=4240
ENV RTBIT_DOWNLOAD_FOLDER=/home/rtbit/downloads

VOLUME /home/rtbit/db
VOLUME /home/rtbit/cache
VOLUME /home/rtbit/downloads
VOLUME /home/rtbit/completed

EXPOSE 3030
EXPOSE 4240

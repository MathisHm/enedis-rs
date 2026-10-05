ARG BUILD_FROM=debian:bookworm-slim

# Stage 1: Builder
FROM rust:1.80-slim-bookworm AS builder

WORKDIR /usr/src/enedis-rs
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    sqlite3 \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Compilation de l'exécutable enedis avec fonctionnalités complètes
RUN cargo build --release --bin enedis --features cli,api,storage-sqlite,mqtt,agent

# Stage 2: Image finale légère
FROM ${BUILD_FROM}

ENV LANG=C.UTF-8 \
    RUST_LOG=info

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    jq \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/enedis-rs/target/release/enedis /usr/local/bin/enedis
COPY run.sh /run.sh
RUN chmod +x /run.sh

EXPOSE 8080

CMD [ "/run.sh" ]

# ==============================================================================
# Multi-stage Dockerfile pour enedis-rs
# Compatible architectures amd64 et arm64 (Raspberry Pi 4 / 5, Apple Silicon, etc.)
# ==============================================================================

# Étape 1 : Construction du binaire Rust
FROM rust:1-slim-bookworm AS builder

WORKDIR /usr/src/enedis-rs

# Dépendances de compilation C/Perl pour openssl-src (vendored)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    perl \
    make \
    gcc \
    libc6-dev \
    && rm -rf /var/lib/apt/lists/*

# Mise en cache des dépendances Cargo
COPY Cargo.toml Cargo.lock ./

# Création d'un squelette de code pour pré-compiler les dépendances
RUN mkdir -p src/bin tests && \
    echo "fn main() {}" > src/bin/enedis.rs && \
    echo "pub fn dummy() {}" > src/lib.rs && \
    cargo build --release --bin enedis --features "cli,agent,api,mock-sge,storage-postgres,storage-sqlite,mqtt,parquet,analytics" && \
    rm -rf src target/release/deps/enedis* target/release/enedis*

# Copie des véritables sources du projet
COPY src ./src

# Compilation du binaire de production optimisé
RUN cargo build --release --bin enedis --features "cli,agent,api,mock-sge,storage-postgres,storage-sqlite,mqtt,parquet,analytics"

# Étape 2 : Image d'exécution minimale et sécurisée
FROM debian:bookworm-slim AS runtime

# Certificats racines nécessaires pour mTLS et connexions TLS vers Enedis
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Utilisateur système non-privilégié
RUN groupadd -g 10001 enedis && \
    useradd -u 10001 -g enedis -m -d /home/enedis -s /bin/false enedis

# Répertoire de données / certificats
RUN mkdir -p /etc/enedis /data && \
    chown -R enedis:enedis /home/enedis /etc/enedis /data

# Copie du binaire compilé depuis l'étape de build
COPY --from=builder /usr/src/enedis-rs/target/release/enedis /usr/local/bin/enedis

USER enedis
WORKDIR /home/enedis

# 8080 = API HTTP / Métriques Prometheus
# 9090 = Simulateur Mock SGE (si activé)
EXPOSE 8080 9090

# Healthcheck natif interrogeant /health
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/enedis"]
CMD ["serve"]

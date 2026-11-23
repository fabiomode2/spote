# syntax=docker/dockerfile:1
#
# Imagen de `spoted`: solo el backend social. El audio, yt-dlp y la TUI no
# entran aquí. El contenedor escucha en 0.0.0.0:7070 y espera detrás de nginx.

FROM rust:1.92-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p spote-server --bin spoted

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl tini passwd sqlite3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 spote \
    && useradd --system --uid 10001 --gid spote \
        --home-dir /nonexistent --shell /usr/sbin/nologin spote

COPY --from=builder /src/target/release/spoted /usr/local/bin/spoted
RUN install -d -o 10001 -g 10001 /data

ENV SPOTE_BIND=0.0.0.0:7070 \
    SPOTE_DB=/data/spote-server.db \
    RUST_LOG=info,spote_server=info,tower_http=warn

USER 10001:10001
EXPOSE 7070
STOPSIGNAL SIGTERM

HEALTHCHECK --interval=30s --timeout=4s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:7070/healthz || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/spoted"]

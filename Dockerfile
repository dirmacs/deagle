# syntax=docker/dockerfile:1
#
# deagle — code-intelligence HTTP + MCP server.
#
# Multi-stage build:
#   1. builder: compiles the `deagle-serve` binary. Needs a C toolchain
#      because rusqlite (bundled SQLite) and tree-sitter compile C sources.
#   2. runtime: distroless-style slim image; the binary is statically linked
#      against the bundled SQLite/tree-sitter, so no runtime C deps are needed.
#
# Build:  docker build -t deagle .
# Run:    docker run --rm -p 3500:3500 -v "$PWD:/data" deagle
#         # then POST /api/map to index /data, GET /api/stats to inspect it.
#
# Runtime configuration (all optional):
#   DEAGLE_PORT  listen port (default 3500)
#   DEAGLE_ROOT  directory the server indexes/serves (default /data)
#   DEAGLE_DB    path to the graph database (default /data/.deagle/graph.db)

# ---- builder ---------------------------------------------------------------
FROM rust:1.98-bookworm AS builder

WORKDIR /build

# Copy the workspace manifests and source. This repo intentionally does NOT
# commit Cargo.lock (it is gitignored), so the build resolves dependencies
# fresh; do not pass --locked and do not COPY a lockfile that is not tracked.
COPY Cargo.toml ./
COPY crates ./crates

# Build only the server binary in release mode. tree-sitter + bundled sqlite
# require cc/pkg-config, which the rust image already provides.
RUN cargo build --release -p deagle-server --bin deagle-serve

# ---- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# tini gives a proper PID 1 so the server reaps children and handles signals.
RUN apt-get update \
 && apt-get install -y --no-install-recommends tini ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user.
RUN useradd --system --uid 10001 --create-home deagle

COPY --from=builder /build/target/release/deagle-serve /usr/local/bin/deagle-serve

# /data is the default index/serving root and the graph DB location.
# Mount the repository you want to query here.
RUN mkdir -p /data && chown deagle:deagle /data
WORKDIR /data

USER deagle

ENV DEAGLE_PORT=3500 \
    DEAGLE_ROOT=/data \
    DEAGLE_DB=/data/.deagle/graph.db

EXPOSE 3500

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/deagle-serve"]

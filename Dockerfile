# syntax=docker/dockerfile:1.7

FROM rust:1.98-bookworm AS builder

WORKDIR /app

COPY Cargo.toml ./
COPY crates/ ./crates/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin deagle --bin deagle-serve && \
    cp /app/target/release/deagle /tmp/deagle && \
    cp /app/target/release/deagle-serve /tmp/deagle-serve

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends tini ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --create-home deagle

WORKDIR /data

COPY --from=builder /tmp/deagle /usr/local/bin/deagle
COPY --from=builder /tmp/deagle-serve /usr/local/bin/deagle-serve

RUN mkdir -p /data && chown deagle:deagle /data

USER deagle

ENV RUST_LOG=info
ENV DEAGLE_PORT=3500
ENV DEAGLE_ROOT=/data
ENV DEAGLE_DB=/data/.deagle/graph.db

EXPOSE 3500

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/deagle-serve"]

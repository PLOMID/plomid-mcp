# © 2026 PLOMID Technologies INDIA
# Licensed under the Apache License, Version 2.0.

FROM rust:1.99-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY src ./src
RUN cargo build --release --locked --bin plomid-mcp

FROM debian:bookworm-slim
RUN set -eux; \
    groupadd --system --gid 10001 plomid; \
    useradd --system --uid 10001 --gid plomid --home-dir /nonexistent \
      --shell /usr/sbin/nologin plomid
COPY --from=builder /build/target/release/plomid-mcp /usr/local/bin/plomid-mcp
RUN chmod 0755 /usr/local/bin/plomid-mcp
USER plomid:plomid
ENTRYPOINT ["/usr/local/bin/plomid-mcp"]

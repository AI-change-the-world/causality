# syntax=docker/dockerfile:1.7

FROM rust:1.92-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
ENV CARGO_TARGET_DIR=/app/target

COPY Cargo.toml Cargo.lock* ./

RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn dummy() {}" > src/lib.rs

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release

RUN rm -f target/release/causality

COPY src ./src
COPY migrations ./migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release && \
    strip target/release/causality


FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false -u 1000 causality

WORKDIR /app

COPY --from=builder /app/target/release/causality /app/causality
COPY --from=builder /app/migrations /app/migrations

RUN mkdir -p /app/config && chown -R causality:causality /app

USER causality

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=10s --retries=3 \
    CMD /app/causality --healthcheck || exit 1

ENV RUST_LOG=info
ENV CONFIG_PATH=/app/config/config.yaml

CMD ["/app/causality"]

# Memory Server Dockerfile
# Multi-stage build for minimal image size

# ============================================
# Stage 1: Build environment
# ============================================
FROM rust:1.75-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create a new empty shell project
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock* ./

# Create dummy source to cache dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn dummy() {}" > src/lib.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release && \
    rm -rf src target/release/deps/memory_server*

# Copy actual source code
COPY src ./src
COPY migrations ./migrations

# Build the actual application
RUN cargo build --release

# ============================================
# Stage 2: Runtime environment
# ============================================
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -r -s /bin/false -u 1000 memory-server

# Create app directory
WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/memory-server /app/memory-server

# Copy migrations (needed for runtime migration execution)
COPY --from=builder /app/migrations /app/migrations

# Create config directory
RUN mkdir -p /app/config && chown -R memory-server:memory-server /app

# Switch to non-root user
USER memory-server

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Set environment variables
ENV RUST_LOG=info
ENV CONFIG_PATH=/app/config/config.yaml

# Run the binary
CMD ["/app/memory-server"]

# syntax=docker/dockerfile:1

# ============================================================================
# Stage 1: Build the application
# ============================================================================
FROM rust:1.92-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    libclang-dev \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY bin/maestro/Cargo.toml bin/maestro/
COPY crates/core/Cargo.toml crates/core/
COPY crates/substrate/Cargo.toml crates/substrate/
COPY crates/storage/Cargo.toml crates/storage/
COPY crates/graphql/Cargo.toml crates/graphql/
COPY crates/handlers/Cargo.toml crates/handlers/

# Create dummy source files to build dependencies
RUN mkdir -p bin/maestro/src crates/core/src crates/substrate/src \
    crates/storage/src crates/graphql/src crates/handlers/src \
    && echo "fn main() {}" > bin/maestro/src/main.rs \
    && echo "pub fn dummy() {}" > crates/core/src/lib.rs \
    && echo "pub fn dummy() {}" > crates/substrate/src/lib.rs \
    && echo "pub fn dummy() {}" > crates/storage/src/lib.rs \
    && echo "pub fn dummy() {}" > crates/graphql/src/lib.rs \
    && echo "pub fn dummy() {}" > crates/handlers/src/lib.rs

# Build dependencies only (cached layer)
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin maestro && rm -rf target/release/.fingerprint/maestro*

# Copy actual source code
COPY bin/ bin/
COPY crates/ crates/

# Build the actual application
RUN cargo build --release --bin maestro

# ============================================================================
# Stage 2: Runtime image
# ============================================================================
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -s /bin/false maestro

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/maestro /usr/local/bin/maestro

# Use non-root user
USER maestro

# Expose ports
# GraphQL API
EXPOSE 4000
# Prometheus metrics
EXPOSE 9090

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/maestro", "--help"]

# Default entrypoint
ENTRYPOINT ["/usr/local/bin/maestro"]

# Default arguments (can be overridden)
CMD ["--log-level", "info"]

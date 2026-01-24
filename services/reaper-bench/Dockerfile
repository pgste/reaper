# Build stage
FROM rust:1.83-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY services/ services/
COPY tools/ tools/
COPY tests/ tests/

# Build the benchmark service
RUN cargo build --release -p reaper-bench

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary
COPY --from=builder /build/target/release/reaper-bench /app/reaper-bench

# Copy policies
COPY services/reaper-bench/policies /app/policies

# Default port
EXPOSE 3000

# Set environment
ENV RUST_LOG=info

# Run the benchmark service
ENTRYPOINT ["/app/reaper-bench"]

# =============================================================================
# Reaper Platform Dockerfile
# =============================================================================
# Multi-stage build for minimal production image
#
# Build: docker build -t reaper-platform -f services/reaper-platform/Dockerfile .
# Run:   docker run -p 8081:8081 reaper-platform
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Build
# -----------------------------------------------------------------------------
FROM rust:1.83-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY services/ services/
COPY tools/ tools/
COPY tests/ tests/

# Build only the platform service in release mode
RUN cargo build --release -p reaper-platform

# -----------------------------------------------------------------------------
# Stage 2: Runtime
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user (UID 1000)
RUN useradd -r -u 1000 -s /bin/false reaper

# Create directories
RUN mkdir -p /data && \
    chown -R reaper:reaper /data

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/reaper-platform /app/reaper-platform

# Set ownership
RUN chown -R reaper:reaper /app

# Switch to non-root user
USER reaper

# Expose port
EXPOSE 8081

# Health check
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --quiet --tries=1 --spider http://localhost:8081/health || exit 1

# Default environment variables
ENV RUST_LOG=info,reaper_platform=info
ENV REAPER_BIND_ADDR=0.0.0.0:8081

# Labels for metadata
LABEL org.opencontainers.image.title="Reaper Platform"
LABEL org.opencontainers.image.description="Reaper policy management platform"
LABEL org.opencontainers.image.source="https://github.com/reaper-policy/reaper"
LABEL com.reaper.service="platform"

# Run the platform
ENTRYPOINT ["/app/reaper-platform"]

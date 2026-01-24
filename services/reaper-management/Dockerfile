# =============================================================================
# Reaper Management Server Dockerfile
# =============================================================================
# Multi-stage build for minimal production image
#
# Build: docker build -t reaper-management -f services/reaper-management/Dockerfile .
# Run:   docker run -p 3000:3000 reaper-management
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Build
# -----------------------------------------------------------------------------
FROM rust:1.83-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY services/ services/
COPY tools/ tools/

# Build only the management service in release mode
RUN cargo build --release -p reaper-management

# -----------------------------------------------------------------------------
# Stage 2: Runtime
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libpq5 \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false reaper

# Create directories
RUN mkdir -p /var/lib/reaper/storage && \
    chown -R reaper:reaper /var/lib/reaper

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/reaper-management /app/reaper-management

# Set ownership
RUN chown -R reaper:reaper /app

# Switch to non-root user
USER reaper

# Expose port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --quiet --tries=1 --spider http://localhost:3000/health || exit 1

# Default environment variables
ENV RUST_LOG=info,reaper_management=info
ENV REAPER_PORT=3000
ENV REAPER_BIND_ADDRESS=0.0.0.0
ENV REAPER_STORAGE_TYPE=filesystem
ENV REAPER_STORAGE_PATH=/var/lib/reaper/storage

# Labels for metadata
LABEL org.opencontainers.image.title="Reaper Management"
LABEL org.opencontainers.image.description="Reaper centralized policy management server"
LABEL org.opencontainers.image.source="https://github.com/reaper-policy/reaper"
LABEL com.reaper.service="management"

# Run the server
CMD ["/app/reaper-management"]

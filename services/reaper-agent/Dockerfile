# =============================================================================
# Reaper Agent Dockerfile
# =============================================================================
# Multi-stage build for minimal production image
#
# Build: docker build -t reaper-agent -f services/reaper-agent/Dockerfile .
# Run:   docker run -p 8080:8080 reaper-agent
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

# Build only the agent service in release mode
RUN cargo build --release -p reaper-agent

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
RUN mkdir -p /data /policies /var/log/reaper && \
    chown -R reaper:reaper /data /policies /var/log/reaper

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/reaper-agent /app/reaper-agent

# Set ownership
RUN chown -R reaper:reaper /app

# Switch to non-root user
USER reaper

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD wget --quiet --tries=1 --spider http://localhost:8080/health || exit 1

# Default environment variables
ENV RUST_LOG=info,reaper_agent=info
ENV REAPER_BIND_ADDR=0.0.0.0:8080

# Labels for metadata
LABEL org.opencontainers.image.title="Reaper Agent"
LABEL org.opencontainers.image.description="High-performance policy enforcement agent"
LABEL org.opencontainers.image.source="https://github.com/reaper-policy/reaper"
LABEL com.reaper.service="agent"

# Run the agent
ENTRYPOINT ["/app/reaper-agent"]

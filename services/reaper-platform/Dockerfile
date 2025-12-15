# syntax=docker/dockerfile:1

# Multi-stage build for minimal final image size
FROM rust:1.75-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev

# Create working directory
WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY services/ ./services/
COPY tools/ ./tools/

# Build release binary
RUN cargo build --release --bin reaper-platform

# Runtime stage - minimal image
FROM alpine:3.19

# Install runtime dependencies
RUN apk add --no-cache ca-certificates wget

# Create non-root user
RUN adduser -D -u 1000 reaper

# Copy binary from builder
COPY --from=builder /build/target/release/reaper-platform /usr/local/bin/reaper-platform

# Set ownership
RUN chown reaper:reaper /usr/local/bin/reaper-platform

# Create data directory
RUN mkdir -p /data && chown -R reaper:reaper /data

# Switch to non-root user
USER reaper

# Expose port
EXPOSE 8081

# Health check
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD wget --quiet --tries=1 --spider http://localhost:8081/health || exit 1

# Set working directory
WORKDIR /app

# Run the platform
ENTRYPOINT ["/usr/local/bin/reaper-platform"]

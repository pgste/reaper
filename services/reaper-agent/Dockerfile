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
RUN cargo build --release --bin reaper-agent

# Runtime stage - minimal image
FROM alpine:3.19

# Install runtime dependencies
RUN apk add --no-cache ca-certificates wget

# Create non-root user
RUN adduser -D -u 1000 reaper

# Copy binary from builder
COPY --from=builder /build/target/release/reaper-agent /usr/local/bin/reaper-agent

# Set ownership
RUN chown reaper:reaper /usr/local/bin/reaper-agent

# Create data directory
RUN mkdir -p /data /policies && chown -R reaper:reaper /data /policies

# Switch to non-root user
USER reaper

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD wget --quiet --tries=1 --spider http://localhost:8080/health || exit 1

# Set working directory
WORKDIR /app

# Run the agent
ENTRYPOINT ["/usr/local/bin/reaper-agent"]

#!/bin/bash
# =============================================================================
# Reaper E2E Test Runner
# =============================================================================
# This script starts the full Reaper stack and runs end-to-end tests.
#
# Usage:
#   ./run-e2e-tests.sh           # Run with Docker Compose
#   ./run-e2e-tests.sh --local   # Run against local services
# =============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

echo_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

echo_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if running locally or with Docker
USE_DOCKER=true
if [[ "$1" == "--local" ]]; then
    USE_DOCKER=false
fi

# Wait for a service to be healthy
wait_for_service() {
    local url=$1
    local name=$2
    local max_attempts=30
    local attempt=1

    echo_info "Waiting for $name to be healthy at $url..."

    while [[ $attempt -le $max_attempts ]]; do
        if curl -s -o /dev/null -w "%{http_code}" "$url/health" | grep -q "200"; then
            echo_info "$name is healthy!"
            return 0
        fi
        echo "  Attempt $attempt/$max_attempts..."
        sleep 2
        ((attempt++))
    done

    echo_error "$name failed to become healthy after $max_attempts attempts"
    return 1
}

cleanup() {
    if [[ "$USE_DOCKER" == "true" ]]; then
        echo_info "Stopping Docker Compose services..."
        cd "$PROJECT_ROOT"
        docker compose -f docker-compose.full.yml --profile managed down -v 2>/dev/null || true
    fi
}

# Trap cleanup on exit
trap cleanup EXIT

main() {
    cd "$PROJECT_ROOT"

    if [[ "$USE_DOCKER" == "true" ]]; then
        echo_info "Starting Reaper stack with Docker Compose..."

        # Build images
        echo_info "Building Docker images..."
        docker compose -f docker-compose.full.yml build

        # Start services
        echo_info "Starting services..."
        docker compose -f docker-compose.full.yml --profile managed up -d

        # Wait for services
        wait_for_service "http://localhost:3000" "Management Server"
        wait_for_service "http://localhost:8082" "Agent (Managed)"

        # Export URLs for tests
        export REAPER_MANAGEMENT_URL="http://localhost:3000"
        export REAPER_AGENT_URL="http://localhost:8082"
    else
        echo_info "Running against local services..."
        export REAPER_MANAGEMENT_URL="${REAPER_MANAGEMENT_URL:-http://localhost:3000}"
        export REAPER_AGENT_URL="${REAPER_AGENT_URL:-http://localhost:8082}"

        # Check if services are running
        if ! wait_for_service "$REAPER_MANAGEMENT_URL" "Management Server"; then
            echo_error "Management server not running. Start with: cargo run -p reaper-management"
            exit 1
        fi

        if ! wait_for_service "$REAPER_AGENT_URL" "Agent"; then
            echo_warn "Agent not running. Some tests may be skipped."
        fi
    fi

    echo ""
    echo_info "Running E2E tests..."
    echo ""

    # Run the tests
    cargo test -p reaper-e2e-tests --test e2e_tests -- --test-threads=1 --nocapture

    echo ""
    echo_info "E2E tests completed successfully!"
}

main "$@"

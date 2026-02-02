#!/bin/bash
# Reaper vs OPA Comparison Test Runner
#
# This script:
# 1. Starts Reaper Agent and OPA in Docker
# 2. Deploys policies and data to both
# 3. Runs comparison tests
# 4. Reports results
#
# Usage:
#   ./run-comparison.sh              # Run full comparison
#   ./run-comparison.sh --reaper     # Run Reaper-only tests
#   ./run-comparison.sh --cleanup    # Stop containers and clean up

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

REAPER_URL="${REAPER_URL:-http://localhost:8080}"
OPA_URL="${OPA_URL:-http://localhost:8181}"

# Parse arguments
REAPER_ONLY=false
CLEANUP=false
SKIP_DOCKER=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --reaper|--reaper-only)
            REAPER_ONLY=true
            shift
            ;;
        --cleanup)
            CLEANUP=true
            shift
            ;;
        --skip-docker)
            SKIP_DOCKER=true
            shift
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Cleanup function
cleanup() {
    log_info "Stopping containers..."
    cd "$SCRIPT_DIR"
    docker compose down --remove-orphans 2>/dev/null || true
    log_success "Containers stopped"
}

if [ "$CLEANUP" = true ]; then
    cleanup
    exit 0
fi

# Copy test data to OPA data directory
setup_opa_data() {
    log_info "Setting up OPA data..."

    # Copy RBAC test data
    if [ -f "$WORKSPACE_ROOT/test-data/rbac-test-data.json" ]; then
        cp "$WORKSPACE_ROOT/test-data/rbac-test-data.json" "$SCRIPT_DIR/opa_data/rbac.json"
    fi

    # Copy string test data
    if [ -f "$WORKSPACE_ROOT/test-data/string-test-data.json" ]; then
        cp "$WORKSPACE_ROOT/test-data/string-test-data.json" "$SCRIPT_DIR/opa_data/string.json"
    fi

    log_success "OPA data setup complete"
}

# Start services
start_services() {
    log_info "Starting Docker services..."
    cd "$SCRIPT_DIR"

    # Build and start
    docker compose up -d --build

    # Wait for services to be healthy
    log_info "Waiting for services to be healthy..."

    local max_wait=60
    local waited=0

    while [ $waited -lt $max_wait ]; do
        local reaper_healthy=$(docker inspect --format='{{.State.Health.Status}}' reaper-comparison 2>/dev/null || echo "none")
        local opa_healthy=$(docker inspect --format='{{.State.Health.Status}}' opa-comparison 2>/dev/null || echo "none")

        if [ "$reaper_healthy" = "healthy" ] && [ "$opa_healthy" = "healthy" ]; then
            log_success "All services healthy"
            return 0
        fi

        sleep 2
        waited=$((waited + 2))
        echo -n "."
    done

    echo ""
    log_error "Services did not become healthy in time"
    docker compose logs
    return 1
}

# Deploy policy to Reaper
deploy_reaper_policy() {
    local policy_file="$1"
    local policy_name="$2"

    log_info "Deploying $policy_name to Reaper..."

    if [ ! -f "$policy_file" ]; then
        log_warn "Policy file not found: $policy_file"
        return 1
    fi

    # Read policy content
    local policy_content=$(cat "$policy_file")

    # Deploy via Reaper API
    local response=$(curl -s -X POST "$REAPER_URL/api/v1/policies/deploy" \
        -H "Content-Type: application/json" \
        -d "{
            \"name\": \"$policy_name\",
            \"content\": $(echo "$policy_content" | jq -Rs .)
        }")

    if echo "$response" | grep -q "error"; then
        log_error "Failed to deploy policy: $response"
        return 1
    fi

    log_success "Deployed $policy_name to Reaper"
}

# Load data into Reaper
load_reaper_data() {
    local data_file="$1"

    log_info "Loading data into Reaper..."

    if [ ! -f "$data_file" ]; then
        log_warn "Data file not found: $data_file"
        return 1
    fi

    local response=$(curl -s -X POST "$REAPER_URL/api/v1/data/load" \
        -H "Content-Type: application/json" \
        -d @"$data_file")

    log_success "Loaded data into Reaper"
}

# Run comparison test
run_comparison_test() {
    local test_id="$1"
    local principal="$2"
    local action="$3"
    local resource="$4"
    local expected="$5"
    local opa_package="$6"

    # Query Reaper
    local reaper_start=$(date +%s%N)
    local reaper_response=$(curl -s -X POST "$REAPER_URL/api/v1/evaluate" \
        -H "Content-Type: application/json" \
        -d "{
            \"principal\": \"$principal\",
            \"action\": \"$action\",
            \"resource\": \"$resource\"
        }")
    local reaper_end=$(date +%s%N)
    local reaper_time_us=$(( (reaper_end - reaper_start) / 1000 ))

    local reaper_decision=$(echo "$reaper_response" | jq -r '.decision // .allowed // "deny"' | tr '[:upper:]' '[:lower:]')
    if [ "$reaper_decision" = "true" ]; then reaper_decision="allow"; fi
    if [ "$reaper_decision" = "false" ]; then reaper_decision="deny"; fi

    local opa_decision="skipped"
    local opa_time_us=0

    if [ "$REAPER_ONLY" = false ]; then
        # Query OPA
        local opa_start=$(date +%s%N)
        local opa_response=$(curl -s -X POST "$OPA_URL/v1/data/$opa_package/allow" \
            -H "Content-Type: application/json" \
            -d "{
                \"input\": {
                    \"principal\": \"$principal\",
                    \"action\": \"$action\",
                    \"resource\": \"$resource\"
                }
            }")
        local opa_end=$(date +%s%N)
        opa_time_us=$(( (opa_end - opa_start) / 1000 ))

        local opa_result=$(echo "$opa_response" | jq -r '.result // false')
        if [ "$opa_result" = "true" ]; then
            opa_decision="allow"
        else
            opa_decision="deny"
        fi
    fi

    # Check results
    local status="PASS"
    if [ "$reaper_decision" != "$expected" ]; then
        status="FAIL"
    fi
    if [ "$REAPER_ONLY" = false ] && [ "$opa_decision" != "$expected" ]; then
        status="FAIL"
    fi

    # Print result
    if [ "$status" = "PASS" ]; then
        echo -e "${GREEN}✓${NC} $test_id: Reaper=$reaper_decision (${reaper_time_us}µs), OPA=$opa_decision (${opa_time_us}µs), expected=$expected"
    else
        echo -e "${RED}✗${NC} $test_id: Reaper=$reaper_decision (${reaper_time_us}µs), OPA=$opa_decision (${opa_time_us}µs), expected=$expected"
    fi

    [ "$status" = "PASS" ]
}

# Run RBAC comparison tests
run_rbac_tests() {
    log_info "Running RBAC comparison tests..."
    echo ""

    local passed=0
    local failed=0

    # Admin tests
    run_comparison_test "admin_read" "user_0" "read" "resource_100" "allow" "rbac" && ((passed++)) || ((failed++))
    run_comparison_test "admin_write" "user_0" "write" "resource_200" "allow" "rbac" && ((passed++)) || ((failed++))
    run_comparison_test "admin_delete" "user_0" "delete" "resource_500" "allow" "rbac" && ((passed++)) || ((failed++))

    # Owner tests
    run_comparison_test "owner_read" "user_50" "read" "resource_50" "allow" "rbac" && ((passed++)) || ((failed++))
    run_comparison_test "owner_other" "user_50" "read" "resource_51" "allow" "rbac" && ((passed++)) || ((failed++))

    # Viewer tests
    run_comparison_test "viewer_read" "user_700" "read" "resource_900" "allow" "rbac" && ((passed++)) || ((failed++))

    echo ""
    log_info "RBAC Results: $passed passed, $failed failed"
    return $failed
}

# Run string operations comparison tests
run_string_tests() {
    log_info "Running String Operations comparison tests..."
    echo ""

    local passed=0
    local failed=0

    # Case insensitive
    run_comparison_test "case_insensitive" "user_mixed_case" "read" "case_insensitive" "allow" "string_ops" && ((passed++)) || ((failed++))

    # Uppercase code
    run_comparison_test "uppercase_code" "user_uppercase_code" "read" "code_entry" "allow" "string_ops" && ((passed++)) || ((failed++))

    # Trimmed role
    run_comparison_test "trimmed_role_allow" "user_whitespace_role" "read" "trimmed_check" "allow" "string_ops" && ((passed++)) || ((failed++))
    run_comparison_test "trimmed_role_deny" "user_wrong_role" "read" "trimmed_check" "deny" "string_ops" && ((passed++)) || ((failed++))

    # Contains
    run_comparison_test "email_contains" "user_email_contains_company" "view" "internal_docs" "allow" "string_ops" && ((passed++)) || ((failed++))
    run_comparison_test "email_no_contains" "user_external_email" "view" "internal_docs" "deny" "string_ops" && ((passed++)) || ((failed++))

    # Startswith
    run_comparison_test "username_prefix" "user_admin_username" "configure" "system_settings" "allow" "string_ops" && ((passed++)) || ((failed++))
    run_comparison_test "no_username_prefix" "user_regular_username" "configure" "system_settings" "deny" "string_ops" && ((passed++)) || ((failed++))

    # Endswith
    run_comparison_test "gov_email" "user_gov_email" "access" "classified_docs" "allow" "string_ops" && ((passed++)) || ((failed++))
    run_comparison_test "commercial_email" "user_commercial_email" "access" "classified_docs" "deny" "string_ops" && ((passed++)) || ((failed++))

    # Split
    run_comparison_test "full_name" "user_full_name" "read" "profile" "allow" "string_ops" && ((passed++)) || ((failed++))
    run_comparison_test "single_name" "user_single_name" "read" "profile" "deny" "string_ops" && ((passed++)) || ((failed++))

    # Complex
    run_comparison_test "complex_email" "user_complex_email" "validate" "email_check" "allow" "string_ops" && ((passed++)) || ((failed++))

    echo ""
    log_info "String Operations Results: $passed passed, $failed failed"
    return $failed
}

# Main
main() {
    echo ""
    echo "======================================"
    echo "  Reaper vs OPA Comparison Tests"
    echo "======================================"
    echo ""

    if [ "$REAPER_ONLY" = true ]; then
        log_info "Running in Reaper-only mode"
    fi

    # Setup
    setup_opa_data

    if [ "$SKIP_DOCKER" = false ]; then
        start_services || exit 1
    fi

    echo ""

    # Deploy policies and data to Reaper
    deploy_reaper_policy "$WORKSPACE_ROOT/crates/policy-engine/examples/policies/rbac.reap" "rbac" || true
    deploy_reaper_policy "$WORKSPACE_ROOT/crates/policy-engine/examples/policies/string_policy.reap" "string_ops" || true
    load_reaper_data "$WORKSPACE_ROOT/test-data/rbac-test-data.json" || true
    load_reaper_data "$WORKSPACE_ROOT/test-data/string-test-data.json" || true

    echo ""

    # Run tests
    local total_failed=0

    run_rbac_tests || total_failed=$((total_failed + $?))
    echo ""
    run_string_tests || total_failed=$((total_failed + $?))

    echo ""
    echo "======================================"
    if [ $total_failed -eq 0 ]; then
        log_success "All comparison tests passed!"
    else
        log_error "$total_failed test(s) failed"
    fi
    echo "======================================"

    return $total_failed
}

main "$@"

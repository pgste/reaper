#!/bin/bash
# Reaper vs OPA Fair Comparison Benchmark
# Single entry point for all benchmark operations

set -e
cd "$(dirname "$0")/.."

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# Default values
SCENARIO="multilayer"
SCALE="10k"
REQUESTS=50000
CONCURRENCY=50
MODE="compare"  # compare, reaper-only, opa-only, all

# Show usage
usage() {
    cat << EOF
${BOLD}Reaper vs OPA Benchmark${NC}

Usage: $0 [OPTIONS]

OPTIONS:
    -s, --scenario SCENARIO     Policy scenario: rbac, abac, rebac, multilayer, all (default: multilayer)
    -n, --scale SCALE          Entity scale: 10k, 100k, both (default: 10k)
    -r, --requests NUM         Number of requests (default: 50000)
    -c, --concurrency NUM      Concurrent requests (default: 50)
    -m, --mode MODE            Run mode: compare, reaper-only, opa-only, all (default: compare)
    -h, --help                 Show this help

EXAMPLES:
    # Run multilayer scenario with 10K entities
    $0 --scenario multilayer --scale 10k

    # Run all scenarios with both 10K and 100K entities
    $0 --scenario all --scale both

    # Run just RBAC with 100K entities
    $0 --scenario rbac --scale 100k --requests 100000

    # Run comprehensive test (all scenarios, all scales)
    $0 --mode all

EOF
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -s|--scenario) SCENARIO="$2"; shift 2 ;;
        -n|--scale) SCALE="$2"; shift 2 ;;
        -r|--requests) REQUESTS="$2"; shift 2 ;;
        -c|--concurrency) CONCURRENCY="$2"; shift 2 ;;
        -m|--mode) MODE="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

# Validate inputs
validate_scenario() {
    local s=$1
    if [[ ! "$s" =~ ^(rbac|abac|rebac|multilayer|all)$ ]]; then
        echo -e "${RED}✗ Invalid scenario: $s${NC}"
        echo "Valid: rbac, abac, rebac, multilayer, all"
        exit 1
    fi
}

validate_scale() {
    local s=$1
    if [[ ! "$s" =~ ^(10k|100k|both)$ ]]; then
        echo -e "${RED}✗ Invalid scale: $s${NC}"
        echo "Valid: 10k, 100k, both"
        exit 1
    fi
}

# Run a single benchmark
run_benchmark() {
    local scenario=$1
    local scale=$2
    local requests=$3

    echo ""
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}  Benchmark: ${scenario} @ ${scale}${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""

    # Deploy to both systems
    ./bin/deploy-reaper.sh "$scenario" "$scale"
    ./bin/deploy-opa.sh "$scenario" "$scale"

    # Get process info
    REAPER_PID=$(pgrep -f "reaper-agent" | head -1)
    OPA_PID=$(pgrep -f "opa.*run.*server" | head -1)

    if [ -z "$REAPER_PID" ] || [ -z "$OPA_PID" ]; then
        echo -e "${RED}✗ Services not running${NC}"
        exit 1
    fi

    # Track memory - write to temp file so it's available after subprocess exits
    MEM_FILE=$(mktemp)
    echo "0 0" > "$MEM_FILE"

    (
        REAPER_MEM_PEAK=0
        OPA_MEM_PEAK=0
        while true; do
            R=$(ps -o rss= -p "$REAPER_PID" 2>/dev/null | awk '{print $1}' || echo "0")
            R_MB=$((R / 1024))
            [ "$R_MB" -gt "$REAPER_MEM_PEAK" ] && REAPER_MEM_PEAK=$R_MB

            O=$(ps -o rss= -p "$OPA_PID" 2>/dev/null | awk '{print $1}' || echo "0")
            O_MB=$((O / 1024))
            [ "$O_MB" -gt "$OPA_MEM_PEAK" ] && OPA_MEM_PEAK=$O_MB

            echo "$REAPER_MEM_PEAK $OPA_MEM_PEAK" > "$MEM_FILE"
            sleep 1
        done
    ) &
    MEM_TRACKER=$!

    # Run benchmark
    RESULTS_DIR="results/${scale}/${scenario}"
    mkdir -p "$RESULTS_DIR"

    cargo run --release -- \
        --reaper-url http://localhost:8080 \
        --opa-url http://localhost:8181 \
        --scenario "$scenario" \
        --requests "$requests" \
        --concurrency "$CONCURRENCY" \
        --save "$RESULTS_DIR/results.json"

    kill $MEM_TRACKER 2>/dev/null || true

    # Read memory stats from temp file
    read REAPER_MEM_PEAK OPA_MEM_PEAK < "$MEM_FILE"
    rm -f "$MEM_FILE"

    # Parse results
    REAPER_RPS=$(jq -r '.[0].throughput_rps' "$RESULTS_DIR/results.json" | awk '{printf "%.0f", $1}')
    REAPER_P99=$(jq -r '.[0].latency_p99_us' "$RESULTS_DIR/results.json" | awk '{printf "%.0f", $1}')
    REAPER_ALLOW=$(jq -r '.[0].allowed' "$RESULTS_DIR/results.json")
    REAPER_DENY=$(jq -r '.[0].denied' "$RESULTS_DIR/results.json")

    OPA_RPS=$(jq -r '.[1].throughput_rps' "$RESULTS_DIR/results.json" | awk '{printf "%.0f", $1}')
    OPA_P99=$(jq -r '.[1].latency_p99_us' "$RESULTS_DIR/results.json" | awk '{printf "%.0f", $1}')
    OPA_ALLOW=$(jq -r '.[1].allowed' "$RESULTS_DIR/results.json")
    OPA_DENY=$(jq -r '.[1].denied' "$RESULTS_DIR/results.json")

    # Calculate improvements
    PERF_IMPROVEMENT=$(awk "BEGIN {printf \"%.2f\", $REAPER_RPS / $OPA_RPS}")
    LATENCY_IMPROVEMENT=$(awk "BEGIN {printf \"%.2f\", $OPA_P99 / $REAPER_P99}")

    # Calculate memory ratio with safety check
    if [ "$OPA_MEM_PEAK" -gt 0 ]; then
        MEM_RATIO=$(awk "BEGIN {printf \"%.2f\", $REAPER_MEM_PEAK / $OPA_MEM_PEAK}")
    else
        MEM_RATIO="N/A"
    fi

    # Generate report
    cat > "$RESULTS_DIR/report.txt" << EOF
═══════════════════════════════════════════════════════════════
BENCHMARK RESULTS: ${scenario} @ ${scale}
═══════════════════════════════════════════════════════════════

PERFORMANCE
───────────────────────────────────────────────────────────────
                    Reaper          OPA             Improvement
Throughput:         ${REAPER_RPS} req/s     ${OPA_RPS} req/s       ${PERF_IMPROVEMENT}x
p99 Latency:        ${REAPER_P99}μs        ${OPA_P99}μs          ${LATENCY_IMPROVEMENT}x faster
Allow:              ${REAPER_ALLOW}         ${OPA_ALLOW}
Deny:               ${REAPER_DENY}          ${OPA_DENY}

MEMORY
───────────────────────────────────────────────────────────────
                    Reaper          OPA             Ratio
Peak Memory:        ${REAPER_MEM_PEAK} MB        ${OPA_MEM_PEAK} MB          ${MEM_RATIO}x

SUMMARY
───────────────────────────────────────────────────────────────
Reaper is ${PERF_IMPROVEMENT}x faster than OPA with ${MEM_RATIO}x memory usage
at ${scale} entity scale.

Generated: $(date)
EOF

    cat "$RESULTS_DIR/report.txt"

    echo ""
    echo -e "${GREEN}✅ Results saved to: $RESULTS_DIR/${NC}"
    echo ""
}

# Main execution
main() {
    echo -e "${BOLD}${BLUE}Reaper vs OPA Fair Comparison Benchmark${NC}"
    echo ""

    # Validate inputs
    validate_scenario "$SCENARIO"
    validate_scale "$SCALE"

    # Check services
    if ! curl -s http://localhost:8080/health > /dev/null 2>&1; then
        echo -e "${RED}✗ Reaper not running at localhost:8080${NC}"
        echo "Start it: cd /workspaces/reaper && ./target/release/reaper-agent"
        exit 1
    fi

    if ! curl -s http://localhost:8181/health > /dev/null 2>&1; then
        echo -e "${RED}✗ OPA not running at localhost:8181${NC}"
        echo "Start it: opa run --server --addr localhost:8181"
        exit 1
    fi

    echo -e "${GREEN}✓ Services running${NC}"
    echo ""

    # Determine which scenarios to run
    SCENARIOS=()
    if [ "$SCENARIO" = "all" ]; then
        SCENARIOS=("rbac" "abac" "rebac" "multilayer")
    else
        SCENARIOS=("$SCENARIO")
    fi

    # Determine which scales to run
    SCALES=()
    if [ "$SCALE" = "both" ]; then
        SCALES=("10k" "100k")
    else
        SCALES=("$SCALE")
    fi

    # Run benchmarks
    for scale in "${SCALES[@]}"; do
        for scenario in "${SCENARIOS[@]}"; do
            run_benchmark "$scenario" "$scale" "$REQUESTS"

            # Cleanup between runs
            ./bin/cleanup.sh
            sleep 3
        done
    done

    # Generate master summary if multiple tests
    if [ ${#SCENARIOS[@]} -gt 1 ] || [ ${#SCALES[@]} -gt 1 ]; then
        echo ""
        echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
        echo -e "${BOLD}${CYAN}  Master Summary${NC}"
        echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════${NC}"
        echo ""

        echo "Scenario       Scale   Reaper RPS  OPA RPS  Speedup  Reaper Mem  OPA Mem"
        echo "───────────────────────────────────────────────────────────────────────"

        for scale in "${SCALES[@]}"; do
            for scenario in "${SCENARIOS[@]}"; do
                RFILE="results/${scale}/${scenario}/results.json"
                if [ -f "$RFILE" ]; then
                    R_RPS=$(jq -r '.[0].throughput_rps' "$RFILE" | awk '{printf "%.0f", $1}')
                    O_RPS=$(jq -r '.[1].throughput_rps' "$RFILE" | awk '{printf "%.0f", $1}')
                    SPEEDUP=$(awk "BEGIN {printf \"%.2f\", $R_RPS / $O_RPS}")x

                    # Get memory from report
                    REPORT="results/${scale}/${scenario}/report.txt"
                    R_MEM=$(grep "Peak Memory:" "$REPORT" | awk '{print $3}')
                    O_MEM=$(grep "Peak Memory:" "$REPORT" | awk '{print $5}')

                    printf "%-14s %-7s %-11s %-8s %-8s %-11s %-8s\n" \
                        "$scenario" "$scale" "$R_RPS" "$O_RPS" "$SPEEDUP" "$R_MEM" "$O_MEM"
                fi
            done
        done

        echo ""
    fi

    echo -e "${BOLD}${GREEN}✅ Benchmark complete!${NC}"
    echo ""
}

main

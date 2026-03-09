#!/bin/bash
# Profiling script for Reaper Policy Engine
# Phase 7.3: Optimization & Flamegraph Profiling

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🔥 Reaper Policy Engine Profiler${NC}"
echo ""

# Check if flamegraph is installed
if ! command -v flamegraph &> /dev/null; then
    echo -e "${YELLOW}Installing flamegraph...${NC}"
    cargo install flamegraph
fi

# Profile mode
MODE=${1:-"e2e"}

case $MODE in
    "builtins")
        echo -e "${GREEN}Profiling built-in functions...${NC}"
        cargo flamegraph --bench builtins_bench -- --bench --profile-time=10
        ;;
    "caching")
        echo -e "${GREEN}Profiling regex caching...${NC}"
        cargo flamegraph --bench caching_bench -- --bench --profile-time=10
        ;;
    "simd")
        echo -e "${GREEN}Profiling SIMD aggregates...${NC}"
        cargo flamegraph --bench simd_bench -- --bench --profile-time=10
        ;;
    "e2e"|*)
        echo -e "${GREEN}Profiling end-to-end evaluation...${NC}"
        cargo flamegraph --bench e2e_bench -- --bench --profile-time=10
        ;;
esac

echo ""
echo -e "${GREEN}✅ Flamegraph generated: flamegraph.svg${NC}"
echo -e "${YELLOW}Open with: firefox flamegraph.svg${NC}"

#!/bin/bash
# Cleanup data and policies between benchmark runs

set -e

# Clear OPA data and policies
curl -s -X DELETE http://localhost:8181/v1/data/entities > /dev/null 2>&1 || true
curl -s -X DELETE http://localhost:8181/v1/policies/rbac > /dev/null 2>&1 || true
curl -s -X DELETE http://localhost:8181/v1/policies/abac > /dev/null 2>&1 || true
curl -s -X DELETE http://localhost:8181/v1/policies/rebac > /dev/null 2>&1 || true
curl -s -X DELETE http://localhost:8181/v1/policies/multilayer > /dev/null 2>&1 || true

# Restart Reaper for clean state
pkill -f "reaper-agent" || true
sleep 2

# Find reaper-agent binary (works in both codespace and CI)
if [ -f "./target/release/reaper-agent" ]; then
    REAPER_BIN="./target/release/reaper-agent"
elif [ -f "../../target/release/reaper-agent" ]; then
    REAPER_BIN="../../target/release/reaper-agent"
else
    echo "Error: reaper-agent binary not found"
    exit 1
fi

nohup $REAPER_BIN > /tmp/reaper-agent.log 2>&1 &
sleep 3

echo "✓ Cleanup complete"

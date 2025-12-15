#!/bin/bash
#
# Test Prometheus Metrics Endpoint
# This script demonstrates the decision streaming functionality
#

set -e

AGENT_URL="http://localhost:8080"
PLATFORM_URL="http://localhost:8081"

echo "🔍 Reaper Policy Engine - Prometheus Metrics Test"
echo "=================================================="
echo ""

# Function to check if service is running
check_service() {
    local url=$1
    local name=$2

    if curl -s -f "$url/health" > /dev/null 2>&1; then
        echo "✅ $name is running"
        return 0
    else
        echo "❌ $name is not running at $url"
        return 1
    fi
}

# Check Agent health
echo "Step 1: Checking Agent health..."
if ! check_service "$AGENT_URL" "Reaper Agent"; then
    echo ""
    echo "Please start the Agent first:"
    echo "  cargo run -p reaper-agent"
    exit 1
fi
echo ""

# Send some test policy evaluations to generate metrics
echo "Step 2: Generating policy decisions..."
echo ""

# Test 1: Allow decision
echo "📝 Test 1: Allowed access"
curl -s -X POST "$AGENT_URL/api/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "demo-allow-all",
    "resource": "/api/users/123",
    "action": "read"
  }' | jq -r '"Decision: \(.decision), Latency: \(.evaluation_time_microseconds)µs"'
echo ""

# Test 2: Another allow
echo "📝 Test 2: Allowed access to different resource"
curl -s -X POST "$AGENT_URL/api/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "demo-allow-all",
    "resource": "/api/documents/456",
    "action": "write"
  }' | jq -r '"Decision: \(.decision), Latency: \(.evaluation_time_microseconds)µs"'
echo ""

# Test 3: Test cache hit
echo "📝 Test 3: Cache hit test (same policy)"
curl -s -X POST "$AGENT_URL/api/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "demo-allow-all",
    "resource": "/api/secrets/789",
    "action": "delete"
  }' | jq -r '"Decision: \(.decision), Latency: \(.evaluation_time_microseconds)µs"'
echo ""

# Test 4: Policy not found (to trigger cache miss)
echo "📝 Test 4: Policy not found (triggers cache miss and error)"
curl -s -X POST "$AGENT_URL/api/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "policy_name": "non-existent-policy",
    "resource": "/api/data",
    "action": "read"
  }' | jq -r 'if .error then "Error: \(.error)" else "Decision: \(.decision)" end'
echo ""
echo ""

# Fetch Prometheus metrics
echo "Step 3: Fetching Prometheus metrics..."
echo "========================================"
echo ""

curl -s "$AGENT_URL/metrics" | grep -E "^(reaper_|# HELP|# TYPE)" | head -80

echo ""
echo ""
echo "🎯 Key Metrics to Watch:"
echo "========================"
echo ""
echo "Decision Counters:"
curl -s "$AGENT_URL/metrics" | grep "^reaper_decisions_total"
echo ""

echo "Decision Latency (seconds):"
curl -s "$AGENT_URL/metrics" | grep "^reaper_decision_duration_seconds" | head -5
echo ""

echo "Cache Performance:"
curl -s "$AGENT_URL/metrics" | grep "^reaper_cache_"
echo ""

echo "Active Policies:"
curl -s "$AGENT_URL/metrics" | grep "^reaper_active_policies"
echo ""

echo "Errors:"
curl -s "$AGENT_URL/metrics" | grep "^reaper_errors_total"
echo ""

echo ""
echo "✅ Test complete!"
echo ""
echo "📊 View metrics in Prometheus:"
echo "   http://localhost:9090/graph"
echo ""
echo "📈 View dashboards in Grafana:"
echo "   http://localhost:3000"
echo "   Default credentials: admin/admin"
echo ""
echo "🔍 Example PromQL queries:"
echo "   rate(reaper_decisions_total[5m])                     # Decisions per second"
echo "   histogram_quantile(0.99, rate(reaper_decision_duration_seconds_bucket[5m]))  # P99 latency"
echo "   rate(reaper_cache_hits_total[5m]) / rate(reaper_decisions_total[5m])        # Cache hit rate"
echo ""

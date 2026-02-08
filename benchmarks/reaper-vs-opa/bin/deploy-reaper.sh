#!/bin/bash
# Deploy policy and data to Reaper

set -e
cd "$(dirname "$0")/.."

SCENARIO=${1:?Scenario required}
SCALE=${2:?Scale required}

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

REAPER_URL=${REAPER_URL:-http://localhost:8080}
DATA_FILE="data/${SCALE}/${SCENARIO}.json"
POLICY_FILE="policies/reaper/${SCENARIO}.reap"

echo -e "${YELLOW}Deploying to Reaper: ${SCENARIO} @ ${SCALE}${NC}"

# Load entities
TEMP_PAYLOAD=$(mktemp)
python3 -c "import json, sys; data=json.load(open('$DATA_FILE')); print(json.dumps({'data': json.dumps(data)}))" > "$TEMP_PAYLOAD"

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${REAPER_URL}/api/v1/data" \
    -H "Content-Type: application/json" \
    --data @"$TEMP_PAYLOAD")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | head -n -1)

rm -f "$TEMP_PAYLOAD"

if [ "$HTTP_CODE" -ne 200 ]; then
    echo -e "${RED}✗ Data load failed (HTTP $HTTP_CODE): $BODY${NC}" >&2
    exit 1
fi

ENTITY_COUNT=$(jq '.entities | length' "$DATA_FILE")
echo -e "${GREEN}✓ Loaded ${ENTITY_COUNT} entities${NC}"

# Deploy policy
TEMP_POLICY=$(mktemp)
python3 -c "import json; print(json.dumps({'policy_content': open('$POLICY_FILE').read(), 'policy_name': '${SCENARIO}-policy'}))" > "$TEMP_POLICY"

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${REAPER_URL}/api/v1/policies/compile" \
    -H "Content-Type: application/json" \
    --data @"$TEMP_POLICY")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | head -n -1)

rm -f "$TEMP_POLICY"

if [ "$HTTP_CODE" -ne 200 ]; then
    echo -e "${RED}✗ Policy deploy failed (HTTP $HTTP_CODE): $BODY${NC}" >&2
    exit 1
fi

echo -e "${GREEN}✓ Policy deployed${NC}"

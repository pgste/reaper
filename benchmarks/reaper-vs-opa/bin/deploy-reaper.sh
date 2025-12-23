#!/bin/bash
# Deploy policy and data to Reaper

set -e
cd "$(dirname "$0")/.."

SCENARIO=${1:?Scenario required}
SCALE=${2:?Scale required}

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

REAPER_URL=${REAPER_URL:-http://localhost:8080}
DATA_FILE="data/${SCALE}/${SCENARIO}.json"
POLICY_FILE="policies/reaper/${SCENARIO}.reap"

echo -e "${YELLOW}Deploying to Reaper: ${SCENARIO} @ ${SCALE}${NC}"

# Load entities
TEMP_PAYLOAD=$(mktemp)
python3 -c "import json, sys; data=json.load(open('$DATA_FILE')); print(json.dumps({'data': json.dumps(data)}))" > "$TEMP_PAYLOAD"

curl -s -X POST "${REAPER_URL}/api/v1/data" \
    -H "Content-Type: application/json" \
    --data @"$TEMP_PAYLOAD" > /dev/null

rm -f "$TEMP_PAYLOAD"

ENTITY_COUNT=$(jq '.entities | length' "$DATA_FILE")
echo -e "${GREEN}✓ Loaded ${ENTITY_COUNT} entities${NC}"

# Deploy policy
TEMP_POLICY=$(mktemp)
python3 -c "import json; print(json.dumps({'policy_content': open('$POLICY_FILE').read(), 'policy_name': '${SCENARIO}-policy'}))" > "$TEMP_POLICY"

curl -s -X POST "${REAPER_URL}/api/v1/policies/compile" \
    -H "Content-Type: application/json" \
    --data @"$TEMP_POLICY" > /dev/null

rm -f "$TEMP_POLICY"

echo -e "${GREEN}✓ Policy deployed${NC}"

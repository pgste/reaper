#!/bin/bash
# Deploy policy and data to OPA

set -e
cd "$(dirname "$0")/.."

SCENARIO=${1:?Scenario required}
SCALE=${2:?Scale required}

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

OPA_URL=${OPA_URL:-http://localhost:8181}
DATA_FILE="data/${SCALE}/${SCENARIO}.json"
POLICY_FILE="policies/opa/${SCENARIO}.rego"

echo -e "${YELLOW}Deploying to OPA: ${SCENARIO} @ ${SCALE}${NC}"

# Transform entities array to a map for O(1) lookups (fair comparison with Reaper).
# The map VALUE is the entity's attributes object only — this mirrors Reaper's
# evaluation model exactly: Reaper resolves `user.<field>` / `resource.<field>`
# from the entity's attributes map (see Entity::get_attribute) and does NOT
# expose the top-level id/type as readable fields. The rego reads these values
# directly as `data.entities[<id>].<field>` (no `.attributes` indirection).
# From: {"entities": [{"id": "user1", "type": "user", "attributes": {...}}, ...]}
# To:   {"user1": {...attributes...}, ...}
ENTITY_MAP=$(jq '.entities | map({(.id): .attributes}) | add' "$DATA_FILE")

# Load entities as map directly into /v1/data/entities namespace
echo "$ENTITY_MAP" | curl -s -X PUT "${OPA_URL}/v1/data/entities" \
    -H "Content-Type: application/json" \
    -d @- > /dev/null

ENTITY_COUNT=$(jq '.entities | length' "$DATA_FILE")
echo -e "${GREEN}✓ Loaded ${ENTITY_COUNT} entities as indexed map${NC}"

# Load policy
curl -s -X PUT "${OPA_URL}/v1/policies/${SCENARIO}" \
    -H "Content-Type: text/plain" \
    --data-binary @"$POLICY_FILE" > /dev/null

echo -e "${GREEN}✓ Policy deployed${NC}"

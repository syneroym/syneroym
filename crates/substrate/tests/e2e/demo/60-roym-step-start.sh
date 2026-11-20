#!/usr/bin/env bash
# Roym step story, terminal part 2: Alice, who gives the slot, marks the work as
# started. The agreement id comes from the browser part (60-roym-step.spec.ts).
source "$(dirname "$0")/lib.sh"

if [ -z "${DEMO_STEP_AGREEMENT_ID:-}" ]; then
  echo "DEMO_STEP_AGREEMENT_ID is not set. Run 60-roym-step.spec.ts, not this script alone." >&2
  exit 1
fi

host="${ALICE_HUB_URL#http://}"
host="${host%:*}"

caption "Alice marks the work as started"
run rc1 roym step add \
  --agreement "$DEMO_STEP_AGREEMENT_ID" --commitment c1 --type scheduling/started \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$host"

caption "The step is listed for the agreement"
run rc1 roym step list --agreement "$DEMO_STEP_AGREEMENT_ID" \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$host"

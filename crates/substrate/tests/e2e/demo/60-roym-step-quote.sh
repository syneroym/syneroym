#!/usr/bin/env bash
# Roym step story, terminal part 1: Alice answers Bob's enquiry with a quote for
# the slot she made in her Hub. The values come from the browser part
# (60-roym-step.spec.ts) in DEMO_STEP_* variables.
source "$(dirname "$0")/lib.sh"

for name in DEMO_STEP_ENQUIRY_ID DEMO_STEP_LISTING_ID DEMO_STEP_SLOT_ID \
  DEMO_STEP_SLOT_START DEMO_STEP_SLOT_END DEMO_STEP_TAG; do
  if [ -z "${!name:-}" ]; then
    echo "$name is not set. Run 60-roym-step.spec.ts, not this script alone." >&2
    exit 1
  fi
done

# The Hub's host name, without the scheme and the port.
host="${ALICE_HUB_URL#http://}"
host="${host%:*}"

terms_dir=$(mktemp -d)
printf 'Notice of 24 hours is needed.\n' > "$terms_dir/cancellation.txt"
printf 'Full refund if the work is not done.\n' > "$terms_dir/refund.txt"

caption "Alice answers the enquiry with a quote for the slot"
run rc1 roym transaction quote \
  --enquiry "$DEMO_STEP_ENQUIRY_ID" \
  --scope "Garden clearance $DEMO_STEP_TAG" --lang en \
  --currency INR --amount 100 --payee "$ALICE_DID" --method cash \
  --timing after-work --where at-receiver \
  --schedule "$DEMO_STEP_SLOT_START,$DEMO_STEP_SLOT_END" \
  --listing-id "$DEMO_STEP_LISTING_ID" --slot "$DEMO_STEP_SLOT_ID" \
  --cancellation-file "$terms_dir/cancellation.txt" \
  --refund-file "$terms_dir/refund.txt" \
  --dispute "Informal mediation" --expires-hours 1 \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$host"

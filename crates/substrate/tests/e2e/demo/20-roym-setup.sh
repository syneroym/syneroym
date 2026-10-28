#!/usr/bin/env bash
# Roym chat story, terminal part: deploy Roym on both nodes, sign in the two
# owners, and learn the conversation address of each. The browser part is in
# 20-roym-chat.spec.ts.
source "$(dirname "$0")/lib.sh"

# Set up Roym on one node.  usage: roym_node <Name> <dir> <substrate-did> <gateway-port>
# Writes <NAME>_HUB_URL, <NAME>_KEY, <NAME>_DID and <NAME>_ADDR to the env file.
roym_node() {
  local name=$1 dir=$2 substrate=$3 gw=$4
  local upper
  upper=$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')
  local ctl=(roymctl --dir "$dir" --api-url "$REGISTRY" --substrate "$substrate" --as owner)

  caption "Deploy the Roym app on $name's node"
  # The manifest paths are relative to the repository root.
  (cd "$REPO" && run "${ctl[@]}" app deploy roym crates/roym_core/app/roym.toml \
    --mint-masters --registry-url "$REGISTRY" --journal-path "$dir/roym-deployments.db")

  caption "Give $name's Hub a host name"
  local web_did host
  web_did=$(did_of "$dir" "member-roym#web-0")
  run roymctl --dir "$dir" --api-url "$REGISTRY" registry register \
    --identity "member-roym#web-0" --substrate "$substrate" --nickname roym
  host=$(host_of "$web_did" --nickname roym --interface http-native)
  demo_set "${upper}_HUB_URL" "http://$host:$gw"
  echo "$name's Hub: $(eval echo "\$${upper}_HUB_URL")"

  caption "Create $name's session key and sign in"
  demo_set "${upper}_KEY" "$dir/session-key.json"
  run "${ctl[@]}" session delegate --registry-url "$REGISTRY" --out "$dir/session-key.json"
  run "${ctl[@]}" session login --gateway-url "http://127.0.0.1:$gw" --registry-url "$REGISTRY"
  run "${ctl[@]}" roym enrol-signing --master owner --gateway-url "http://127.0.0.1:$gw" \
    --host "$host" --registry-url "$REGISTRY"
  demo_set "${upper}_DID" "$(did_of "$dir" owner)"

  caption "Read $name's conversation address"
  local answer
  answer=$("${ctl[@]}" roym address)
  printf '%s\n' "$answer"
  local addr
  addr=$(printf '%s\n' "$answer" | sed -n 's/^conversation service id: //p')
  if [ -z "$addr" ]; then
    echo "roymctl roym address printed no conversation service id for $name" >&2
    return 1
  fi
  demo_set "${upper}_ADDR" "$addr"
}

roym_node Alice "$S1_DIR" "$S1_DID" "$S1_GW"
roym_node Bob "$S2_DIR" "$S2_DID" "$S2_GW"

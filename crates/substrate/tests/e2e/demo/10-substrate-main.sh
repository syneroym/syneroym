#!/usr/bin/env bash
# Substrate main story, terminal part: deploy a small service on S1, call it
# on S1 and through S2, then deploy a web app. The browser part is in
# 10-substrate-main.spec.ts.
source "$(dirname "$0")/lib.sh"

caption "Two nodes, nothing deployed yet"
run roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --substrate "$S1_DID" --as owner svc list
run roymctl --dir "$S2_DIR" --api-url "$REGISTRY" --substrate "$S2_DID" --as owner svc list

caption "Deploy a small service on S1"
run roymctl --dir "$S1_DIR" identity create --name greeter
GREETER_DID=$(did_of "$S1_DIR" greeter)
echo "service identity: $GREETER_DID"
run roymctl --dir "$S1_DIR" --api-url "$REGISTRY" registry register \
  --identity greeter --substrate "$S1_DID" --nickname greeter
run rc1 svc deploy --svc-id "$GREETER_DID" --interfaces "$GREETER_IFACE" --wasm "$GREETER_WASM"
run rc1 svc list

caption "Call it on S1"
GREETER_HOST=$(host_of "$GREETER_DID" --nickname greeter --interface "$GREETER_IFACE")
echo "host name: $GREETER_HOST"
rpc "$S1_GW" "$GREETER_HOST" greet '["Syneroym"]'

caption "Call the same service through the other node, S2"
rpc "$S2_GW" "$GREETER_HOST" greet '["from the other node"]'

caption "Deploy a full web app on S1"
ASSETS="$DEMO_HOME/miniapp-assets.tar.gz"
run env COPYFILE_DISABLE=1 tar -czf "$ASSETS" -C "$MINIAPP_DIR/static" .
run roymctl --dir "$S1_DIR" identity create --name webapp
WEB_DID=$(did_of "$S1_DIR" webapp)
echo "service identity: $WEB_DID"
run roymctl --dir "$S1_DIR" --api-url "$REGISTRY" registry register \
  --identity webapp --substrate "$S1_DID" --nickname webapp
run rc1 svc deploy --svc-id "$WEB_DID" \
  --interfaces "syneroym:http/incoming-handler@0.1.0,syneroym:http/websocket-handler@0.1.0,syneroym:messaging/guest-api@0.1.0,syneroym:messaging/stream-types@0.1.0" \
  --wasm "$MINIAPP_WASM" \
  --assets "$ASSETS" --asset-visibility public \
  --custom-config "$MINIAPP_DIR/routes.json"

# The HTTP bridge is reached through the reserved native-capability interface.
demo_set WEB_HOST "$(host_of "$WEB_DID" --nickname webapp --interface http-native)"
echo "web app host name: $WEB_HOST"

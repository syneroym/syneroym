#!/usr/bin/env bash
# Roym client story: follow "Build a Roym client" in docs/developer-guide.md with curl.
# The commands are not copied here. The script reads the guide's tagged code blocks
# (a first line like "```bash roym-client=login") and runs them, so a command that stops
# working in the guide fails this story. It starts from the state story 20 leaves: Roym
# deployed on both nodes, and Alice and Bob each with a session key file and an address.
# It runs once for each "mise run demo:up".
source "$(dirname "$0")/lib.sh"

GUIDE="$REPO/docs/developer-guide.md"
WORK="$DEMO_HOME/client"

# Print the body of the one guide block whose first line ends in "roym-client=<name>".
# usage: guide_block <name>
guide_block() {
  local name=$1 count
  count=$(grep -cE "^\`\`\`(bash|js) roym-client=${name//./\\.}\$" "$GUIDE" || true)
  if [ "$count" != 1 ]; then
    echo "guide block roym-client=$name: found $count in $GUIDE, expected 1" >&2
    return 1
  fi
  awk -v tag="roym-client=$name" '
    $0 == "```bash " tag || $0 == "```js " tag { inside = 1; next }
    inside && $0 == "```" { exit }
    inside { print }
  ' "$GUIDE"
}

# On a failure, say which guide block was running last.
CURRENT_BLOCK=none
set -E
trap 'echo "The story stopped. Last guide block: roym-client=$CURRENT_BLOCK" >&2' ERR

# Show a guide block and run it in this shell, as a reader would.
# usage: run_block <name>
run_block() {
  local body
  CURRENT_BLOCK=$1
  body=$(guide_block "$1")
  printf '%s\n' "$body"
  eval "$body"
}

# Start a guide block in the background, with its output in a file.
# usage: start_block <name> <output file>
start_block() {
  local body
  CURRENT_BLOCK=$1
  body=$(guide_block "$1")
  printf '%s\n' "$body"
  # An empty environment, with only the values the guide tells the reader to set, so a block
  # that needs another variable fails here.
  env -i PATH="$PATH" CURL_HOME="$CURL_HOME" HUB_URL="$HUB_URL" TOKEN="$TOKEN" CURSOR="${CURSOR:-}" \
    bash -c "$body" > "$2" 2>&1 &
}

# Wait until the output file has a line with both texts. usage: wait_for <file> <text> [text]
wait_for() {
  local file=$1 first=$2 second=${3:-} i
  for i in $(seq 1 100); do
    if grep -F -- "$first" "$file" 2>/dev/null | grep -qF -- "$second"; then
      return 0
    fi
    sleep 0.1
  done
  echo "no line with '$first' and '$second' in $file within 10 s:" >&2
  cat "$file" >&2 || true
  return 1
}

fail() { echo "$*" >&2; return 1; }

trap 'kill $(jobs -p) 2>/dev/null || true' EXIT

caption "Set-up, not part of the guide: a directory on Bob's node, and the nickname of Alice's Hub"
mkdir -p "$WORK"
printf 'Be kind. Keep your listings honest.\n' > "$WORK/rules.txt"
BOB_HUB_HOST=${BOB_HUB_URL#http://}
BOB_HUB_HOST=${BOB_HUB_HOST%:*}
run rc2 roym directory serve --name "Bob's guild" --rules-file "$WORK/rules.txt" \
  --category gardening --support support@example.org --dispute "Write to support." \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$BOB_HUB_HOST"
DIRECTORY_DID=$(did_of "$S2_DIR" "member-roym#directory-0")
# Story 20 registered Alice's Hub with this nickname.
HUB_NICKNAME=roym
SESSION_KEY=$ALICE_KEY
PEER_ADDR=$BOB_ADDR
GATEWAY_PORT=$S1_GW
cd "$WORK"
# Report whether this machine sends *.localhost to 127.0.0.1 by itself. It never fails the story.
if curl -sS -o /dev/null --max-time 5 "http://auth.localhost:$S1_GW/_syneroym/session/methods"; then
  echo "This machine resolves *.localhost."
else
  echo "This machine needs connect-to for *.localhost."
fi
# Not every machine sends *.localhost to 127.0.0.1. The guide says what to do when it does not:
# one line in ~/.curlrc. This story keeps that line in its own folder, so it runs on any machine.
printf 'connect-to = "::127.0.0.1:"\n' > "$WORK/.curlrc"
export CURL_HOME=$WORK

caption "What you need: find the Hub host"
# The guide runs "roymctl roym address" as the node owner. Here that is Alice.
roymctl() { command roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --substrate "$S1_DID" --as owner "$@"; }
run_block address
unset -f roymctl
[ -n "${HUB_URL:-}" ] && [ -n "${AUTH_URL:-}" ] || fail "address: HUB_URL or AUTH_URL is not set"

caption "Sign in"
guide_block sign-challenge.mjs > sign-challenge.mjs
run_block login
[ -n "$TOKEN" ] && [ "$TOKEN" != null ] || fail "login: no token"

caption "Call a method"
run_block rpc
roym session.whoami | jq -e --arg d "$ALICE_DID" '.result.did == $d and .result.auth == "delegated"' > /dev/null \
  || fail "rpc: session.whoami through $HUB_URL does not say delegated for $ALICE_DID"

caption "Conversations: list"
run_block list
roym conversation.list | jq -e '.result.conversations | type == "array"' > /dev/null \
  || fail "list: the answer has no conversations list"

caption "Follow the event feed (a second terminal in the guide)"
start_block feed "$WORK/feed.txt"
wait_for "$WORK/feed.txt" "event: ready" || fail "feed: no ready frame"

caption "Conversations: open one and send a message"
run_block open-send
[ -n "$CONV" ] && [ -n "$MESSAGE_ID" ] || fail "open-send: CONV or MESSAGE_ID is not set"

caption "Follow the event feed: the message shows up"
wait_for "$WORK/feed.txt" '"kind":"conversation.message"' "\"item\":\"$MESSAGE_ID\"" \
  || fail "feed: no conversation.message event for $MESSAGE_ID"
cat "$WORK/feed.txt"

caption "Follow the event feed: catch up with the last cursor"
CURSOR=$(sed -n 's/^id: //p' "$WORK/feed.txt" | head -1)
[ -n "$CURSOR" ] || fail "catch-up: the ready frame has no id"
start_block catch-up "$WORK/catch-up.txt"
wait_for "$WORK/catch-up.txt" '"kind":"conversation.message"' "\"item\":\"$MESSAGE_ID\"" \
  || fail "catch-up: the event after the cursor did not come again"
cat "$WORK/catch-up.txt"
kill $(jobs -p) 2>/dev/null || true

caption "Search"
run_block search
roym directory.find '{"query":{"text":"plumber"}}' \
  | jq -e --arg d "$DIRECTORY_DID" '.result.outcomes | length == 1 and .[0].source == $d and .[0].error == null' \
    > /dev/null || fail "search: expected one outcome for $DIRECTORY_DID with error null"

caption "Show cards"
run_block cards
roym transaction.thread "{\"conversation\":\"$CONV\"}" | jq -e '.result.cards | type == "array"' > /dev/null \
  || fail "cards: the answer has no cards list"

caption "End the session"
OLD_TOKEN=$TOKEN
run_block refresh-logout
printf '\n'
for t in "$OLD_TOKEN" "$NEW_TOKEN"; do
  TOKEN=$t roym conversation.list | jq -e '.error.code == -32010' > /dev/null \
    || fail "refresh-logout: a call with a logged-out token did not answer -32010"
done
echo "Both tokens are logged out."

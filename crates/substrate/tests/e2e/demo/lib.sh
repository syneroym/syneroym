# Shared helpers for the demo story scripts. Source it; do not run it.
#
#   source "$(dirname "$0")/lib.sh"
#
# It loads the env file that "mise run demo:up" (or the demo setup) wrote, and
# keeps the helper names of docs/demo/demo.env.example.
#
# DEMO_MODE=present  pause after each caption (DEMO_PACE_MS, default 600 ms)
# DEMO_STEP=1        wait for Enter after each caption (live talks, from a terminal)

set -euo pipefail

DEMO_E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_ENV_FILE="${DEMO_ENV_FILE:-$DEMO_E2E_DIR/.e2e-data-demo/demo.env}"
if [ ! -f "$DEMO_ENV_FILE" ]; then
  echo "No demo env file at $DEMO_ENV_FILE. Run: mise run demo:up" >&2
  exit 1
fi
# shellcheck disable=SC1090
source "$DEMO_ENV_FILE"
export PATH="$BIN:$PATH"

# Print a step title. This is the line the viewer reads.
caption() {
  printf '\n== %s ==\n' "$*"
  if [ "${DEMO_STEP:-}" = "1" ]; then
    read -r -p "(Enter to continue) " _
  elif [ "${DEMO_MODE:-}" = "present" ]; then
    sleep "$(awk "BEGIN { print ${DEMO_PACE_MS:-600} / 1000 }")"
  fi
}

# Print a command, then run it.
run() {
  printf '$ %s\n' "$*"
  "$@"
}

# Remember a value for later scripts and for the Playwright specs.
demo_set() {
  # Escape the characters that are special inside double quotes; readDemoEnv undoes this.
  local escaped
  escaped=$(printf '%s' "$2" | sed 's/[\\"$`]/\\&/g')
  printf 'export %s="%s"\n' "$1" "$escaped" >> "$DEMO_ENV_FILE"
  export "$1=$2"
}

# "Run roymctl against substrate 1 / 2, as its owner."
rc1() { roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --substrate "$S1_DID" --as owner "$@"; }
rc2() { roymctl --dir "$S2_DIR" --api-url "$REGISTRY" --substrate "$S2_DID" --as owner "$@"; }

# Print the DID of a locally stored identity.  usage: did_of <dir> <name>
did_of() { roymctl --dir "$1" identity show --name "$2" | grep -oE 'did:key:[a-z0-9]+' | head -1; }

# Print the gateway host name for a service.  usage: host_of <did> [alias flags...]
host_of() { roymctl alias "$@" | tail -1; }

# JSON-RPC call through a gateway. Prints the answer and fails if it is not a result.
#   usage: rpc <gateway-port> <host> <method> '<json params>'
rpc() {
  printf '$ curl -X POST http://127.0.0.1:%s/ -H "Host: %s" -d '"'"'{"method":"%s","params":%s}'"'"'\n' \
    "$1" "$2" "$3" "${4:-[]}"
  local answer
  answer=$(curl -sS -X POST "http://127.0.0.1:$1/" -H "Host: $2" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"$3\",\"params\":${4:-[]},\"id\":1}")
  printf '%s\n' "$answer"
  case "$answer" in
    *'"result"'*) ;;
    *) echo "rpc $3 did not return a result" >&2; return 1 ;;
  esac
}

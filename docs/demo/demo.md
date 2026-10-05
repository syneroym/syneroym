# Syneroym demo runbook

A script to present Syneroym without losing your place. Every command is a
copy-paste block. Every step says what you should see.

> ## ⚠ Draft: not run end to end yet
>
> This runbook was **written from the end-to-end test setup, `roymctl --help` and the
> developer guide. It has never been run start to finish on a cloud machine.**
> Expect gaps and wrong details: a flag, a port, a host name, a wait time.
> Do **not** use it live in front of people until you have rehearsed it once.
> Steps marked **Rehearse** are the ones we know are least certain. When you fix a
> step, fix it here. Remove this box when the whole runbook has been run once.
>
> Setup, start-up, preflight and reset are written as manual steps for now. They are
> meant to become `mise` tasks or scripts later.

## Read this first: known traps

1. **The relay data port is UDP.** Open **UDP 7965** on the cloud machine, not only TCP.
2. **The data-key (KEK) is lost on every restart.** After starting or restarting any node,
   run `rc1 kek inject "$KEK"` (and `rc2` for S2). WASM services fail without it.
3. **IP-only cloud.** Browser host names use `<ip>.nip.io`, which needs internet DNS.
   If that fails, use "Path B" (local gateway) in the main story.
4. **The Roym Hub needs a `*.localhost` host name** (a secure browser context). Never open
   it through the cloud IP.
5. **Run `source ~/syneroym-demo.env` and `demo_dids` in every new terminal.**
6. **`identity issue-grant` takes one `--can` and `--with`** and prints the token to the
   screen. Redirect it to a file with `>`.
7. **Some stories have no CLI "call" command yet** (row security, messaging). They use the
   browser app, `curl`, or run an existing end-to-end test live. Those tests are listed in the story.
8. **Build the test binaries the day before** (section 1.1) so nothing compiles on stage.

**How to read the markers**

| Marker | Meaning |
|---|---|
| (no marker) | The command comes from the working end-to-end test setup, or from `roymctl --help`. |
| **Rehearse** | Command shape is right, but it was not run end to end yet, or one detail is unknown. Run it once before the first real demo. |
| **Say** | One or two sentences to tell the audience. |
| **Expect** | What the screen should show. If it does not match, go to [section 6](#6-if-something-breaks). |

All stories show the **normal, working path**. Failure cases are left out on purpose.

---

## Contents

0. [Topology and cheat sheet](#0-topology-and-cheat-sheet)
1. [Preparation](#1-preparation)
2. [Main story: run an app on my node, reach it from anywhere](#2-main-story)
3. [Feature stories](#3-feature-stories) — pick any, in any order
   - F1 Many kinds of service
   - F2 A full web app: pages, REST, files, WebSocket, SSE
   - F3 Owning a node and delegating control
   - F4 Person identity and sessions
   - F5 Data protection: keys, secrets, encrypted files
   - F6 Discovery: registry and names
   - F7 Multi-hop coordinators
   - F8 One app on many nodes, run by the supervisor
   - F9 Scheduled jobs
   - F10 Durable calls and sagas *(deferred, outline only)*
   - F11 Row and column level security
   - F12 Messaging (publish / subscribe)
   - F13 Operations: health, metrics, smoke tests, benchmarks
4. [Roym](#4-roym) — R0 to R7
5. [Reset and teardown](#5-reset-and-teardown)
6. [If something breaks](#6-if-something-breaks)

Time guide: main story 12 min. Each feature story 3 to 8 min. Roym 25 min in total.

---

## 0. Topology and cheat sheet

Two machines.

```
   CLOUD machine (public IP, Linux)              YOUR MAC
 ┌──────────────────────────────────┐     ┌──────────────────────────────────────┐
 │ "C": coordinator + relay         │     │ S1 "alice-node"   gateway :7970      │
 │      + community registry        │◄────┤ S2 "bob-node"     gateway :7980      │
 │  7961 registry (HTTP)            │     │ Browser window A, window B           │
 │  7962 WebRTC bootstrap page      │     │ Terminals T1..T5 (below)             │
 │  7963 WebRTC signalling          │     └──────────────────────────────────────┘
 │  7964 relay signalling (HTTP)    │
 │  7965 relay data (QUIC, **UDP**) │
 └──────────────────────────────────┘
```

Nodes find each other **by identity (DID), not by IP**. They publish where they
are to the registry on the cloud machine. The relay on the cloud machine carries
traffic when two nodes cannot connect directly.

**Terminals on the Mac.** Keep this layout for the whole demo. Name each tab.

| Tab | Role | What runs there |
|---|---|---|
| T0 | Cloud (ssh) | `syneroym-substrate` for C, inside `tmux` |
| T1 | S1 log | `syneroym-substrate` for S1 (foreground, shows logs) |
| T2 | S2 log | `syneroym-substrate` for S2 (foreground, shows logs) |
| T3 | Operator 1 | `rc1 ...` commands against S1 |
| T4 | Operator 2 | `rc2 ...` commands against S2 |
| T5 | Spare | `curl`, `tar`, anything else |

In **every** Mac terminal, run first:

```bash
source ~/syneroym-demo.env
```

That gives you these helpers (defined in [demo.env.example](demo.env.example)):

| Helper | Does |
|---|---|
| `rc1 <args>` / `rc2 <args>` | `roymctl` against S1 / S2, as its owner |
| `did_of <dir> <name>` | print the DID of a stored identity |
| `host_of <did> --nickname n --interface i` | print the gateway host name for a service |
| `rpc <port> <host> <method> '<json params>'` | JSON-RPC call through a gateway |

**Ports on the Mac.** S1 gateway `7970`, S2 gateway `7980`, S1 health `7976`, S1 metrics `7977`.

**Host names.** A service is reached by a host name built from its DID:
`<nickname>-s<hash>[-i<interface-hash>].<domain>`. Only the first part matters.
Use `.localhost` for the Mac gateways. Use `$CLOUD_DOMAIN` (`<ip>.nip.io`) when the
browser must reach the cloud machine.

---

## 1. Preparation

### 1.1 One week to one day before

On the **Mac**:

```bash
cd "$REPO"
mise install
cargo build --release --bin roymctl --bin syneroym-substrate
mise run build:test-components      # all WASM test components, incl. the web app
mise run build:roym                 # Roym UI bundle + six Roym WASM services
```

Pre-build the two test programs that stories F11 and F12 run live (so nothing compiles on stage):

```bash
cargo nextest run -p syneroym-substrate --test federated_fdae_e2e --test messaging_client_e2e --no-run
```

Check the web app UI exists (it is built into the WASM file):

```bash
ls "$MINIAPP_WASM" "$GREETER_WASM" "$REPO/target/wasm32-wasip2/release/syneroym_roym_web.wasm"
```

**Expect:** three paths, no "No such file".

Copy the env file and set the cloud IP:

```bash
cp "$REPO/docs/demo/demo.env.example" ~/syneroym-demo.env
$EDITOR ~/syneroym-demo.env        # set CLOUD_IP
```

On the **cloud machine** (Linux):

1. Open the firewall: TCP `7961 7962 7963 7964`, **UDP `7965`**. The relay data port
   is UDP. This is the most common mistake.
2. Build the substrate there. Cross-building is not worth the risk before a demo.

```bash
git clone https://github.com/syneroym/syneroym.git ~/syneroym   # or git pull
cd ~/syneroym && cargo build --release --bin syneroym-substrate
```

**Browser host names (IP-only cloud).** The WebRTC page is opened with a host name
that carries the service name. `*.nip.io` names resolve to the IP inside them, so no
`/etc/hosts` edit is needed. Check it works on the Mac:

```bash
dig +short test.$CLOUD_IP.nip.io
```

**Expect:** your `CLOUD_IP`. If your network blocks public DNS, add a line to
`/etc/hosts` for each host name you will use (the story prints them) or use
"Path B" in the main story, which needs no cloud DNS.

### 1.2 Thirty minutes before: start the cloud machine

> *Manual for now. To be scripted later.*

On the cloud machine, in `tmux` (so it survives a dropped ssh):

```bash
export CLOUD_IP="<this machine's public IP>"
export DEMO_HOME=$HOME/syneroym-demo
mkdir -p $DEMO_HOME/cloud

cat > $DEMO_HOME/cloud/config.toml <<EOF
config_version = 1
app_config_dir = "$DEMO_HOME/cloud"
app_local_data_dir = "$DEMO_HOME/cloud"
app_data_dir = "$DEMO_HOME/cloud"
profile = "full"

[identity]
key = "substrate.key"
nickname = "cloud-c"

[roles.community_registry]
access = "everyone"
http_bind_address = "0.0.0.0:7961"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:7964"
quic_bind_address = "0.0.0.0:7965"
info_http_bind_address = "0.0.0.0:0"
community_registry_url = "http://127.0.0.1:7961"
share_in_registry = true

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:7963"
bootstrap_page_bind_address = "0.0.0.0:7962"

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:7961"
EOF

cd ~/syneroym
RUST_LOG=info NO_COLOR=1 ./target/release/syneroym-substrate run \
  --config $DEMO_HOME/cloud/config.toml 2>&1 | tee $DEMO_HOME/cloud/substrate.log
```

**Expect:** log lines that say the registry and coordinator started, no errors.

Check from the **Mac** (T5):

```bash
source ~/syneroym-demo.env
curl -s -o /dev/null -w "registry HTTP %{http_code}\n" "$REGISTRY/"
curl -s -o /dev/null -w "webrtc bootstrap HTTP %{http_code}\n" "http://$CLOUD_IP:7962/"
```

**Expect:** a number (200 or 404 are both fine). `000` means a firewall or the process is down.

### 1.3 Thirty minutes before: create and start S1 and S2 on the Mac

Each node needs: a key, an owner identity, a claim (ownership), a config, and a start.
Do S1, then S2. (T5 for the setup, T1 and T2 for the logs.)

> *Manual for now. To be scripted later.*

**S1 setup (T5):**

```bash
mkdir -p "$S1_DIR"
roymctl substrate init --dir "$S1_DIR"
roymctl --dir "$S1_DIR" identity create --name owner
roymctl --dir "$S1_DIR" substrate claim --controller owner

cat > "$S1_DIR/config.toml" <<EOF
config_version = 1
app_config_dir = "$S1_DIR"
app_local_data_dir = "$S1_DIR"
app_data_dir = "$S1_DIR"
profile = "full"

[identity]
key = "substrate.key"
nickname = "alice-node"

[roles.app_sandbox]

[roles.client_gateway]
http_port = $S1_GW
identity_mode = "login"

[roles.auth]

[roles.supervisor]
poll_interval_secs = 10

[roles.observability.health]
enabled = true
bind_address = "127.0.0.1:$S1_HEALTH"
endpoint = "/health"

[roles.observability.metrics]
enabled = true
bind_address = "127.0.0.1:$S1_METRICS"
endpoint = "/metrics"

[parent_coordinator.iroh]
url = "http://$CLOUD_IP:7964"

[parent_coordinator.webrtc]
signaling_url = "ws://$CLOUD_IP:7963/ws"
bootstrap_url = "ws://$CLOUD_IP:7962"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "$REGISTRY"
EOF
```

**S2 setup (T5)** — same, other directory, other port, no supervisor, no metrics:

```bash
mkdir -p "$S2_DIR"
roymctl substrate init --dir "$S2_DIR"
roymctl --dir "$S2_DIR" identity create --name owner
roymctl --dir "$S2_DIR" substrate claim --controller owner

cat > "$S2_DIR/config.toml" <<EOF
config_version = 1
app_config_dir = "$S2_DIR"
app_local_data_dir = "$S2_DIR"
app_data_dir = "$S2_DIR"
profile = "full"

[identity]
key = "substrate.key"
nickname = "bob-node"

[roles.app_sandbox]

[roles.client_gateway]
http_port = $S2_GW
identity_mode = "login"

[roles.auth]

[parent_coordinator.iroh]
url = "http://$CLOUD_IP:7964"

[parent_coordinator.webrtc]
signaling_url = "ws://$CLOUD_IP:7963/ws"
bootstrap_url = "ws://$CLOUD_IP:7962"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "$REGISTRY"
EOF
```

**Start S1 (T1) and S2 (T2).** Each in its own tab, in the foreground:

```bash
# T1
RUST_LOG=info NO_COLOR=1 syneroym-substrate run --config "$S1_DIR/config.toml" 2>&1 | tee "$S1_DIR/substrate.log"
```

```bash
# T2
RUST_LOG=info NO_COLOR=1 syneroym-substrate run --config "$S2_DIR/config.toml" 2>&1 | tee "$S2_DIR/substrate.log"
```

**Read the DIDs and unlock the data keys (T3):**

```bash
demo_dids
rc1 kek inject "$KEK"
rc2 kek inject "$KEK"
```

**Expect:** two `did:key:...` lines, then two "injected" style answers.
The key-encryption key (KEK) is **not stored on disk**. You must inject it after
**every restart** of a node. Without it, WASM services cannot open their data.

> **Rehearse:** (a) the sample config uses `profile = "full"` with role tables to turn
> roles on, like the test setup does. If a role does not start, check the start-up log
> for the role name. (b) `[roles.supervisor]` with only `poll_interval_secs` assumes the
> other fields have defaults.

### 1.4 Five minutes before: preflight

> *Manual for now. To be scripted later.*

Run this block in T3. Every line must pass.

```bash
echo "--- cloud registry";   curl -s -o /dev/null -w "%{http_code}\n" "$REGISTRY/"
echo "--- S1 health";        curl -s "http://127.0.0.1:$S1_HEALTH/health"; echo
echo "--- S1 services";      rc1 svc list
echo "--- S2 services";      rc2 svc list
echo "--- clock";            date -u
```

**Expect:** registry answers, S1 health says healthy, both `svc list` calls return an
empty list, and the Mac clock is correct (certificates and sessions use it).

Then:

- Browser window A and window B are open, side by side, blank.
- Text size is large. Notifications are off.
- T1 and T2 logs are visible in a small font (they show the audience that something is happening).
- Close anything left from a previous run, or do a full [reset](#5-reset-and-teardown).

---

## 2. Main story

**Message:** *I run an app on my own machine. Anyone, anywhere, can reach it by its
identity. No public IP, no port forwarding, no hosting account.*

Time: 12 minutes. Needs: [Preparation](#1-preparation) done.

### 2.1 Show the empty nodes

```bash
rc1 svc list
rc2 svc list
```

**Say:** "Two nodes on this laptop. One public coordinator on the cloud. Nothing deployed yet."

### 2.2 Deploy a small service on S1

Every service has its own identity (a DID). Create one, publish it to the registry,
then deploy the WASM component.

```bash
roymctl --dir "$S1_DIR" identity create --name greeter
GREETER_DID=$(did_of "$S1_DIR" greeter); echo "$GREETER_DID"

roymctl --dir "$S1_DIR" --api-url "$REGISTRY" registry register \
  --identity greeter --substrate "$S1_DID" --nickname greeter

rc1 svc deploy --svc-id "$GREETER_DID" \
  --interfaces "$GREETER_IFACE" \
  --wasm "$GREETER_WASM"

rc1 svc list
```

**Expect:** `svc list` shows the greeter with its interface.
**Say:** "A WebAssembly component. It runs in a sandbox with limits on CPU and memory."

### 2.3 Call it on S1

```bash
GREETER_HOST=$(host_of "$GREETER_DID" --nickname greeter --interface "$GREETER_IFACE")
echo "$GREETER_HOST"
rpc "$S1_GW" "$GREETER_HOST" greet '["Syneroym"]'
```

**Expect:** a JSON result like `Hello, Syneroym! Greetings from ...`.

### 2.4 Call the same service through the OTHER node

Same host name. Different gateway: S2 does not have this service. It looks it up in
the cloud registry and connects to S1 for you.

```bash
rpc "$S2_GW" "$GREETER_HOST" greet '["from the other node"]'
```

**Expect:** the same kind of answer. Watch T1: S1 logs the call.
**Say:** "S2 never knew where S1 was. It asked the registry for the identity and
connected. If a direct link is not possible, the cloud relay carries the traffic."

### 2.5 Deploy a full web app

Package the app's static files, create its identity, register, and deploy with its
routes and assets.

```bash
cd "$MINIAPP_DIR"
mkdir -p "$DEMO_HOME/tmp"
COPYFILE_DISABLE=1 tar -czf "$DEMO_HOME/tmp/miniapp-assets.tar.gz" -C static .

roymctl --dir "$S1_DIR" identity create --name webapp
WEB_DID=$(did_of "$S1_DIR" webapp); echo "$WEB_DID"

roymctl --dir "$S1_DIR" --api-url "$REGISTRY" registry register \
  --identity webapp --substrate "$S1_DID" --nickname webapp

rc1 svc deploy --svc-id "$WEB_DID" \
  --interfaces "syneroym:http/incoming-handler@0.1.0,syneroym:http/websocket-handler@0.1.0,syneroym:messaging/guest-api@0.1.0,syneroym:messaging/stream-types@0.1.0" \
  --wasm "$MINIAPP_WASM" \
  --assets "$DEMO_HOME/tmp/miniapp-assets.tar.gz" --asset-visibility public \
  --custom-config "$MINIAPP_DIR/routes.json"
cd "$REPO"
```

**Expect:** deploy succeeds. (This needs the KEK from step 1.3.)

### 2.6 Open it in the browser

**Path A — through the cloud (WebRTC).** The browser talks to the cloud bootstrap
page, which connects it to S1 over WebRTC.

```bash
WEB_HOST_CLOUD=$(host_of "$WEB_DID" --nickname webapp --interface http-native --domain "$CLOUD_DOMAIN")
echo "http://$WEB_HOST_CLOUD:7962/?force_tunnel=false"
```

Open that URL in **window A**. The page shows "Hello world from ...". Click
**Comments etc.**

**Path B — local gateway (no cloud DNS needed).** Use this if Path A is slow or the
network is bad.

```bash
WEB_HOST=$(host_of "$WEB_DID" --nickname webapp --interface http-native)
echo "http://$WEB_HOST:$S1_GW/"
```

**Say:** "This web app is a WebAssembly file on my laptop. The browser reached it by identity."
`?force_tunnel=true` forces the traffic through the cloud relay instead of a direct
link. It is a good toggle if the audience asks what the relay does.

### 2.7 Show it live

1. In window A: type a comment, press **Submit**. **Expect:** "Comment saved!" and the comment in the list.
2. Open the same URL in window B (a private window). Go to **Comments etc.**
3. Submit a comment in window B. **Expect:** window A updates its "last updated" markers (WebSocket and SSE).

**Say:** "Two sessions. Updates arrive live. Same app, same node."

Optional, for the cross-node point: open `http://<WEB_HOST>:$S2_GW/` (Path B host
name, S2's port) in window B instead. The app still runs on S1.

> **Rehearse:** WebSocket and SSE through the S2 gateway to a service on S1.

**Main story done.** Next: any feature story, or [Roym](#4-roym).

---

## 3. Feature stories

Each story is independent. State at the start of each story is the state at the end of
the main story, unless it says otherwise.

### F1 Many kinds of service

**Message:** WASM, an existing local server, and containers all become named services
with the same tools.

Needs: main story 2.1 done.

**TCP passthrough.** Put a normal web server behind a service identity.

```bash
mkdir -p "$DEMO_HOME/tmp/site" && echo "<h1>Hello from a plain web server</h1>" > "$DEMO_HOME/tmp/site/index.html"
( cd "$DEMO_HOME/tmp/site" && python3 -m http.server 8099 --bind 127.0.0.1 ) &   # note the job number for cleanup

roymctl --dir "$S1_DIR" identity create --name plain
PLAIN_DID=$(did_of "$S1_DIR" plain)
roymctl --dir "$S1_DIR" --api-url "$REGISTRY" registry register \
  --identity plain --substrate "$S1_DID" --nickname plain
rc1 svc deploy --svc-id "$PLAIN_DID" --interfaces http --tcp 127.0.0.1:8099

PLAIN_HOST=$(host_of "$PLAIN_DID" --nickname plain --interface http)
curl -s "http://127.0.0.1:$S1_GW/" -H "Host: $PLAIN_HOST"
curl -s "http://127.0.0.1:$S2_GW/" -H "Host: $PLAIN_HOST"      # same thing, via the other node
```

**Expect:** the HTML twice.
**Say:** "Any existing server can join without a code change."

**Container (optional, needs Podman).**

```bash
roymctl --dir "$S1_DIR" identity create --name web-container
CT_DID=$(did_of "$S1_DIR" web-container)
rc1 svc deploy --svc-id "$CT_DID" --interfaces default \
  --image docker.io/library/nginx:alpine --port default:80
```

**Rehearse:** Podman on macOS needs `podman machine start` first. Skip this part if it
is not ready. Reaching it works like the TCP example (`--interface default`).

### F2 A full web app: pages, REST, files, WebSocket, SSE

**Message:** One WASM file serves pages, an API, large file transfer and live updates.

Needs: main story 2.5 and 2.6 done. Window A on the app.

| Action in the browser | What to point out |
|---|---|
| Load the page | Static files came straight from storage. No code ran for them. |
| Submit a comment | REST call handled by the WASM guest. |
| Choose a file, press **Upload** (try a few MB) | Streamed in chunks. "Upload successful!" |
| Click the uploaded file name | Streamed back down. |
| Header "WebSocket: Connected" and the echo line | Two-way channel. |
| Post from window B | The WebSocket and SSE "last updated" fields change in window A. |

### F3 Owning a node and delegating control

**Message:** A node has one owner. The owner can hand a narrow, time-limited right to
someone else, with no shared password.

Needs: main story 2.2 done.

```bash
# An operator identity. In real life this is another person's key.
roymctl --dir "$S1_DIR" identity create --name ops
OPS_DID=$(did_of "$S1_DIR" ops)

# The owner signs a grant for 1 day: "may read status of any app on this node".
roymctl --dir "$S1_DIR" --as owner identity issue-grant \
  --from owner --to "$OPS_DID" \
  --can orchestrator/status --with "substrate:$S1_DID/app/*" \
  --expires-days 1 > "$S1_DIR/ops-grant.json"

# The operator uses it.
roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --substrate "$S1_DID" \
  --as ops --ucan "$S1_DIR/ops-grant.json" svc list
```

**Expect:** the service list, shown to `ops`.
**Say:** "The grant is a signed token. Nobody changed any server setting. It ends in a day."

> **Rehearse:** abilities are exact names, not patterns. The three node abilities are
> `orchestrator/deploy`, `orchestrator/status` and `orchestrator/undeploy`. One token
> carries one ability. Check that `svc list` is covered by `orchestrator/status`. To show
> a deploy right, issue a second token with `--can orchestrator/deploy` and run
> `svc deploy` with it (a failed deploy also needs `orchestrator/undeploy`).

### F4 Person identity and sessions

**Message:** People log in with a key they own. The node learns *who* is calling, and
services can use that.

Needs: S1 running with `[roles.auth]` and `identity_mode = "login"` (1.3).

```bash
roymctl --dir "$S1_DIR" --as owner session delegate \
  --registry-url "$REGISTRY" --out "$S1_DIR/session-key.json"
roymctl --dir "$S1_DIR" --as owner session login \
  --gateway-url "http://127.0.0.1:$S1_GW" --registry-url "$REGISTRY"
roymctl --dir "$S1_DIR" session status --gateway-url "http://127.0.0.1:$S1_GW"
```

**Expect:** a session for the owner's DID.
**Say:** "The master key stays on this machine. The browser and CLI get a short-lived
delegated key."

Call a service with the token:

```bash
curl -s "http://127.0.0.1:$S1_GW/_syneroym/session/whoami" \
  -H "Authorization: Bearer $(roymctl --dir "$S1_DIR" session token --gateway-url "http://127.0.0.1:$S1_GW")"
roymctl --dir "$S1_DIR" session logout --gateway-url "http://127.0.0.1:$S1_GW"
```

**Rehearse:** the `whoami` call may need the `auth.<domain>` host. If it fails, use
`-H "Host: auth.localhost"`.

### F5 Data protection: keys, secrets, encrypted files

**Message:** Data on disk is encrypted. The key that opens it is not on disk.

Needs: main story 2.5 done (a service with a database).

1. Show the KEK idea. Stop S1 (Ctrl-C in T1) and start it again. Try the web app: it
   does not work. Inject the key:
   ```bash
   rc1 kek inject "$KEK"
   ```
   **Expect:** the app works again.
2. Show a database file is not readable:
   ```bash
   find "$S1_DIR/db" -name '*.db' | head -3
   head -c 64 "$(find "$S1_DIR/db/services" -name state.db | head -1)" | xxd | head -3
   ```
   **Expect:** random bytes, not the text `SQLite format 3`.
3. Store a secret in a service's private vault:
   ```bash
   rc1 secret set "$WEB_DID" demo-api-key
   ```
4. Rotate the master key (optional):
   ```bash
   rc1 kek rotate 3131313131313131313131313131313131313131313131313131313131313131
   ```
   After a rotation, use the new value as `KEK` for later injects.

**Rehearse:** `secret set` reads its value from stdin or a prompt (not confirmed). Check
where the database files really are with the `find` command.

### F6 Discovery: registry and names

**Message:** Look up any service by its identity or a human nickname. No DNS, no IP lists.

Needs: main story 2.2 done.

```bash
roymctl --api-url "$REGISTRY" registry lookup "$GREETER_DID"
roymctl --api-url "$REGISTRY" registry lookup "greeter-$(roymctl shorthash "$GREETER_DID")"
curl -s "$REGISTRY/lookup/$GREETER_DID"; echo
```

**Expect:** a signed record: which node hosts the service and how to reach it.
**Say:** "The record is signed by the service's own key. The registry cannot forge it."

### F7 Multi-hop coordinators

**Message:** Coordinators can be chained. A private coordinator "Cp" (think: inside a
company network) uses the public one on the cloud as its parent. A browser or node can
still reach a service behind Cp through the cloud.

```
 browser ──► C (cloud) ──► Cp (Mac) ──► S2        "inbound"
 browser ──► Cp (Mac) ──► C (cloud) ──► S1        "reverse"
```

Needs: main story done, the cloud config from 1.2 (it includes `share_in_registry`).
Adds a third node, **Cp**, in a new tab **T6**. Cp only does coordinator work: it hosts no apps.

**1. Create and start Cp (T5, then T6).**

```bash
export CP_DIR="$DEMO_HOME/cp"
mkdir -p "$CP_DIR"
roymctl substrate init --dir "$CP_DIR"

cat > "$CP_DIR/config.toml" <<EOF
config_version = 1
app_config_dir = "$CP_DIR"
app_local_data_dir = "$CP_DIR"
app_data_dir = "$CP_DIR"
profile = "full"

[identity]
key = "substrate.key"
nickname = "cp-private"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "127.0.0.1:7984"
quic_bind_address = "127.0.0.1:7985"
info_http_bind_address = "127.0.0.1:0"
community_registry_url = "$REGISTRY"
share_in_registry = true

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "127.0.0.1:7983"
bootstrap_page_bind_address = "127.0.0.1:7982"

[parent_coordinator.iroh]
url = "http://$CLOUD_IP:7964"

[parent_coordinator.webrtc]
signaling_url = "ws://$CLOUD_IP:7963/ws"
bootstrap_url = "ws://$CLOUD_IP:7962"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "$REGISTRY"
EOF
```

```bash
# T6
RUST_LOG=info NO_COLOR=1 syneroym-substrate run --config "$CP_DIR/config.toml" 2>&1 | tee "$CP_DIR/substrate.log"
```

**2. Move S2 behind Cp.** Stop S2 (Ctrl-C in T2). Write a second config for it that
points to Cp, not to the cloud. Same keys, same DID.

```bash
sed -e "s#url = \"http://$CLOUD_IP:7964\"#url = \"http://127.0.0.1:7984\"#" \
    -e "s#signaling_url = .*#signaling_url = \"ws://127.0.0.1:7983/ws\"#" \
    -e "s#bootstrap_url = .*#bootstrap_url = \"ws://127.0.0.1:7982\"#" \
    "$S2_DIR/config.toml" > "$S2_DIR/config-behind-cp.toml"
grep -A2 parent_coordinator "$S2_DIR/config-behind-cp.toml"
```

```bash
# T2
RUST_LOG=info NO_COLOR=1 syneroym-substrate run --config "$S2_DIR/config-behind-cp.toml" 2>&1 | tee -a "$S2_DIR/substrate.log"
```

```bash
# T4
rc2 kek inject "$KEK"
```

**3. Put an app on S2** (the same web app, with a second name):

```bash
roymctl --dir "$S2_DIR" identity create --name webapp2
WEB2_DID=$(did_of "$S2_DIR" webapp2)
roymctl --dir "$S2_DIR" --api-url "$REGISTRY" registry register \
  --identity webapp2 --substrate "$S2_DID" --nickname webapp2
rc2 svc deploy --svc-id "$WEB2_DID" \
  --interfaces "syneroym:http/incoming-handler@0.1.0,syneroym:http/websocket-handler@0.1.0,syneroym:messaging/guest-api@0.1.0,syneroym:messaging/stream-types@0.1.0" \
  --wasm "$MINIAPP_WASM" \
  --assets "$DEMO_HOME/tmp/miniapp-assets.tar.gz" --asset-visibility public \
  --custom-config "$MINIAPP_DIR/routes.json"
```

**4. Inbound: browser → C → Cp → S2.** Open this in window A:

```bash
WEB2_HOST_CLOUD=$(host_of "$WEB2_DID" --nickname webapp2 --interface http-native --domain "$CLOUD_DOMAIN")
echo "http://$WEB2_HOST_CLOUD:7962/?force_tunnel=false"
```

**Expect:** the app page. Post a comment.

**5. Reverse: browser → Cp → C → S1.** Open the main-story app (on S1) through Cp's own page:

```bash
WEB_HOST=$(host_of "$WEB_DID" --nickname webapp --interface http-native)
echo "http://$WEB_HOST:7982/?force_tunnel=false"
```

**Expect:** the S1 app page, reached through the private coordinator.
**Say:** "Neither side needed to know the other's address. The coordinators passed the request up and down."

**To undo:** stop S2 and Cp, start S2 with its normal `config.toml`, run `rc2 kek inject "$KEK"`.

> **Rehearse:** The layout and config follow the multi-hop end-to-end test
> (`crates/substrate/tests/e2e/global-setup-multihop.ts`, `tests/multi-hop.spec.ts`).
> The `sed` edit assumes the S2 config has exactly one line each for `signaling_url` and `bootstrap_url`.

### F8 One app on many nodes, run by the supervisor

**Message:** Describe an app once. Spread its services over nodes. The supervisor keeps
it healthy and renews its certificates.

Needs: main story done. S1 has the supervisor role (1.3).

1. Write an inventory (which nodes exist) and an app manifest:

```bash
mkdir -p "$DEMO_HOME/app" && cd "$DEMO_HOME/app"

cat > substrates.toml <<EOF
[substrates.alice]
did = "$S1_DID"
api_url = "$REGISTRY"
identity = "owner"

[substrates.bob]
did = "$S2_DID"
api_url = "$REGISTRY"
identity = "owner"
EOF

cat > duo.toml <<EOF
id = "syneroym:duo"
version = "0.1.0"

[services.front]
service_type = "wasm"
source = "$GREETER_WASM"
interfaces = ["$GREETER_IFACE"]
visibility = "internal"
[services.front.placement]
substrate = "alice"

[services.back]
service_type = "wasm"
source = "$GREETER_WASM"
interfaces = ["$GREETER_IFACE"]
visibility = "internal"
replicas = 2
[services.back.placement]
substrate = "bob"
EOF
```

2. Deploy by hand first, to show the idea:

```bash
roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --as owner app deploy duo-1 duo.toml \
  --mint-masters --registry-url "$REGISTRY" --inventory substrates.toml \
  --journal-path "$DEMO_HOME/app/deployments.db"
rc1 svc list ; rc2 svc list
```

**Expect:** `front` on S1, two `back` members on S2.

3. Health:

```bash
roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --as owner app health duo-1 \
  --inventory substrates.toml --journal-path "$DEMO_HOME/app/deployments.db"
```

4. Hand the app to the supervisor (instead of manual deploys):

```bash
rc1 supervisor submit duo-2 duo.toml --inventory substrates.toml
rc1 supervisor adopt duo-2
rc1 supervisor status duo-2
rc1 supervisor alerts duo-2
```

**Say:** "From now on the supervisor re-checks every 10 seconds, restarts what stopped,
and renews certificates before they expire."

5. Find who answers for a logical service:

```bash
APP_DID=$(rc1 supervisor status duo-2 | grep -oE '"app_master_did": *"did:key:[a-z0-9]+"' | grep -oE 'did:key:[a-z0-9]+')
roymctl --api-url "$REGISTRY" --dir "$S1_DIR" --as owner app resolve "$APP_DID" back
```

**Expect:** the member DIDs of `back`.

**Rehearse:** this whole story. Known gap: a supervisor on S1 that manages S2 needs
**a grant from S2's owner to S1's node DID, covering both `orchestrator/deploy` and
`orchestrator/status`**. The command line makes one ability per token, so this is not
possible from the CLI yet (see the backlog). A possible way around it for a demo is one
token with `--can substrate/admin` (whole-node rights; broad, demo only), placed in the
inventory as `ucan = ...` for `bob`. Until this is tried, show `duo-1` (manual deploy,
steps 1 to 3) and keep the supervisor steps (4 and 5) for a single node: put both services
on `alice`. `supervisor` commands only need `--substrate`; `rc1` sends the same flags harmlessly.

### F9 Scheduled jobs

**Message:** The supervisor can run a method on a schedule (cron, UTC).

Needs: F8 set-up (supervisor on S1).

```bash
cat > "$DEMO_HOME/app/tick.toml" <<EOF
id = "syneroym:tick"
version = "0.1.0"

[services.worker]
service_type = "wasm"
source = "$SCHED_WASM"
interfaces = ["$SCHED_IFACE"]
visibility = "internal"

[services.worker.placement]
substrate = "alice"

[services.worker.schedule]
cron = "* * * * *"
interface = "$SCHED_IFACE"
method = "tick"
EOF

rc1 supervisor submit tick-1 "$DEMO_HOME/app/tick.toml" --inventory "$DEMO_HOME/app/substrates.toml"
rc1 supervisor adopt tick-1
sleep 70
rc1 supervisor schedules tick-1
```

**Expect:** `last-run-at` is filled in. Wait another minute and it moves.

### F10 Durable calls and sagas  *(deferred, not scripted)*

**Message:** Calls between services survive crashes. A multi-step workflow can undo
itself. These are visible to an operator through three commands:

```bash
rc1 svc proxy-outbox       --svc-id <SERVICE-DID>  # queued calls waiting to be sent
rc1 svc proxy-dead-letters --svc-id <SERVICE-DID>  # calls the node gave up on
rc1 svc sagas              --svc-id <SERVICE-DID>  # workflows and their state
```

Not written yet. The test fixtures are `test-components/proxy-test` and
`test-components/saga-test`. The runnable scenarios to copy from are
`crates/substrate/tests/proxy_outbox_e2e.rs` (a queued call to an offline node lands after
it returns) and `crates/substrate/tests/saga_e2e.rs` (a failed workflow is undone in
reverse order). Both deploy their driver service with `--master <name> --registry-url`, so it
holds an instance certificate, and both call methods with named JSON parameters, for example
`begin-workflow` with `{"deadline-secs": 3600}`. Tracked in the backlog.

### F11 Row and column level security

**Message:** A service can declare, in one JSON file, who may see which rows. The
platform enforces it, even across nodes. The application code has no `if user == ...` checks.

Needs: main story done. About 8 minutes.

**1. Show the policy.** It is a small file. This one lets a person read only their own
employee row, and lets a seeding role write.

```bash
mkdir -p "$DEMO_HOME/app"
cat > "$DEMO_HOME/app/fdae-policy.json" <<'POLICY'
{
  "version": "fdae/v1",
  "definitions": {
    "employee": {
      "table": "employees",
      "principal_column": "did",
      "resolvable_without_capability": true,
      "permissions": {
        "view_self": {"allows": ["data-layer/read"], "paths": [["caller"]]},
        "seed":      {"allows": ["data-layer/write"], "paths": []}
      }
    }
  }
}
POLICY
cat "$DEMO_HOME/app/fdae-policy.json"
```

**Say:** "`principal_column` says which column holds a person's identity. `paths` says how
the caller must be related to the row. No code."

**2. Attach it to a service at deploy time** (through an app manifest):

```bash
cat > "$DEMO_HOME/app/records.toml" <<EOF
id = "syneroym:records"
version = "0.1.0"

[services.records]
service_type = "wasm"
source = "$GREETER_WASM"
interfaces = ["$GREETER_IFACE"]

[services.records.fdae]
policy = "$DEMO_HOME/app/fdae-policy.json"
EOF

roymctl --dir "$S1_DIR" --api-url "$REGISTRY" --substrate "$S1_DID" --as owner \
  app deploy records-1 "$DEMO_HOME/app/records.toml" \
  --journal-path "$DEMO_HOME/app/records-deployments.db"
```

**Expect:** the deploy succeeds. A broken policy would be refused at this point, so a
mistake shows up at deploy time and not when someone is wrongly denied access.

**3. See it enforced.** There is no command-line "query as this person" verb yet. The
live demonstration is an existing end-to-end test. It starts two real nodes:
- Node A holds an HR table of employees.
- Node B holds a `documents` table. Its policy says "you may read a document if you own it, and
  *ownership is checked by asking node A*."
- Alice queries node B. She gets back **only her own document**. The ownership check crossed
  nodes in the middle of the query.

```bash
cd "$REPO"
cargo nextest run -p syneroym-substrate --test federated_fdae_e2e --no-capture
```

**Expect:** the test passes. **Say:** "The database filtered the rows. The app asked for
all documents and got only the ones Alice may see."

> **Rehearse:** (a) how long the test takes and how noisy its output is. Redirect to a file
> and show only the last lines if needed. (b) Whether `--no-capture` adds value.
> (c) Whether the policy file above deploys as written (it is copied from the test's
> policy, minus the second definition).
> **Gap:** a `roymctl svc call`-style verb, able to present a person's capability token,
> would let this story run live from the terminal. Tracked in the backlog.

### F12 Messaging (publish / subscribe)

**Message:** Every service gets a built-in message bus. Publishers and subscribers do
not know each other. Delivery is pushed, fast, and works across the network.

Needs: main story 2.5 done (the web app is a publisher and a subscriber).

**Part A, in the browser (2 min).** In window A, on **Comments etc.**, both live
indicators ("WebSocket" and "SSE") are fed by one topic, `comment-updates`. Post a comment
in window B and watch both indicators change in window A.

**Part B, with `curl` (3 min).** Subscribe in one terminal, publish in another.

```bash
# T5: subscribe. Leaves the connection open and prints events as they arrive.
WEB_HOST=$(host_of "$WEB_DID" --nickname webapp --interface http-native)
curl -N "http://127.0.0.1:$S1_GW/api/events" -H "Host: $WEB_HOST"
```

```bash
# T3: publish, by posting a comment. The app publishes the event.
WEB_HOST=$(host_of "$WEB_DID" --nickname webapp --interface http-native)
curl -s -X POST "http://127.0.0.1:$S1_GW/api/comments" \
  -H "Host: $WEB_HOST" -H 'Content-Type: application/json' \
  -d '{"text":"hello from curl"}'
```

**Expect:** an event line appears in T5 within a moment of the POST.
Now subscribe from the **other** node: run the T5 command against `$S2_GW`. Post again.
**Say:** "The subscriber is on a different node. The registry and relay found the path."

**Part C, native speed (3 min).** The platform's own subscriber test measures
push latency. Stop S1 and S2 first: this one test uses fixed local ports (7970 to 7974).

```bash
# T1 and T2: Ctrl-C both substrates. Then:
cd "$REPO"
cargo nextest run -p syneroym-substrate --test messaging_client_e2e --no-capture 2>&1 | grep -E "latency|PASS|FAIL"
```

**Expect:** a line like `native messaging-subscriber delivery latency: p99=... (n=20)`
and a PASS. The design budget is 5 ms at the 99th percentile. Start S1 and S2 again after this
(section 1.3) and inject the KEK.

> **Rehearse:** (a) the SSE route (`/api/events`) is not marked public in `routes.json`. In
> `login` mode it may need a session. If `curl` is refused, use the browser (Part A) or
> add an `Authorization: Bearer` header (see F4). (b) The `grep` may hide the latency line
> if nextest prints it differently.
> **Also available:** the supervisor publishes alerts on the topic
> `supervisor/alerts/<app-instance>`. There is no command-line subscriber yet.

### F13 Operations: health, metrics, smoke tests, benchmarks

```bash
curl -s "http://127.0.0.1:$S1_HEALTH/health"; echo
curl -s "http://127.0.0.1:$S1_METRICS/metrics" | head -20
```

**Expect:** health says healthy; metrics are Prometheus text.

Smoke test against the cloud machine from the Mac:

```bash
cd "$REPO" && mise run test:smoke -- --coordinator-url "http://$CLOUD_IP:7964"
```

**Rehearse:** the flag takes the coordinator base URL. The docs use an HTTPS domain
example. Check which URL (7964 or 7961) the smoke test needs.

Benchmarks (long; run before the talk and show the numbers):

```bash
mise run bench:latency
```

TLS hot reload (only if the cloud machine has a certificate):

```bash
kill -USR1 $(pgrep syneroym-substrate)     # on the machine that holds the cert
```

---

## 4. Roym

**Message:** Roym is a product built on Syneroym. It is six services that work
together: profile, conversation, catalog, transaction, directory and a web Hub. One
person can offer a service, another can find it, talk, get a quote and agree.

Two installations: **alice** on S1 and **bob** on S2. They talk to each other through
the cloud registry and relay.

### R0 Deploy Roym on both nodes

```bash
cd "$REPO"
for n in 1 2; do
  if [ $n = 1 ]; then D="$S1_DIR"; DID="$S1_DID"; else D="$S2_DIR"; DID="$S2_DID"; fi
  roymctl --dir "$D" --api-url "$REGISTRY" --substrate "$DID" --as owner \
    app deploy roym crates/roym_core/app/roym.toml \
    --mint-masters --registry-url "$REGISTRY" \
    --journal-path "$D/roym-deployments.db"
done
```

**Expect:** six services deployed on each node. (Run the file paths from `$REPO`:
the manifest paths are relative to the repository root.)

Give each Hub a gateway name, then read the host names:

```bash
roymctl --dir "$S1_DIR" --api-url "$REGISTRY" registry register \
  --identity "member-roym#web-0" --substrate "$S1_DID" --nickname roym
roymctl --dir "$S2_DIR" --api-url "$REGISTRY" registry register \
  --identity "member-roym#web-0" --substrate "$S2_DID" --nickname roym

ROYM1_DID=$(did_of "$S1_DIR" "member-roym#web-0")
ROYM2_DID=$(did_of "$S2_DIR" "member-roym#web-0")
ROYM1_HOST=$(host_of "$ROYM1_DID" --nickname roym --interface http-native)
ROYM2_HOST=$(host_of "$ROYM2_DID" --nickname roym --interface http-native)
echo "alice Hub: http://$ROYM1_HOST:$S1_GW"
echo "bob   Hub: http://$ROYM2_HOST:$S2_GW"
```

Sign in material, and the "can sign records" step, for each person:

```bash
roymctl --dir "$S1_DIR" --as owner session delegate --registry-url "$REGISTRY" --out "$S1_DIR/alice-session-key.json"
roymctl --dir "$S2_DIR" --as owner session delegate --registry-url "$REGISTRY" --out "$S2_DIR/bob-session-key.json"

roymctl --dir "$S1_DIR" --as owner session login --gateway-url "http://127.0.0.1:$S1_GW" --registry-url "$REGISTRY"
roymctl --dir "$S1_DIR" --as owner roym enrol-signing --master owner \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST" --registry-url "$REGISTRY"

roymctl --dir "$S2_DIR" --as owner session login --gateway-url "http://127.0.0.1:$S2_GW" --registry-url "$REGISTRY"
roymctl --dir "$S2_DIR" --as owner roym enrol-signing --master owner \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST" --registry-url "$REGISTRY"

roymctl --dir "$S1_DIR" --as owner roym signing-status --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
```

**Expect:** one line per service that signs (profile, catalog, conversation) saying it is enrolled.
**Say:** "Enrolment lets each service sign records for me. Records carry proof of who made them."

> **Rehearse:** the identity named `owner` is used as the person on both nodes (each
> node has its own `owner`). The test setup does the same. The Hub must be open on a
> `*.localhost` host name: browsers treat it as a secure context, which the Hub login
> needs. Do not open the Hub through the cloud IP.

### R1 Sign in and profile

1. Window A: open `http://$ROYM1_HOST:$S1_GW`. Window B: `http://$ROYM2_HOST:$S2_GW`.
2. On each, choose **Sign in**, and select the matching `session-key.json` file
   (`$S1_DIR/alice-session-key.json`, `$S2_DIR/bob-session-key.json`).
3. **Expect:** the top bar shows the person's `did:key:...` and "delegated".
4. Go to **Profile**. Set a display name. **Expect:** a signed record ID appears.

**Say:** "The Hub never gets my master key. It holds a temporary one, in the browser."

### R2 Listings (catalog)

In alice's Hub, **Listings** tab: create a listing (for example, "Garden work", with a
category and a service area). Save.
**Expect:** the listing is saved and signed.

### R3 Messages between two installations

Each person must know the other's conversation address.

```bash
roymctl --dir "$S1_DIR" roym address       # prints alice's conversation service id
roymctl --dir "$S2_DIR" roym address       # prints bob's
```

In each Hub, **Profile** tab: put that value in **conversation address** and save. Then
in **Contacts**, add the other person. In **Messages**, send a message from alice to bob.

**Expect:** the message shows "pending" and then arrives in bob's Hub.
**Say:** "Direct, signed messages. No central chat server."

**Rehearse:** exact tab labels and where the other person's DID or address is entered.
The Hub tests (`crates/substrate/tests/e2e/tests/roym-hub.spec.ts`, tests 5, 6, 10) are the
reference for the clicks.

### R4 Request → quote → agreement

Bob asks for work. Alice answers with a price. Bob accepts. Both hold a signed receipt.

In the Hub **Messages** tab: bob sends a **request** card; alice sends a **quote**
card; bob presses **Accept**. Then show the receipt.

CLI equivalent (from a person's terminal):

```bash
# bob
roymctl --dir "$S2_DIR" --as owner roym transaction request \
  --conversation <CONVERSATION-ID> --description "Mow the lawn" \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
# alice
roymctl --dir "$S1_DIR" --as owner roym transaction quote --request <REQUEST-ID> \
  --scope "Mow the lawn" --currency INR --amount 500 --payee <ALICE-DID> \
  --timing "this week" --where "at your home" \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
# bob
roymctl --dir "$S2_DIR" --as owner roym transaction accept --quote <QUOTE-ID> \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
roymctl --dir "$S2_DIR" --as owner roym transaction agreement --quote <QUOTE-ID> \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
```

**Rehearse:** the ids (`<CONVERSATION-ID>`, `<REQUEST-ID>`, `<QUOTE-ID>`) come from the
previous command's output or `roym transaction thread`. The required flags for `quote`
were read from `--help`; check the values accepted for `--timing` and `--where`.

### R5 Directory and SynOrg (a group of providers)

**Message:** A *SynOrg* is a guild or association. It runs a *Directory*. People add a
directory as a source and search many directories at once. Directories are optional.

Alice runs a SynOrg; bob searches it.

```bash
printf 'Be kind. Keep your listings honest.\n' > "$DEMO_HOME/tmp/rules.txt"
roymctl --dir "$S1_DIR" --as owner roym directory serve \
  --name "Garden Guild" --rules-file "$DEMO_HOME/tmp/rules.txt" \
  --category gardening --support support@example.org --dispute "Write to support." \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"

ROYM1_DIRECTORY_DID=$(did_of "$S1_DIR" "member-roym#directory-0")
roymctl --dir "$S1_DIR" --as owner roym directory publish <LISTING-ID> --to "$ROYM1_DIRECTORY_DID" \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"

roymctl --dir "$S2_DIR" --as owner roym directory add "$ROYM1_DIRECTORY_DID" --label "Garden Guild" \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
roymctl --dir "$S2_DIR" --as owner roym directory find --category gardening \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
```

**Expect:** bob's search shows alice's listing, its source, its age, and two honest
"unknown" lines: `revocation: unknown`, `membership: not checked`.
**Say:** "The tool tells you what it could and could not check. It never says 'verified'
when it did not verify."

Membership (optional): alice issues a credential to a member, and bob sees the
membership checked on his own node:

```bash
roymctl --dir "$S1_DIR" --as owner roym directory credential issue --help
roymctl --dir "$S1_DIR" --as owner roym directory member add <BOB-PERSON-DID> \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
```

**Rehearse:** `credential issue` flags (run `--help`). The browser version of this is
in `crates/substrate/tests/e2e/tests/roym-trust.spec.ts`. The Hub also has **Directory** and
**SynOrg** tabs that do the same thing.

### R6 Group chat

```bash
roymctl --dir "$S1_DIR" --as owner roym group create --name "Garden friends" \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
roymctl --dir "$S1_DIR" --as owner roym group add --group <GROUP-ID> \
  --address <BOB-CONVERSATION-ADDRESS> --person-did <BOB-PERSON-DID> \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
roymctl --dir "$S1_DIR" --as owner roym group send --group <GROUP-ID> --body "Hello all" \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
roymctl --dir "$S2_DIR" --as owner roym group sync --group <GROUP-ID> \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
roymctl --dir "$S2_DIR" --as owner roym group history --group <GROUP-ID> \
  --gateway-url "http://127.0.0.1:$S2_GW" --host "$ROYM2_HOST"
```

**Expect:** bob's history shows alice's message. The Hub has a **Groups** tab for the same flow.
**Rehearse:** the group id and how bob learns it (from `roym group list`).

### R7 Backup and restore

**Message:** A person can back up their identity and data, encrypted, and restore it
on another machine.

```bash
roymctl --dir "$S1_DIR" --as owner roym backup create \
  --master "$S1_DIR/identities/owner.key" --out "$DEMO_HOME/tmp/alice-backup.bin" \
  --recovery-key-out "$DEMO_HOME/tmp/alice-recovery.key" \
  --gateway-url "http://127.0.0.1:$S1_GW" --host "$ROYM1_HOST"
ls -l "$DEMO_HOME/tmp/alice-backup.bin"
```

**Expect:** an encrypted file. **Say:** "Without the recovery key nobody can open it,
including us."

**Rehearse:** `--master` takes a path. Restore (`restore-identity`, `restore-data`) is
best shown from a clean directory; see `crates/substrate/tests/roym_restore_e2e.rs`.

---

## 5. Reset and teardown

> *Manual for now. To be scripted later.*

**Between two demos (same machines):** stop and wipe the Mac nodes, keep the cloud.

```bash
# T1 and T2: Ctrl-C the substrates. Then in T5:
jobs; kill %1 2>/dev/null          # stop the python server from F1, if running
rm -rf "$S1_DIR" "$S2_DIR" "$DEMO_HOME/app" "$DEMO_HOME/tmp"
```

Then redo [1.3](#13-thirty-minutes-before-create-and-start-s1-and-s2-on-the-mac).

The registry on the cloud keeps old records of the deleted nodes. They are harmless,
but to start with an empty registry too:

```bash
# on the cloud machine: Ctrl-C the substrate in tmux, then
rm -rf $DEMO_HOME/cloud/*.db $DEMO_HOME/cloud/db 2>/dev/null   # then redo 1.2
```

**Rehearse:** where the registry stores its data. If unsure, delete the whole
`$DEMO_HOME/cloud` directory and redo 1.2 (the cloud node gets a new key, which is fine).

**Restart without wiping (for example a crash):** start the substrate again with the
same command from 1.3, then **inject the KEK again**: `rc1 kek inject "$KEK"`.

---

## 6. If something breaks

| Symptom | Most likely cause | Fix |
|---|---|---|
| `curl` to `$REGISTRY` returns `000` | Firewall, or the cloud process is down | Open TCP 7961-7964. Check `tmux` on the cloud machine. |
| Nodes start but cannot reach each other across the cloud | UDP 7965 blocked | Open **UDP** 7965. |
| `svc deploy` of a WASM service fails, error mentions keys or vault | KEK not injected (after every restart) | `rc1 kek inject "$KEK"` |
| `rc1 ...` says "unauthorized" or sees nothing | Wrong `--as`, or the node is not claimed | Use the `rc1` helper. Check `agreement.json` exists in `$S1_DIR`. Restart the node after `claim`. |
| `rc1` fails with an empty `--substrate` | `demo_dids` was not run in this terminal | `source ~/syneroym-demo.env; demo_dids` |
| A host name does not reach the service | Alias built with a different nickname or interface | Rebuild it with `host_of`, using the same `--nickname` as in `registry register`. |
| Browser page on Path A never loads | `nip.io` not resolving, or WebRTC blocked | Use Path B (local gateway). Or `?force_tunnel=true`. |
| Hub login button does nothing | Hub opened on a non-secure origin | Use the `*.localhost` host name, not the cloud IP. |
| S2 cannot reach a service on S1 | Service not registered in the registry | `registry register ...` for that service (step 2.2). |
| Strange failures, "address already in use", "os error 55" | Old processes still running | `lsof -iTCP -sTCP:LISTEN -n -P | grep -E '79[0-9]{2}'` and kill them. On macOS, `netstat -m` shows buffer exhaustion. |
| Everything is slow | Debug build | Use `--release` binaries (`BIN` in the env file). |

**Fallbacks that keep the show going**

- Path A (cloud WebRTC) fails → Path B (local gateway). The story still works.
- Cloud machine unreachable → run C on the Mac too: use the `[roles.coordinator]` and
  `[roles.community_registry]` tables from 1.2 in a third local node, and set
  `CLOUD_IP=127.0.0.1` with different ports. Cross-machine claims are then not shown.
- A single step is stuck → skip to the next story. Every feature story is independent.

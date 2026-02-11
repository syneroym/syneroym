# Phase 1 Executable Demo Runbook

This runbook defines the step-by-step procedures to execute the "Substrate Baseline" demo. It validates the core behaviors scoped in [Phase 1 Scope](./03-phase-1-scope.md) using the conceptual model defined in [System Model](./04-system-model.md).

## Prerequisites

- **Syneroym CLI**: Built from source. The binary (`syneroym`) acts as the client, host runner, and controller.
- **Environment**: Three distinct terminal sessions (or machines) to simulate network separation:
    - **Terminal 1**: Substrate Instance A (Host Control) - *Daemon Mode*
    - **Terminal 2**: Substrate Instance B (Host Control / Backup) - *Daemon Mode*
    - **Terminal 3**: Substrate Instance C (Controller / Service Owner) - *Client Mode*
- **Artifacts**: A feature-rich WASM module: `demo-suite.wasm`.
    - *Capabilities*: Static file serving, Request/Response (RPC), Event Streaming, WebSockets.

---

## Scenario 0: Local Development (Loopback)

**Goal**: Quickly test the `demo-suite.wasm` on a single machine without complex networking.

### Step 0.1: Run in Dev Mode
This command spins up an ephemeral Host and Controller in memory, deploys the app, and proxies the output to your terminal.

```bash
$ syneroym dev run ./demo-suite.wasm
> [INFO] Starting ephemeral host...
> [INFO] Deploying demo-suite...
> [INFO] App running at: http://localhost:3000
> [LOG] [demo-suite] System initialized.
```
*(Open http://localhost:3000 in your browser to verify the mini-app UI)*

---

## Scenario 1: Host Onboarding

**Goal**: Initialize two Substrate Instances with **Host Control** enabled and make them discoverable via the Relay/DHT network.

### Step 1.1: Initialize Instance A (Terminal 1)
Configure the local environment for the primary host.
*Note: `--profile host-a` is a local configuration alias. The network identity is derived from cryptographic keys.*

```bash
# Initialize configuration for a host node
$ syneroym config init --profile host-a

# Set resource caps
$ syneroym config set resources.cpu=2 resources.memory=512MB --profile host-a

# Enable "Demo Mode" auto-consent
$ syneroym config set policy.auto_consent=true --profile host-a

# Start the instance process
$ syneroym daemon run --profile host-a
> [INFO] Syneroym Substrate Instance v0.1.0
> [INFO] Identity: did:key:zHostA...
> [INFO] Discovery: Connected to Relay (region: us-east)
> [INFO] Management Socket: /tmp/syneroym-host-a.sock (Listening for CLI commands)
```

### Step 1.2: Generate Invitation (Terminal 1 - New Tab)
Generate an Out-of-Band (OOB) Ticket that contains the Host's ID, Relay info, and direct IP addresses.

```bash
# Target the local running instance via socket
$ syneroym host invite --profile host-a --valid-for 1h
> Generated OOB Ticket:
> syn://invite/host-a?key=zHostA...&relay=...&addrs=...
```
*(Copy this ticket for Step 3.1)*

### Step 1.3: Initialize Instance B (Terminal 2)
Repeat for the backup host.

```bash
$ syneroym config init --profile host-b
$ syneroym config set policy.auto_consent=true --profile host-b
$ syneroym daemon run --profile host-b
> [INFO] Identity: did:key:zHostB...
> [INFO] Discovery: Connected to Relay.
```

*In a split pane:*
```bash
$ syneroym host invite --profile host-b
> Generated OOB Ticket:
> syn://invite/host-b?key=zHostB...
```
*(Copy this ticket for Step 4.2)*

---

## Scenario 2: Service Preparation

**Goal**: Prepare the Controller and package the comprehensive demo suite.

### Step 2.1: Initialize Controller (Terminal 3)
Configure the client-side environment.
*Note: The Controller holds the stable Master Identity (`did:key:zMaster`) which never leaves this machine.*

```bash
$ syneroym config init --profile controller
> Identity created: did:key:zMaster...
> Context set to "controller"
```

### Step 2.2: Package Peer Bundle (Terminal 3)
Create a manifest for the demo suite.

```bash
$ syneroym bundle init --name "demo-app" --wasm ./demo-suite.wasm
> Created syneroym.toml

$ syneroym bundle pack
> Packed peer bundle: demo-app.syn (hash: QmBundleHash...)
> Peer Identity: did:key:zMaster...
```

---

## Scenario 3: Service Deployment (Primary)

**Goal**: Deploy the peer to Instance A using the **Warrant/Delegation** model.

### Step 3.1: Add Host Context (Terminal 3)
Register Instance A as a remote context.

```bash
$ syneroym remote add host-a <OOB_TICKET_FROM_HOST_A>
> Verifying identity... OK.
> Added remote context "host-a" (did:key:zHostA...).
```

### Step 3.2: Negotiate Consent (Terminal 3)
Request permission to run the peer.

```bash
$ syneroym host request-consent --remote host-a --bundle demo-app.syn
> Requesting consent...
> Status: GRANTED (Auto-approved)
```

### Step 3.3: Deploy with Warrant (Terminal 3)
Upload the bundle and issue a Delegation Warrant.
*The Host generates a temporary Session Key. The Controller signs a Warrant authorizing that key for 6 hours.*

```bash
$ syneroym peer deploy --file demo-app.syn --remote host-a
> Uploading bundle... Done.
> Host generated Session Key: did:key:zSessionHostA...
> Signing Warrant with Master Key (valid 6h)... Done.
> Sending Warrant to Host... Done.
> Peer "demo-app" running on "host-a".
> Public Address: syn://did:key:zMaster...
```

### Step 3.4: Verify Resource Enforcement (Terminal 1)
Confirm the host is enforcing limits.

```bash
$ syneroym host inspect --profile host-a --peer did:key:zMaster...
> Peer Status: RUNNING (Delegated via Warrant)
> Resources:
>   CPU: 0.1% / 200%
>   Memory: 45MB / 512MB
```

---

## Scenario 4: Service Consumption & Capabilities

**Goal**: Verify the `demo-suite` functionality (RPC, Streams, etc.).

### Step 4.1: Standard RPC (Terminal 3)
Call a simple Request/Response method.

```bash
$ syneroym call syn://did:key:zMaster... --method echo --args "Hello Syneroym"
> Connecting via Relay... Connected.
> Verifying Warrant... Valid.
> Result: "Echo: Hello Syneroym"
```

### Step 4.2: Stream Data (Terminal 3)
Test large file streaming (upload).

```bash
$ syneroym call syn://did:key:zMaster... --method upload_file --input ./large-image.png
> Streaming... 100%
> Result: "File received: 5MB"
```

### Step 4.3: Real-time Message (Terminal 3)
Simulate a WebSocket-like event subscription.

```bash
$ syneroym listen syn://did:key:zMaster... --event "system-status"
> [Event] cpu_load: 12%
> [Event] memory_usage: 45MB
> (Ctrl+C to exit)
```

---

## Scenario 5: Revocation and Recovery

**Goal**: Simulate host failure and migration.

### Step 5.1: Revoke Consent (Terminal 1)
Host A goes down or revokes access.

```bash
$ syneroym host revoke --profile host-a --peer did:key:zMaster...
> Revocation issued. Peer stopped.
```

### Step 5.2: Redeploy to Instance B (Terminal 3)
Migrate to Host B. A **new** Session Key and Warrant will be generated.

```bash
$ syneroym remote add host-b <OOB_TICKET_FROM_HOST_B>
$ syneroym peer deploy --file demo-app.syn --remote host-b --force
> Host generated Session Key: did:key:zSessionHostB...
> Signing Warrant... Done.
> Peer "demo-app" running on "host-b".
```

### Step 5.3: Verify Recovery (Terminal 3)
Call the *same* Master Identity. The discovery layer resolves the new route.

```bash
$ syneroym call syn://did:key:zMaster... --method echo --args "Am I back?"
> Resolving peer... Found on host-b.
> Verifying Warrant... Valid.
> Result: "Echo: Am I back?"
```

---

## Scenario 6: Lifecycle Management

**Goal**: Suspend and Resume.

### Step 6.1: Suspend Peer (Terminal 3)
```bash
$ syneroym peer suspend --remote host-b --peer did:key:zMaster...
> Peer suspended.
```

### Step 6.2: Resume Peer (Terminal 3)
```bash
$ syneroym peer resume --remote host-b --peer did:key:zMaster...
> Peer resumed.
```

---

## Scenario 7: Proxying Existing Services

**Goal**: Expose a legacy local service.

### Step 7.1: Start Local Service (Terminal 2)
```bash
$ python3 -m http.server 8080
```

### Step 7.2: Deploy Proxy (Terminal 3)
Create a proxy bundle. Note that for proxies, the Host acts as a gateway.

```bash
$ syneroym bundle init --name "local-proxy" --proxy "http://localhost:8080"
$ syneroym peer deploy --file local-proxy.syn --remote host-b
> Peer "local-proxy" running on "host-b".
```

### Step 7.3: Access Proxy (Terminal 3)
```bash
$ syneroym call syn://did:key:zProxyMaster... --method GET --args "/"
> Result: "<!DOCTYPE HTML PUBLIC ... (Directory listing)"
```

---

## Scenario 8: Peer Removal

**Goal**: Permanently remove deployment.

### Step 8.1: Remove Peer (Terminal 3)
```bash
$ syneroym peer remove --remote host-b --peer did:key:zMaster...
> Stopping peer...
> Removing artifacts...
> Peer removed.
```

---

## Success Validation Checklist

- [ ] **Local Dev**: `syneroym dev run` works instantly (Scenario 0).
- [ ] **Discovery**: Daemon publishes to Relay; Clients connect via Ticket/DID.
- [ ] **Security (Warrants)**:
    - Master Key stays on Controller.
    - Host signs traffic with Session Key.
    - Consumers verify the Warrant chain.
- [ ] **Capabilities**: `demo-suite.wasm` successfully handles RPC, Streams, and Events.
- [ ] **Migration**: Identity persists (`zMaster`) while physical hosts change.

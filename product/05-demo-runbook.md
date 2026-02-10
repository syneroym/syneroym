# Phase 1 Executable Demo Runbook

This runbook defines the step-by-step procedures to execute the "Substrate Baseline" demo. It validates the core behaviors scoped in [Phase 1 Scope](./03-phase-1-scope.md) using the conceptual model defined in [System Model](./04-system-model.md).

## Prerequisites

- **Syneroym CLI**: Built from source. The binary (`syneroym`) acts as both the client and the substrate instance runner.
- **Environment**: Three distinct terminal sessions (or machines) to simulate network separation:
    - **Terminal 1**: Substrate Instance A (Host Control) - *Daemon Mode*
    - **Terminal 2**: Substrate Instance B (Host Control / Backup) - *Daemon Mode*
    - **Terminal 3**: Substrate Instance C (Service Control) - *Client Mode*
- **Artifacts**: A sample WASM module (e.g., `hello-world.wasm`).

---

## Scenario 1: Host Onboarding

**Goal**: Initialize two Substrate Instances with **Host Control** enabled and make them discoverable.

> **Note on Discovery**: Phase 1 supports a fallback order of OOB Token -> PKARR -> DNS. This demo primarily uses **Out-of-Band (OOB) Tokens** to ensure connectivity in isolated environments without relying on external DNS or DHT propagation delays.

### Step 1.1: Initialize Instance A (Terminal 1)
Configure the local environment for the primary host.

```bash
# Initialize configuration for a host node
$ syneroym config init --profile host-a

# Set resource caps (simulating a small slice)
$ syneroym config set resources.cpu=2 resources.memory=512MB --profile host-a

# Enable "Demo Mode" auto-consent (for streamlined demo execution)
# In production, this would default to "manual" requiring explicit approval.
$ syneroym config set policy.auto_consent=true --profile host-a

# Start the instance process
$ syneroym daemon run --profile host-a
> [INFO] Syneroym Substrate Instance v0.1.0
> [INFO] Identity: did:key:zHostA...
> [INFO] Listening on: /ip4/127.0.0.1/tcp/4001
> [INFO] Management Socket: /tmp/syneroym-host-a.sock
```

### Step 1.2: Generate Invitation for Instance A (Terminal 1 - New Tab)
Use the CLI to talk to the running instance to generate an invite token.

```bash
# Target the local running instance via socket or profile
$ syneroym host invite --profile host-a --valid-for 1h
> Generated OOB Token:
> syn://invite/host-a?key=...&addrs=...
```
*(Copy this token for Step 3.1)*

### Step 1.3: Initialize Instance B (Terminal 2)
Repeat the process for the backup host.

```bash
$ syneroym config init --profile host-b
$ syneroym config set resources.cpu=2 resources.memory=512MB --profile host-b
$ syneroym config set policy.auto_consent=true --profile host-b
$ syneroym daemon run --profile host-b
> [INFO] Syneroym Substrate Instance v0.1.0
> [INFO] Identity: did:key:zHostB...
```

*In a split pane or new tab:*
```bash
$ syneroym host invite --profile host-b
> Generated OOB Token:
> syn://invite/host-b?key=...
```
*(Copy this token for Step 4.2)*

---

## Scenario 2: Service Preparation

**Goal**: Prepare the Substrate Instance with **Service Control** enabled and package the peer application.

### Step 2.1: Initialize Controller (Terminal 3)
Configure the client-side environment. In this terminal, the CLI acts as the interface for the **Substrate Instance (Service Control)**.

```bash
$ syneroym config init --profile controller
> Identity created: did:key:zOwner...
> Context set to "controller"
```

### Step 2.2: Package Peer Bundle (Terminal 3)
Create a manifest and package the WASM artifact into a Syneroym Bundle (`.syn`).

```bash
# Create a manifest for the WASM module
$ syneroym bundle init --name "hello-peer" --wasm ./hello.wasm
> Created syneroym.toml

# Build the bundle (WASM + Manifest + Signature)
$ syneroym bundle pack
> Packed peer bundle: hello-peer.syn (hash: QmBundleHash...)
> Peer Identity: did:key:zPeer...
```

---

## Scenario 3: Service Deployment (Primary)

**Goal**: Deploy the peer to Instance A.

### Step 3.0: Security Sanity Check (Terminal 3)
Verify that invalid credentials are rejected.

```bash
# Attempt to add a host with a tampered token
$ syneroym remote add host-fake "syn://invite/host-fake?key=INVALID_KEY"
> Error: Signature verification failed. Token invalid.
```

### Step 3.1: Add Host Context (Terminal 3)
Register Instance A as a remote context using the valid OOB token.

```bash
$ syneroym remote add host-a <OOB_TOKEN_FROM_HOST_A>
> Verifying identity... OK.
> Added remote context "host-a" (did:key:zHostA...).
```

### Step 3.2: Negotiate Consent (Terminal 3)
Request permission to run the peer on Instance A.

```bash
$ syneroym host request-consent --remote host-a --bundle hello-peer.syn
> Requesting consent...
> Status: GRANTED (Auto-approved via policy.auto_consent=true)
```

### Step 3.3: Deploy Peer (Terminal 3)
Upload and start the peer.

```bash
$ syneroym peer deploy --file hello-peer.syn --remote host-a
> Uploading bundle... Done.
> Starting peer...
> Peer "hello-peer" running on "host-a".
> Public Address: syn://did:key:zPeer...
```

### Step 3.4: Verify Resource Enforcement (Terminal 1 - Instance A CLI)
Confirm the host is enforcing limits on the running peer.

```bash
$ syneroym host inspect --profile host-a --peer did:key:zPeer...
> Peer Status: RUNNING
> Resources:
>   CPU: 0.1% / 200% (Limit: 2 cores)
>   Memory: 12MB / 512MB (Limit: 512MB)
```

---

## Scenario 4: Service Consumption

**Goal**: Verify the deployed service is accessible.

### Step 4.1: Connect to Peer (Terminal 3 or any Client)
Interact with the running peer.

```bash
$ syneroym connect syn://did:key:zPeer...
> Connecting to peer... Connected via host-a.
> Sending "PING"
> Received "PONG from hello-peer running on host-a"
```

---

## Scenario 5: Revocation and Recovery

**Goal**: Simulate a host failure/revocation and migrate the service to Instance B.

### Step 5.1: Revoke Consent (Terminal 1 - Instance A CLI)
Simulate the host owner revoking access.

```bash
$ syneroym host revoke --profile host-a --peer did:key:zPeer...
> Revocation issued. Peer stopped.
```

### Step 5.2: Observe Failure (Terminal 3)
Try to connect again.

```bash
$ syneroym connect syn://did:key:zPeer...
> Error: Peer unreachable.
```

### Step 5.3: Add Instance B (Terminal 3)
Add the backup host using the token from Step 1.3.

```bash
$ syneroym remote add host-b <OOB_TOKEN_FROM_HOST_B>
> Added remote context "host-b".
```

### Step 5.4: Redeploy to Instance B (Terminal 3)
Migrate the workload.

```bash
# Force deployment to a new target
$ syneroym peer deploy --file hello-peer.syn --remote host-b --force
> Requesting consent... GRANTED.
> Uploading bundle... Done.
> Peer "hello-peer" running on "host-b".
```

### Step 5.5: Verify Recovery (Terminal 3)
Connect to the *same* peer identity, now running on a new host.

```bash
$ syneroym connect syn://did:key:zPeer...
> Resolving peer... Found on host-b.
> Connected.
> Received "PONG from hello-peer running on host-b"
```

---

## Success Validation Checklist

- [ ] **Instance/Client Split**: CLI correctly interacts with a separate instance process via socket/RPC.
- [ ] **Identities Verified**: All communications authenticated via DIDs; invalid tokens rejected (Step 3.0).
- [ ] **Caps Enforced**: Host inspection (Step 3.4) shows resource limits applied to the WASM runtime.
- [ ] **Migration Success**: Peer Identity persisted across host migration; only the transport route changed.
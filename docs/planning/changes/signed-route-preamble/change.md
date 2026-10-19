---
status: approved
living-docs-touched:
---
# Signed route preamble

*Reader: developers.*

A caller proves that it holds the key it names in the route preamble. A call
that passes through a coordinator is encrypted end to end. A service that
calls another service on behalf of a person passes that person's proof on.

Terms: the **route preamble** is the first line of every stream
(`<scheme>://<interface>.<service_id>?<params>`, `crates/router/src/preamble.rs`).
A **temporary key** is an Ed25519 key that a master DID delegates with a
**delegation certificate**. An **instance certificate** is the delegation
certificate of one deployed service instance (ADR-0020 §1). A **forwarder** is a
coordinator that accepts a stream for a service it does not host and passes it
to the host (`relay_to_next_hop`). See [TERMINOLOGY](../../../TERMINOLOGY.md).

Two slices of milestone M06D build this: C4b (Part 1) and C4c (Part 2). Both
come before C5. The order is in the
[M06D plan](../../milestones/M06D-reachability-and-deferred-delivery/task.md).
The trust rules are in [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
Amendment 2.

## Problem

The router checks a caller's delegation certificate: the signature, the
validity window, the scope, and revocation (`HandshakeVerifier::verify_preamble`).
It does not check that the caller holds the temporary key that the certificate
names. The preamble carries only the public key. So a certificate is a bearer
credential: whoever has a copy can act as its owner.

Copies are common. A service sends its instance certificate on every remote
call it makes (`ProxyRouter::invoke_remote_at`), so every node it ever called
holds one. The mailbox depends on this check. With a copy of a receiver's
certificate, a node can call `register`, `unregister`, `claim` and `ack` for
that receiver. Repeated `unregister` calls turn the mailbox off for one chosen
receiver. With a copy of a sender's certificate, a node can make deposits as
that sender, and fair eviction then removes that sender's messages. No message
is read, faked as delivered or lost (ADR-0026 A1), but this is a denial of
service against a chosen service.

Three related gaps:

- **No command revokes a person's temporary key.** The router refuses a key
  listed in `revoked_keys` of the master anchor, but
  `roymctl identity publish-anchor` always publishes an empty list. A stolen
  session key stays valid until its certificate expires.
- **A forwarder reads Rust calls in plaintext.** Each Iroh leg is encrypted,
  but the QUIC connection ends at the forwarder. Only the browser runs the
  end-to-end handshake (`enc=ecdh-p256`). The Rust client has no client side
  of it. The preamble field `pubkey` is the Ed25519 identity key in one place
  and the P-256 handshake key in another, so one stream cannot carry both.
- **Any node forwards.** On a local registry miss, every node calls
  `relay_to_next_hop`, not only coordinators. A substrate that no longer hosts
  a service forwards its streams and reads them in plaintext.

## Scope and non-goals

- In scope, Part 1 (C4b):
  - The caller signs the destination, the interface and the time with its key.
    The router requires this signature whenever the preamble names a key.
  - The handshake key moves from `pubkey` to a new field, `epk`.
  - Only coordinators forward. Any other node answers `not_hosted_here`.
  - A service in a call chain passes the original caller's proof on
    (`on_behalf`).
  - A `roymctl` command that revokes a temporary key.
- In scope, Part 2 (C4c):
  - The client side of the end-to-end handshake in Rust.
  - The client uses it when it does not dial the host directly.
  - A forwarder refuses a plaintext stream that names a key.
  - A key derivation function (KDF) over the handshake secret.
- Not in scope:
  - Replay protection inside the time window (nonce, replay cache). See
    *Rejected options*.
  - A hash of the request body in the signature.
  - End-to-end encryption on a direct connection. QUIC already protects it.
  - A UCAN grant from a person to a service at login. This is the general form
    of `on_behalf` for longer grants, and is not needed for the cases here.
  - The session-cookie path of the client gateway. A person's proof of
    possession happens at login there, and is unchanged.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| SP-1 | A preamble with a `pubkey` and no `sig`, or a `sig` that does not verify with that key, is refused. This holds with and without a `delegation`. | |
| SP-2 | A caller that presents another party's certificate and public key, without the private key, is refused at each mailbox method (`register`, `unregister`, `deposit`, `claim`, `ack`, `status`). | |
| SP-3 | A `ts` more than 15 minutes before or after the destination's clock is refused. A `ts` inside the window is accepted. | |
| SP-4 | A signature made for one `service_id` or `interface` is refused at another. | |
| SP-5 | The router reads the handshake key only from `epk`. A preamble that has a `pubkey` and an `epk` that differs from the signed `epk` is refused. The browser tunnel works with `epk`. | |
| SP-6 | A node that is not a coordinator and does not host the service answers `not_hosted_here` and does not forward. The SDK and `ProxyRouter` then look the service up again and reach the new host. | |
| SP-7 | Person P calls service A, A calls B, and B calls C, each on a different node. C sets `anchor_did` to P, and `resolve-relation(principal = P)` succeeds. C refuses the stream when P's proof has a `ts` outside the window or a forged signature. A guest cannot send `on_behalf`. A preamble with both `on_behalf` and `ucan` is refused. | |
| SP-8 | After `roymctl identity` revokes a temporary key and publishes the anchor, the router refuses that key. | |
| SP-9 | The Rust client runs the handshake when the endpoint it dials is not the host named in the target's record, or when the host is unknown. It does not run it on a direct connection. | |
| SP-10 | A call with an identity through a forwarder (C and Cp in `crates/coordinator_iroh/tests/multi_hop_relay.rs`) is encrypted end to end, and the forwarder cannot read or change it. A forwarder that swaps `epk` makes the stream fail. | |
| SP-11 | A forwarder refuses a stream that has a `pubkey` and no `enc` with `encryption_required`. | |
| SP-12 | Both sides derive the AES key with HKDF-SHA256. The browser and the Rust client both work with the router. | |

## Design

### Part 1 (C4b): the signature

**Signed bytes.** The caller signs these bytes with the Ed25519 key in `pubkey`:

```text
syneroym-preamble-v1\n<service_id>\n<interface>\n<ts>
```

When the preamble has `epk`, `\n<epk>` is added at the end. `service_id` and
`interface` are the values the caller sends. The router checks the signature
before it changes them: it rewrites an empty HTTP interface and maps the
interface to its canonical name later (`rewrite_empty_http_interface`,
`registry.lookup`).

**New preamble parameters.**

| Parameter | Value |
| --- | --- |
| `ts` | Unix seconds, decimal. |
| `sig` | The Ed25519 signature, 128 lowercase hex characters. |
| `epk` | The P-256 handshake key, lowercase hex of the SEC1 encoding (was `pubkey`). |
| `on_behalf` | The original caller's proof, hex of JSON (below). |

The pubkey is not in the signed bytes. It is the key that checks the
signature, and the certificate already names it. The caller signs these few
fields, not the raw preamble line, so a forwarder can keep writing the
preamble again as it does today (`to_preamble_line`).

**Window ±15 minutes.** This covers normal clock skew, phones included. The
router keeps no state: no nonce and no replay cache. *Accepted gaps* below says
what this allows.

**Required whenever the preamble has a `pubkey`,** with or without
`delegation`. There is no compatibility flag, because the product is not
released. A preamble without `pubkey` stays anonymous, as today. A `pubkey`
without a certificate now means "the caller holds this key". A `ucan` chain in
the same preamble is now presented by a proven key holder too.

**Who signs.** Each place that builds a preamble with a `pubkey` signs when it
writes the preamble, on every attempt:

- `SyneroymClient` (`crates/sdk/src/client.rs`). The client gateway and
  `roymctl` use it.
- `IrohHop::call`, for `ProxyRouter::invoke_remote_at`
  (`crates/router/src/proxy/`). It signs with the service instance key or the
  node key, which the node holds.
- `roymctl session` (`apps/roymctl/src/commands/session.rs`).
- *Unverified:* that this list is complete. Search for every writer of
  `pubkey` before building.

The retries of `invoke_remote_at` stay inside one call timeout, so one
signature would fit the window. Signing again costs little and has no edge
case.

**`epk`.** `pubkey` always means the Ed25519 identity key. Today
`route_handler/encryption.rs` reads `pubkey` as the P-256 key when
`enc=ecdh-p256`, and `crates/coordinator_webrtc/templates/peer-proxy.js` puts
its P-256 key there. Both change to `epk`. Because `epk` is in the signed
bytes, a forwarder that swaps in its own handshake key breaks the signature.
Without that, a forwarder could run the handshake itself and talk to the
destination under the caller's identity. The browser sends `epk` and no
`pubkey` (its identity comes from the session cookie), so it signs nothing.

### Part 1 (C4b): only coordinators forward

`RouteHandler` gets a flag that says whether it forwards. Only
`RouteHandler::new_coordinator` sets it. On a local registry miss:

- A coordinator forwards, as today.
- Any other node writes `not_hosted_here` and closes the stream.

The old host of a service was its trusted host until recently, so trusting it
to refuse adds no new trust. The SDK and `ProxyRouter` treat `not_hosted_here`
like a transport failure: they look the service up again and retry once. The
lookup in `net_iroh::resolve_peer` already asks the registry before it falls
back to the stored peer record (`peer_records.db`).

The only test found on this path,
`crates/substrate/tests/stream_client_e2e.rs` (unregistered protocol), checks
"no hang, no bytes", which a refusal also passes.

### Part 1 (C4b): passing the caller's proof along a call chain

Person P calls service A. A calls B, and B calls C. Example: A's access rules
ask service Y `resolve-relation(principal = P)` (`crates/rpc/src/fdae_fetch.rs`).
Y admits that call only when the caller is the principal: `resolve_relation`
(`crates/control_plane/src/synsvc_native/relation.rs`) compares `principal`
with `anchor_did`, or with `subject_did` when there is no `anchor_did`.

Today A's node presents P's certificate as its own identity (the
`(Some(proof), CallOrigin::Native)` arm of `invoke_remote_at`). After the
signature, it cannot: it does not hold P's key. Presenting A's own key alone
would make Y deny every such check.

**Trust rule (ADR-0016 Amendment 2):** a caller trusts the services its call
passes through.

**Mechanism.**

1. When the router verifies P's preamble at the first node, it keeps P's
   proof in `CallerProof` (`crates/rpc/src/native.rs`): `pubkey_hex`,
   `delegation_json`, and now also `service_id`, `interface`, `ts`, `epk` and
   `sig`.
2. A native call (`CallOrigin::Native`) that carries such a proof presents the
   node's own identity: the service instance key and certificate, or the node
   key, by the same rules as the `(None, CallOrigin::Native)` arms today. It
   puts P's proof, unchanged, in `on_behalf`. A service that received
   `on_behalf` passes the same value on. It does not wrap it.
3. The destination checks the immediate caller as usual. Then it checks P's
   proof: it rebuilds P's signed bytes from the fields, checks P's signature,
   the certificate, revocation, and that `ts` is inside the window. If both
   checks pass, it sets `anchor_did` to P's master DID, or to the DID of P's
   key when P has no certificate. If P's proof fails, it refuses the stream,
   as for a bad certificate.

`on_behalf` JSON:

```json
{ "pubkey": "<hex>", "delegation": "<certificate JSON or null>",
  "service_id": "...", "interface": "...", "ts": 0, "epk": "<hex or null>",
  "sig": "<hex>" }
```

**Limits.**

- A guest never sends `on_behalf`. Only `CallOrigin::Native` does, as today
  for the forwarded proof.
- A preamble may carry `ucan` or `on_behalf`, not both. Both set
  `anchor_did`, and one stream has one anchor.
- `on_behalf` sets only `anchor_did`. Capabilities are still checked. A
  service that reads the immediate caller's own certificate is not affected.
  The mailbox keys a sender on it (`crates/mailbox/src/caller.rs`).
- Not checked: that P's proof was made for the first service of the chain.
  The instance certificate does not name its service.

### Part 1 (C4b): revoke command

A `roymctl identity` command adds a temporary key's DID to `revoked_keys` of
the master anchor and publishes the anchor. It keeps the keys that are already
in the list. The router already refuses a revoked key at the next stream.

### Part 2 (C4c): encryption through a forwarder

**Client side in Rust.** Add the client side of `enc=ecdh-p256` in one module
of `crates/router` that the SDK and `IrohHop` share (the SDK already depends
on `crates/router` for `RoutePreamble`). The client:

1. makes a P-256 key and sends it in `epk`, inside the signed bytes;
2. waits for the destination's key and its Ed25519 signature over
   `server key || client epk`, before it sends any data;
3. checks that signature with the host's node key, as `peer-proxy.js` does
   with `TARGET_PUBKEY_HEX`;
4. derives the AES key, then sends.

**When to use it.** The client compares the Iroh endpoint id it dials with
the node key of the host named in the target's signed record (`substrate_id`).
The key of a substrate's `did:key` is its Iroh endpoint id.

- **Equal:** the connection is direct. QUIC is end to end, so there is no
  handshake.
- **Not equal, or no record** (mechanisms given by config, as in
  `SyneroymClient::new_with_mechanisms`): the client uses `enc`.

A direct call costs nothing extra. A call through a forwarder costs one round
trip.

**Forwarder refusal.** A forwarder refuses a stream that has a `pubkey` and no
`enc`, with `encryption_required`. This protects only against an honest
forwarder that would otherwise see the plaintext, for example in its logs. The
client rule above is the real protection.

**KDF.** Both sides derive the AES-256-GCM key with HKDF-SHA256 over the ECDH
secret, with a fixed `info` tag and both public keys. Today the raw secret is
the key. The router and `peer-proxy.js` change together.

### Accepted gaps

These are recorded in the deferred backlog when C4b lands.

- Someone who reads a preamble in plaintext can use it again for the same
  service and interface inside the window. Who can read one: a forwarder
  (until C4c, for Rust calls), and anyone who reads `debug` logs, because the
  router logs the preamble. QUIC is end to end, and an Iroh relay server sees
  only ciphertext.
- Until C4c, a forwarder can also change a plaintext body.
- Any service that P called in the last 15 minutes can act on behalf of P
  through `on_behalf`.
- A replay can reach another member of a `Redundant` or `Sharded` service,
  inside the window.

### Rejected options

- **A nonce and a replay cache.** These would stop a replay inside the window.
  But they add state: memory bounded by a rate limit, and a rule for a cache
  that is empty after a restart. Only a party that already sees plaintext can
  capture a preamble, which is rare.
- **A hash of the request body in the signature.** This would stop a
  forwarder from changing a one-shot JSON-RPC body. C4c's encryption covers
  every body, streams included.
- **A day stamp instead of seconds.** It is the same code, but a captured
  preamble would stay valid for up to 72 hours. ±15 minutes already allows
  normal skew.
- **A challenge from the server.** It costs an extra round trip on every
  stream.
- **Binding the proof to the QUIC connection.** This breaks forwarders,
  because the connection ends at each hop.
- **Signing the raw preamble line.** Forwarders write the preamble again, so
  they would have to forward raw bytes.
- **The destination node DID in the signed bytes.** The client does not know
  the node behind a forwarder or inside a `Redundant` service. `service_id`
  binds the destination well enough.
- **Reading the identity key from the certificate and reusing `pubkey` for
  the handshake key.** `pubkey` would then mean two things, depending on
  whether `delegation` is present. A caller without a certificate needs
  `pubkey` anyway.
- **An allow list of linked services in the callee's policy.** The call chain
  already says who the caller trusts.
- **Encryption on every call.** It costs one round trip even on a direct
  connection, which QUIC already protects.
- **A forwarder refusal as the only rule.** A malicious forwarder ignores it.

## Open questions

1. **How a refusal reaches the client.** Today the router writes the text line
   `Unauthorized\n` before any framing (`route_handler/io.rs`). Decide how
   `not_hosted_here`, `encryption_required` and the new signature refusals
   reach the SDK and `IrohHop`, and how they tell these apart from a transport
   failure. *Unverified:* how the SDK reads `Unauthorized` today.
2. The name and flags of the revoke command.
3. The HKDF salt and `info` tag (C4c).
4. A forwarder still runs `verify_preamble` before it forwards, which can wait
   up to 5 s for a master anchor. Keep it, or skip identity checks on a
   forwarder.

## Tasks

Part 1 (C4b):

- [ ] Signed bytes, `ts`, `sig`, `epk` in `RoutePreamble` and in
      `verify_preamble`.
- [ ] Every preamble writer signs.
- [ ] `epk` in `encryption.rs` and `peer-proxy.js`.
- [ ] The forward flag on `RouteHandler`, `not_hosted_here`, and the client
      retry.
- [ ] `on_behalf`: `CallerProof` fields, `invoke_remote_at`, the check, and
      `anchor_did`.
- [ ] The revoke command.
- [ ] Tests for SP-1 to SP-8.

Part 2 (C4c):

- [ ] The client handshake module, used by the SDK and `IrohHop`.
- [ ] The host comparison.
- [ ] The forwarder refusal.
- [ ] HKDF in the router and in `peer-proxy.js`.
- [ ] Tests for SP-9 to SP-12.

## Deviations

None yet.

## Close-out

Before merge, update the living docs so they describe the result, then set
`status: done` and fill in `living-docs-touched`. Expected:

- `docs/system-architecture.md`:
  - stream identity, and the "does not check that the caller holds the
    private part" Envisioned blocks in FND-IAM and in the encryption section;
  - "One field, two uses";
  - the client gateway paragraph;
  - the coordinator and multi-hop flow text: who forwards, and when Rust
    calls are encrypted.
- The parameter table in the module doc of `crates/router/src/preamble.rs`.
- `docs/developer-guide.md`, if it lists preamble parameters.

Record anything you postponed in
[deferred-backlog.md](../../deferred-backlog.md), including the accepted gaps
above.

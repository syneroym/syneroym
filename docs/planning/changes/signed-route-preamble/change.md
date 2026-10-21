---
status: done
living-docs-touched:
  - docs/system-architecture.md
  - docs/system-requirements-spec.md
  - docs/TERMINOLOGY.md
  - docs/developer-guide.md
  - docs/planning/traceability-matrix.md
  - docs/planning/deferred-backlog.md
---
# Signed route preamble

*Reader: developers.*

A caller proves that it holds the key it names in the route preamble. On a
connection through a coordinator, the caller sends its identity only inside an
end-to-end encrypted channel. A service that calls another service on behalf
of a person passes that person's proof on.

Terms: the **route preamble** is the first line of every stream
(`<scheme>://<interface>.<service_id>?<params>`, `crates/router/src/preamble.rs`).
A **temporary key** is an Ed25519 key that a master DID delegates with a
**delegation certificate**. An **instance certificate** is the delegation
certificate of one deployed service instance (ADR-0020 §1). A **forwarder** is a
coordinator that accepts a stream for a service it does not host and passes it
to the host (`relay_to_next_hop`). A connection is **direct** when the client
dials the node that hosts the service, and **indirect** when it dials a
forwarder. See [TERMINOLOGY](../../../TERMINOLOGY.md).

Two slices of milestone M06D build this: C4b (Part 1) and C4c (Part 2). Both
come before C5. The order is in the
[M06D plan](../../milestones/M06D-reachability-and-deferred-delivery/task.md).
The rules are in [ADR-0002](../../../decisions/0002-handshake-authorization-point.md)
Amendment 1 (proof of possession, forwarding, encryption) and
[ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
Amendment 2 (`on_behalf`).

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

Related gaps:

- **No command revokes a person's temporary key.** The router refuses a key
  listed in `revoked_keys` of the master anchor, but
  `roymctl identity publish-anchor` always publishes an empty list. A stolen
  session key stays valid until its certificate expires.
- **A forwarder reads every identity.** The preamble is plaintext to a
  forwarder: it must read it to route the stream, and the end-to-end
  handshake (`enc=ecdh-p256`) runs only after it. So the certificate and any
  capability token are visible to the forwarder, even when the caller encrypts.
  The router also logs the whole preamble at `debug` when it forwards
  (`route_handler/io.rs`). Only the browser runs the handshake. The Rust client
  has no client side of it, so a Rust call is plaintext to a forwarder.
- **`pubkey` has two meanings.** It is the Ed25519 identity key in
  `verify_preamble` and the P-256 handshake key in `route_handler/encryption.rs`,
  so one stream cannot carry both.
- **Any node forwards.** On a local registry miss, every node calls
  `relay_to_next_hop`, not only coordinators. A substrate that no longer hosts
  a service forwards its streams and reads them in plaintext.

## Scope and non-goals

- In scope, Part 1 (C4b):
  - The caller signs the destination, the interface and the time with its key.
    The router refuses any preamble with a `pubkey` and no valid signature.
  - The handshake key moves from `pubkey` to a new field, `epk`.
  - Only coordinators forward. Any other node answers `not_hosted_here`.
  - A service in a call chain passes the original caller's proof on
    (`on_behalf`).
  - The router stops logging identity fields.
  - A `roymctl` command that revokes a temporary key, in its own small PR.
- In scope, Part 2 (C4c):
  - The client side of the end-to-end handshake in Rust.
  - On an indirect connection, the client sends its identity only after the
    handshake, and only after it has checked the host's key.
  - A key derivation function (KDF) over the handshake secret.
- Not in scope:
  - Replay protection inside the time window (nonce, replay cache). See
    *Rejected options*.
  - End-to-end encryption on a direct connection. QUIC already protects it.
  - Checks on a forwarder. A forwarder only routes. A sender that sends its
    identity in plaintext through a forwarder exposes only its own identity.
  - A UCAN grant from a person to a service at login. This is the general form
    of `on_behalf` for longer grants.
  - Carrying the person's UCAN chain to the next hop. Today's forwarding drops
    it too.
  - `on_behalf` for a person who reaches a service through the client gateway's
    session cookie (`login` mode). Such a caller has no preamble proof
    (`proof: None` in `route_handler/http/auth.rs`), so the service presents
    only its own identity, and `resolve-relation(principal = P)` is still
    denied for that person. This is the same as today. It is the main way a
    person reaches a service (the Hub in a browser), so it is a backlog row.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| SP-1 | A preamble with a `pubkey` and no `sig`, or a `sig` that does not verify with that key, is refused. This holds with and without a `delegation`. | |
| SP-2 | A caller that presents another party's certificate and public key, without the private key, is refused at each mailbox method (`register`, `unregister`, `deposit`, `claim`, `ack`, `status`). | |
| SP-3 | A `ts` more than 15 minutes before or after the destination's clock is refused. A `ts` inside the window is accepted. | |
| SP-4 | A signature made for one `service_id` or `interface` is refused at another. | |
| SP-5 | The router reads the handshake key only from `epk`. The browser tunnel works with `epk`. | |
| SP-6 | A node that is not a coordinator and does not host the service answers `not_hosted_here` and does not forward. The SDK and `ProxyRouter` then look the service up again and reach the new host, for idempotent and non-idempotent calls. | |
| SP-7 | Person P, with a preamble proof (not a session cookie), calls service A, A calls B, and B calls C, each on a different node. C sets `on_behalf_did` to P and leaves `anchor_did` unset (D-C4c-12), and `resolve-relation(principal = P)` succeeds. C refuses the stream when P's proof has a `ts` outside the window or a forged signature. A guest cannot send `on_behalf`. A preamble with both `on_behalf` and `ucan` is refused. | |
| SP-8 | After `roymctl identity` revokes a temporary key and publishes the anchor, the router refuses that key. | |
| SP-9 | The router's logs do not contain `sig`, a certificate, a `ucan` chain or `on_behalf`. | |
| SP-10 | On an indirect connection (through C and through Cp in `crates/coordinator_iroh/tests/multi_hop_relay.rs`), the Rust client runs the handshake, checks the host's key, and then sends its identity encrypted. The forwarder sees no identity field. On a direct connection the client runs no handshake. | |
| SP-11 | When the client does not know the host's key, it does not send its identity through a forwarder, and the call fails with a clear error. | |
| SP-12 | A forwarder that removes `enc` or swaps `epk` gets no identity: the client stops before it sends one. A stream with `enc` that carries identity fields in its plaintext preamble is refused. | |
| SP-13 | Both sides derive the AES key with HKDF-SHA256. The browser, the SDK, `IrohHop` and the client gateway passthrough all work with the router. | |

## Design

### Part 1 (C4b): the signature

**Signed bytes.** The caller signs these fields with the Ed25519 key in
`pubkey`. Each field is written as a 4-byte big-endian length and then its
bytes, so no value can be mistaken for a separator:

1. the tag `syneroym-preamble-v1`;
2. `service_id`, as the caller sends it;
3. `interface`, as the caller sends it;
4. `ts`, as decimal text.

The router checks the signature before it changes `interface`: it rewrites an
empty HTTP interface and maps the interface to its canonical name later
(`rewrite_empty_http_interface`, `registry.lookup`).

**Why each part is needed.**

- The signature proves that the caller holds the key. Encryption hides the
  identity from forwarders, not from the destination. Every destination
  receives the caller's certificate.
- `ts` limits how long a proof can be used. Without it the signed bytes are the
  same on every call, and a proof would stay valid until the certificate
  expires. `ts` limits to 15 minutes what a service that received the proof
  can do with it: pass it on in `on_behalf`. It also bounds what a forwarder
  can do with a Rust identity between C4b and C4c.
- `service_id` and `interface` stop a proof from being used at another service
  or another interface. Each member of a `Redundant` or `Sharded` service has
  its own `ServiceId` (ADR-0022), so a proof for one member does not verify at
  another.

**New preamble parameters.**

| Parameter | Value |
| --- | --- |
| `ts` | Unix seconds, decimal. |
| `sig` | The Ed25519 signature, 128 lowercase hex characters. |
| `epk` | The P-256 handshake key, lowercase hex of the SEC1 encoding (was `pubkey`). |
| `on_behalf` | The original caller's proof, hex of JSON (below). |

**Window ±15 minutes.** This covers normal clock skew, phones included. The
router keeps no state: no nonce and no replay cache.

**Required whenever the preamble has a `pubkey`,** with or without
`delegation`. There is no compatibility flag, because the product is not
released. Today a failed check is a hard reject only when a `delegation` is
present, and otherwise the caller becomes anonymous (the `Err` branch of
`read_and_verify_preamble`, `route_handler/io.rs`). That branch changes: any
`pubkey` that fails the check is refused. A preamble without `pubkey` stays
anonymous, as today. A `pubkey` without a certificate now means "the caller
holds this key". A `ucan` chain in the same preamble is now presented by a
proven key holder too.

**Who writes a preamble with a `pubkey`.** Each one signs when it writes the
preamble, on every attempt:

- the `SyneroymClient` request path (`crates/sdk/src/client.rs`, the builder
  that sets `pubkey` from `identity`);
- `SyneroymClient::passthrough_with_conn` (`crates/sdk/src/client.rs`). The
  client gateway uses it (`crates/client_gateway/src/gateway.rs`), with the
  gateway's node key and, in `fixed` mode, a person's certificate;
- `IrohHop::call`, for `ProxyRouter::invoke_remote_at`
  (`crates/router/src/proxy/`). It signs with the service instance key or the
  node key, which the node holds.

Two more places handle preamble fields:

- `relay_to_next_hop` parses the preamble and writes it again with
  `to_preamble_line`. The new fields must survive this unchanged.
- `crates/coordinator_webrtc/templates/peer-proxy.js` writes `pubkey=` for its
  handshake key (three places). It changes to `epk=`.

The retries of `invoke_remote_at` stay inside one call timeout, so one
signature would fit the window. Signing again costs little and has no edge
case.

**`epk`.** `pubkey` always means the Ed25519 identity key. The P-256
handshake key moves to `epk`, in `route_handler/encryption.rs` and in
`peer-proxy.js`. The browser sends `epk` and no `pubkey`: its identity comes
from the session cookie, so it signs nothing.

**Logs.** `to_preamble_line` is `format!("{self}\n")`, so `Display` is the
wire format and must keep every field. Logs use a separate redacted view of
`RoutePreamble` that leaves out `sig`, `delegation`, `ucan` and `on_behalf` and
prints the certificate's master DID instead. The line to change is the
"Forwarding original preamble" debug line in `relay_to_next_hop`
(`route_handler/io.rs`); check every other place that logs a whole
preamble.

### Part 1 (C4b): only coordinators forward

`RouteHandler` gets a flag that says whether it forwards. Only
`RouteHandler::new_coordinator` sets it. On a local registry miss:

- A coordinator forwards, as today. It checks nothing about the caller: it
  skips `verify_preamble`, so a skewed clock or a slow master-anchor lookup on
  the forwarder cannot refuse a valid stream. Only the destination uses the
  identity.
- Any other node writes `not_hosted_here` and closes the stream.

The old host of a service was its trusted host until recently, so trusting it
to refuse adds no new trust.

**Retry.** `not_hosted_here` comes before any dispatch, so it is safe to retry
for every call, idempotent or not. Today `invoke_remote_at` retries only
`Transport` and `Timeout` errors, only for idempotent calls, and always on the
same address. So the retry belongs in `invoke_remote`, not in that loop: on
`not_hosted_here`, `invoke_remote` resolves the service again
(`net_iroh::resolve_peer`, which asks the registry before it falls back to the
stored peer record) and calls `invoke_remote_at` once more. In the SDK,
`not_hosted_here` arrives on the request stream, not at `connect`, so the SDK
retries in its request path: it connects again after a fresh lookup and
sends the request once more.

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
   `delegation_json`, and now also `service_id`, `interface`, `ts` and `sig`.
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
  "service_id": "...", "interface": "...", "ts": 0, "sig": "<hex>" }
```

**Limits.**

- A guest never sends `on_behalf`. Only `CallOrigin::Native` does, as today
  for the forwarded proof.
- A preamble may carry `ucan` or `on_behalf`, not both. Both set
  `anchor_did`, and one stream has one anchor.
- `on_behalf` sets only `anchor_did`. Capabilities are still checked. A
  service that reads the immediate caller's own certificate is not affected.
  The mailbox keys a sender on it (`crates/mailbox/src/caller.rs`).
- The destination does not check that P's proof was made for a service in the
  chain. Only a service P called can hold P's proof: a forwarder never sees it
  after Part 2, and the logs no longer print it. So this matches the trust rule.

### Part 1 (C4b): revoke command

A `roymctl identity` command adds a temporary key's DID to `revoked_keys` of
the master anchor and publishes the anchor. It keeps the keys that are already
in the list. The router already refuses a revoked key at the next stream. This
command does not depend on the rest of the slice, so it ships in its own small
PR.

### Part 2 (C4c): identity inside the encrypted channel

**Rule.** A sender is responsible for its own end-to-end encryption. A
forwarder only routes, and it can copy or replay encrypted bytes without harm:
each handshake uses a fresh server key, so a replayed stream does not decrypt.
On an indirect connection, the sender must therefore keep its identity out of
the plaintext preamble.

**Direct or indirect.** The client compares the Iroh endpoint id it dials with
the node key of the host. The key of a substrate's `did:key` is its Iroh
endpoint id.

- **Equal:** the connection is direct. QUIC is end to end. The identity
  travels in the preamble, as in Part 1. There is no handshake.
- **Not equal:** the connection is indirect. The client follows the steps
  below.

**Steps on an indirect connection.**

1. The preamble carries only the route (scheme, `interface`, `service_id`),
   `enc=ecdh-p256` and `epk`. No identity field.
2. The destination sends its P-256 key and an Ed25519 signature over
   `server key || client epk`, made with its node key. This is the existing
   server side of the handshake.
3. The client checks that signature with the **host key it expects** (below).
   If the check fails, the client stops. It has sent no identity.
4. The client sends one encrypted line with the identity parameters: `pubkey`,
   `delegation`, `ucan`, `ts`, `sig`, `on_behalf`. The signed bytes are the
   same as in Part 1.
5. The destination reads that line after its encryption stage, runs the same
   check as in Part 1, and only then dispatches. By then the router has
   already replaced `interface` with its canonical name
   (`preamble.interface = canonical_interface`, `route_handler/io.rs`), so it
   keeps the value as sent and checks the signature against that.

A stream with `enc` takes its identity only from the encrypted line. If its
plaintext preamble carries identity fields, the destination refuses it, so a
sender that leaks its identity fails at once instead of working.

What a forwarder can do: remove `enc` (the destination sees an anonymous
stream with no identity), swap `epk` (the client's check in step 3 fails), or
replay the bytes (they do not decrypt). It cannot learn or use the identity.

**The host key the client expects.** This check carries the whole design. If
the client checks against a wrong key, a forwarder can act as the destination
and receive the identity.

- The client takes the host from the target's signed record (`substrate_id`),
  and uses the node key in that DID. It looks the record up in the registry
  even when it dials a forwarder given by config.
- When the target id is itself a substrate DID (`did:key`), that is the host
  key.
- When the client cannot find a host key, it does not send an identity
  through a forwarder. The call fails with a clear error.
- A `Redundant` or `Sharded` service: each member has its own `ServiceId`
  and its own record that names its host (ADR-0022), and the client always
  dials one member's id. So the client uses the record of the member it
  dials. The app record names the supervising node, and the client dials the
  supervisor by its own `did:key`, so its key is known too.

**Users.** One module in `crates/router` holds the client side. The SDK
already depends on `crates/router` for `RoutePreamble`. Its users:

- the SDK request path;
- `SyneroymClient::passthrough_with_conn`, so the client gateway's forwarded
  HTTP bytes are encrypted too, including the `syneroym_session` cookie,
  which is a bearer token;
- `IrohHop`.

**KDF.** Both sides derive the AES-256-GCM key with HKDF-SHA256 over the ECDH
secret, with a fixed `info` tag and both public keys. Today the raw secret is
the key. The router and `peer-proxy.js` change together.

### Accepted gaps

These are recorded in the deferred backlog when C4b lands.

- Any service that P called in the last 15 minutes can act on behalf of P
  through `on_behalf`. C4c narrows this to `resolve-relation` questions
  about P (D-C4c-12).
- Between C4b and C4c, a Rust call through a forwarder still carries its
  identity in plaintext. Both slices land before C5.
- A sender that does not follow Part 2 on an indirect connection exposes its
  own identity. Our clients follow it. A third-party client is responsible for
  its own.

### Rejected options

- **A nonce and a replay cache.** These would stop a replay inside the window.
  They add state: memory bounded by a rate limit, and a rule for a cache that
  is empty after a restart. After Part 2, only a service the caller called can
  hold its proof.
- **Signing every preamble field** (scheme, `enc`, `epk`, `dir`, hashes of the
  certificate and the chain). This was needed only while the identity was
  visible to a forwarder. With the identity inside the encrypted channel, a
  forwarder that changes those fields gets nothing.
- **Binding P's proof to the first service, with a signed link for each later
  hop.** This protects against a service in the chain misusing the proof,
  which the trust rule accepts. It also needs the instance certificate to name
  its service, which it does not today.
- **A forwarder that refuses a plaintext stream with a `pubkey`.** A malicious
  forwarder ignores it, and the sender is responsible for its own encryption.
- **A hash of the request body in the signature.** Part 2's encryption covers
  every body on an indirect connection.
- **A day stamp instead of seconds.** It is the same code, but a proof would
  stay valid for up to 72 hours.
- **A challenge from the server.** It costs an extra round trip on every
  stream.
- **Binding the proof to the QUIC connection.** This breaks forwarders,
  because the connection ends at each hop.
- **Signing the raw preamble line.** Forwarders write the preamble again, so
  they would have to forward raw bytes.
- **The destination node DID in the signed bytes.** The client does not know
  the node behind a forwarder. `service_id` already names one service member.
- **Reading the identity key from the certificate and reusing `pubkey` for
  the handshake key.** `pubkey` would then mean two things, depending on
  whether `delegation` is present. A caller without a certificate needs
  `pubkey` anyway.
- **Encryption on every call.** It costs one round trip even on a direct
  connection, which QUIC already protects.

## Open questions

1. **How a refusal reaches the client.** Decide in the C4b plan. Fact to start
   from: today the router writes the text line `Unauthorized\n` before any
   framing (`route_handler/io.rs`). The SDK reads it as a frame: `read_frame`
   takes the bytes `Unau` as a big-endian length (about 1.4 GB) and fails with
   "frame length … exceeds MAX_FRAME_SIZE" (`crates/rpc/src/framing.rs`). So
   today no refusal is readable. `not_hosted_here` must be readable, because
   the client acts on it.
2. The name and flags of the revoke command. Decide in its PR.
3. **The HKDF salt and `info` tag, and the format of the encrypted identity line.** Decided in C4c: see [D-C4c-1](#deviations) and [D-C4c-2](#deviations).
4. **Tests that dial with no record.** Decided in C4c: `SyneroymClient::with_host` allows tests without registry records to provide the expected host DID key directly. See [D-C4c-7](#deviations).

## Tasks

Part 1 (C4b):

- [x] The revoke command (its own PR).
- [x] Signed bytes, `ts`, `sig`, `epk` and `on_behalf` in `RoutePreamble`,
      including `to_preamble_line`.
- [x] The check in `verify_preamble`, and the reject branch in
      `read_and_verify_preamble`.
- [x] Every preamble writer signs: the SDK request path,
      `passthrough_with_conn`, `IrohHop`.
- [x] `epk` in `encryption.rs` and `peer-proxy.js`.
- [x] Logs without identity fields.
- [x] The forward flag on `RouteHandler`, no identity check on a forwarder,
      `not_hosted_here`, and the retry in `invoke_remote` and the SDK.
- [x] `on_behalf`: `CallerProof` fields, `invoke_remote_at`, the check, and
      `anchor_did`.
- [x] Tests for SP-1 to SP-9.

Part 2 (C4c):

- [x] The client handshake module, used by the SDK request path,
      `passthrough_with_conn` and `IrohHop`.
- [x] Direct or indirect, and the expected host key.
- [x] The encrypted identity line on both sides.
- [x] HKDF in the router and in `peer-proxy.js`.
- [x] A host key for the tests that dial with no record (open question 4).
- [x] Tests for SP-10 to SP-13.

## Deviations

- **D-C4b-1 (Refusal framing):** The router writes a refusal in the framing the preamble requested:
  `Binary` sends a JSON-RPC error frame with code `-32096` (`not_hosted_here`) or `-32097` (`unauthenticated`);
  `Http` sends a complete HTTP/1.1 response (status 421 or 401, `x-syneroym-refusal` header, JSON body);
  `Raw` sends one text line. The router then flushes, closes its write half, and returns `Ok(())` without logging at error level.
- **D-C4b-2 (Order of checks in `handle_stream`):** Forwarders do not verify caller identity. Signature is checked
  against `interface` as received, before `rewrite_empty_http_interface`. Check sequence: read preamble → check whether
  local node hosts `service_id` via `EndpointRegistry::hosts_service` → if not, forward (coordinator) or refuse `not_hosted_here`
  → verify caller → rewrite empty HTTP interface → registry lookup → plan → encrypt → dispatch.
- **D-C4b-3 (Unknown interface on hosted service):** A hosted service with an unknown interface is refused with
  `SERVICE_NOT_FOUND_RPC_CODE` (`-32601`, `unknown_interface`), not forwarded. This prevents a forwarding loop back to the same node.
- **D-C4b-4 (Identity fields without pubkey refused):** A preamble is anonymous only when it has none of `pubkey`,
  `delegation`, `ucan`, `ts`, `sig`, `on_behalf`. A preamble with identity fields but missing `pubkey` is refused rather than silently
  falling back to anonymous.
- **D-C4b-5 (Strict syntax parse for new fields before forward decision):** `RoutePreamble::parse` strictly validates `ts` (u64),
  `sig` (128 lowercase hex characters), and `on_behalf` (JSON matching `OnBehalfProof`). Forwarders drop syntactically malformed streams
  instead of forwarding unparseable bytes. This is a deviation from the literal text of ADR-0002 A1 rule 3 ("checks nothing about the caller");
  it is justified because syntax validation does not depend on host clocks or anchor lookups, and forwarders re-serialize the preamble anyway.
- **D-C4b-6 (CallerProof carries on_behalf and optional signed part):** Received `on_behalf` is stored in `CallerProof::on_behalf`.
  Native calls pass on `proof.on_behalf` if present, else caller's own proof (`CallerProof::to_pass_on`). Local same-node calls in C5/C6
  leave `signed: None` and do not generate an unsigned `on_behalf` proof that downstream hosts would reject.
- **D-C4b-7 (OnBehalfProof wire type location):** `OnBehalfProof` lives in `syneroym-rpc` so both `CallerProof` and `RoutePreamble`
  share the definition without circular crate dependencies.
- **D-C4b-8 (Preamble signing on each attempt):** Writers sign at the moment of preamble writing via `RoutePreamble::sign(&mut self, key: &Identity, now_secs)`.
  `RemoteHop::call` accepts `identity: Option<&OutboundIdentity>` to sign per attempt.
- **D-C4b-9 (Proxy retry on `not_hosted_here`):** `invoke_remote` resolves the service endpoint again upon receiving `not_hosted_here` (`-32096`).
  If the endpoint changed, it retries once. If the endpoint is identical or the second attempt fails, it returns `ProxyError::ServiceNotFound` (retryable in outbox).
- **D-C4b-10 (SDK interior mutability and compare-before-replace):** `SyneroymClient` uses `Mutex<Option<TransportConnection>>` and serializes
  re-homing under `rehome_lock` with compare-before-replace based on `stable_id()`. This prevents concurrent requests from replacing a newly established connection.
- **D-C4b-11 (Client gateway re-home and replay):** Client gateway detects `421 Misdirected Request` (`NotHostedHere`) on the first bytes received from the host.
  The gateway re-homes the cached `SyneroymClient` via compare-before-replace, reconnects with a fresh lookup, and replays the request if no body bytes beyond the initial buffer were sent.
- **D-C4b-12 (Native call identity and on_behalf threading):** A native call (`CallOrigin::Native`) presents node/instance identity and threads caller proof as `on_behalf`
  when present. Removed the old `(Some(proof), Native)` arm that presented caller proof directly as node identity.
- **D-C4b-13 (Destination check of `on_behalf` and refusal logging):** A stream carrying both `on_behalf` and `ucan` is refused. A valid `on_behalf`
  verifies signature, timestamp, certificate, and revocation, setting `caller.session.anchor_did = Some(verified.master_did)` while preserving
  `caller_did` as the immediate caller. Refusals are logged once at `debug` and counted in `substrate.router.refused{kind}`.
- **D-C4b-14 (Revoke command CLI):** `roymctl identity revoke-key --master <name> --key <did:key> --registry-url <url>`. Requires `did:key`,
  forbids revoking master's own DID, and waits for DHT propagation (`sync_dht: true`).
- **D-C4b-15 (Publish-anchor preserves revocations):** `roymctl identity publish-anchor` preserves existing `revoked_keys` via read-modify-write
  rather than overwriting with an empty list.
- **D-C4b-16 (RegistryClient::revoke_key generalization):** `revoke_instance_key` was renamed `revoke_key` and accepts `sync_dht: bool`,
  supporting both instance keys and person temporary keys.
- **D-C4b-17 (Debug formatting and log redaction):** `RoutePreamble`, `CallerProof`, `PreambleSignature`, `OnBehalfProof`, and `OutboundIdentity`
  implement custom `Debug` implementations omitting raw signatures, certificates, UCAN tokens, and `on_behalf` payloads. Preamble parse errors log only route text before `?`.
- **D-C4b-18 (Client reads refusal after write error):** When router refuses a large request, QUIC stream shuts down the write half. `IrohHop::call` and
  `SyneroymClient` attempt reading a response frame (up to 2 s) after write errors before returning the write failure, allowing clients to receive `-32096`/`-32097`.
- **D-C4b-19 (Queued calls retry on `-32096` and `-32097`; conversation outbox behavior change):** `proxy_outbox` and `conversation/transport` classify
  `-32096` and `-32097` as `Disposition::Retry` rather than terminal errors. In the conversation outbox, refused messages now consume their attempt budget
  and dead-letter upon exhaustion instead of remaining in `Unreachable` state for up to 30 days (`max_pending_age_secs`).
- **D-C4c-1 (HKDF salt, info tags, and directional keys):** ECDH derivation uses salt `"syneroym-e2e-v1"` and info tags `"c2s"` (client-to-server) and `"s2c"` (server-to-client) concatenated with the client's 65-byte uncompressed ephemeral public key and server's 65-byte uncompressed ephemeral public key. Client and server derive two separate 32-byte AES-256-GCM keys (`client_key` and `server_key`) to eliminate reflection attacks. `peer-proxy.js` derives and uses matching directional keys and tags.
- **D-C4c-2 (Encrypted identity line format and strict validation):** On an encrypted stream (`enc=ecdh-p256`), identity parameters (`pubkey`, `delegation`, `ucan`, `ts`, `sig`, `on_behalf`) are stripped from the initial preamble line and transmitted as a single encrypted line (`params.join("&") + "\n"`) immediately after the handshake. Only identity keys are permitted in this line; route keys (such as `enc`, `epk`, `target`, `transport`) trigger an encrypted `-32097` refusal. The line is capped at 4096 bytes and must arrive within 5 seconds. An anonymous stream transmits an empty encrypted line (`"\n"`).
- **D-C4c-3 (Order of checks on encrypted streams):** Forwarders perform no handshake and no decryption. On a destination host, `unknown_interface` (`SERVICE_NOT_FOUND_RPC_CODE`, `-32601`) is checked and answered in plain text before the handshake. Decryption and verification of the encrypted identity line occur after the handshake.
- **D-C4c-4 (First-byte rule for indirect streams):** On indirect connections, the client sends no application data or identity line before reading and verifying the server's handshake response (`epk` and signature). The server's handshake answer begins with `0x04` (65-byte uncompressed P-256 public key). Any refusal frame (such as a 4-byte-framed JSON-RPC error starting with `0x00`, or HTTP 4xx/5xx plain text) is read as a pre-handshake refusal; caller identity is never sent.
- **D-C4c-5 (`write_frame` flushes):** `syneroym_rpc::framing::write_frame` explicitly flushes its writer. Because encrypted writers buffer ciphertexts, flushing ensures frames reach the remote peer before the client waits for replies.
- **D-C4c-6 (Strict `epk` requirement):** When `enc=ecdh-p256` is present in the route preamble, `epk` is mandatory and must be a 65-byte uncompressed P-256 public key (starting with `0x04`), hex-encoded (130 hex characters). Compressed points or invalid keys are rejected immediately.
- **D-C4c-7 (SDK host selection rule order):** `SyneroymClient` resolves the target host key using five ordered rules: (1) `TargetRecord` (looked-up record in registry), (2) `ExplicitHost` (configured via `with_host`), (3) `Direct` (dialed node matches target DID), (4) `LookUp` (registry lookup when dialed node differs from target), (5) `TargetDid` (fallback to target DID if valid `did:key`).
- **D-C4c-8 (Host mismatch error handling and `-32098`):** When handshake verification fails due to a mismatched host signature, the client terminates with code `-32098` (`HOST_MISMATCH_RPC_CODE`). No router sends it across the wire; it is local to the SDK, Universal Proxy, and Client Gateway (`PassthroughOutcome::HostMismatch`), which handle `-32098` by refreshing registry records, re-homing, and retrying. Both proxy outbox and conversation outbox classify `-32098` as `Retry`. `HostKeyUnknown` maps to `ServiceNotFound`.
- **D-C4c-9 (Handshake timeout):** The E2E channel handshake enforces a 10-second timeout on waiting for the peer's ephemeral public key and signature.
- **D-C4c-10 (`RemoteHop::call` and `RemotePeer`):** `RemoteHop::call` accepts `peer: &RemotePeer`, combining the dial address with the expected host DID key (`host_key: Option<VerifyingKey>`).
- **D-C4c-11 (`SyneroymClient::with_identity` builder):** `SyneroymClient::with_identity(identity: Identity) -> Self` builder method allows callers and test harnesses to configure or reconfigure caller identity on an existing client instance.
- **D-C4c-12 (`on_behalf` answers only `resolve-relation`, ADR-0016 Amendment 3):** The design above says the destination sets `anchor_did` to P. That gave every service P called P's full access to rows for 15 minutes, because the FDAE row policy and write attribution read `anchor_did`. Now the destination puts P's verified master DID in `CallerContext::on_behalf_did` and leaves `anchor_did` unset. Only `resolve_relation` (`crates/control_plane/src/synsvc_native/relation.rs`) reads it, in the order `on_behalf_did`, `anchor_did`, `subject_did`. Rows are read and written as the immediate caller. The rule "`on_behalf` or `ucan`, not both" stays, because `resolve-relation` answers for one principal. Federated FDAE keeps working. Removing `on_behalf` entirely was rejected: it would break cross-service relationship checks until UCAN delegation exists.

## Close-out

Before merge, update the living docs so they describe the result, then set
`status: done` and fill in `living-docs-touched`. Expected:

- `docs/system-architecture.md`:
  - stream identity, and the "does not check that the caller holds the
    private part" Envisioned blocks in FND-IAM and in the encryption section;
  - "One field, two uses";
  - the client gateway paragraph;
  - the coordinator and multi-hop flow text: who forwards, what a forwarder
    sees, and when Rust calls are encrypted.
- The parameter table in the module doc of `crates/router/src/preamble.rs`.
- `docs/developer-guide.md`, if it lists preamble parameters.

Record anything you postponed in
[deferred-backlog.md](../../deferred-backlog.md), including the accepted gaps
above.

# M06D slice C4c — Identity inside the encrypted channel: implementation plan

*Reader: the developer (or AI) who implements slice C4c. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish (in the background; no `--skip` on the
  final run), the import clean-up pass, the deferred backlog, no planning
  references in code (no "M06D", "C4c", "SP-10", "C4c-4" in code, comments or
  test names; ADR ids are fine), simple English in comments, the 800-line
  file cap, the 100-line function cap, short match arms, the module layout
  rule (`foo.rs` beside `foo/`, never `mod.rs` under `src/`), no hard-coded
  ports in tests, and the two-sided ratchets.
  **This plan names its decisions `C4c-1` to `C4c-14`. Those ids are for this
  plan only. When a step says "for the reason in C4c-n", write the reason
  itself in the comment, never the id.**
- `task.md` in this folder: §3 slices C4b, C4c, C5 (forward note "from C4b
  and C4c").
- `docs/planning/changes/signed-route-preamble/change.md`: criteria `SP-10`
  to `SP-13`, Design "Part 2", "Accepted gaps", open questions 3 and 4, and
  the C4b `## Deviations` (`D-C4b-1` to `D-C4b-19`).
- `slice-c4b-implementation-plan.md` in this folder: §7 "Forward notes",
  item **C4c** (the first-byte rule and where `verify_caller` must move).
  `task.md` does not repeat this note (see §3 item 1).
- [ADR-0002](../../../decisions/0002-handshake-authorization-point.md)
  Amendment A1, rules 4 and 5 (they land here).
- The code this slice changes (read before you edit). Line numbers are from
  branch `feat/m06d-c4b-implementation` at `bbe0de5d`:
  - `crates/router/src/preamble.rs` (whole file) and `preamble/proof.rs`,
    `preamble/redacted.rs`.
  - `crates/router/src/route_handler/encryption.rs` (whole file).
  - `crates/router/src/route_handler/io.rs:265-436` (`read_preamble`,
    `handle_stream`, `CallerCheck`, `verify_caller`, `check_caller`).
  - `crates/router/src/route_handler/dispatch.rs:315-330` (`plan_pipeline`,
    the `enc` match) and `:379-500` (frame writers).
  - `crates/router/src/route_handler/refusal.rs` (`write_refusal`).
  - `crates/router/src/net_iroh.rs:155-268` (`ResolvedPeer`, `resolve_peer`).
  - `crates/router/src/proxy/hop.rs` (whole file), `proxy/router.rs:298-444`
    (`invoke_remote`, `resolve_addr`, `invoke_rehoming`, `invoke_remote_at`),
    `proxy/tests/helpers.rs:68-71,440-500` (`synthetic_addr`, `MockHop`).
  - `crates/rpc/src/framing.rs:24-32` (`write_frame`).
  - `crates/sdk/src/client.rs` (whole file), `client/rehome.rs`,
    `client/passthrough.rs`.
  - `crates/client_gateway/src/gateway.rs:477-575` (`connected`,
    `forward_to_service`).
  - `crates/coordinator_webrtc/templates/peer-proxy.js:185-260,303-520`.
  - `crates/coordinator_iroh/tests/common/nodes.rs:229-296`,
    `multi_hop_relay.rs`, `signed_preamble_nodes.rs:47-84`.

## 1. What C4c builds, in one paragraph

After C4b, a caller signs its preamble, but the identity fields (`pubkey`,
`delegation`, `ucan`, `ts`, `sig`, `on_behalf`) still travel in the plain-text
preamble line. A coordinator that forwards the stream (a **forwarder**) can
read them. After C4c, a Rust client first decides whether its connection is
**direct** (it dialed the node that hosts the target) or **indirect** (it
dialed another node, a forwarder). On a direct connection nothing changes:
QUIC already protects the bytes end to end. On an indirect connection, the
client sends a preamble with only the route, `enc=ecdh-p256` and `epk`. It
reads the host's reply, checks the host's signature with the host key it
expects (from the target's signed record), and only then sends one encrypted
**identity line** with its identity fields. With no known host key, it sends
nothing and fails with a clear error. The destination reads that line after
the handshake and runs the C4b check on it. Both sides now derive the AES keys
with HKDF-SHA256 (one key for each direction). The browser page
(`peer-proxy.js`) changes in the same PR. Three Rust callers use the new
client side: the SDK request path, `SyneroymClient::passthrough_with_conn`
(the client gateway) and `IrohHop` (the Universal Proxy).

One PR.

## 2. Decisions this plan makes (check them before you start)

The ones marked **(confirmed)** go beyond the change doc's text. The owner
confirmed both (C4c-4 and C4c-5) as written on 2026-10-20. Build them as
written.

- **C4c-1. Where the code lives.**
  - New module `crates/router/src/e2e_channel.rs` (public, `pub mod
    e2e_channel` in `lib.rs`), with children:
    - `e2e_channel/cipher.rs`: `ReaderWriter`, `OwnedStream`,
      `EncryptedReader`, `EncryptedWriter` (moved out of
      `route_handler/encryption.rs`, unchanged except the two keys of C4c-4)
      and a new `fn wrap(...)`.
    - `e2e_channel/kdf.rs`: `ChannelKeys` and `fn derive(...)` (C4c-4).
    - `e2e_channel/client.rs`: `Path`, `fn path(...)`, `Opened`,
      `E2eError`, `async fn open_stream(...)` (C4c-2, C4c-7).
    - `e2e_channel/tests.rs`: unit tests.
  - `IdentityFields` and `IdentityRef` go in a new
    `crates/router/src/preamble/identity.rs` (C4c-5).
  - `route_handler/encryption.rs` keeps only the server side
    (`apply_encryption_stage`), now built on `cipher` and `kdf`.
  - New `crates/router/src/route_handler/caller.rs` takes `CallerCheck`,
    `verify_caller` and `check_caller` out of `io.rs` and adds the
    identity-line read (Step 5). `build_caller` stays in `io.rs`
    (`io/tests.rs` uses it).
  - Why one module for both sides: the change doc asks for "one module in
    `crates/router`" for the client; the cipher and the KDF must be the same
    code on both sides, so they move next to it.

- **C4c-2. Direct or indirect.**
  ```rust
  pub enum Path { Direct, Indirect(VerifyingKey) }

  /// `dialed`: the Iroh endpoint id the connection reached (32 bytes).
  /// `host`: the node key the client expects to host `target`.
  pub fn path(dialed: &[u8; 32], host: Option<&VerifyingKey>, target: &str)
      -> Result<Path, E2eError>
  {
      match host {
          Some(h) if h.as_bytes() == dialed => Ok(Path::Direct),
          Some(h) => Ok(Path::Indirect(*h)),
          None => Err(E2eError::HostKeyUnknown { target: target.to_string() }),
      }
  }
  ```
  Callers compute the path **before** they open a stream, so an unknown host
  key opens nothing and sends nothing (SP-11). There is no "anonymous may go
  without a host key" exception: every client we ship signs, so the
  exception would never run.

- **C4c-3. Where the expected host key comes from.**
  - `IrohHop`: from the record. `net_iroh::resolve_peer` already reads the
    target's signed record; it now also returns
    `host_key = resolve_did_key(&record.info.substrate_id).ok()`. For a
    substrate record, `substrate_id == service_id`.
  - SDK. The SDK picks the host **after** it has dialed, because the
    dialed key decides whether a lookup is needed. In this order:
    1. after a registry lookup that found the mechanisms (`connect` with no
       given mechanisms, and `rehome`): the looked-up record's
       `info.substrate_id`;
    2. else `expected_host`, a new field. `new_with_record` sets it to the
       record's `substrate_id`. A new builder `with_host(host_did)` sets it;
    3. else, when the target `service_id` is a `did:key` whose key **is**
       the dialed endpoint id: the target itself (a direct dial to a node
       DID, no lookup needed);
    4. else, when the client has a registry URL: look the target up
       (`lookup(target, false)`, no mechanism resolution) and take its
       `info.substrate_id`. This is the change doc's "it looks the record up
       in the registry even when it dials a forwarder given by config", and
       the architecture doc's multi-hop example (a client given the address
       of Cp sends a stream for Ax). A failed lookup goes on to rule 5;
    5. else the target `service_id` itself, when it parses as a `did:key`.
    The chosen DID is turned into a key with `resolve_did_key`; a DID that
    does not parse gives `None`, which fails at the first request with
    `HostKeyUnknown`.
    Rule 4 runs only when the dialed key is not the target's own key, so a
    plain direct dial (the common case, and the test harness's
    `connect_with_mechanisms` in `crates/substrate/tests/common/node.rs:533`)
    costs no extra lookup.
    Rule 5 is safe even when the target is a *service* DID and not a node
    DID (see §3 item 2): the client then thinks the connection is indirect,
    runs the handshake, and the host's signature does not verify with the
    service key, so the call fails with `BadHostSignature` (its text names
    `with_host`, C4c-10) and sends no identity.
  - Record shapes (the change doc's Redundant/Sharded/app bullet). A
    `Redundant` or `Sharded` member has its own `Service` record whose
    `substrate_id` names its host (`crates/sdk/src/deploy/certify.rs:180`),
    so rule 1 (or 4) gives that host. An app record is a `Substrate`-type
    record with `service_id` = the app DID, `substrate_id` = the supervising
    node and no mechanisms (`crates/app_supervisor/src/tier1.rs:179-190`).
    Nobody dials it: `RegistryClient::lookup` resolves mechanisms only for
    `Service` records, so a client resolves an app through the supervisor,
    which it dials by the supervisor's own `did:key` (rule 1: a substrate
    record's `substrate_id` is itself). The host choice is a pure function
    (`choose_host`, Step 7.3) with a unit test for all three shapes, and the
    gateway e2e tests prove it end to end (Step 11 item 7).
  - Client gateway: it always connects through the registry, so rule 1. It
    is direct unless a record's mechanism names another node; in that case
    a stale host key gives `BadHostSignature`, and the gateway re-homes and
    replays (Step 8, C4c-10).
  - Browser: unchanged. The page already checks against `TARGET_PUBKEY_HEX`,
    which `coordinator_webrtc/src/bootstrap/peer_proxy.rs:76` takes from
    the target record's `substrate_id`.

- **C4c-4. Key derivation (change doc open question 3) (confirmed: two keys).**
  - Input: the 32-byte ECDH shared secret (`raw_secret_bytes()`, the
    x-coordinate; WebCrypto `deriveBits(..., 256)` gives the same bytes).
  - `HKDF-SHA256`, salt = the ASCII bytes `syneroym-e2e-v1`.
  - Two outputs of 32 bytes each, one for each direction:
    - client to server: `info = b"c2s" || client_epk || server_pk`;
    - server to client: `info = b"s2c" || client_epk || server_pk`.
    `client_epk` and `server_pk` are the 65-byte SEC1 uncompressed points as
    they were sent (C4c-9).
  - Why two keys and not one: with one key, a forwarder can send the
    client's own encrypted chunks back to it, and they decrypt (reflection).
    Two keys stop that for the cost of one more HKDF `expand`, and both sides
    change in this PR anyway. Rejected: one key for both directions.
  - Not done (backlog row, Step 13): chunk order. A forwarder can still drop,
    repeat or reorder whole chunks inside one stream, because nonces are
    random. A counter nonce would stop it, but the browser `send` path is
    async and can reorder chunks itself, so it needs its own change.

- **C4c-5. The identity line (change doc open question 3) (confirmed:
  always one line, empty = anonymous).**
  - On every stream whose preamble has `enc=ecdh-p256`, the client sends
    **exactly one** encrypted line right after it verified the host. The
    destination always reads exactly one line before dispatch.
  - Format: the same `key=value&key=value` syntax as the preamble query,
    then `\n`. Only identity keys are allowed: `pubkey`, `delegation`,
    `ucan`, `ts`, `sig`, `on_behalf`, with the same value encodings and the
    same strict parse as in the preamble (`ts` as `u64`, `sig` as 128
    lowercase hex, `on_behalf` hex JSON). Any other key (`enc`, `epk`,
    `dir`, unknown) is an error, and the destination refuses the stream.
  - An empty line (`\n`) means anonymous (the browser sends this; its
    identity is its session cookie inside the HTTP bytes).
  - Limits: the same `MAX_PREAMBLE_LINE_BYTES` (256 KiB) and
    `PRE_AUTH_READ_TIMEOUT` (5 s) as the preamble line.
  - The signed bytes do not change: the client signs `service_id` and
    `interface` as written in the plain-text preamble, and the destination
    checks against the interface **as sent** (it keeps a copy before the
    rewrite, C4c-6).
  - Why always one line: one code path on the destination; no flag in the
    preamble that a forwarder could remove to make the destination read the
    line as request bytes.

- **C4c-6. Order of checks on the destination for a stream with
  `enc=ecdh-p256`.** Read preamble → `hosts_service` (forward or
  `not_hosted_here`, unchanged) → **if the plain-text preamble has any
  identity field: refuse `unauthenticated`** (plain text, SP-12) → keep
  `sent_interface = preamble.interface.clone()` → rewrite empty HTTP
  interface → registry lookup (`unknown_interface`, plain text) → plan →
  handshake (`apply_encryption_stage`) → read the identity line → merge its
  fields into the preamble → `verify_caller` on a copy whose `interface` is
  `sent_interface` → on refusal write `unauthenticated` **through the
  encrypted writer** → dispatch.
  Streams without `enc` keep the C4b order exactly (`D-C4b-2`).
  Why `unknown_interface` now comes before the caller check on an encrypted
  stream: the handshake needs the pipeline, and the pipeline needs the
  lookup. The interface name is already visible to the forwarder in the
  route, so the early answer tells the caller nothing new.

- **C4c-7. A refusal before the handshake (the first-byte rule).** The
  destination still writes `not_hosted_here`, `unknown_interface` and the
  plain-text `unauthenticated` of C4c-6 in plain text, before any
  handshake. The client tells this apart from the handshake by the first
  byte: the server's P-256 key is SEC1 uncompressed and starts with `0x04`;
  a binary refusal frame starts with `0x00` (a big-endian length below
  16 MiB), an HTTP refusal with `H`, a raw one with a letter. The client
  peeks the first byte without consuming it (`BufReader::fill_buf`). Not
  `0x04` → `Opened::Refused(stream)`: the caller reads the refusal in the
  route's framing from the same reader. The browser does the same (Step 9).

- **C4c-8. Frames are flushed.** `EncryptedWriter` buffers up to 8192
  plain-text bytes and encrypts on `flush`. The binary dispatch path never
  flushes after `framing::write_frame` (`dispatch.rs:391,395,473,568,598`):
  on a plain `SendStream` that does not matter, but on an encrypted stream a
  response would stay in the buffer and be lost when the handler returns.
  Fix it in one place: `framing::write_frame` calls `writer.flush().await?`
  after the frame. This covers every frame writer on both sides (router,
  SDK, `IrohHop`, subscribe pushes). Rejected: an `EncryptedWriter` that
  encrypts on every `poll_write`. It still leaves a chunk unsent when the
  inner writer returns `Pending` on its last write, so callers must flush
  anyway.

- **C4c-9. `epk` is strict.** The destination accepts only a 65-byte SEC1
  uncompressed P-256 point (first byte `0x04`). Both of our clients send
  that form, and the KDF `info` uses these exact bytes. Error text names
  `epk`.

- **C4c-10. Client errors.** In `e2e_channel/client.rs`, with `thiserror`
  (already a router dependency):
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum E2eError {
      #[error("no host key for '{target}': the client cannot tell a direct connection \
               from a forwarder, so it sends no identity; connect through the registry \
               or give the host with `with_host`")]
      HostKeyUnknown { target: String },
      #[error("the node that answered did not prove it is host '{host}' of '{target}' \
               (bad handshake signature); no identity was sent. If '{target}' is a \
               service and not a node, give its host with `with_host` or a registry URL")]
      BadHostSignature { host: String, target: String },   // host: the expected did:key
      #[error("no handshake reply within {0:?}; no identity was sent")]
      Timeout(Duration),
      #[error("the stream closed before the handshake; no identity was sent")]
      Closed,
      #[error("invalid handshake reply: {0}")]
      BadReply(String),
      #[error(transparent)]
      Io(#[from] io::Error),
  }
  ```
  How each error is classified. The aim: retry only what can change, and
  never let a wrong host look like "the peer is unreachable".

  | `E2eError` | `IrohHop` returns | Retried by the `invoke_remote_at` loop | Proxy outbox (`disposition_of`, `proxy_outbox.rs:102`) | Conversation outbox (`classify`, `conversation/src/transport.rs:657`) | `invoke_rehoming` |
  | --- | --- | --- | --- | --- | --- |
  | `HostKeyUnknown` | `ProxyError::ServiceNotFound(target)` | no (not `Transport`) | `Retry`, as for any `ServiceNotFound` | `Unreachable`, as for any `ServiceNotFound` (unchanged) | returns it |
  | `BadHostSignature` | `ProxyError::Callee { code: HOST_MISMATCH_RPC_CODE, .. }` (new code `-32098`, next to `-32096`/`-32097` in `crates/rpc/src/proxy.rs`) | no (a `Callee` error is never retried there) | `Retry` (add the code to the existing `-32096`/`-32097` arm) | `Retry` (add the code to the existing `-32096`/`-32097` arm, `transport.rs:653-654`) | resolves again, like `-32096` |
  | `Timeout`, `Closed`, `BadReply`, `Io` | `ProxyError::Transport(e.to_string())` | yes, as today | `Retry`, as today | `Unreachable`, as today | returns it |

  Why `HostKeyUnknown` keeps `ServiceNotFound`, with the conversation
  outbox's `Unreachable`: it means "no record gives a usable host", which is
  what `ServiceNotFound` already means on both paths, and a later record can
  fix it. It cannot happen with records our code writes (their
  `substrate_id` is always a `did:key`), so it needs no class of its own.

  Why a synthetic `Callee` code and not a new `ProxyError` variant: C4b
  already delivers `-32096` this way, and a new variant would also need a
  new case in the guest WIT `proxy-error` and both conversions
  (`sandbox_wasm/src/host_capabilities/capabilities_proxy.rs:24`,
  `app_host_native/src/convert/proxy.rs:38`).
  Why `invoke_rehoming` resolves again on `-32098`: with a record whose
  mechanism names a forwarder, a service that moved is answered by its new
  host, whose key is not the one in the old record. A fresh lookup gives
  the new host key. When the fresh `RemotePeer` has the same address **and**
  the same host key, `invoke_rehoming` returns the `-32098` error as it is
  (unlike `-32096`, it does not turn it into `ServiceNotFound`: a node that
  fails the host check twice is not "missing", and hiding that would hide a
  forwarder that attacks).
  The SDK returns `E2eError` inside `anyhow::Error`. `request_raw` and
  `subscribe` re-home on `BadHostSignature` as they do on `-32096` (only
  when `can_rehome()`): the registry then gives the host's own address, so
  the next try is direct.
  The client gateway re-homes on it too (Step 8), so the SDK, the
  Universal Proxy and the gateway all treat `-32098` like `-32096`.
  How C5 reacts to these is C5's decision. Facts for it (Step 14 adds them
  to the C5 forward note): `HostKeyUnknown` arrives as `ServiceNotFound`,
  which the proxy outbox retries and the conversation outbox classifies as
  `Unreachable`, exactly like a missing record; it fails again until the
  record changes. `-32098` means some node answered that is not the host;
  both outboxes classify it `Retry`, never `Unreachable`.

- **C4c-11. Client handshake timeout.** `pub const HANDSHAKE_TIMEOUT:
  Duration = Duration::from_secs(10);` around the read of the server's
  reply only. Why: a forwarder that removes `enc` leaves the destination
  waiting for a request and the client waiting for a key. The SDK request
  path has no overall timeout, so without this it would wait forever.
  `IrohHop`'s own call timeout still wraps the whole attempt.

- **C4c-12. Tests that dial with no record (change doc open question 4).**
  Add `SyneroymClient::with_host(host_did)`. A test whose target is a node
  DID needs nothing: C4c-3 rule 3 covers a direct dial to that node, and
  rule 5 (no registry URL) or rule 4 (with one) covers a dial through a
  forwarder. Tests that dial an app service with the
  node's mechanisms add `.with_host(<node DID>)`. To make that easy,
  `SubstrateTestContext` gets `pub node_did: String`. The full list is in
  Step 10.

- **C4c-13. `MessageStream::stop` becomes async.** The send half of a
  subscription can be encrypted, and closing it must flush first. `pub async
  fn stop(&mut self) -> Result<()>` calls `self.send.shutdown().await`. One
  caller: `crates/substrate/tests/messaging_client_e2e.rs:146`.

- **C4c-14. An `enc` value other than `ecdh-p256`.** Unchanged: it is
  ignored, as today (`dispatch.rs:321-324`), and the stream is plain. Add
  `pub const ENC_ECDH_P256: &str = "ecdh-p256";` and
  `RoutePreamble::wants_e2e(&self) -> bool { self.enc.as_deref() ==
  Some(ENC_ECDH_P256) }`, and use this one predicate in `plan_pipeline`, in
  `handle_stream` and in the client.

## 3. Things in task.md and the change doc that are ambiguous or stale

Each item says what this plan does about it. Items 2, 3 and 6 need a word
from the owner only if the plan's choice is wrong.

1. **`task.md` C4c misses the C4b forward note.** The C4b plan §7 says to
   add its notes to `task.md` at close-out. The C5, C6 and C7 notes went
   in; the C4c one did not (the first-byte rule, and that `verify_caller`
   must move after the encryption stage for an `enc` stream). This plan
   includes it (C4c-6, C4c-7). Close-out adds a short line to `task.md`
   C4c.
2. **"When the target id is itself a substrate DID (`did:key`), that is the
   host key"** (change doc, "The host key the client expects"). Every
   service member DID is also a `did:key`, so a client cannot tell a node
   DID from a service DID by its form. C4c-3 rule 5 uses the target key
   anyway (and rule 3 when the client dialed exactly that key), because a
   wrong guess fails closed (no identity sent). The cost: tests that dial an
   app service with node mechanisms and no registry URL need `with_host`
   (C4c-12).
3. **`IrohHop` is always direct in production today.** `resolve_peer`
   takes the address from the host substrate's *own* record, and a
   substrate publishes its own endpoint id
   (`crates/substrate/src/runtime/publish.rs:217-246`). So the indirect
   branch of `IrohHop` runs only when a record's Iroh mechanism names
   another node. The plan builds and tests it anyway (the change doc and
   the C5 forward note require it), with a test that passes a coordinator
   address by hand (Step 11).
4. **The SDK is indirect only with given mechanisms.** A registry
   `connect` dials the address in the host's own record, so it is direct.
   Indirect happens with given mechanisms (`new_with_mechanisms`,
   `connect_with_mechanisms`, or a record whose mechanism names another
   node) that point at a coordinator, as in
   `coordinator_iroh/tests/multi_hop_relay.rs`. SP-10 is tested there. In
   that case the change doc asks the client to look the target's record up
   anyway, to learn the host. C4c-3 rule 4 does this when the client has a
   registry URL; without one, the client needs `with_host`, or a target
   that is a node DID.
5. **Test fixtures use an Iroh key that is not the node key.**
   `coordinator_iroh/tests/common/nodes.rs:256` (`spawn_z_target_substrate`)
   and `multi_hop_relay.rs:157` (`spawn_x_target_substrate`) bind the
   endpoint with `SecretKey::generate`, while the node DID comes from a
   different `Identity`. In production both come from the same key
   (`connection_router.rs:93` and `route_handler.rs:391`). With such a
   fixture, a client that dials the node itself sees "indirect" and runs a
   needless (but working) handshake. Step 10 makes both fixtures use
   `SecretKey::from_bytes(&secret_z_bytes)` (and `_x_`), like
   `NativeCapabilityCalleeNode` already does (`nodes.rs:339`).
6. **The change doc's threat list misses chunk-level changes.** "A
   forwarder can copy or replay encrypted bytes without harm" is true for a
   whole stream. Inside one stream, with one AES key for both directions
   and random nonces, a forwarder can reflect, drop or reorder chunks.
   C4c-4 fixes reflection; chunk order goes to the backlog.
7. **The server never flushes binary frames** (C4c-8). Not mentioned
   anywhere; it only matters once a Rust client encrypts, which is this
   slice.
8. **Raw stream-protocol streams over `enc` are not covered.**
   `crates/sandbox_wasm` never flushes its writer (`grep flush()` finds
   nothing), so a raw stream with `enc` could stall like C4c-8. No client we
   ship opens one: the browser uses `enc` only with `http://`
   (`peer-proxy.js:587,944`), the SDK has no raw API, and `IrohHop` is
   binary. Backlog row (Step 13), no code change.
9. **Stale backlog row.** "A sender that ignores Part 2 on an indirect
   connection exposes its own identity" targets "M06D C4c", but the change
   doc lists it as a permanent accepted gap. Close-out sets the target to
   `TBD (accepted)`.
10. **Stale architecture text from C4b.** `docs/system-architecture.md:1851`
    still says "The caller sends its ephemeral P-256 key in the preamble
    field `pubkey`". Step 12 fixes it with the rest of that section.
11. **The browser refusal case has no automated test.** The Playwright
    suite has no target that answers `not_hosted_here` on the tunnel path.
    The plan tests the first-byte rule in Rust and treats the browser part
    as checked by review plus the existing `force_tunnel=true` runs; Step
    13 records it.

## 4. Steps

Do them in this order. Each step lists every file and call site.

### Step 1 — `crates/rpc/src/framing.rs`: flush every frame (C4c-8)

```rust
pub async fn write_frame<W>(writer: &mut W, frame: &[u8]) -> Result<()>
where W: AsyncWrite + Unpin + Send,
{
    let len = frame.len() as u32;
    writer.write_u32(len).await?;
    writer.write_all(frame).await?;
    // An encrypted writer holds bytes until it is flushed; a frame must
    // reach the peer before the writer waits for an answer.
    writer.flush().await?;
    Ok(())
}
```
No call site changes. Run `cargo nextest run -p syneroym-rpc` and the
router tests after this step to see nothing else depended on the old
behaviour.

### Step 2 — `crates/router/src/preamble.rs` and `preamble/identity.rs` (C4c-5, C4c-14)

2.1 New file `preamble/identity.rs`:

```rust
/// The identity parameters of a preamble. On a direct connection they are
/// part of the preamble line; on a stream with `enc=ecdh-p256` the client
/// sends them as one encrypted line after the handshake.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IdentityFields {
    pub pubkey: Option<String>,
    pub delegation: Option<DelegationCertificate>,
    pub ucan: Option<CapabilityToken>,
    pub ts: Option<u64>,
    pub sig: Option<String>,
    pub on_behalf: Option<OnBehalfProof>,
}

/// A borrowed view, so `Display` of a preamble writes the same parameters
/// without cloning certificates and chains.
pub(crate) struct IdentityRef<'a> {
    pub pubkey: Option<&'a str>,
    pub delegation: Option<&'a DelegationCertificate>,
    pub ucan: Option<&'a CapabilityToken>,
    pub ts: Option<u64>,
    pub sig: Option<&'a str>,
    pub on_behalf: Option<&'a OnBehalfProof>,
}

impl IdentityRef<'_> {
    /// Pushes `key=value` for each present field, in wire order:
    /// pubkey, delegation, ucan, ts, sig, on_behalf.
    pub(crate) fn push_params(&self, params: &mut Vec<String>) { /* body moved
        from RoutePreamble's Display, lines 473-496 */ }
}

impl IdentityFields {
    // Named `view`, not `as_ref`, so clippy does not ask for the `AsRef` trait.
    pub(crate) fn view(&self) -> IdentityRef<'_>;
    pub fn is_empty(&self) -> bool;            // all None
    /// The encrypted identity line: the parameters and `\n`. An empty
    /// identity is the line `\n`.
    pub fn to_line(&self) -> String {
        let mut params = Vec::new();
        self.view().push_params(&mut params);
        format!("{}\n", params.join("&"))
    }
    /// Parses an identity line. Only identity keys are allowed.
    pub fn parse_line(raw: &str) -> Result<Self> {
        let mut out = Self::default();
        let body = raw.trim_end_matches(['\n', '\r']);
        if body.is_empty() { return Ok(out); }
        for part in body.split('&') {
            let (k, v) = part.split_once('=')
                .ok_or_else(|| anyhow!("identity line: parameter without '='"))?;
            if !out.parse_param(k, v)? {
                return Err(anyhow!("identity line: '{k}' is not an identity parameter"));
            }
        }
        Ok(out)
    }
    /// Sets one identity parameter. `Ok(false)` when `k` is not one.
    pub(crate) fn parse_param(&mut self, k: &str, v: &str) -> Result<bool> {
        match k {
            "pubkey" => self.pubkey = Some(v.to_string()),
            "delegation" => self.delegation = decode_delegation(v),
            "ucan" => self.ucan = decode_ucan(v),
            "ts" => /* same as ParsedQuery today */,
            "sig" => /* same strict check as ParsedQuery today */,
            "on_behalf" => self.on_behalf = Some(decode_on_behalf(v)?),
            _ => return Ok(false),
        }
        Ok(true)
    }
}
```
Move `decode_delegation`, `decode_ucan`, `decode_on_behalf` into
`identity.rs` (they are only used by the identity keys). Hand-written
`Debug` for `IdentityFields`, like `RoutePreamble`'s: print only the key DID
of `pubkey`, the master DID of `delegation`, `ts`, and `set`/`-` for `sig`,
`ucan`, `on_behalf` (no raw values, `D-C4b-17`).

2.2 In `preamble.rs`:
- `mod identity; pub use identity::IdentityFields; pub(crate) use
  identity::IdentityRef;`
- `pub const ENC_ECDH_P256: &str = "ecdh-p256";`
- `ParsedQuery` loses its six identity fields and gets `identity:
  IdentityFields`. `ParsedQuery::parse_param`: handle `enc`, `epk`, `dir`;
  else `self.identity.parse_param(k, v)?;` and ignore unknown keys as today
  (the preamble stays lenient about unknown keys; only the identity line is
  strict).
- `RoutePreamble::parse` fills the six fields from `parsed_query.identity`.
- New methods on `RoutePreamble`:
  ```rust
  pub fn wants_e2e(&self) -> bool { self.enc.as_deref() == Some(ENC_ECDH_P256) }
  pub(crate) fn identity_ref(&self) -> IdentityRef<'_> { /* borrow the six fields */ }
  /// Moves the identity fields out, leaving the route.
  pub fn take_identity(&mut self) -> IdentityFields { /* Option::take each */ }
  /// Sets the identity fields (replaces all six).
  pub fn set_identity(&mut self, f: IdentityFields) { /* assign each */ }
  ```
- `has_identity_fields` becomes `!self.identity_ref().is_empty()`-style (or
  keep the current six `is_some()` checks; both are fine).
- `Display`: keep the order `enc`, `epk`, identity block, `dir`. Replace
  lines 473-496 with `self.identity_ref().push_params(&mut params);`. The
  wire output must stay byte-identical; `preamble/tests.rs` round-trip tests
  prove it.
- Module doc (lines 36-71): add one row and one paragraph:
  "On a stream with `enc=ecdh-p256` the identity parameters are not in this
  line. The client sends them as one encrypted line after the handshake
  (ADR-0002 A1 rule 4). The destination refuses an `enc` stream whose
  preamble line has an identity parameter."
- `lib.rs`: export `ENC_ECDH_P256` and `IdentityFields` next to the other
  preamble exports.

2.3 `preamble/tests.rs` (new tests, keep the file under 800 lines; if it
would pass 800, put them in `preamble/tests/identity.rs` with `tests.rs`
declaring `mod identity;`):
- `identity_line_round_trips` (all six fields set; `parse_line(to_line())
  == original`).
- `empty_identity_line_is_anonymous` (`"\n"` → `is_empty()`).
- `identity_line_refuses_route_keys` (`enc=...`, `epk=...`, `dir=...`,
  `foo=1` each → `Err`).
- `identity_line_checks_sig_and_ts` (bad `sig` length, upper-case hex,
  non-number `ts` → `Err`).
- `take_identity_leaves_only_the_route` and `display_is_unchanged` (a
  preamble with every field: its `Display` string equals a pinned literal
  written before the refactor).

### Step 3 — `crates/router/src/e2e_channel.rs` and children (C4c-1, -2, -4, -7, -9, -10, -11)

3.1 `crates/router/Cargo.toml`: add `hkdf.workspace = true` and
`sha2.workspace = true` (both are already workspace dependencies:
`Cargo.toml:159,178`).

3.2 `e2e_channel.rs`:
```rust
//! The end-to-end channel inside a stream (`enc=ecdh-p256`): the client
//! side of the handshake, the key derivation and the chunk cipher that both
//! sides use (ADR-0002 A1 rules 4 and 5).
mod cipher;
mod client;
mod kdf;
#[cfg(test)]
mod tests;

pub use cipher::{OwnedStream, ReaderWriter};
pub use client::{E2eError, HANDSHAKE_TIMEOUT, Opened, Path, open_stream, path};
pub(crate) use cipher::wrap;
pub(crate) use kdf::{ChannelKeys, derive};

/// The first byte of a SEC1 uncompressed point. The server's handshake
/// reply starts with it; a refusal written before the handshake never does.
pub(crate) const SEC1_UNCOMPRESSED: u8 = 0x04;
/// Length of a SEC1 uncompressed P-256 point.
pub(crate) const P256_POINT_LEN: usize = 65;
```

3.3 `cipher.rs`: move lines 22-263 of `route_handler/encryption.rs`
(`ready!`, `ReaderWriter` and its impls, `OwnedStream`, `ReadState`,
`EncryptedReader`, `PendingWrite`, `EncryptedWriter`) here unchanged. Add:
```rust
/// Wraps a stream in the chunk cipher: writes are sealed with `send_key`,
/// reads opened with `recv_key`.
pub(crate) fn wrap<R, W>(reader: R, writer: W, send_key: &[u8; 32], recv_key: &[u8; 32])
    -> OwnedStream
where R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static
{
    let reader = EncryptedReader {
        reader,
        cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(recv_key)),
        state: ReadState::ReadLen { buf: [0; 2], bytes_read: 0 },
    };
    let writer = EncryptedWriter {
        writer,
        cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(send_key)),
        write_buf: Vec::with_capacity(8192),
        pending_write: None,
    };
    ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) }
}
```

3.4 `kdf.rs`:
```rust
const KDF_SALT: &[u8] = b"syneroym-e2e-v1";

pub(crate) struct ChannelKeys {
    pub client_to_server: [u8; 32],
    pub server_to_client: [u8; 32],
}

/// HKDF-SHA256 over the ECDH secret. Each direction gets its own key, so
/// bytes sent one way never decrypt when sent back the other way. Both
/// public keys are in `info`, so the keys belong to this one handshake.
pub(crate) fn derive(shared_x: &[u8], client_epk: &[u8], server_pk: &[u8]) -> ChannelKeys {
    let hk = Hkdf::<Sha256>::new(Some(KDF_SALT), shared_x);
    let expand = |label: &[u8]| {
        let info = [label, client_epk, server_pk].concat();
        let mut key = [0u8; 32];
        // 32 bytes is far below HKDF-SHA256's 8160-byte limit.
        hk.expand(&info, &mut key).expect("32-byte HKDF output");   // see note
        key
    };
    ChannelKeys { client_to_server: expand(b"c2s"), server_to_client: expand(b"s2c") }
}
```
Note: the workspace denies `expect_used` outside tests. Either return
`Result<ChannelKeys>` and map the error (`anyhow!("hkdf: {e}")`), or add
`#[expect(clippy::expect_used, reason = "...")]` on the closure. Prefer the
`Result`; it is two lines more and needs no suppression.

3.5 `client.rs`:
```rust
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Path { Direct, Indirect(VerifyingKey) }
pub fn path(dialed: &[u8; 32], host: Option<&VerifyingKey>, target: &str)
    -> Result<Path, E2eError>;   // plan note, do not copy: body as in decision C4c-2

/// A stream ready for the request, or one the destination refused before
/// any handshake. A refused stream holds the refusal, unread, in the
/// framing the preamble asked for.
pub enum Opened { Ready(OwnedStream), Refused(OwnedStream) }

pub enum E2eError { ... }   // plan note, do not copy: variants as in decision C4c-10

/// Writes `preamble` (with its identity fields set and signed) on a new
/// stream. On a direct path it writes it as it is. On an indirect path it
/// moves the identity out of the preamble, runs the handshake, checks the
/// host's signature, and sends the identity as one encrypted line.
pub async fn open_stream<R, W>(reader: R, mut writer: W, mut preamble: RoutePreamble,
                               path: &Path) -> Result<Opened, E2eError>
where R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static
{
    let host = match path {
        Path::Direct => {
            writer.write_all(preamble.to_preamble_line().as_bytes()).await?;
            return Ok(Opened::Ready(plain(reader, writer)));
        }
        Path::Indirect(host) => host,
    };
    let identity = preamble.take_identity();
    let secret = EphemeralSecret::random(&mut OsRng);
    let client_epk = EncodedPoint::from(secret.public_key());       // uncompressed, 65 bytes
    preamble.enc = Some(ENC_ECDH_P256.to_string());
    preamble.epk = Some(hex::encode(client_epk.as_bytes()));
    writer.write_all(preamble.to_preamble_line().as_bytes()).await?;
    writer.flush().await?;

    let mut reader = BufReader::new(reader);
    let reply = time::timeout(HANDSHAKE_TIMEOUT, read_reply(&mut reader)).await
        .map_err(|_| E2eError::Timeout(HANDSHAKE_TIMEOUT))??;
    let Some((server_pk, sig)) = reply else {
        return Ok(Opened::Refused(plain(reader, writer)));
    };
    let signed = [server_pk.as_slice(), client_epk.as_bytes()].concat();
    host.verify_strict(&signed, &Signature::from_bytes(&sig))
        .map_err(|_| E2eError::BadHostSignature {
            host: substrate::derive_did_key(host),
            target: preamble.service_id.clone(),
        })?;
    let server_point = PublicKey::from_sec1_bytes(&server_pk)
        .map_err(|e| E2eError::BadReply(e.to_string()))?;
    let shared = secret.diffie_hellman(&server_point);
    let keys = kdf::derive(shared.raw_secret_bytes(), client_epk.as_bytes(), &server_pk)
        .map_err(|e| E2eError::BadReply(e.to_string()))?;
    let mut stream = cipher::wrap(reader, writer, &keys.client_to_server, &keys.server_to_client);
    stream.writer.write_all(identity.to_line().as_bytes()).await?;
    stream.writer.flush().await?;
    Ok(Opened::Ready(stream))
}

/// `None` when the first byte says the destination refused in plain text.
/// The refusal stays in `reader`'s buffer.
async fn read_reply<R: AsyncRead + Unpin>(reader: &mut BufReader<R>)
    -> Result<Option<([u8; 65], [u8; 64])>, E2eError>
{
    let first = reader.fill_buf().await?;
    match first.first() {
        None => return Err(E2eError::Closed),
        Some(&b) if b != SEC1_UNCOMPRESSED => return Ok(None),
        Some(_) => {}
    }
    let mut buf = [0u8; P256_POINT_LEN + 64];
    reader.read_exact(&mut buf).await?;           // UnexpectedEof -> Io
    let mut server_pk = [0u8; P256_POINT_LEN];
    let mut sig = [0u8; 64];
    server_pk.copy_from_slice(&buf[..P256_POINT_LEN]);
    sig.copy_from_slice(&buf[P256_POINT_LEN..]);
    Ok(Some((server_pk, sig)))
}

fn plain<R, W>(reader: R, writer: W) -> OwnedStream
where R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static
{
    ReaderWriter { reader: Box::new(reader), writer: Box::new(writer) }
}
```
Keep `open_stream` under 100 lines; if it gets close, move the part after
`read_reply` into `fn finish_handshake`.

Do not log anything from `identity` here. A `debug!` line may name
`preamble.redacted()` and the path kind.

### Step 4 — Server handshake: `route_handler/encryption.rs`, `dispatch.rs` (C4c-4, C4c-9, C4c-14)

4.1 `encryption.rs` keeps only `apply_encryption_stage`, now:
```rust
EncryptionStage::TerminateEcdhP256 => {
    let epk_hex = preamble.epk.as_deref().ok_or_else(|| anyhow!("Missing 'epk' ..."))?;
    let client_epk = hex::decode(epk_hex).map_err(|e| anyhow!("Invalid hex epk in preamble: {e}"))?;
    if client_epk.len() != P256_POINT_LEN || client_epk[0] != SEC1_UNCOMPRESSED {
        return Err(anyhow!("epk must be a 65-byte uncompressed P-256 point"));
    }
    let client_point = PublicKey::from_sec1_bytes(&client_epk).map_err(|e| anyhow!("Invalid epk point: {e}"))?;
    let secret = EphemeralSecret::random(&mut OsRng);
    let server_pk = EncodedPoint::from(secret.public_key());
    let shared = secret.diffie_hellman(&client_point);
    let keys = kdf::derive(shared.raw_secret_bytes(), &client_epk, server_pk.as_bytes())?;
    let signed = [server_pk.as_bytes(), client_epk.as_slice()].concat();   // unchanged
    let signature = identity.sign(&signed);
    writer.write_all(server_pk.as_bytes()).await?;
    writer.write_all(&signature.to_bytes()).await?;
    writer.flush().await?;
    Ok(e2e_channel::wrap(reader, writer, &keys.server_to_client, &keys.client_to_server))
}
```
`EncryptionStage::None` uses `ReaderWriter` from `e2e_channel`.
Update the module doc: the handshake now derives two keys with HKDF.
`encryption/tests.rs`: the existing two tests keep working (the reply format
is unchanged). Add `epk_compressed_point_is_refused` (33-byte point → error
names `epk`).

4.2 `io.rs` imports change from `route_handler::encryption::{OwnedStream,
...}` and `encryption::ReaderWriter` to `crate::e2e_channel::{OwnedStream,
ReaderWriter}`; `apply_encryption_stage` still comes from `encryption`.

4.3 `dispatch.rs:321-324`: `let encryption = if preamble.wants_e2e() {
EncryptionStage::TerminateEcdhP256 } else { EncryptionStage::None };`
(same behaviour, one predicate).

### Step 5 — Server identity: `route_handler/caller.rs` (new) and `handle_stream` (C4c-5, C4c-6)

5.1 Move `CallerCheck`, `verify_caller`, `check_caller` (`io.rs:390-436`)
to `route_handler/caller.rs` unchanged (`route_handler.rs`: `mod caller;`).
Add a variant and two methods:

```rust
pub(super) enum CallerCheck {
    Anonymous,
    Verified(Box<CallerContext>),
    Refused,
    /// The identity comes after the handshake, in the encrypted line.
    InChannel,
}

impl CallerCheck {
    /// The caller of an `Anonymous` or `Verified` check.
    pub(super) fn into_caller(self) -> Option<CallerContext> {
        match self { Self::Verified(c) => Some(*c), _ => None }
    }
}

impl RouteHandler {
    /// The caller check before the encryption stage.
    pub(super) async fn check_plain_caller(&self, preamble: &RoutePreamble) -> CallerCheck {
        if !preamble.wants_e2e() {
            return self.verify_caller(preamble).await;
        }
        if preamble.has_identity_fields() {
            // A sender must keep its identity out of the plain-text line on
            // an encrypted stream; refusing at once shows such a sender its
            // mistake instead of working while it leaks.
            debug!(service_id = %preamble.service_id, "identity in plain text on an encrypted stream; refused");
            return CallerCheck::Refused;
        }
        CallerCheck::InChannel
    }

    /// Reads the encrypted identity line, sets its fields on `preamble`,
    /// and checks them against the interface as the caller sent it.
    pub(super) async fn check_channel_caller(&self, stream: &mut OwnedStream,
        preamble: &mut RoutePreamble, sent_interface: &str) -> CallerCheck
    {
        let mut reader = BufReader::new(mem::replace(&mut stream.reader, Box::new(io::empty())));
        let line = time::timeout(PRE_AUTH_READ_TIMEOUT, read_line_bounded(&mut reader)).await;
        stream.reader = Box::new(reader);
        let fields = match line {
            Ok(Ok(raw)) => IdentityFields::parse_line(&raw),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("timed out reading the identity line")),
        };
        let fields = match fields {
            Ok(f) => f,
            Err(e) => { debug!(service_id = %preamble.service_id, "identity line refused: {e}");
                        return CallerCheck::Refused; }
        };
        preamble.set_identity(fields);
        let mut signed_view = preamble.clone();
        signed_view.interface = sent_interface.to_string();
        self.verify_caller(&signed_view).await
    }
}
```
`verify_caller` keeps the anonymous rule (`D-C4b-4`): an empty line gives
`Anonymous`. `build_caller` sets `proof` from `signed_view`, so the proof
carries the interface as sent, as on a direct stream.

5.2 `io.rs`: split `read_preamble` into
```rust
pub(super) async fn read_line_bounded<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Result<String>
// the body of read_preamble up to the '\n' check
pub async fn read_preamble<R>(reader) -> Result<RoutePreamble> { RoutePreamble::parse(&read_line_bounded(reader).await?) }
```
`PRE_AUTH_READ_TIMEOUT` and `MAX_PREAMBLE_LINE_BYTES` become `pub(super)`.

5.3 `handle_stream`: split in two so neither passes 100 lines.
```rust
pub async fn handle_stream<S>(self, stream: S) -> Result<()> {
    // unchanged: stop_signal, split, read preamble with timeout, debug line,
    // hosts_service -> forward or not_hosted_here.
    self.serve_hosted(preamble, reader, writer, stop_signal).await
}

async fn serve_hosted<R, W>(&self, mut preamble: RoutePreamble, reader: BufReader<R>,
                            mut writer: W, stop_signal: Pin<Box<dyn Future<Output = ()> + Send>>)
    -> Result<()>
where R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static
{
    // The signature covers the interface as sent, before the rewrite below.
    let sent_interface = preamble.interface.clone();
    let early = self.check_plain_caller(&preamble).await;
    if matches!(early, CallerCheck::Refused) {
        write_refusal(&mut writer, preamble.transport, Refusal::Unauthenticated).await;
        return Ok(());
    }
    // 2. rewrite + lookup + UnknownInterface refusal (unchanged lines 341-351)
    // 3. plan + log_pipeline (unchanged)
    // 4. encryption
    let mut stream = apply_encryption_stage(reader, writer, &pipeline.encryption, &preamble,
                                            &self.inner.identity).await?;
    let check = match early {
        CallerCheck::InChannel =>
            self.check_channel_caller(&mut stream, &mut preamble, &sent_interface).await,
        other => other,
    };
    if matches!(check, CallerCheck::Refused | CallerCheck::InChannel) {
        write_refusal(&mut stream.writer, preamble.transport, Refusal::Unauthenticated).await;
        return Ok(());
    }
    let caller = check.into_caller();
    // 5. dispatch (unchanged lines 368-386)
}
```
`check_channel_caller` never returns `InChannel`; the `matches!` includes it
only so the match is total without a panic. If `InChannel` reaches a stream
without `TerminateEcdhP256` (cannot happen: both read `wants_e2e`), it is
refused. Update the doc comment of `handle_stream` (the 5-step list) to name
the identity line.

5.4 `relay_to_next_hop` needs no change: a forwarder forwards the plain
route line and copies bytes, including the encrypted identity line.

### Step 6 — Proxy and `IrohHop` (C4c-3, C4c-10)

6.1 `net_iroh.rs`:
```rust
pub struct ResolvedPeer {
    pub addr: Option<EndpointAddr>,
    pub mailbox: Option<EndpointMailbox>,
    /// The node key of the host the record names (`substrate_id`). A client
    /// that dials another node checks the handshake against it.
    pub host_key: Option<VerifyingKey>,
}
// resolve_peer:
let host_key = substrate::resolve_did_key(&record.info.substrate_id).ok();
```
`resolve_iroh_addr` unchanged. Callers that read only `.addr`/`.mailbox`
(`mailbox_record_e2e.rs`, `net_iroh/tests.rs`) need no change. Add one
assertion in `net_iroh/tests.rs:94` area: `peer.host_key` equals the key of
the record's `substrate_id`.

6.2 `proxy/hop.rs`:
```rust
/// Where a remote call goes: the address to dial and the node key that
/// must answer the handshake when that address is not the host itself.
#[derive(Debug, Clone)]
pub struct RemotePeer {
    pub addr: EndpointAddr,
    pub host_key: Option<VerifyingKey>,
}

pub trait RemoteHop: Send + Sync + Debug {
    async fn call(&self, peer: &RemotePeer, route: &RoutePreamble,
                  identity: Option<&OutboundIdentity>, request: &JsonRpcRequest,
                  timeout: Duration) -> Result<Value, ProxyError>;
}
```
`proxy.rs` already has `pub use hop::*;`; add `RemotePeer` to the
`pub use proxy::{...}` list in `lib.rs`.

`IrohHop::call`, inside the existing `time::timeout(timeout, async { ... })`:
```rust
let path = e2e_channel::path(peer.addr.id.as_bytes(), peer.host_key.as_ref(), &route.service_id)
    .map_err(e2e_err)?;                             // before connect: nothing sent
let conn = connect_with_retry(endpoint, peer.addr.clone(), ...).await.map_err(transport_err)?;
let (send, recv) = conn.open_bi().await.map_err(transport_err)?;
let mut stream = match e2e_channel::open_stream(recv, send, preamble, &path).await.map_err(e2e_err)? {
    Opened::Refused(mut s) => return framing::read_frame(&mut s.reader).await.map_err(transport_err),
    Opened::Ready(s) => s,
};
let wrote: Result<(), anyhow::Error> = async {
    framing::write_frame(&mut stream.writer, &body).await?;
    stream.writer.shutdown().await?;                // replaces send.finish(): flushes, then FIN
    Ok(())
}.await;
match wrote { /* unchanged, reading from stream.reader */ }
```
`noq`'s `SendStream::poll_shutdown` is `finish()` and does not wait for the
peer (checked in `noq-0.17.0/src/send_stream.rs:339-341`), so `shutdown()`
keeps today's behaviour. The refused frame then goes through the existing
"success or error envelope" parse, so `-32096` still becomes
`ProxyError::Callee` and `invoke_rehoming` still sees it.

`e2e_err` (new, in `hop.rs`) applies the table in C4c-10:
```rust
fn e2e_err(e: E2eError) -> ProxyError {
    match e {
        E2eError::HostKeyUnknown { target } => ProxyError::ServiceNotFound(target),
        e @ E2eError::BadHostSignature { .. } => ProxyError::Callee {
            code: HOST_MISMATCH_RPC_CODE, message: e.to_string(), data: None,
        },
        other => transport_err(other),
    }
}
```

6.2a `crates/rpc/src/proxy.rs`: add, next to `STREAM_UNAUTHENTICATED_RPC_CODE`:
```rust
/// JSON-RPC code a client gives itself when the node that answered the
/// end-to-end handshake is not the host its record names (ADR-0002 A1).
/// No node sends it; it never crosses the wire.
pub const HOST_MISMATCH_RPC_CODE: i32 = -32098;
```
Export it from `syneroym_rpc` (`crates/rpc/src/lib.rs:42-45`) like the other
two. Add it to the two places that classify `-32096`/`-32097` (`D-C4b-19`):
`crates/router/src/proxy_outbox.rs:114-121` (`disposition_of`, the `Retry`
arm) and `crates/conversation/src/transport.rs:653-654`. Add one case to the
existing classification tests of each.

6.3 `proxy/router.rs`:
- `resolve_addr` → `resolve_peer_target(&self, target) -> Result<RemotePeer,
  ProxyError>`: build `RemotePeer { addr, host_key: peer.host_key }`.
- `invoke_rehoming<F, Fut>` where `Fut: Future<Output = Result<RemotePeer,
  ProxyError>>`:
  ```rust
  let first = resolve().await?;
  match self.invoke_remote_at(&first, req).await {
      Err(e) if is_not_hosted_here(&e) || is_host_mismatch(&e) => {
          let fresh = resolve().await?;
          let same = fresh.addr.id == first.addr.id && fresh.host_key == first.host_key;
          if same {
              // A node that is not the host is not "missing": keep the error.
              return Err(if is_not_hosted_here(&e) { ServiceNotFound(..) } else { e });
          }
          self.invoke_remote_at(&fresh, req).await.map_err(/* -32096 -> ServiceNotFound, as today */)
      }
      other => other,
  }
  ```
  `is_host_mismatch` sits next to `is_not_hosted_here` (`router.rs:625`).
- `invoke_remote_at(&self, peer: &RemotePeer, req)`; pass `peer` to
  `self.hop.call`.

6.4 Tests in `proxy/tests/`:
- `helpers.rs`: replace `synthetic_addr() -> EndpointAddr` with
  `synthetic_peer() -> RemotePeer` (a random key; `host_key` = the same key
  as a `VerifyingKey`, so the path is direct). `MockHop::call` takes
  `_peer: &RemotePeer`.
- Rename every `synthetic_addr()` call to `synthetic_peer()`
  (`local_dispatch.rs` ×1, `on_behalf.rs` ×6, `remote_dispatch.rs` ×15,
  `rehome.rs` ×9; counted with `grep -c 'synthetic_addr()'`). The
  `rehome.rs` closures return `RemotePeer`. The compiler finds any that
  this list misses.
- `rehome.rs`: add `host_mismatch_resolves_again_and_retries` (first call
  `-32098`, fresh peer with another host key, second call succeeds) and
  `host_mismatch_with_same_peer_keeps_the_error` (fresh peer equal → the
  `-32098` error comes back unchanged, one hop call).
- `helpers.rs` is 670 lines; this step does not grow it.

### Step 7 — SDK (`crates/sdk`) (C4c-3, C4c-12, C4c-13)

`client.rs` is 794 lines. This step moves the stream code out to a new
`client/streams.rs`, so `client.rs` ends near 680.

7.1 `client.rs` types:
```rust
pub enum TransportConnection {
    Iroh {
        endpoint: Endpoint,
        conn: Connection,
        /// The node key expected to host the target. When it is not the
        /// key of the node this connection reached, the connection is
        /// through a forwarder, and the identity goes in the encrypted line.
        host_key: Option<VerifyingKey>,
    },
}
```
`Debug` prints `host_key` as its `did:key` or `-`. `stable_id` unchanged.

`SyneroymClient` gets `expected_host: Option<String>` (doc: "The host DID
given by `with_host` or by the record of `new_with_record`. Used when no
registry lookup named the host."). Set it to `None` in `new`,
`new_with_mechanisms`, `new_with_identity`; to
`Some(record.info.substrate_id.clone())` in `new_with_record` (before the
move into `registry_lookup_override`).

```rust
/// The DID of the node that hosts the target, for a client that dials
/// given mechanisms and has no record: the client checks the handshake of a
/// forwarder against this node's key.
#[must_use]
pub fn with_host(mut self, host_did: impl Into<String>) -> Self {
    self.expected_host = Some(host_did.into());
    self
}
```

7.2 `connect()`:
```rust
let (mechanisms, looked_up_host) = if let Some(m) = &self.provided_mechanisms {
    (m.clone(), None)
} else if !self.registry_url.is_empty() {
    let info = self.registry_client().lookup(lookup_target, true).await?.info;
    if self.registry_lookup_override.is_none() { self.service_id = info.service_id; }
    (info.mechanisms, Some(info.substrate_id))
} else { return Err(...) };
let conn = self.dial(mechanisms, looked_up_host.as_deref()).await?;
*self.connection.get_mut()... = Some(conn);
```
`connect_with_mechanisms(&mut self, mechanisms)` (public):
`self.dial(mechanisms, None).await`.

7.3 `client/rehome.rs` and a new `client/host.rs`:
- `dial(&self, mechanisms, looked_up_host: Option<&str>)`: after the Iroh
  connect succeeds, `let host_key = self.host_key_for(looked_up_host,
  conn.remote_id().as_bytes()).await;` and build
  `TransportConnection::Iroh { endpoint, conn, host_key }`.
- `rehome`: `let info = ...lookup(target, true).await?.info; let conn =
  self.dial(info.mechanisms, Some(&info.substrate_id)).await?;`
- `client/host.rs` (new; `client.rs` declares `mod host;`):
  ```rust
  /// Where the host DID comes from. A pure choice, so it can be tested
  /// without a network; see `host_key_for` for the lookup.
  pub(crate) enum HostChoice<'a> { Known(&'a str), LookUp, Target }

  /// The host of the target, in this order: a host a registry lookup
  /// already named; a host given by `with_host` or by a record; the
  /// target itself when the client dialed exactly the target's key;
  /// a fresh registry lookup when there is a registry; else the target.
  pub(crate) fn choose_host<'a>(looked_up: Option<&'a str>, expected: Option<&'a str>,
      target: &str, dialed: &[u8; 32], has_registry: bool) -> HostChoice<'a>
  {
      if let Some(h) = looked_up.or(expected) { return HostChoice::Known(h); }
      let target_is_dialed = substrate::resolve_did_key(target)
          .is_ok_and(|k| k.as_bytes() == dialed);
      if target_is_dialed || !has_registry { HostChoice::Target } else { HostChoice::LookUp }
  }

  impl SyneroymClient {
      /// The key the client checks a forwarder's handshake against.
      /// `None` when no rule gives a `did:key`.
      pub(crate) async fn host_key_for(&self, looked_up: Option<&str>, dialed: &[u8; 32])
          -> Option<VerifyingKey>
      {
          let has_registry = !self.registry_url.is_empty();
          let did = match choose_host(looked_up, self.expected_host.as_deref(),
                                      &self.service_id, dialed, has_registry) {
              HostChoice::Known(h) => h.to_string(),
              HostChoice::Target => self.service_id.clone(),
              HostChoice::LookUp => match self.registry_client().lookup(&self.service_id, false).await {
                  Ok(record) => record.info.substrate_id,
                  Err(e) => {
                      debug!("no record names the host of {}: {e}", self.service_id);
                      self.service_id.clone()
                  }
              },
          };
          substrate::resolve_did_key(&did).ok()
      }
  }
  ```
- `crates/sdk/src/client/host/tests.rs` (or a `#[cfg(test)] mod tests` in
  `host.rs` if it stays small): `choose_host` for
  - a member `Service` record looked up (→ `Known(record substrate_id)`);
  - a substrate record looked up (`substrate_id == service_id` → that DID);
  - an app record looked up (`substrate_id` = supervisor → supervisor);
  - `with_host` given (→ `Known`);
  - a node DID target dialed directly (→ `Target`, no lookup);
  - a service DID target through a forwarder, with a registry (→ `LookUp`)
    and without one (→ `Target`).
- `request_raw` and `subscribe`: re-home when `is_not_hosted_here(&e) ||
  is_bad_host(&e)` (C4c-10). `is_bad_host` sits next to
  `is_not_hosted_here` in `rehome.rs`:
  `e.downcast_ref::<E2eError>().is_some_and(|x| matches!(x, E2eError::BadHostSignature { .. }))`.
  `send_request` (Step 7.4) must return the `E2eError` unwrapped into
  `anyhow` (`?` does this), so the downcast works.

7.4 New `client/streams.rs` (move `request_once`, `open_request_stream`,
`build_preamble` here from `client.rs:531-656`, then change them):
```rust
/// What happened after the request was written.
pub(crate) enum Sent {
    /// The stream is open; the answer comes on `stream.reader`.
    Open(OwnedStream),
    /// The node answered before the request was fully sent (a refusal
    /// before the handshake, or one read after a failed write).
    Answered(Vec<u8>),
}

impl SyneroymClient {
    pub(crate) fn build_preamble(&self, interface: &str) -> RoutePreamble; // unchanged body

    /// Opens a stream, writes the preamble (and, through a forwarder, runs
    /// the handshake and sends the identity line), then writes one request
    /// frame. `finish` closes the send half after the frame.
    async fn send_request(&self, conn: &TransportConnection, interface: &str,
                          req_bytes: &[u8], finish: bool) -> Result<Sent>
    {
        let TransportConnection::Iroh { conn, host_key, .. } = conn;
        let path = e2e_channel::path(conn.remote_id().as_bytes(), host_key.as_ref(), &self.service_id)?;
        let (send, recv) = conn.open_bi().await?;
        let mut stream = match e2e_channel::open_stream(recv, send, self.build_preamble(interface), &path).await? {
            Opened::Refused(mut s) => return Ok(Sent::Answered(framing::read_frame(&mut s.reader).await?)),
            Opened::Ready(s) => s,
        };
        let wrote: Result<()> = async {
            framing::write_frame(&mut stream.writer, req_bytes).await?;   // flushes (Step 1)
            if finish { stream.writer.shutdown().await?; }
            Ok(())
        }.await;
        match wrote {
            Ok(()) => Ok(Sent::Open(stream)),
            Err(write_err) => match time::timeout(REFUSAL_READ_GRACE, framing::read_frame(&mut stream.reader)).await {
                Ok(Ok(f)) if !f.is_empty() => Ok(Sent::Answered(f)),
                _ => Err(write_err),
            },
        }
    }
}
```
- `request_once`: serialize + `check_frame_size` (unchanged); `let conn =
  self.connection()`; `send_request(&conn, interface, &req_bytes,
  true)`; `Sent::Open(mut s)` → `framing::read_frame(&mut s.reader)`;
  `Sent::Answered(f)` → `f`; then the unchanged empty/parse logic. Return
  `(res, Some(conn))` exactly as today, so `request_raw`'s re-home still
  works.
- `open_request_stream` → returns `Result<OwnedStream>`: `send_request(...,
  false)`; `Sent::Open(s)` → `Ok(s)`; `Sent::Answered(f)` → parse as
  `JsonRpcErrorResponse` and return its `error` (as `anyhow`), else
  `anyhow!("the node answered before the subscribe request was sent")`.
- `subscribe_once` (stays in `client.rs`): `let ReaderWriter { reader: mut
  recv, writer: send } = self.open_request_stream(...)`; the ack and the
  read loop use `recv` unchanged (`framing::read_frame` is generic).
- `MessageStream { receiver, send: Box<dyn AsyncWrite + Unpin + Send> }`;
  `pub async fn stop(&mut self) -> Result<()> { self.send.shutdown().await.map_err(Into::into) }`
  (C4c-13). Update its doc: dropping or stopping closes the send half.
- The `REFUSAL_READ_GRACE` const moves to `streams.rs` (keep one copy; the
  passthrough has its own, leave it).

7.5 `client/passthrough.rs`: same signature for both public functions (the
host key travels in `TransportConnection`). Body:
```rust
let TransportConnection::Iroh { conn, host_key, .. } = conn_wrapper;
let path = e2e_channel::path(conn.remote_id().as_bytes(), host_key.as_ref(), service_id)?;
let (send, recv) = conn.open_bi().await?;
let mut preamble = RoutePreamble::new(Http, JsonRpc, service_id, interface_name);
preamble.delegation = delegation.cloned();
preamble.sign(identity, now_secs());
let (stream, refused) = match e2e_channel::open_stream(recv, send, preamble, &path).await? {
    Opened::Ready(s) => (s, false),
    Opened::Refused(s) => (s, true),
};
let ReaderWriter { reader: mut recv, writer: mut send } = stream;
let mut write_err: Option<io::Error> = None;
if !refused {
    if let Err(e) = async { send.write_all(initial_bytes).await?; send.flush().await }.await {
        write_err = Some(e);
    }
}
// rest unchanged, with these edits:
```
- `read_first_response` becomes generic: `recv: &mut (impl AsyncRead +
  Unpin)`, `send: &mut (impl AsyncWrite + Unpin)`, `write_err: &mut
  Option<io::Error>`. `recv.read(first)` (iroh, `Option<usize>`) becomes
  `AsyncReadExt::read` (`usize`, `0` = end). `send.finish()` becomes
  `send.shutdown().await`. After each `send.write_all(&buf[..n])`, call
  `send.flush()`.
- The tail: `io::copy(&mut recv, &mut tcp_w)`, `io::join(recv, send)` and
  `copy_bidirectional` already work on any `AsyncRead`/`AsyncWrite`
  (`copy_bidirectional` flushes when its reader has nothing ready).
- `starts_with_not_hosted_refusal` is unchanged; on a refused stream it
  reads the plain-text 421.
- `PassthroughOutcome::NotHostedHere { replayable }`: on a refused stream
  nothing was sent, so `client_sent_more` stays `false` and the request is
  replayable, as today.

7.6 `crates/sdk/src/lib.rs`: nothing new to export (`with_host` is a
method). `crates/sdk/src/tests.rs:172-186` (`build_preamble` test):
unchanged.

### Step 8 — Client gateway

The gateway connects through the registry (C4c-3 rule 1) and calls
`passthrough_with_conn` with the connection that carries the host key. One
change: re-home on a host mismatch, as on `not_hosted_here`.

8.1 `crates/sdk/src/client/passthrough.rs`: add a variant
```rust
pub enum PassthroughOutcome {
    Done,
    NotHostedHere { replayable: bool },
    /// The node that answered the handshake is not the host the record
    /// names. Nothing was sent, so the caller may send the same request
    /// after it looks the service up again.
    HostMismatch,
}
```
In `passthrough_with_conn`, map an `open_stream` error
`E2eError::BadHostSignature { .. }` to `Ok(PassthroughOutcome::HostMismatch)`
(other `E2eError`s stay errors). The handshake fails before any of
`initial_bytes` is written, so the request is always replayable.

8.2 `crates/client_gateway/src/gateway.rs:541-575` (`forward_to_service`):
treat `HostMismatch` like `NotHostedHere { replayable: true }`: `rehome`,
`connected` again, replay with `passthrough_with_conn`. The second try's
`NotHostedHere { .. }` **or** `HostMismatch` writes `502 Bad Gateway`, as
today. Keep the match arms short: move the shared re-home-and-replay body
into a helper `async fn rehome_and_replay(...)` if the arm passes ~10 lines.

8.3 Test: `crates/client_gateway/src/gateway/tests.rs`, new
`forward_to_service_rehomes_and_replays_on_host_mismatch`, built like the
existing `forward_to_service_returns_bad_gateway_when_retry_also_refused`.

Why this shape: the existing fake server reads a plain preamble and cannot
answer a handshake, and in a unit test `rehome` cannot succeed (the client
has no registry), so `connected` reuses the same connection, with the same
wrong host key. A replay that succeeds therefore cannot be shown here; what
can be shown is that the gateway treats `HostMismatch` like
`NotHostedHere { replayable: true }`: one re-home attempt, one replay, then
`502`. The re-home-then-success path is the one `NotHostedHere` already
uses, which `crates/substrate/tests/signed_preamble_e2e.rs` covers.

- Client: `SyneroymClient::new_with_mechanisms("target-svc-3", <server
  mechanism>).with_registry_dht(false).with_host(<did:key of a fresh
  `Identity`>)`. The dialed key is the server endpoint's, so the path is
  indirect and the client runs the handshake.
- Fake server, for each accepted bi-stream: count the attempt; read up to
  the first `\n` and `RoutePreamble::parse` it; assert `enc ==
  Some("ecdh-p256")` and no `pubkey`; decode `epk`; make a P-256 key
  (`p256::ecdh::EphemeralSecret::random`); write its 65-byte SEC1 point and
  an Ed25519 signature over `server_point || epk` made with **another**
  random `Identity` (not the one given to `with_host`); then read until the
  client closes and record every byte read after the preamble line.
- Assert: the TCP client side reads `HTTP/1.1 502`; `attempts == 2`; the
  bytes recorded after each preamble line are empty (the client sent no
  identity line and no request bytes).
- `crates/client_gateway/Cargo.toml` `[dev-dependencies]`: add
  `p256.workspace = true` and `hex.workspace = true` if they are not
  already dependencies (`syneroym-identity` and `syneroym-router` already
  are).

`passthrough_with_conn`'s own mapping (`BadHostSignature` →
`HostMismatch`) is covered by the same test, since the gateway calls it
directly.

Its other tests change in Step 10.

### Step 9 — Browser: `crates/coordinator_webrtc/templates/peer-proxy.js` (C4c-4, C4c-5, C4c-7)

9.1 New helper next to `verifyAndDeriveSharedSecret`:
```js
const KDF_SALT = new TextEncoder().encode("syneroym-e2e-v1");

async function deriveChannelKeys(sharedBits, clientPubRaw, serverPubBytes) {
    const base = await window.crypto.subtle.importKey("raw", sharedBits, "HKDF", false, ["deriveKey"]);
    const info = (label) => {
        const l = new TextEncoder().encode(label);
        const out = new Uint8Array(l.length + 65 + 65);
        out.set(l, 0);
        out.set(new Uint8Array(clientPubRaw), l.length);
        out.set(serverPubBytes, l.length + 65);
        return out;
    };
    const derive = (label, usage) => window.crypto.subtle.deriveKey(
        { name: "HKDF", hash: "SHA-256", salt: KDF_SALT, info: info(label) },
        base, { name: "AES-GCM", length: 256 }, false, [usage]);
    return { sendKey: await derive("c2s", "encrypt"), recvKey: await derive("s2c", "decrypt") };
}
```
9.2 In `verifyAndDeriveSharedSecret` (after the signature check), replace
the `deriveKey` call with
`const sharedBits = await window.crypto.subtle.deriveBits({ name: "ECDH",
public: serverPub }, clientKeyPair.privateKey, 256);` and return `{ keys:
await deriveChannelKeys(sharedBits, clientPubRaw, serverPubBytes), leftovers
}`.

9.3 In `connectTunnel`'s fallback branch:
- `sharedSecret` becomes `channelKeys` (`{ sendKey, recvKey }`).
  `new Decryptor(channelKeys.recvKey, ...)`; `encryptChunk(channelKeys.sendKey, data)`.
- Right after the keys exist and **before** `resolve(...)`: send the empty
  identity line, so it is the first encrypted chunk:
  `tunnelWS.send(await encryptChunk(channelKeys.sendKey, new
  TextEncoder().encode("\n")));` with a comment: the destination reads one
  encrypted identity line before the request; the page's identity is its
  session cookie, so the line is empty.
- First-byte rule, at the top of the `if (clientKeyPair && !channelKeys)`
  block, after appending `bytes` to `serverKeyBytes`:
  ```js
  if (serverKeyBytes.length > 0 && serverKeyBytes[0] !== 0x04) {
      // The node refused before the handshake (for example "not hosted
      // here"). The refusal is a plain HTTP response; pass it on as it is.
      plainRefusal = true;
      resolve({ send: async () => {}, close: () => { closed = true; activeWebSockets.delete(tunnelWS); tunnelWS.close(); } });
      onData(serverKeyBytes);
      serverKeyBytes = new Uint8Array(0);
      return;
  }
  ```
  and at the top of `onmessage`: `if (plainRefusal) { onData(bytes); return; }`.
  Declare `let plainRefusal = false;` next to `serverKeyBytes`.
- The `verifyAndDeriveSharedSecret` length check (`< 129`) stays.

9.4 Comments in this file follow the same rules as Rust code (no planning
ids).

### Step 10 — Update existing tests and fixtures (compiler-led, then these)

The compiler finds the type changes (Steps 6-7). These need a behaviour
change because the host key is otherwise unknown or wrong (C4c-12):

| File:line | Change |
| --- | --- |
| `crates/substrate/tests/common/mod.rs:213-345` | Add `pub node_did: String` to `SubstrateTestContext` (`#[allow(dead_code)]` like its neighbours), set from `substrate_service_id`. |
| `crates/substrate/tests/guest_http_e2e.rs:195-198` | `connect_peer(app_service_id, mechanisms, node_did: &str)` adds `.with_host(node_did)`; update its callers to pass `&ctx.node_did`. |
| `crates/substrate/tests/static_assets_e2e.rs:148-151` | Same. |
| `crates/substrate/tests/http_passthrough_e2e.rs:230-233` | Same. |
| `crates/substrate/tests/websocket_e2e.rs:308` | `.with_host(&ctx.node_did)` (target `"test-ws-pubsub"` is not a DID). |
| `crates/substrate/tests/basic_lifecycle.rs:357` | `.with_host(&ctx.node_did)`. |
| `crates/substrate/tests/stream_client_e2e.rs:112,193` | `.with_host(&ctx.node_did)`. These tests then write raw preambles on the direct connection themselves; no other change. |
| `crates/substrate/tests/messaging_client_e2e.rs:57,69` | `.with_host(&ctx.node_did)`; line 146 `stream.stop().await.expect(...)` (C4c-13). |
| `crates/substrate/tests/signed_preamble_e2e.rs:694` | `.with_host(node1.did())`. |
| `crates/client_gateway/src/gateway/tests.rs:163,234` | `.with_host(substrate::derive_did_key(&VerifyingKey::from_bytes(server_ep.id().as_bytes())?))` — the fake server is the node. The fake server reads a plain preamble, as today. |
| `crates/sdk/tests/connect_timeout.rs:42` | None: the connect fails before any stream. |
| `crates/coordinator_iroh/tests/connection_limit.rs:174` | `DialInfo.substrate_id` comes from `/v1/info`, which reports the raw endpoint id (`coordinator.rs:324`), not a `did:key:h…`, so no host rule gives a key and every probe would fail with `HostKeyUnknown` (counted as `Rejected`). Add `.with_host(<did>)` where `<did>` is `derive_did_key` of the key decoded from `endpoint_addr_bytes` (`EndpointAddr.id`). The path is then direct and the probe behaves as today. |
| `crates/coordinator_iroh/tests/multi_hop_relay.rs:275,319` | None in the client: the target is a node DID and the dialed node is a coordinator. `new_with_mechanisms` has no registry URL, so C4c-3 rule 5 gives the target key as the host, and these two tests now run the encrypted path. |
| `crates/coordinator_iroh/tests/common/nodes.rs:256` and `multi_hop_relay.rs:157` | Bind the endpoint with `SecretKey::from_bytes(&secret_z_bytes)` / `&secret_x_bytes` instead of `SecretKey::generate` (§3 item 5). |
| `crates/coordinator_iroh/tests/common/nodes.rs:224-296` (`TargetSubstrate`, `spawn_z_target_substrate`) | Add `pub route_handler: RouteHandler` to `TargetSubstrate` (`RouteHandler` is `Clone`; clone `route_handler_z` before it moves into `Router::builder(..).accept(..)`). The new test in Step 11 item 4 needs it to register a member endpoint (`route_handler.registry().register(..)`) and a native service (`register_native_service`). Existing users read only `did` and `router`, so nothing else changes. |

`crates/substrate/tests/common/streams.rs`, `dual_build_fixture_e2e.rs` and
`guest_http_e2e.rs:132` match `TransportConnection::Iroh { conn, .. }`; the
`..` already covers the new field.

### Step 11 — New tests

1. `crates/router/src/e2e_channel/tests.rs` (unit):
   - `kdf_known_answer`: fixed `shared_x = [1; 32]`, `client_epk = [4, 2..]`
     (65 bytes), `server_pk = [4, 3..]`; pin both output keys as hex
     literals (compute once, paste). Also assert the two keys differ. This
     pins the wire contract with `peer-proxy.js`.
   - `path_rules`: equal key → `Direct`; other key → `Indirect`; `None` →
     `HostKeyUnknown` whose text names the target.
   - `open_stream_direct_writes_identity_in_preamble`: duplex; read the line
     on the far end; it has `pubkey=` and `sig=`, no `enc=`.
   - `open_stream_indirect_round_trip`: duplex to a task that runs
     `apply_encryption_stage` with a known `Identity`, then reads the
     identity line with `read_line_bounded` and parses it; assert the
     fields equal what the client signed, and that one request frame and
     one answer frame cross in both directions.
   - `open_stream_refused_before_handshake`: the far end writes a binary
     refusal frame (`-32096`) and closes; `open_stream` returns
     `Opened::Refused`, and `framing::read_frame` on it yields that frame.
     Same with an HTTP `421` (first byte `H`).
   - `open_stream_bad_host_signature_sends_nothing_more`: the far end signs
     with another key; result `BadHostSignature`; the far end then reads EOF
     with no bytes after the preamble line.
   - `open_stream_times_out` (`#[tokio::test(start_paused = true)]`): the
     far end never answers; result `Timeout`.
2. `crates/router/tests/native_dispatch_identity/encrypted_identity.rs`
   (new module in that test binary; add the `#[path]` `mod` line in
   `native_dispatch_identity.rs`). Uses `test_route_handler()`; its node key
   is `Identity::from_bytes(&[7u8; 32])`. A small **spy forwarder** helper
   in this file: two `duplex` pairs and a task that reads the client's first
   line, applies an optional rewrite (none / strip `enc` and `epk` / replace
   `epk`), writes it to the router side, then copies both ways and records
   every client-to-router byte in an `Arc<Mutex<Vec<u8>>>`.
   - SP-10: client `open_stream(..., Path::Indirect(node_key))` through the
     spy to `handle_stream`; a capturing native service sees the caller's
     `caller_did` and `proof`; the recorded bytes contain no `pubkey=`, no
     hex of the caller key, no hex of the delegation.
   - SP-10 (direct): `Path::Direct` without the spy; works; no handshake
     bytes (the first answer byte is the frame length, not `0x04`).
   - SP-12 (strip `enc`): client gets `Timeout`; recorded bytes have no
     identity.
   - SP-12 (swap `epk`): client gets `BadHostSignature`; recorded bytes end
     after the preamble line.
   - SP-12 (destination): a preamble with `enc=ecdh-p256`, an `epk` and a
     `pubkey` in plain text → binary refusal `-32097` before any handshake
     (first byte `0x00`).
   - Identity line errors: a line with `enc=...` → encrypted `-32097`; no
     line within 5 s (`start_paused`) → encrypted `-32097`; empty line →
     anonymous (a native interface then answers its own "unauthenticated
     caller" error, as on a direct anonymous stream).
   - Interface as sent: an HTTP stream with an empty interface to a service
     with HTTP routes (the rewrite case) and a signed identity line →
     accepted (the signature is checked against the empty interface).
   - Refusal before the handshake through `open_stream`: unknown service →
     `Opened::Refused` with `-32096`.
3. `crates/coordinator_iroh/tests/common/logs.rs` (new): move
   `SharedLogBuffer`, `LogWriter`, `SHARED_LOGS`, `init_trace_collector`
   from `signed_preamble_nodes.rs:47-84`; `common/mod.rs` declares `pub mod
   logs;`; `signed_preamble_nodes.rs` uses it.
4. `crates/coordinator_iroh/tests/multi_hop_relay.rs`:
   - In both existing tests, call `init_trace_collector()` first, and after
     the request assert that a captured line contains `Forwarding preamble`,
     the target DID, `enc=ecdh-p256`, `master=-`, `sig=-`, and no
     `pubkey=` (SP-10 on a real forwarder, two topologies).
   - New `iroh_hop_through_forwarder`: `IrohHop::new(Some(client_ep),
     policy).call(&RemotePeer { addr: <C's addr>, host_key:
     Some(<target_z key>) }, &RoutePreamble::binary_json_rpc(&target_z.did,
     "orchestrator"), Some(&OutboundIdentity::key_only(...)),
     &readyz_request, 10 s)` → `{"status":"ok"}`; same log assertion.
   - New `passthrough_through_forwarder`: an SDK client with C's mechanisms
     for `target_z.did`; a local `TcpListener` on `127.0.0.1:0` gives a
     `TcpStream` pair; `client.passthrough("orchestrator", <HTTP POST with `Content-Type: application/json` and a
     readyz JSON-RPC body>, &mut server_side, None)` → `Done`, and the
     other end of the TCP pair reads an `HTTP/1.1 200` with
     `{"status":"ok"}` (SP-13 for the gateway's code path).
   - New `unknown_host_key_sends_nothing` (SP-11): a client with C's
     mechanisms and target `"not-a-did"` → error text contains `no host key
     for 'not-a-did'`, and no `Forwarding preamble` line names it.
   - New `service_target_through_forwarder_looks_up_the_host` (C4c-3 rule
     4): make a member identity and DID; build its record with
     `syneroym_sdk::deploy::member_registry_record(Visibility::Public, &member_did,
     &target_z.did, &member_identity, not_after, None)`
     (`crates/sdk/src/deploy/certify.rs:156`; its doc asks test harnesses to
     share it, and it gives a `Service` record with no mechanisms) and post
     the JSON to R's `/register`. On target_z, through the new
     `TargetSubstrate::route_handler`, register endpoint `(member_did,
     "default")` as `SubstrateEndpoint::NativeHostChannel { service_id:
     member_did }` and a native service for `member_did` that answers
     `{"status":"ok"}` (`deploy` is a public module and re-exports
     `certify::*`, `crates/sdk/src/deploy.rs:25,32`; `coordinator_iroh`'s
     dev-dependencies need `syneroym-sdk`, which `multi_hop_relay.rs`
     already uses). A client from `SyneroymClient::new_with_identity(member
     DID, r_url, id)` that calls `connect_with_mechanisms(<C's mechanism>)`
     reaches it through C (encrypted). The same client without a registry
     URL (`new_with_mechanisms`) fails with `BadHostSignature`, whose text
     names `with_host`, and no `Forwarding preamble` line has `pubkey=`.
   - Keep the file under 800 lines (it is 569). If it would pass 800, put
     the new tests in a new `crates/coordinator_iroh/tests/
     e2e_channel_nodes.rs`.
5. Playwright: no new spec. `webrtc.spec.ts`, `multi-hop.spec.ts` and
   `wasm-app.spec.ts` with `force_tunnel=true` exercise the browser HKDF,
   the two keys and the
   identity line against the router (SP-13). They must pass in `mise run
   verify`.
6. Existing tests that now cover more and must stay green: the C4b set
   (`signed_preamble.rs`, `signed_preamble_nodes.rs`,
   `signed_preamble_e2e.rs`, `revoke_key_e2e.rs`), `encryption/tests.rs`,
   `client_gateway/src/gateway/tests.rs`, `messaging_client_e2e.rs`
   (subscription push through the flush of Step 1),
   `coordinator_iroh/tests/connection_limit.rs` (after its Step 10 fix).
7. Proof that the host key comes from the right record (the change doc's
   Redundant/Sharded/app bullet), end to end. On a direct connection, a
   wrong host key makes the client see "indirect", run the handshake, and
   fail with `BadHostSignature`. So these existing tests pass only when the
   right key is chosen; name them as evidence in `status.md`:
   - `crates/substrate/tests/gateway_hostname_e2e.rs`
     `a_routing_key_header_crosses_the_gateway_to_the_backend_per_request`
     (a `Redundant` member, through the gateway, by the member's record);
   - same file, `an_http_client_reaches_an_apps_logical_service_by_hostname_alone`
     (an app: the gateway resolves through the supervisor, then dials the
     member);
   - `crates/substrate/tests/federated_fdae_e2e.rs` (a service on another
     node, called through `IrohHop` by its `Service` record).
   The pure choice is unit-tested in Step 7.3 (`choose_host`, all three
   record shapes).

### Step 12 — Living docs (this PR)

- `docs/system-architecture.md`:
  - Line ~328 ("A stream is end-to-end encrypted ... only when the preamble
    asks"): the Rust client now asks for it whenever it dials a node that is
    not the host; on such a stream the preamble has no identity field, and
    the identity travels in one encrypted line after the client has checked
    the host's signature. Remove "The Rust client (`SyneroymClient`) does
    not yet ask for it" and the list of identity fields a coordinator sees
    (it now sees them only from a third-party client that ignores the rule).
  - The client gateway paragraph (~689): its forwarded HTTP bytes,
    including the session cookie, are encrypted when the connection is
    indirect.
  - Line ~549 ("Routing": "The router reads the preamble and checks who the
    caller is. Then it plans a pipeline"): add that on a stream with
    `enc=ecdh-p256` the router plans the pipeline and runs the handshake
    first, and checks the caller on the encrypted identity line after it.
  - Lines ~1620-1625, the whole "Optional end-to-end stream layer" list:
    - "Who uses it" (~1622): replace "The Rust client (`SyneroymClient`)
      does not. Nothing else in the SDK or the gateway sets it." with: the
      SDK, the client gateway passthrough and the Universal Proxy set it
      whenever the connection is indirect, and never on a direct one.
    - "Only the node is authenticated": keep; add that the caller proves
      itself in the identity line.
    - "No key derivation step" → HKDF-SHA256, salt `syneroym-e2e-v1`, one
      key for each direction, `info` = direction label and both public
      keys.
  - Lines ~1818, ~1849-1855: who runs the handshake (browser on the tunnel
    path; Rust client on an indirect connection), `epk` (not `pubkey`, fixes
    §3 item 10), the identity line, the first-byte rule for a refusal.
  - Lines ~1838 and ~1844 (multi-hop flow 4, "sends the preamble for Ax
    (including its public key) to Cp" and "including Sz's public key or an
    ephemeral public key"): the preamble sent to a forwarder carries only
    the route, `enc` and `epk`; the caller's key and certificate follow in
    the encrypted identity line. Add that the client looks up Ax's record to
    learn the host key it checks.
  - The FDAE/IAM "What the router checks" list (~796-802): on an encrypted
    stream the check runs on the identity line, after the handshake.
  - Line ~1913 (client-side crypto): add the Rust client.
- `docs/system-requirements-spec.md` [NET-PRM] ~808 ("Reachability and
  Transparent Re-homing"): add that the SDK, `ProxyRouter` and the client
  gateway also look the service up again and retry once when the node that
  answered the end-to-end handshake is not the host its record names
  (client-side code `-32098`, `HostMismatch`; it never crosses the wire).
  The gateway replays such a request always, because nothing was sent
  before the handshake failed.
- `docs/system-requirements-spec.md` [NET-PRM] (~803): `enc`/`epk` now
  says HKDF and the identity line; add a bullet "A client that does not dial
  the host directly runs the handshake, checks the host's key from the
  target's signed record, and sends its identity only inside the encrypted
  channel; with no known host key it sends nothing".
- `docs/planning/traceability-matrix.md:85` [NET-PRM]: add the encrypted
  identity line and HKDF to the description; add `e2e_channel.rs` and the
  new test files as evidence.
- `docs/TERMINOLOGY.md`: add **Direct connection / indirect connection**
  (the change doc's definition) and **Identity line**.
- Module docs: `crates/router/src/preamble.rs` (Step 2.2),
  `route_handler/encryption.rs` (Step 4.1), `e2e_channel.rs` (Step 3.2).
- `docs/developer-guide.md`: no preamble parameter list there today
  (checked: no `pubkey=`); nothing unless you add one.

### Step 13 — Deferred backlog (this PR)

In `docs/planning/deferred-backlog.md`:

- Move to "Recently resolved":
  - "A Rust call through a forwarder still carries its identity in plain
    text" (row ~175).
  - "On a stream with `enc`, a refusal arrives in plain text before the
    handshake" (row ~179): resolved by the first-byte rule; the refusal
    stays plain text but every client reads it.
- Edit "End-to-end handshake hardening" (row ~172): the KDF and the Rust
  client side are done. What stays: the client's ephemeral key is not
  signed (not needed: the client proves itself in the identity line), and
  the security review. Keep the row for the review. Change its link from
  `crates/router/src/route_handler/encryption.rs:312-328` to
  `crates/router/src/e2e_channel.rs` and `route_handler/encryption.rs`
  (the code moved; give the new line range of `apply_encryption_stage`).
- Edit "A sender that ignores Part 2 ..." (row ~176): target `TBD`, text
  "Accepted gap" (§3 item 9).
- Edit "No replay protection inside the 15-minute window" (row ~177):
  remove "by intermediate forwarders (until C4c)". After C4c a forwarder
  sees no proof from our clients; only a service the caller called can
  replay it.
- Add rows (§3 Access control unless noted):
  - *A forwarder can drop, repeat or reorder whole chunks inside one
    encrypted stream* (nonces are random; a counter nonce needs the browser
    `send` path serialized first). TBD. Link `e2e_channel/cipher.rs`.
  - *A raw stream-protocol stream with `enc` may stall: the sandbox stream
    writer never flushes* (§3 item 8). TBD. Link
    `crates/sandbox_wasm/src/engine/guest_stream.rs`.
  - *The browser's refusal-before-handshake path has no automated test*
    (§3 item 11). TBD.
- No new in-code `TODO` markers are planned. If you add one, add its row to
  "Open in-code markers".

### Step 14 — Change doc, task.md, status.md (close-out)

- `change.md`: tick the six Part 2 task boxes; set `status: done`; add to
  `living-docs-touched` any file from Step 12 not already listed. Add
  `## Deviations` entries `D-C4c-1` onward for: C4c-4 (two keys, salt and
  `info`), C4c-5 (identity line format, always one line, empty =
  anonymous), C4c-6 (order on an encrypted stream; `unknown_interface`
  before the caller check), C4c-7 (first-byte rule), C4c-8 (`write_frame`
  flushes), C4c-9 (strict `epk`), C4c-3 (the five SDK rules: the host is
  chosen after the dial; a registry lookup only when the dialed key is not
  the target's; the target key as the last default) and C4c-12
  (`with_host`), C4c-10 (the error table: `HostKeyUnknown` →
  `ServiceNotFound`, `BadHostSignature` → `-32098`
  `HOST_MISMATCH_RPC_CODE`, re-resolve on `-32098` in the SDK, the Universal
  Proxy and the gateway, `PassthroughOutcome::HostMismatch`), C4c-11 (handshake
  timeout), and `RemotePeer` (the `RemoteHop::call` signature). Answer open
  questions 3 and 4 in place with a link to the deviation.
- `task.md`: status line (C4c done); C4c entry "Done" with a pointer to the
  deviations; one line under C4c recording the C4b forward note it carried
  (§3 item 1). Update the C5 forward note:
  - `RemoteHop::call` now takes a `RemotePeer` (address and host key);
    `outbound_preamble` keeps it.
  - The new error facts: an unknown host key arrives as `ServiceNotFound`,
    which the proxy outbox retries and the conversation outbox classifies
    as `Unreachable`, exactly like a missing record; it fails again until
    the record changes. `-32098` (`HOST_MISMATCH_RPC_CODE`) means a node
    that is not the host answered the handshake, after one fresh lookup in
    `invoke_rehoming`; both outboxes classify it `Retry`, never
    `Unreachable`. How C5's mailbox decision treats them is C5's
    decision.
- `status.md`: the C4c row with evidence (test files, counts, verify
  result), implemented by / reviewed by.

### Step 15 — Completion

1. Import clean-up pass over every edited file (types by `use`, functions
   through their parent module, no inline multi-`::` paths).
2. Comment pass: no planning ids, no line counts or size notes, simple
   English.
3. `cargo +nightly-2026-04-06 fmt --all`, then `mise run verify` in the
   background; read the summary; all gates pass, no `--skip`. In a git
   worktree use `cargo xtask verify` with `CARGO_TARGET_DIR`.
4. Ratchets: `client.rs` and `encryption.rs` shrink (no entries for them);
   if the duplication gate reports a lower value (the SDK's two stream
   writers become one, the log collector is shared), lower
   `MAX_EXACT_DUPLICATE_PERCENT` as its message says.
5. `cargo dupes report` over the changed files (recipe in `AGENTS.md`).
6. PR body includes the CLA line from `AGENTS.md` verbatim.

## 5. Test list (summary)

| Criterion | Where |
| --- | --- |
| SP-10 | `router/tests/native_dispatch_identity/encrypted_identity.rs` (spy forwarder, direct case); `coordinator_iroh/tests/multi_hop_relay.rs` (two topologies, forwarder log has no identity; `IrohHop` through C) |
| SP-11 | `e2e_channel/tests.rs` (`path_rules`); `multi_hop_relay.rs` (`unknown_host_key_sends_nothing`, `service_target_through_forwarder_looks_up_the_host`); `sdk/src/client/host` tests (`choose_host`) |
| SP-12 | `encrypted_identity.rs` (strip `enc`, swap `epk`, plain-text identity on `enc` refused); `e2e_channel/tests.rs` (bad signature sends nothing more) |
| SP-13 | `e2e_channel/tests.rs` (KDF known answer, indirect round trip); `multi_hop_relay.rs` (SDK request, `IrohHop`, passthrough through C); Playwright `webrtc.spec.ts`, `multi-hop.spec.ts`, `wasm-app.spec.ts` with `force_tunnel=true` (browser) |
| Host key from the right record | `choose_host` unit tests; `gateway_hostname_e2e.rs` (Redundant member, app); `federated_fdae_e2e.rs` (`IrohHop`) |
| Error classes (C4c-10) | `proxy/tests/rehome.rs` (`-32098` re-resolve, same peer); `proxy_outbox` and `conversation/transport.rs` classification tests; `client_gateway/src/gateway/tests.rs` (`forward_to_service_rehomes_and_replays_on_host_mismatch`: `HostMismatch` → one re-home, one replay, `502`, no identity bytes sent) |

## 6. Review changes (2026-10-20)

A review of the first version raised 9 points. All were checked against the
code and all are incorporated.

| # | Point | Answer |
| --- | --- | --- |
| 1 | `connection_limit.rs` fails: its target is a raw endpoint id from `/v1/info`, not a `did:key`, so no host key is found. | Incorporated: confirmed at `coordinator.rs:324` and `substrate.rs:151`. Step 10 adds `.with_host(<did:key of the endpoint id>)`; Step 11 item 6 lists the test as one that must stay green. |
| 2 | The SDK does not look the target's record up when mechanisms are given, which the change doc asks for; the narrowing is not recorded, and the `BadHostSignature` text misleads. | Incorporated: C4c-3 now chooses the host after the dial, and looks the target up (rule 4) when the dialed key is not the target's and a registry URL exists. `choose_host` and `host_key_for` in Step 7.3. A new multi-hop test (Step 11 item 4) covers the architecture doc's "client given Cp's address" case. `BadHostSignature` now names the expected host, the target and `with_host`. Step 14's deviation lists all five rules. Without a registry URL the client still needs `with_host` (§3 item 4). |
| 3 | The Redundant / Sharded / app-record bullet has no step and no test. | Incorporated: C4c-3 states what each record shape gives (an app record is a `Substrate`-type record with no mechanisms, so it is never dialed). `choose_host` unit tests cover the three shapes; Step 11 item 7 names the e2e tests that fail closed if the wrong key is chosen. |
| 4 | Every `E2eError` becomes `Transport`, which is retried and which C5 would read as "unreachable"; a stale host key is never resolved again. | Incorporated: the C4c-10 table. `HostKeyUnknown` → `ServiceNotFound`; `BadHostSignature` → a new local code `-32098` (`HOST_MISMATCH_RPC_CODE`, delivered as `Callee` like `-32096`, so no WIT change); `invoke_rehoming` resolves again on it; both outboxes classify it `Retry`; the SDK re-homes on it. New tests in `rehome.rs`. The facts go into the C5 forward note (Step 14); C5 still decides its own mailbox reaction. |
| 5 | Step 12 misses `system-architecture.md:1622`, `:1838`/`:1844` and `:549`. | Incorporated: all three are in Step 12. |
| 6 | Step 13 misses backlog row 177 ("until C4c") and the stale `encryption.rs:312-328` link in row 172. | Incorporated: both are in Step 13. |
| 7 | The Step 7.3 snippet's doc comment names `C4c-3`, which would fail the planning-refs gate if copied. | Incorporated: the snippet's comments now state the rule itself. |
| 8 | Call-site counts in Step 6.4 are wrong. | Incorporated: recounted with `grep -c` (`remote_dispatch.rs` 15, `rehome.rs` 9, `on_behalf.rs` 6, `local_dispatch.rs` 1). |
| 9 | Step 11.5 leaves out `wasm-app.spec.ts`, which also runs with `force_tunnel`. | Incorporated: added to Step 11 item 5 and §5. |

Second review (2026-10-20), four points, all checked against the code and
incorporated:

| # | Point | Answer |
| --- | --- | --- |
| 1 | The conversation outbox maps `ServiceNotFound` to `Unreachable` (`transport.rs:657`), so the C5 note "neither means unreachable" is false for `HostKeyUnknown`. | Incorporated: the C4c-10 table has a conversation-outbox column, and the C5 facts (C4c-10 and Step 14) say that `HostKeyUnknown` is `Unreachable` there, like a missing record. The mapping stays `ServiceNotFound`: "no record gives a usable host" is what that error means, and our records always name a `did:key` host. |
| 2 | Re-homing on `-32098` is not in the requirements, and the gateway does not do it. | Incorporated: Step 8 adds `PassthroughOutcome::HostMismatch` and the gateway re-homes and replays on it (always replayable: nothing is sent before the handshake). Step 12 updates the [NET-PRM] re-homing bullet. C4c-3 explains when the gateway can be indirect at all. |
| 3 | Rule numbers are stale after the renumbering to five rules. | Incorporated: §3 item 2, C4c-12 and the Step 10 `multi_hop_relay.rs` row now name the right rules (the multi-hop clients use rule 5). |
| 4 | The new forwarder test cannot reach target_z's registry, and should use `member_registry_record`. | Incorporated: Step 10 adds `route_handler: RouteHandler` to `TargetSubstrate`; Step 11 item 4 builds the record with `syneroym_sdk::deploy::member_registry_record`. |

Third review (2026-10-20): the §5 "Error classes" row did not list the
gateway test of Step 8.3, and that step's first option could not work (the
fake server read a plain preamble, so the client would see a plain refusal,
not `HostMismatch`). Incorporated: Step 8.3 now specifies one test with a
fake server that answers the handshake with a wrong-key signature, and says
why a successful replay cannot be shown in a unit test; the fallback is
removed; §5 lists the test.

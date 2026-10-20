# M06D slice C4b — Signed route preamble: implementation plan

*Reader: the developer (or AI) who implements slice C4b. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  deferred backlog, no planning references in code (no "M06D", "C4b",
  "SP-3", "C4b-7" in code, comments or test names; ADR ids are fine), simple
  English in comments, the 800-line file cap, the 100-line function cap,
  short match arms, the module layout rule (`foo.rs` beside `foo/`, never
  `mod.rs` under `src/`), no hard-coded ports in substrate tests, and the
  two-sided ratchets (oversized test files must never grow; when one
  shrinks, lower its entry in `xtask/oversized-test-files.txt`).
  **This plan names its decisions `C4b-1` to `C4b-19` (18 and 19 were added
  after review and sit next to the decision they extend). Those ids are for
  this plan only. When a step says "for the reason in C4b-n", write the
  reason itself in the comment, never the id.**
- `task.md` in this folder: §3 slices C4b, C4c, C5 (forward note "from C4b
  and C4c"), C7 (forward note "from C4").
- `docs/planning/changes/signed-route-preamble/change.md`: criteria `SP-1`
  to `SP-9`, Design "Part 1", "Accepted gaps", "Rejected options", "Open
  questions" 1 and 2.
- [ADR-0002](../../../decisions/0002-handshake-authorization-point.md)
  Amendment A1 (rules 1 to 3 land here; rules 4 and 5 are C4c).
- [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
  §6 and Amendment 2 (`on_behalf`).
- The code this slice changes (read before you edit):
  - `crates/router/src/preamble.rs` (whole file).
  - `crates/router/src/handshake.rs:24-83` (`verify_preamble`).
  - `crates/router/src/route_handler/io.rs:158-514` (`build_caller`,
    `read_preamble`, `handle_stream`, `read_and_verify_preamble`,
    `relay_to_next_hop`).
  - `crates/router/src/route_handler/encryption.rs:268-351`
    (`apply_encryption_stage`).
  - `crates/router/src/route_handler.rs:92-497` (`RouteHandlerInner`,
    `init`, `new_coordinator`).
  - `crates/router/src/proxy/router.rs:297-445` (`invoke_remote`,
    `invoke_remote_at`), `crates/router/src/proxy/hop.rs` (`RemoteHop`,
    `IrohHop`), `crates/router/src/proxy/tests/helpers.rs:440-490`
    (`MockHop`).
  - `crates/rpc/src/native.rs:20-55` (`CallerContext`, `CallerProof`),
    `crates/rpc/src/proxy.rs:40-150` (`CallOrigin`, `ProxyError`, codes).
  - `crates/sdk/src/client.rs` (`SyneroymClient`: `connect`,
    `open_request_stream`, `build_preamble`, `request_raw`, `subscribe`,
    `passthrough_with_conn`).
  - `crates/client_gateway/src/gateway.rs:477-536` (`forward_to_service`).
  - `crates/coordinator_webrtc/templates/peer-proxy.js:429-438,587,944`.
  - `crates/coordinator_webrtc/src/bootstrap/tunnel.rs:60-92` (logs).
  - `crates/core/src/dht_registry/client.rs:386-575`
    (`publish_master_anchor`, `refresh_master_anchor`,
    `revoke_instance_key`), `apps/roymctl/src/commands/identity.rs`.
  - `crates/core/src/local_registry.rs:219-264` (`resolve_interface`,
    `lookup`, `lookup_by_service`).
  - `crates/mailbox/src/caller.rs` (reads `CallerProof`; must keep working
    unchanged).

## 1. What C4b builds, in one paragraph

Today a route preamble names a public key (`pubkey`) and maybe a delegation
certificate, but the router never checks that the caller holds the private
key. After C4b, every preamble that names a `pubkey` also carries `ts` (Unix
seconds) and `sig` (an Ed25519 signature by that key over the destination
`service_id`, `interface` and `ts`). The router refuses a missing or wrong
signature, and a `ts` more than 15 minutes from its own clock. The P-256 key
of the end-to-end handshake moves from `pubkey` to a new field `epk`. Only a
coordinator forwards a stream for a service it does not host; any other node
answers `not_hosted_here`, and the SDK, the Universal Proxy (`ProxyRouter`)
and the client gateway look the service up again and retry once. A node that
makes a native call for a caller now presents its own identity and passes the
caller's proof on, unchanged, in a new field `on_behalf`; the destination
checks it and sets `anchor_did`. Refusals become readable by the client. Logs
stop printing identity fields. A separate, first PR adds `roymctl identity
revoke-key`.

Two PRs, in this order:

1. **PR 1 — revoke command** (§4). Small. Independent of everything else.
2. **PR 2 — signed preamble** (§5). Everything else.

## 2. Decisions this plan makes (check them before you start)

The ones marked **(confirmed)** needed an owner decision. The owner
confirmed the recommended form of both on 2026-10-19. Build them as written.

- **C4b-1. How a refusal reaches the client** (change doc open question 1).
  The router writes the refusal in the framing the preamble asked for, so the
  client can read it:
  - `RouteTransport::Binary`: one frame (`framing::write_frame`) holding a
    JSON-RPC error response, `{"jsonrpc":"2.0","id":null,"error":{"code":C,
    "message":M,"data":{"reason":R}}}`.
  - `RouteTransport::Http`: a complete HTTP/1.1 response with status S, the
    header `x-syneroym-refusal: R`, `content-type: application/json`,
    `connection: close`, and the same JSON body.
  - `RouteTransport::Raw`: one text line `R\n`.

  | Refusal | `C` (code) | `R` (reason) | HTTP `S` |
  | --- | --- | --- | --- |
  | not hosted here | `NOT_HOSTED_HERE_RPC_CODE = -32096` (new) | `not_hosted_here` | `421 Misdirected Request` |
  | caller refused | `STREAM_UNAUTHENTICATED_RPC_CODE = -32097` (new) | `unauthenticated` | `401 Unauthorized` |
  | service here, interface unknown | `SERVICE_NOT_FOUND_RPC_CODE` (-32601, existing) | `unknown_interface` | `404 Not Found` |

  `-32096` and `-32097` are free today (the highest used is `-32095`,
  `crates/async_queue/src/dedup.rs`). The old `Unauthorized\n` line goes
  away; nothing outside `io.rs` reads it.

  After it writes a refusal, the router flushes, shuts down its write half
  (QUIC `FIN`), and returns `Ok(())`. A refusal is an answer, not a router
  error, so `RouteHandler::accept` does not log it at `error`.
- **C4b-18. A client reads the answer even when its own write fails.** When
  the router refuses and returns, it drops the read half of the stream, and
  QUIC sends `STOP_SENDING`. Both Rust clients write the whole request before
  they read (`hop.rs:79-84`, `client.rs:533-537`). With a request larger than
  the stream's flow-control window (a deploy with WASM bytes), the client's
  write fails first, so `IrohHop` reports `Transport` (never re-homed) and
  the SDK reports a write error, not `-32096`. The refusal bytes are already
  in the client's receive buffer. So `IrohHop::call` and the SDK's
  `request_once` do this: if writing the preamble or the request frame fails,
  try `framing::read_frame` once, bounded by what is left of the attempt
  timeout (at most 2 s). A frame that parses as a JSON-RPC response is the
  answer; otherwise return the original write error. Rejected: the router
  reads and discards the request before it closes. That makes every refusal
  cost the node the full upload, and a bound on the discard would bring back
  the same failure above the bound.
- **C4b-19. Queued calls retry on the two new codes.** Today a refused
  stream looks like a frame-length error, which is `Transport`, so a queued
  call is retried. After C4b, `Callee { code: -32097 }` would be `Terminal`
  in `syneroym_router::proxy_outbox::disposition_of`
  (`proxy_outbox.rs:105-123`) and in `syneroym_conversation::transport::
  classify` (`crates/conversation/src/transport.rs:632-656`). Some causes are
  temporary: the 5 s master-anchor lookup timeout, and clock skew. So both
  functions classify `NOT_HOSTED_HERE_RPC_CODE` and
  `STREAM_UNAUTHENTICATED_RPC_CODE` like `SERVICE_NOT_FOUND_RPC_CODE`
  (`Disposition::Retry`). A refusal that never clears (an expired
  certificate) then ends in the dead-letter list after the retry budget. A
  queued call cannot carry a forged signature: the node signs it itself.

  For the proxy outbox this is the same end as before: its old `Transport`
  result was also `Retry`. For the conversation outbox it is a **change of
  behaviour**: there the old `Transport` result was `Disposition::
  Unreachable` (`transport.rs:649-651`), which delays the item and does not
  use the attempt budget. So a refused message used to wait until
  `max_pending_age_secs` (30 days by default, `conversation/src/store.rs:92`)
  and then fail with the reason "recipient never became reachable"
  (`conversation/src/outbox.rs:195-204`), which names the wrong cause. Now
  it uses the budget and ends in the dead-letter list. That is wanted: a
  refusal that never clears becomes visible after the retry budget, not
  after 30 days, and is not reported as an unreachable recipient. Record
  it in the change doc's `## Deviations` and in the comment on the
  `classify` arm.
- **C4b-2. Order of checks in `handle_stream`.** A forwarder must not check
  the caller (change doc), and the signature must be checked against the
  `interface` as sent, before `rewrite_empty_http_interface` changes it. So:
  read preamble → "does this node host `service_id`?" → if not: forward
  (coordinator) or refuse `not_hosted_here` → verify the caller → rewrite the
  empty HTTP interface → registry lookup → plan → encrypt → dispatch.
  "Hosts" means the endpoint registry has at least one interface for that
  `service_id` (new `EndpointRegistry::hosts_service`). This answers the
  question without the interface rewrite, which only ever applies to a
  service that is local.
- **C4b-3. A hosted service with an unknown interface is refused, not
  forwarded** (the change doc is silent). Today such a miss goes to
  `relay_to_next_hop`, which resolves the same service back to this node. A
  `not_hosted_here` answer would make the client look up and retry the same
  node. So it gets its own refusal (`unknown_interface`, C4b-1 table).
- **C4b-4. Identity fields without a `pubkey` are refused.** "Identity
  fields" are `delegation`, `ucan`, `ts`, `sig`, `on_behalf`. Today a
  `delegation` without `pubkey` is already refused, but a `ucan` without
  `pubkey` silently becomes anonymous. One rule is simpler and safer: a
  preamble is anonymous only when it has none of `pubkey`, `delegation`,
  `ucan`, `ts`, `sig`, `on_behalf`. Every other preamble must pass the full
  check or is refused. No production writer sends `ucan` without `pubkey`.
- **C4b-5. Strict parse for the new fields.** `RoutePreamble::parse` returns
  `Err` for a present but malformed `ts` (not a `u64`), `sig` (not exactly
  128 lowercase hex characters), or `on_behalf` (not hex of JSON that
  matches `OnBehalfProof`, unknown fields refused). `epk` is kept as a
  string; `encryption.rs` checks it as today. The old fields keep their
  permissive parse (a bad `delegation` or `ucan` becomes `None`). This is
  safe for the old fields only because a dropped `delegation` now still
  needs a valid signature by `pubkey`.

  The parse runs in `read_preamble`, before the forward decision, so a
  coordinator drops a stream with a malformed `ts`, `sig` or `on_behalf`
  instead of forwarding it. This is a deviation from the letter of ADR-0002
  A1 rule 3 ("checks nothing about the caller"), recorded as such. Reasons:
  rule 3 exists so that a forwarder never refuses a *valid* stream because of
  its own clock or a slow anchor lookup; a syntax check has neither cause.
  A forwarder already drops a preamble it cannot parse today (no
  `service_id`). And a forwarder writes the preamble again
  (`to_preamble_line`), so it must parse the fields it writes; keeping raw
  strings only for the forwarder would give the type two shapes. Our writers
  never produce a malformed field (round-trip tests, Step 14). On a parse
  error the router logs at `debug` (with `route_text`), writes nothing and
  returns `Ok(())`: without a parsed preamble it does not know the framing.
- **C4b-6. Where the `on_behalf` proof is kept: inside `CallerProof`.**
  ADR-0016 A2 says "a service that received `on_behalf` passes the same value
  on". So the receiving node must remember it. `CallerContext` has about 130
  struct literals in the workspace; `CallerProof` has 13. So the received
  proof goes in a new field `CallerProof::on_behalf`. The value a native call
  passes on is `proof.on_behalf` when present, else this caller's own proof
  (`CallerProof::to_pass_on`). `CallerProof.pubkey_hex` and
  `delegation_json` keep their meaning (the immediate caller), so
  `crates/mailbox/src/caller.rs` is unchanged.

  The signed part is **optional**: `CallerProof::signed:
  Option<PreambleSignature>`. The router always fills it. C5 and C6 build a
  `CallerProof` in production for a same-node call (an instance key and
  certificate, with no stream and so no signature); they leave `signed` as
  `None`. `to_pass_on()` returns `Option<OnBehalfProof>`: the carried
  `on_behalf`, else this caller's own proof when it is signed, else `None`.
  So an unsigned local proof is never sent as an `on_behalf` that every
  destination would refuse; the call travels with the node's identity only.
- **C4b-7. The `on_behalf` wire type lives in `syneroym-rpc`.** Both
  `CallerProof` (rpc) and `RoutePreamble` (router) need it, and router
  depends on rpc. Name: `OnBehalfProof`. Its JSON is exactly the change doc's
  shape: `{"pubkey","delegation","service_id","interface","ts","sig"}`, with
  `delegation` the certificate JSON as a string, or `null`.
- **C4b-8. Who signs, and when.** Each writer signs at the moment it writes
  the preamble, on every attempt (change doc; C5 forward note). The shared
  primitive is `RoutePreamble::sign(&mut self, key: &Identity, now_secs)`. The
  proxy chooses *who* to present once per call (`OutboundIdentity`), and
  `IrohHop::call` applies and signs it per attempt. `RemoteHop::call` gains an
  `identity: Option<&OutboundIdentity>` parameter.
- **C4b-9. Proxy retry on `not_hosted_here`.** In `invoke_remote` (not the
  retry loop of `invoke_remote_at`): resolve again, and if the new address
  has a different endpoint id, call `invoke_remote_at` once more. If the
  address is the same, or the second attempt also answers `not_hosted_here`,
  return `ProxyError::ServiceNotFound`. That variant is already "retry later"
  for the outbox (`proxy_outbox.rs:117`) and the dead-letter rule
  (`call_dedup.rs:145`). For tests, the resolve step is a closure (§5.8).
- **C4b-10. SDK retry needs interior mutability.** `request`,
  `request_raw` and `subscribe` take `&self`, but a reconnect changes
  `self.connection`. `connection` becomes `std::sync::Mutex<Option<
  TransportConnection>>` and `pending_closes` becomes
  `std::sync::Mutex<Vec<JoinHandle<()>>>`. Never hold either lock across an
  `.await`: clone out, release, then await. The SDK re-homes only when it
  can look the service up again: `provided_mechanisms` is `None` and
  `registry_url` is not empty. Otherwise the readable error is returned.

  Two requests can get `not_hosted_here` at the same time. Re-homing is
  therefore serialised by a `tokio::sync::Mutex<()>` (`rehome_lock`) and is
  compare-before-replace: each request passes the `stable_id()` of the
  connection it used; under the lock, if the current connection has another
  `stable_id`, another request already re-homed, and this one just retries
  on the current connection. So a second re-home never replaces the
  connection the first one just opened.
- **C4b-11 (confirmed). The client gateway also re-homes.** The change doc
  covers the SDK and `ProxyRouter` only. But the gateway caches one
  `SyneroymClient` (one QUIC connection) per service. After a service moves,
  the old host is not a coordinator, so it now refuses instead of forwarding,
  and every gateway request to that service would fail until the cached
  connection closes. That is a regression. Recommended: `passthrough_with_conn`
  detects the HTTP refusal in the first bytes the host sends back, and the
  gateway drops its cached client, connects again (fresh lookup) and replays
  the request once — only when no client bytes beyond `initial_bytes` were
  sent yet (true for every request whose head and body fit the gateway's
  first read; always true for `GET`). Otherwise the 421 response goes to the
  HTTP client as is, and the gateway still drops its cached client so the
  next request works. (Rejected: drop only the cached client on a 421, with
  no replay. The first request after every move would then fail.)
- **C4b-12. `on_behalf` and identity.** A native call (`CallOrigin::Native`)
  always presents this node's own identity: the instance key and certificate
  of `service_id` when it has an unexpired certificate and a recorded owner,
  otherwise the node key — the same rules as today's `(None, Native)` arms.
  If `req.caller.proof` is `Some`, it also sets `on_behalf =
  proof.to_pass_on()`. A guest call (`CallOrigin::Guest`) is unchanged and
  never sets `on_behalf`. The old `(Some(proof), Native)` arm (present the
  caller's proof as this node's identity) is removed.
- **C4b-13. Destination check of `on_behalf`.** After the immediate caller
  passes, if `preamble.on_behalf` is `Some`: refuse when `preamble.ucan` is
  also `Some`; else verify the proof with the same code as a preamble proof
  (signature over *its own* `service_id`/`interface`/`ts`, window,
  certificate, revocation) and set `caller.session.anchor_did =
  Some(verified.master_did)`. `caller_did`, `subject_did` and `auth` stay the
  immediate caller's. Any failure refuses the stream (`unauthenticated`).

  A refused stream is logged once, at `debug`, with the reason, by the code
  that decides the refusal (`verify_caller` for `unauthenticated`,
  `handle_stream` for the other two). `write_refusal` writes and counts, and
  logs nothing. The count is a new counter `substrate.router.refused` with
  label `kind` (`unauthenticated`, `not_hosted_here`, `unknown_interface`),
  named like the other workspace metrics (`substrate.proxy.retries`,
  `substrate.mailbox.deposits_refused{kind}`). Not `warn`
  or `error`: a flood of forged streams must not fill the logs. An operator
  sees refusals in the metric; a client sees the readable refusal.
- **C4b-14. Revoke command name and flags** (change doc open question 2):
  `roymctl identity revoke-key --master <name> --key <did:key> --registry-url
  <url>`. It refuses a `--key` that is not a `did:key`, and refuses the
  master's own DID (revoking the master would cut every key it ever
  delegated). It waits for the DHT copy (`sync_dht: true`), because the
  process exits right after.
- **C4b-15 (confirmed). `publish-anchor` must stop erasing revocations.** Today
  `roymctl identity publish-anchor` publishes `revoked_keys: vec![]`. Run
  after `revoke-key`, it would silently un-revoke every key, so SP-8 would
  hold only until the next `publish-anchor`. PR 1 changes it to the existing
  read-modify-write path (carry forward `revoked_keys` and
  `revoke_list_registry`). Side effect: `publish-anchor` now fails when an
  existing anchor cannot be read, instead of overwriting it. That is the
  behaviour `refresh_master_anchor` already documents as correct.
- **C4b-16. `RegistryClient::revoke_instance_key` is renamed `revoke_key`**
  and gains `sync_dht: bool`. It already does the right read-modify-write for
  any temporary key; only the name is about instance keys. Callers:
  `crates/app_supervisor/src/anchors.rs:74` (`false`),
  `crates/substrate/tests/cert_renewal_e2e.rs:19,328,347` (`false`), roymctl
  (`true`).
- **C4b-17. Logs.** `RoutePreamble` loses `#[derive(Debug)]`. Its `Debug`
  and a new `RoutePreamble::redacted()` view print scheme, interface,
  service id, `enc`, `dir`, `pubkey`, the certificate's master DID, `ts`, and
  only *whether* `epk`, `sig`, `ucan` and `on_behalf` are present. Never
  `sig`, the certificate, the `ucan` chain or the `on_behalf` value. Parse
  errors stop echoing the raw line; they echo only the text before `?`.
  The same rule covers the other types that hold proof material:
  `CallerProof`, `PreambleSignature`, `OnBehalfProof` and `OutboundIdentity`
  get a hand-written `Debug` that prints the key, the master DID and `ts`,
  and only whether a signature, certificate or `on_behalf` is present. So a
  future `?caller` log field cannot leak them.

## 3. Things in task.md and the change doc that are ambiguous or stale

Flagged, not guessed. Each has the decision or the action this plan takes.

1. **Grammar separator.** The change doc (§Terms) and `system-architecture.md`
   write `<scheme>://<interface>.<service_id>`. The code uses `|`
   (`PREAMBLE_SEPARATOR`, `preamble.rs:98`). No effect on signing (the signed
   bytes use field values). Fix the architecture doc text in Step 15.
2. **"The router checks the signature before it changes `interface`."**
   True only after reordering `handle_stream` (C4b-2). Today
   `read_and_verify_preamble` runs before the lookup, which also means a
   forwarder verifies — the opposite of the change doc. C4b-2 fixes both.
3. **Hosted service, unknown interface** is not covered by the change doc.
   See C4b-3.
4. **SDK retry "in its request path"** is not possible with today's `&self`
   signatures. See C4b-10.
5. **The client gateway's passthrough** also meets `not_hosted_here` and is
   not mentioned. See C4b-11.
6. **`publish-anchor` erases revocations**, which defeats SP-8 in practice.
   The change doc says only "a command adds a key ... and keeps the keys that
   are already in the list". See C4b-15.
7. **A command already writes `revoked_keys`**: `RegistryClient::
   revoke_instance_key` (used by the App Supervisor). The backlog row "No
   operator command revokes a person's temporary key" is correct that no
   *person* command exists, but PR 1 reuses that function (C4b-16).
8. **Where `on_behalf` is kept on the receiving node** is not in the change
   doc or ADR-0016 A2. See C4b-6.
9. **Stale comments that describe the old forwarding of the caller's proof**
   (they must be rewritten in Step 12): `crates/rpc/src/fdae_fetch.rs:64-72`
   ("`invoke_remote_at` already forwards `caller.proof` verbatim"),
   `crates/rpc/src/fdae_fetch.rs:78-84` (cites the removed "`(None,
   Native)` arm"), `crates/rpc/src/native.rs:35-44` (`CallerContext::proof`
   doc), `crates/rpc/src/proxy.rs:64-67`,
   `crates/router/src/proxy/router.rs:131-137` ("re-verifies the forwarded
   proof"), and the long comment block at the top of `invoke_remote_at`.
10. **An existing test asserts the old behaviour.**
    `crates/coordinator_iroh/tests/multi_hop_relay.rs`
    `test_cross_node_native_capability_identity_forwarding` asserts that the
    callee sees the *original caller's* master DID as `caller_did`. After
    C4b, `caller_did` is the calling node and `anchor_did` is the original
    caller. Step 14 moves and rewrites it.
11. **"The logs stop printing identity fields"** (task.md) covers more than
    the one line the change doc names: the "Forwarding original preamble"
    line (`io.rs:498`), the parse error text (`preamble.rs:219,287`) which
    `RouteHandler::accept` logs at `error`, the derived `Debug` of
    `RoutePreamble`, and two logs in the WebRTC blind tunnel
    (`tunnel.rs:73` raw line at `debug`, `tunnel.rs:78` raw line on a parse
    error). All are in scope (C4b-17).
12. **A long-lived stream and `on_behalf`.** P's `ts` is from the moment P
    opened its stream. A native call made for P more than 15 minutes later
    (for example an FDAE fetch during a long subscribe or WebSocket) carries
    a proof the destination refuses, and the whole stream is refused. This
    follows the change doc's rule ("If P's proof fails, it refuses the
    stream") but the doc does not name the case. Record it as an accepted gap
    in the backlog (Step 16). No outbox path carries a caller proof today:
    queued calls are guest-origin.
13. **`StopSignal` is not implemented for `tokio::io::DuplexStream`**, so a
    router integration test cannot drive `handle_stream` over an in-memory
    stream. Step 14 adds the empty impl (the trait's default method suffices).
14. **Oversized test files** that this slice must edit:
    `crates/router/src/route_handler/io/tests.rs` (1192, capped) and
    `crates/coordinator_iroh/tests/multi_hop_relay.rs` (1110, capped). New
    tests go in new files; edits there must not add lines.
15. **The change doc's SP-7 says "C refuses the stream when P's proof has a
    `ts` outside the window or a forged signature".** In a chain A → B → C,
    B checks P's proof first, so a bad proof is refused at B and never
    reaches C. The test therefore checks the refusal at the first node that
    receives the bad `on_behalf`, plus a direct crafted stream to C (§6).

## 4. PR 1 — revoke command

Branch: `feat/m06d-c4b-revoke-key` off `main` (or this branch if the owner
prefers one branch per slice). Conventional commit, for example
`feat(roymctl): add identity revoke-key`.

### Step 1.1 — `crates/core/src/dht_registry/client.rs`

1. Rename `revoke_instance_key` to `revoke_key`. Add `sync_dht: bool` as the
   last parameter and pass it to `publish_master_anchor`. Rewrite its doc
   comment: it revokes any temporary key the master delegated — an instance
   key or a person's device key — never the master's own DID.
2. Add `republish_master_anchor(&self, master: &Identity, sync_dht: bool)`:
   the body of today's `refresh_master_anchor` with `sync_dht` passed
   through. Make `refresh_master_anchor(master)` a one-line call to
   `republish_master_anchor(master, false)` so its five callers do not
   change. Keep the long "why read-modify-write" doc comment on
   `republish_master_anchor`.
3. To avoid two copies of the read-modify-write, both `republish_master_anchor`
   and `revoke_key` call one private helper:

   ```rust
   async fn republish_with(
       &self,
       master: &Identity,
       extra_revoked: Option<&str>,
       sync_dht: bool,
   ) -> anyhow::Result<()> {
       let master_did = substrate::derive_did_key(&master.public_key());
       let (mut revoked_keys, revoke_list_registry) = self
           .fetch_own_master_anchor(&master_did)
           .await?
           .map(|prev| (prev.revoked_keys, prev.revoke_list_registry))
           .unwrap_or_default();
       if let Some(did) = extra_revoked
           && !revoked_keys.iter().any(|k| k == did)
       {
           revoked_keys.push(did.to_string());
       }
       self.publish_master_anchor(&master_did, revoked_keys, revoke_list_registry, master, sync_dht)
           .await
   }
   ```
4. Update the callers named in C4b-16.

### Step 1.2 — `apps/roymctl/src/commands/identity.rs`

1. New variant in `IdentityCommands`, after `PublishAnchor`:

   ```rust
   /// Revoke a temporary key of a master: add its DID to the master
   /// anchor's revoked-key list and publish the anchor. Keys already in the
   /// list stay. Nodes refuse the key at its next stream.
   RevokeKey {
       /// Name of the local master identity that delegated the key.
       #[arg(long)]
       master: String,
       /// The `did:key` of the temporary key to revoke.
       #[arg(long)]
       key: String,
       #[arg(long)]
       registry_url: String,
   },
   ```
2. Two functions, so a test can turn the DHT off (with the DHT on,
   `sync_dht: true` waits for a real Mainline DHT publish,
   `dht_registry/client.rs:30-43`, which is slow or hangs without network):

   ```rust
   /// What `identity revoke-key` runs: the HTTP registry plus the DHT, and
   /// waits for the DHT copy because the process exits right after.
   async fn handle_revoke_key(dir: &Path, master: &str, key: &str, registry_url: &str)
       -> anyhow::Result<()>
   {
       let client = RegistryClient::new(true, Some(registry_url.to_string()));
       revoke_key(&client, dir, master, key, true).await
   }

   /// Public so a substrate test can call it with a DHT-less client, like
   /// `roymctl::commands::coordinator::leases`.
   pub async fn revoke_key(
       client: &RegistryClient, dir: &Path, master: &str, key: &str, sync_dht: bool,
   ) -> anyhow::Result<()>
   ```
   `revoke_key` body:

   ```text
   load master identity (require_key_present(dir, master, "Master identity"))
   master_did = derive_did_key(master.public_key())
   substrate::resolve_did_key(key)  -> on Err: bail "--key must be a did:key: {e}"
   if key == master_did             -> bail "--key names the master itself; revoke a delegated key"
   client.revoke_key(&master, key, sync_dht).await?
   println!("Revoked {key} for {master_did}")
   ```
   Do both checks before any network call. A `RegistryClient` built with
   `enable_dht = false` has no DHT client, so `publish_master_anchor` skips
   the DHT step whatever `sync_dht` says.
3. `handle_publish_anchor`: replace the `publish_master_anchor(&master_id,
   vec![], None, &identity, true)` call with
   `client.republish_master_anchor(&identity, true)`. Update the
   `PublishAnchor` doc line: it keeps the revoked keys already published.
4. Wire `IdentityCommands::RevokeKey` to `handle_revoke_key` in the `match`
   at `identity.rs:407`.

### Step 1.3 — Tests (PR 1)

- `apps/roymctl/tests/cli_args.rs`: `identity revoke-key --help` parses and
  lists the three flags.
- `apps/roymctl/src/commands/identity.rs` has no `tests` module today. Add
  `identity/tests.rs` (module layout rule) with two tests: a non-`did:key`
  `--key` and the master's own DID are both refused, with a DHT-less client
  for the registry URL `http://registry.invalid` (a host that never
  resolves, so the test has no port literal). Assert the error text
  (`"must be a did:key"`, `"names the master itself"`): a network failure
  also returns `Err`, so only the text proves the checks ran before any
  network call.
- New `crates/substrate/tests/revoke_key_e2e.rs` (SP-8). Boot one
  `SubstrateNode` (it serves the registry). Create a master identity and save
  it under `<tempdir>/identities/<name>.key` like
  `relay_passport_admin.rs:64-67`. Issue a `routing`-scoped certificate for a
  fresh temporary key; publish the anchor (`refresh_master_anchor`). A
  `SyneroymClient::new_with_identity(node_did, registry_url, temp)
  .with_registry_dht(false).with_delegation(cert)` calls
  `orchestrator`/`readyz`: it succeeds. Call
  `roymctl::commands::identity::revoke_key(&RegistryClient::new(false,
  Some(registry_url)), dir, name, temp_did, false)`. A *new* client with the
  same key and certificate fails. Then call `republish_master_anchor(&master,
  false)` on the same DHT-less client (what `publish-anchor` now runs) and
  check the key is still refused. No step of this test touches the DHT.
  In PR 1 the failure is not yet readable (it is the frame-length error), so
  assert only `is_err()`. PR 2 tightens the assertion to
  `STREAM_UNAUTHENTICATED_RPC_CODE` (Step 14).

### Step 1.4 — Docs and backlog (PR 1)

- `docs/developer-guide.md`: in the identity commands section, add
  `identity revoke-key` and the new `publish-anchor` behaviour.
- `docs/planning/deferred-backlog.md`: move the row "No operator command
  revokes a person's temporary key" (§3) to "Recently resolved".
- Change doc `## Deviations`: `D-C4b-1` the command name and flags (C4b-14),
  `D-C4b-2` `publish-anchor` now carries the list forward (C4b-15), `D-C4b-3`
  the rename (C4b-16). Tick the "revoke command" task box.
- `mise run verify` must pass on its own.

## 5. PR 2 — signed preamble

Commit in small steps. The order below compiles at each step only roughly;
the whole PR passes `mise run verify` at the end.

### Step 2 — `crates/rpc`: codes and proof types

1. `crates/rpc/src/proxy.rs`, next to `SERVICE_NOT_FOUND_RPC_CODE`:

   ```rust
   /// JSON-RPC code a node that is not a coordinator answers with when it
   /// does not host the service a stream names. It comes before any
   /// dispatch, so a caller may look the service up again and retry, for
   /// any call.
   pub const NOT_HOSTED_HERE_RPC_CODE: i32 = -32096;
   /// JSON-RPC code for a stream whose caller identity the router refused:
   /// a missing or wrong signature, a time outside the window, or a bad
   /// certificate.
   pub const STREAM_UNAUTHENTICATED_RPC_CODE: i32 = -32097;
   ```
   Re-export both from `crates/rpc/src/lib.rs`.
2. `crates/rpc/src/native.rs` (C4b-6, C4b-17):

   ```rust
   /// See [`CallerContext::proof`].
   #[derive(Clone, Default, PartialEq, Eq)]
   pub struct CallerProof {
       /// Hex Ed25519 public key (the preamble's `pubkey=`).
       pub pubkey_hex: String,
       /// JSON `DelegationCertificate` (the preamble's `delegation=`, pre-hex).
       pub delegation_json: Option<String>,
       /// What the caller signed on its stream (ADR-0002 A1). The router
       /// always sets it. `None` for a proof built on this node for a
       /// same-node call, which has no stream to sign.
       pub signed: Option<PreambleSignature>,
       /// The original caller's proof that this caller passed on
       /// (ADR-0016 Amendment 2). `None` when this caller is the original.
       pub on_behalf: Option<OnBehalfProof>,
   }

   /// The signed fields of a preamble, as the caller sent them.
   #[derive(Clone, PartialEq, Eq)]
   pub struct PreambleSignature {
       pub service_id: String,
       pub interface: String,
       pub ts: u64,
       /// Hex Ed25519 signature (the preamble's `sig=`).
       pub sig_hex: String,
   }

   /// The original caller's proof, as it travels in `on_behalf=`.
   #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(deny_unknown_fields)]
   pub struct OnBehalfProof {
       pub pubkey: String,
       pub delegation: Option<String>,
       pub service_id: String,
       pub interface: String,
       pub ts: u64,
       pub sig: String,
   }

   impl CallerProof {
       /// The proof a native call made for this caller passes on: the one
       /// this caller carried, else this caller's own when it is signed.
       /// `None` for an unsigned local proof: a destination would refuse
       /// it, so it is not sent. Never wrapped.
       #[must_use]
       pub fn to_pass_on(&self) -> Option<OnBehalfProof> {
           if let Some(carried) = &self.on_behalf {
               return Some(carried.clone());
           }
           let signed = self.signed.as_ref()?;
           Some(OnBehalfProof {
               pubkey: self.pubkey_hex.clone(),
               delegation: self.delegation_json.clone(),
               service_id: signed.service_id.clone(),
               interface: signed.interface.clone(),
               ts: signed.ts,
               sig: signed.sig_hex.clone(),
           })
       }
   }
   ```
   Hand-written `Debug` for all three (C4b-17): `CallerProof` prints
   `pubkey_hex`, `delegation: present|absent`, `signed` (its `Debug`), and
   `on_behalf` (its `Debug`); `PreambleSignature` prints `service_id`,
   `interface`, `ts`, `sig: present`; `OnBehalfProof` prints `pubkey`,
   `delegation: present|absent`, `service_id`, `interface`, `ts`,
   `sig: present`. `CallerContext` keeps its derived `Debug`, which now uses
   these.

   Re-export `PreambleSignature` and `OnBehalfProof` from `lib.rs`. Rewrite
   the `CallerContext::proof` doc comment: the destination re-verifies it; a
   node never presents it as its own identity; a native call passes it on in
   `on_behalf` when it is signed. `#[derive(Default)]` gives an unsigned
   proof: test literals add `..Default::default()` (sites in Step 13), and C5
   and C6 may build one in production for a same-node call.
3. **Retry on the new codes** (C4b-19). In
   `crates/router/src/proxy_outbox.rs` `disposition_of` and
   `crates/conversation/src/transport.rs` `classify`, add
   `NOT_HOSTED_HERE_RPC_CODE` and `STREAM_UNAUTHENTICATED_RPC_CODE` to the
   `Callee` arm that already lists `SERVICE_NOT_FOUND_RPC_CODE`, with one
   comment line: the refusal comes before any dispatch, and some of its causes
   (an anchor lookup timeout, clock skew) clear on their own. Extend each
   function's existing unit test with both codes.

### Step 3 — `crates/router/src/preamble.rs`: fields, signing, redaction

`preamble.rs` is 548 lines and gains about 250. Split it first, with no
behaviour change, in its own commit:

- Move the inline `mod tests { ... }` to `crates/router/src/preamble/tests.rs`
  (`#[cfg(test)] mod tests;` in `preamble.rs`).
- New code goes in `preamble/proof.rs` (signed bytes, `sign`,
  `check_signature`, `OutboundIdentity`) and `preamble/redacted.rs`
  (`RedactedPreamble`, `route_text`). Declare both with `mod` in
  `preamble.rs` and re-export the public items.

Then:

1. **New fields** on `RoutePreamble`, after `ucan`:

   ```rust
   /// Unix seconds when the caller signed (ADR-0002 A1).
   pub ts: Option<u64>,
   /// Ed25519 signature by `pubkey` over `proof_bytes(service_id, interface,
   /// ts)`, lowercase hex.
   pub sig: Option<String>,
   /// The caller's P-256 key for the end-to-end handshake, lowercase hex
   /// SEC1. `pubkey` is only ever the Ed25519 identity key.
   pub epk: Option<String>,
   /// The original caller's proof, passed on by a service in a call chain
   /// (ADR-0016 Amendment 2).
   pub on_behalf: Option<OnBehalfProof>,
   ```
   Fix the `pubkey` doc: "Ed25519 identity key of the caller (hex)".
2. **`RoutePreamble::new(transport, protocol, service_id, interface)`**, every
   optional field `None`. `binary_json_rpc` and `from_http_path` call it.
   Every struct literal in the workspace (the compiler lists them: `sdk`
   `passthrough_with_conn` and the test files in Step 13; the two in
   `route_handler/http/blobs.rs` already end in `..self.preamble.clone()` and
   need no change) uses `RoutePreamble { .., ..RoutePreamble::new(..) }` or
   sets the four new fields to `None`.
3. **Parse** (C4b-5). In the query loop add:

   ```text
   "ts"        => ts = Some(v.parse::<u64>().map_err(|_| anyhow!("invalid ts in preamble"))?)
   "sig"       => require v.len()==128 && all lowercase hex, else Err("invalid sig in preamble"); sig = Some(v)
   "epk"       => epk = Some(v)
   "on_behalf" => on_behalf = Some(decode_on_behalf(v)?)   // hex -> utf8 -> serde_json strict
   ```
   Keep the loop body a table: move the old `delegation`/`ucan` arms and the
   new arms into small helper fns so `parse` stays under 100 lines and each
   match arm stays short.
4. **Display** writes the parameters in this fixed order: `enc`, `epk`,
   `pubkey`, `delegation`, `ucan`, `ts`, `sig`, `on_behalf`, `dir`. `on_behalf` is `hex::encode(serde_json::to_string(&p))`. The order
   does not affect the signature, but a fixed order makes round-trip tests
   simple.
5. **`has_identity_fields(&self) -> bool`**: any of `pubkey`, `delegation`,
   `ucan`, `ts`, `sig`, `on_behalf` is `Some` (C4b-4).
6. **`caller_proof(&self) -> Option<CallerProof>`**: `Some` when `pubkey`,
   `ts` and `sig` are all `Some`. It fills `pubkey_hex`, `delegation_json`
   (from `delegation.to_json()`), `signed: Some(PreambleSignature { .. })`
   with `service_id` and `interface` as they are *now* (callers use it
   before any rewrite), and `on_behalf`. Used by `build_caller` and by tests
   that need a real signed proof.
7. **`preamble/proof.rs`**:

   ```rust
   /// Domain tag of the signed bytes (ADR-0002 A1).
   const PROOF_TAG: &[u8] = b"syneroym-preamble-v1";

   /// How far a preamble's `ts` may be from the receiver's clock.
   pub const PROOF_WINDOW_SECS: u64 = 15 * 60;

   /// The bytes a caller signs: the tag, `service_id`, `interface` and `ts`
   /// as decimal text, each as a 4-byte big-endian length and then the
   /// bytes, so no value can be read as a separator.
   pub(crate) fn proof_bytes(service_id: &str, interface: &str, ts: u64) -> Vec<u8> {
       let ts = ts.to_string();
       let mut out = Vec::with_capacity(4 * 4 + PROOF_TAG.len() + service_id.len() + interface.len() + ts.len());
       for field in [PROOF_TAG, service_id.as_bytes(), interface.as_bytes(), ts.as_bytes()] {
           out.extend_from_slice(&(field.len() as u32).to_be_bytes());
           out.extend_from_slice(field);
       }
       out
   }

   impl RoutePreamble {
       /// Names `key` as the caller and proves it holds it: sets `pubkey`,
       /// `ts` and `sig` over this preamble's `service_id` and `interface`.
       /// Call it after the route fields are final.
       pub fn sign(&mut self, key: &Identity, now_secs: u64) {
           self.pubkey = Some(hex::encode(key.public_key().to_bytes()));
           self.ts = Some(now_secs);
           let sig = key.sign(&proof_bytes(&self.service_id, &self.interface, now_secs));
           self.sig = Some(hex::encode(sig.to_bytes()));
       }
   }

   /// Checks `sig_hex` by `key` over the proof bytes.
   pub(crate) fn check_signature(
       key: &VerifyingKey, service_id: &str, interface: &str, ts: u64, sig_hex: &str,
   ) -> Result<()>   // hex decode -> [u8; 64] -> Signature::from_bytes -> key.verify_strict(..)

   /// Who an outbound call presents itself as. Chosen once per call, and
   /// signed into the preamble on every attempt.
   #[derive(Clone)]   // hand-written Debug: key DID, master DID, on_behalf present or not
   pub struct OutboundIdentity {
       pub key: Arc<Identity>,
       pub delegation: Option<DelegationCertificate>,
       pub on_behalf: Option<OnBehalfProof>,
   }

   impl OutboundIdentity {
       pub fn key_only(key: Arc<Identity>) -> Self { .. }
       /// `route` with this identity set and signed at `now_secs`.
       #[must_use]
       pub fn apply(&self, route: &RoutePreamble, now_secs: u64) -> RoutePreamble {
           let mut p = route.clone();
           p.delegation = self.delegation.clone();
           p.on_behalf = self.on_behalf.clone();
           p.sign(&self.key, now_secs);
           p
       }
   }
   ```
   Use `verify_strict` (it refuses weak keys and non-canonical signatures).
   `Identity` does not implement `Clone`, so `OutboundIdentity` holds an
   `Arc<Identity>`, the way the proxy already holds the node key
   (`proxy.rs:136`).
8. **`preamble/redacted.rs`** (C4b-17):

   ```rust
   /// A log view of a preamble: no signature, certificate, capability chain
   /// or `on_behalf` value.
   pub struct RedactedPreamble<'a>(&'a RoutePreamble);
   impl Display for RedactedPreamble<'_> { /* scheme://iface|svc enc=.. dir=..
       pubkey=.. master=<cert master DID or -> ts=.. epk=<set|-> sig=<set|->
       ucan=<set|-> on_behalf=<set|-> */ }
   impl RoutePreamble { pub fn redacted(&self) -> RedactedPreamble<'_> }
   impl Debug for RoutePreamble { fn fmt(..) { Display::fmt(&self.redacted(), f) } }
   /// The route part of a raw preamble line (text before `?`), at most 256
   /// characters, for error messages and logs.
   pub fn route_text(raw: &str) -> &str
   ```
   Factor the scheme string of `Display` into a private `fn scheme(&self) ->
   String` so both views use it.
9. Change the two parse errors to use `route_text(raw)` instead of `raw`.
10. Update the module doc's parameter table: add `ts`, `sig`, `epk`,
    `on_behalf`; change the `enc`/`pubkey` row to `enc`/`epk`; state that a
    `pubkey` needs `ts` and `sig`. Replace "Authentication status" with the
    current rule (signature, window, certificate, revocation; anonymous only
    when there is no identity field).
11. Re-export from `crates/router/src/lib.rs`: `OutboundIdentity`,
    `PROOF_WINDOW_SECS`, `route_text`.

### Step 4 — `crates/router/src/handshake.rs`: one check for both proofs

`handshake.rs` is 430 lines, 345 of them tests. First move the inline tests,
unchanged, to `handshake/tests.rs`; new tests go to `handshake/tests/proof.rs`
and `handshake/tests/on_behalf.rs` (declared from `handshake/tests.rs`).

```rust
/// The parts of one signed proof, from a preamble or from `on_behalf`.
struct Presented<'a> {
    pubkey_hex: &'a str,
    delegation: Option<DelegationCertificate>,
    service_id: &'a str,
    interface: &'a str,
    ts: Option<u64>,
    sig_hex: Option<&'a str>,
}

impl HandshakeVerifier {
    /// Wall-clock form, kept for existing callers.
    pub async fn verify_preamble(preamble, resolver) -> Result<VerifiedIdentity> {
        Self::verify_preamble_at(preamble, resolver, now_secs()).await
    }

    pub async fn verify_preamble_at(preamble: &RoutePreamble, resolver, now_secs: u64)
        -> Result<VerifiedIdentity>
    {
        let pubkey_hex = preamble.pubkey.as_deref()
            .ok_or_else(|| anyhow!("Missing client public key (pubkey) in preamble"))?;
        verify_presented(Presented {
            pubkey_hex, delegation: preamble.delegation.clone(),
            service_id: &preamble.service_id, interface: &preamble.interface,
            ts: preamble.ts, sig_hex: preamble.sig.as_deref(),
        }, resolver, now_secs).await
    }

    pub async fn verify_on_behalf_at(proof: &OnBehalfProof, resolver, now_secs: u64)
        -> Result<VerifiedIdentity>
    {
        // Strict here: a certificate that does not parse is an error, not "no certificate".
        let delegation = proof.delegation.as_deref()
            .map(DelegationCertificate::from_json).transpose()
            .map_err(|e| anyhow!("unreadable on_behalf certificate: {e}"))?;
        verify_presented(Presented { pubkey_hex: &proof.pubkey, delegation,
            service_id: &proof.service_id, interface: &proof.interface,
            ts: Some(proof.ts), sig_hex: Some(&proof.sig) }, resolver, now_secs).await
    }
}

async fn verify_presented(p: Presented<'_>, resolver, now_secs) -> Result<VerifiedIdentity> {
    let key = decode_ed25519(p.pubkey_hex)?;                       // today's three error cases
    let ts = p.ts.ok_or_else(|| anyhow!("preamble names a pubkey but has no ts"))?;
    let sig = p.sig_hex.ok_or_else(|| anyhow!("preamble names a pubkey but has no sig"))?;
    if ts.abs_diff(now_secs) > PROOF_WINDOW_SECS {
        return Err(anyhow!("preamble ts is more than {PROOF_WINDOW_SECS} s from this node's clock"));
    }
    check_signature(&key, p.service_id, p.interface, ts, sig)?;   // cheap checks first,
    let temporary_did = substrate::derive_did_key(&key);         // before any network lookup
    let Some(cert) = p.delegation else {
        return Ok(VerifiedIdentity { master_did: temporary_did.clone(), temporary_did });
    };
    cert.verify_at(&cert.master_did, &TRANSPORT_SCOPES, now_secs)?;
    // ... today's temporary_did match and 5 s master-anchor revocation check, unchanged
}
```

Keep today's comment about the confused-deputy no-op and the scope check.
The window and signature run before the anchor lookup on purpose: a forged
stream costs the node no network round trip.

### Step 5 — `encryption.rs`: `epk`

In `apply_encryption_stage`, read `preamble.epk` instead of `preamble.pubkey`;
the error text names `epk`. Nothing else changes (HKDF is C4c). Update
`crates/router/benches/preamble.rs:29` (`pubkey=` → `epk=`).

### Step 6 — Refusals: `crates/router/src/route_handler/refusal.rs` (new)

```rust
/// Why the router closed a stream before dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal { NotHostedHere, Unauthenticated, UnknownInterface }

impl Refusal {
    const fn code(self) -> i32;          // C4b-1 table
    const fn reason(self) -> &'static str;
    const fn http_status(self) -> &'static str;   // "421 Misdirected Request", ...
    fn json_body(self) -> Vec<u8>;       // JsonRpcErrorResponse, id: None, data: {"reason": ..}
}

/// Writes `refusal` in the framing `transport` uses, flushes, shuts the
/// write half down (QUIC `FIN`), and counts it in
/// `substrate.router.refused{kind}`. It does not log: the caller that
/// decided the refusal logs it once. Write errors are ignored: the peer may
/// already be gone, and the stream ends either way.
pub(crate) async fn write_refusal(
    writer: &mut (impl AsyncWrite + Unpin), transport: RouteTransport, refusal: Refusal,
)
```
`Http` writes `format!("HTTP/1.1 {status}\r\n{REFUSAL_HEADER}: {reason}\r\n
content-type: application/json\r\ncontent-length: {len}\r\nconnection:
close\r\n\r\n")` then the body. Export `pub const REFUSAL_HEADER: &str =
"x-syneroym-refusal";` from `crates/router/src/lib.rs` (the SDK reads it).
Declare `mod refusal;` in `route_handler.rs`.

### Step 7 — `handle_stream`: the forward flag and the new order

1. `crates/core/src/local_registry.rs`: add

   ```rust
   /// Whether this node hosts any interface of `service_id`.
   #[must_use]
   pub fn hosts_service(&self, service_id: &str) -> bool {
       self.active_endpoints.iter().any(|e| e.key().0 == service_id)
   }
   ```
   (A linear scan, like `lookup_by_service`. Fine at today's sizes.)
2. `RouteHandlerInner`: new field

   ```rust
   /// Whether a stream for a service this node does not host is forwarded
   /// to its host (ADR-0002 A1). Only a coordinator forwards; any other
   /// node answers `not_hosted_here`.
   pub forwards: bool,
   ```
   `init` sets `false`; `new_coordinator` sets `true`.
3. `io.rs` `handle_stream`, new body (keep it under 100 lines; move steps
   into helpers as needed):

   ```text
   stop_signal = stream.stop_signal()
   split, BufReader
   preamble = match timeout(PRE_AUTH_READ_TIMEOUT, read_preamble(&mut reader)).await {
       Err(_) => return Err(anyhow!("timed out reading route preamble")),   // as today
       Ok(Err(e)) => { debug!("[Router] unreadable preamble: {e}"); return Ok(()) }   // C4b-5
       Ok(Ok(p)) => p,
   }
   debug!("[Router] Preamble received: {}", preamble.redacted())

   if !self.inner.registry.hosts_service(&preamble.service_id) {
       if self.inner.forwards {
           return self.relay_to_next_hop(preamble, reader, writer).await   // no caller check
       }
       debug!(service_id = %preamble.service_id, "not hosted here; refused");
       write_refusal(&mut writer, preamble.transport, Refusal::NotHostedHere).await;
       return Ok(())
   }

   // The signature is checked against the interface as sent, before the rewrite below.
   let caller = match self.verify_caller(&preamble).await {
       CallerCheck::Anonymous => None,
       CallerCheck::Verified(caller) => Some(caller),
       CallerCheck::Refused => {
           write_refusal(&mut writer, preamble.transport, Refusal::Unauthenticated).await;
           return Ok(())
       }
   };

   self.rewrite_empty_http_interface(&mut preamble)
   let Some((endpoint, canonical_interface)) = registry.lookup(..) else {
       debug!(service_id = %preamble.service_id, "unknown interface; refused");
       write_refusal(&mut writer, preamble.transport, Refusal::UnknownInterface).await;
       return Ok(())
   }
   ... unchanged from "preamble.interface = canonical_interface" on
   ```
4. Replace `read_and_verify_preamble` with `verify_caller`:

   ```rust
   /// The caller of a stream this node hosts.
   enum CallerCheck {
       /// The preamble has no identity field at all.
       Anonymous,
       Verified(CallerContext),
       /// The preamble names an identity that did not prove itself.
       Refused,
   }

   async fn verify_caller(&self, preamble: &RoutePreamble) -> CallerCheck {
       if !preamble.has_identity_fields() { return CallerCheck::Anonymous; }
       match self.check_caller(preamble).await {
           Ok(caller) => CallerCheck::Verified(caller),
           Err(e) => {
               // Once, at debug (C4b-13): forged streams must not flood the logs.
               debug!(service_id = %preamble.service_id, "caller refused: {e}");
               CallerCheck::Refused
           }
       }
   }

   async fn check_caller(&self, preamble: &RoutePreamble) -> Result<CallerContext> {
       if preamble.on_behalf.is_some() && preamble.ucan.is_some() {
           return Err(anyhow!("a preamble carries on_behalf or ucan, not both"));
       }
       let now = now_secs();
       let resolver = self.inner.registry_client.as_ref();
       let id = HandshakeVerifier::verify_preamble_at(preamble, resolver, now).await?;
       let mut caller = build_caller(preamble, &id, /* as today */).await;
       if let Some(proof) = &preamble.on_behalf {
           let original = HandshakeVerifier::verify_on_behalf_at(proof, resolver, now).await?;
           caller.session.anchor_did = Some(original.master_did);
       }
       Ok(caller)
   }
   ```
   `build_caller` builds `proof: preamble.caller_proof()` instead of its
   inline `CallerProof { .. }`. Its doc comment's last line about a malformed
   delegation being a hard reject "in `handle_stream`" stays true; point it at
   `verify_caller`. Every refusal path in `handle_stream` returns `Ok(())`,
   so `RouteHandler::accept` (`route_handler.rs:571-576`) does not log it a
   second time at `error`. The error text from `verify_presented` names DIDs and
   reasons only, never `sig` or a certificate, so logging it is fine (SP-9).
5. `relay_to_next_hop`: replace the "Forwarding original preamble" debug line
   with `debug!("[Router] Forwarding preamble: {}", preamble.redacted())`.
   The `to_preamble_line()` write stays: the new fields round-trip.
6. `crates/router/src/stop_signal.rs`: add `impl StopSignal for
   tokio::io::DuplexStream {}` with a one-line comment (an in-memory stream
   has no stop signal; tests drive `handle_stream` over it).

### Step 8 — Proxy: identity, signing per attempt, re-homing

1. `crates/router/src/proxy/hop.rs`:

   ```rust
   pub trait RemoteHop: Send + Sync + Debug {
       async fn call(
           &self,
           addr: &EndpointAddr,
           route: &RoutePreamble,
           identity: Option<&OutboundIdentity>,
           request: &JsonRpcRequest,
           timeout: Duration,
       ) -> Result<Value, ProxyError>;
   }
   ```
   `IrohHop::call`, inside the timed attempt, just before the write:

   ```rust
   let preamble = identity.map_or_else(|| route.clone(), |id| id.apply(route, now_secs()));
   ```
   Then the write and read become (C4b-18):

   ```text
   let wrote = async {
       send.write_all(preamble line)?; framing::write_frame(&mut send, &body)?; send.finish()
   }.await;
   let frame = match wrote {
       Ok(()) => framing::read_frame(&mut recv).await.map_err(transport_err)?,
       // The node may have answered and stopped reading (a refusal before a
       // large request was fully sent). Its answer is still in our receive buffer.
       Err(write_err) => match timeout(REFUSAL_READ_GRACE, framing::read_frame(&mut recv)).await {
           Ok(Ok(frame)) if !frame.is_empty() => frame,
           _ => return Err(transport_err(write_err)),
       },
   };
   ```
   `REFUSAL_READ_GRACE = 2 s`, a private const in `hop.rs`; the whole attempt
   still sits under the call `timeout`.
   `MockHop` (`proxy/tests/helpers.rs`) applies the identity the same way
   (with a fixed `now`) and stores the signed preamble in `last_preamble`, so
   existing assertions on `pubkey`/`delegation` keep working.
2. `crates/router/src/proxy/router.rs`:

   ```rust
   /// Who this node presents on an outbound call (ADR-0016 Amendment 2).
   /// A node never presents another party's proof as its own identity. A
   /// native call made for a caller passes that caller's proof on in
   /// `on_behalf`; a guest call never does, because the guest chooses the
   /// call and must not travel under the caller's authority.
   fn outbound_identity(&self, req: &ProxyRequest) -> Option<OutboundIdentity> {
       match &req.origin {
           CallOrigin::Native { service_id } => {
               let mut id = service_id.as_deref()
                   .and_then(|sid| self.instance_identity(sid))
                   .unwrap_or_else(|| OutboundIdentity::key_only(self.node_identity.clone()));
               id.on_behalf = req.caller.proof.as_ref().and_then(CallerProof::to_pass_on);
               Some(id)
           }
           CallOrigin::Guest { service_id } => self.instance_identity(service_id),
       }
   }

   /// The certified instance identity of `service_id`: its derived key and
   /// unexpired certificate. `None` without both a certificate and an owner.
   fn instance_identity(&self, service_id: &str) -> Option<OutboundIdentity> {
       let cert = self.registry.instance_cert(service_id).filter(|c| !c.is_expired())?;
       let owner = self.registry.owner_of(service_id)?;
       let key = self.node_identity.derive_service_identity(&owner, service_id);
       Some(OutboundIdentity { key: Arc::new(key), delegation: Some(cert), on_behalf: None })
   }
   ```
   Keep, shortened, the two reasons in today's comment block: why a guest may
   present its own instance identity, and why an expired certificate is worse
   than none. Delete the rest (it describes the removed arm).
   `invoke_remote_at` builds `route = RoutePreamble::binary_json_rpc(..)` and
   `identity = self.outbound_identity(req)` once, and passes
   `identity.as_ref()` to every `self.hop.call(..)` in the loop.
3. Re-homing (C4b-9). Split `invoke_remote`:

   ```rust
   pub(super) async fn invoke_remote(&self, req: &ProxyRequest) -> Result<Value, ProxyError> {
       self.invoke_rehoming(req, || self.resolve_addr(&req.target_service)).await
   }

   /// `None` address (a peer with only a mailbox) is `ServiceNotFound`, as today.
   async fn resolve_addr(&self, target: &str) -> Result<EndpointAddr, ProxyError>;

   /// Calls the resolved address; on `not_hosted_here`, resolves again and
   /// calls once more. The refusal comes before any dispatch, so this is safe
   /// for every call, idempotent or not.
   pub(super) async fn invoke_rehoming<F, Fut>(&self, req: &ProxyRequest, mut resolve: F)
       -> Result<Value, ProxyError>
   where F: FnMut() -> Fut, Fut: Future<Output = Result<EndpointAddr, ProxyError>>
   {
       let addr = resolve().await?;
       match self.invoke_remote_at(&addr, req).await {
           Err(e) if is_not_hosted_here(&e) => {
               let fresh = resolve().await?;
               if fresh.id == addr.id {
                   return Err(ProxyError::ServiceNotFound(req.target_service.clone()));
               }
               self.invoke_remote_at(&fresh, req).await.map_err(|e| {
                   if is_not_hosted_here(&e) { ProxyError::ServiceNotFound(req.target_service.clone()) } else { e }
               })
           }
           other => other,
       }
   }

   fn is_not_hosted_here(e: &ProxyError) -> bool {
       matches!(e, ProxyError::Callee { code, .. } if *code == NOT_HOSTED_HERE_RPC_CODE)
   }
   ```
   `router.rs` is 628 lines; if it passes 800, move `outbound_identity`,
   `instance_identity` and the re-homing functions to a sibling
   `proxy/remote.rs`.

### Step 9 — SDK (`crates/sdk`)

`client.rs` is 729 lines. Put new code in siblings under the existing
`crates/sdk/src/client/` directory: `client/rehome.rs` and
`client/passthrough.rs` (move `passthrough` and `passthrough_with_conn`
there).

1. **Fields** (C4b-10): `connection: std::sync::Mutex<Option<
   TransportConnection>>`, `pending_closes: std::sync::Mutex<Vec<
   JoinHandle<()>>>`. Update every use: constructors, `connect` (`get_mut`),
   `connect_with_mechanisms`, `connection()` (lock, clone), `shutdown` and
   `Drop` (take), `open_request_stream` (lock, clone the `Connection`, drop
   the guard, then `open_bi().await`). Use `lock().unwrap_or_else(
   PoisonError::into_inner)` or the workspace's usual poison handling; check
   how other crates do it and match.
2. Extract `async fn dial(&self, mechanisms: Vec<EndpointMechanism>) ->
   Result<TransportConnection>` from `connect_with_mechanisms` (failed
   endpoints still go to `pending_closes`). `connect_with_mechanisms` becomes
   "dial, then store".
3. New `pub fn disconnect(&mut self)`: take the connection; close its
   endpoint in the background and keep the handle in `pending_closes`.
4. **Sign** in `build_preamble`: after setting `ucan` and `delegation`, call
   `preamble.sign(&self.identity, now_secs())`. Remove the `TODO(FDAE)`
   paragraph on the `identity` field (the key is now proved) and its row in
   the backlog's "Open in-code markers" (Step 16).
5. **Re-home** (`client/rehome.rs`):

   ```rust
   impl SyneroymClient {
       fn can_rehome(&self) -> bool {
           self.provided_mechanisms.is_none() && !self.registry_url.is_empty()
       }
   }

   pub(crate) fn is_not_hosted_here(e: &anyhow::Error) -> bool {
       e.downcast_ref::<JsonRpcError>().is_some_and(|j| j.code == NOT_HOSTED_HERE_RPC_CODE)
   }
   ```
   With C4b-10's compare-before-replace, `rehome` takes the connection the
   failed request used:

   ```rust
   /// Swaps in a connection to the service's current host, unless another
   /// request already did since `failed` was used.
   async fn rehome(&self, failed: &TransportConnection) -> Result<()> {
       let _serial = self.rehome_lock.lock().await;
       if self.current_stable_id() != Some(stable_id(failed)) {
           return Ok(());   // another request re-homed; retry on its connection
       }
       let target = self.registry_lookup_override.as_deref().unwrap_or(&self.service_id);
       let info = self.registry_client().lookup(target, true).await?.info;
       let conn = self.dial(info.mechanisms).await?;
       let old = lock(&self.connection).replace(conn);
       if let Some(TransportConnection::Iroh { endpoint, .. }) = old {
           lock(&self.pending_closes).push(spawn_background_close(endpoint));
       }
       Ok(())
   }
   ```
   New field
   `rehome_lock: tokio::sync::Mutex<()>`. Closing the old endpoint can cut a
   request still in flight on it; that request was talking to a node that
   no longer hosts the service, so it would have been refused anyway.

   `request_raw` becomes a two-try wrapper around today's body (renamed
   `request_once`, which now also returns the `TransportConnection` it used):
   on `Err(e)` with `is_not_hosted_here(&e) && self.can_rehome()`, call
   `rehome(&used)` and `request_once` again, once.

   `request_once` and `open_request_stream` also follow C4b-18: if writing the
   preamble or the request frame fails, try `framing::read_frame` once within
   2 s; a frame that parses as a JSON-RPC response (success or error) is the
   answer, else return the write error.
   `subscribe` does the same around "open stream + read the ack": move the
   ack parse to the same error-response fallback `request_raw` uses, so a
   refusal frame becomes a `JsonRpcError` there too.
6. **Passthrough** (`client/passthrough.rs`), C4b-11 recommended form:

   ```rust
   /// What happened to a passed-through HTTP request.
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum PassthroughOutcome {
       /// The host answered (any HTTP status); bytes were copied both ways.
       Done,
       /// The node refused with `not_hosted_here`. `replayable` is true when
       /// nothing was sent to the client and no client bytes beyond
       /// `initial_bytes` were consumed, so the caller may send the same
       /// request to the new host.
       NotHostedHere { replayable: bool },
   }

   pub async fn passthrough_with_conn(/* same params */) -> Result<PassthroughOutcome>
   ```
   Body:

   ```text
   open_bi; build the Http preamble (RoutePreamble::new + delegation), preamble.sign(identity, now)
   write preamble line, write initial_bytes
   let (mut tcp_r, mut tcp_w) = tcp_stream.split()
   let mut client_sent_more = false; let mut tcp_open = true
   // Phase 1: wait for the host's first bytes; forward client bytes meanwhile.
   loop {
       select! {
           r = recv.read(&mut first) => {
               n = r?; if n == 0 { return Ok(Done) }
               if starts_with_not_hosted_refusal(&first[..n]) {
                   if !client_sent_more { return Ok(NotHostedHere { replayable: true }) }
                   tcp_w.write_all(&first[..n]).await?; return Ok(NotHostedHere { replayable: false })
               }
               tcp_w.write_all(&first[..n]).await?; break
           }
           r = tcp_r.read(&mut buf), if tcp_open => {
               n = r?; if n == 0 { send.finish()?; tcp_open = false } else { send.write_all(&buf[..n]).await?; client_sent_more = true }
           }
       }
   }
   // Phase 2: copy both ways until either side closes (as today).
   ```
   C4b-18 applies here too: if writing the preamble, `initial_bytes` or a
   forwarded client chunk fails in phase 1, stop reading from the client
   (`tcp_open = false`, `client_sent_more` stays as it was) and keep waiting
   for the host's first bytes for at most 2 s; a refusal found there is
   handled as above, otherwise return the write error.
   `starts_with_not_hosted_refusal`: the chunk starts with `b"HTTP/1.1 421 "`
   and its header block (up to the first `\r\n\r\n`, or the chunk end)
   contains `x-syneroym-refusal: not_hosted_here` (ASCII case-insensitive
   name). The router writes the whole refusal in one write and flush, and it
   is under 300 bytes, so it arrives as one chunk in practice; a split chunk
   is passed through as a normal 421. If phase 2 cannot reuse
   `io::copy_bidirectional` after a half-close in phase 1, finish phase 2
   with one `io::copy` per direction joined with `tokio::try_join!`.
   `passthrough` (the `&self` wrapper) returns the outcome too.

### Step 10 — Client gateway

`crates/client_gateway/src/gateway.rs` `forward_to_service`:

1. Extract `async fn connected(client_arc, connect_service_id, is_auth_host)
   -> Result<(TransportConnection, String)>` (the current locked block) so
   `forward_to_service` stays short.
2. After `passthrough_with_conn`:

   ```text
   match outcome {
       Done => Ok(()),
       NotHostedHere { replayable } => {
           client_arc.lock().await.disconnect();          // next connect() looks up again
           if !replayable { return Ok(()) }               // client already has the 421
           let (conn, id) = connected(..).await?;         // fresh lookup
           match passthrough_with_conn(conn, &id, ..same args..).await? {
               NotHostedHere { .. } => write_json_rpc_error(stream, 502, "Bad Gateway").await,
               Done => Ok(()),
           }
       }
   }
   ```

### Step 11 — Browser handshake key

`crates/coordinator_webrtc/templates/peer-proxy.js`:
- line 438: `preamble.replace(/epk=[^&\n]*/, \`epk=${clientPubHex}\`)`;
- lines 587 and 944: `?enc=ecdh-p256&epk=placeholder`.

The page sends no `pubkey`; its identity is the session cookie. Playwright
`webrtc.spec.ts` and `multi-hop.spec.ts` (forced tunnel) cover it (SP-5).

### Step 12 — Logs and stale comments elsewhere

- `crates/coordinator_webrtc/src/bootstrap/tunnel.rs:73`: log
  `route_text(&preamble_str)`, not the raw line; `:78`: same.
- `io.rs:377-386` debug line: replace with the `redacted()` line from Step
  7.3 (one line, not two).
- Search once more: `rg -n 'preamble' --type rust | rg '(debug|info|warn|error|trace)!'`
  and `rg -n '\{preamble:\?\}|\?preamble|%preamble'`. Every hit prints a
  field, `redacted()` or `route_text`.
- Rewrite the stale comments named in §3 item 9.

### Step 13 — Update existing tests and fixtures (compiler-led)

These build preambles or `CallerProof`s and break or start failing:

- `CallerProof` literals add the new fields or `..Default::default()`:
  `crates/mailbox/src/test_support.rs:62`, `crates/mailbox/src/caller/tests.rs`
  (47, 61, 93, 120), `crates/mailbox/src/service/tests.rs` (330, 411),
  `crates/router/src/proxy/tests/remote_dispatch.rs` (72, 160, 403),
  `crates/coordinator_iroh/tests/multi_hop_relay.rs:1058` (moved, Step 14).
- Hand-built preambles with a `pubkey` must sign (`preamble.sign(&id,
  now)`), or they are now refused: `crates/router/src/handshake.rs` tests
  (all; move to `handshake/tests.rs` first),
  `crates/router/src/route_handler/io/tests.rs:222` (oversized: same line
  count), `crates/substrate/tests/guest_http_e2e.rs:130-170`,
  `static_assets_e2e.rs:126`, `http_passthrough_e2e.rs:117`,
  `cert_renewal_e2e.rs:90-104` (it calls `verify_preamble` directly: sign
  with the instance key it already derives), and any other site the compiler
  or a failing test shows.
- `websocket_e2e.rs:154-175` and `miniapp_demo1_wasm_e2e.rs:180-197` each
  have the same `open_http_stream`, which sends a `pubkey` with no `sig` and
  would now be refused. Replace both with one signed helper,
  `open_http_stream(conn, service_id, Option<&Identity>)`, in a new
  `crates/substrate/tests/common/streams.rs` (declared in `common/mod.rs`),
  which builds the preamble with `RoutePreamble::new` and calls `sign` when
  an identity is given. Use the same helper in `guest_http_e2e.rs`,
  `static_assets_e2e.rs` and `http_passthrough_e2e.rs` where their helper
  has the same shape (the duplication gate).
- `crates/router/tests/native_dispatch_identity/helpers.rs:200`
  (`preamble_for`): a struct literal; use `RoutePreamble::new`. Its callers
  pass `caller` separately and need no signature.
- Checked, no change expected: `router/tests/native_dispatch_identity/
  {cross_service_fetch,resolve_relation}.rs` build `CallerContext` with
  `proof: None` and dispatch on one node, so no proof crosses a wire.
- `crates/substrate/tests/federated_fdae_e2e.rs`: the only real two-node
  test of the one production native call that carries a proof
  (`fdae_fetch.rs:109`). Alice connects with her own key and a `ucan`; node
  B's FDAE fetch calls `resolve-relation(principal = alice)` on node A.
  Before C4b, A saw alice's key as the caller; after C4b, A sees B's
  instance identity with `on_behalf` = alice, and `anchor_did` = alice makes
  the relation check pass. The test should pass unchanged, and it is the
  real-node evidence for SP-7's "`resolve-relation(principal = P)`
  succeeds". Update its comments that describe the old forwarding (around
  line 183, "alice's forwarded `resolve-relation` identity"), and add one
  assertion if a cheap one exists (for example the second query in the bad
  app path still fails for the reason it states).
- `RemoteHop::call` callers in tests: `MockHop` (Step 8.1).
- `remote_dispatch.rs` tests that assert "a native call with a proof
  presents the caller's key" now assert: `pubkey` is the node (or instance)
  key, and `on_behalf` equals the caller's proof. Rename such tests to match.
- `crates/sdk/src/tests.rs:175` (`build_preamble`): assert `ts` and `sig`
  are set and verify with `HandshakeVerifier::verify_preamble`.
- The two classifier unit tests (Step 2.3).
- `crates/substrate/tests/stream_client_e2e.rs` (unregistered protocol):
  today it checks "no hang, no bytes". After C4b the node may answer with a
  refusal; assert the refusal (or no bytes) explicitly so the test keeps its
  meaning.

### Step 14 — New tests

See §6 for the map to criteria. Files:

1. `crates/router/src/preamble/tests.rs` (moved) plus new cases:
   - `proof_bytes` fixed vector: for `service_id = "svc"`, `interface = "api"`,
     `ts = 1_700_000_000`, assert the exact hex of `proof_bytes`, and for
     `Identity::from_bytes(&[7u8; 32])` the exact hex of `sig`. Compute once,
     paste, never change (it pins the wire format).
   - round trip of every new field through `Display`/`parse`;
     `on_behalf` hex of JSON round trip; strict parse errors for bad `ts`,
     short `sig`, uppercase `sig`, `on_behalf` with an unknown field.
   - `redacted()` and `{:?}` output contain none of: the `sig` hex, the
     certificate hex or JSON, the `ucan` hex, the `on_behalf` hex (SP-9).
   - `route_text` cuts at `?` and at 256 characters.
2. `crates/router/src/handshake/tests/proof.rs` (SP-1, SP-3, SP-4), each with
   and without a certificate where it applies:
   - no `sig`; no `ts`; `sig` by another key; `sig` over another
     `service_id`; over another `interface`; a valid proof.
   - `ts = now - 901`, `now + 901` refused; `now - 899`, `now + 899`
     accepted (`verify_preamble_at` with a fixed `now`).
   - a forged signature refused *without* calling the resolver (a resolver
     that panics when called).
3. `crates/router/src/handshake/tests/on_behalf.rs`: valid P proof; P proof
   with a forged `sig`; with old `ts`; with a revoked P key; with an
   unreadable `delegation`.
4. `crates/router/tests/native_dispatch_identity/signed_preamble.rs` (new
   module of the existing `native_dispatch_identity` test binary: add its
   `#[path]` line in `crates/router/tests/native_dispatch_identity.rs`).
   Drive `RouteHandler::handle_stream` over `tokio::io::duplex`. Reuse
   `test_route_handler()` from `native_dispatch_identity/helpers.rs:135` and
   its `RecordingNativeService`-style double, registered under one
   `service_id`, recording the `CallerContext` it got. Cases:
   - Binary stream, `pubkey` without `sig` → one frame with code `-32097`;
     the service saw nothing (SP-1).
   - Binary stream for a `service_id` this node does not host → frame with
     `-32096`, nothing forwarded (`iroh_endpoint` is `None` in this harness,
     so a forward attempt would error instead) (SP-6).
   - The same on an `http://` preamble → bytes start with `HTTP/1.1 421 ` and
     carry `x-syneroym-refusal: not_hosted_here`; on `raw://` → the line
     `not_hosted_here\n`.
   - Hosted service, unknown interface → `-32601` with reason
     `unknown_interface`.
   - `ucan` without `pubkey` → refused (C4b-4).
   - `on_behalf` together with `ucan` → refused (SP-7).
   - valid immediate caller + valid `on_behalf` for P → the service sees
     `caller_did` = immediate, `session.anchor_did` = P's master DID (SP-7).
   - valid immediate caller + `on_behalf` with forged `sig` or old `ts` →
     refused (SP-7, C's own check).
   - A refused stream ends with `Ok(())` from `handle_stream` (no `error`
     log from `accept`), and the refusal is followed by end of stream.
   - SP-9 (node side): install a capturing `tracing_subscriber` with
     `tracing::subscriber::set_default` (the `#[tokio::test]` runtime is
     single-threaded, so the guard covers the handler; add
     `tracing-subscriber` to router `[dev-dependencies]` with
     `workspace = true`) at `trace` for `syneroym_router`; run a refused
     stream, an accepted stream with `delegation` + `ucan`, and one with
     `on_behalf`; assert the buffer contains none of the `sig` hex, the
     certificate hex or JSON, the `ucan` hex, the `on_behalf` hex. The
     forwarding log line is covered in item 8 (it runs only after a real
     resolve and connect).
5. `crates/router/src/route_handler/encryption.rs` has no tests module.
   Add `encryption/tests.rs`: a client P-256 key in `epk` completes the
   handshake (server key + signature come back, signature verifies with the
   node key); a preamble with `enc` but only `pubkey` fails with an error
   naming `epk` (SP-5).
6. `crates/router/src/proxy/tests/rehome.rs` (new; declare in
   `proxy/tests.rs`). `MockHop` outcomes plus a closure resolver that
   returns addresses from a list (SP-6):
   - not hosted, then success at a new address — non-idempotent call: two
     hop calls, `Ok`;
   - the same with `idempotent: true`;
   - the resolver returns the same endpoint id → `ServiceNotFound`, one hop
     call;
   - not hosted twice → `ServiceNotFound`, two hop calls;
   - a transport error is not re-homed (unchanged retry loop behaviour).
7. `crates/router/src/proxy/tests/remote_dispatch.rs` (or a new
   `on_behalf.rs` there if the file would pass 800 lines):
   - native call with `caller.proof = Some(P)` and `service_id: None` →
     preamble `pubkey` = node key, `on_behalf` = P's proof, `sig` verifies;
   - native call with `service_id: Some(s)` and a certificate → instance key
     and certificate, `on_behalf` = P;
   - caller proof that itself carries `on_behalf = Q` → passes Q, unchanged;
   - native call with an unsigned local proof (`signed: None`, as C5 and C6
     build for a same-node caller) → no `on_behalf`, node identity only;
   - guest call with `caller.proof = Some(P)` → `on_behalf` is `None`
     (SP-7 "a guest cannot send `on_behalf`");
   - two attempts of a retried idempotent call carry their own `ts` (sign per
     attempt): use a `MockHop` that records every preamble.
8. `crates/coordinator_iroh/tests/signed_preamble_nodes.rs` (new, real Iroh
   nodes in one process). First move, unchanged, the node helpers it needs
   out of `multi_hop_relay.rs` into
   `crates/coordinator_iroh/tests/common/nodes.rs` (declared in
   `common/mod.rs`): `build_test_route_handler_deps`, `wait_for_local_addr`,
   `create_signed_info_with_full_addr`, `spawn_global_coordinator`,
   `spawn_z_target_substrate`, and the callee/caller node spawners with
   `CapturingNativeService`. Move
   `test_cross_node_native_capability_identity_forwarding` here too.
   `multi_hop_relay.rs` shrinks: lower its line count in
   `xtask/oversized-test-files.txt` (or remove the entry if it is now under
   800).
   - `CapturingNativeService` also records `session.anchor_did`.
   - The moved test now builds P's proof with
     `RoutePreamble::sign` + `caller_proof()` and asserts the callee sees
     `caller_did` = caller node DID and `anchor_did` = P's master DID.
   - New chain test, P → Sa → Sb → Sc, three nodes and **two** `on_behalf`
     wire hops. Sa is the node P called: its proxy is invoked with a
     `CallerContext` whose `proof` is P's signed proof (what Sa's router
     built from P's preamble). Hop 1, Sa → Sb: Sa presents its own node key
     and `on_behalf` = P (from `proof.to_pass_on()`, P's own proof). Sb
     registers a test native service on interface `chain` that, on
     dispatch, calls Sc's `data-layer` through Sb's own `ServiceProxy` with
     `caller: invocation.caller.clone()` and `origin: CallOrigin::Native {
     service_id: None }` (proxy handle set after `RouteHandler::init`
     through a `OnceLock<Weak<dyn ServiceProxy>>`, the same two-phase wiring
     the control plane uses). Hop 2, Sb → Sc: Sb received P's proof *over the
     wire* in `on_behalf`, so its `CallerProof` is Sa's proof with
     `on_behalf: Some(P)`, and `to_pass_on()` returns that P value
     unchanged. Assert at Sc: `caller_did` = Sb's DID (not Sa's) and
     `anchor_did` = P's master DID. Also assert at Sb (record in the `chain`
     service): `caller_did` = Sa's DID, `anchor_did` = P.
   - Same chain with P's proof forged, and with `ts = now - 16 min`: Sb (the
     first node that receives the bad `on_behalf`) refuses, the call fails
     with `Callee { code: -32097 }`, and Sc recorded nothing. Sc's own check
     of a bad `on_behalf` is item 4.
   - Large request (C4b-18): call `IrohHop::call` (public) directly from
     Sa's endpoint to Sb's address, for a `service_id` Sb does not host, with
     `params` holding 4 MiB (under `MAX_FRAME_SIZE`, above the stream
     window). Assert `Err(ProxyError::Callee { code: -32096, .. })`, not
     `Transport`. The re-home that follows is covered by the `MockHop` tests
     in item 6.
   - SP-9 (forwarding line): registry R, coordinator C
     (`spawn_global_coordinator`), target Sz. A raw stream to C carries a
     signed preamble for a service on Sz with `delegation`, `ucan` and
     `on_behalf`, so C runs the "Forwarding preamble" line. Capture logs with
     a global subscriber (`set_global_default` once per binary, behind a
     `OnceLock`; C runs on other tasks) writing to a shared buffer at
     `trace` for `syneroym_router`; assert none of the `sig`, certificate,
     `ucan` or `on_behalf` values appear, and that the redacted line did
     appear (so the test cannot pass by logging nothing).
9. `crates/substrate/tests/signed_preamble_e2e.rs` (new, real nodes via
   `SubstrateNode`):
   - SP-2: a node with `[roles.mailbox]` (reuse the mailbox e2e setup; move
     a helper into `tests/common/` if needed rather than copying). A real
     instance key + certificate. An attacker writes, on a raw Iroh stream
     (`client.connection()` → `conn.open_bi()`), a preamble with the
     victim's `pubkey` and certificate, a fresh `ts`, and a `sig` made by the
     attacker's own key, then one request frame, for each of `register`,
     `unregister`, `deposit`, `claim`, `ack`, `status`. Each answer is a
     frame with code `-32097`. A control: the real key holder's `info` (or
     `status`) is not refused by the router.
   - SP-6 (SDK): nodes N1 and N2 on one registry. A client
     `SyneroymClient::new(n2_did, registry_url)` connected by hand to N1's
     mechanisms (`connect_with_mechanisms`, mechanisms from a lookup of N1's
     record) calls `orchestrator`/`readyz`: it succeeds (it re-homed to N2).
     A client made with `new_with_mechanisms(n2_did, n1_mechanisms)` (cannot
     re-home) gets an error whose `JsonRpcError.code` is `-32096`, for a
     small request and for one whose `params` hold 4 MiB (C4b-18).
   - Concurrent re-home (C4b-10): one client stuck on N1 sends 8 requests at
     once (`join_all`); all succeed, and the client ends with one live
     connection (to N2).
   - SP-6 (gateway, C4b-11): N1 with a client gateway; a service S deployed
     on N2. Prime the gateway's cached client for S with a connection to N1
     (hard to do from outside; if no clean way exists, test
     `passthrough_with_conn` directly in `crates/sdk` against a local node
     that does not host the target, asserting `NotHostedHere { replayable:
     true }`, and test the gateway's retry branch with a unit test).
10. `crates/substrate/tests/revoke_key_e2e.rs` (from PR 1): assert the
    refused call's `JsonRpcError.code` is `-32097` (SP-8).
11. `apps/roymctl` / SDK: nothing more.

### Step 15 — Living docs (this PR)

The change doc stays `status: approved` until C4c closes it. Update what is
now false:

- `docs/system-architecture.md`:
  - FND-IAM "Stream identity" (line ~553) and the "What the router checks"
    list (~796-802): the signature, `ts` window, anonymous rule (C4b-4),
    `on_behalf` and `anchor_did`. Remove the part of the **Envisioned**
    block that this slice builds (proof of possession); keep the part C4c
    builds (identity hidden from forwarders).
  - Encryption section: "One field, two uses" (~1626) is no longer true;
    say the handshake key is `epk`. "What the router checks about a caller"
    (~1649-1651): same as FND-IAM.
  - Client gateway paragraph (~689): the gateway signs each stream with its
    node key; on `not_hosted_here` it looks the service up again (C4b-11).
  - Coordinator and multi-hop text (~324, ~328, ~1818-1853): only
    coordinators forward; a forwarder checks nothing; any other node answers
    `not_hosted_here`; what a forwarder still sees (the identity, in plain
    text, until C4c).
  - The routing paragraph (~549): the separator is `|` (§3 item 1).
- `docs/system-requirements-spec.md` [NET-PRM] (lines ~795-805): the
  grammar separator `|`; `enc` with `epk` (not `pubkey`); a new bullet for
  proof of possession (`pubkey` needs `ts` and `sig` over `service_id`,
  `interface` and `ts`, ±15 minutes); `on_behalf`; the readable refusals
  and who forwards.
- `docs/planning/traceability-matrix.md:85` ([NET-PRM] row): add the
  signature check and `on_behalf` to the description and name the new test
  files as evidence.
- The module doc of `crates/router/src/preamble.rs` (Step 3.10).
- `docs/developer-guide.md`, the exported metrics list (around line 1753):
  add `substrate.router.refused`, a counter of streams the router refused
  before dispatch, labelled by `kind` (`unauthenticated`, `not_hosted_here`,
  `unknown_interface`). Put it in its own sentence, not under the mailbox
  role, because every node exports it.
- `docs/developer-guide.md`: search for `pubkey=`; if it lists preamble
  parameters, add `ts`, `sig`, `epk`, `on_behalf`.
- `docs/TERMINOLOGY.md`: add **Forwarder** and **On-behalf proof** if they
  are not there.
- Change doc: tick the Part 1 task boxes; add `## Deviations` entries
  `D-C4b-4` onward for each decision C4b-1 to C4b-13 and C4b-17 to C4b-19
  that the
  change doc did not already state, and for anything that changed while you
  built. Fill `living-docs-touched` with the files above (it stays open for
  C4c to add its own).

### Step 16 — Deferred backlog (this PR)

In `docs/planning/deferred-backlog.md`:

- Move to "Recently resolved": "Router does not check that the caller holds
  the temporary key" (§3).
- Edit "End-to-end handshake hardening": the `pubkey` part is done (`epk`);
  the rest stays for C4c.
- Remove the marker row `crates/sdk/src/lib.rs:60` (the `TODO(FDAE)`
  self-asserted pubkey; its real place was `crates/sdk/src/client.rs:82`).
- Add rows (§3 Access control, unless noted):
  - *Any service that P called in the last 15 minutes can act on behalf of P
    through `on_behalf`* (accepted gap; ADR-0016 A2 trust rule). TBD.
  - *A Rust call through a forwarder still carries its identity in plain
    text* — target M06D C4c.
  - *A sender that ignores Part 2 on an indirect connection exposes its own
    identity* (accepted). TBD.
  - *No replay protection inside the 15-minute window* (rejected option,
    recorded so it is not re-proposed without the reason). TBD.
  - *An `on_behalf` proof older than 15 minutes refuses the whole stream*
    (long-lived streams, §3 item 12). TBD.
  - *On a stream with `enc`, a refusal arrives in plain text before the
    handshake, and the browser reads it as a failed handshake* (§7 C4c
    note). Target M06D C4c.
  - *Gateway passthrough cannot replay a request after it has sent body
    bytes* (§7 Gateway). TBD.
  - *`EndpointRegistry::hosts_service` scans all endpoints per stream* (§7,
    perf, only if it shows in a profile). TBD.
- Keep "A person with a session cookie gets no `on_behalf`" as is.

## 6. Test list (summary)

| Criterion | Where |
| --- | --- |
| SP-1 | `handshake/tests/proof.rs`; `router/tests/native_dispatch_identity/signed_preamble.rs` (refusal frame) |
| SP-2 | `substrate/tests/signed_preamble_e2e.rs` (six mailbox methods) |
| SP-3 | `handshake/tests/proof.rs` (±899 accepted, ±901 refused) |
| SP-4 | `handshake/tests/proof.rs` (other `service_id`, other `interface`) |
| SP-5 | `route_handler/encryption/tests.rs`; Playwright `webrtc.spec.ts`, `multi-hop.spec.ts` |
| SP-6 | `router/tests/native_dispatch_identity/signed_preamble.rs` (refusal, no forward, three framings); `proxy/tests/rehome.rs` (idempotent and not); `coordinator_iroh/tests/signed_preamble_nodes.rs` (4 MiB request still gets `-32096`); `substrate/tests/signed_preamble_e2e.rs` (SDK, large request, concurrent re-home, gateway) |
| SP-7 | `router/tests/native_dispatch_identity/signed_preamble.rs` (anchor, both fields, C's own check); `proxy/tests/remote_dispatch.rs` (guest never sends, pass on unchanged, unsigned local proof); `coordinator_iroh/tests/signed_preamble_nodes.rs` (P → Sa → Sb → Sc, two wire hops, forged, old); `substrate/tests/federated_fdae_e2e.rs` (real `resolve-relation(principal = P)` succeeds) |
| SP-8 | `substrate/tests/revoke_key_e2e.rs` (PR 1, tightened in PR 2) |
| SP-9 | `preamble/tests.rs` (redacted views); `router/tests/native_dispatch_identity/signed_preamble.rs` (node logs); `coordinator_iroh/tests/signed_preamble_nodes.rs` (the forwarding line) |

## 7. Forward notes for later slices (add to task.md in the close-out)

- **C4c.** `verify_caller` runs before the encryption stage today. C4c must
  move the identity read after `apply_encryption_stage` for a stream with
  `enc`, read the identity line there, and refuse a stream with `enc` whose
  plaintext preamble has identity fields. The signed bytes and
  `verify_presented` stay as they are; C4c only changes where the fields come
  from. `OutboundIdentity::apply` is the one place a Rust client sets
  identity fields; on an indirect connection C4c sends the same fields in the
  encrypted line instead.
  A refusal on a stream with `enc` (`not_hosted_here`, `unknown_interface`)
  is written in plain text *before* any handshake, because it is decided
  before the encryption stage. A client that expects the server's handshake
  bytes first must tell the two apart. They differ in the first byte: the
  server's P-256 key is SEC1 uncompressed and starts with `0x04`; a binary
  refusal frame starts with a big-endian length (`0x00`), an HTTP refusal
  with `H`, a raw one with a letter. C4c's Rust client and its
  `peer-proxy.js` change must check this byte and read the refusal in the
  stream's framing. Until then the browser fails the handshake on such a
  stream (no wrong data is accepted).
- **C5.** Classify the new codes: `NOT_HOSTED_HERE_RPC_CODE` (`-32096`)
  normally never reaches C5, because `invoke_rehoming` turns it into
  `ServiceNotFound`; `STREAM_UNAUTHENTICATED_RPC_CODE` (`-32097`) is
  "may succeed later" (anchor lookup timeout, clock skew, a renewed
  certificate). Both outboxes already retry them (C4b-19); C5's mailbox
  decision must treat them the same way, not as a final refusal.
  A same-node sender's `CallerProof` (instance key and certificate) has
  `signed: None`; that is correct, and `to_pass_on()` keeps it off the wire.
- **C5.** `invoke_remote_at` already takes the identity as an
  `OutboundIdentity` chosen once per call and signed per attempt by
  `IrohHop`. The C5 `outbound_preamble` split should build on
  `outbound_identity`, not replace it. `invoke_rehoming` is where a
  mailbox-aware retry would sit.
- **C6.** A receiver on the mailbox host presents its instance proof in a
  `CallerProof` it builds itself: fill `pubkey_hex` and `delegation_json`,
  leave `signed: None` and `on_behalf: None` (`..Default::default()`). A
  same-node call has no stream, so there is nothing to sign, and an unsigned
  proof is never passed on in `on_behalf`. A remote receiver's calls are
  signed by `IrohHop`.
- **C7.** The C4-4 risk (a copied certificate acts as a service) is closed
  by this slice.

## 8. Completion steps

For each PR:

1. Import clean-up pass over every edited file (types by `use`, functions
   through their parent module, no inline multi-`::` paths).
2. Comment pass: no planning ids (`C4b`, `SP-n`, `D-C4b-n`), no line counts
   or size notes, simple English.
3. `cargo +nightly-2026-04-06 fmt --all`, then run `mise run verify` in the
   background and read the summary; all gates pass, no `--skip`. In a git
   worktree use `cargo xtask verify` with `CARGO_TARGET_DIR` (see the
   worktree note in the project memory).
4. Ratchets: oversized-file entries lowered where a file shrank;
   `MAX_EXACT_DUPLICATE_PERCENT` lowered if the duplication gate says so.
5. Deferred backlog and change-doc `## Deviations` updated (Steps 1.4, 15,
   16).
6. `status.md`: the C4b row with evidence (test files and counts, verify
   result), implemented by / reviewed by.
7. PR body includes the CLA line from `AGENTS.md` verbatim.

## 9. Review changes (2026-10-19)

A review of the first version raised 15 points. Each is answered here, so
a later reader can see why the plan has its current shape.

| # | Point | Answer |
| --- | --- | --- |
| 1 | A refusal is lost on a request larger than the flow-control window: the client's write fails first. | Incorporated: C4b-18 (the client reads the answer after a failed write) and the router shuts its write half down after a refusal (C4b-1). Rejected the router-side discard, with the reason in C4b-18. Tests: 4 MiB cases in Step 14 items 8 and 9. |
| 2 | `-32097` becomes `Terminal` for queued calls; C5 must classify the codes. | Incorporated: C4b-19, Step 2.3, §7 C5 note. |
| 3 | A production `CallerProof` without a signature (C5, C6) turns into an `on_behalf` every destination refuses. | Incorporated: the signed part is optional (`signed: Option<PreambleSignature>`), `to_pass_on()` returns `Option` (C4b-6, Step 2.2); §7 C6 note says what to fill in; test in Step 14 item 7. |
| 4 | A refusal on an `enc` stream is plain text and comes before the handshake. | Incorporated as a C4c forward note (§7) with the first-byte rule, and a backlog row. Not changed in C4b: the refusal is decided before the encryption stage, and C4c already rewrites both client handshakes. |
| 5 | The SP-7 chain is one hop short; `resolve-relation` has no test; `federated_fdae_e2e.rs` and two router tests are not named. | Partly pushed back: the planned chain already had two `on_behalf` wire hops (Sa → Sb, Sb → Sc), and Sb is the node that receives `on_behalf` over the wire and passes it on. The text did not make this clear; Step 14 item 8 now spells out each hop and asserts at both Sb and Sc. Incorporated: `federated_fdae_e2e.rs` is named as the real `resolve-relation(principal = P)` evidence (Step 13, §6); the two router tests were checked and need no change (Step 13). |
| 6 | The SP-9 test never runs the "Forwarding preamble" line. | Incorporated: the forwarding case moved to a real coordinator with a resolvable target (Step 14 item 8), and it asserts that the redacted line did appear. |
| 7 | The revoke tests depend on the real DHT; the unit test must assert the error text. | Incorporated: `revoke_key` takes a `RegistryClient` and `sync_dht`; tests use a DHT-less client; the unit test asserts the text (Steps 1.2, 1.3). |
| 8 | `websocket_e2e.rs` and `miniapp_demo1_wasm_e2e.rs` break and share a duplicate helper. | Incorporated: one signed `open_http_stream` in `crates/substrate/tests/common/streams.rs` (Step 13). |
| 9 | Concurrent re-homes in the SDK close each other's connection. | Incorporated: a re-home lock and compare-before-replace by `stable_id` (C4b-10, Step 9.5); concurrency test in Step 14 item 9. |
| 10 | Strict parse runs before the forward decision, so a coordinator drops a malformed stream. | Pushed back, recorded as a deviation (C4b-5): rule 3 protects valid streams from the forwarder's clock and anchor lookups, not malformed ones; a forwarder must parse what it writes again; it already drops an unparseable preamble today. Incorporated the logging part: a parse error is logged at `debug` and returns `Ok(())`. |
| 11 | Each refused caller is logged twice, once at `error`. | Incorporated: refusals return `Ok(())`, are logged once at `debug`, and are counted in `substrate.router.refused` (C4b-1, C4b-13, Step 7). |
| 12 | The requirements spec [NET-PRM] and the traceability matrix are missing from Step 15. | Incorporated (Step 15). |
| 13 | Two more stale comments. | Incorporated (§3 item 9). |
| 14 | Derived `Debug` on the proof types prints signatures and certificates. | Incorporated: hand-written `Debug` (C4b-17, Step 2.2, Step 3.7). |
| 15 | `blobs.rs` needs no change; `native_dispatch_identity/helpers.rs` already builds a `RouteHandler`. | Incorporated: Step 3.2 corrected; the router stream tests now live in the `native_dispatch_identity` test binary and reuse `test_route_handler()` (Step 14 item 4). |

Second review (2026-10-19), three small points, all incorporated:

| # | Point | Answer |
| --- | --- | --- |
| 1 | C4b-19 said the new end matches the old `Transport` path; for the conversation outbox the old result was `Unreachable`, not `Retry`. | Fixed the sentence; C4b-19 now records the change for the conversation outbox (it now uses the attempt budget and can dead-letter) and asks for a `## Deviations` entry. |
| 2 | `router.streams.refused` breaks the `substrate.` metric prefix; the developer guide lists metrics. | Renamed to `substrate.router.refused{kind}` (label `kind`, like `substrate.mailbox.deposits_refused`); Step 15 adds it to the developer guide. |
| 3 | An unauthenticated refusal was logged twice at `debug`. | `write_refusal` now only writes and counts; the code that decides each refusal logs it once (C4b-13, Steps 6 and 7). |

Third review (2026-10-19): C4b-19 said a refused conversation message
"used to wait without end". It waited up to `max_pending_age_secs` (30 days)
and then failed as "recipient never became reachable". Sentence corrected;
the `Retry` decision is unchanged.

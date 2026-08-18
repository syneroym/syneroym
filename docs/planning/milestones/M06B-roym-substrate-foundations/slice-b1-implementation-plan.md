# M06B Slice B1 — Person Identity at the Client Gateway: Implementation Plan

> **Scope.** Gap **G3** from
> [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md#g3--person-identity-at-the-client-gateway),
> slice **B1** of [task.md](task.md). A local session model that binds an
> authenticated local client to a **person's** identity, so the gateway
> presents that person's DID under an owner→node delegation instead of the
> node's own. The bind stays `127.0.0.1` (D-06B-5).
>
> **Status.** Plan only. Nothing implemented. Written against the tree at
> 2026-08-18 (`main`, `1347e02`).
>
> Read §9 first if you are reviewing: it lists what the input documents leave
> ambiguous or state inaccurately, and the one hard cost neither document
> mentions (F7).

---

## §0 Review response (revision 2)

Two reviews, 15 substantive items. Every checkable claim was re-verified
against the tree; all of them held. Disposition below; the sections named
carry the change.

| # | Item | Disposition |
|---|---|---|
| R1-1 | B1 delivers *route as this person*, not *sign as this person*; backlog row 179 asks for the second | **Accepted.** New **D-B1-12** states the boundary and hands B4 the open question. §8's row edits narrowed |
| R1-2 | `/_syneroym/` is reserved at the gateway only; a deployed app can still declare a route there, reachable over iroh/WebRTC | **Accepted.** New **D-B1-13**: deploy-time refusal in `validate_route` **and** on the normalized asset key (§3.9) |
| R1-3 | The WebRTC peer-proxy builds its own preamble in the page; B1 does nothing for that browser path | **Accepted.** New **F13**; §9.8 names which browser path M06C is expected to use; backlog row owed |
| R1-4 | `http.wit`'s `self-asserted` doc comment becomes false | **Accepted.** §3.10; moved out of §4's "deliberately unchanged" with the enum/comment split made explicit |
| R1-5 | `Set-Cookie` is host-only, and every app is its own first DNS label | **Accepted.** §9.7 states it; D-B1-3's browser argument now rests on it explicitly rather than implicitly |
| R1-6 | After login *every* connection carries a delegation, so a brief registry outage blanks a page that worked before login | **Accepted, and it changes the design.** New **D-B1-14**: the gateway resolves the anchor once at login and refuses the login loudly, instead of letting every later request fail. Two new tests |
| R1-7 | The "second local process" guarantee is OS-user-level, not gateway-level | **Accepted.** New **D-B1-15** states the limit against exit criterion 3. Verified: `--dir` defaults to `.` and `Identity::save_to_path` is `0600` on unix only |
| R1-8 | Test 23 (restart) has no mechanism, and the harness registry is in-memory | **Accepted, resolved by removal.** Confirmed: restart is a per-file `Node::boot`/`teardown` pattern with a pinned tempdir (`durable_outbox_e2e.rs:616`), and that node's in-memory registry loses the master anchor on restart — so the test would have to republish it too. ~80 lines to pin a property that is structural. Dropped, with §7's note saying why |
| R1-9 | `Expect: 100-continue` hangs the body loop for `curl` | **Accepted.** §5.7 answers it before reading the body |
| R1-10 | D-B1-10 pins the resolve path to `grant_resolve_to_node_did`, the gate B2 changes | **Accepted.** Recorded in §9.9 as a decision B2 must re-open rather than inherit |
| R1-s1 | Backlog rows 179/180 are in §7 *Gateway & networking*, not §3 | **Accepted**, verified (§7 starts at line 174). Row 52 is in §1, as written. task.md carries the same error |
| R1-s2 | §8's "See §9.5" is a wrong pointer | **Accepted**, removed |
| R1-s3 | Marking *Gateway caller = substrate-owner DID threading* resolved is an overclaim | **Accepted** — "resolved for the session path" |
| R1-s4 | `write_json_rpc_error` is at `:369`, not `:349` | **Accepted**, verified and corrected |
| R2-1 | `serde` missing from `client_gateway/Cargo.toml` | **Accepted**, verified (only `serde_json` is there today) |
| R2-2 | `test_state` (`gateway.rs:421`) builds `GatewayState` literally | **Accepted**, verified; added to §4 |
| R2-3 | `gateway_hostname_e2e.rs:190` builds `ClientGatewayRole` with no `..Default::default()` | **Accepted**, verified — and it is the **only** such site: the other 20 construction sites all spread `..Default::default()` |
| R2-4 | `refresh_anchor_or_warn` takes `Option<&str>` | **Accepted**, `registry_url.as_deref()` in §5.9 |
| R2-5 | `assertion_value`'s exact JSON is unspecified, and both sides must produce identical bytes | **Accepted.** §3.3 now fixes the key names and the `typ` string |

---

## §0a Review response (revision 3)

Four items, all self-inflicted by revision 2. All four verified and accepted;
two were compile-or-test breaks and are fixed before P1.

| # | Item | Disposition |
|---|---|---|
| R3-1 | D-B1-13's asset half names the wrong file **and** could never fire: `reject_archive_entry_path` is at [deploy_docs.rs:162](../../../../crates/core/src/deploy_docs.rs#L162) in `syneroym-core`, and it rejects any `RootDir` component, so an accepted entry never starts with `/` | **Accepted**, and the tree makes the fix cleaner than proposed: `unpack_asset_bundle` **already** rejects a reserved prefix on the *normalized* key, two lines below (`RESERVED_BLOBS_PREFIX`, [assets.rs:177](../../../../crates/control_plane/src/assets.rs#L177)). The new check goes beside it, as one more arm of an existing pattern. §3.9 and §4 row 20 rewritten |
| R3-2 | `login`'s `&RegistryClient` contradicts test 8a's stub, and breaks the successful-login unit tests; `MasterAnchorResolver` lives in `syneroym-router`, which `client_gateway` does not depend on | **Accepted.** Verified: the trait appears nowhere outside `crates/router`. **Moving it to `syneroym_core::dht_registry`**, beside `RegistryClient` — its only implementor, already in `core` — with a `pub use` from `handshake.rs` so no router call site changes. §3.3, §3.4, §3.11 |
| R3-3 | §0's R1-2/R1-4 pointers are off by one | **Accepted**, and a full pass over every internal `§` reference was done — see *Internal reference audit* below |
| R3-4 | §4 row 13 lists only `hex, rand`; §3.6 adds `serde` | **Accepted** |

### Internal reference audit

Every `§` cross-reference in this document was re-checked against the current
headings after revision 2's insertions, and again after revision 3 inserted
§3.11 and pushed the fixture section to §3.12. Corrections made: R1-2
`§3.10`→**§3.9**; R1-4 `§3.11`→**§3.10**; every reference to the fixture
section →**§3.12**; R3-2's own `§3.12`→**§3.11**. All `§9.x` pointers verified
correct (7 = cookie scope, 8 = browser paths, 9 = the B2 gate, 5 = the master
anchor). All `§4`, `§5.x`, `§6`, `§7`, `§8`, and `§10` pointers verified
correct. Every `§3.x` reference now resolves to an existing heading (3.1-3.12,
no gaps).

---

## §0b Review response (revision 4)

Two non-blocking notes. Both verified, both accepted — one corrects an
overstated sentence, one adds a fact the CLI author needs.

| # | Item | Disposition |
|---|---|---|
| R4-1 | §3.9's rationale claims `reject_archive_entry_path` is called by volumes and other artifact kinds; it has one production caller | **Accepted**, verified: the only production call is [assets.rs:164](../../../../crates/control_plane/src/assets.rs#L164); volumes use the sibling `reject_relative_escape` ([sandbox_podman/engine.rs:114](../../../../crates/sandbox_podman/src/engine.rs#L114)). The placement is unchanged — it rested on the check being unable to fire in that helper at all, which is untouched — but the supporting sentence is rewritten to say what is true |
| R4-2 | §5.2 consumes the nonce before the anchor lookup, so `AnchorUnresolvable` burns the challenge | **Accepted as correct-by-design, and now stated.** Single-use-before-verify is what makes a captured signature unreplayable, so the order stays. §5.2 and §5.9 now say so explicitly, and §5.9's error path fetches a **fresh challenge** rather than re-POSTing |

---

## §1 Findings from reading the tree

### F1 — the router already accepts exactly the credential B1 needs; nothing in the router changes

`HandshakeVerifier::verify_preamble`
([handshake.rs:42-99](../../../../crates/router/src/handshake.rs#L42)) already
does the whole job when `preamble.delegation` is `Some`:

- `cert.verify(&cert.master_did, &TRANSPORT_SCOPES)` — signature, scope,
  window;
- `cert.temporary_did == derive_did_key(preamble.pubkey)` — binds the
  certificate to the key on this connection;
- master-anchor revocation lookup;
- returns `VerifiedIdentity { master_did, temporary_did }`.

`build_caller` ([io.rs:158-261](../../../../crates/router/src/route_handler/io.rs#L158))
sets `CallerContext.caller_did = id.master_did`. So **putting an
owner→node `DelegationCertificate` on the gateway's outbound preamble is
sufficient** to make every downstream consumer see the person's DID. No
router, no `CallerContext`, no WIT change.

`SCOPE_ROUTING` (`"routing"`, [delegation.rs:13](../../../../crates/identity/src/delegation.rs#L13))
is already in `TRANSPORT_SCOPES` and is documented as covering exactly this
case — *"an operator's device key, a client session key"*.

### F2 — the `caller-auth` label flips for free

`guest_caller_identity` ([http.rs:487-510](../../../../crates/router/src/route_handler/http.rs#L487))
picks `GuestCallerAuth::Delegated` from `preamble.delegation.is_some()`.
Attaching the certificate flips the guest-visible label from `self-asserted`
to `delegated` with no code change on that path. See §9.1: the enum has no
`verified` value, which is what task.md calls it.

### F3 — the gateway is a byte passthrough, and the preamble is written once per TCP connection

`handle_connection` ([gateway.rs:216-341](../../../../crates/client_gateway/src/gateway.rs#L216))
parses only the **first** request's headers on a socket, then hands the whole
socket to `SyneroymClient::passthrough_with_conn`, which writes one preamble
and then `io::copy_bidirectional`s for the connection's lifetime. Two
consequences B1 inherits rather than creates:

- the session, like `X-Syneroym-Routing-Key` ([gateway.rs:249-262](../../../../crates/client_gateway/src/gateway.rs#L249)),
  is a **per-connection** decision read from the first request;
- later requests an HTTP keep-alive reuses the connection for are copied
  byte-for-byte, unparsed. The gateway cannot strip or re-read anything on
  them.

`passthrough_with_conn` ([sdk/lib.rs:1050-1089](../../../../crates/sdk/src/lib.rs#L1050))
hardcodes `delegation: None`. Its only caller in the tree is
[gateway.rs:314](../../../../crates/client_gateway/src/gateway.rs#L314); the
sibling `passthrough` ([sdk/lib.rs:1031](../../../../crates/sdk/src/lib.rs#L1031))
has **no** callers.

### F4 — the credential the browser sends is forwarded to the target service today

Because the head is forwarded verbatim (`&buf[..bytes_read]`), any `Cookie:`
or `Authorization:` header on the first request reaches the deployed service —
which since M06A A2 can be **guest code**. A session token that a deployed app
can read is a token that app can replay to act as the person. B1 must remove
its own credential from the forwarded bytes (D-B1-7).

### F5 — the gateway already holds everything it needs to be its own endpoint

`GatewayState` ([gateway.rs:79-95](../../../../crates/client_gateway/src/gateway.rs#L79))
holds the node `Identity`. `parse_target_host`
([protocol_utils.rs:173-200](../../../../crates/core/src/protocol_utils.rs#L173))
returns `None` for any first label without a well-formed `-s<hash>` segment, so
`Host: localhost:7960` is already a 400 today — a login client needs no service
hostname. The gateway's only response writer is `write_json_rpc_error`
([gateway.rs:369-380](../../../../crates/client_gateway/src/gateway.rs#L369)); a
plain JSON writer has to be added.

### F6 — the minting side of the credential already exists in `roymctl`

`roymctl identity delegate --master <name> --temp-did <node_did> --scope
routing --expires-days N` ([identity.rs:182-207](../../../../apps/roymctl/src/commands/identity.rs#L182))
already mints exactly the owner→node certificate. `Identity::sign_json`
([keys.rs:201](../../../../crates/identity/src/keys.rs#L201)) and
`substrate::verify_json_signature` ([substrate.rs:203](../../../../crates/identity/src/substrate.rs#L203))
are a matched RFC-8785 sign/verify pair, so **no new crypto dependency** is
needed on either side.

### F7 — presenting a delegation makes every gateway stream resolve the person's master anchor (the hidden cost)

`verify_preamble` resolves the master anchor **from the community registry or
the DHT, per stream, with no cache**, and a miss is an `Err`
([handshake.rs:83-93](../../../../crates/router/src/handshake.rs#L83)) —
which `handle_stream` turns into a **hard reject** because
`preamble.delegation.is_some()` ([io.rs:346-355](../../../../crates/router/src/route_handler/io.rs#L346)).
`RegistryClient::resolve_master_anchor` errors when neither the HTTP registry
nor the DHT has the record ([dht_registry.rs:507-511](../../../../crates/core/src/dht_registry.rs#L507)).

So a person whose master anchor is not published **cannot open a working
session at all**, and every session-bearing TCP connection costs one registry
round trip on the destination node. Neither task.md nor the spec mentions
this. It is the single most likely thing to surprise the implementer. The
precedent for the requirement already exists (D-A1-7: `refresh_anchor_or_warn`,
[member_identity.rs:125-145](../../../../apps/roymctl/src/commands/member_identity.rs#L125)),
and B1 follows it (D-B1-9); the per-stream cost is recorded as backlog, not
fixed here (§10).

### F8 — the topology-resolution path must keep using the node identity

`AppHostResolver`'s Tier-2 fetcher is built with the **node** identity
([gateway.rs:126-140](../../../../crates/client_gateway/src/gateway.rs#L126))
and depends on `[iam].grant_resolve_to_node_did` matching the node DID
([io.rs:204-210](../../../../crates/router/src/route_handler/io.rs#L204)).
It is a different client from the passthrough. **Do not switch it to the
person's identity** — the same-node resolve grant would stop matching and
every app-scoped (`-a…-s…`) hostname would start failing.

### F9 — an existing e2e pins today's behaviour, and it must keep passing

`test_through_the_gateway_a_non_public_route_is_reached_and_reports_self_asserted_node_did`
([guest_http_e2e.rs:323-397](../../../../crates/substrate/tests/guest_http_e2e.rs#L323))
asserts `self-asserted:<node_did>` for a gateway request with no credential.
B1 keeps the no-session path exactly as it is, so this test is unchanged. It
becomes the negative half of failure-matrix row 1.

### F10 — the guest fixture already reports what B1 needs to assert

`test-components/http-guest-test`'s `/whoami` answers `"{auth}:{did}"` and
already spells `delegated` ([lib.rs:43,102-108](../../../../test-components/http-guest-test/src/lib.rs#L43)).
No fixture change is needed for the identity assertions. `/echo`'s
`describe_request` reports `header_count` but **not** header names
([lib.rs:52-71](../../../../test-components/http-guest-test/src/lib.rs#L52)),
so proving F4's strip needs one additive field there.

### F11 — dependency and config state

- `crates/client_gateway` depends on `core`, `sdk`, `rpc`, `identity`,
  `anyhow`, `tokio`, `tracing`, `serde_json`, `dashmap`, `httparse`. It needs
  `hex` and `rand` added (both already workspace deps).
- `ClientGatewayRole` ([config.rs:1001-1020](../../../../crates/core/src/config.rs#L1001))
  has `http_port` and `resolve_ucan` only.
- `roymctl` already has `reqwest` with `json`, `hex`, and `syneroym-identity`.

### F12 — an owner session at the gateway becomes a node-admin credential

Today the developer guide records that gateway `curl` examples are denied
because the gateway presents the node DID
([developer-guide.md:254-265](../../../../docs/developer-guide.md#L254)). Once
the **controller** can open a session, `orchestrator`/`security` over
`http://localhost:7960` succeed for that session. That is the intended
outcome — it closes the *Gateway caller = substrate-owner DID threading*
backlog row — but it means the token is a privileged local credential and must
be treated as one (D-B1-6, D-B1-7, D-B1-8).

### F13 — the WebRTC browser path builds its own preamble and B1 does not reach it

`crates/coordinator_webrtc/templates/peer-proxy.js` composes a preamble **in
the page** — `http://<interface>|<service_id>?enc=ecdh-p256&pubkey=…` at
[:587](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L587) and
[:944](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L944),
with the `pubkey` placeholder replaced by a per-session ECDH key at
[:438](../../../../crates/coordinator_webrtc/templates/peer-proxy.js#L438) —
and never sets `delegation`. A browser reaching an app that way does not pass
through the client gateway at all, so **B1 changes nothing for it**. §9.8
records which path M06C is expected to use.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-B1-1** | **The person is bound by a nonce challenge answered with the person's master key over the gateway's own loopback HTTP listener.** Not a Unix-socket peer credential, not a client certificate. | Settles task.md's open design point. A peer credential identifies an **OS user**, not a person's DID, and a browser cannot present one at all — and D6 makes the browser a first-class client. Client certificates need a browser enrolment flow nobody will build for a first release. A signed challenge is the only one of the three that works from `roymctl` today and from a browser in M06C without changing the mechanism. |
| **D-B1-2** | **The wire credential is an owner→node `DelegationCertificate` with `scope = "routing"`, minted by the person and supplied at login — not pre-installed from config.** | F1: it is the exact credential the router already verifies, and D7/ADR-0013 §1 name this delegation directly. Supplying it at login means no config key, no restart to admit a new person, and no second provisioning artifact — the person's own key signs both the challenge and the delegation, so one command does everything. The node's single key legitimately holds delegations from several masters; nothing in `verify` prevents that. |
| **D-B1-3** | **Challenge and login are two calls, not one signed request.** | The nonce is transferable. That is what lets M06C's browser flow work with this exact API: the page fetches a challenge on the app's own origin, the person signs it out of band, the page POSTs the login there, and `Set-Cookie` lands on the right origin. A single-shot signed request would force a browser hand-off mechanism B1 would have to invent and M06C would have to replace. |
| **D-B1-4** | **No session = today's behaviour, unchanged: the node's self-asserted pubkey.** Not a 401. | Failure-matrix row 1 says a process that is not the person's authenticated client is refused *the person's identity* — not refused service. A blanket 401 would break M06A's `public` routes, static assets, and F9's pinned e2e, and would make the login endpoint itself unreachable. The guest can already tell the two apart: `self-asserted` vs `delegated` is exactly what that enum is for. |
| **D-B1-5** | **Sessions live in memory and die with the process.** | Simplest thing that is correct. A persisted session store is a second credential-at-rest with its own threat model, for a token whose whole lifetime is one desktop sitting. `roymctl session login` is one command to redo. Reference-scenario step 7's restart requirement is about messages (B4), not sessions. |
| **D-B1-6** | **The session token is a 32-byte random value, presented as `Cookie: syneroym_session=<token>` or `Authorization: Bearer <token>`, and never appears in a URL, a log line, or an error body.** | It is a bearer credential for a person's DID, and on a claimed substrate that DID may be the node owner (F12). Cookie for the browser, bearer for `curl`/scripts; nothing else. |
| **D-B1-7** | **The gateway strips its own credential from the bytes it forwards.** The `Cookie` pair is removed from the header value (other cookies survive; an emptied `Cookie` line is dropped); an `Authorization: Bearer` line is dropped **only** when its token was the one consumed. | F4. A deployed service — including guest code since M06A A2 — must not be able to harvest a token that lets it act as the person. Leaving an unrelated `Authorization` header alone matters: an app may use it for its own scheme. The keep-alive limitation (F3) is recorded, not fixed. |
| **D-B1-8** | **Bounded: 64 outstanding challenges (60 s TTL, single use), 64 active sessions, oldest evicted; session expiry is `min(now + session_ttl_secs, cert.expires_at_secs)`.** | `challenge` is unauthenticated by construction. Unbounded growth from a local process is the same class of failure as task.md's row 12. Clamping to the certificate's own expiry means the session can never outlive the authorization it rests on. |
| **D-B1-9** | **`roymctl session login` publishes/refreshes the person's master anchor via the existing `refresh_anchor_or_warn`, before it logs in, and warns loudly when it cannot.** Paired with **D-B1-14**, the gateway-side check that also covers a login with no `roymctl` in it. | F7. Without an anchor the session is silently useless — every proxied stream is rejected with `Unauthorized` at the destination. This is the same requirement, and the same helper, `identity certify-instance` already uses (D-A1-7). |
| **D-B1-10** | **The topology/Tier-2 resolution path keeps the node identity.** | F8. Switching it would break `[iam].grant_resolve_to_node_did` and every app-scoped hostname. |
| **D-B1-11** | **`/_syneroym/` is a reserved request-path prefix on every gateway hostname.** | The gateway must be reachable without a service hostname (F5), and reserving a path is the one addressing scheme that works identically for `curl` and for a browser page on the app's own origin (D-B1-3). Cost: a deployed app can never own a path under `/_syneroym/`. Documented, like `/.well-known`. |
| **D-B1-12** | **B1 delivers *routing* identity, not a signing capability.** The session's certificate is `scope = "routing"`, is only ever written onto a preamble, and gives the substrate no key with which to sign content as the person. | This is the half of backlog row 179 that row actually asks for — *"a local session model"* — and it is what makes `caller_did` a person downstream. The other half (the substrate producing a signature attributable to the person, which B4's DAG entries need per sender) is a separate capability that does not exist anywhere in the tree today. **B4 owes the decision** of whether to reuse this session with a second scope, or to keep signing keys out of the substrate entirely. Stating the boundary here is what stops B4 from assuming B1 shipped it. |
| **D-B1-13** | **`/_syneroym/` is refused at deploy time**, in `validate_route` for `http_routes` and in the asset-bundle path validator for static assets. | D-B1-11 shadows such a path through the gateway but leaves it reachable over direct iroh and over the WebRTC peer-proxy — the same path answering differently depending on how you arrive. A refusal at deploy is one check in each of two places and removes the inconsistency instead of documenting it. |
| **D-B1-14** | **The gateway resolves the person's master anchor once, at login, and refuses the login when it cannot** — naming the anchor as the cause. | Without this, login succeeds and *every* later connection fails: after login a client attaches a delegation to **all** of its traffic, static assets and `public` routes included, and each one costs an uncached anchor lookup that hard-rejects on miss (F7, [io.rs:346-355](../../../../crates/router/src/route_handler/io.rs#L346)). A page that rendered before login goes blank after it. One clear failure at login beats a diffuse failure afterwards, and it is the only check that also covers M06C's browser login, which has no `roymctl` step to run `refresh_anchor_or_warn`. It does not make later failures impossible — the registry can go away after login — but it removes the case where a session was **never** going to work. |
| **D-B1-15** | **Two people are separated by OS file permissions, not by the gateway.** The session file and the identity key are both `0600` on unix; `--dir` defaults to the current directory. | Honest statement of what exit criterion 3 and failure row 2 actually buy. A second process running as a **different OS user** cannot read the token or the key, so it cannot obtain the person's DID. Two people sharing one OS user and one `--dir` are not separated — inherited from the identity-key model, not created here, but it is exactly the case task.md's open design point names, so it must not be left implied. |

---

## §3 Exact type and signature changes

### 3.1 `crates/core/src/protocol_utils.rs` — two constants and one shared payload builder

Add beside `ROUTING_KEY_HEADER` ([:71](../../../../crates/core/src/protocol_utils.rs#L71)):

```rust
/// Request-path prefix the client gateway answers itself instead of
/// proxying. Reserved on every gateway hostname, so a deployed service can
/// never own a path under it -- the gateway must be reachable with no
/// service hostname at all, and a browser page on an app's own origin must
/// be able to reach it without a cross-origin request.
pub const GATEWAY_RESERVED_PATH_PREFIX: &str = "/_syneroym/";

/// Cookie carrying a local gateway session token. Consumed and removed by
/// the gateway; it is never forwarded to the target service.
pub const SESSION_COOKIE_NAME: &str = "syneroym_session";
```

plus `gateway_session_assertion(..)` — the statement a person signs at login,
built here so the gateway and `roymctl` call one function instead of keeping
two literals in step (body in §3.3).

### 3.2 `crates/core/src/config.rs` — one config field

```rust
const fn default_session_ttl_secs() -> u64 {
    8 * 3600
}

pub struct ClientGatewayRole {
    pub http_port: u16,
    pub resolve_ucan: Option<PathBuf>,
    /// Ceiling on how long a local person session stays valid. The
    /// effective expiry is the earlier of this and the presented
    /// delegation certificate's own `expires_at_secs` -- a session must
    /// never outlive the authorization it rests on.
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
}
```

`Default::default()` at [config.rs:1016-1020](../../../../crates/core/src/config.rs#L1016)
gains `session_ttl_secs: default_session_ttl_secs()`.

**One construction site breaks.** Of the 21 places that build a
`ClientGatewayRole`, exactly one omits `..Default::default()` —
[gateway_hostname_e2e.rs:190](../../../../crates/substrate/tests/gateway_hostname_e2e.rs#L190):

```rust
Some(ClientGatewayRole { http_port: gateway_port, resolve_ucan: resolve_ucan_path });
```

Add `..Default::default()` there. Every other site (including
`podman_lifecycle.rs:82` and `common/mod.rs:186`) already spreads the default
and needs no edit.

### 3.3 New module — `crates/client_gateway/src/session.rs`

Wire types (all `serde`):

```rust
#[derive(Debug, Serialize)]
pub struct ChallengeResponse {
    pub nonce: String,          // 32 random bytes, hex
    pub node_did: String,       // so the client can mint the certificate
    pub expires_at_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub person_did: String,
    pub nonce: String,
    pub signature: String,      // z-base-32, over `assertion_value(..)`
    pub delegation: DelegationCertificate,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub person_did: String,
    pub expires_at_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct WhoamiResponse {
    pub person_did: String,
    pub auth: &'static str,     // always "delegated" on this path
    pub expires_at_secs: u64,
}
```

The signed statement, built identically on both sides:

```rust
/// The statement a person signs to open a local gateway session. Domain-
/// separated by `typ` so a signature harvested here can never be replayed
/// as a delegation certificate or a UCAN payload, and bound to `node_did`
/// so it cannot be replayed to a different substrate.
///
/// **The exact object matters.** `roymctl` signs it and the gateway
/// re-derives it; RFC-8785 canonicalization sorts keys, so field order in
/// the literal is free, but every key name and the `typ` string must match
/// byte for byte or verification fails with a bare `BadSignature`. Both
/// sides call THIS function -- neither hand-builds the object.
#[must_use]
pub fn assertion_value(node_did: &str, nonce: &str, person_did: &str) -> serde_json::Value {
    serde_json::json!({
        "typ": "syneroym:gateway-session-assertion:v1",
        "node_did": node_did,
        "nonce": nonce,
        "person_did": person_did,
    })
}
```

**Where it lives:** `syneroym_core::protocol_utils`, named
`gateway_session_assertion`, **not** in this module — `roymctl` must call the
same function, and a literal duplicated across two crates is the one thing
here that would fail silently. `session.rs` re-exports it as
`assertion_value` for local readability.

Store:

```rust
#[derive(Debug, Clone)]
pub struct PersonSession {
    pub person_did: String,
    pub delegation: DelegationCertificate,
    pub expires_at_secs: u64,
}

#[derive(Debug)]
pub struct SessionStore {
    node_did: String,
    ttl_secs: u64,
    challenges: DashMap<String, u64>,          // nonce -> expires_at_secs
    sessions: DashMap<String, PersonSession>,  // token -> session
}

impl SessionStore {
    pub fn new(node_did: String, ttl_secs: u64) -> Self;
    pub fn node_did(&self) -> &str;
    pub fn issue_challenge(&self) -> ChallengeResponse;
    /// `async` for one reason: D-B1-14's anchor lookup. Everything else
    /// in it is pure and synchronous.
    ///
    /// Takes the trait, not `RegistryClient`: the successful-login unit
    /// tests (1, 9, 12) must reach the mint step, and a concrete client
    /// cannot resolve an anchor without a live registry.
    pub async fn login(
        &self,
        req: &LoginRequest,
        anchor_lookup: &dyn MasterAnchorResolver,
    ) -> Result<LoginResponse, SessionError>;
    pub fn lookup(&self, token: &str) -> Option<PersonSession>;
    pub fn logout(&self, token: &str) -> bool;
}

/// Never carries the token or the nonce in its message: these become HTTP
/// bodies.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    UnknownOrUsedNonce,
    ExpiredNonce,
    BadSignature,
    BadDelegation(String),
    WrongDelegate,      // cert.temporary_did != this node's DID
    DelegationExpired,
    /// The person's master anchor is not resolvable from this node
    /// (D-B1-14). Distinct from every value above: nothing the caller
    /// presented is wrong -- the credential simply cannot be verified by
    /// the peers this session would talk to.
    AnchorUnresolvable,
    TooManySessions,
}

impl SessionError {
    /// 401 for the credential failures, 503 for `TooManySessions`, 409
    /// for `AnchorUnresolvable` -- not 401: nothing the caller presented
    /// was rejected, so a client must not retry with a different key.
    pub const fn http_status(&self) -> u16;
    pub const fn message(&self) -> &'static str;
}
```

Credential handling, all pure and unit-testable:

```rust
/// Which header carried the token, so only that one is stripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Cookie,
    Bearer,
}

/// The gateway's own session credential on a parsed request head. `Cookie`
/// wins over `Authorization` when both carry one, so an app's own bearer
/// scheme never shadows the session.
#[must_use]
pub fn extract_credential(headers: &[httparse::Header<'_>]) -> Option<(String, CredentialSource)>;

/// The request head with the gateway's own credential removed, followed by
/// whatever body bytes were already read. Everything else is byte-identical.
/// `None` when nothing needed removing, so the caller forwards the original
/// slice untouched.
#[must_use]
pub fn strip_credential(
    raw: &[u8],
    header_len: usize,
    token: &str,
    source: CredentialSource,
) -> Option<Vec<u8>>;

/// What `handle_connection` does with a first request, decided from its
/// path alone.
#[derive(Debug, PartialEq, Eq)]
pub enum RequestKind {
    Session(SessionRoute),
    Proxy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionRoute {
    Challenge,
    Login,
    Logout,
    Whoami,
    Unknown,
}

/// Classifies by method and path (query stripped). Reserved-prefix paths
/// never reach a deployed service, including unknown ones -- a 404 from the
/// gateway, not a proxied request.
#[must_use]
pub fn classify(method: &str, path: &str) -> RequestKind;
```

Constants: `NONCE_TTL_SECS: u64 = 60`, `MAX_PENDING_CHALLENGES: usize = 64`,
`MAX_ACTIVE_SESSIONS: usize = 64`, `MAX_SESSION_BODY_BYTES: usize = 4096`,
`TOKEN_BYTES: usize = 32`.

### 3.4 `crates/client_gateway/src/gateway.rs`

```rust
struct GatewayState {
    registry_url: String,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
    identity: Identity,
    app_host_resolver: AppHostResolver,
    /// Local person sessions (M06B B1). Empty at boot and after every
    /// restart by design (D-B1-5).
    sessions: SessionStore,
    /// Used at login only, to resolve the person's master anchor once and
    /// refuse a session that could never have worked (D-B1-14). Separate
    /// from the Tier-1/Tier-2 clients `app_host_resolver` owns: this one
    /// answers a different question and runs on a different path.
    ///
    /// Held as the trait so `test_state` can stub it (§4 row 12a). `init`
    /// builds the real `RegistryClient` from `substrate.registry_url` and
    /// `substrate.enable_bep0044_dht`, the same pair the router uses.
    anchor_lookup: Arc<dyn MasterAnchorResolver>,
}
```

Two new free functions:

```rust
/// Answers a reserved-path request from the gateway itself. Never proxies.
async fn handle_session_request(
    stream: &mut TcpStream,
    state: &GatewayState,
    route: SessionRoute,
    credential: Option<&str>,
    body: &[u8],
    ttl_secs: u64,
) -> Result<()>;

/// Writes a JSON body with an explicit status, optionally with one
/// `Set-Cookie`. Always `Connection: close`.
async fn write_json(
    stream: &mut TcpStream,
    status: u16,
    body: &serde_json::Value,
    set_cookie: Option<&str>,
) -> Result<()>;
```

The doc-comment block at [gateway.rs:46-56](../../../../crates/client_gateway/src/gateway.rs#L46)
(`TODO(post-B0)`) is **deleted** and replaced with a short note stating what
the gateway now presents and when it falls back.

The comment block at [gateway.rs:57-67](../../../../crates/client_gateway/src/gateway.rs#L57)
(M06A's "a guest HTTP handler sees the node's DID") is rewritten: it now
describes the no-session path only.

The `TODO` at [gateway.rs:175-177](../../../../crates/client_gateway/src/gateway.rs#L175)
(`127.0.0.1` bind) **stays** — D-06B-5 keeps it deliberately, so its comment
gains a pointer to that decision instead of reading as an unfinished item.

### 3.5 `crates/sdk/src/lib.rs` — one parameter, twice

```rust
pub async fn passthrough_with_conn(
    conn_wrapper: TransportConnection,
    service_id: &str,
    interface_name: &str,
    initial_bytes: &[u8],
    tcp_stream: &mut TcpStream,
    identity: &Identity,
    // NEW: an owner->node routing certificate, when the caller has one.
    // `Some` makes `caller_did` at the destination the certificate's
    // `master_did` instead of `identity`'s own DID; `None` is the
    // unchanged self-asserted behaviour.
    delegation: Option<&DelegationCertificate>,
) -> Result<()>
```

and the same trailing parameter on `passthrough`
([:1031](../../../../crates/sdk/src/lib.rs#L1031)). The hardcoded
`delegation: None` at [:1072](../../../../crates/sdk/src/lib.rs#L1072) becomes
`delegation: delegation.cloned()`.

**Not done:** no `delegation` field on `SyneroymClient` and no
`with_delegation` builder. Nothing needs one — the gateway calls the static
form — and an unused field would be dead configuration.

### 3.6 `crates/client_gateway/Cargo.toml`

```toml
serde = { workspace = true }   # required: §3.3's wire types derive Serialize/Deserialize
hex.workspace = true
rand.workspace = true
```

The crate has `serde_json` today but **not** `serde`, so without this line
`session.rs` does not compile.

`[dev-dependencies]` gains `tempfile.workspace = true` only if a unit test
needs a path (it should not).

### 3.7 New module — `apps/roymctl/src/commands/session.rs`

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum SessionCommands {
    /// Open a local person session at a client gateway, so the gateway
    /// proxies as this person's DID instead of the node's own.
    Login {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        /// Community registry to publish this person's master anchor to.
        /// Without a published anchor every proxied request is rejected at
        /// the destination -- the same requirement `identity
        /// certify-instance` has.
        #[arg(long)]
        registry_url: Option<String>,
        /// Lifetime of the owner->node delegation certificate this mints.
        #[arg(long, default_value_t = 24)]
        expires_hours: u64,
    },
    /// Who the gateway currently thinks this client is.
    Status {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
    },
    /// Print the stored token, for `curl -H "Authorization: Bearer $(...)"`.
    Token {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
    },
    /// End the session at the gateway and delete the local file.
    Logout {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
    },
}

pub async fn handle(
    command: &SessionCommands,
    dir: &Path,
    run_as: Option<&str>,
) -> anyhow::Result<()>;
```

`DEFAULT_GATEWAY_URL` (`"http://localhost:7960"`) goes in
`apps/roymctl/src/lib.rs` beside `DEFAULT_API_URL`.

Stored session file: `<dir>/sessions/<sanitized-gateway-url>.json`, mode
`0600`, holding `{gateway_url, node_did, person_did, token, expires_at_secs}`.
One file per gateway, so no read-modify-write race between concurrent
invocations.

`Login` uses the **global** `--as <name>` for the person's identity — the
same flag every other signing command uses. `--as` is required here; the
command errors naming it when absent.

### 3.8 `apps/roymctl/src/commands.rs`

- `pub mod session;`
- `Commands::Session { #[command(subcommand)] command: SessionCommands }`, with
  the doc line *"Manage local person sessions at the client gateway"*.
- `run(..)` arm: `session::handle(&command, &dir, run_as.as_deref()).await?;`

### 3.9 `crates/control_plane/src/http_routes.rs` — refuse the reserved prefix (D-B1-13)

At the top of `validate_route`
([:52](../../../../crates/control_plane/src/http_routes.rs#L52)), before the
`target`/`operation` match:

```rust
if route.path.starts_with(GATEWAY_RESERVED_PATH_PREFIX) {
    return Err(format!(
        "http_routes entry `{} {}` declares a path under the reserved `{}` prefix, which the \
         client gateway answers itself and never proxies",
        route.method, route.path, GATEWAY_RESERVED_PATH_PREFIX
    ));
}
```

A static asset at `/_syneroym/…` is shadowed through the gateway in exactly
the same way, so it needs the same refusal — but **not** in
`reject_archive_entry_path`
([deploy_docs.rs:162](../../../../crates/core/src/deploy_docs.rs#L162), in
`syneroym-core`, not in `assets.rs` as revision 2 said). That helper rejects
any `RootDir` component and returns the entry's **raw** name, so an accepted
entry never begins with `/` and a `starts_with("/_syneroym/")` check there is
dead code. Archive entries look like `_syneroym/x.js` or `./_syneroym/x.js`;
the leading slash is added afterwards by `normalize_asset_path`
([assets.rs:258](../../../../crates/control_plane/src/assets.rs#L258)).

It goes on the **normalized key**, in `unpack_asset_bundle`
([assets.rs:164-180](../../../../crates/control_plane/src/assets.rs#L164)),
where the identical check already exists for a different reserved prefix:

```rust
let key = normalize_asset_path(&accepted);
// ... existing duplicate-key check ...
if key.starts_with(RESERVED_BLOBS_PREFIX) { /* existing */ }
// NEW, one more arm of the same pattern:
if key.starts_with(GATEWAY_RESERVED_PATH_PREFIX) {
    return Err(format!(
        "asset path {key:?} is under the reserved {GATEWAY_RESERVED_PATH_PREFIX} prefix, \
         which the client gateway answers itself and never proxies"
    ));
}
```

Two reasons this placement is the right one and not merely the working one.
The shared constant is used unchanged, so the prefix is never spelled twice.
And `reject_archive_entry_path` is a `pub` helper in `core` whose whole job is
lexical archive-entry safety — its own doc comment pairs it with
`reject_relative_escape`, the volume-file equivalent
([sandbox_podman/engine.rs:114](../../../../crates/sandbox_podman/src/engine.rs#L114)) —
so a gateway routing rule does not belong in it by kind. It has exactly one
production caller today ([assets.rs:164](../../../../crates/control_plane/src/assets.rs#L164)),
so this is an argument about layering, not about blast radius.

### 3.10 `crates/wit_interfaces/wit/http/http.wit` — one doc comment, no enum change

`self-asserted`'s comment
([:36-42](../../../../crates/wit_interfaces/wit/http/http.wit#L36)) currently
reads *"This is what every request proxied by the local client gateway looks
like, and there `did` is the node's own key."* That becomes false the moment a
session exists. It is guest-facing text, so it gets the same rewrite §3.4 gives
the gateway's own comment blocks:

> …This is what a request proxied by the local client gateway looks like
> **when no person session is attached**, and there `did` is the node's own
> key — the same value for every visitor. With a session the gateway presents
> a verified delegation instead, and the value here is `delegated`.

The `caller` field's own comment ([:86-88](../../../../crates/wit_interfaces/wit/http/http.wit#L86))
carries the same claim and needs the same qualification.

**The enum is unchanged** — no value added, renamed, or removed. Comment-only
edits to a `.wit` file do not change generated bindings.

### 3.11 `MasterAnchorResolver` moves from `syneroym-router` to `syneroym-core`

The trait is declared at
[handshake.rs:16-21](../../../../crates/router/src/handshake.rs#L16) with a
single impl for `RegistryClient` at
[:23-31](../../../../crates/router/src/handshake.rs#L23). `RegistryClient`
itself lives in `syneroym_core::dht_registry`, so today a `core` type's trait
impl sits in the router crate, and no crate outside the router can name the
trait at all — `client_gateway` depends on `core`, `sdk`, `rpc`, `identity`,
and `app_orchestration`, and must not gain a dependency on the router.

**Move both the trait and its impl to `crates/core/src/dht_registry.rs`**,
beside the type that implements it, and leave a re-export behind:

```rust
// crates/router/src/handshake.rs
pub use syneroym_core::dht_registry::MasterAnchorResolver;
```

Every router call site (`verify_preamble`'s `&dyn MasterAnchorResolver`
parameter, `build_caller`'s, the `MockResolver` in `handshake.rs`'s own
tests) keeps compiling unchanged. `syneroym-core` gains no new dependency:
`async_trait` is already a workspace dep and `MasterAnchorPayload` is already
defined in that module.

**Alternative rejected:** a small private trait in `session.rs` with its own
impl for `RegistryClient`. It needs no cross-crate change, but it makes a
second trait with the same shape and the same single implementor, and leaves
the original oddity — the impl living apart from its type — in place. Moving
it is fewer lines and removes a wart rather than adding one.

### 3.12 `test-components/http-guest-test/src/lib.rs`

`describe_request` ([:52-71](../../../../test-components/http-guest-test/src/lib.rs#L52))
gains one field so a test can prove the gateway's credential never arrives:

```rust
"headers": request.headers,
```

Additive — existing assertions on `header_count` are unaffected.

---

## §4 Call sites

Every site that must change, and every site that must **not**.

| # | File / anchor | Change |
|---|---|---|
| 1 | [core/src/protocol_utils.rs:71](../../../../crates/core/src/protocol_utils.rs#L71) | Add the two constants (§3.1) |
| 2 | [core/src/config.rs:1001-1020](../../../../crates/core/src/config.rs#L1001) | Add `session_ttl_secs` + its default fn; update `impl Default` |
| 2a | [substrate/tests/gateway_hostname_e2e.rs:190](../../../../crates/substrate/tests/gateway_hostname_e2e.rs#L190) | **Compile break.** The one `ClientGatewayRole` literal without `..Default::default()`; add it. Belongs to phase **P1** |
| 3 | `client_gateway/src/lib.rs` | `mod session;` — private, like the existing `mod gateway;`; only `gateway.rs` uses it, and its unit tests live in the module itself. `pub use gateway::ClientGateway;` is unchanged |
| 4 | `client_gateway/src/session.rs` | New (§3.3) |
| 5 | [client_gateway/src/gateway.rs:36-67](../../../../crates/client_gateway/src/gateway.rs#L36) | Delete the `TODO(post-B0)` block; rewrite the node-identity and M06A comment blocks |
| 6 | [client_gateway/src/gateway.rs:79-95](../../../../crates/client_gateway/src/gateway.rs#L79) | `GatewayState.sessions` |
| 7 | [client_gateway/src/gateway.rs:118-170](../../../../crates/client_gateway/src/gateway.rs#L118) (`init`) | Derive `node_did`, read `session_ttl_secs`, build the store and the `anchor_lookup` client. **Leave the `AppHostResolver` construction alone** (F8/D-B1-10) |
| 8 | [client_gateway/src/gateway.rs:175-177](../../../../crates/client_gateway/src/gateway.rs#L175) | Keep the `127.0.0.1` bind; reword the `TODO` as a D-06B-5 pointer |
| 9 | [client_gateway/src/gateway.rs:239](../../../../crates/client_gateway/src/gateway.rs#L239) | `_header_len` becomes `header_len`; add the `classify` branch |
| 10 | [client_gateway/src/gateway.rs:249-262](../../../../crates/client_gateway/src/gateway.rs#L249) | Extract the credential in the same pass as the routing key |
| 11 | [client_gateway/src/gateway.rs:313-321](../../../../crates/client_gateway/src/gateway.rs#L313) | Pass the session's delegation and the stripped bytes |
| 12 | [client_gateway/src/gateway.rs:369](../../../../crates/client_gateway/src/gateway.rs#L369) | Add `write_json` beside `write_json_rpc_error` |
| 12a | [client_gateway/src/gateway.rs:421-432](../../../../crates/client_gateway/src/gateway.rs#L421) (`test_state`) | **Compile break.** Builds `GatewayState` literally; add `sessions` and `anchor_lookup` — the latter a stub `MasterAnchorResolver`, in the shape `UnreachableTier1` already uses in that module |
| 13 | `client_gateway/Cargo.toml` | `serde`, `hex`, `rand` (§3.6 — `serde` is the compile-critical one) |
| 14 | [sdk/src/lib.rs:1031-1048](../../../../crates/sdk/src/lib.rs#L1031) | `passthrough` gains the parameter and forwards it |
| 15 | [sdk/src/lib.rs:1050-1089](../../../../crates/sdk/src/lib.rs#L1050) | `passthrough_with_conn` gains the parameter; `delegation: None` → the parameter |
| 16 | `apps/roymctl/src/commands/session.rs` | New (§3.7) |
| 17 | [apps/roymctl/src/commands.rs:22-29, 38-108, 186+](../../../../apps/roymctl/src/commands.rs#L22) | Module, enum variant, dispatch arm |
| 18 | [apps/roymctl/src/lib.rs:9](../../../../apps/roymctl/src/lib.rs#L9) | `DEFAULT_GATEWAY_URL` |
| 19 | [control_plane/src/http_routes.rs:52](../../../../crates/control_plane/src/http_routes.rs#L52) (`validate_route`) | Reserved-prefix refusal (§3.9) |
| 20 | [control_plane/src/assets.rs:164-180](../../../../crates/control_plane/src/assets.rs#L164) (`unpack_asset_bundle`) | Same refusal on the **normalized key**, beside the existing `RESERVED_BLOBS_PREFIX` arm (§3.9). **Not** in `reject_archive_entry_path` ([deploy_docs.rs:162](../../../../crates/core/src/deploy_docs.rs#L162)), where it could never match |
| 20a | [router/src/handshake.rs:16-31](../../../../crates/router/src/handshake.rs#L16) → [core/src/dht_registry.rs](../../../../crates/core/src/dht_registry.rs) | Move `MasterAnchorResolver` + its `RegistryClient` impl to `core`; `pub use` back from `handshake.rs` so no router call site changes (§3.11) |
| 21 | [wit_interfaces/wit/http/http.wit:36-42, 86-88](../../../../crates/wit_interfaces/wit/http/http.wit#L36) | Doc comments only, no enum change (§3.10) |
| 22 | [test-components/http-guest-test/src/lib.rs:61-70](../../../../test-components/http-guest-test/src/lib.rs#L61) | `"headers"` field (§3.12) |

**Deliberately unchanged, and each would be a bug to change:**

- `crates/router/**` — F1/F2. The certificate path already works; the
  guest-visible label already flips. The **one** router edit is row 20a, a
  trait move with a re-export: no behaviour, no signature, no call site.
- `crates/core/src/guest_http.rs` — no new variant (§9.1). `http.wit`'s
  **enum** is likewise unchanged; only its doc comments move (§3.10), which is
  a different thing and is listed above as a real edit.
- `guest_http_e2e.rs:323-397` — F9, the no-session path is unchanged.
- `RegistryTopologyFetcher` / `AppHostResolver` wiring — F8.

---

## §5 Pseudo-code

### 5.1 `SessionStore::issue_challenge`

```
now = now_secs()
sweep(now)                       # drop expired challenges and sessions
if challenges.len() >= MAX_PENDING_CHALLENGES:
    evict the entry with the smallest expires_at            # oldest first
nonce = hex(random 32 bytes)
expires = now + NONCE_TTL_SECS
challenges.insert(nonce, expires)
return ChallengeResponse { nonce, node_did: self.node_did, expires_at_secs: expires }
```

### 5.2 `SessionStore::login`

```
now = now_secs()
sweep(now)

# 1. Single-use nonce. `remove` is the check: a second attempt with the same
#    nonce finds nothing, so a captured signature cannot be replayed.
expires = challenges.remove(&req.nonce) or return UnknownOrUsedNonce
if now >= expires: return ExpiredNonce

# 2. Proof of possession of the person's master key.
value = assertion_value(&self.node_did, &req.nonce, &req.person_did)
substrate::verify_json_signature(&req.person_did, &value, &req.signature)
    or return BadSignature

# 3. The authorization: this person delegated THIS node, for routing.
#    `verify` (not `verify_chain`) -- a live credential being presented.
req.delegation.verify(&req.person_did, &[SCOPE_ROUTING])
    or return BadDelegation(e)
if req.delegation.temporary_did != self.node_did: return WrongDelegate
if req.delegation.expires_at_secs <= now: return DelegationExpired

# `verify`'s first argument already forces master_did == person_did, so a
# certificate for someone else cannot be attached to this login.

# 4. The anchor must be resolvable NOW, or this session could never work.
#    Note the order: step 1 already consumed the nonce, so a failure here
#    burns the challenge and the client needs a fresh one to retry. That is
#    deliberate -- removing the nonce BEFORE anything is verified is what
#    makes a captured signature unreplayable. Verifying first and consuming
#    on success would leave a live nonce for every failed attempt.
#    (D-B1-14 / F7). One lookup here replaces a hard reject on every later
#    connection -- including static assets and `public` routes, which
#    worked fine before login.
anchor_lookup.resolve_master_anchor(&req.person_did) with a 5s timeout
#   `&dyn MasterAnchorResolver` -- the trait takes no cached_timestamp, which
#   is what the router's own call passes None for anyway
    or return AnchorUnresolvable
    # The error message names the anchor and the fix
    # (`roymctl identity publish-anchor`), never the nonce or the token.

# 5. Mint.
if sessions.len() >= MAX_ACTIVE_SESSIONS:
    evict the entry with the smallest expires_at
    if still full: return TooManySessions
expires_at = min(now + self.ttl_secs, req.delegation.expires_at_secs)
token = hex(random 32 bytes)
sessions.insert(token, PersonSession { person_did, delegation, expires_at })
return LoginResponse { token, person_did, expires_at_secs: expires_at }
```

### 5.3 `SessionStore::lookup`

```
s = sessions.get(token)?          # a 256-bit random key; map lookup, no
                                  # constant-time compare (nothing to guess)
if now_secs() >= s.expires_at_secs:
    sessions.remove(token)
    return None
return Some(s.clone())
```

### 5.4 `classify`

```
path = path_before('?')
if not path.starts_with(GATEWAY_RESERVED_PATH_PREFIX): return Proxy
match (method, path):
    ("POST", "/_syneroym/session/challenge") -> Session(Challenge)
    ("POST", "/_syneroym/session/login")     -> Session(Login)
    ("POST", "/_syneroym/session/logout")    -> Session(Logout)
    ("GET",  "/_syneroym/session/whoami")    -> Session(Whoami)
    _                                        -> Session(Unknown)   # 404 here,
                                                                   # never proxied
```

### 5.5 `extract_credential`

```
cookie_token = None
for h in headers where name ~= "cookie":
    for pair in h.value.split(';'):
        (k, v) = pair.trim().split_once('=') or continue
        if k == SESSION_COOKIE_NAME: cookie_token = Some(v.to_string())
if cookie_token: return Some((cookie_token, Cookie))

for h in headers where name ~= "authorization":
    if h.value starts_with_ignore_case "bearer ":
        return Some((rest.trim().to_string(), Bearer))
return None
```

Cookie wins so an app's own bearer scheme can never shadow the session.

### 5.6 `strip_credential`

Operates on the raw head text, line by line, so every other byte survives
exactly. Header-block length is not framed by `Content-Length`, so shortening
it is safe: the receiving parser reads to `CRLFCRLF`.

```
head = raw[..header_len]           # ends with CRLF CRLF
tail = raw[header_len..]           # body bytes already read, untouched
out_lines = []
changed = false

for line in head.split("\r\n"):
    if line is empty: break        # end of the header block
    if source == Cookie and line starts_with_ignore_case "cookie:":
        pairs = value.split(';').map(trim)
                     .filter(pair -> pair != "<SESSION_COOKIE_NAME>=<token>")
        if pairs is empty: changed = true; skip the line entirely
        else:              changed = true; emit "Cookie: " + pairs.join("; ")
    elif source == Bearer and line starts_with_ignore_case "authorization:"
         and bearer_value_of(line) == token:
        changed = true; skip the line
    else:
        emit line unchanged

if not changed: return None        # caller forwards the original slice
return out_lines.join("\r\n") + "\r\n\r\n" + tail
```

Only the header that actually carried the consumed token is touched; an
`Authorization` header with any other value is forwarded untouched (D-B1-7).

### 5.7 `handle_connection`, at `Ok(Status::Complete(header_len))`

```
# `req` borrows `buf` immutably for the rest of this arm. Read any extra
# body bytes into a SEPARATE Vec -- never back into `buf` -- so nothing
# needs a mutable borrow and no field has to be copied out early.

kind = classify(req.method.unwrap_or(""), req.path.unwrap_or(""))
credential = session::extract_credential(req.headers)     # (token, source)

if kind is Session(route):
    content_length = parse "content-length" or 0
    # `curl` sends `Expect: 100-continue` for bodies over ~1 KB, and a login
    # body (certificate JSON plus signature) sits right at that line. Without
    # this the client waits for a `100` the gateway never sends and the read
    # loop below waits for bytes the client never sends. `reqwest` does not
    # use it, so `roymctl` would never have caught this.
    if any header "expect" ~= "100-continue":
        write "HTTP/1.1 100 Continue\r\n\r\n" 
    if content_length > MAX_SESSION_BODY_BYTES:
        return write_json(stream, 413, {"error": "request body too large"}, None)
    body = buf[header_len..bytes_read].to_vec()
    while body.len() < content_length:
        n = stream.read(&mut chunk).await?
        if n == 0: return write_json(stream, 400, {"error": "truncated body"}, None)
        body.extend(chunk[..n])
    body.truncate(content_length)
    return handle_session_request(stream, &state, route,
                                  credential.map(|(t, _)| t), &body, ttl)

# --- proxy path, otherwise exactly as today ---
host/target/routing_key/resolve_target/client cache: UNCHANGED

session = credential.and_then(|(t, src)| state.sessions.lookup(&t).map(|s| (s, t, src)))

match session:
    Some((s, token, src)):
        forwarded = strip_credential(&buf[..bytes_read], header_len, &token, src)
        bytes     = forwarded.as_deref().unwrap_or(&buf[..bytes_read])
        delegation = Some(&s.delegation)
        debug!(person = %s.person_did, "gateway proxying under a person session")
    None:
        bytes = &buf[..bytes_read]
        delegation = None

# The pubkey on the preamble is ALWAYS the node's key. The certificate is
# what makes `caller_did` the person's DID at the destination -- the node
# key is the delegate, and `verify_preamble` checks exactly that pairing.
passthrough_identity = Identity::from_bytes(&state.identity.to_bytes())
SyneroymClient::passthrough_with_conn(conn, &service_id, &interface,
                                      bytes, &mut stream,
                                      &passthrough_identity, delegation).await
```

### 5.8 `handle_session_request`

```
match route:
    Challenge -> write_json(200, store.issue_challenge())
    Login     -> req = serde_json::from_slice(body) or 400 "malformed login request"
                 match store.login(&req):
                     Ok(grant)  -> write_json(200, grant, Some(set_cookie(&grant)))
                     Err(e)     -> write_json(e.http_status(), {"error": e.message()}, None)
                                   # message names the failure class only:
                                   # never the nonce, never the token
    Logout    -> token = credential or 401
                 store.logout(&token)
                 write_json(200, {"status":"ended"},
                            Some("syneroym_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict"))
    Whoami    -> token = credential or 401 {"error":"no session"}
                 s = store.lookup(&token) or 401 {"error":"no session"}
                 write_json(200, {person_did, auth:"delegated", expires_at_secs})
    Unknown   -> write_json(404, {"error":"unknown gateway endpoint"})

set_cookie(grant) =
  "syneroym_session={token}; Path=/; Max-Age={ttl}; HttpOnly; SameSite=Strict"
  # No `Secure`: the gateway is plain HTTP on loopback (D-06B-5). SameSite=
  # Strict, not Lax, so no other local page can drive login or logout as a
  # side effect of a top-level navigation.
```

### 5.9 `roymctl session login`

```
identity = load <dir>/identities/<--as>.key           # error naming --as if absent
person_did = derive_did_key(identity.public_key())

# 1. Challenge -- also how the CLI learns the node DID it must delegate to.
ch = POST {gateway_url}/_syneroym/session/challenge -> ChallengeResponse

# 2. The two signatures, both by the person's own master key.
node_pubkey = substrate::resolve_did_key(&ch.node_did)
cert = DelegationCertificate::issue(&identity, node_pubkey,
                                    expires_hours * 3600, SCOPE_ROUTING)
sig  = identity.sign_json(&assertion_value(&ch.node_did, &ch.nonce, &person_did))

# 3. The anchor FIRST, then login. Order matters now: the gateway itself
#    refuses a login whose anchor it cannot resolve (D-B1-14), so publishing
#    after logging in would fail the login it was meant to enable.
member_identity::refresh_anchor_or_warn(registry_url.as_deref(), &identity).await?
#    ^^^^^^^^^^ takes Option<&str>; the CLI flag is Option<String>

# 4. Login. A 409 here means the anchor is still not visible to the gateway
#    -- surface the gateway's own message, which names the fix.
grant = POST {gateway_url}/_syneroym/session/login
        {person_did, nonce: ch.nonce, signature: sig, delegation: cert}

# Retry, if this command ever grows one: go back to step 1. The nonce is
# consumed before any verification (§5.2), so EVERY login failure -- 409
# included -- invalidates it. Re-POSTing the same body always answers
# UnknownOrUsedNonce, which would report a stale-nonce error in place of
# the real cause. Step 3 makes the 409 rare in the first place.

# 5. Persist, 0600.
write <dir>/sessions/<sanitize(gateway_url)>.json
      {gateway_url, node_did: ch.node_did, person_did, token: grant.token,
       expires_at_secs: grant.expires_at_secs}

print person DID, node DID, expiry -- never the token
```

`session token` prints the stored token (that is its whole purpose);
`session status` calls `whoami` and prints the gateway's answer, not the
file's; `session logout` POSTs and then deletes the file even if the POST
fails (the gateway may have restarted).

---

## §6 Phases

Each phase compiles, passes `cargo clippy --workspace --all-targets
--all-features`, and leaves the tree working.

| # | Phase | Contents |
|---|---|---|
| **P1** | Constants, shared payload builder, config | §3.1, §3.2, plus the one `ClientGatewayRole` literal that breaks (`gateway_hostname_e2e.rs:190`). No behaviour change |
| **P2** | `MasterAnchorResolver` move, then `session.rs` in isolation | §3.11 first (a mechanical move that must compile the whole workspace on its own), then §3.3 with its full unit suite (§7 tests 1-15). Not wired to anything |
| **P3** | SDK parameter | §3.5 + the gateway call site updated to pass `None`. Pure refactor; every existing test still passes |
| **P4** | Gateway wiring | §3.4, §5.7, §5.8. The endpoint answers, the proxy path honours a session |
| **P5** | `roymctl session` | §3.7, §3.8, §5.9 |
| **P5a** | Reserved-prefix refusal | §3.9 (`validate_route` **and** the normalized asset key) + its two tests. Independent of P2-P5; can land first if convenient |
| **P6** | e2e, fixture field, WIT comments, docs, backlog | §3.10, §3.12, the e2e suite, §8 |

P3 before P4 keeps the signature change reviewable on its own.

---

## §7 Tests

**Unit — `crates/client_gateway/src/session.rs`**

1. challenge → sign → login → `lookup` returns the person's DID and cert.
2. a signature made by a **different** key, claiming Alice's DID → `BadSignature`.
3. the same nonce used twice → second attempt `UnknownOrUsedNonce`.
4. a nonce older than `NONCE_TTL_SECS` → `ExpiredNonce`.
5. a certificate delegating to some **other** node's DID → `WrongDelegate`.
6. a certificate whose `master_did` is not the claimed `person_did` → `BadDelegation`.
7. a certificate with `scope = "service-instance"` → `BadDelegation` (proves
   `TRANSPORT_SCOPES` is **not** what this accepts; only `routing` is).
8. an already-expired certificate → `DelegationExpired`.
8a. an unresolvable master anchor, everything else valid → `AnchorUnresolvable`
    with a 409, and **no** session is created. *D-B1-14.* Every test in this
    group passes a stub `MasterAnchorResolver` (§3.11) — the success cases
    an always-`Ok` one, this case an always-`Err` one. No registry runs in
    any unit test.
9. `expires_at` is `min(ttl, cert expiry)` — asserted with a cert shorter than
   the TTL and with one longer.
10. `lookup` of a token past its expiry → `None`, and the entry is gone.
11. `MAX_PENDING_CHALLENGES + 10` challenges → map never exceeds the cap.
12. `MAX_ACTIVE_SESSIONS` reached → oldest evicted, newest usable.
13. `extract_credential`: cookie among several cookies; bearer; both present →
    cookie wins; neither → `None`; `Authorization: Basic …` → `None`.
14. `strip_credential`: cookie among others keeps the others; sole cookie drops
    the whole line; a bearer line matching the consumed token is dropped; a
    bearer line with a **different** value survives byte-identically; body
    bytes after the head survive byte-identically; `None` when nothing matched.
15. `classify`: the four routes, an unknown reserved path → `Session(Unknown)`,
    a normal app path → `Proxy`, `/_syneroym/session/whoami?x=1` → `Whoami`.

**e2e — new `crates/substrate/tests/gateway_session_e2e.rs`**, deployed
`http-guest-test`, ports via `common::alloc_ports`, master anchors published to
the harness registry.

16. **No session** → `/whoami` answers `self-asserted:<node_did>`. (Failure row
    1's negative half; mirrors F9 without replacing it.)
17. **With a session** → `/whoami` answers `delegated:<alice_did>`. *Exit
    criterion 3, reference-scenario step 4.*
18. **Two people, one node** → two logins, two tokens; each `/whoami` returns
    its own DID and never the other's, never the node's. *Failure row 2.*
19. **A second local process** with no token, on the same node, at the same
    moment Alice's session is live → `self-asserted:<node_did>`. *Failure row 1.*
20. **A forged login**: an attacker signs the challenge with its own key while
    claiming Alice's DID → 401, and `/whoami` still reports self-asserted.
21. **The guest never sees the credential**: `/echo` with the cookie set
    reports no `syneroym_session` in `headers`; with `Authorization: Bearer
    <token>` reports no `authorization`; with an unrelated `Authorization:
    Basic …` **does** report it. *D-B1-7.*
22. **Logout** → the next request falls back to `self-asserted`.
23. **Login with no published anchor** → the login is refused with 409 naming
    the anchor, and `/whoami` through the gateway still reports
    `self-asserted`. *D-B1-14, at e2e level: the failure lands at login, not
    on the next page load.*
24. **A route under the reserved prefix is refused at deploy** — an
    `http_routes` entry at `/_syneroym/x` fails `validate_route`, and an
    archive entry named `_syneroym/x.js` (**no** leading slash — that is what
    a real archive holds) fails `unpack_asset_bundle` on its normalized key.
    Both name the prefix. *D-B1-13.* Unit-level in `control_plane`; no
    substrate needed. Mirror the existing `RESERVED_BLOBS_PREFIX` test for
    the second one, including a `./_syneroym/x.js` case, since
    `normalize_asset_path` is what makes the two spellings equivalent.
25. **`Expect: 100-continue`** → a login sent with that header completes.
    Drive it with a raw TCP write, since `reqwest` never sets it. *R1-9.*

**Dropped: a restart e2e for D-B1-5.** The property (a session dies with the
process) is structural — `SessionStore` is per-process state built in
`ClientGateway::init` with no persistence path anywhere — and the test to
pin it is disproportionate. There is no restart helper on
`SubstrateTestContext`; the precedent is a per-file `Node::boot`/`teardown`
pair with a pinned tempdir ([durable_outbox_e2e.rs:616](../../../../crates/substrate/tests/durable_outbox_e2e.rs#L616)),
and that node's community registry is **in memory**, so a restart there also
drops the master anchor the session depends on — the test would have to
republish it and would then be pinning two things at once. D-B1-5 is
documented instead.

**e2e — unchanged, must stay green**

- `guest_http_e2e.rs` in full, especially F9's test.
- `gateway_hostname_e2e.rs`, `http_passthrough_e2e.rs`,
  `miniapp_demo1_wasm_e2e.rs`, and `mise run test:e2e` (Playwright) — none
  sends a session, so all take the unchanged path.

**CLI**

26. `roymctl session login` with no `--as` → error naming `--as`.
27. `roymctl session login --registry-url` absent → the warning text is
    emitted (assert on stderr, matching `refresh_anchor_or_warn`'s shape),
    and the subsequent 409 from the gateway is surfaced verbatim rather than
    swallowed.

---

## §8 Documents this slice edits

| Document | Edit |
|---|---|
| [deferred-backlog.md](../../deferred-backlog.md) §1 line 52 (*gateway presents no real end-user identity to a guest*) | → **Recently resolved** |
| [deferred-backlog.md](../../deferred-backlog.md) **§7** line 179 (*No person identity at the gateway*) | → **Recently resolved**: the row's stated need is *"a local session model"*, which B1 delivers. Its sentence about *signing* on a person's behalf is **not** closed — add a one-line note pointing at D-B1-12 and B4 |
| [deferred-backlog.md](../../deferred-backlog.md) **§7** line 180 (*Gateway caller = substrate-owner DID threading*) | → **Narrowed, not resolved.** Resolved **for the session path**: a controller who logs in now reaches `orchestrator`/`security` through the gateway (F12). With no session the gateway still presents the node DID, unchanged (D-B1-4), so the row stays open for that path. task.md says "two rows" move; there are three, plus a marker row |
| [deferred-backlog.md](../../deferred-backlog.md) "Open in-code markers" line 316 (`gateway.rs:34`) | Row deleted with the `TODO(post-B0)` comment. Note the line number is stale — the marker is at `:46`, and the block starts at `:36` |
| [deferred-backlog.md](../../deferred-backlog.md) §7 line 178 (*Client-gateway remote-access security*) and marker line 317 (`gateway.rs:108`, actually `:175`) | **Stay open**, re-pointed at D-06B-5, which keeps the loopback bind deliberately. Fix the stale line number |
| [deferred-backlog.md](../../deferred-backlog.md) | **Five new rows** (§10) |
| [developer-guide.md:254-265](../../../developer-guide.md#L254) | Replace the "denied for anything but `list`" note: a session-bearing `curl` as the controller now works. Add the `session login` → `curl -H "Authorization: Bearer …"` recipe |
| [developer-guide.md](../../../developer-guide.md) gateway-hostname section (~:1279) | Document `/_syneroym/` as reserved (D-B1-11) and the session header/cookie |
| [task.md](task.md) | B1's row → Complete; correct "`verified`" to "`delegated`" (§9.1); correct its own "§1 and §3" backlog-section reference to **§1 and §7** — it carries the same error this plan did |
| `status.md` (this directory) | **Created with B1**, per task.md |

---

## §9 Ambiguities and stale statements in the input documents

Flagged rather than guessed at. Items 1, 3, and 5 need a decision from the
reviewer before or during implementation; the rest are corrections.

1. **`verified` is not a value the WIT has.** task.md's B1 row and the
   milestone's Migration-impact section both say the `caller-auth` label
   becomes `verified`. The enum is `delegated | ucan | self-asserted`
   ([http.wit:20-43](../../../../crates/wit_interfaces/wit/http/http.wit#L20)).
   This plan produces **`delegated`**, which is what M06A defined it to mean —
   *"A verified delegation certificate"*. **No new enum value should be added**;
   adding one would be a WIT-breaking change for the label M06A already
   designed for this case. task.md's wording needs the correction.

2. **Stale line numbers.** deferred-backlog's marker table says
   `gateway.rs:34` (the comment block starts at `:36`, the `TODO` at `:46`) and
   `gateway.rs:108` (the bind `TODO` is at `:175`). task.md's own references
   (`:46`, `:57-67`, `:82-88`, `:140-155`, `:175-177`) are accurate. The spec's
   G3 cites `:36` and `#L128`, both approximately right.

3. **"Refused" in failure-matrix row 1 needs an interpretation.** Read
   literally as "the request is refused", every unauthenticated gateway request
   becomes a 401 — which breaks M06A's `public` routes, static asset serving,
   the Playwright suite, and the login endpoint itself. This plan reads it as
   **refused the person's identity** (D-B1-4): no session means the unchanged
   node-DID self-asserted path, which is exactly what F9's existing e2e pins.
   If the intent was the literal reading, D-B1-4 and tests 16/19 change and the
   e2e fallout is large — confirm before P4.

4. **The open design point is settled here, not in task.md.** *"What a person
   is bound to at the gateway"* — this plan chooses a signed challenge over a
   loopback HTTP listener, and rules out Unix-socket peer credentials (they
   name an OS user, not a DID, and no browser can present one) and client
   certificates (no browser enrolment path). D-B1-1 records the reasoning.

5. **Neither document mentions the master-anchor dependency (F7)** — the
   largest practical cost in the slice. Presenting a delegation makes every
   session-bearing stream resolve the person's master anchor from the registry
   or DHT at the destination, uncached, and a miss is a hard connection reject.
   Consequences: (a) `session login` must publish the anchor (D-B1-9); (b) a
   substrate with no registry and no DHT **cannot** carry person sessions at
   all. If offline/local-only person sessions are required for M06C, the fix is
   a local-first `MasterAnchorResolver` in the router (a node-local store of
   signed anchors consulted before the network, `RouteHandlerInner` holding
   `Arc<dyn MasterAnchorResolver>` instead of `Arc<RegistryClient>` at
   [io.rs:329,340](../../../../crates/router/src/route_handler/io.rs#L329)) —
   roughly 150 lines, and deliberately **not** in this plan. Say so now if it
   should be.

6. **The browser half of D6 is not in B1.** task.md's B1 row and exit criterion
   3 describe a local person session and a second local process; no browser
   step appears in the reference scenario until M06C. D-B1-3 makes the API
   usable from a browser without change, but the page that drives it is M06C's.
   Recorded as a backlog row rather than silently assumed done.

7. **`Set-Cookie` is host-only, and every app is its own origin.** A cookie set
   with no `Domain` attribute is sent back only to the exact host that set it,
   and the gateway gives each app its own first DNS label
   (`nickname-s<hash>.localhost:7960`). So a session opened at
   `localhost:7960` is **never** sent to any app hostname, and a browser needs
   one login per app origin. This is not a defect in D-B1-3 — it is why
   challenge and login are separate calls: the page POSTs the login on its own
   origin and the cookie lands there. But D6's "one browser origin" is what
   makes one login enough for Roym, and the plan should not read as though a
   single cookie covers the gateway. Bearer tokens are unaffected, which is
   why no CLI test would ever surface this.

8. **Two browser paths exist, and B1 reaches one of them.** F13: the WebRTC
   peer-proxy composes its own preamble in the page with no delegation, so a
   browser reaching an app that way has no person identity and B1 does not
   change that. M06C must state which path Roym's UI uses. If it is the
   WebRTC path, person identity there is unbuilt work nobody has scheduled —
   flag it now rather than at integration.

9. **D-B1-10 pins a gate that B2 changes.** The Tier-2 resolve path keeps the
   node identity because `[iam].grant_resolve_to_node_did` matches on the node
   DID ([io.rs:204-210](../../../../crates/router/src/route_handler/io.rs#L204)).
   B2 implements ADR-0022 §5's per-logical-service *"open to all"*, which is
   the mechanism that gate stands in for. No conflict today, but **B2 should
   re-open D-B1-10 deliberately** rather than inherit it. No conflict found
   with B3, B4, or B5 — except D-B1-12's open question, which is B4's.

10. **Milestone-doc statement that is now out of date.** The spec's G3 says the
    gateway *"does not authenticate the HTTP client"*; after B1 it authenticates
    an **optional** one. The spec edit is owed at milestone close, not per slice.

---

## §10 What this plan does not decide, and the backlog rows it owes

New rows for [deferred-backlog.md](../../deferred-backlog.md):

| Row | Why it is deferred | Target |
|---|---|---|
| **A session is a per-connection decision, so a keep-alive connection's later requests are attributed to the first request's session — and their credentials are forwarded unstripped** | Same root cause as the existing `X-Syneroym-Routing-Key` row: `passthrough_with_conn` hands the socket to one iroh stream and copies bytes without parsing (F3). Fixing it means the gateway parsing every request on a socket, which is a different gateway | TBD |
| **Every session-bearing gateway stream costs an uncached master-anchor lookup at the destination** | F7. D-B1-14 removes the case where a session was *never* going to work, but not the per-stream cost, and not a registry that goes away after login. A cache trades revocation latency for cost and deserves its own decision; a local-first anchor resolver is a router change outside B1's scope (§9.5) | TBD |
| **No browser-driven login flow** | D-B1-3 makes the API support one unchanged, but the page that drives it is Roym's UI | M06C |
| **The WebRTC peer-proxy browser path has no person identity at all** | F13: `peer-proxy.js` builds its own preamble in the page with no delegation, so B1's session reaches only browsers talking to the local gateway. Which path Roym's UI uses is M06C's to state (§9.8) | M06C |
| **Two people sharing one OS user and one `--dir` are not separated** | D-B1-15: separation is file permissions on the `0600` session file and identity key, not a gateway mechanism. Inherited from the identity-key model, but it is the case task.md's open design point names | TBD |

Explicitly **not** decided here: remote binding or TLS at the gateway
(D-06B-5); multi-tenant/hosted gateway auth (D8); persisting sessions across
restarts (D-B1-5); any change to `syneroym:messaging`, the outbox, or service
visibility (B2/B4).

# Slice A1 Implementation Plan — Endpoint Records Under the Member Master DID

**Status:** 📋 Planned (2026-07-28, revised after review). Eleven decisions
below (D-A1-1 … D-A1-11). The two decisions the first draft left open are now
**taken**: D-A1-7 (publish a member master's anchor) and D-A1-9 (authenticate
the whole record, not just its `service_id`) are both in A1's scope.

Two review passes. The first found **eight** problems, all folded in. Three
changed a design decision rather than a detail:

- **D-A1-5's rationale was wrong.** After A1 an expired instance certificate
  breaks *resolution*, not just the handshake — because
  `RegistryClient::lookup` verifies the record it reads, and verification now
  includes the certificate. New **D-A1-10** splits the check so publishing is
  strict and reading is not.
- **D-A1-7's first sketch wiped revocations** on every renewal.
  `publish_master_anchor` overwrites the anchor from its argument, and the
  sketch passed `vec![]` — daily, under the attended posture. Now a
  read-modify-write.
- **§3.6 wired the publisher onto a type-erased field** and would not have
  compiled.

The second pass found **seven** more, six of which are folded in and one of
which does not hold:

- **§3.7's revised snippet still did not compile** — `Identity` is not `Clone`
  ([keys.rs:88-91](../../../../crates/identity/src/keys.rs#L88)), and the match
  mixed an owned value with a borrow. Rewritten as owned bindings plus an
  `Option<&Identity>` choice.
- **The third deploy shape had no key to sign with.** `operator_identity` does
  not exist without `--as`: `client_for` falls through to an ephemeral key
  ([commands.rs:139-140](../../../../apps/roymctl/src/commands.rs#L139)), and
  there is no file to load. D-A1-8 now signs that record with a **fresh
  ephemeral key**, and the publisher verifies a stored record before ever
  replaying one.
- **The anchor read-modify-write dropped `revoke_list_registry`** — the same
  bug the decision exists to close, one field over. It now carries every
  stateful field forward, and it moved onto `RegistryClient` where it belongs.
- **D-A1-10's TTL bound was conditional** on `info.ttl` being absent, and
  `build_record` copied `ttl` from an explicitly unverified blob. The publisher
  no longer copies `ttl` at all, so the bound is unconditional.
- **Test 25 needed dev-dependencies `roymctl` does not have.** It moves to
  `crates/community_registry`, which already has everything it needs.
- Two minor fixes: the anchor helper keeps the DHT enabled, and each master's
  anchor is published once per deploy, not twice.
- **Not folded in:** "three existing unit tests call `verify()` … the compile
  surface is five." Those three
  ([dht_registry.rs:590](../../../../crates/core/src/dht_registry.rs#L590),
  `:594`, `:606`) are `SignedMasterAnchor::verify`, reached through
  `MasterAnchorPayload::sign`, which A1 does not touch. The compile surface is
  two.

A third pass found one more, and it is the sharpest of the three rounds:

- **The anchor read-modify-write degraded to a wipe on the stale path** — and
  the stale path is the *common* path, since the refresh cadence and the
  24-hour freshness bound are the same 24 hours. `resolve_master_anchor`
  returns `Err` for "too old" as well as "not found", so `.ok()` collapsed the
  two and the fix published an empty payload over a perfectly authentic
  anchor. New **D-A1-12** splits `SignedMasterAnchor::verify` the same way
  D-A1-10 split the certificate verifier, and makes the fetch distinguish
  "absent" from "unreadable" so a refresh can refuse instead of guessing.
  §5.2's backdated-anchor helper makes the stale path directly testable rather
  than argued for.
- Three small corrections applied: the `Identity` struct anchor
  (`keys.rs:88-91`, not `:81-83`, which lands inside `impl Drop for
  ZeroizingKey`), §5's test numbering renumbered with no gaps, and every
  commit anchor repointed from the branch tip `f7d8d0a` to the squash
  `f50febd` that actually landed on `main`.

A fourth pass found one more, on the same function D-A1-12 rewrites:

- **`SignedMasterAnchor::verify` authenticates only `timestamp`** — the same
  hole D-A1-9 closes for endpoint records, at the anchor, where the payload is
  a revocation list. Two things make it A1's rather than background debt:
  D-A1-7 is what gives member masters an anchor at all, and D-A1-7's
  read-modify-write would re-sign a tampered list with the real master key,
  turning an unauthenticated tamper into a permanent authentic one. New
  **D-A1-13** compares the whole payload. It also composes with D-A1-12
  correctly: a tampered anchor now lands on the "present but unreadable" arm
  and refuses the refresh instead of laundering it.

Planning found **eight** places where the design of record describes the tree
inaccurately or asserts a property it does not have (§6). One of them (§6
item 6) is a real, currently-broken behavior that Slice A0 shipped without
noticing.

**A fifth pass reopened the design itself, on branch, before merge.** An
operator question surfaced a load-bearing false premise in D-A1-2: it treated
"the hosting substrate signs the record" as fixed, when nothing requires it —
the *deployer* already holds the member master key and can sign the record
directly. That single change collapses most of what this slice built to make
delegation-signed records work at all:

- **D-A1-2 reverses.** Every record is now self-signed by the key its own
  `service_id` resolves to, so the DHT-cannot-carry-it restriction this
  decision existed to contain no longer applies — every record has a DHT home.
- **D-A1-1's second keying shape, D-A1-5, D-A1-6, and D-A1-10 are deleted.**
  A record carries no certificate to check, expire, or revoke; verification is
  the single self-signed check every record (member or not) now shares.
- **D-A1-3 and D-A1-4 survive with a narrower job**: the substrate stores and
  replays the deployer-signed blob verbatim; it never builds or signs one.
- **D-A1-8 changes**: `--master` always signs the record now, not only when
  `--nickname` is given, and the ephemeral-key envelope shape it used to fall
  back to is gone — a record signed by a throwaway key can never verify under
  the master's own `service_id`, so there is nothing left for that shape to
  do.
- **Two new decisions close what the reversal opens up.** Without the
  substrate re-signing on every heartbeat, a relocated-away substrate could
  keep replaying its last blob forever with nothing to stop it — a real
  version of the flap D-A1-11 first described. **D-A1-14** adds a
  last-writer-wins compare-and-swap on the record's own pkarr/BEP44 timestamp,
  uniformly at the DHT and the HTTP registry, so a strictly newer
  master-signed record always displaces an older one and a rollback is
  refused at both stores. **D-A1-15** adds `EndpointInfo.not_after`, a
  generous freshness backstop for the one case the timestamp alone does not
  cover: the signer stopping renewal altogether.
- **D-A1-7, D-A1-9, D-A1-12, D-A1-13 are untouched** — all four are anchor-side
  or whole-record-authentication decisions, orthogonal to who signs the
  endpoint record.

Net effect: less code than the fourth pass shipped, not more, and one
deferred-backlog row this slice opened (§7) closes outright — *master-DID
resolution requires an HTTP registry* — rather than needing a follow-up
slice to close it. Two more rows narrow without closing; see §7 for exactly
what changed and what is honestly still open.

**All line anchors are against `f50febd`** (`feat(identity): stable member
identity`, #109 -- the squash that landed A0 on `main`). Planning for this slice
was done on the pre-squash branch tip `f7d8d0a`, which is no longer an ancestor
of `main`; the two trees are identical across `crates/` and `apps/`, so no
anchor moved, but only `f50febd` resolves for anyone who did not have the
branch locally.

**Source of record:**
[ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §6,
[task.md](task.md) Slice A1.
**Paired:** [ADR-0021](../../../decisions/0021-binding-propagation-and-app-supervisor.md).
**Requirement:** `[FND-IDT]`, `[PLT-DAP-01]`.
**Slice order:** A0 ✅ → **A1** → A2 → P0 → A3 → A4 → A5. Depends on A0.

---

## 0. What this slice is, in one paragraph

A0 made a member's identity its **master DID**, so a service keeps the same
identity when it is restarted or moved. But nothing can turn a master DID into
a network address. An endpoint record is verified against the key resolved from
the `service_id` it is keyed under, and the hosting substrate holds only a
*delegated instance key*, never the master's — so the substrate cannot sign a
record that verifies under the master. A1 builds the thing that produces such
a record — **there is no publish path for service endpoint records under a
master DID today at all** (§6 item 5). Today the substrate only replays a file
the deploying operator signed, on an hourly heartbeat.

**Resolved (fifth pass, per ADR-0020 §6): the deployer signs the record
directly, with the master key it already holds.** The hosting substrate never
builds, signs, or modifies an endpoint record for a member it hosts — it
stores whatever finished, self-verifying blob the deploy call carried, and
replays those exact bytes on every heartbeat. Verification is the ordinary
self-signed check (the key `service_id` resolves to must be the key that
signed the packet), applied uniformly to every record. There is no
certificate on the record, and nothing for the registry to check at admission
beyond that one signature.

An earlier version of this slice had the *substrate* sign, using its
delegated instance key, with the record carrying a `DelegationCertificate`
binding that key to the master. §1's history below keeps that reasoning
visible rather than erasing it, since three review passes were spent getting
it right before the premise itself turned out to be avoidable — but the
design that shipped is the one above.

---

## 1. Design decisions

### D-A1-1 — One verification function, two keying shapes

**Superseded (fifth pass).** The second keying shape below (`Some(cert)`) is
gone: a record carries no certificate at all now, so there is exactly **one**
keying shape — the `None` row, applied unconditionally. `verify` no longer
takes a trust-level argument (D-A1-10, below, is what that argument existed
for, and it is gone too). What's still true: this remains one verification
function with the same two production consumers named below, and D-A1-9's
whole-record comparison (not just `service_id`) still applies. Kept for the
reasoning that carries forward.

**Resolved (original): rewrite `SignedEndpointInfo::verify`
([dht_registry.rs:103-156](../../../../crates/core/src/dht_registry.rs#L103)).
Do not add a second verifier.**

task.md's A1 text says there are "two verification paths, not one." That is not
what the tree does — see §6 item 1. `verify_endpoint_signature`
([registry.rs:234](../../../../crates/community_registry/src/registry.rs#L234))
*calls* `payload.verify()`; its own body is a `resolve_did_key` for a debug log
whose result the single caller discards. There is **one** verification function
with two production consumers — the HTTP registry's `POST /register` handler
([registry.rs:239](../../../../crates/community_registry/src/registry.rs#L239))
and `RegistryClient::lookup`'s HTTP branch
([dht_registry.rs:261](../../../../crates/core/src/dht_registry.rs#L261)) — so
§6's second acceptance path is one edit, not a reconciliation of two
implementations.

The rule, replacing the inverse one A0 pinned there:

| `info.delegation` | Record key (`info.service_id`) | Signing key | Certificate check |
|---|---|---|---|
| `None` | the signer's own DID | `service_id` | — (unchanged) |
| `Some(cert)` | **`cert.master_did`** | `cert.temporary_did` | master match + `SCOPE_SERVICE_INSTANCE` + signature; expiry per D-A1-10 |

Two things follow:

- **The narrow single-value scope check A0 deferred lands here.** A0's ingress
  admits either transport scope, because the router cannot tell a person's
  device key from a service instance. Here it can: publishing an endpoint
  record is the service-instance role and nothing else, so a `routing`
  certificate must not admit a record. A0 already wrote
  `&[SCOPE_SERVICE_INSTANCE]` at this site; A1 keeps it and adds the test that
  makes it evidence for failure-matrix row 2.
- **The expected-master argument stops being a tautology here.** On the
  router's path the expected master is read from the certificate itself
  ([delegation.rs:114-128](../../../../crates/identity/src/delegation.rs#L114)).
  Here it is `info.service_id` — the key the record is *stored under*, which is
  independent of the certificate. So this is the first production call site
  where the confused-deputy check compares two different things. Say so in the
  comment, so the next reader does not mirror the router's caveat here by
  copy-paste.

### D-A1-2 — The DHT cannot carry a delegation-signed record, and that must fail loudly, not silently

**Reversed (fifth pass).** This whole decision was a consequence of the
substrate signing with a delegated instance key while the record was keyed by
the master DID — a mismatch pkarr's signer-keyed storage cannot carry. With
the deployer signing directly, the signing key and the record's key are
always the same, so the mismatch cannot occur: **every record now has a DHT
home**, and `register`'s HTTP-registry-required refusal is deleted along with
the DHT-skip branch. Kept for the reasoning that carries forward: pkarr keys a
published packet by its signing key, full stop, and that fact is still what
makes the design work now, not what constrains it.

**Resolved (original): a delegation-signed record publishes to the HTTP registry only.
The DHT leg is skipped with a `debug!`, and `register` returns an error when
there is no HTTP registry to publish it to.**

pkarr/BEP0044 stores a packet under the public key that signed it, and resolves
by that same key. `RegistryClient::register`'s DHT leg
([dht_registry.rs:226-235](../../../../crates/core/src/dht_registry.rs#L226))
derives the pkarr public key from `info.service_id` and calls
`SignedPacket::from_relay_payload(&pkarr_pubkey, …)`, which verifies the
signature against it. For a delegated record that key is the **master**, and
the packet was signed by the **instance** — so the call fails, `?` propagates,
and the whole `register` returns `Err` even though the HTTP publish already
succeeded. Left unchanged, A1's first delegated record breaks the heartbeat
loop for every service on the node.

There is no version of §6's keying that works on the DHT. Publishing under the
instance key would work mechanically but is useless: `lookup`'s DHT branch
([dht_registry.rs:276-299](../../../../crates/core/src/dht_registry.rs#L276))
resolves by the *queried* DID's key, so a dependent asking for the master DID
would find nothing there. The one shape that would work is a forward index in
`MasterAnchorPayload`, which ADR-0020 §6 rejected outright on the two-hop cost.

So: **master-DID resolution requires a configured `registry_url`.** That is a
real narrowing of where this design works, it is not written anywhere today,
and it goes in the ADR amendment and the backlog rather than being discovered
by an operator running DHT-only.

Rejected alternative — publish the delegated record to the DHT under the
instance key "for completeness": it puts a record at an address nothing
queries, and it leaks the instance DID of every hosted member to the public
DHT for no benefit.

### D-A1-3 — The substrate publishes the record, at deploy and on the heartbeat

**Resolved: a new `EndpointPublisher` in `crates/core`, called from
`ControlPlaneService::deploy` and from the existing heartbeat loop.**

ADR-0020 §6 says "the publish path attaches the certificate," which implies a
publish path exists. It does not (§6 item 5). What exists is:

1. `roymctl svc deploy --identity <name>` builds an `EndpointInfo` client-side
   and self-signs it
   ([svc.rs:112-123](../../../../apps/roymctl/src/commands/svc.rs#L112));
2. `deploy` writes that blob to `hosted_apps/<service_id>.json`
   ([orchestration.rs:574-585](../../../../crates/control_plane/src/service/orchestration.rs#L574));
3. the heartbeat loop reads the directory and re-POSTs each file verbatim
   ([runtime.rs:680-696](../../../../crates/substrate/src/runtime.rs#L680)).

The substrate never *builds* a record. It cannot: it holds no key that the
record could be keyed under. A1 gives it one — the certified instance key — so
it can build and sign the record itself.

**Publish at deploy, not only on the heartbeat.** `HEARTBEAT_INTERVAL_SECS` is
3600. The reference scenario's step 4 has a restarted member republish under
its unchanged master, and the milestone's convergence budget is 5 s. An hourly
sweep cannot meet that, so `deploy` publishes immediately and the heartbeat
becomes the refresh/repair pass it already is for the substrate's own record.

**Publish failure never fails a deploy.** Warn and continue — the heartbeat
retries. That matches today's posture, where a registry being down does not
stop a deploy. That makes `publish_all_services` the recovery path, so it gets
its own test (§5.4), not just incidental coverage.

### D-A1-4 — Which services the substrate publishes for, and what wins

**Resolved: an installed instance certificate makes the substrate
authoritative for that service's record. Everything else keeps today's replay
behavior, byte for byte.**

Per service, at publish time:

| `registry.instance_cert(service_id)` | Behavior |
|---|---|
| `Some(cert)`, not expired, owner row present | Build a fresh record, key it by `cert.master_did`, sign with the derived instance key, attach `cert`. The stored file is read **only** for `nickname` and `is_private`. |
| `Some(cert)`, expired | Publish nothing, `warn!`. (D-A1-5) |
| `Some(cert)`, no recorded owner | Publish nothing, `warn!` — the instance key is derived from the owner DID, so without it there is no key to sign with. |
| `None` | Replay `hosted_apps/<service_id>.json`, **after verifying it**. |

The last row is what keeps pre-A0 services working untouched, matching A0's own
`None`-means-fallback rule everywhere else. It gains one check today's
heartbeat does not do: verify before replaying. Today the loop re-POSTs any
file it finds and lets the registry reject it. After D-A1-8 a stored file may
legitimately be unverifiable — a metadata envelope for a service that has a
certificate — and a later redeploy *without* a certificate would leave that
file behind to be replayed (`deploy` only overwrites the file when the manifest
carries one). Verifying first turns a confusing 401 in the logs into a local
warning, and is a strict improvement on blind replay regardless.

**`ttl` is deliberately not carried over.** D-A1-10's safety argument rests on
the registry's default TTL bounding how long an unrefreshed record survives,
and the registry prefers `info.ttl` when it is set
([registry.rs:151](../../../../crates/community_registry/src/registry.rs#L151)).
Copying `ttl` out of an explicitly-unverified blob would make that bound
operator-settable to any length. Nothing in the tree sets a non-`None` `ttl`
today, so this costs nothing; if a real TTL surface ever appears, the right
shape is a cap at `cert.expires_at_secs`, which is already in the record.

**The stored file is the exception, not the norm** (review point 7). On the
`--master` and `--instance-certificate` paths its *only* remaining job is to
carry operator metadata; its signature is neither trusted nor checked, because
a record built from it is never republished. And `app deploy` never sets
`registry_certificate` at all
([mapper.rs:207](../../../../crates/sdk/src/mapper.rs#L207)), so every
app-deployed member — including both members of the milestone's own reference
scenario — publishes with `nickname: None` and `is_private: false`. That is
intended, but the table above must not read as though a stored file is the
common case. Carrying this metadata inside a signed record is historical; the
clean shape is a `DeployManifest` field, which is a WIT change plus ~40 test
literals and is recorded in the backlog rather than taken here.

**The record's `mechanisms` stay empty and `substrate_id` is the node's own
DID.** That is what makes relocation work with no operator action:
`lookup(resolve = true)` follows `substrate_id` to the hosting substrate's
record and copies its mechanisms
([dht_registry.rs:316-320](../../../../crates/core/src/dht_registry.rs#L316)),
so a member that moves publishes a record naming its new host and resolution
follows automatically. This is the mechanism the slice exists for, so §5.7
tests it through `lookup(resolve = true)` and `resolve_iroh_addr`, not by
reading record fields.

### D-A1-5 — An expired certificate publishes nothing (deleted, fifth pass)

**No longer applicable.** A record carries no certificate to expire. What
replaces this concern is D-A1-15: the record's own `not_after`, checked the
same way — a record that fails to verify (now including an elapsed
`not_after`) is skipped and warned, never force-published. Body kept for the
superseded reasoning.

**Resolved (original): skip and warn.** Same posture as A0 took on the proxy's
presentation arm ([proxy.rs:405-422](../../../../crates/router/src/proxy.rs#L405)):
publishing a record that no verifier accepts is worse than publishing nothing.
There is no fallback to a self-signed record — the substrate has no master key,
by ADR-0020 §3.

**Correction to the first draft.** It justified this with "publishing nothing
leaves the previous, still-valid record in place until its TTL." That is wrong:
the previous record carries the *same* certificate, so it stops verifying at
the same moment. What actually bounds the exposure is D-A1-10 plus the
registry's TTL — see there.

### D-A1-6 — Revocation is checked at admission, not at lookup (deleted, fifth pass)

**No longer applicable.** With no certificate on the record, there is no
instance key named on it to check against `revoked_keys` at admission —
revocation is now purely what it already was for every other purpose: a
handshake-time check on the connection a caller presents, per this decision's
own closing paragraph below. Body kept for the superseded reasoning; the
honest-revocation-scope correction to ADR-0020 §6 that this decision produced
carries forward unchanged into the amended §6.

**Resolved (original): a best-effort, registry-local revocation check in
`register_endpoint`. Nothing added to `verify` and nothing added to the
resolution path.**

ADR-0020 §6 closes with "Revocation continues to work unchanged … a record
signed by a revoked key stops verifying." That is **false today and false after
A1** (§6 item 3): `SignedEndpointInfo::verify` is a pure, synchronous function
that never consults a master anchor, and `DelegationCertificate::verify` checks
the signature, window, and scope — not revocation.

Three places could check it, and only one is affordable:

| Site | Verdict |
|---|---|
| `SignedEndpointInfo::verify` | No. It is sync and has no resolver; giving it one turns every verification into a network call. |
| `RegistryClient::lookup` | No. It would add a second network lookup to the resolution path, which the milestone's own performance budget forbids and which is the exact cost ADR-0020 §6 rejected the anchor-index option over. |
| The HTTP registry's `register_endpoint` | **Yes.** The registry already holds master anchors in `state.master_anchors` ([registry.rs:59](../../../../crates/community_registry/src/registry.rs#L59)) from its own `/register_master` endpoint. The check is a `DashMap` get and a `Vec::contains`. No network, no new dependency. |

**This check depends entirely on D-A1-7** (review point 4). `state.master_anchors`
is populated only by `POST /register_master`, and nothing publishes an anchor
for a member master today. Had D-A1-7 been deferred, this would have been dead
code in production plus one unit test that publishes an anchor by hand. With
D-A1-7 taken, every certified member master has an anchor and the check fires.
The two are one decision, and the plan should not have presented them as
independent.

State plainly what this buys and what it does not: it stops a revoked instance
key from *refreshing* a record at a registry that holds the anchor. It is
defence in depth. **The real gate is the handshake**, where
`HandshakeVerifier::verify_preamble` already resolves the master anchor and
rejects a revoked temporary DID
([handshake.rs:84-90](../../../../crates/router/src/handshake.rs#L84)) — a
revoked instance can therefore keep a stale record alive and still not answer a
single call. That is the honest version of §6's claim and it goes in the ADR
amendment.

### D-A1-7 — **Taken.** Publish the member master's anchor, preserving its revocation list

`verify_preamble` resolves the master anchor for any presented certificate and
propagates the failure with `??`
([handshake.rs:84-87](../../../../crates/router/src/handshake.rs#L84)), and
`resolve_master_anchor` returns `Err` when no anchor is found
([dht_registry.rs:396-398](../../../../crates/core/src/dht_registry.rs#L396)).
Nothing in A0's path publishes an anchor for a minted member master:
`resolve_or_mint_member_master` writes a key file and
[`certify_instance`](../../../../apps/roymctl/src/commands/member_identity.rs#L95)
issues a certificate; neither calls `publish_master_anchor`. The only publisher
in the tree is `roymctl identity publish-anchor`
([identity.rs:190-202](../../../../apps/roymctl/src/commands/identity.rs#L190)),
which an operator has no reason to run for a member master because nothing
tells them to.

So **a guest-origin call that presents its instance certificate is rejected at
the destination** with "Master Anchor not found". A0 did not catch this because
its e2e fixture never drives the guest arm over a real hop (its own module doc
says so), and `proxy.rs`'s unit tests build preambles without running a
handshake.

**The naive fix wipes revocations, and must not be shipped** (review point 3).
`publish_master_anchor` builds `MasterAnchorPayload { revoked_keys, .. }`
straight from its argument
([dht_registry.rs:411-413](../../../../crates/core/src/dht_registry.rs#L411)),
and `register_master_endpoint` overwrites the stored anchor outright
([registry.rs:288](../../../../crates/community_registry/src/registry.rs#L288)).
Publishing with `vec![]` on every `certify-instance` would clear that master's
revocation list — daily, under the attended posture — which directly undoes
failure-matrix row 14, whose A0 evidence is
`a_revoked_instance_key_is_rejected_while_the_member_master_still_certifies_a_new_one`.

So the publish is a **read-modify-write**: resolve the current anchor, carry
**every stateful field** forward, republish under the same master key.
`MasterAnchorPayload` has four fields, and two of them carry state —
`revoked_keys` *and* `revoke_list_registry`, which delegates the revocation
list to an external registry. The first draft of this fix preserved only the
first, which is the same bug one field over: a renewal would silently detach a
delegated revocation list. `schema` and `timestamp` are re-derived by `sign`
and must not be carried.

**It lives on `RegistryClient`, not in `roymctl`.** It is a registry operation
composed of two registry operations, and putting it in the CLI made its
behavior test need `syneroym-community-registry` as a `roymctl`
dev-dependency — `roymctl`'s test suite drives the binary and has only
`assert_cmd`, `predicates`, and `tempfile`. As a `RegistryClient` method, the
test lives in `crates/community_registry`, which already depends on
`syneroym-core` and already builds an `EcosystemRegistry` in its own tests. It
also inherits the client's DHT setting instead of hardcoding one — an anchor
is self-signed by the master, so unlike an endpoint record it *does* have a
valid DHT home, and `roymctl identity publish-anchor` already enables it
([identity.rs:196](../../../../apps/roymctl/src/commands/identity.rs#L196)).

Two limits, both stated rather than hidden:

- **It races.** Two concurrent renewals for the same master can each read the
  old list and drop a revocation added between them. Acceptable for an
  operator-run command under the attended posture, and it is strictly better
  than unconditional clearing. A5's unattended issuer is where this needs a
  real answer.
- **Nothing in the tree can *add* to `revoked_keys`.** `roymctl identity
  publish-anchor` hardcodes `vec![]`
  ([identity.rs:200](../../../../apps/roymctl/src/commands/identity.rs#L200)),
  and the only non-empty publisher is a test. So revocation is currently a
  mechanism with no operator surface. A1 preserves what is there and adds a
  backlog row for the missing `revoke` verb rather than growing one here.

Second half of the same gap: `SignedMasterAnchor::verify` rejects any payload
older than 24 hours
([dht_registry.rs:537-541](../../../../crates/core/src/dht_registry.rs#L537)),
so an anchor is not publish-once — it is a **daily** republication duty. A1
makes each `certify-instance` refresh it, which under the attended posture puts
the anchor on the same cadence as the certificate. Automating the cadence is
A5's, with the rest of the online-key posture.

**When `--registry-url` is not supplied, warn loudly** rather than silently
skipping: the certificate that was just minted will be rejected at every
destination until an anchor exists.

### D-A1-8 — `roymctl svc deploy`: three shapes, not two

**Revised (fifth pass).** The third row below — `--instance-certificate`
signing with a fresh ephemeral key — is gone. It relied on the substrate
re-signing and discarding the outer signature (D-A1-4's old job); with no
re-signing, a record signed by a throwaway key can never verify under
`service_id` and would never be replayed, so the shape bought nothing under
the new design and is deleted rather than kept as dead capability. The
`--master` row changes too: it now signs **unconditionally**, not only when
`--nickname` is given, because it is the *only* way a member's record is ever
produced — not an optional metadata carrier. Concretely: `signing_identity`
collapses from a three-armed match to
`named_identity.as_ref().or(master_identity.as_ref())`. Body kept for the
reasoning that still applies to the surviving two shapes.

**Resolved (original): keep `--identity`, change what it is for, and give all three
deploy shapes a way to carry a nickname.**

Today `--nickname` is only read inside `if let Some(name) = identity`
([svc.rs:109-123](../../../../apps/roymctl/src/commands/svc.rs#L109)), so an
operator deploying with `--master` and wanting a nickname has to also pass
`--identity <the same master>` — which works only by coincidence, because the
master identity happens to be the one key that can self-sign a record keyed by
the master DID.

`svc deploy` has **three** mutually-exclusive-ish shapes, and the first draft
covered two (review point 7):

| Shape | Signs the metadata record with | Why |
|---|---|---|
| `--identity <name>` | that identity | Unchanged. The no-master, self-signed publish route. |
| `--master <name>` | the master identity `--master` already loads | It is local by definition on this path. |
| `--instance-certificate <path>` | a **fresh ephemeral key** | Nothing usable is guaranteed to be on this machine — see below. The record is metadata only here (D-A1-4), never republished, so its signature is not load-bearing. |

The third row is the one the first draft dropped, leaving an operator holding a
certificate signed elsewhere with no way to set a nickname.

**Why ephemeral and not the operator's own key** (second review pass). The
first revision proposed the `--as` identity. There is not always one:
`client_for` falls through to `SyneroymClient::new` when `run_as` is `None`
([commands.rs:139-140](../../../../apps/roymctl/src/commands.rs#L139)), which
mints a fresh ephemeral key rather than loading a file — so
`identities/<name>.key` need not exist, and the third shape would have failed
on the default invocation, which is exactly the case the flag exists for. The
master key is unavailable by definition on this path, and its *name* cannot be
recovered from the certificate. So there is no meaningful key to reach for,
and pretending otherwise adds a flag requirement instead of a capability.

An ephemeral signature is honest about what this blob is: an envelope carrying
three operator-chosen strings to the substrate, which the substrate opens and
throws away. It is one line and needs no flag.

Three consequences to write down:

- The stored file on the third path **does not self-verify** (signer ≠
  `service_id`), by design.
- That is safe because a service with an installed certificate never replays
  its stored file, **and** because D-A1-4 now verifies a stored record before
  replaying it on the `None`-certificate path. Without that second guard there
  is a reachable hole: `deploy` only writes the file when the manifest carries
  one, so redeploying a service *without* a certificate leaves the old
  unverifiable envelope on disk to be replayed and 401'd. `undeploy` does
  delete both file and certificate
  ([orchestration.rs:956-961](../../../../crates/control_plane/src/service/orchestration.rs#L956),
  [:1050](../../../../crates/control_plane/src/service/orchestration.rs#L1050)),
  so the hole is redeploy-only, but it is real.
- No clap `conflicts_with` between `--identity` and `--master`: they are not
  contradictory, one just supersedes the other's signature.

**Rejected for A1: carry the metadata in `DeployManifest` instead.** That is
the shape this wants — metadata should not travel inside a credential envelope
at all. It costs a WIT record change plus ~40 `DeployManifest` literals, *and*
a new per-service persistence sidecar, because the publisher reads this at
heartbeat time and a manifest is not stored per service; `hosted_apps/<id>.json`
is currently that storage. Too much for two cosmetic fields. Backlog row in §7,
with the sidecar cost written into it so the row is actionable.

### D-A1-9 — **Taken.** Authenticate the whole record, not just its `service_id`

`SignedEndpointInfo::verify` parses the `EndpointInfo` out of the signed pkarr
packet and compares exactly one field against the outer copy —
`parsed_info.service_id == self.info.service_id`
([dht_registry.rs:127](../../../../crates/core/src/dht_registry.rs#L127)).
Every other field of the outer `info` is unauthenticated: `substrate_id`,
`mechanisms`, `nickname`, `is_private`, `ttl`. The registry stores and serves
the **outer** copy
([registry.rs:223](../../../../crates/community_registry/src/registry.rs#L223),
[:275](../../../../crates/community_registry/src/registry.rs#L275)) and
`resolve_iroh_addr` reads `info.info.mechanisms`
([net_iroh.rs:134](../../../../crates/router/src/net_iroh.rs#L134)). So anyone
who can post to the registry can take a valid signed record, rewrite
`substrate_id` to a host they control, and redirect every call to that service.

Fix: derive `PartialEq, Eq` on `EndpointInfo` and compare the whole struct.
Pre-existing, but A1 is rewriting this exact comparison, and after A1 the
unauthenticated set would include `delegation` itself.

Safe by construction on every producing path: `EndpointInfo::sign` builds
`SignedEndpointInfo` from the same value it serializes into the packet, and
`lookup`'s DHT branch constructs the outer copy *from* the parsed packet. The
one place the outer copy is deliberately mutated after verification is
`lookup`'s `resolve = true` mechanism copy
([dht_registry.rs:316-320](../../../../crates/core/src/dht_registry.rs#L316)),
which runs **after** the DHT backfill `register` call
([:312](../../../../crates/core/src/dht_registry.rs#L312)) — so no mutated
record is ever re-verified. Confirmed, not assumed.

### D-A1-10 — Certificate expiry is checked when publishing, not when reading (deleted, fifth pass)

**No longer applicable.** There is no certificate on the record to have an
expiry, and `verify` no longer takes a trust-level argument at all — every
call site uses the single `verify()`. The cliff this decision existed to
avoid is real again in a different shape, and D-A1-15 is its replacement:
`not_after` is checked uniformly (no publish/read split), but is set
generously enough that the routine case — a signer that renews on a normal
cadence — never gets near it. Body kept for the reasoning that carries
forward into D-A1-15's own rationale.

**New (original), from review point 2. This is the change that keeps A1 from turning a
missed renewal into a name-resolution outage.**

`RegistryClient::lookup`'s HTTP branch calls `info.verify()` and returns `Err`
on failure, deliberately without falling back to the DHT
([dht_registry.rs:261-264](../../../../crates/core/src/dht_registry.rs#L261)).
After D-A1-1, `verify()` validates the attached certificate, and
`DelegationCertificate::verify` rejects an expired one
([delegation.rs:161-167](../../../../crates/identity/src/delegation.rs#L161)).

Composed, that means: **the moment a member's instance certificate expires,
every lookup of its master DID hard-errors.** Affected consumers —
`resolve_iroh_addr` ([net_iroh.rs:133](../../../../crates/router/src/net_iroh.rs#L133)),
`coordinator_webrtc/src/bootstrap.rs:183` and `:319`,
`sdk/src/lib.rs:243`, `roymctl registry lookup`, and the smoke tests. With
`DEFAULT_INSTANCE_CERT_EXPIRES_HOURS = 24`
([svc.rs:22](../../../../apps/roymctl/src/commands/svc.rs#L22)), a single
missed daily renewal would take out name resolution as well as the handshake.

**Resolved: strict when admitting, chain-only when reading.**

A read path has no business re-adjudicating a publishing credential. The
registry already decided this record was admissible while the certificate was
live; the reader's question is only "does the trust chain hold" — master match,
scope, signature. Expiry is then bounded by the registry's own TTL
(`DEFAULT_REGISTRY_TTL_SECS` = 7200) instead of biting instantly: once the
certificate lapses, D-A1-5 stops the publisher refreshing the record, and the
registry drops it within two hours. Fails closed, on a clock that gives an
operator a window instead of a cliff.

**That bound is only real because D-A1-4 stops copying `ttl`.** The registry
uses `info.ttl` when it is set and `DEFAULT_REGISTRY_TTL_SECS` only as the
fallback
([registry.rs:151](../../../../crates/community_registry/src/registry.rs#L151)),
and the first revision had `build_record` copy `ttl` from the stored blob —
whose signature D-A1-8 explicitly does not check. That would have made this
decision's entire safety argument settable to an arbitrary length through an
unverified field. The publisher now sets `ttl: None` unconditionally.

Two API changes:

```rust
// crates/identity/src/delegation.rs
impl DelegationCertificate {
    /// The master match, the scope, and the signature -- everything `verify`
    /// checks except the validity window.
    ///
    /// For one case only: reading a record that some other party already
    /// admitted while this certificate was live. Re-checking the window
    /// there turns a lapsed renewal into an immediate resolution failure for
    /// every consumer, when the thing the credential proves -- that the
    /// master authorized this key -- has not stopped being true.
    ///
    /// **Never admit anything with this.** Connecting, publishing, and
    /// installing all check the window, because there the certificate is a
    /// live credential being presented. When in doubt, use `verify`.
    pub fn verify_chain(&self, expected_master_did: &str, accepted_scopes: &[&str]) -> Result<()>;

    /// `verify_chain` plus the validity window.
    pub fn verify(&self, expected_master_did: &str, accepted_scopes: &[&str]) -> Result<()>;
}
```

```rust
// crates/core/src/dht_registry.rs
/// Which side of a record's life this verification is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordTrust {
    /// Admitting a record for storage. The certificate must be valid now.
    Publishing,
    /// Reading a record a registry already admitted. The trust chain is
    /// checked; the certificate's expiry is not (D-A1-10).
    Reading,
}

impl SignedEndpointInfo {
    pub fn verify(&self, trust: RecordTrust) -> Result<(), anyhow::Error>;
}
```

Both production call sites are known and there are only two:
`registry.rs:239` → `Publishing`; `dht_registry.rs:261` → `Reading`.

**`task.md`'s failure-matrix row 3 needs correcting.** It says the
missed-renewal failure mode "is row 1" and needs no distinct code behavior.
After A1 that is no longer true: a lapse now also removes the member from name
resolution, on the registry TTL clock. That is a distinct behavior, it gets a
test (§5.1 test 10), and the row's text has to say so.

**Rejected alternative — keep the strict read.** It is one fewer concept, and
it makes expiry bite everywhere at once. Rejected because the blast radius is
out of proportion to the miss: an operator who is one hour late renewing loses
every dependent's ability to *find* the service, not just to authenticate to
it, with no grace period at all.

### D-A1-11 — Two live publishers under one master: the heartbeat changes the race

**Narrowed (fifth pass) rather than resolved.** D-A1-14's compare-and-swap
closes the flap's *symmetric* form: once the master signs a strictly newer
record for the new placement, the old substrate's replayed heartbeat — same
bytes, same timestamp, forever — is rejected as stale at both stores, not
merely raced against. What D-A1-14 does not close: the old substrate is still
*trying* to publish, and it does not know it has lost the race, so the flap
this decision names becomes "one silently-failing publisher plus one
succeeding one" instead of "two nodes alternately winning." Detecting and
stopping the losing publisher from trying at all still needs a placement view
neither this slice nor D-A1-14 has — the backlog row narrows rather than
closes. Body kept for the reasoning that still applies to the residual case.

**New (original), from review point 6. Not fixed in A1; named, tested around, and given a
backlog row.**

ADR-0020 §6 and task.md both argue that one master per member means at most one
live publisher, so the registry's last-writer-wins insert
([registry.rs:223](../../../../crates/community_registry/src/registry.rs#L223))
is already correct. Nothing enforces that invariant.

Before A1 a duplicate could only arise if two operators each supplied a
`registry_certificate`. After A1 it is the default shape for every mastered
service: each node with an installed certificate rebuilds and republishes
hourly, so a relocation that leaves the old instance deployed produces **two
nodes publishing the same master forever** — a continuous hourly flap between
two `substrate_id` values, not a single last-writer-wins insert that settles.

A1 does not fix this. Detecting it needs either a publisher generation stamp or
the supervisor's own view of where a member is meant to run, and both belong to
A3/A5, where relocation becomes real. What A1 does:

- **The e2e models a clean relocation**, undeploying from node B before
  deploying on node A (§5.7). The first draft's version left both deployed and
  passed only because `HEARTBEAT_INTERVAL_SECS` is 3600 — a test that passes
  for the wrong reason.
- **Backlog row** naming the shape and pointing at A3/A5.

### D-A1-12 — Reading back your own anchor is not the same trust question as consuming one

**New, from the third review pass. Without this, D-A1-7's read-modify-write
degrades to exactly the wipe it exists to prevent — and it does so precisely
when a refresh is needed.**

`resolve_master_anchor` returns `Err` for more than "not found." Its HTTP
branch verifies the fetched anchor and **returns early** on failure
([dht_registry.rs:345-349](../../../../crates/core/src/dht_registry.rs#L345)),
and `SignedMasterAnchor::verify` rejects any payload older than 24 hours
([dht_registry.rs:537-541](../../../../crates/core/src/dht_registry.rs#L537)).
So a stale anchor produces `Err`, `.ok()` turns that into `None`,
`unwrap_or_default()` produces an empty `revoked_keys` and
`revoke_list_registry: None`, and `publish_master_anchor` writes that over the
old anchor as authoritative.

Four things make this the common case rather than an edge:

- **It fires exactly when the refresh is needed.** D-A1-7's own argument is
  that an anchor is a daily duty, and the attended cadence is
  `DEFAULT_INSTANCE_CERT_EXPIRES_HOURS` = 24. An operator renewing at hour 25
  hits the stale path every single time. The read-modify-write would protect
  the case where nothing needed refreshing and fail the case that motivated it.
- **The DHT does not rescue it.** That `return Err` is an early return, so the
  DHT fallback below is never reached.
- **It is not transient.** The registry's expiry sweep walks `endpoints` only,
  never `master_anchors`
  ([registry.rs:149-159](../../../../crates/community_registry/src/registry.rs#L149)),
  so a stale anchor sits in the map being served and failing verification until
  something overwrites it.
- **A dead registry does not cause it.** `publish_master_anchor` returns `Err`
  when the POST fails, so an unreachable registry aborts before writing. The
  wipe is specific to "anchor found, too old."

**Resolved: split the anchor verifier the same way D-A1-10 split the
certificate verifier, and distinguish "absent" from "present but unreadable."**

The 24-hour bound exists for *consumers* — do not trust a stale revocation
list. It has no business gating a master reading back its own last-published
state before re-signing it. That is the identical distinction D-A1-10 draws,
at a second site, so it gets the identical shape:

```rust
// crates/core/src/dht_registry.rs
impl SignedMasterAnchor {
    /// Everything `verify` checks except the 24-hour freshness bound: the
    /// master DID resolves, the pkarr packet is signed by it, the payload
    /// carried alongside is byte-for-byte the one inside the packet, and
    /// that payload's timestamp is the packet's.
    ///
    /// For a master reading back its own anchor to re-sign it. Freshness is
    /// a consumer's question -- "is this revocation list current enough to
    /// act on" -- and applying it here would make a late refresh silently
    /// publish an empty payload over a stale but perfectly authentic one.
    ///
    /// **Never consume an anchor through this.** Revocation checks use
    /// `verify`.
    pub fn verify_signature(&self) -> Result<(), anyhow::Error>;

    /// `verify_signature` plus the 24-hour freshness bound.
    pub fn verify(&self) -> Result<(), anyhow::Error>;
}
```

and a fetch that reports the three states apart instead of collapsing two of
them:

```rust
impl RegistryClient {
    /// The anchor currently published for `master_did`, for a holder of that
    /// master key about to republish it. `Ok(None)` means the registry has no
    /// anchor for this master; `Err` means one exists but could not be read,
    /// which a caller must not treat as "start empty."
    ///
    /// HTTP only. The DHT is a fallback for *finding* an anchor, and a master
    /// republishing its own state should not adopt a payload from a source it
    /// cannot correct.
    async fn fetch_own_master_anchor(
        &self,
        master_did: &str,
    ) -> anyhow::Result<Option<MasterAnchorPayload>>;
}
```

`refresh_master_anchor` then refuses rather than guessing:

```
match self.fetch_own_master_anchor(&master_did).await {
    Ok(Some(prev)) => carry (prev.revoked_keys, prev.revoke_list_registry) forward,
    Ok(None)       => empty,          # genuinely no anchor yet
    Err(e)         => return Err(e),  # never publish an empty payload over
                                      # something we could not read
}
```

A stale anchor now routes through the first arm, because `verify_signature`
takes no time input at all — its state is carried forward and republished with
a fresh timestamp, which is the whole point of a refresh.

**One tightening taken while here.** Nothing today checks that a fetched
anchor's `master_id` is the one that was asked for. `SignedMasterAnchor::verify`
resolves the key from `self.master_id`, so an anchor for a *different* master
verifies happily, and a registry could answer a lookup with someone else's —
which on this path would splice their revocation list onto ours, and on the
handshake path could drop a revocation. `fetch_own_master_anchor` asserts the
equality, and so does `resolve_master_anchor`. Two lines, no behavior change
for an honest registry. The larger version of the same problem — the anchor's
*contents* being unauthenticated — is D-A1-13.

### D-A1-13 — **Taken.** D-A1-9 at the anchor: authenticate the whole payload

**New, from the fourth review pass, on the function D-A1-12 rewrites.**

`SignedMasterAnchor::verify` compares exactly one field of the payload against
the signed packet. Reading
[dht_registry.rs:543-573](../../../../crates/core/src/dht_registry.rs#L543),
the only comparisons are `parsed_payload.timestamp != packet_timestamp` and
`self.payload.timestamp != packet_timestamp`. `revoked_keys`,
`revoke_list_registry`, and `schema` in the outer copy are never checked
against the signed copy — and the outer copy is what everything consumes:
`resolve_master_anchor`'s HTTP branch returns `signed_anchor.payload`
([:361](../../../../crates/core/src/dht_registry.rs#L361)),
`register_master_endpoint` stores the whole received struct
([registry.rs:288](../../../../crates/community_registry/src/registry.rs#L288)),
and `lookup_master_endpoint` serves it back.

So anything that can answer a `/lookup_master` — a hostile registry, a
compromised one, a relay — can add or remove entries from `revoked_keys` with
the timestamp untouched, and every consumer accepts it. Stripping an entry
makes `verify_preamble` admit a revoked instance key; adding one denies service
to a live member. It is exactly the D-A1-9 shape, one struct over.

**Why this is A1's rather than background debt.** Three reasons, and the third
is the one that makes it non-optional:

- **D-A1-7 makes anchors load-bearing.** Before this slice no member master had
  an anchor at all, so the field carried nothing. After it, every certified
  master has one and the handshake resolves it on every delegated connection.
- **D-A1-6 reads this same unauthenticated field**, out of
  `state.master_anchors`, which is populated by that same verify-then-store
  path.
- **The read-modify-write launders a tamper into a signature.**
  `refresh_master_anchor` carries `prev.revoked_keys` forward and re-signs it
  with the real master key. A stripped revocation that arrived unauthenticated
  comes back out *authentically signed*, and permanently. Nothing re-signed
  anchors before this slice; after it, `certify-instance` does, daily. That is
  a consequence A1 creates, not one it inherits — which is what settles it.

**Fix: derive `PartialEq, Eq` on `MasterAnchorPayload` and compare the whole
struct.** Every field is `String`, `Vec<String>`, `Option<String>`, or `u64`.
In `verify_signature`, `parsed_payload == self.payload` replaces the in-loop
timestamp check; the trailing `self.payload.timestamp != packet_timestamp` stays,
because binding the payload to the *packet's* timestamp is a separate property
from binding outer to embedded (and `resolve_master_anchor`'s `cached_timestamp`
comparison reads `payload.timestamp` directly). The `schema` guard stays in the
`let` chain, where it selects which TXT record to read rather than validating
one. `MASTER_ANCHOR_SCHEMA_V1` stays checked against the constant.

Safe by construction on the only producing path: `sign` serializes `self` into
the TXT record and returns that same value as `payload`
([dht_registry.rs:482-500](../../../../crates/core/src/dht_registry.rs#L482)).
There is no legitimate divergence anywhere — the DHT branch builds its result
from the parsed packet, and the registry stores and serves what it was handed.

**It composes with D-A1-12 the right way, and that is worth stating.** A
tampered anchor now fails `verify_signature`, so `fetch_own_master_anchor`
returns `Err`, so `refresh_master_anchor` lands on D-A1-12's "present but
unreadable" arm and **refuses** — rather than laundering the tamper. The cost
is that a tampered or corrupt anchor blocks automatic refresh for that master
until someone intervenes. The escape hatch already exists and is deliberately
separate: `roymctl identity publish-anchor` publishes an empty payload
unconditionally, so resetting a poisoned anchor is an explicit operator act,
never an automatic one.

**A0's two anchor tests still pass**, with one changing its failure reason:
`test_master_anchor_payload_timestamp_validation` mutates
`signed.payload.timestamp` and expects `Err`, which it now gets from the
equality check rather than the timestamp check.

### D-A1-14 — New (fifth pass). A monotonic timestamp, uniformly enforced at the DHT and the HTTP registry

**Resolved: `SignedEndpointInfo::verify` returns the pkarr packet's own
signed timestamp; the registry admits a record only if that timestamp is
strictly newer than what is stored, or equal and byte-identical.**

Once the substrate stops re-signing, the record it replays on every heartbeat
is frozen — the exact bytes handed to it at deploy. That is what makes a
rollback attack against the *registry* newly meaningful in a way it was not
before: a substrate the member has moved away from can keep POSTing that same
frozen blob forever, and the registry's plain `DashMap::insert`
([registry.rs:223](../../../../crates/community_registry/src/registry.rs#L223)
in the pre-fifth-pass tree) accepts whatever arrives last with no ordering
check at all. The DHT already does not have this hole — `mainline`'s own
server refuses a `put` whose sequence number is lower than the one it holds —
but `RegistryClient::lookup` tries the HTTP registry first, so the weaker of
the two stores is the one that actually answers a lookup.

The fix reuses the number mainline already trusts rather than inventing a
second one: pkarr signs `<timestamp><packet>`, and that timestamp *is*
BEP44's `seq`. It is already inside the signed bytes (unforgeable without the
signing key) and already the number the DHT compares. Two new fields would
give two numbers that could disagree; one field, read out of the packet
`verify` already parses, cannot.

The registry's own admission rule mirrors mainline's, including the case that
would be easy to get wrong: an **equal** timestamp with byte-identical bytes
must succeed as a refresh (it resets the TTL clock), not fail as a conflict —
the substrate replays the identical frozen blob every heartbeat now, and
rejecting that would mean the record simply expires two hours after every
deploy, never refreshed again. Equal-but-different is rejected, same as
older: two records claiming the same instant cannot be resolved by preferring
one arbitrarily. `DashMap::entry` is used rather than a read followed by a
write, so two concurrent refreshes cannot interleave into the older one
landing last.

Applied identically to master-anchor registration for consistency
(`MasterAnchorPayload.timestamp`, already authenticated as equal to the
packet's own timestamp by D-A1-13's whole-payload check, serves as the same
CAS key with no new field).

### D-A1-15 — New (fifth pass). `EndpointInfo.not_after`: a generous freshness backstop, not the sharp control

**Resolved: a required `not_after: u64` (Unix seconds) field, checked by
`verify` uniformly — no publish/read split, unlike D-A1-10's now-deleted
certificate-expiry split.**

D-A1-14's timestamp ordering is the sharp control for the case it covers — a
member that actually moves. It does nothing for the case where a signer stops
renewing *at all*: a lost master key, a decommissioned member whose last
substrate just keeps heartbeating the same never-superseded blob forever.
`not_after` is the backstop for that case alone, which is why it can be — and
should be — generous: weeks, not hours. `DEFAULT_ENDPOINT_NOT_AFTER_SECS` is
30 days, deliberately far longer than an instance certificate's lifetime
(hours), because a reader that enforced it tightly would recreate exactly the
cliff D-A1-10 existed to avoid: a routine missed renewal would turn into an
instant, sitewide resolution failure the moment the bound lapsed, rather than
the record quietly aging past a backstop nobody was relying on day to day.

No publish/read split is needed here the way D-A1-10 needed one for
certificates: `not_after` is not a credential a reader is re-adjudicating, it
is the record's own stated claim about itself, so there is only one question
("has this record's own bound passed?") and one place to ask it.

---

## 2. Phase plan

Each phase compiles and its tests pass on its own.

| # | Phase | Gate |
|---|---|---|
| 1 | `crates/identity`: `verify_chain` / `verify` split (D-A1-10) | `cargo test -p syneroym-identity`; A0's six scope tests stay green |
| 2 | `crates/core/src/dht_registry.rs`: `RecordTrust`, `verify`'s two keying shapes, `sign_as_instance`, the DHT-leg skip, `PartialEq`; `SignedMasterAnchor::verify_signature`, `fetch_own_master_anchor`, `refresh_master_anchor` | Unit tests §5.1, §5.2 |
| 3 | `crates/community_registry`: admission revocation check, `verify_endpoint_signature` simplification, the anchor-refresh regression tests | Unit tests §5.3 |
| 4 | `crates/core/src/endpoint_publisher.rs`: new module | Unit tests §5.4 |
| 5 | `crates/control_plane` + `crates/substrate/src/runtime.rs`: wiring, publish-on-deploy, heartbeat sweep | Unit tests §5.5; existing suites stay green |
| 6 | `apps/roymctl`: D-A1-7's anchor refresh, D-A1-8's three shapes | §5.6 |
| 7 | e2e: `crates/substrate/tests/master_endpoint_record_e2e.rs` | §5.7, sandbox disabled |
| 8 | Docs: ADR-0020 amendment, task.md rows 3 and 4, traceability matrix, status.md, backlog | §7, §8 |

---

## 3. Exact changes

**Superseded by the fifth pass for the sections it touched (§3.2
`dht_registry.rs`, §3.3 `community_registry/registry.rs`, §3.4
`endpoint_publisher.rs`, §3.6's `EndpointPublisher::new` call and
`build_signed_endpoint_info` snippets, §3.7 `svc.rs`) — kept as the record of
the fourth-pass design, not as a diff of what shipped.** The snippets build
the instance-key-signs-with-a-delegation-certificate design D-A1-2's
reversal replaced; §0 and the decisions in §1 describe what actually
shipped, and the source files are the literal record of it. §3.1
(`delegation.rs`), the rest of §3.5 (`control_plane`) and §3.6 (the
substrate's own self-record's construction, its control-plane wiring, and
its error handling), and §3.8 (`roymctl` anchor-publishing) are **not**
superseded — none of that depends on who signs a member's endpoint record,
so it still describes what shipped.

### 3.1 `crates/identity/src/delegation.rs`

Split `verify` (lines 129-190) into `verify_chain` + a window check, per
D-A1-10. `verify_chain` keeps today's order — master match, then scope, then
signature — and `verify` runs the window checks after it.

**Watch the error ordering when implementing.** Today the window is checked
*before* the signature, so a certificate that is both expired and forged
reports "expired". After the split it reports the signature failure. No
accept/reject behavior changes; re-run A0's six delegation tests and adjust any
that assert on message text.

### 3.2 `crates/core/src/dht_registry.rs`

**(a) `EndpointInfo` derives equality** (D-A1-9):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointInfo { /* unchanged fields */ }
```

`EndpointType`, `EndpointMechanism`, and `DelegationCertificate` already derive
`PartialEq, Eq`; every remaining field is `String`, `bool`, `Option<u64>`, or
`Vec<EndpointMechanism>`, so both derives compile as-is.

**(b) `RecordTrust`**, as in D-A1-10.

**(c) New constructor, so the delegated shape cannot be assembled wrong:**

```rust
impl EndpointInfo {
    /// Signs this record with `instance` -- the substrate-derived key that
    /// `cert` certifies -- so it can be keyed by the member master DID
    /// instead of by the signer's own (ADR-0020 §6). Sets `service_id` and
    /// `delegation` from the certificate rather than trusting the caller to
    /// keep all three in agreement.
    pub fn sign_as_instance(
        mut self,
        instance: &Identity,
        cert: DelegationCertificate,
    ) -> Result<SignedEndpointInfo, anyhow::Error> {
        let instance_did = substrate::derive_did_key(&instance.public_key());
        if instance_did != cert.temporary_did {
            return Err(anyhow::anyhow!(
                "signing key {instance_did} is not the key this certificate certifies ({})",
                cert.temporary_did
            ));
        }
        self.service_id = cert.master_did.clone();
        self.delegation = Some(cert);
        self.sign(instance)
    }
}
```

**(d) `SignedEndpointInfo::verify` — full replacement of lines 103-156:**

```rust
pub fn verify(&self, trust: RecordTrust) -> Result<(), anyhow::Error> {
    // Two acceptance shapes (ADR-0020 §6). Without a certificate a record is
    // self-signed and keyed by the signer's own DID -- the original rule.
    // With one, the record is keyed by a *member master* DID and signed by
    // an instance key the certificate binds to that master, which is how a
    // substrate holding only a delegated key publishes for the member it
    // hosts.
    //
    // Unlike the router's ingress check, the expected master here is not
    // read from the certificate: it is the key the record is stored under,
    // which is independent of what the certificate claims. So the
    // confused-deputy comparison genuinely bites on this path.
    // `service-instance` alone is accepted -- publishing a record is that
    // role and no other, so a `routing` certificate must not admit one.
    //
    // Expiry is checked when admitting a record and not when reading one: a
    // reader re-adjudicating a publishing credential would turn a lapsed
    // renewal into an instant resolution failure for every consumer, where
    // letting the registry's TTL drop the unrefreshed record fails closed
    // with a window instead of a cliff.
    let signer_did = match &self.info.delegation {
        Some(cert) => {
            match trust {
                RecordTrust::Publishing => {
                    cert.verify(&self.info.service_id, &[SCOPE_SERVICE_INSTANCE])?
                }
                RecordTrust::Reading => {
                    cert.verify_chain(&self.info.service_id, &[SCOPE_SERVICE_INSTANCE])?
                }
            }
            cert.temporary_did.as_str()
        }
        None => self.info.service_id.as_str(),
    };

    let pubkey = substrate::resolve_did_key(signer_did)
        .map_err(|e| anyhow::anyhow!("Failed to parse public key from signer DID: {e}"))?;
    let expected_pkarr_pubkey = PublicKey::try_from(pubkey.as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid ed25519 pubkey for pkarr: {e}"))?;

    let packet_bytes = hex::decode(&self.pkarr_packet_hex)
        .map_err(|_| anyhow::anyhow!("Invalid hex encoding for pkarr packet"))?;
    let bytes_obj = Bytes::from(packet_bytes);
    let signed_packet = SignedPacket::from_relay_payload(&expected_pkarr_pubkey, &bytes_obj)
        .map_err(|e| anyhow::anyhow!("Invalid pkarr packet signature or structure: {e}"))?;

    if signed_packet.public_key() != expected_pkarr_pubkey {
        return Err(anyhow::anyhow!("Signed packet public key does not match the signer DID"));
    }

    // The whole record, not just its service_id (D-A1-9): the registry
    // stores and serves this outer copy, and `substrate_id` is what a
    // lookup follows to an address, so anything left uncompared here is
    // rewritable by whoever relays the record.
    let mut found_txt = false;
    for answer in signed_packet.resource_records(PKARR_DNS_NAME) {
        if let RData::TXT(txt) = &answer.rdata
            && let Ok(full_string) = String::try_from(txt.clone())
            && let Ok(parsed_info) = serde_json::from_str::<EndpointInfo>(&full_string)
            && parsed_info == self.info
        {
            found_txt = true;
            break;
        }
    }

    if !found_txt {
        return Err(anyhow::anyhow!("pkarr packet does not contain this exact EndpointInfo"));
    }

    Ok(())
}
```

**(e) `RegistryClient::lookup`** (line 261): `info.verify(RecordTrust::Reading)`.

**(e2) `RegistryClient::refresh_master_anchor`** — D-A1-7's read-modify-write,
here rather than in `roymctl` because it is two registry operations composed
and because its behavior test then needs no new dev-dependency anywhere:

```rust
/// Publishes or refreshes `master`'s anchor, carrying forward every stateful
/// field the current anchor holds.
///
/// A `DelegationCertificate` is unusable on the wire until its master's
/// anchor is resolvable: the destination's handshake resolves it to check
/// revocation and fails closed when it is missing. Anchors also stop
/// verifying after 24 hours, so this is a refresh as much as a first
/// publish.
///
/// Read-modify-write, because `publish_master_anchor` overwrites the whole
/// payload: republishing with defaults would silently un-revoke every
/// retired instance key of this master, and detach a delegated revocation
/// list. `schema` and `timestamp` are re-derived by `sign` and are not
/// carried. Races with a concurrent refresh of the same master; acceptable
/// for an operator-run command, and what an unattended issuer has to solve
/// properly.
///
/// Reads through `fetch_own_master_anchor`, not `resolve_master_anchor`: the
/// latter rejects an anchor older than 24 hours, and the refresh is a daily
/// duty, so a late operator would land on that path every time and publish
/// an empty payload over a stale but authentic one (D-A1-12). An unreadable
/// anchor aborts here; it never degrades to "start empty."
pub async fn refresh_master_anchor(&self, master: &Identity) -> anyhow::Result<()> {
    let master_did = substrate::derive_did_key(&master.public_key());
    let (revoked_keys, revoke_list_registry) = self
        .fetch_own_master_anchor(&master_did)
        .await?
        .map(|prev| (prev.revoked_keys, prev.revoke_list_registry))
        .unwrap_or_default();
    self.publish_master_anchor(&master_did, revoked_keys, revoke_list_registry, master, true)
        .await
}
```

Plus `fetch_own_master_anchor` and the `SignedMasterAnchor::verify_signature` /
`verify` split, both specified in D-A1-12, and the `master_id` equality check
added to `fetch_own_master_anchor` and to `resolve_master_anchor`.

The DHT is whatever this client was built with, not hardcoded off. Unlike an
endpoint record (D-A1-2), an anchor is self-signed by the master and has a
valid DHT home, so `roymctl` builds the client with the DHT enabled, matching
`identity publish-anchor`.

**(f) `RegistryClient::register` — DHT leg (lines 226-235):**

```rust
if signed_info.info.delegation.is_some() && self.registry_url.is_none() {
    // pkarr keys a packet by its signing key, so a delegation-signed
    // record can only ever land under the instance DID -- never under the
    // master DID a dependent looks up. There is no DHT-only home for it.
    return Err(anyhow::anyhow!(
        "a delegation-signed endpoint record needs an HTTP registry; the DHT keys records by \
         their signing key and cannot hold one under its master DID"
    ));
}

if let Some(dht) = &self.dht_client {
    if signed_info.info.delegation.is_some() {
        tracing::debug!(
            service_id = %signed_info.info.service_id,
            "skipping DHT publish for a delegation-signed endpoint record"
        );
    } else {
        // ... existing body, unchanged ...
    }
}
```

The early return goes **before** the existing `http_success` block so the error
names the real cause rather than surfacing as a pkarr signature failure.

### 3.3 `crates/community_registry/src/registry.rs`

`verify_endpoint_signature` (lines 234-249) becomes:

```rust
fn verify_endpoint_signature(
    state: &RegistryState,
    payload: &SignedEndpointInfo,
) -> Result<(), (StatusCode, String)> {
    if let Err(e) = payload.verify(RecordTrust::Publishing) {
        return Err((StatusCode::UNAUTHORIZED, format!("Signature verification failed: {e}")));
    }

    // Defence in depth, not the gate: this stops a revoked instance key from
    // refreshing a record at a registry that already holds the master's
    // anchor. It costs a map lookup and no network call. Revocation is
    // actually enforced at the handshake, where a revoked temporary DID
    // cannot complete a connection at all -- so a record that slips past
    // this check still buys its holder nothing.
    if let Some(cert) = &payload.info.delegation
        && let Some(anchor) = state.master_anchors.get(&cert.master_did)
        && anchor.0.payload.revoked_keys.contains(&cert.temporary_did)
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("instance key {} has been revoked by its master", cert.temporary_did),
        ));
    }

    Ok(())
}
```

Two edits fall out:

- The single caller becomes `verify_endpoint_signature(&state, &payload)?;`
  ([registry.rs:204](../../../../crates/community_registry/src/registry.rs#L204)).
- The `ed25519_dalek::VerifyingKey` import
  ([registry.rs:20](../../../../crates/community_registry/src/registry.rs#L20))
  is dropped — the returned key was discarded by its only caller, and
  `payload.verify` already resolves and validates the DID. `substrate` is still
  used by this file's tests, so scope that import to `mod tests` rather than
  removing it (mandatory import cleanup, per AGENTS.md).

Nothing else in this file changes. Alias handling, the last-writer-wins insert,
and parent propagation are all correct as-is for a master-keyed record.

### 3.4 `crates/core/src/endpoint_publisher.rs` (new module)

Registered in `crates/core/src/lib.rs` as `pub mod endpoint_publisher;`. No new
crate dependencies: `syneroym-core` already owns `RegistryClient`
(`dht_registry`), `EndpointRegistry` (`local_registry`), and depends on
`syneroym-identity`.

```rust
//! Publishing a hosted service's endpoint record under its member master DID.
//!
//! A substrate holds a delegated instance key for each member it hosts, never
//! the member's master key (ADR-0020 §3). This builds the record that key is
//! entitled to publish: keyed by the master DID, signed by the instance key,
//! carrying the certificate that binds them.

pub struct EndpointPublisher {
    registry_client: Arc<RegistryClient>,
    registry: EndpointRegistry,
    node_identity: Arc<Identity>,
    node_did: String,
    hosted_apps_dir: PathBuf,
}

impl EndpointPublisher {
    pub fn new(
        registry_client: Arc<RegistryClient>,
        registry: EndpointRegistry,
        node_identity: Arc<Identity>,
        node_did: String,
        hosted_apps_dir: PathBuf,
    ) -> Self;

    /// Publishes `service_id`'s endpoint record. `Ok(false)` means there was
    /// nothing to publish -- no installed certificate and no stored record --
    /// which is a normal state, not a failure.
    pub async fn publish_service(&self, service_id: &str) -> anyhow::Result<bool>;

    /// Every hosted service. A per-service failure is warned and the sweep
    /// continues; this is the retry path for a deploy-time publish that
    /// failed, so one unreachable record must not stop the rest.
    pub async fn publish_all_services(&self);

    /// Split out from `publish_service` so the whole decision table is
    /// testable without a registry to publish to.
    fn build_record(&self, service_id: &str) -> Option<SignedEndpointInfo>;
}
```

`build_record` pseudo-code (the D-A1-4 table, in order):

```
# On the certificate path this blob is metadata only, and its signature is
# neither trusted nor checked: with `--instance-certificate` it is signed by
# an ephemeral key and cannot self-verify (D-A1-8). Safe because a service
# with a certificate never republishes it verbatim -- and the `None` branch
# below verifies before it does.
stored = read_to_string(hosted_apps_dir/<service_id>.json)
           .ok()
           .and_then(|s| serde_json::from_str::<SignedEndpointInfo>(&s).ok())

cert = registry.instance_cert(service_id)
if cert is None:
    # Pre-A0 path: replay, but only a record that still verifies. A service
    # redeployed without a certificate can leave an earlier metadata
    # envelope behind (`deploy` overwrites this file only when the manifest
    # carries one), and re-POSTing it would earn a 401 the operator has no
    # way to explain.
    if stored is Some(record) and record.verify(RecordTrust::Publishing).is_err():
        warn!(service_id, "stored endpoint record no longer verifies; not republishing")
        return None
    return stored

if cert.is_expired():
    warn!(service_id, "instance certificate expired; not republishing")
    return None

owner = registry.owner_of(service_id)
if owner is None:
    warn!(service_id, "no recorded owner; cannot derive the instance key")
    return None

instance = node_identity.derive_service_identity(&owner, service_id)
if derive_did_key(&instance.public_key()) != cert.temporary_did:
    # owner row and certificate have drifted apart -- a redeploy by a
    # different operator, most likely. Publishing under a key the
    # certificate does not name would produce a record nothing accepts.
    warn!(service_id, "derived instance key does not match the installed certificate")
    return None

info = EndpointInfo {
    service_id:    cert.master_did.clone(),   # equals service_id, set by sign_as_instance
    substrate_id:  node_did.clone(),
    endpoint_type: EndpointType::Service,
    mechanisms:    vec![],                    # resolution follows substrate_id
    nickname:      stored.and_then(|s| s.info.nickname.clone()),
    is_private:    stored.map_or(false, |s| s.info.is_private),
    ttl:           None,                      # never from the blob -- D-A1-10
    delegation:    None,                      # set by sign_as_instance
}
info.sign_as_instance(&instance, cert).ok()   # warn! on Err
```

`publish_all_services` pseudo-code:

```
ids: BTreeSet<String> =
    registry.all_instance_certs().map(|(id, _)| id)
    ∪ hosted_apps_dir entries whose file stem parses as a service id

for id in ids:
    match publish_service(&id):
        Ok(true)  => debug!(id, "published endpoint record")
        Ok(false) => {}                       # nothing to publish; not an error
        Err(e)    => warn!(id, %e, "failed to publish endpoint record")
```

`publish_service` is the thin wrapper: `build_record` → `registry_client
.register(&record, false).await` → `Ok(true)`.

The `hosted_apps_dir` half of the union preserves today's heartbeat behavior
exactly for services with no certificate.

### 3.5 `crates/control_plane`

**`service.rs`** — one field, one setter, one line in `init`:

```rust
// in `struct ControlPlaneService`
/// Set after construction by the substrate's composition root. A setter
/// rather than an `init` parameter because `init` has 36 call sites, 35 of
/// them tests with nothing to publish. Same two-phase wiring `service_proxy`
/// already uses, for the same ordering reason.
endpoint_publisher: OnceLock<Arc<EndpointPublisher>>,
```

```rust
pub fn set_endpoint_publisher(&self, publisher: Arc<EndpointPublisher>) {
    let _ = self.endpoint_publisher.set(publisher);
}
```

`init` adds `endpoint_publisher: OnceLock::new()` to its struct literal
([service.rs:113-140](../../../../crates/control_plane/src/service.rs#L113)).
**No `init` call site changes.** `OnceLock::set` takes `&self`, so this works
after the service is already inside an `Arc`
([runtime.rs:535](../../../../crates/substrate/src/runtime.rs#L535)).

**`service/orchestration.rs` — `deploy`**, after the `set_owner` (line 855) and
`set_instance_cert` / `remove_instance_cert` (lines 876-877) branches succeed:

```rust
// Publish now rather than at the next heartbeat: a member reinstantiated
// here has to become resolvable under its unchanged master DID promptly,
// and the heartbeat runs hourly. Never fatal -- a registry that is down
// must not fail a deploy, and the heartbeat sweep repairs it.
if let Some(publisher) = self.endpoint_publisher.get()
    && let Err(e) = publisher.publish_service(&service_id).await
{
    tracing::warn!("Failed to publish endpoint record for {}: {}", service_id, e);
}
```

`deploy_plan` delegates to `deploy`, so app deploys are covered by the same
hook — checked, not assumed.

**`service/orchestration.rs` — `undeploy`**: no change. There is no unregister
endpoint on the HTTP registry, so a retired member's record ages out by TTL.
`undeploy` already removes both the stored file and the certificate, so nothing
republishes it. Pre-existing; backlog row in §7.

### 3.6 `crates/substrate/src/runtime.rs`

**`setup_router`** (line 421) returns the publisher and wires it into the
control plane before returning.

**Correction from review point 1.** The first draft used
`route_handler_deps.control_plane_service`, which is `Arc<dyn NativeService>`
([route_handler.rs:171](../../../../crates/router/src/route_handler.rs#L171))
and has no `set_endpoint_publisher`. The concrete handle is a different field,
`control_plane: Option<Arc<ControlPlaneService>>`
([route_handler.rs:179](../../../../crates/router/src/route_handler.rs#L179)),
`None` for the router's test doubles and `Some` in this composition root
([runtime.rs:545](../../../../crates/substrate/src/runtime.rs#L545)). Both
fields move into `ConnectionRouter::init`, so clone first.

`None` here would mean publish-on-deploy silently off. That is a composition-
root bug, not a supported configuration, so it **errors** rather than degrades:

```rust
async fn setup_router(
    config: &SubstrateConfig,
    service_id: &str,
    secret_key: [u8; 32],
) -> anyhow::Result<(ConnectionRouter, EndpointRegistry, Option<Arc<EndpointPublisher>>)> {
    // ... unchanged through `build_route_handler_deps` ...
    let route_handler_deps = build_route_handler_deps(...).await?;
    let control_plane = route_handler_deps.control_plane.clone();

    let router = ConnectionRouter::init(...).await?;

    // Built here rather than in `build_route_handler_deps` because it needs
    // the finished `EndpointRegistry`, and handed to the control plane so a
    // deploy can publish immediately instead of waiting for the heartbeat.
    let publisher = (config.substrate.registry_url.is_some()
        || config.substrate.enable_bep0044_dht)
        .then(|| {
            Arc::new(EndpointPublisher::new(
                Arc::new(RegistryClient::new(
                    config.substrate.enable_bep0044_dht,
                    config.substrate.registry_url.clone(),
                )),
                endpoint_registry.clone(),
                Arc::new(Identity::from_bytes(&secret_key)),
                service_id.to_string(),
                config.hosted_apps_dir(),
            ))
        });

    if let Some(publisher) = &publisher {
        // A registry is configured, so a deploy must be able to publish. A
        // type-erased control plane cannot, and silently skipping the wiring
        // would leave deploy-time publishing off with nothing to notice it.
        let control_plane = control_plane.ok_or_else(|| {
            anyhow::anyhow!(
                "a community registry is configured but no concrete ControlPlaneService was \
                 built, so a deploy could not publish its endpoint record"
            )
        })?;
        control_plane.set_endpoint_publisher(publisher.clone());
    }

    Ok((router, endpoint_registry, publisher))
}
```

**Its single call site** (line 385) destructures three values:

```rust
let (router, endpoint_registry, publisher) = setup_router(config, &service_id, secret_key).await?;

if let Some(publisher) = publisher
    && let Some(endpoint_addr) = router.endpoint_addr()
{
    let relay_url = config.parent_coordinator.iroh.as_ref().map(|c| c.url.clone());
    publish_to_community_registry(
        config.substrate.registry_url.clone(),
        config.substrate.enable_bep0044_dht,
        service_id,
        endpoint_addr,
        relay_url,
        secret_key,
        config.identity.nickname.clone(),
        publisher,
    );
}
```

The old `(dht || registry_url)` guard is now carried by `publisher.is_some()`,
the same condition, so the `if` reads as one test instead of two.

**`publish_to_community_registry`** (line 624): the `hosted_apps_dir: PathBuf`
parameter becomes `publisher: Arc<EndpointPublisher>`, and the hosted-apps
block (lines 679-696) collapses to:

```rust
// Hosted services. A service with an installed instance certificate gets a
// freshly built, instance-signed record keyed by its member master DID; one
// without keeps replaying the operator-signed record it was deployed with.
publisher.publish_all_services().await;
```

`RegistryClient::new` inside the spawned task (line 635) stays — the publisher
holds its own. Consolidating the substrate's three `RegistryClient` instances
(this one, `route_handler.rs:202`, and the publisher's) is noise cleanup, not
A1's job; noted in §6 item 9.

**`build_signed_endpoint_info`** (line 748): unchanged. The substrate's own
record is self-signed by the node key and is not a delegated record.

### 3.7 `apps/roymctl/src/commands/svc.rs` — D-A1-8

Replace the `identity`/`nickname` block at lines 108-123 so all three deploy
shapes can carry metadata. The `--master` arm below already loads the master
identity; hoist that load so the key is read once, not twice.

**`Identity` is not `Clone`** — no derive and no manual impl
([keys.rs:88-91](../../../../crates/identity/src/keys.rs#L88)), and it cannot
cheaply be one, since it holds a `Box<ZeroizingKey>` over locked memory. So the
candidates are bound as owned values first and the choice is a **borrow**;
`Identity::sign` takes `&self`, and the `--master` arm below borrows the same
binding instead of re-loading:

```rust
// The record the substrate stores at deploy. Whenever an instance
// certificate is involved the substrate re-signs its own record with the
// certified instance key and reads only this one's nickname and privacy,
// so any key at all is enough to carry it -- `--identity` is the no-master
// path's actual self-signed publish route.
//
// Bound owned, chosen by reference: `Identity` is not `Clone`, and the
// `--master` arm below needs the same key again.
let named_identity = identity.as_deref().map(|n| load_identity(dir, n)).transpose()?;
let master_identity = master
    .as_deref()
    .map(|n| member_identity::resolve_member_master(dir, n))
    .transpose()?;
// No master key and no named identity, but a nickname to carry: sign the
// envelope with a throwaway key. There is no operator key to reach for --
// without `--as`, `client_for` mints an ephemeral one rather than loading a
// file -- and the substrate discards this signature anyway.
let envelope_identity = match (nickname, &named_identity, &master_identity, instance_certificate) {
    (Some(_), None, None, Some(_)) => Some(Identity::generate()?),
    _ => None,
};

let signing_identity: Option<&Identity> = match (&named_identity, nickname) {
    (Some(id), _) => Some(id),
    (None, Some(_)) => master_identity.as_ref().or(envelope_identity.as_ref()),
    (None, None) => None,
};

let cert = signing_identity
    .map(|id| EndpointInfo { /* as today */ }.sign(id))
    .transpose()?;
```

The `--master` arm at lines 125-145 then reads
`master_identity.as_ref().ok_or_else(...)` instead of calling
`resolve_member_master` a second time.

Doc-comment change on `--identity`: say it is the self-signed publish route for
a service with no member master, and that with `--master` or
`--instance-certificate` the substrate publishes the record itself and this
blob carries metadata only.

### 3.8 `apps/roymctl` — D-A1-7

- Add `#[arg(long)] registry_url: Option<String>` to
  `IdentityCommands::CertifyInstance`, `SvcCommands::Deploy`, and `app deploy`.
- The helper itself is `RegistryClient::refresh_master_anchor` (§3.2 (e2)), not
  a `roymctl` function. `roymctl` builds the client with the DHT enabled,
  matching `identity publish-anchor`:

  ```rust
  RegistryClient::new(true, Some(url.clone())).refresh_master_anchor(master).await?;
  ```

- **Call it once per master, at the call sites — not inside
  `certify_instance`.** `substitute_and_certify_members` calls
  `certify_instance` in a loop, so putting the refresh inside the callee would
  publish each master's anchor twice per `app deploy`, widening the race
  D-A1-7 already acknowledges for no benefit. The three call sites are:
  `identity certify-instance`'s handler, `svc deploy`'s `--master` arm, and
  `substitute_and_certify_members`' loop over resolved masters — one refresh
  per master in each.
- When no registry URL was supplied, print a warning naming the consequence:

  ```
  No --registry-url given, so no master anchor was published for <did>.
  Any connection presenting this certificate will be rejected until one is
  (`roymctl identity publish-anchor`).
  ```

---

## 4. Ordering, semantics, and interaction notes

1. **Phase 1 before phase 2, phase 2 before phase 4.** `verify` needs
   `verify_chain`; `EndpointPublisher` needs `sign_as_instance`.
2. **Phases 1-3 change no behavior.** `EndpointInfo.delegation` is set nowhere
   in the tree at `f50febd` (A0 established this), so the new branches are
   unreachable until phase 4 wires a producer. That makes them independently
   mergeable and independently revertible.
2b. **`SignedEndpointInfo::verify`'s compile surface is exactly two**, both
   production: `registry.rs:239` and `dht_registry.rs:261`. No test in the tree
   calls it. The three `signed.verify()` calls in `dht_registry.rs`'s own test
   module (`:590`, `:594`, `:606`) belong to a different type —
   `SignedMasterAnchor::verify`, reached through `MasterAnchorPayload::sign` —
   so `RecordTrust` never reaches them. They keep compiling for a second and
   separate reason: A1 *does* touch that type (D-A1-12 splits its `verify`,
   D-A1-13 rewrites the comparison inside it), but the split leaves `verify`'s
   own signature unchanged and factors the new `verify_signature` out beneath
   it. Both halves are worth stating: the two verifiers read identically at a
   glance, and "A1 does not touch anchors" would be the wrong thing for a
   future reader to conclude from an unchanged call site.
3. **`RegistryClient::lookup`'s DHT branch is deliberately untouched.** It
   never calls `verify` — it relies on pkarr's own signature check against the
   queried key — and by D-A1-2 it can never return a delegation-signed record
   for a master DID anyway (the record's `service_id` would not equal the
   queried id). Adding a `verify` call there would be dead code that reads like
   a safety property.
4. **Resolution stays at one lookup.** `resolve_iroh_addr` is unchanged; a
   master DID now hits a record the same way a self-signed one does. The
   `resolve = true` second hop to the substrate record is the pre-existing
   Service-record indirection, not something A1 adds. §5.7 measures this rather
   than asserting it.
5. **No WIT change.** A1 touches no interface a guest imports, so
   `wasm32-wasip2` fixtures rebuild unchanged. (Carrying nickname/privacy in
   the manifest *would* be a WIT change; deliberately not taken — D-A1-4.)
6. **No storage-schema change.** The instance-certificate table A0 added is the
   only persistence A1 reads.
7. **`is_private` still governs parent propagation** unchanged
   ([registry.rs:225-229](../../../../crates/community_registry/src/registry.rs#L225)),
   and D-A1-4 carries the operator's choice through from the stored record.
8. **Packet size is not at risk.** A service record has empty `mechanisms`, and
   `TXT::try_from` chunks at 255 bytes, so the added certificate stays well
   inside the 1000-byte pkarr limit the substrate record's address pruning
   exists for.

---

## 5. Tests

**Superseded by the fifth pass as a test list — the tests below exercised the
design D-A1-2's reversal replaced, and most no longer exist under those
names.** status.md's evidence section has the actual, `git diff`-verified
test count and names for what shipped; that is the source of record, kept in
sync at merge time rather than duplicated here. Kept below for what test
*shapes* mattered and why, which still transfers: whole-record tamper
checks, the anchor's stale/wrong-master/tamper regression tests (D-A1-12,
D-A1-13, all still present), and a live-registry sweep test proving
per-service failure containment all still exist under the new design, just
against a smaller decision table with no certificate to route around.

### 5.1 Unit — `crates/core/src/dht_registry.rs`

1. `a_record_keyed_by_its_master_verifies_when_signed_by_a_certified_instance_key`
2. `a_record_keyed_by_a_master_is_rejected_when_signed_by_an_uncertified_key`
   — failure-matrix **row 4**. `delegation: None`, signed with an instance key.
3. `a_record_whose_certificate_names_a_different_master_is_rejected`
4. `a_record_carrying_a_routing_scoped_certificate_is_rejected`
   — failure-matrix **row 2**, narrow half.
5. `a_self_signed_record_still_verifies` — regression for the `None` path.
6. `rewriting_the_substrate_id_after_signing_is_rejected` — D-A1-9. Sign,
   mutate `signed.info.substrate_id`, assert `verify` errors.
7. `rewriting_the_delegation_after_signing_is_rejected` — D-A1-9, the field
   A1 itself adds to the unauthenticated set if left uncompared.
8. `sign_as_instance_rejects_a_key_the_certificate_does_not_name`
9. `a_delegation_signed_record_cannot_be_registered_without_an_http_registry`
   — D-A1-2's early return, via `RegistryClient::new(true, None)`.
10. `an_expired_certificate_blocks_publishing_but_not_reading` — **D-A1-10, and
    the evidence for task.md's corrected failure-matrix row 3.** One record,
    one expired certificate: `verify(Publishing)` errors,
    `verify(Reading)` is `Ok`.

### 5.2 Unit — `crates/core/src/dht_registry.rs`, anchors

A **backdated anchor helper** makes the stale path directly testable, so
D-A1-12 does not have to rest on a structural argument.
`MasterAnchorPayload::sign` only ever uses `Timestamp::now()`, but
`ntimestamp` (pkarr 6.0's `Timestamp`) has `impl From<u64>`, and
`SignedPacket::new` takes the timestamp as an argument — so a test can mirror
`sign`'s body with `now_micros - 25h` and produce a genuinely stale, genuinely
signed anchor.

11. `a_stale_anchor_still_passes_verify_signature_and_still_fails_verify`
    — the split D-A1-12 rests on, in one assertion pair.
12. `an_anchor_for_a_different_master_is_rejected`
    — D-A1-12's tightening, against both `fetch_own_master_anchor` and
    `resolve_master_anchor`.
13. `stripping_a_revoked_key_after_signing_is_rejected` — **D-A1-13**, and the
    one that matters most: sign an anchor revoking a key, remove it from
    `signed.payload.revoked_keys`, assert `verify_signature` errors. This is
    the tamper that would otherwise be laundered into a real signature by the
    next refresh.
14. `adding_a_revoked_key_after_signing_is_rejected` — D-A1-13, the
    denial-of-service direction.
15. `rewriting_the_revoke_list_registry_after_signing_is_rejected` — D-A1-13,
    the third stateful field.

### 5.3 Unit — `crates/community_registry/src/registry.rs`

16. `a_delegation_signed_record_registers_and_looks_up_under_its_master_did`
17. `a_record_signed_by_a_revoked_instance_key_is_rejected_at_admission`
    — D-A1-6: register a master anchor listing the instance DID in
    `revoked_keys`, then attempt the record; expect `401`.
18. `a_delegated_records_alias_is_derived_from_its_master_did`
    — guards against the alias silently keying on the signer.
19. `refreshing_a_master_anchor_keeps_its_revocations_and_its_revoke_list_registry`
    — **D-A1-7's regression guard**, and the test that stops failure-matrix
    row 14 from being undone. Lives here rather than in `roymctl`: this crate
    already depends on `syneroym-core` and already builds registry state in
    its own tests, so it needs no new dev-dependency. Bind an
    `EcosystemRegistry` on an ephemeral port, publish an anchor carrying both
    a revoked key and a `revoke_list_registry`, call
    `RegistryClient::refresh_master_anchor`, resolve, assert both survived.
20. `refreshing_a_stale_master_anchor_keeps_its_revocations`
    — **D-A1-12's regression guard, and the one that actually matters**, since
    a late renewal is the common case rather than the edge. Seed the registry
    with a backdated anchor carrying a revocation (§5.2's helper), refresh,
    assert the revocation survived and the republished anchor is fresh.
21. `refreshing_refuses_to_overwrite_an_anchor_it_cannot_read`
    — insert a corrupted `SignedMasterAnchor` into `RegistryState` directly,
    assert `refresh_master_anchor` errors and the stored anchor is untouched.

### 5.4 Unit — `crates/core/src/endpoint_publisher.rs`

Testing `build_record` directly, so no registry is needed.

22. `a_service_with_an_installed_certificate_publishes_under_its_master_did`
    — assert `service_id == master_did`, `substrate_id == node_did`,
    `delegation.temporary_did == derived instance did`, and
    `verify(Publishing)` is `Ok`.
23. `an_expired_certificate_publishes_nothing`
24. `a_service_without_a_certificate_replays_its_stored_record_verbatim`
25. `the_published_record_keeps_the_stored_records_nickname_and_privacy`
26. `a_service_with_no_recorded_owner_publishes_nothing`
27. `a_certificate_naming_a_key_this_node_does_not_derive_publishes_nothing`
28. `a_stored_record_that_does_not_self_verify_still_supplies_its_metadata`
    — D-A1-8's third shape: the file is signed by an ephemeral key, and the
    published record must still carry its nickname.
29. `a_stored_record_that_no_longer_verifies_is_not_replayed`
    — the other side of test 28, on the `None`-certificate path: the same
    envelope, with the certificate gone, must be dropped rather than
    re-POSTed. (D-A1-4)
30. `the_published_record_ignores_a_stored_ttl` — D-A1-10's bound. A stored
    record carrying `ttl: Some(huge)` must not carry it through.
31. `the_sweep_covers_certified_and_stored_only_services_and_survives_one_failing`
    — **D-A1-3's recovery path.** Two certified services and one stored-only,
    one of them unpublishable; assert the id union is complete and that the
    failure does not stop the rest.

### 5.5 Unit — wiring

32. `crates/control_plane/src/service.rs`:
    `set_endpoint_publisher_is_set_once` — a second call is ignored, not a
    panic.

The publish-on-deploy *trigger* is not unit-tested: `EndpointPublisher` is a
concrete type holding a real `RegistryClient`, and introducing a trait plus a
mock for one call is more machinery than the assertion is worth. Covered end to
end in §5.7 instead. Called out here so the gap is a decision, not an oversight.

### 5.6 CLI — `apps/roymctl`

33. `deploy_with_a_master_and_a_nickname_needs_no_identity_flag` — parse level.
34. `deploy_with_an_instance_certificate_and_a_nickname_needs_no_identity_flag`
    — D-A1-8's third shape, parse level, and the case that fails without an
    ephemeral fallback because there is no `--as`.

D-A1-7's and D-A1-12's regression guards are **tests 19-21 in
`crates/community_registry`**, not here. An earlier revision put them in
`roymctl` as behavior tests, which would have needed
`syneroym-community-registry` added to a dev-dependency list holding only
`assert_cmd`, `predicates`, and `tempfile` — every `roymctl` test today drives
the binary, none run in process. Moving the helper onto `RegistryClient`
(§3.2 (e2)) put those tests somewhere they cost nothing.

### 5.7 e2e — `crates/substrate/tests/master_endpoint_record_e2e.rs` (new)

Modeled on
[instance_identity_e2e.rs](../../../../crates/substrate/tests/instance_identity_e2e.rs),
including its own `Node` (the shared `SubstrateTestContext` deadlocks with two
live nodes — that file's module doc explains why). Node A runs the community
registry; node B is configured with node A's `registry_url`.

35. `a_member_master_did_resolves_to_an_address_and_follows_the_member_across_nodes`
    - Generate a member master `M` locally; publish its anchor.
    - On node B: `orchestrator/instance-identity` → pubkey; issue a
      `SCOPE_SERVICE_INSTANCE` certificate; deploy a bare TCP service with
      `service_id = M` and that certificate.
    - **Resolve, do not just read** — this is the assertion the slice exists
      for, and the first draft omitted it. Call
      `RegistryClient::lookup(M, resolve = true)` and assert the returned
      `mechanisms` are node B's substrate mechanisms, proving D-A1-4's empty-
      `mechanisms` + `substrate_id` indirection actually works. Then call
      `net_iroh::resolve_iroh_addr(&client, M)` and assert it yields node B's
      `EndpointAddr`. Assert the record's `delegation.temporary_did` is node
      B's derived instance DID and that `verify(Publishing)` is `Ok`.
    - **Reference-scenario step 4, as a clean relocation:** `undeploy` `M` from
      node B, then deploy `M` on node A with a fresh certificate from the same
      master. Undeploying first is deliberate — leaving both deployed creates
      D-A1-11's two-publisher flap, and a test that passes only because the
      heartbeat is hourly is a test that passes for the wrong reason. Then
      re-resolve: `service_id` unchanged, `resolve_iroh_addr` now yields node
      A's address, `delegation.temporary_did` now node A's instance DID.
    - Negative, failure-matrix **row 4** over the wire: hand-build a record
      keyed by `M`, sign it with an uncertified key, POST it to node A's
      registry, assert `401`.

Run with the sandbox disabled — real port binds, per this repo's standing note.

---

## 6. Things in the ADRs / `task.md` that are stale or under-specified

1. **task.md A1: "Two verification paths, not one."** There is one.
   `verify_endpoint_signature`
   ([registry.rs:234](../../../../crates/community_registry/src/registry.rs#L234))
   *calls* `SignedEndpointInfo::verify`; the rest of its body resolves a key
   for a `debug!` and returns it to a caller that discards it. The genuinely
   separate path is `RegistryClient::lookup`'s **DHT branch**
   ([dht_registry.rs:276-299](../../../../crates/core/src/dht_registry.rs#L276)),
   which calls `verify` **not at all**. That is the path worth naming, and
   task.md does not name it.
2. **ADR-0020 §6: "keeps resolution at exactly one lookup, unchanged from
   today."** True on the HTTP registry, and **impossible on the DHT**: BEP0044
   keys a record by its signing key, so a delegation-signed record cannot be
   stored at, or resolved by, the master DID. Neither the ADR nor task.md says
   master-DID resolution requires a configured registry. It does. (D-A1-2)
3. **ADR-0020 §6: "Revocation continues to work unchanged … a record signed by
   a revoked key stops verifying."** False, before and after A1.
   `SignedEndpointInfo::verify` never consults a master anchor, and
   `DelegationCertificate::verify` checks signature, window, and scope only.
   (D-A1-6)
4. **task.md A1: "Covers every `ServiceType` with no special case."** True of
   the key derivation, false of the surface. `SyneroymClient::deploy_container`
   hardcodes `instance_certificate: None`
   ([lib.rs:616](../../../../crates/sdk/src/lib.rs#L616)) and `roymctl svc
   deploy` has no container flag at all, so a container service cannot carry a
   member master and therefore cannot get a master-keyed record. Backlog row 79
   already records this with target "A1 … or whenever container deploy gets a
   CLI surface, whichever is later" — A1 does **not** meet it, and the row
   should be retargeted rather than left reading as A1 scope.
5. **ADR-0020 §6: "the publish path attaches the certificate."** There is no
   substrate-side publish path for service endpoint records. The substrate
   replays an operator-signed file from `hosted_apps_dir` on an hourly
   heartbeat and never constructs a record. A1 builds it (D-A1-3) — the same
   category of gap A0's plan found in §1's "presents that certificate on its
   route preamble."
6. **A0 gap, currently broken: no master anchor is ever published for a member
   master.** `verify_preamble` resolves the anchor and fails closed when it is
   missing, so presenting an instance certificate on a connection is rejected
   at the destination today. A0's e2e never drives the guest arm over a real
   hop, and its router tests build preambles without a handshake, so nothing
   caught it. Compounded by `SignedMasterAnchor::verify`'s 24-hour freshness
   bound, which makes anchor publication a daily duty nobody performs. Fixed in
   A1 (D-A1-7); the cadence is A5's.
7. **task.md failure-matrix row 3 stops being true after A1.** It says a missed
   renewal is "not a distinct code behavior — the failure mode *is* row 1."
   After A1 a lapse also removes the member from name resolution, on the
   registry's TTL clock rather than the handshake's. Distinct behavior,
   distinct test (§5.1 test 10), and the row needs rewriting. (D-A1-10)
8. **ADR-0020 §6 and task.md both lean on "at most one live publisher per
   master."** Nothing enforces it, and A1 changes the failure shape from a
   one-off duplicate insert into a permanent hourly flap between two
   `substrate_id` values. (D-A1-11)
9. **Pre-existing: `SignedEndpointInfo::verify` authenticates one field out of
   the whole record.** `substrate_id` — the field a lookup follows to an
   address — is unauthenticated and rewritable by anything that relays the
   record. Fixed in A1 (D-A1-9).
10. **Pre-existing, and the same bug at the anchor:
   `SignedMasterAnchor::verify` authenticates only `timestamp`.**
   `revoked_keys` and `revoke_list_registry` in the served outer copy are never
   compared to the signed one, so anything that can answer a `/lookup_master`
   can add or strip revocations — admitting a revoked instance key, or denying
   service to a live one. Two things turn it from background debt into A1's:
   D-A1-7 is what gives member masters an anchor to tamper with in the first
   place, and D-A1-7's read-modify-write would re-sign a tampered list with
   the real master key, making the tamper permanent and authentic. Fixed in A1
   (D-A1-13).
11. **`verify_endpoint_signature` returns a `VerifyingKey` nobody uses.** Its
    only caller writes `verify_endpoint_signature(&payload)?;`. Cleaned up in
    §3.3 as a side effect of adding the state parameter.
12. **Three `RegistryClient`s per substrate**, after A1: the router's
    ([route_handler.rs:202](../../../../crates/router/src/route_handler.rs#L202)),
    the heartbeat loop's
    ([runtime.rs:635](../../../../crates/substrate/src/runtime.rs#L635)), and
    the publisher's. Each opens its own pkarr DHT client when the DHT is on.
    Not a bug and not A1's job; noted so it is not read as something A1
    introduced.

---

## 7. Deferred-backlog updates (mandatory, per AGENTS.md)

**One row below is fully resolved by the fifth pass and moved to "Recently
resolved" in [deferred-backlog.md](../../deferred-backlog.md) directly, not
narrated again here**: *Master-DID endpoint resolution requires an HTTP
registry* (D-A1-2 reversed — every record has a DHT home now). Two more rows
are updated rather than resolved or closed, applied directly in
deferred-backlog.md: *Two nodes can publish the same member master
indefinitely* narrows — D-A1-14's compare-and-swap closes the flap's
symmetric form (the losing publisher's replayed record is now permanently
rejected everywhere once a newer one lands), but the losing publisher still
does not know it has lost and needs the same placement view this row already
named to actually stop trying. *A retired member's endpoint record lives
until its TTL* is genuinely **unresolved** by this pass and stays open
verbatim: `undeploy` deletes only the local stored file
([orchestration.rs:965-972](../../../../crates/control_plane/src/service/orchestration.rs#L965));
nothing publishes a superseding (or tombstoning) record to the registry or
DHT on undeploy, so the mechanism that *would* let it be removed immediately
exists (D-A1-14's monotonic timestamp) but nothing calls it yet. *Endpoint
records are not revocation-checked at resolution* is reworded rather than
resolved: with no certificate on a
record, there is nothing there to revocation-check in the first place, so the
row's remaining content is purely "the handshake is the actual gate," which
was already true.

**Move to "Recently resolved":**

- Row 74 (*The DHT endpoint-record path has its own delegation check…*).
  Resolution: there is one verification function; A1 gave it ADR-0020 §6's
  keying (record keyed by the master, signed by the instance key), replacing
  A0's inverse placeholder check.

**New rows, §3 *Access control, identity & security*:**

- **Master-DID endpoint resolution requires an HTTP registry.** BEP0044/pkarr
  keys a record by its signing key, so a delegation-signed record has no DHT
  home under its master DID. DHT-only deployments cannot resolve a member
  master. Target: TBD — the fix is either a forward index in the anchor
  (rejected by ADR-0020 §6 on its two-hop cost) or a registry that is not
  content-keyed. Source: D-A1-2.
- **Endpoint records are not revocation-checked at resolution.** A1 checks at
  admission, registry-locally and best-effort; the enforcing gate stays the
  handshake. A revoked instance key can therefore keep a stale record alive
  while being unable to answer a call. Corrects ADR-0020 §6's claim.
  Target: TBD. Source: D-A1-6.
- **No operator surface for revoking an instance key.** Nothing in the tree can
  *add* to a master anchor's `revoked_keys`: `roymctl identity publish-anchor`
  hardcodes `vec![]`, and the only non-empty publisher is a test. A1 stops
  renewal from clearing the list (D-A1-7) but adds no way to populate it, so
  failure-matrix row 14's revocation is a mechanism with no way to trigger it
  outside tests. Target: **A5** (with master-key custody). Source: D-A1-7.
- **Master-anchor refresh is a read-modify-write with a race, and a daily
  operator duty.** Two concurrent renewals for the same master can each read
  the same prior state and drop a revocation added between them. A1 closes the
  worse half — a *late* refresh no longer wipes the anchor it could not read
  as fresh (D-A1-12) — but the concurrent case needs a compare-and-set the
  registry does not offer, or a single writer. Also still a duty nothing
  performs on a schedule: an anchor older than 24 hours stops verifying.
  Target: **A5** (online-key posture). Source: D-A1-7, D-A1-12.
- **`master_anchors` are never expired by the registry's sweep.** The 15-minute
  sweep walks `endpoints` only
  ([registry.rs:149-159](../../../../crates/community_registry/src/registry.rs#L149)),
  so a stale anchor is served — and fails verification at every consumer —
  until something overwrites it. A1 makes a refresh able to overwrite one
  correctly; it does not make the registry drop one nobody refreshes.
  Target: TBD. Source: D-A1-12.
- **Two nodes can publish the same member master indefinitely.** After A1 every
  node with an installed certificate republishes hourly, so a relocation that
  leaves the old instance deployed produces a permanent flap between two
  `substrate_id` values rather than one last-writer-wins insert. Needs a
  publisher generation stamp or the supervisor's own placement view. Target:
  **A3/A5**. Source: D-A1-11.
- **A retired member's endpoint record lives until its TTL.** The HTTP registry
  has no unregister endpoint, so `undeploy` cannot remove a record. Pre-existing;
  more visible once relocation is real. Target: TBD.

**New row, §10 *Product surfaces & UX*:**

- **Endpoint-record metadata (nickname, privacy) travels inside a signed record
  it no longer needs.** With an instance certificate installed, the stored
  `registry_certificate` blob exists only to carry two operator-chosen fields,
  and on the `--instance-certificate` path it is signed by a throwaway key and
  cannot self-verify. The clean shape is `DeployManifest` fields. Not taken in
  A1, and the cost is three parts, not one: a WIT record change, ~40
  `DeployManifest` literals, **and** a new per-service persistence sidecar —
  the publisher reads this at heartbeat time, and a manifest is not stored per
  service, so `hosted_apps/<id>.json` is currently doing that job. Target: TBD.
  Source: D-A1-4/D-A1-8.

**Amend:**

- Row 79 (*Container/podman deploys cannot carry a member master*): retarget
  off A1. A1 does not deliver it; it needs a container deploy CLI surface
  first. (§6 item 4)

---

## 8. Completion checklist

- [ ] `cargo +nightly fmt --all`
- [ ] `cargo clippy --workspace --all-targets --all-features` — clean
- [ ] `cargo test --workspace` — green except the documented environmental
      socket-bind failures; diff against unmodified `main` run the same way
- [ ] `mise run test:e2e` — sandbox disabled, 12/12
- [ ] New e2e test verified individually, sandbox disabled, twice
- [ ] `wasm32-wasip2` fixtures build (expected unchanged — no WIT edit)
- [ ] Import cleanup pass over every edited file
- [ ] ADR-0020 amendment extended with §6 items 2, 3, 5, 6, 7, 8
- [ ] task.md: failure-matrix row 4 gains its A1 evidence; **row 3 rewritten**
      per D-A1-10; the "two verification paths" bullet corrected
- [ ] `docs/planning/traceability-matrix.md` **row 47** (`[FND-IDT]`, stable
      service identity): move "endpoint records published under the master DID
      (A1)" from *Not yet delivered* into the delivered list with evidence;
      the row stays open until A2 lands
- [ ] `docs/planning/traceability-matrix.md` **row 50** (`[FND-IAM]`): record
      that A1 discharges M04A B7's registry-trust-model ADR debt
- [ ] status.md: A1 marked complete with evidence; slice table updated
- [ ] deferred-backlog.md updated per §7

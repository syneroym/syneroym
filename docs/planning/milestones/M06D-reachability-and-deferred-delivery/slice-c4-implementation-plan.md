# M06D slice C4 — Mailbox service: implementation plan

*Reader: the developer (or AI) who implements slice C4. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  deferred backlog, no planning references in code (no "M06D", "C4", "C5",
  "M-12", "D-C4-n" in code, comments or test names; ADR ids are fine),
  simple English in comments, the 800-line file cap, the 100-line function
  cap, short match arms, the module layout rule (`foo.rs` beside `foo/`,
  never `mod.rs` under `src/`), and no hard-coded ports in substrate tests.
  **This plan names its decisions `C4-1` to `C4-18`. Those ids are for this
  plan only. When a step says "for the reason in C4-n", write the reason
  itself in the comment, never the id.**
- `task.md` in this folder: §1, §3 slice C4, and the slices that use C4
  (C5, C6, C8).
- `docs/planning/changes/service-mailbox/change.md`: criteria M-1 to M-14
  and K-4; Design sections "What the mailbox sees", "Service record and its
  life", "Mailbox methods", "Store", "Limits", "Dedicated mailbox and
  passport"; `## Deviations` (`D-C2-1` to `D-C2-12`, `D-C3-8`, `D-C3-10`).
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) Decisions
  1 to 3, 7, 8 and amendment A1;
  [ADR-0016](../../../decisions/0016-native-dispatch-identity-threading.md)
  §2, §3, §6 (`CallerContext`, caller proof);
  [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1
  and §6 (instance certificate, records replayed verbatim).
- `slice-c2-implementation-plan.md` §5 (what C4 takes from C2: `Deposit`,
  `Deposit::check`, `DepositId`, `Receipt`,
  `Receipt::unsigned_rejection(RejectCode::TooManyClaims, ..)`).
- The code this slice builds on (read first):
  - `crates/mailbox_format/src/{lib.rs,deposit.rs,receipt.rs,error.rs}`.
  - `crates/rpc/src/native.rs` (`CallerContext`, `CallerProof`,
    `AuthLevel`, `NativeService`), `crates/rpc/src/lib.rs:66-97`
    (`RpcError`).
  - `crates/router/src/route_handler/io.rs:158-260` (`build_caller`: the
    caller is the certificate's master DID, and `proof.delegation_json` is
    the verified certificate), `crates/router/src/handshake.rs:25-83`
    (`verify_preamble`: certificate, scope, key match, revocation).
  - `crates/router/src/call_dedup.rs:170-190` (`is_node_level_interface`).
  - `crates/router/src/proxy/router.rs:213-260` (`invoke_local` passes
    `req.caller` unchanged to a native service).
  - `crates/router/src/peer_records.rs` (the SQLite store pattern to copy:
    `Mutex<Connection>`, WAL, `to_sql`).
  - `crates/core/src/dht_registry/{types.rs,client.rs}` (`EndpointInfo`,
    `EndpointMailbox`, `SignedEndpointInfo::verify`,
    `RegistryClient::register`).
  - `crates/community_registry/src/registry.rs:259-286` (`admit_endpoint`:
    a byte-identical record with the same timestamp is a refresh; an older
    timestamp is refused).
  - `crates/passport/src/limiter.rs` (`RequestLimiter`).
  - `crates/core/src/config/roles.rs` (`RolesConfig`, `SupervisorRole`).
  - `crates/control_plane/src/service.rs:60-71` and
    `crates/control_plane/src/service/orchestration/deploy/admission.rs:24-38`
    (reserved service ids).
  - `crates/substrate/src/runtime/{router.rs,services.rs,supervisor.rs}`,
    `crates/substrate/src/runtime.rs:120-157`.
  - `crates/substrate/tests/common/node.rs` (`SubstrateNode` builder,
    `configure`, `client_as`, `restart_in_place`) and
    `crates/substrate/tests/mailbox_record_e2e.rs` (record helpers).
  - `crates/sdk/src/client.rs:60-140, 300-330, 479-530`.

**Branch.** Work on `feat/m06d-c4-implementation`. It was cut from `main`
after C3 merged (#311) and holds only this plan (and the value-exchange
change doc commit `04b81abd`, which C4 does not touch).

## 1. What C4 builds, in one paragraph

A substrate with `[roles.mailbox]` runs a **mailbox**: a native service on
interface `mailbox` under the node's own DID (ADR-0026 Decision 2). A
receiving service instance **registers** a queue with its signed service
record. Any verified sender **deposits** a sealed message for that queue.
The newest instance of the receiver **claims** messages, which hides them
for a visibility time, and **acks** them with a receipt it made (signed
`delivered`, or a rejection). The sender reads the receipt with **status**.
The mailbox stores everything in one small SQLite file. It keeps limits for
each queue and for the whole node, removes messages fairly when a queue is
full, closes a message after 5 claims, removes expired messages, old
receipts and idle queues, and republishes the stored service record to its
registry every hour, so a sleeping receiver stays findable (K-4). The
mailbox never sees message content: the inner part is sealed for the
receiver (C2). Nobody uses the mailbox yet: the sender path is C5, the pull
worker is C6, passports are C8, and the registry list and `auto` are C7.

## 2. Decisions this plan makes (check them before you start)

Each fills a gap in task.md or the change doc (the gaps are listed in §6).
The owner confirmed C4-4 (Option A) on 2026-10-18 (§6 item 1). A review
on the same day changed C4-2, C4-3, C4-8, C4-9 and C4-16, decided §6 item
2, and added §6 items 15 to 17 and the C7 note in §7.

| # | Decision | Why |
| --- | --- | --- |
| C4-1 | **Crate homes.** New crate `crates/mailbox` (`syneroym-mailbox`): the native service, the store and the background worker. The **wire types, method names, the interface name and the error codes** go in `syneroym-core`, new module `crates/core/src/mailbox.rs`. The role config goes in `crates/core/src/config/mailbox.rs`. | C5 (in `crates/router`) and C6 (in `crates/substrate`) must build requests, read answers and classify errors. They must not link the store (the same reason C2-1 kept the formats out of this crate). `core` already depends on `syneroym-mailbox-format` (C3) and has `SignedEndpointInfo`, so the wire types can use both. `roles.rs` is 561 lines; a sibling file keeps it small. |
| C4-2 | **Registration.** At boot, when `[roles.mailbox]` is set: `endpoint_registry.register(node_did, "mailbox", NativeHostChannel { service_id: "mailbox" })` and `native_dispatch.insert("mailbox", service)`. New `pub const MAILBOX_RESERVED_SERVICE_ID: &str = syneroym_core::mailbox::MAILBOX_INTERFACE;` in `crates/control_plane/src/service.rs`; deploy admission refuses it; `call_dedup::is_node_level_interface` also matches it. When the role is **not** set and a stored `(node_did, "mailbox")` endpoint exists from an earlier boot, remove it (`EndpointRegistry::remove`). New substrate Cargo feature `mailbox`, in `default`. `mailbox` is **not** added to `NODE_NATIVE_INTERFACES`, but `EndpointRegistry::resolve_interface` leaves `mailbox` out when the caller names no interface (Step 3.4). | Same shape as the supervisor (`runtime/supervisor.rs`, `router.rs:251-285`). The `"mailbox"` dispatch key must be reserved, or a deploy under that name takes over every mailbox call. A keyed call (one with an `idempotency_key`) aimed at a node-level interface must be refused, or it mints a DEK and a database for a service that does not exist (`call_dedup.rs:170-190`). The registry persists endpoints, so a node that drops the role would otherwise keep routing `mailbox` calls to a dispatch key nobody serves. `NODE_NATIVE_INTERFACES` blocks guest proxy calls to owner-only interfaces; the mailbox is open to every verified caller by design, so it does not belong there. But a call with no interface picks "the one interface" of the target, with only `NATIVE_CAPABILITY_INTERFACES` and `NODE_NATIVE_INTERFACES` left out. On a node without the supervisor role, `mailbox` would become that one interface of the node DID, and a call with no interface would reach the mailbox. With the supervisor role, the node DID would have two, so a call with no interface that reaches the supervisor today would be refused as ambiguous. Leaving `mailbox` out of that one choice changes neither case. The supervisor has the same issue on its own; C4 does not change it (backlog, §5 item 8). |
| C4-3 | **Who the caller is.** Receiver methods (`register`, `unregister`, `claim`, `ack`) need an **instance caller**: `auth` is `Delegated` or `Ucan`; `proof.delegation_json` is present and parses; its `scope` is `service-instance`; its `master_did` equals `caller.caller_did`; the DID of `proof.pubkey_hex` equals its `temporary_did`; `verify_at(master, [service-instance], now)` passes; the master anchor does not list the instance DID in `revoked_keys` (C4-8). Sender methods (`deposit`, `status`) need `auth` `Delegated` or `Ucan`; the sender is `caller.caller_did`. `info` admits any caller that dispatch admits. **No request body carries a certificate.** | M-2 and M-3. The router already verified the certificate and put it, verbatim, in `CallerContext.proof` (`build_caller`). Checking scope is needed: the transport also admits `routing` (person-device) certificates (`TRANSPORT_SCOPES`). The other checks, revocation included, repeat the router's on purpose: a same-node call (`ProxyRouter::invoke_local`) passes `req.caller` with no handshake, so the mailbox must not assume one ran. A `System` or `LocalElevated` caller (a local guest's `service_system`) is refused. The change doc says `register` "carries the whole instance certificate in its body, because the router keeps only the two DIDs"; that is stale (§6 item 2). |
| C4-4 | **C4 relies on the transport identity, although the router does not check that the caller holds the instance key.** No extra proof is added in C4. The backlog row "Router does not check that the caller holds the temporary key" gets the mailbox consequences (§5). Confirmed by the owner on 2026-10-18. | The router checks the certificate but not possession of its key (`handshake.rs`; backlog). Instance certificates are not secret: a sender gets the receiver's certificate in every `delivered` receipt, and an open mailbox lets anyone deposit. So a stranger can call `register`, `unregister`, `claim` and `ack` as a receiver it has received a receipt from, and can deposit as a sender whose certificate it holds. **The risk is denial of service, not loss of secrecy or of messages.** Bodies stay sealed. A fake `delivered` receipt does not verify at the sender (A1). An `unknown` or `expired` receipt sends the sender back to direct attempts with the payload kept (S-5). But an attacker who calls `unregister` again and again turns the mailbox off for one chosen receiver; and deposits made in a sender's name make fair eviction remove that sender's real messages. The proper fix is in the router, and it helps every native interface. **Rejected alternative:** a per-request signature by the instance key on the receiver methods (`{domain, mailbox did, method, master did, at_secs, params hash}`, ±5 minutes; `deposit` too, to stop sender spoofing). It closes the gap for the mailbox only, adds a format C5 and C6 must also sign, and would be removed when the router fix lands. |
| C4-5 | **Store.** One SQLite file `<storage.db_dir>/<db_name>` (default `mailbox.db`), WAL, **plaintext**, three tables (§3 Step 4.6). Each queue row keeps its own `bytes` and `messages` counters and its own limits. Every removal of a message goes through **one** helper (`remove_message`), which writes the receipt and updates the counters in the same transaction. | The change doc asks to use the encrypted open helper "if it costs little". It needs the KEK, which arrives after boot by `inject-kek`; a mailbox must serve from boot. Bodies are already ciphertext; the metadata (who writes to whom) is the part left in clear, which the backlog records (§5). Per-queue counters make the global total a sum over at most `max_queues` rows, not over all messages. One removal helper is how the counters cannot drift. Limits per queue are where C8 puts the raised quotas. No `async_queue`: the change doc's reason holds (attempts and backoff are unused; the work is metadata queries). |
| C4-6 | **Deposits and receipts are keyed on `(sender, deposit_id)`.** A repeat with the same key and byte-identical `sealed` and the same destination answers `stored` with the first time (a safe retry). A repeat with other content, or one whose key already has a receipt with no message (the message was delivered, rejected or removed), is refused with `duplicate_deposit`. Another sender may use the same `deposit_id`. | task.md C4. The sender picks the id, so keying on the id alone lets one sender block another's id. A sealed deposit is used once (`D-C2-9`), so a second, different deposit under an old id is a bug or an attack. |
| C4-7 | **Fair eviction (M-8).** While the new message does not fit (`messages + 1 > max_messages` or `bytes + size > max_bytes`): take the sender with the most bytes **in this queue** (ties: the smaller sender DID text); if it is the depositor, refuse with `queue_full`; else remove that sender's oldest message (lowest `seq`, claimed or not) with the receipt `expired`. **Global cap:** after eviction, if the node total plus the new size is above `max_total_bytes`, refuse with `mailbox_full`; the transaction rolls back, so nothing was evicted. No eviction across queues. | M-8 names the queue rule but no receipt for an evicted message. `expired` needs no new state or code, and the sender then does what S-5 says for `expired`: back to direct attempts. A new reject code would also work (an unknown code is treated like `expired`), but adds a wire string for no gain. The global cap has no stated rule; refusing is the simple, safe choice (§6 item 5). |
| C4-8 | **Newest certificate (M-12).** A queue keeps `(newest_issued_at, newest_instance_did)`. An instance caller is admitted when its `instance_did` equals the stored one (its `issued_at` replaces the stored one if newer), or when its `issued_at_secs` is **strictly** greater (it becomes the newest). Otherwise `stale_instance`. `register`, `unregister`, `claim` and `ack` all apply the rule. **Revocation is checked by the mailbox too:** before the rule, each of these four methods reads the master's anchor (`MasterAnchorResolver::resolve_master_anchor`, 5-second timeout, the same call and bound as `verify_preamble`) and refuses an instance DID listed in `revoked_keys` with `not_an_instance`. An anchor that cannot be read refuses the call with `-32603` (the receiver tries again at its next pull). | Same instance key = same instance, so a renewed certificate (newer `issued_at`) or a call still in flight with the previous one both work. Two instances whose certificates have the same `issued_at` second cannot both win; the first one seen keeps the queue. Revocation: the router refuses a revoked instance key at every new stream, but a same-node call (C6 receiver on the mailbox host) runs no handshake (C4-3). M-12 admits the newest certificate "only if that certificate is not revoked", so the mailbox cannot leave this to the transport. A revoked newest instance is refused, and older instances are stale: the queue waits for a newer certificate, which is what M-12 asks. Cost: one anchor read per receiver call. A wire call already pays one in the router, so this doubles it for receivers only. There is no cache: a cache would let a revoked key work until its entry ends. |
| C4-9 | **`register` (M-3, K-4).** Params `{ record: SignedEndpointInfo }`. Checks, cheap ones first: `record.info.service_id == instance.master_did` (M-3); `endpoint_type == Service`; not `is_private`; `info.mailbox` is `Some` and its `service_id` is **this node's DID**; the record JSON is at most 4 KiB; `record.verify()` passes (signature and `not_after`). An expired record is refused also for an existing queue; the queue, its record and its messages stay, and `claim` keeps working. A new queue is refused with `mailbox_full` when `max_queues` queues exist. For an existing queue, the stored record is replaced only by one with a **newer** pkarr timestamp; an older or equal one is kept silently (no error). A new or replaced record is due for publishing now, and the worker is woken. No `passport` field yet; an unknown field is ignored (C8 adds it). Answer `{ limits, queue_ends_at_secs }`. | A record that does not name this mailbox would make it a free republisher for any record. A private record must never leave its registry (`D-C3-4`). The registry's own rule is newer-timestamp-wins (`admit_endpoint`); the mailbox keeps the same rule, so it never republishes an older copy than it has. Answering instead of refusing an older record keeps a slow receiver working. The receiver needs `limits.max_ttl_secs` for `OpenPolicy` and for its idempotency memory (C6). An expired record cannot be republished (`records_due` skips it, and the registry would refuse it), so keeping it would only hide the problem; only the master holder can sign a new one (by a new deploy). C6 handles the refusal (§7). |
| C4-10 | **`claim` and `ack` shapes.** `claim { max? }` returns up to `min(max, 32)` visible messages of the caller's own queue, oldest first, at most 4 MiB of sealed bytes (always at least one message), each `{ sender, deposit_id, sealed, deposited_at_secs, expires_at_secs, claim_count }`, plus `visible_again_at_secs`. `ack { acks: [{ sender, deposit_id, receipt }] }` (at most 32) answers one outcome per entry, in order: `stored`, `not_found` or `refused`. `refused`: the receipt state is not `delivered` or `rejected`, or its JSON is over 4 KiB. A `stored` ack removes the body and stores the receipt **as it came**. Claim and ack work only on the caller's own queue: the queue id is `instance.master_did`, never a parameter. | M-5 holds by construction. Ids are unique per sender only (C4-6), so an ack must name the sender. The mailbox cannot check a receipt's proof: it does not know the message hash (task.md C4, `D-C2-7`). The size bound stops a receiver from storing junk receipts. The batch bounds keep one answer far below the 16 MiB frame limit (`rpc/src/framing.rs`). |
| C4-11 | **Closing after 5 claims (M-14).** `MAX_CLAIMS = 5`. A message whose `claim_count >= 5` and whose visibility time has ended is closed with `Receipt::unsigned_rejection(RejectCode::TooManyClaims, now)`, by the next `claim` of its queue or by the sweep, whichever runs first. A claim never returns such a message. | "Claimed 5 times without an acknowledgement" ends when the 5th visibility time ends: the receiver may still ack during it. |
| C4-12 | **Queue lifetime (M-10).** A queue's lifetime is counted from its last `register` **or** `claim` (`last_active_at`). `queue_ends_at_secs = last_active_at + queue_lifetime_secs`. A removed queue closes its messages with `expired`. | The change doc says "not claimed for 30 days". A queue that was just registered has never been claimed, and must not be removed at once; counting `register` too is the smallest fix. A receiver claims on every pull (60 s, R-1), so in practice the claim keeps it alive. |
| C4-13 | **Sweeps and republish: one worker loop** (`MailboxWorker::run`), ticking every `sweep_interval_secs` (60) and also woken by a new record. Each tick: (1) sweep (expired messages → `expired`, M-7; over-claimed messages → `too_many_claims`, M-14; idle queues removed, M-10; receipts past their keep time deleted, M-9), at most 1,000 rows per kind per tick; (2) republish due records (`next_publish_at <= now`, `record_not_after > now`, at most 100 per tick) with `RegistryClient::register(&record, false)`; on success the next is `now + republish_interval_secs` (3,600), on failure `now + 300`; (3) set the gauges. The registry client is the host's own (`substrate.registry_url`, `enable_bep0044_dht`); without one, the mailbox logs one warning at boot and never publishes. | K-4 says "publishes that record to the registry every hour". The only registry the host knows is its own; a receiver in another registry community is not helped (§6 item 6). A failed publish retried after 5 minutes keeps the record alive well inside the registry's 2-hour lifetime. Bounded batches keep the store lock short. One loop is simpler than two timers and both jobs are light. |
| C4-14 | **Deposit rate limits.** Two fixed one-minute windows: per sender DID (`deposits_per_minute_per_sender`, default 60) and for the whole node (`deposits_per_minute_total`, default 6,000). `RequestLimiter` in `crates/passport` becomes generic over its key (`RequestLimiter<K = IpAddr>`); the mailbox uses `RequestLimiter<String>` and `RequestLimiter<()>`. Refusal: `rate_limited`. | The change doc asks for "deposit rate limits for each node", but a native service sees only the caller's DIDs, not the sending node (§6 item 3). A sender DID costs nothing to make, so the node-wide window is the real limit (the change doc says so). Reusing the limiter avoids a second copy (the duplication gate would flag one). |
| C4-15 | **Config `[roles.mailbox]`** (`MailboxRole`, all fields with defaults, §3 Step 2.1). Checked once at startup by `Limits::from_role` in the mailbox crate; a bad value stops the boot with a message that names the field. The sealed-size limit is not a setting: it is `MAX_SEALED_BYTES` of the format. `max_ttl_secs` may not exceed `OPEN_MAX_TTL_SECS` (7 days). | The change doc calls the values "first values to tune", so they are settings. Tests need small values (short visibility, small queues). A ttl above 7 days is the dedicated-mailbox case (C8, from the passport facts), not an open-mailbox setting. |
| C4-16 | **Error codes** `-32040` to `-32050` (§3 Step 2.2), each with `data: { "kind": "<snake_case name>" }`. `ttl_too_long` (`-32050`) also carries `data.max_ttl_secs`, the limit of **that queue**. Bad JSON or a bad field (also a zero ttl, a destination that is not a `did:key`, a sealed message under the minimum) is `-32602`; a store failure or an unreadable master anchor is `-32603`. | M-4 asks for one error for each refusal. The range is free (`git grep -E '\-320(4[0-9]\|50)\b'` finds nothing). The `kind` lets C5 tell "queue full" from "unknown destination" without parsing text. A ttl over the limit needs its own code: today every queue has the open limit, which `info` reports, but C8 gives queues their own limits, and `info` cannot report those. With the limit in the refusal, C5 can tell this refusal from bad input and deposit again with a ttl that fits. |
| C4-17 | **SDK:** `SyneroymClient::with_delegation(cert)` puts the certificate on every JSON-RPC request preamble the client opens (like `with_ucan`). | The e2e tests must call the mailbox as a service instance, and the SDK has no way to send a delegation on a JSON-RPC request today (only `passthrough` takes one). It is small and also what a future operator CLI would use. |
| C4-18 | **`setup_connection_router` returns a struct** `RouterSetup` with named fields, in place of the 5-tuple (and `setup_router` likewise, in place of its 6-tuple), with a new field `mailbox: Option<MailboxParts>`. | A 6th tuple element would be hard to read. The unused `_publisher` element is dropped (the control plane holds its own `Arc`). |

## 3. Step-by-step changes

Do the steps in order. Each step names every file and call site. Counts
were taken at `04b81abd`; re-run the given `grep` before you edit, in case
`main` moved. The grep is the source of truth.

### Step 1 — generic `RequestLimiter` (`crates/passport`)

`crates/passport/src/limiter.rs`:

```rust
use std::hash::Hash;

#[derive(Debug)]
pub struct RequestLimiter<K = IpAddr> {
    per_window: u32,
    max_sources: usize,
    state: Mutex<State<K>>,
}

#[derive(Debug)]
struct State<K> {
    windows: HashMap<K, Window>,
    order: VecDeque<(K, u64)>,
}
// `impl<K> Default for State<K>` by hand (derive would need `K: Default`).

impl Default for RequestLimiter<IpAddr> { /* unchanged values */ }

impl RequestLimiter<IpAddr> {
    /// Count one request from `source` (IPv6 counted per /64, as before).
    pub fn allow(&self, source: IpAddr, now_secs: u64) -> bool {
        self.allow_key(source_key(source), now_secs)
    }
}

impl<K: Hash + Eq + Clone> RequestLimiter<K> {
    /// `per_window` requests per key in each 60-second window; at most
    /// `max_sources` keys are tracked.
    #[must_use]
    pub fn new(per_window: u32, max_sources: usize) -> Self { .. }

    /// The body of today's `allow`, without the address normalisation.
    pub fn allow_key(&self, key: K, now_secs: u64) -> bool { .. }
}
```

- `State::pop_oldest` and `State::drop_ended` become `impl<K: Hash + Eq + Clone> State<K>`.
- Update the type doc: it now counts any key; addresses are one use.
- `new` becomes `pub` (it was private). The two test calls
  (`limiter/tests.rs:40,52`) keep compiling.
- Call sites: `crates/coordinator_iroh/src/passport.rs:30,48,73` use
  `RequestLimiter` (default type parameter), `RequestLimiter::default()` and
  `.allow(ip, now)`; no change is expected. `git grep -n RequestLimiter`.
- New test in `limiter/tests.rs`: `a_limiter_keyed_by_text_counts_each_key_apart`
  (`RequestLimiter::<String>::new(2, 10)`: two pass for `"a"`, the third is
  refused, `"b"` still passes).

### Step 2 — config and wire types (`crates/core`)

#### 2.1 `crates/core/src/config/mailbox.rs` (new)

```rust
//! `[roles.mailbox]`: this node holds sealed messages for services that
//! cannot be reached (ADR-0026). Values are the limits of an open mailbox.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailboxRole {
    /// SQLite file under `storage.db_dir`.
    pub db_name: String,                       // "mailbox.db"
    pub max_queue_bytes: u64,                  // 5 * 1024 * 1024
    pub max_queue_messages: u32,               // 500
    pub max_ttl_secs: u64,                     // 7 * 24 * 3600
    pub receipt_keep_secs: u64,                // 14 * 24 * 3600
    pub visibility_secs: u64,                  // 300
    pub queue_lifetime_secs: u64,              // 30 * 24 * 3600
    pub max_total_bytes: u64,                  // 1024 * 1024 * 1024
    pub max_queues: u32,                       // 10_000
    pub deposits_per_minute_per_sender: u32,   // 60
    pub deposits_per_minute_total: u32,        // 6_000
    pub sweep_interval_secs: u64,              // 60
    pub republish_interval_secs: u64,          // 3_600
}
```

- A doc comment on each field (what it limits, in plain words). Write the
  defaults as `const fn default_…` or one `impl Default`, like
  `SupervisorRole` (`roles.rs:220-245`).
- `crates/core/src/config.rs`: `pub mod mailbox;` and `pub use mailbox::*;`
  beside the others.
- `crates/core/src/config/roles.rs`: in `RolesConfig` add
  ```rust
  /// The service mailbox (ADR-0026). Absent = this node holds no
  /// messages for other services.
  pub mailbox: Option<MailboxRole>,
  ```
  Every `RolesConfig { .. }` literal already ends in `..Default::default()`
  (`git grep -n 'RolesConfig {'`: 7 sites), so none changes.
- `crates/core/src/config/tests.rs`: `a_mailbox_role_parses_with_its_defaults`
  (`[roles.mailbox]` alone gives `MailboxRole::default()`; one overridden
  field is read) and `no_mailbox_section_means_no_mailbox`.

#### 2.2 `crates/core/src/mailbox.rs` (new) and `crates/core/src/lib.rs`

`lib.rs`: `pub mod mailbox;`. The module holds only names, wire types and
codes; no I/O.

```rust
//! What a mailbox and its callers send each other (ADR-0026): the method
//! names, the request and answer shapes, and the error codes. The sealed
//! message and the receipt themselves are in `syneroym-mailbox-format`.
//!
//! No type here refuses unknown fields: a mailbox and its callers may run
//! different versions.

pub const MAILBOX_INTERFACE: &str = "mailbox";
pub const PROTOCOL_VERSION: u32 = 1;

pub const METHOD_INFO: &str = "info";
pub const METHOD_REGISTER: &str = "register";
pub const METHOD_UNREGISTER: &str = "unregister";
pub const METHOD_DEPOSIT: &str = "deposit";
pub const METHOD_CLAIM: &str = "claim";
pub const METHOD_ACK: &str = "ack";
pub const METHOD_STATUS: &str = "status";

/// A message claimed this many times without an ack is closed.
pub const MAX_CLAIMS: u32 = 5;
pub const MAX_CLAIM_BATCH: u32 = 32;
pub const MAX_CLAIM_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ACK_BATCH: usize = 32;
pub const MAX_STATUS_BATCH: usize = 100;
pub const MAX_RECEIPT_JSON_BYTES: usize = 4 * 1024;
pub const MAX_RECORD_JSON_BYTES: usize = 4 * 1024;

/// JSON-RPC error codes. Each answer also carries `data.kind`, the
/// matching `ErrorKind` text.
pub mod codes {
    pub const UNKNOWN_DESTINATION: i32 = -32040;
    pub const TOO_LARGE: i32 = -32041;
    pub const QUEUE_FULL: i32 = -32042;
    pub const RATE_LIMITED: i32 = -32043;
    pub const MAILBOX_FULL: i32 = -32044;
    pub const NOT_AN_INSTANCE: i32 = -32045;
    pub const STALE_INSTANCE: i32 = -32046;
    pub const BAD_RECORD: i32 = -32047;
    pub const DUPLICATE_DEPOSIT: i32 = -32048;
    pub const UNAUTHENTICATED: i32 = -32049;
    /// Also carries `data.max_ttl_secs`, the limit of the queue.
    pub const TTL_TOO_LONG: i32 = -32050;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueLimits { pub max_bytes: u64, pub max_messages: u32, pub max_ttl_secs: u64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoAnswer {
    pub version: u32,
    /// The node DID of this mailbox.
    pub mailbox: String,
    pub limits: QueueLimits,
    pub max_sealed_bytes: usize,
    pub receipt_keep_secs: u64,
    pub visibility_secs: u64,
    pub queue_lifetime_secs: u64,
    pub max_claims: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterParams { pub record: SignedEndpointInfo }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterAnswer { pub limits: QueueLimits, pub queue_ends_at_secs: u64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnregisterAnswer { pub removed: bool }

// `deposit` takes a `syneroym_mailbox_format::Deposit` as its params and
// answers a `Receipt::Stored`.

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimParams { #[serde(default)] pub max: Option<u32> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedMessage {
    /// The depositor, as the mailbox saw it on the transport.
    pub sender: String,
    pub deposit_id: DepositId,
    pub sealed: SealedBytes,
    pub deposited_at_secs: u64,
    pub expires_at_secs: u64,
    pub claim_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimAnswer { pub messages: Vec<ClaimedMessage>, pub visible_again_at_secs: u64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckEntry { pub sender: String, pub deposit_id: DepositId, pub receipt: Receipt }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckParams { pub acks: Vec<AckEntry> }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckOutcome { Stored, NotFound, Refused }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckAnswer { pub results: Vec<AckOutcome> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusParams { pub deposit_ids: Vec<DepositId> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry { pub deposit_id: DepositId, pub receipt: Receipt }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusAnswer { pub receipts: Vec<StatusEntry> }
```

`SealedBytes` and `DepositId` are public and serde in `mailbox_format`, so
these types derive serde with no new code. `crates/core/Cargo.toml` already
depends on `syneroym-mailbox-format` (check).

Tests `crates/core/src/mailbox/tests.rs` (add `#[cfg(test)] mod tests;`):

1. `wire_shapes_are_fixed`: the exact JSON of one `ClaimAnswer`, one
   `AckParams`, one `AckAnswer`, one `StatusAnswer` and one
   `RegisterAnswer` (`serde_json::json!` comparisons). This is the
   contract C5 and C6 build on.
2. `error_codes_are_distinct`: the eleven codes are pairwise different.
3. `unknown_fields_are_ignored`: `ClaimParams` and `RegisterParams` parse
   with an extra field.

### Step 3 — reserved id and the dedup check (`control_plane`, `router`)

1. `crates/control_plane/src/service.rs` after line 70:
   ```rust
   /// The `native_dispatch` key the mailbox role registers under. A deploy
   /// under this name would take over every `mailbox` call to this node.
   pub const MAILBOX_RESERVED_SERVICE_ID: &str = syneroym_core::mailbox::MAILBOX_INTERFACE;
   ```
   `crates/control_plane/src/lib.rs:17`: export it with the other two.
2. `crates/control_plane/src/service/orchestration/deploy/admission.rs:31-33`:
   add `|| service_id == MAILBOX_RESERVED_SERVICE_ID`; extend the comment
   above (it lists the reserved names) and the import at
   `orchestration.rs:56`.
   Test: in `orchestration/tests/app_instance.rs` next to the supervisor
   case at :215, the same refusal for `MAILBOX_RESERVED_SERVICE_ID`.
3. `crates/router/src/call_dedup.rs:178-190`: chain
   `std::iter::once(&MAILBOX_RESERVED_SERVICE_ID)` after the supervisor
   one; update the comment ("the supervisor's and the mailbox's
   interfaces are of the same kind"). Import from `syneroym_control_plane`.
   Test in the existing `call_dedup` tests: `is_node_level_interface("mailbox")`
   and its `short_hash` are both `true`.
4. `crates/core/src/local_registry.rs:219-227` (`resolve_interface`, the
   empty-name case): also leave out `MAILBOX_INTERFACE`
   (`crate::mailbox::MAILBOX_INTERFACE`). Extend the doc comment: the
   mailbox is registered under the node DID by a role, not declared by an
   app, so it is never "the one interface" of a call that names none; a
   call to the mailbox always names it. Do not add `supervisor` here (C4-2).
   Tests in the existing `local_registry` tests:
   `a_call_with_no_interface_never_picks_the_mailbox` (node DID with only
   `orchestrator`, `security` and `mailbox`: `None`) and
   `the_mailbox_does_not_make_the_supervisor_ambiguous` (node DID with
   `supervisor` and `mailbox`: `Some("supervisor")`, as without the
   mailbox).

### Step 4 — the mailbox crate (`crates/mailbox`)

#### 4.1 Layout and `Cargo.toml`

```text
crates/mailbox/Cargo.toml
crates/mailbox/src/lib.rs
crates/mailbox/src/error.rs           + error/tests.rs
crates/mailbox/src/limits.rs          + limits/tests.rs
crates/mailbox/src/caller.rs          + caller/tests.rs
crates/mailbox/src/record.rs          + record/tests.rs
crates/mailbox/src/store.rs           (open, schema, shared helpers)
crates/mailbox/src/store/queue.rs     (register, unregister, admit_instance)
crates/mailbox/src/store/deposit.rs   (deposit, eviction, status)
crates/mailbox/src/store/claim.rs     (claim, ack)
crates/mailbox/src/store/sweep.rs     (sweep, records_due, mark_published, totals)
crates/mailbox/src/store/tests.rs     (declares the files below)
crates/mailbox/src/store/tests/{queue,deposit,claim,sweep,plaintext}.rs
crates/mailbox/src/service.rs         + service/tests.rs
crates/mailbox/src/worker.rs          + worker/tests.rs
crates/mailbox/src/test_support.rs    (#[cfg(test)])
```

`Cargo.toml` (package `syneroym-mailbox`, `[lints] workspace = true`):
dependencies `syneroym-core`, `syneroym-identity`, `syneroym-mailbox-format`,
`syneroym-passport`, `syneroym-rpc`, `anyhow`, `async-trait`, `hex`,
`ed25519-dalek` (only if `caller.rs` needs `VerifyingKey`; check how
`handshake.rs` decodes the key and copy it), `metrics`, `rusqlite`, `serde`,
`serde_json`, `thiserror`, `tokio`, `tokio-util`, `tracing`; dev
`tempfile`, `tokio` (`macros`, `rt`). All `.workspace = true`. Add the crate
to the root `Cargo.toml` `[workspace.dependencies]` next to
`syneroym-mailbox-format`: `syneroym-mailbox = { path = "crates/mailbox" }`.

`lib.rs`:

```rust
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The service mailbox (ADR-0026): a native service that holds sealed
//! messages for services that cannot be reached, and gives them to the
//! newest instance of the receiver. It reads only the outer part of a
//! message; the inner part is sealed for the receiver.

mod caller; mod error; mod limits; mod record; mod service; mod store; mod worker;

pub use error::MailboxError;
pub use limits::Limits;
pub use service::MailboxService;
pub use worker::MailboxWorker;

#[cfg(test)]
mod test_support;
```

#### 4.2 `error.rs`

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MailboxError {
    #[error("the caller has no verified identity")]
    Unauthenticated,
    #[error("the caller is not a service instance: {0}")]
    NotAnInstance(String),
    #[error("this mailbox has no queue for {0}")]
    UnknownDestination(String),
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")]
    TooLarge { bytes: usize, max: usize },
    #[error("the queue of {0} is full")]
    QueueFull(String),
    #[error("too many deposits; try again later")]
    RateLimited,
    #[error("this mailbox is full")]
    MailboxFull,
    #[error("a newer instance of {0} holds this queue")]
    StaleInstance(String),
    #[error("the service record is not accepted: {0}")]
    BadRecord(String),
    #[error("deposit id {0} was already used by this sender")]
    DuplicateDeposit(String),
    #[error("time-to-live {ttl_secs} s is over this queue's {max_secs} s maximum")]
    TtlTooLong { ttl_secs: u64, max_secs: u64 },
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl MailboxError {
    /// The `data.kind` text, snake_case of the variant name.
    pub fn kind(&self) -> &'static str { .. }
}

impl From<MailboxError> for RpcError {
    fn from(e: MailboxError) -> Self {
        // InvalidParams -> RpcError::InvalidParams, Internal -> RpcError::InternalError,
        // every other variant -> RpcError::Custom(code, e.to_string(), Some(json!({"kind": e.kind()})))
        // with the code from `syneroym_core::mailbox::codes`; TtlTooLong adds
        // "max_ttl_secs": max_secs to the same `data` object.
    }
}

impl From<rusqlite::Error> for MailboxError { /* Internal(e.to_string()) */ }
```

No message contains a key, a body or a receipt. Test `error/tests.rs`:
`each_refusal_maps_to_its_own_code_and_kind` (a table over every variant).

#### 4.3 `limits.rs`

```rust
/// The limits of this mailbox, checked once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub queue: QueueLimits,
    pub receipt_keep_secs: u64,
    pub visibility_secs: u64,
    pub queue_lifetime_secs: u64,
    pub max_total_bytes: u64,
    pub max_queues: u32,
}

impl Limits {
    pub fn from_role(role: &MailboxRole) -> anyhow::Result<Self>
}
```

`from_role` refuses, each with a message naming the `[roles.mailbox]` key:
any of `max_queue_bytes`, `max_queue_messages`, `max_ttl_secs`,
`receipt_keep_secs`, `visibility_secs`, `queue_lifetime_secs`,
`max_total_bytes`, `max_queues`, both rate limits, `sweep_interval_secs`,
`republish_interval_secs` equal to 0; `max_ttl_secs > OPEN_MAX_TTL_SECS`;
`max_queue_bytes < MAX_SEALED_BYTES as u64`; `max_total_bytes <
max_queue_bytes`; `db_name` empty or containing `/`, `\` or `..`.
Tests: `the_default_role_is_valid`, one test per refusal group (zero,
ttl, sizes, name).

#### 4.4 `caller.rs`

```rust
/// A service instance proven by the transport (ADR-0016): the member master
/// it acts for, its instance key DID, and when its certificate was issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Instance {
    pub master_did: String,
    pub instance_did: String,
    pub issued_at_secs: u64,
}

pub(crate) fn instance_of(caller: &CallerContext, now_secs: u64) -> Result<Instance, MailboxError>
pub(crate) async fn check_not_revoked(resolver: &dyn MasterAnchorResolver, who: &Instance) -> Result<(), MailboxError>
pub(crate) fn sender_of(caller: &CallerContext) -> Result<String, MailboxError>
```

```text
instance_of:
  require caller.auth in {Delegated, Ucan}           else Unauthenticated
  proof = caller.proof                               else NotAnInstance("no certificate")
  json  = proof.delegation_json                      else NotAnInstance("no certificate")
  cert  = DelegationCertificate::from_json(json)     else NotAnInstance("unreadable certificate")
  require cert.scope == SCOPE_SERVICE_INSTANCE       else NotAnInstance("not a service-instance certificate")
  require cert.master_did == caller.caller_did       else NotAnInstance("certificate is for another master")
  key_did = did of hex-decoded proof.pubkey_hex      (decode as handshake.rs:34-42 does)
  require key_did == cert.temporary_did              else NotAnInstance("key does not match the certificate")
  cert.verify_at(&cert.master_did, &[SCOPE_SERVICE_INSTANCE], now_secs)
                                                     else NotAnInstance(<error text>)
  Ok(Instance { master_did, instance_did: cert.temporary_did, issued_at_secs: cert.issued_at_secs })

check_not_revoked:
  anchor = time::timeout(5 s, resolver.resolve_master_anchor(&who.master_did))
           timeout or Err(e)                         -> Internal("could not read the master anchor: ..")
  require !anchor.revoked_keys.contains(&who.instance_did)
                                                     else NotAnInstance("the instance key is revoked")
  Ok(())

sender_of:
  require caller.auth in {Delegated, Ucan}           else Unauthenticated
  Ok(caller.caller_did.clone())
```

Doc comment on `instance_of` and `check_not_revoked`: the router has
already made these checks for a wire call; a same-node call reaches the
service with no handshake, so they are made again here. `check_not_revoked`
is a separate `async` function because it does network I/O; every receiver
handler calls it right after `instance_of`. Tests (`caller/tests.rs`, with
`test_support::Member` and `test_support::Anchors`): one pass, and one
refusal for each line above (`CallerContext::service_system`, no proof,
routing scope, other master, other pubkey, expired certificate via
`now = expires_at + 1`, revoked instance DID, resolver error).

#### 4.5 `record.rs`

```rust
/// A service record the mailbox may keep and republish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckedRecord {
    pub json: String,
    /// The pkarr timestamp, as `SignedEndpointInfo::verify` returns it.
    pub signed_at: u64,
    pub not_after: u64,
}

impl CheckedRecord {
    pub(crate) fn check(
        record: &SignedEndpointInfo,
        owner: &Instance,
        mailbox_did: &str,
    ) -> Result<Self, MailboxError>
}
```

```text
info = &record.info
info.service_id == owner.master_did                  else BadRecord("the record is for another service")
info.endpoint_type == EndpointType::Service          else BadRecord("not a service record")
!info.is_private                                     else BadRecord("a private record is never republished")
info.mailbox is Some(m) && m.service_id == mailbox_did
                                                     else BadRecord("the record does not name this mailbox")
json = serde_json::to_string(record)?; json.len() <= MAX_RECORD_JSON_BYTES
                                                     else BadRecord("the record is too large")
signed_at = record.verify().map_err(|e| BadRecord(e.to_string()))?.as_u64()
Ok(CheckedRecord { json, signed_at, not_after: info.not_after })
```

`verify` also refuses a record past `not_after` (wall clock). Tests
(`record/tests.rs`, records built like `mailbox_record_e2e.rs:73-100` with
`EndpointInfo::sign`): accepted; each refusal above; a tampered record (one
`info` field changed after signing) is `BadRecord`.

#### 4.6 `store.rs` — open, schema and the shared helpers

```rust
pub(crate) struct MailboxStore {
    conn: Mutex<Connection>,
    limits: Limits,
}

impl MailboxStore {
    pub(crate) fn open(path: &Path, limits: Limits) -> anyhow::Result<Self>
    pub(crate) fn open_in_memory(limits: Limits) -> anyhow::Result<Self>   // tests
    pub(crate) fn limits(&self) -> &Limits
    fn lock(&self) -> MutexGuard<'_, Connection>   // unwrap_or_else(PoisonError::into_inner)
}
fn to_sql(value: u64) -> i64           // as peer_records.rs:58
fn from_sql(value: i64) -> u64         // max(0)
```

`open` creates the parent directory, sets `journal_mode = WAL`, and runs:

```sql
CREATE TABLE IF NOT EXISTS queues (
    service_id          TEXT PRIMARY KEY,   -- member master DID
    record_json         TEXT NOT NULL,      -- SignedEndpointInfo as registered
    record_signed_at    INTEGER NOT NULL,
    record_not_after    INTEGER NOT NULL,
    newest_issued_at    INTEGER NOT NULL,
    newest_instance_did TEXT NOT NULL,
    max_bytes           INTEGER NOT NULL,
    max_messages        INTEGER NOT NULL,
    max_ttl_secs        INTEGER NOT NULL,
    bytes               INTEGER NOT NULL DEFAULT 0,
    messages            INTEGER NOT NULL DEFAULT 0,
    last_active_at      INTEGER NOT NULL,
    next_publish_at     INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    service_id    TEXT NOT NULL,
    sender        TEXT NOT NULL,
    deposit_id    TEXT NOT NULL,
    sealed        TEXT NOT NULL,      -- base64, as on the wire
    size          INTEGER NOT NULL,   -- decoded length
    deposited_at  INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    claimed_until INTEGER NOT NULL DEFAULT 0,
    claim_count   INTEGER NOT NULL DEFAULT 0,
    UNIQUE (sender, deposit_id)
);
CREATE INDEX IF NOT EXISTS messages_queue ON messages (service_id, seq);
CREATE INDEX IF NOT EXISTS messages_queue_sender ON messages (service_id, sender);
CREATE INDEX IF NOT EXISTS messages_expiry ON messages (expires_at);
CREATE TABLE IF NOT EXISTS receipts (
    sender       TEXT NOT NULL,
    deposit_id   TEXT NOT NULL,
    receipt_json TEXT NOT NULL,             -- syneroym_mailbox_format::Receipt
    expires_at   INTEGER NOT NULL,
    PRIMARY KEY (sender, deposit_id)
);
CREATE INDEX IF NOT EXISTS receipts_expiry ON receipts (expires_at);
```

No schema version or migration (the product is not released). Every write
method takes `now_secs` from its caller, so tests set the time. Each write
method runs in one `conn.transaction_with_behavior(TransactionBehavior::Immediate)`.

Shared helpers in `store.rs` (all take `&Transaction`):

```rust
/// Removes one message, writes its receipt, and keeps the queue counters
/// right. The only path that removes a message.
fn remove_message(tx: &Transaction, seq: i64, receipt: &Receipt, keep_secs: u64, now: u64) -> Result<(), MailboxError>
//   SELECT service_id, sender, deposit_id, size FROM messages WHERE seq = ?   (none -> Ok(()))
//   DELETE FROM messages WHERE seq = ?
//   UPDATE queues SET bytes = bytes - ?size, messages = messages - 1 WHERE service_id = ?
//   put_receipt(tx, sender, deposit_id, receipt, now + keep_secs)

fn put_receipt(tx: &Transaction, sender: &str, deposit_id: &str, receipt: &Receipt, expires_at: u64) -> Result<(), MailboxError>
//   INSERT INTO receipts ... ON CONFLICT (sender, deposit_id) DO UPDATE SET receipt_json = excluded.receipt_json, expires_at = excluded.expires_at

struct QueueRow { service_id, record_signed_at, newest_issued_at, newest_instance_did, limits: QueueLimits, bytes, messages, last_active_at }
fn queue_row(tx: &Transaction, service_id: &str) -> Result<Option<QueueRow>, MailboxError>
```

#### 4.7 `store/queue.rs` — `admit_instance`, `register`, `unregister`

```rust
/// The newest-certificate rule. Returns the queue, after recording a newer
/// certificate of the caller.
pub(super) fn admit_instance(tx: &Transaction, who: &Instance) -> Result<QueueRow, MailboxError>
```

```text
row = queue_row(tx, &who.master_did)?  else UnknownDestination(master)
if who.instance_did == row.newest_instance_did:
    if who.issued_at_secs > row.newest_issued_at: UPDATE newest_issued_at
    return row
if who.issued_at_secs > row.newest_issued_at:
    UPDATE newest_issued_at = who.issued_at_secs, newest_instance_did = who.instance_did
    return row
Err(StaleInstance(master))
```

```rust
pub(crate) struct Registered { pub answer: RegisterAnswer, pub publish_now: bool }

impl MailboxStore {
    pub(crate) fn register(&self, who: &Instance, record: &CheckedRecord, now: u64) -> Result<Registered, MailboxError>
    pub(crate) fn unregister(&self, who: &Instance, now: u64) -> Result<bool, MailboxError>
}
```

```text
register:
  tx
  match queue_row(tx, master):
    None:
      SELECT COUNT(*) FROM queues >= limits.max_queues  -> MailboxFull
      INSERT queues (service_id, record_json, record_signed_at, record_not_after,
                     newest_issued_at = who.issued_at_secs, newest_instance_did = who.instance_did,
                     max_bytes/max_messages/max_ttl_secs = limits.queue,
                     last_active_at = now, next_publish_at = now)
      publish_now = true; queue_limits = limits.queue
    Some(_):
      row = admit_instance(tx, who)?
      publish_now = record.signed_at > row.record_signed_at
      if publish_now: UPDATE record_json, record_signed_at, record_not_after, next_publish_at = now
      UPDATE last_active_at = now
      queue_limits = row.limits
  commit
  Registered { answer: RegisterAnswer { limits: queue_limits,
               queue_ends_at_secs: now + limits.queue_lifetime_secs }, publish_now }

unregister:
  tx
  if queue_row(tx, master) is None: return Ok(false)
  admit_instance(tx, who)?
  for seq in SELECT seq FROM messages WHERE service_id = master:
      remove_message(tx, seq, &Receipt::Expired { at_secs: now }, keep, now)
  DELETE FROM queues WHERE service_id = master
  commit; Ok(true)
```

`unregister` of an unknown queue answers `removed: false`, not an error, so
a receiver can call it twice. The `now` argument of `unregister` is the
receipt time.

#### 4.8 `store/deposit.rs` — `deposit` and `status`

```rust
impl MailboxStore {
    pub(crate) fn deposit(&self, sender: &str, deposit: &Deposit, now: u64) -> Result<DepositOutcome, MailboxError>
    pub(crate) fn status(&self, sender: &str, ids: &[DepositId], now: u64) -> Result<Vec<StatusEntry>, MailboxError>
}
pub(crate) struct DepositOutcome { pub receipt: Receipt, pub evicted: u32 }
```

```text
deposit:
  tx
  row = queue_row(tx, &deposit.destination)?   else UnknownDestination(destination)
  deposit.check(row.limits.max_ttl_secs) map:
      DepositError::TooLarge{bytes,max}             -> TooLarge{bytes,max}
      DepositError::TtlTooLong{ttl_secs,max_secs}   -> TtlTooLong{ttl_secs,max_secs}
      every other variant                           -> InvalidParams(e.to_string())
  id = deposit.deposit_id.to_string(); size = deposit.sealed.len()
  if let Some((dest, sealed, at)) = SELECT service_id, sealed, deposited_at FROM messages
                                    WHERE sender = ? AND deposit_id = ?:
      if dest == destination && sealed == String::from(deposit.sealed.clone()):
          return Stored { at_secs: at }          (no write; commit not needed)
      return Err(DuplicateDeposit(id))
  if SELECT 1 FROM receipts WHERE sender = ? AND deposit_id = ? exists:
      return Err(DuplicateDeposit(id))
  evicted = make_room(tx, &row, sender, size, now)?        (C4-7, below)
  total = SELECT COALESCE(SUM(bytes), 0) FROM queues
  if total + size > limits.max_total_bytes: return Err(MailboxFull)   (rollback undoes eviction)
  expires = now + deposit.ttl_secs
  INSERT messages (service_id, sender, deposit_id, sealed, size, deposited_at = now, expires_at = expires)
  UPDATE queues SET bytes = bytes + size, messages = messages + 1 WHERE service_id = destination
  put_receipt(tx, sender, id, &Stored { at_secs: now }, expires + limits.receipt_keep_secs)
  commit
  DepositOutcome { receipt: Stored { at_secs: now }, evicted }

make_room(tx, row, depositor, size, now) -> Result<u32, MailboxError>:
  (bytes, messages) = (row.bytes, row.messages); evicted = 0
  while messages + 1 > row.limits.max_messages || bytes + size > row.limits.max_bytes:
      top = SELECT sender, SUM(size) AS b FROM messages WHERE service_id = ?
            GROUP BY sender ORDER BY b DESC, sender ASC LIMIT 1
      match top:
        None                        -> return Err(QueueFull(destination))
        Some((s, _)) if s == depositor -> return Err(QueueFull(destination))
        Some((s, _)):
          (seq, vsize) = SELECT seq, size FROM messages WHERE service_id = ? AND sender = s
                         ORDER BY seq LIMIT 1
          remove_message(tx, seq, &Expired { at_secs: now }, keep, now)
          bytes -= vsize; messages -= 1; evicted += 1
  Ok(evicted)

status:
  ids.len() <= MAX_STATUS_BATCH                 else InvalidParams
  no transaction; for each id in order:
      SELECT receipt_json FROM receipts WHERE sender = ? AND deposit_id = ? AND expires_at > now
      parse as Receipt (a row that does not parse -> Receipt::Unknown, and a warn! log)
      none -> Receipt::Unknown
  Vec<StatusEntry>
```

Status keys on the caller's own DID, so a sender never reads another
sender's receipt (M-5). A receipt is `Unknown` both when it never existed
and when its keep time ended; the format crate defines `Unknown` that way.

#### 4.9 `store/claim.rs` — `claim` and `ack`

```rust
impl MailboxStore {
    pub(crate) fn claim(&self, who: &Instance, max: u32, now: u64) -> Result<ClaimAnswer, MailboxError>
    pub(crate) fn ack(&self, who: &Instance, acks: &[AckEntry], now: u64) -> Result<Vec<AckOutcome>, MailboxError>
}
```

```text
claim:
  tx
  row = admit_instance(tx, who)?
  close_over_claimed(tx, Some(&row.service_id), now)?      (shared with sweep, C4-11)
  limit = max.clamp(1, MAX_CLAIM_BATCH)
  rows = SELECT seq, sender, deposit_id, sealed, deposited_at, expires_at, claim_count
         FROM messages WHERE service_id = ? AND claimed_until <= now AND expires_at > now
              AND claim_count < MAX_CLAIMS
         ORDER BY seq LIMIT limit
  take rows in order while the running sealed total stays <= MAX_CLAIM_BYTES
       (always take the first row)
  until = now + limits.visibility_secs
  for each taken: UPDATE messages SET claimed_until = until, claim_count = claim_count + 1 WHERE seq = ?
  UPDATE queues SET last_active_at = now WHERE service_id = ?
  commit
  ClaimAnswer { messages: taken as ClaimedMessage (claim_count = new value,
                sealed = SealedBytes::try_from(stored text)), visible_again_at_secs: until }

close_over_claimed(tx, queue: Option<&str>, now) -> Result<u32, MailboxError>:
  SELECT seq FROM messages WHERE claim_count >= MAX_CLAIMS AND claimed_until <= now
         [AND service_id = ?] LIMIT 1000
  each: remove_message(tx, seq, &Receipt::unsigned_rejection(RejectCode::TooManyClaims, now), keep, now)

ack:
  acks.len() <= MAX_ACK_BATCH                    else InvalidParams
  tx
  row = admit_instance(tx, who)?
  for e in acks, in order:
      if !matches!(e.receipt, Receipt::Delivered { .. } | Receipt::Rejected { .. }) -> Refused; continue
      json = serde_json::to_string(&e.receipt)?; if json.len() > MAX_RECEIPT_JSON_BYTES -> Refused; continue
      seq = SELECT seq FROM messages WHERE service_id = row.service_id AND sender = ? AND deposit_id = ?
      None -> NotFound
      Some(seq) -> remove_message(tx, seq, &e.receipt, keep, now); Stored
  commit
```

Why `sealed` is stored as base64 text: `SealedBytes::new` is `pub(crate)`
in `mailbox_format`, so the only public way back from storage to
`SealedBytes` is `try_from(String)`. Storing the wire text needs no format
change; `size` keeps the decoded length for the limits. A stored text that
no longer parses (it cannot happen: the deposit parsed it) is skipped with
a `warn!` and closed as `malformed` by an unsigned rejection.

#### 4.10 `store/sweep.rs` — `sweep`, `records_due`, `mark_published`, `totals`

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SweepReport { pub expired: u32, pub over_claimed: u32, pub queues_removed: u32, pub receipts_removed: u32 }
pub(crate) struct DueRecord { pub service_id: String, pub record: SignedEndpointInfo }
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals { pub queues: u64, pub messages: u64, pub bytes: u64 }

impl MailboxStore {
    pub(crate) fn sweep(&self, now: u64) -> Result<SweepReport, MailboxError>
    pub(crate) fn records_due(&self, now: u64, limit: usize) -> Result<Vec<DueRecord>, MailboxError>
    pub(crate) fn mark_published(&self, service_id: &str, next_at: u64) -> Result<(), MailboxError>
    pub(crate) fn totals(&self) -> Result<Totals, MailboxError>
}
```

```text
sweep (one transaction):
  1. SELECT seq FROM messages WHERE expires_at <= now LIMIT 1000
     -> remove_message(Expired { at_secs: now })                    expired += n      (M-7)
  2. close_over_claimed(tx, None, now)                               over_claimed += n (M-14)
  3. SELECT service_id FROM queues WHERE last_active_at + lifetime <= now LIMIT 1000
     -> for each: every message -> remove_message(Expired), DELETE queue
                                                                     queues_removed += n (M-10)
  4. DELETE FROM receipts WHERE expires_at <= now                    receipts_removed = changes (M-9)
  commit

records_due:
  SELECT service_id, record_json FROM queues
   WHERE next_publish_at <= now AND record_not_after > now
   ORDER BY next_publish_at LIMIT limit
  a record_json that does not parse: warn! and skip (cannot happen: register wrote it)

mark_published: UPDATE queues SET next_publish_at = ? WHERE service_id = ?
totals: SELECT COUNT(*), COALESCE(SUM(messages),0), COALESCE(SUM(bytes),0) FROM queues
```

Write `last_active_at + lifetime <= now` as `last_active_at <= ?` with
`now - lifetime` (saturating) so the index-free comparison stays simple.

#### 4.11 `service.rs` — the native service

```rust
pub struct MailboxService {
    node_did: String,
    store: Arc<MailboxStore>,
    /// Reads master anchors for the revocation check of receiver calls.
    anchors: Arc<dyn MasterAnchorResolver>,
    per_sender: RequestLimiter<String>,
    total: RequestLimiter<()>,
    /// Woken when a register brings a new record to publish.
    wake: Arc<Notify>,
    sweep_every: Duration,
    republish_every_secs: u64,
}

impl MailboxService {
    /// Opens the store at `<db_dir>/<role.db_name>`.
    pub fn open(
        role: &MailboxRole,
        db_dir: &Path,
        node_did: &str,
        anchors: Arc<dyn MasterAnchorResolver>,
    ) -> anyhow::Result<Self>
    /// The background loop for this service. `registry` is `None` when
    /// this node has no registry; then nothing is republished.
    #[must_use]
    pub fn worker(&self, registry: Option<Arc<RegistryClient>>) -> MailboxWorker
}

#[async_trait::async_trait]
impl NativeService for MailboxService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let NativeInvocation { method, params, caller, .. } = invocation;
        let now = syneroym_passport::now_secs();
        let payload = match method.as_str() {
            METHOD_INFO => self.info(),
            METHOD_REGISTER => self.register(&caller, params, now).await,
            METHOD_UNREGISTER => self.unregister(&caller, now).await,
            METHOD_DEPOSIT => self.deposit(&caller, params, now).await,
            METHOD_CLAIM => self.claim(&caller, params, now).await,
            METHOD_ACK => self.ack(&caller, params, now).await,
            METHOD_STATUS => self.status(&caller, params, now).await,
            other => return Err(RpcError::MethodNotFound(other.to_string())),
        }
        .map_err(RpcError::from)?;
        Ok(NativeResponse { payload })
    }
}
```

`Debug` by hand (node DID only). Each handler returns
`Result<Value, MailboxError>` and is a few lines:

```rust
fn parse<T: DeserializeOwned>(params: Value) -> Result<T, MailboxError>   // InvalidParams on error
fn answer<T: Serialize>(value: &T) -> Result<Value, MailboxError>          // Internal on error
async fn blocking<T: Send + 'static>(
    store: &Arc<MailboxStore>,
    f: impl FnOnce(&MailboxStore) -> Result<T, MailboxError> + Send + 'static,
) -> Result<T, MailboxError>   // task::spawn_blocking; a JoinError -> Internal
```

```text
register(caller, params, now):
  who = self.instance(caller, now).await?; p: RegisterParams = parse(params)?
  record = CheckedRecord::check(&p.record, &who, &self.node_did)?
  r = blocking(store, move |s| s.register(&who, &record, now)).await?
  if r.publish_now { self.wake.notify_one() }
  answer(&r.answer)

deposit(caller, params, now):
  sender = sender_of(caller)?
  deposit = parse_deposit(params)?                       (below)
  if !self.per_sender.allow_key(sender.clone(), now) || !self.total.allow_key((), now):
      counter refused{kind=rate_limited}; return Err(RateLimited)
  out = blocking(store, move |s| s.deposit(&sender, &deposit, now)).await
  metrics: accepted -> counter substrate.mailbox.deposits (+1), substrate.mailbox.evictions (+evicted)
           refused  -> counter substrate.mailbox.deposits_refused, label kind = e.kind()
  answer(&out?.receipt)

parse_deposit(params) -> Result<Deposit, MailboxError>:
  #[derive(Deserialize)] struct DepositWire { destination: String, deposit_id: DepositId, ttl_secs: u64, sealed: String }
  w = parse::<DepositWire>(params)?
  sealed = SealedBytes::try_from(w.sealed) map:
      SealedBytesParseError::TooLarge { bytes, max } -> TooLarge { bytes, max }
      NotBase64 -> InvalidParams
  Ok(Deposit { destination: w.destination, deposit_id: w.deposit_id, ttl_secs: w.ttl_secs, sealed })

instance(caller, now):                                  (every receiver method)
  who = instance_of(caller, now)?
  check_not_revoked(self.anchors.as_ref(), &who).await?
  Ok(who)

unregister / claim / ack: who = self.instance(caller, now).await?, parse, blocking store call, answer.
  claim: max = p.max.unwrap_or(MAX_CLAIM_BATCH)
status: sender = sender_of(caller)?, parse, blocking, answer(&StatusAnswer { receipts })
info:   answer(&InfoAnswer { version: PROTOCOL_VERSION, mailbox: node_did, limits: store.limits().queue,
                             max_sealed_bytes: MAX_SEALED_BYTES, receipt_keep_secs, visibility_secs,
                             queue_lifetime_secs, max_claims: MAX_CLAIMS })
```

`DepositWire` exists because a `Deposit` whose `sealed` is too large fails
inside serde, and serde does not keep the error type; M-4 needs a separate
error for size. Write that reason in its doc comment.

#### 4.12 `worker.rs`

```rust
pub struct MailboxWorker {
    store: Arc<MailboxStore>,
    registry: Option<Arc<RegistryClient>>,
    wake: Arc<Notify>,
    sweep_every: Duration,
    republish_every_secs: u64,
}

impl MailboxWorker {
    pub async fn run(self, cancel: CancellationToken)
}
```

```text
run:
  let mut tick = time::interval(sweep_every); tick.set_missed_tick_behavior(MissedTickBehavior::Delay)
  loop:
    select! { () = cancel.cancelled() => return,
              _ = tick.tick() => {},
              () = wake.notified() => {} }
    now = now_secs()
    match blocking(store, |s| s.sweep(now)):
      Ok(report) -> counters substrate.mailbox.expired (+expired),
                    substrate.mailbox.closed_unacked (+over_claimed); debug! when non-zero
      Err(e)     -> warn!(error = %e, "mailbox sweep failed")
    if let Some(registry) = &registry { self.republish(registry, now, &cancel).await }
    if let Ok(t) = blocking(store, |s| s.totals()):
      gauges substrate.mailbox.queues / .messages / .bytes

republish(registry, now, cancel):
  due = blocking(store, move |s| s.records_due(now, 100)) (Err -> warn!, return)
  for r in due:
    result = select! { () = cancel.cancelled() => return, res = registry.register(&r.record, false) => res }
    next = if result.is_ok() { now + republish_every_secs } else {
             counter substrate.mailbox.republish_failures (+1); now + 300 }
    blocking(store, move |s| s.mark_published(&r.service_id, next))  (Err -> warn!)
```

`RegistryClient::register` already logs a warning on each failed HTTP
attempt, so `republish` adds no log line of its own on failure. A `409`
from the registry (it holds a newer record) is a normal case: the
receiver deployed again and has not registered yet. Retry every 5 minutes
is acceptable for it.

#### 4.13 `test_support.rs`

- `Member` like `crates/mailbox_format/src/test_support.rs:20-60`
  (master, instance, `service-instance` certificate, mailbox key), plus
  `fn caller(&self) -> CallerContext` (auth `Delegated`, `caller_did` the
  master DID, `proof` with the instance pubkey hex and the certificate
  JSON) and `fn instance(&self) -> Instance`.
- `fn record(member: &Member, mailbox_did: &str) -> SignedEndpointInfo`
  (public service record naming the mailbox, `not_after` 30 days ahead).
- `fn sealed_deposit(from: &Member, to: &Member, ttl: u64, now: u64) -> Sealed`
  with real `seal` and a fixed `MessageContent` (interface
  `"marker-interface"`, method `"marker-method"`).
- `Anchors`: a `MasterAnchorResolver` over a `Mutex<HashMap<String,
  MasterAnchorPayload>>`, with `publish(master_did)`, `revoke(master_did,
  key_did)` and a `fail` switch. A master with no entry is an error, as
  with the real registry.
- `fn store_with(limits: Limits) -> MailboxStore` (in memory) and
  `fn small_limits() -> Limits` (queue 3 messages, 200 KiB, visibility 10 s,
  keep 100 s, lifetime 1,000 s, total 10 MiB, 4 queues).

#### 4.14 Tests in the crate

Name tests by behaviour. Times are relative to a fixed `NOW` or to the
member certificate (`member.cert.issued_at_secs + 10`).

`store/tests/queue.rs`
1. `register_creates_a_queue_and_reports_its_limits_and_end`.
2. `a_newer_record_replaces_the_stored_one_and_an_older_one_is_kept`
   (also checks `publish_now`).
3. `a_newer_instance_takes_the_queue_and_the_old_one_is_stale` (M-12;
   claim by the old one is `StaleInstance`).
4. `the_same_instance_with_an_older_certificate_is_still_admitted`.
5. `two_instances_with_the_same_issue_time_keep_the_first`.
6. `a_new_queue_is_refused_when_the_mailbox_holds_its_maximum`.
7. `unregister_removes_the_queue_and_expires_its_messages` (status of a
   message in it is `expired`); `unregister_of_no_queue_answers_false`.

`store/tests/deposit.rs`
1. `a_deposit_to_an_unknown_destination_is_refused` (M-4).
2. `a_deposit_is_stored_and_its_receipt_is_stored`.
3. `the_same_deposit_twice_answers_the_first_time`.
4. `the_same_id_with_other_content_is_a_duplicate`; `a_used_id_is_a_duplicate_after_delivery`.
5. `two_senders_may_use_the_same_deposit_id`.
6. `a_ttl_above_the_queue_limit_is_ttl_too_long` (`TtlTooLong` with the queue's `max_ttl_secs`).
7. `a_full_queue_evicts_the_largest_sender_first` (M-8: victim's receipt
   is `expired`, the new deposit is `stored`).
8. `a_full_queue_refuses_the_largest_sender_itself` (M-8: `QueueFull`,
   nothing removed).
9. `the_message_count_limit_also_evicts`.
10. `the_mailbox_cap_refuses_and_evicts_nothing` (`MailboxFull`; the
    would-be victim is still there).
11. `status_shows_only_the_callers_own_receipts` (M-5: another sender's id
    is `unknown`); `status_of_an_unknown_id_is_unknown`.

`store/tests/claim.rs`
1. `claim_returns_messages_oldest_first_up_to_the_batch_and_byte_limit`.
2. `a_claimed_message_is_hidden_until_its_visibility_time_ends` (M-6).
3. `claim_without_a_queue_is_unknown_destination`.
4. `five_claims_without_an_ack_close_the_message_unsigned` (M-14: receipt
   `rejected`, code `too_many_claims`, no proof; at the 5th visibility end,
   not before).
5. `an_ack_stores_the_receipt_as_sent_and_removes_the_body` (M-13: the
   receipt from `OpenedMessage::sign_delivered` reads back equal from
   `status`, and `verify_signed` passes on it).
6. `an_ack_of_a_stored_or_unknown_receipt_is_refused`; `an_oversized_receipt_is_refused`.
7. `an_ack_for_another_queues_message_is_not_found` (M-5).
8. `an_ack_after_eviction_is_not_found` (receipt stays `expired`).

`store/tests/sweep.rs`
1. `an_expired_message_is_removed_and_its_receipt_says_expired` (M-7).
2. `a_receipt_is_kept_for_the_keep_time_then_removed` (M-9: present at
   `keep - 1`, gone at `keep`).
3. `an_idle_queue_is_removed_after_its_lifetime` (M-10) and
   `a_claim_keeps_a_queue_alive`.
4. `a_record_is_due_after_register_and_not_after_it_was_published`;
   `a_record_past_not_after_is_never_due`.
5. `the_counters_match_the_messages_after_every_kind_of_removal`
   (deposit, eviction, ack, expiry, too-many-claims, unregister; then
   compare `totals()` with `SELECT COUNT(*), SUM(size) FROM messages`).

`store/tests/plaintext.rs`
1. `the_store_holds_no_message_content_in_clear` (M-11): a file-backed
   store in a `tempdir`, one deposit from `sealed_deposit`, close the
   store (drop, so WAL is checkpointed; or `PRAGMA wal_checkpoint(TRUNCATE)`),
   read every file in the directory, assert none contains
   `marker-interface`, `marker-method` or the params text.

`service/tests.rs` (in process, `MailboxService::open` on a `tempdir`)
1. `an_unknown_method_is_method_not_found`.
2. `a_system_caller_is_refused` (`CallerContext::service_system`: code
   `-32049`).
3. `a_routing_certificate_cannot_register` (`-32045`).
4. `register_deposit_claim_ack_status_round_trip` (with real `seal`,
   `open` with the member's mailbox key, `sign_delivered`, and
   `verify_signed` on the status answer).
5. `deposits_above_the_sender_rate_are_refused` (role with
   `deposits_per_minute_per_sender = 2`: third is `-32043`, another sender
   still passes) and `the_node_rate_limits_every_sender`.
6. `a_deposit_over_the_size_limit_is_too_large` (params with a 90,000-char
   base64 `sealed`: `-32041`, `data.kind == "too_large"`).
7. `a_register_with_a_record_for_another_mailbox_is_refused` (`-32047`).
8. `info_reports_the_limits`.
9. `a_ttl_over_the_limit_reports_the_queue_limit` (`-32050`,
   `data.kind == "ttl_too_long"`, `data.max_ttl_secs` equal to the
   queue's limit).
10. `a_revoked_instance_is_refused` (the anchor lists the instance DID:
    `register` and `claim` answer `-32045`; a newer instance of the same
    master that is not revoked still registers).
11. `an_unreadable_master_anchor_refuses_receiver_calls` (the resolver
    fails: `claim` answers `-32603`; `deposit` and `status` still work,
    they do not read the anchor).

`worker/tests.rs`
1. `the_worker_sweeps_on_its_tick_and_stops_when_cancelled` (sweep
   interval 1 s, an expired message is `expired` within 3 s; cancel; the
   join finishes within 1 s). Republish is tested end to end (Step 7).

`limits`, `caller`, `record`, `error` tests: see 4.2 to 4.5.

### Step 5 — SDK: a delegation on JSON-RPC requests (`crates/sdk`)

`crates/sdk/src/client.rs`:

- New field `caller_delegation: Option<DelegationCertificate>` beside
  `caller_ucan` (doc: "a certificate that lets this client's key act as its
  master, sent on every request preamble"); `None` in every constructor
  (`git grep -n 'caller_ucan: None' crates/sdk`: set the new field beside
  each one) and listed in the `Debug` impl.
- `pub fn with_delegation(mut self, cert: DelegationCertificate) -> Self`
  beside `with_ucan` (:312).
- `open_request_stream` (:514-517): `preamble.delegation = self.caller_delegation.clone();`.
- Test in the existing SDK client tests: a client built
  `with_delegation(cert)` writes a preamble line whose parsed `delegation`
  equals `cert` (use the preamble builder, not a network).

### Step 6 — substrate wiring (`crates/substrate`)

1. `crates/substrate/Cargo.toml`: `syneroym-mailbox = { workspace = true, optional = true }`;
   feature `mailbox = ["dep:syneroym-mailbox"]`; add `"mailbox"` to
   `default`. Not in `minimal`.
2. New `crates/substrate/src/runtime/mailbox.rs` (declare `mod mailbox;`
   in `runtime.rs` beside `supervisor`):

```rust
//! The mailbox role: registers the native service and builds its worker.

#[cfg(feature = "mailbox")]
pub(super) struct MailboxParts {
    pub(super) worker: syneroym_mailbox::MailboxWorker,
}
#[cfg(not(feature = "mailbox"))]
pub(super) struct MailboxParts;

/// Registers the mailbox on interface `mailbox` of this node, or removes a
/// registration left from an earlier boot when the role is off.
pub(super) async fn init_mailbox(
    config: &SubstrateConfig,
    node_did: &str,
    shared: &SharedNodeHandles,
    endpoint_registry: &EndpointRegistry,
) -> anyhow::Result<Option<MailboxParts>>
```

```text
#[cfg(feature = "mailbox")] init_mailbox:
  let Some(role) = &config.roles.mailbox else {
      if endpoint_registry.lookup_by_service(node_did).iter().any(|(i, _)| i == MAILBOX_INTERFACE) {
          endpoint_registry.remove(node_did, MAILBOX_INTERFACE).await?;
      }
      return Ok(None);
  };
  let client = Arc::new(RegistryClient::new(config.substrate.enable_bep0044_dht,
                                            config.substrate.registry_url.clone()));
  let service = Arc::new(MailboxService::open(role, &config.storage.db_dir, node_did,
                                              client.clone() as Arc<dyn MasterAnchorResolver>)?);
  endpoint_registry.register(node_did.to_string(), MAILBOX_INTERFACE.to_string(),
      SubstrateEndpoint::NativeHostChannel { service_id: MAILBOX_RESERVED_SERVICE_ID.to_string() }).await?;
  shared.native_dispatch().insert(MAILBOX_RESERVED_SERVICE_ID.to_string(),
      service.clone() as Arc<dyn NativeService>);
  // The same client the router builds (`route_handler.rs`), so the
  // revocation check reads the anchors the handshake reads.
  let registry = (config.substrate.registry_url.is_some() || config.substrate.enable_bep0044_dht)
      .then_some(client);
  if registry.is_none() {
      warn!("mailbox role is enabled but this node has no registry: the service records of \
             sleeping receivers will not be kept alive");
  }
  Ok(Some(MailboxParts { worker: service.worker(registry) }))

#[cfg(not(feature = "mailbox"))] init_mailbox:
  if config.roles.mailbox.is_some() { bail!("[roles.mailbox] is configured but this binary was \
      built without the `mailbox` feature") }
  Ok(None)
```

   The removal path also runs without the feature (copy the three lines,
   or put them in a small shared helper used by both versions).

3. `crates/substrate/src/runtime/router.rs`:
   - New struct at the top:
     ```rust
     /// What `setup_connection_router` builds for the rest of the runtime.
     pub(super) struct RouterSetup {
         pub(super) router: ConnectionRouter,
         pub(super) endpoint_registry: EndpointRegistry,
         pub(super) supervisor: Option<Arc<SupervisorHandle>>,
         pub(super) conversation: Arc<ConversationService>,
         pub(super) auth_did: Option<String>,
         pub(super) mailbox: Option<MailboxParts>,
     }
     ```
   - `setup_connection_router(..) -> anyhow::Result<RouterSetup>` (:71-114)
     and `setup_router(..) -> anyhow::Result<RouterSetup>` (:116-249). The
     `publisher` is no longer returned.
   - In `setup_router`, after `register_supervisor` (:146):
     `let mailbox = init_mailbox(config, service_id, &shared, &endpoint_registry).await?;`
   - The final `Ok((..))` (:248) becomes `Ok(RouterSetup { .. })`.
4. `crates/substrate/src/runtime.rs:149-154`:
   ```rust
   let setup = setup_connection_router(&config, keys, relay_leases).await?;
   services.set_supervisor(setup.supervisor);
   services.set_conversation(setup.conversation);
   services.set_mailbox(setup.mailbox);
   #[cfg(feature = "client_gateway")]
   services.set_gateway_auth_did(setup.auth_did);
   Ok(InitializedRuntime { observability, services,
       connection_router: setup.router, endpoint_registry: setup.endpoint_registry })
   ```
5. `crates/substrate/src/runtime/services.rs`:
   - Fields: `mailbox: Option<MailboxParts>` (taken at spawn),
     `mailbox_join: Option<JoinHandle<()>>`, `mailbox_cancel: CancellationToken`
     (doc: raced like the others; **awaited** in `shutdown`, because its
     loop stops at the token and does only local work and short registry
     calls). Init them in `init` (:121-143).
   - `pub(super) fn set_mailbox(&mut self, mailbox: Option<MailboxParts>)`.
   - `spawn_background_workers` (:192-216): with the feature on,
     `self.mailbox_join = self.mailbox.take().map(|parts| { let cancel = self.mailbox_cancel.clone();
     tokio::spawn(parts.worker.run(cancel)) });`. With it off, nothing (the
     type has no worker).
   - `run_until_shutdown`: `let mut mailbox_fut = pin::pin!(join_unit_or_pending(self.mailbox_join.as_mut(), "mailbox worker"));`
     and a `select!` arm `res = &mut mailbox_fut => log_component_exit("mailbox worker", res),`.
   - `shutdown`: `self.mailbox_cancel.cancel(); if let Some(h) = self.mailbox_join.take() { if let Err(e) = h.await { error!(..) } }`.
6. `crates/substrate/config.sample.toml`, after `# [roles.roym]` (:261):
   a commented `# [roles.mailbox]` block with every key, its default and
   one short comment line each.

### Step 7 — integration tests (`crates/substrate/tests/mailbox_service_e2e.rs`)

New file. One node with `[roles.mailbox]` through
`SubstrateNode::builder().owner(&owner).configure(|c| c.roles.mailbox = Some(MailboxRole { visibility_secs: 2, sweep_interval_secs: 1, republish_interval_secs: 2, ..Default::default() })).boot()`.
Check in `common/node.rs` that the node's `substrate.registry_url` is its
own registry (`registry_url()`); if it is not, set it in `configure`. No
port literals: the harness allocates them.

Helpers in the file (keep it under 800 lines; move shared record helpers
into `common/` only if `mailbox_record_e2e.rs` can use them too):
- `Member { master, instance, cert, mailbox }` with a real
  `service-instance` certificate (`DelegationCertificate::issue`).
  `Member::new(node)` also publishes the master's anchor to the node's
  registry with `common::conversation_fixture::publish_master_anchor`.
  Without it, `verify_preamble` refuses every delegated call (it reads the
  anchor for revocation), and so does the mailbox's own check. Do this for
  every member, senders too. `mailbox_record_e2e.rs:106` does the same.
- `async fn publish_anchors(node, members)`: publishes the anchors again.
  The node's registry keeps its data in memory only (`DashMap` in
  `community_registry/src/registry.rs`), so a restart loses them.
- `async fn instance_client(node, member) -> SyneroymClient`:
  `node.client_as(clone of member.instance).with_delegation(member.cert.clone())`, then `connect()`.
- `async fn sender_client(node, member)`: the same for the sender.
- `fn record(member, node_did)`: public service record naming the node as
  mailbox (copy the shape of `mailbox_record_e2e.rs::record_with_mailbox`).
- `async fn call(client, method, params) -> Result<Value, (i32, Value)>`
  over `client.request(MAILBOX_INTERFACE, method, params)`.

Tests:
1. `a_registered_record_is_published_by_the_mailbox` (K-4): the test
   never posts the record itself; after `register`, poll
   `RegistryClient::lookup(service_id, false)` on the node's registry for
   up to 10 s and compare with the registered record.
2. `a_stale_republish_never_replaces_a_newer_record` (K-4): register record
   R1; post a newer R2 of the same master straight to the registry
   (`RegistryClient::register`); wait two republish intervals; lookup
   still returns R2.
3. `a_deposit_reaches_the_receiver_and_its_delivery_receipt_reaches_the_sender`
   (M-1, M-5, M-13, over the wire): sender deposits a real `seal` result;
   receiver claims, `open`s with its mailbox key, `sign_delivered`, acks;
   sender `status` gives a receipt for which `verify_signed(&sent, ..)` is
   `Ok(SignedOutcome::Delivered)`.
4. `a_moved_service_takes_the_queue_from_its_old_instance` (M-12): instance
   A registers; `tokio::time::sleep(1_100 ms)` (certificate times are whole
   seconds); instance B of the same master (new key, new certificate)
   registers; A's `claim` answers `-32046`; B's `claim` works.
5. `refusals_have_their_own_codes`: deposit to an unregistered master
   (`-32040`); `register` by a client with no delegation (`-32045`); a
   second identical deposit answers `stored` and a changed one `-32048`.
6. `the_queue_and_its_messages_survive_a_restart`: deposit, then
   `node.restart_in_place()`, then `publish_anchors` for the sender and
   the receiver (the restarted registry is empty), then the receiver's
   `claim` returns the message.
7. `an_unclaimed_message_returns_after_its_visibility_time` (M-6): claim,
   do not ack, claim again at once (empty), after 3 s claim again (the
   message, `claim_count == 2`).

A node **without** the role is covered by the unit test of Step 3 and the
removal path of Step 6; do not add a network test for it (a call to an
interface the node does not serve goes to `relay_to_next_hop`, whose
behaviour is not part of this slice).

### Step 8 — docs in the same PR

1. `docs/system-architecture.md`: a new subsection "Service mailbox"
   next to the record text at :2061. It says: the role and its config
   section; interface `mailbox` under the node DID; the seven methods, who
   may call each (C4-3) and what each answers; the store (file, plaintext,
   what it holds); the limits and their defaults; fair eviction and the
   mailbox cap (C4-7); the newest-certificate rule (C4-8); receipts kept as
   sent and for how long; closing after 5 claims; the queue lifetime
   (C4-12); hourly republish to the host's own registry (C4-13); the error
   codes table. Say that the sender and receiver paths that use it are not
   built yet. ADR links only; no change-doc or slice ids.
2. `docs/developer-guide.md`: the `[roles.mailbox]` keys (a table: key,
   default, meaning); `mailbox.db` in the database-directory row at :270
   ("the mailbox's queues, messages and receipts; not a cache; plaintext
   metadata, sealed bodies"); the metrics (`substrate.mailbox.deposits`,
   `.deposits_refused` with label `kind`, `.evictions`, `.expired`,
   `.closed_unacked`, `.republish_failures`, gauges `.queues`, `.messages`,
   `.bytes`); the `mailbox` Cargo feature.
3. `docs/TERMINOLOGY.md`: rows "Mailbox" (a native service that holds
   sealed messages for a service that cannot be reached; ADR-0026),
   "Queue lifetime" (how long a queue lives without a register or claim;
   30 days by default) and "Sealed message" (a message for a mailbox: an
   outer part the mailbox reads (destination, deposit id, size, time to
   live) and an inner part sealed to the receiver's mailbox key, which
   holds the call, the sender's instance certificate and the sender's
   signature; ADR-0026). The change doc close-out
   lists all three; the file has none of them today.
4. `docs/system-requirements-spec.md` under `[PLT-ASY]` (:1063): one
   bullet for the mailbox role (what a node with the role provides, its
   limits, M-1 to M-14 in plain words), saying the sender and receiver
   paths are Envisioned.
5. `docs/planning/traceability-matrix.md`: a row for the mailbox role:
   "Partial: the mailbox service is implemented; deposit by senders and
   pull by receivers are Envisioned", with links to `crates/mailbox` and
   `mailbox_service_e2e.rs`.
6. `docs/planning/changes/service-mailbox/change.md`:
   - Fill the `Test` column of M-1 to M-14 and K-4.
   - Tasks: tick "Mailbox role, native service, store, quotas, fair
     eviction, sweeps, newest-certificate rule, signed receipts", and
     change the "Lookup change" line's last part to "(done)".
   - In "Mailbox methods" and "Dedicated mailbox and passport", fix the
     stale sentence about the certificate in the `register` body (§6
     item 2): the certificate comes from the verified caller proof.
   - Deviations `D-C4-1` to `D-C4-n`: one line each for C4-3 (certificate
     from the caller proof, not the body), C4-4 (no proof of possession;
     denial of service only), C4-6, C4-7 (eviction receipt `expired`; mailbox cap
     refuses), C4-8 (tie rule), C4-9 (record
     checks; older record kept), C4-11, C4-12, C4-13 (own registry only),
     C4-14 (per sender and per node, not per sending node), C4-5
     (plaintext store), C4-16 (`ttl_too_long` with the queue's limit),
     C4-2 (`mailbox` never "the one interface"), plus every difference
     found while building. C4-8's line also says that the mailbox checks
     revocation itself, because a same-node call runs no handshake.
   - Do **not** set `status: done`: C5 to C8 remain.
7. `docs/planning/changes/operator-passport/change.md`, `D-B1-17`: add
   these sentences at its end (do not rewrite the rest): "Changed by C4
   (`D-C4-n`): the router passes the verified certificate in
   `CallerContext.proof.delegation_json`, so `register` has no certificate
   in its body. C8 takes the certificate for
   `SignerBinding::InstanceCertificate` from there. The binding form and
   the rule against a 'the transport already checked it' form are
   unchanged." (Decided, §6 item 2.)
8. `task.md`: the status line; C4 marked "Done" with a pointer to the
   decisions; these forward notes (also in §7 below) added to C5, C6, C7
   and C8; and two line changes:
   - C8: replace "`register` carries the instance certificate
     (`D-B1-17`)" with "`register` takes the instance certificate from the
     verified caller proof, not its body (`D-B1-17`, as changed by C4)".
   - C7: change "Needs A2 and C3." to "Needs A2 and C3, and the router
     check that a caller holds the instance key (backlog)." and add the
     requirement of the C7 note in §7.
9. `status.md`: the C4 row with evidence (test files and counts, verify
   result).
10. `docs/planning/deferred-backlog.md`: the rows of §5.

## 4. Test list (summary)

| Criterion | Where |
| --- | --- |
| M-1 | `service/tests.rs` (every method), `mailbox_service_e2e.rs` test 3 |
| M-2, M-3 | `caller/tests.rs`, `record/tests.rs`, `service/tests.rs` 2, 3, e2e 5 |
| M-4 | `store/tests/deposit.rs` 1, 6-10; `service/tests.rs` 5, 6, 9; e2e 5 |
| M-5 | `deposit.rs` 11, `claim.rs` 7 |
| M-6 | `claim.rs` 2, e2e 7 |
| M-7 | `sweep.rs` 1 |
| M-8 | `deposit.rs` 7-9 |
| M-9 | `sweep.rs` 2 |
| M-10 | `sweep.rs` 3 |
| M-11 | `plaintext.rs` |
| M-12 | `queue.rs` 3-5, `caller/tests.rs` (revoked), `service/tests.rs` 10, 11, e2e 4 |
| M-13 | `claim.rs` 5, e2e 3 |
| M-14 | `claim.rs` 4 |
| K-4 | `sweep.rs` 4, e2e 1, 2 |

Plus: limiter (Step 1), config and wire shapes (Step 2), reserved id,
dedup and calls with no interface (Step 3), SDK preamble (Step 5), counters (`sweep.rs` 5), worker
(`worker/tests.rs`).

## 5. Deferred-backlog rows to add (Step 8)

1. **Extend** the row "Router does not check that the caller holds the
   temporary key" (§3 of the backlog): the mailbox relies on it. A holder
   of a copy of a receiver's instance certificate (every sender that got a
   `delivered` receipt has one) can register, unregister, claim and ack
   for that receiver, and a holder of a sender's certificate can deposit
   as it (and so make fair eviction remove that sender's messages).
   This is a denial of service: no message is read, faked as delivered or
   lost (ADR-0026 A1, S-5), but repeated `unregister` calls turn the
   mailbox off for one chosen receiver. Fix it in the router before open
   mailboxes are listed for strangers: C7 needs it, and carries the fix if
   it has not landed (task.md C7, §7). Source:
   `crates/mailbox/src/caller.rs`.
2. **Mailbox store is plaintext.** `mailbox.db` lists who sent to whom,
   when and how much. Bodies are sealed. Encrypting it needs the KEK at
   boot. TBD. `crates/mailbox/src/store.rs`.
3. **No per-node deposit limit.** The mailbox sees the caller's DIDs, not
   the sending node, so it limits per sender DID and for the whole node.
   Needs the transport's remote node id in `CallerContext`. TBD.
   `crates/mailbox/src/service.rs`.
4. **The mailbox cap does not evict across queues.** When the node total
   is reached, every new deposit is refused until messages leave. TBD.
   `crates/mailbox/src/store/deposit.rs`.
5. **The mailbox republishes only to its own registry.** A receiver whose
   senders use another registry is not kept findable there. TBD (C7 lists
   open mailboxes per registry, which makes the two match in practice).
   `crates/mailbox/src/worker.rs`.
6. **DHT refresh of a replayed record is unverified.** The mailbox
   republishes the same signed packet; whether DHT nodes and pkarr relays
   accept an equal timestamp as a refresh was not tested (task.md §1 had
   this as *Unverified*). TBD. `crates/core/src/dht_registry/client.rs`.
7. **No operator surface for the mailbox.** No CLI or control-plane call
   lists queues, removes a queue or blocks a sender. TBD.
8. **A call with no interface to the node DID can reach the supervisor.**
   `EndpointRegistry::resolve_interface` leaves out only
   `NATIVE_CAPABILITY_INTERFACES`, `NODE_NATIVE_INTERFACES` and (from C4)
   `mailbox`. On a node with the supervisor role, `supervisor` is then the
   node DID's "one interface", and a call that names no interface goes to
   it. C4 does not change this, because a caller may rely on it; check
   that no caller does, then leave `supervisor` out too. TBD.
   `crates/core/src/local_registry.rs:219`.
9. **Receiver calls read the master anchor twice.** A wire call reads it
   in the router handshake and again in the mailbox, because the mailbox
   cannot tell a wire call from a same-node call. A `CallerContext` flag
   set only by the handshake would remove the second read. TBD.
   `crates/mailbox/src/caller.rs`.

Remove none: C4 resolves no existing row.

## 6. Things that are ambiguous or stale in task.md and the change docs

1. **No proof of possession (decided: C4-4, Option A).** M-2 says the
   mailbox takes the caller from "the verified transport identity". The
   router verifies the certificate, its scope, the key DID match and
   revocation, but the preamble `pubkey` is only asserted
   (`handshake.rs:25-83`; backlog row). The change doc and `D-2` assume the
   transport proves the instance key. The owner accepted this for C4 on
   2026-10-18: the mailbox risk is denial of service (C4-4), and the fix
   belongs in the router. The backlog row is extended (§5 item 1).
2. **Stale: "the router keeps only the two DIDs of the connection".** The
   change doc ("Dedicated mailbox and passport") and the passport doc
   (`D-B1-17`) say `register` must carry the instance certificate in its
   body because the router does not pass it on. `build_caller`
   (`route_handler/io.rs:250-258`) puts the verified certificate in
   `CallerContext.proof.delegation_json`. C4 reads it from there and adds
   no body field (C4-3). **Decided (2026-10-18): C8 takes the certificate
   for its signer binding from the same place.** It is the same
   certificate, and the router has verified it. Only its source changes:
   the binding stays `SignerBinding::InstanceCertificate`, and D-B1-17's
   rule against a "the transport already checked it" binding form still
   holds. C8 must not add a body field that C4 refused, so the two slices
   cannot differ here. Step 8.6, 8.7 and 8.8 change the change doc, the
   passport doc and task.md C8 together, in the C4 PR.
3. **"Deposit rate limits for each node".** A native service does not see
   the sending node (`CallerContext` has no transport peer id). C4-14
   limits per sender DID and per mailbox node. Backlog row 3.
4. **No receipt state for an evicted message.** M-8 says what is removed
   but not what the sender then reads. C4-7 uses `expired`.
5. **"A global storage cap" has no rule.** C4-7 refuses (`mailbox_full`)
   and does not evict across queues. Backlog row 4.
6. **Which registry K-4 publishes to.** "Publishes that record to the
   registry" does not say which. The mailbox host knows only its own
   (`substrate.registry_url`, and the DHT when enabled). C4-13; backlog
   row 5. The DHT side of "an unchanged record is a refresh" is still
   unverified (task.md §1); backlog row 6.
7. **"Not claimed for the queue lifetime".** A new queue has never been
   claimed. C4-12 counts `register` too.
8. **"Takes an optional passport" (M-1 table).** C4 has no passport field;
   an unknown field is ignored, so a C8 receiver can talk to a C4 mailbox
   and gets open limits, which is what D-1 says for a mailbox without a
   passport.
9. **Same-node calls.** When the receiver (C6) or the sender (C5) runs on
   the mailbox host itself, `ProxyRouter::invoke_inner` serves the call
   locally and passes the `CallerContext` the caller built
   (`proxy/router.rs:213-238`); no handshake runs. The mailbox then needs
   a `CallerContext` with `auth = Delegated`, `caller_did` = the master,
   and `proof` = the instance pubkey and certificate (C4-3). C5 and C6
   must build it so; today `CallOrigin::Native` with no proof would be
   refused with `-32049`. Because no handshake runs, the mailbox also
   checks revocation itself (C4-8). Forward note (§7).
10. **M-12 "only if that certificate is not revoked".** The router checks
    revocation at every stream, but a same-node call (item 9) runs no
    handshake. So the mailbox reads the master anchor on every receiver
    call (C4-8). A revoked newest instance is refused and blocks the queue
    until a newer certificate registers, which is what M-12 asks. The
    price is a second anchor read for wire calls (backlog row 9).
11. **M-14 timing.** "Claimed 5 times without an acknowledgement" could
    mean at the 5th claim or after it. C4-11: after the 5th visibility
    time ends.
12. **Change doc "Store" lists `queues (destination, limits, ...)` and
    `receipts (deposit id, sender, ...)`.** C4 adds per-queue counters,
    the record timestamp and the publish time, and keys receipts on
    `(sender, deposit_id)` (C4-5, C4-6). Same idea, more columns.
13. **`roym-integrated-experience-spec.md` D4 "No third-party
    mailboxes"** is listed in the change doc close-out. It becomes false
    only when the product uses a mailbox (C5/C6). Leave it for the change
    doc close-out after C7.
14. **task.md C4 "hourly record publish (K-4)"** and the change doc's
    task line "Lookup change: ... Mailbox republishes the record (open)":
    both are this slice; Step 8.6 ticks the line.
15. **How a sender learns a queue's ttl limit.** M-4 lists refusals but
    not a ttl over the limit. Only `info` reports limits, and only the
    open ones. C4-16 gives it its own code, `-32050`, with the queue's
    `max_ttl_secs` in `data`, so it stays readable when C8 gives queues
    their own limits.
16. **An expired record at `register`.** A record from `svc deploy` lasts
    30 days (`DEFAULT_ENDPOINT_NOT_AFTER_SECS`), and only the master holder
    can sign a new one. C4-9 refuses an expired record (`-32047`) also for
    an existing queue, which keeps working for `claim`. C6 must handle
    this (§7).
17. **A call with no interface to the node DID.** Not covered by the
    change doc. C4-2 and Step 3.4 keep `mailbox` out of "the one
    interface"; the supervisor's own case stays (backlog row 8).

## 7. Forward notes for later slices (add to task.md in Step 8.8)

- **C5 (sender):** import the wire types and codes from
  `syneroym_core::mailbox`. A `deposit` answer is a `Receipt::Stored`.
  Poll `status` with at most 100 ids per call. A sender on the mailbox host
  itself must present a `CallerContext` with its instance proof (§6 item
  9). How C5 reacts to each refusal is C5's decision; S-4 covers only "the
  mailbox is unreachable". The facts C4 gives it:
  - may succeed later without a change: a transport failure, `-32603`,
    `rate_limited` (`-32043`), `queue_full` (`-32042`), `mailbox_full`
    (`-32044`), `unknown_destination` (`-32040`: the receiver has not
    registered yet, or its queue was removed);
  - will fail again for the same payload: `too_large` (`-32041`; the
    payload is too large for any open mailbox), `ttl_too_long` (`-32050`;
    a new deposit with a ttl at most `data.max_ttl_secs` can succeed);
  - a bug in the sender: `-32602`, `duplicate_deposit` (`-32048`; each
    deposit is sealed again with a new id, S-5, so a correct sender never
    sees it), `not_an_instance` (`-32045`) and `unauthenticated`
    (`-32049`; the caller proof is missing or wrong).
- **C6 (receiver):** register with the stored deploy record; use the
  answer's `limits.max_ttl_secs` for `OpenPolicy` and for the
  idempotency memory (R-7). Ack by `(sender, deposit_id)` from the claim
  answer; at most 32 per call. Re-register when `claim` answers `-32040`
  (the queue was removed). `-32046` means a newer instance holds the
  queue: stop pulling for that service and log it. `-32045` at `register`
  or `claim` can mean the instance key is revoked: stop pulling and wait
  for a renewed certificate (task.md C6). `-32047` at `register` for a queue
  that exists means the stored record is no longer accepted, in practice
  because it is past `not_after` (30 days after deploy): keep claiming
  (the queue keeps its limits and messages), use the limits from the last
  successful `register` (or `info` when there is none), and log a warning
  once an hour that the owner must deploy again to renew the record; the
  mailbox stops republishing it at `not_after`. A receiver on the mailbox
  host must present its instance proof in the `CallerContext` (§6 item 9).
- **C7 (open mailbox list):** before the registry lists open mailboxes
  for strangers, the router must check that a caller holds the instance
  key of its certificate (backlog row "Router does not check that the
  caller holds the temporary key", §5 item 1). The C4-4 risk decision
  holds only until then. If the fix has not landed when C7 starts, C7
  carries it.
- **C8 (dedicated mailbox):** add `passport` to `RegisterParams`; raise
  the queue's `max_bytes`, `max_messages`, `max_ttl_secs` columns from
  the lease (the store already keeps limits per queue); take the
  certificate for `SignerBinding::InstanceCertificate` from
  `CallerContext.proof` (§6 item 2, decided).

## 8. Completion steps

1. Import clean-up pass over every edited file (types by `use`, functions
   by their parent module, no inline paths with several `::`).
2. `cargo +nightly-2026-04-06 fmt --all`.
3. `cargo clippy -q --workspace --all-targets --all-features`.
4. `cargo dupes report` over the changed files (recipe in `AGENTS.md`);
   lower the duplication baseline if the gate asks.
5. `cargo shear` (the new crate's dependencies are all used).
6. Targeted runs while working:
   `cargo nextest run -p syneroym-mailbox`, `-p syneroym-passport`,
   `-p syneroym-core`, `-p syneroym-control-plane`, `-p syneroym-router`,
   `-p syneroym-sdk`, and
   `cargo nextest run -p syneroym-substrate --test mailbox_service_e2e`.
7. Final: `mise run verify` in the background, with no `--skip`; every
   gate must pass. Record the result in `status.md`.
8. Deferred-backlog pass (§5) and living-docs pass (Step 8).
9. Commit on `feat/m06d-c4-implementation` (Conventional Commits, 50/72),
   then a PR whose body has the exact CLA line.

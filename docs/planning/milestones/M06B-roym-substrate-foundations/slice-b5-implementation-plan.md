# M06B Slice B5 — Group Delivery — Implementation Plan

> **Scope.** [task.md](task.md)'s slice B5: *"Gossip DAG with epidemic routing
> and participant relays; total order by `(sender_timestamp, sender_did)`;
> offline catch-up pulled from any online peer; owner-distributed per-epoch
> group key with rekey on every join, every removal, and on a schedule.
> Membership changes are ordinary DAG entries (D5,
> [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md)
> Amendment 1)."*
>
> **Consumer.** M06C (the Roym product), requirement
> [R4 — Private group chat](../../../roym-integrated-experience-spec.md#r4--private-group-chat).
> B5 builds no product service.
>
> **Gates it must close.** [task.md](task.md)'s measurable exit criteria 7, 8,
> 9, and the B5 half of 11; failure-and-security-matrix rows 7, 8, 9, 10, and
> the group half of 6, 12, 13.
>
> **Dependency.** B4 (Complete, 2026-08-21). Everything B4 shipped is verified
> against the tree in §0 below; where the input documents predicted something
> the tree does not show, §14 says so instead of guessing.
>
> **Written against the tree at `1489e92` (clean `main`, 2026-08-24).**

---

## §0 What B4 actually hands B5, verified against the tree

Not taken from [slice-b4-implementation-plan.md](slice-b4-implementation-plan.md)
§15 — re-read from the code, because two of its seven claims are only partly
true (§0.8, §14.2).

### 0.1 The crate

`crates/conversation/` → `syneroym-conversation`, 3216 lines across
`crypto.rs`, `envelope.rs`, `ids.rs`, `lib.rs`, `outbox.rs`, `store.rs`,
`transport.rs`, `wire.rs`. `ConversationService`
([lib.rs:45](../../../../crates/conversation/src/lib.rs#L45)) holds a
`Mutex<HashMap<String, Arc<ConversationStore>>>` keyed by **service id**, one
`conversation.db` per service, opened lazily under `open_lock`.

### 0.2 Ordering is already the three-part key

`(sender_timestamp, author, id)` — the index
`idx_messages_order` and `history`'s `WHERE (sender_timestamp, author, id) >
(?,?,?) ORDER BY sender_timestamp ASC, author ASC, id ASC`
([store.rs:188](../../../../crates/conversation/src/store.rs#L188),
[store.rs:473](../../../../crates/conversation/src/store.rs#L473)). B5's DAG
**must** sort on the same three columns or transcripts are not byte-identical.

### 0.3 Attribution is signature-based, not transport-based

`envelope::canonical_bytes` covers
`(message_id, conversation_id, author, sender_timestamp_ms, content_type,
body)` with a `syneroym:conversation:v1` domain tag and a big-endian length
prefix on `body`
([envelope.rs:29](../../../../crates/conversation/src/envelope.rs#L30)).
`envelope::sign`/`verify` use `ed25519_dalek`. This works identically for a
relayed entry, which is why B5 can reuse it *shape*-wise but not
*byte*-wise — B5's entries carry different fields, so they need their own
domain tag (`D-B5-9`).

### 0.4 Each service has one long-term signing key and one vodozemac account

`local_identity` table: `account_state BLOB` (a vodozemac `AccountPickle`) and
`sig_secret BLOB` (32-byte ed25519 seed), generated once on first use by
`local_identity_or_generate(crypto::generate_identity_bytes)`
([store.rs:600](../../../../crates/conversation/src/store.rs#L600),
[crypto.rs:139](../../../../crates/conversation/src/crypto.rs#L151)). The
ed25519 key is **not** derived from the vodozemac account and neither is
derived from the node identity. B5 signs DAG entries with this same
`sig_secret`.

### 0.5 The 1:1 channel is real X3DH + Double Ratchet

`X3dhDoubleRatchetCrypto` over `vodozemac` 0.10, behind the `SessionCrypto`
trait ([crypto.rs:120](../../../../crates/conversation/src/crypto.rs#L122)).
`sessions` table pins `pinned_sig_key` per `peer_address`; a later envelope
presenting a different key for a pinned address is a hard `PermissionDenied`,
never a silent re-pin
([crypto.rs:337](../../../../crates/conversation/src/crypto.rs#L345)).

### 0.6 The outbox worker and its dispositions

`ConversationService::run_worker(tick, cancel)` loops on
`tokio::time::sleep(tick)` → `drain_once()` → per candidate service
`drain_one` → `queue.claim_due(now, CLAIM_LIMIT_PER_TICK = 16)` → `drain_item`
([outbox.rs:29](../../../../crates/conversation/src/outbox.rs#L29)).
`Disposition` is `Unreachable` (defer, no attempt charged, backoff from
message age), `Terminal(String)`, `Retry`, `Delivered`
([transport.rs:26](../../../../crates/conversation/src/transport.rs#L27)).

### 0.7 The peer-facing transport arm

`conversation` is the seventh entry in
`NATIVE_CAPABILITY_INTERFACES`
([local_registry.rs:40](../../../../crates/core/src/local_registry.rs#L40)) and
the sixth `SynSvcNativeService::dispatch` arm, with exactly two methods —
`prekey-bundle` and `deliver`
([synsvc_native.rs:1519](../../../../crates/control_plane/src/synsvc_native.rs#L1519)).
Outbound calls go through `ConversationService::call_peer`, which refuses
before it dials unless the sending service holds an unexpired instance
certificate **and** a recorded owner
([transport.rs:53](../../../../crates/conversation/src/transport.rs#L53)).

`check_native_capability_gate` refuses a `CallOrigin::Guest` call to a
native-capability interface on a *different* service, and permits it on the
guest's own service id
([proxy.rs:589](../../../../crates/router/src/proxy.rs#L589)). B5's new peer
verbs inherit that gate unchanged; the same-service exemption is handled the
same way B4 handled it — by making the *signature* the invariant, not the
origin (`D-B5-15`).

### 0.8 What B4 did **not** leave behind — corrections to its §15

- **§15 item 5 says "`messages` has no DAG parent column"** — true, and there
  is no `dag_entries` table either. B5 adds both.
- **§15 item 4 says "`conversation-kind::group` and
  `conversation-summary.participants` are already in the WIT"** — true, but
  the *store* hardcodes `'direct'` in three places that B5 must not reuse:
  `get_or_create_direct`'s insert, `insert_incoming_if_absent`'s
  `ON CONFLICT(peer_address) WHERE kind = 'direct'` upsert
  ([store.rs:396](../../../../crates/conversation/src/store.rs#L411)), and the
  partial unique index `idx_conversations_direct_peer`. A group row has no
  `peer_address` at all.
- **`ConversationService::conversations`** synthesises `participants` as
  `[service_id, peer_address]`
  ([lib.rs:255](../../../../crates/conversation/src/lib.rs#L278)). For a group
  it must read `group_members` instead.
- **`derive_conversation_id(a, b)`** is order-independent over an address
  *pair* ([ids.rs:16](../../../../crates/conversation/src/ids.rs#L15)). It
  cannot produce a group id: membership changes, so the id would change with
  it. `D-B5-2`.
- **`peer_deliver_impl` recomputes `derive_conversation_id(svc, author)` and
  refuses any payload whose `conversation_id` differs**
  ([transport.rs:222](../../../../crates/conversation/src/transport.rs#L236)).
  Group-key distribution rides this same path (`D-B5-6`), so that check must
  stay correct for it — it does, because a key-distribution message *is* a 1:1
  message in the owner↔member direct conversation, and only its `body` is
  about a group.

---

## §1 Findings from reading the tree

### F1 — `conversation.db` is already one connection shared with its queue

`ConversationStore::open_encrypted` opens one `Connection`, wraps it in
`Arc<Mutex<..>>`, and hands the same handle to `Queue::from_connection`
([store.rs:138](../../../../crates/conversation/src/store.rs#L138)). Every new
B5 table lives in that same file and can commit atomically with an enqueue
through `Queue::transaction`. **Do not call `self.conn.lock()` inside a
`queue.transaction` closure** — `std::sync::Mutex` is not reentrant and the
node self-deadlocks. `insert_incoming_if_absent`'s doc comment already carries
this warning; every B5 method that takes a `&Transaction` inherits it.

### F2 — `Queue::defer` un-counts the claim, so it has no surviving counter

`defer` sets `visible_at` and does `claim_count = claim_count - 1`
([lib.rs:384](../../../../crates/async_queue/src/lib.rs#L384)). B4 works around
the missing counter by deriving backoff from the message's own age
(`backoff_for_age`, [outbox.rs:157](../../../../crates/conversation/src/outbox.rs#L156)).
B5's relay and sync ticks are **not** queue items and need no such workaround —
they are plain per-tick work (`D-B5-12`).

### F3 — `claim_due` is bounded at 16 items per service per tick

`CLAIM_LIMIT_PER_TICK`
([outbox.rs:25](../../../../crates/conversation/src/outbox.rs#L24)). Group
fan-out multiplies outbox rows by the member count, so a 20-member group posting
one message produces 19 rows and takes two ticks to drain. At the default
`conversation_tick_secs = 5` that is 10 seconds. **Raise `CLAIM_LIMIT_PER_TICK`
to 64** (`D-B5-13`) rather than adding a second queue.

### F4 — `candidate_service_ids` rediscovers a service by its `conversation.db` on disk

It unions the already-open stores with every registry endpoint whose interface
is `conversation` and whose service dir holds a `conversation.db`
([lib.rs:180](../../../../crates/conversation/src/lib.rs#L188)). B5's sync and
rekey ticks reuse it unchanged — a restart therefore resumes group work with no
guest call needed, which is what failure-matrix row 4 demands for groups too.

### F5 — the only cheap identity a member can prove is an ed25519 signature

`CallerContext.caller_did` on an inbound peer call is the **owner's Master
DID**, which cannot distinguish two Conversation services one owner runs
(B4's F16, still true — nothing in the tree changed it). So B5's peer verbs
cannot authorize on the transport caller. They authorize on a signature by the
caller's own conversation signing key, checked against the key the group owner
vouched for in a membership entry (`D-B5-10`).

### F6 — there is no non-member relay in this architecture

[ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §4 says
*"Online members store the message"* and *"the group sustains its own data
availability"*. Every relay is a member. That removes a whole class of design
(blind relays, relay-visible metadata bounds) and lets membership entries be
signed-but-unencrypted (`D-B5-5`).

### F7 — `aes-gcm` and `hkdf` are already workspace dependencies

[Cargo.toml:133](../../../../Cargo.toml#L133), `aes-gcm = "0.10"`;
`hkdf = "0.12"`. Neither is a new dependency for the workspace, so B5 adds no
`cargo deny` / `cargo audit` surface. `blake3` (entry ids) and `rand` are
already `syneroym-conversation` dependencies.

### F8 — the parity suite compares byte-for-byte over a fixed scenario table

`SCENARIOS` is a `&[(&str, &str)]` of request JSON strings, run through both
drivers and `assert_eq!`d as `Vec<(&str, String)>`
([dual_build_parity.rs:328](../../../../crates/app_host_native/tests/dual_build_parity.rs#L328)).
**Only deterministic scenarios belong in that table** — the existing comment
says exactly why `send-message` is excluded (its message id carries a random
nonce). Group ids carry a random nonce too (`D-B5-2`), so `create-group` goes
in a named per-build test, not in `SCENARIOS`.

### F9 — `conversation_prekey_pool_size` is a dead config field

Declared and defaulted at
[config.rs:603](../../../../crates/core/src/config.rs#L603), never read
anywhere in `crates/` or `apps/`. `crypto.rs` generates **one** one-time key on
demand instead ([crypto.rs:257](../../../../crates/conversation/src/crypto.rs#L259)).
Not B5's defect, but B5 touches this config block and should not leave it. See
§14.5.

### F10 — the fixture's `app.rs` carries slice ids in code comments

`/// M06B slice B4: idempotent ...` and three more like it in
[app.rs](../../../../test-components/dual-build-fixture/src/app.rs). This
violates AGENTS.md's **No Planning-Doc References in Code**. B5 edits that file
anyway; delete them in the same pass (§12 step 4).

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-B5-1** | **B5 ships as one commit.** No a/b split. | B4 split because a licence question blocked the crypto module. Nothing here is blocked on an external answer; `aes-gcm` is already in the workspace (F7). |
| **D-B5-2** | **A group id is minted by its owner, not derived from participants.** `derive_group_id(owner_address, created_at_ms, nonce: [u8;16]) -> "conv:" + hex(blake3(b"group" ‖ 0 ‖ owner ‖ 0 ‖ created_be ‖ nonce))`. Every other member adopts the owner's id verbatim from the genesis entry. | `derive_conversation_id` is order-independent over a *pair* (§0.8); membership changes, so a derived group id would change with it. The `conv:` prefix is kept so the WIT type stays opaque and one shape. |
| **D-B5-3** | **The DAG lives in `conversation.db`**, in new `dag_entries` / `dag_parents` tables on the same connection as `messages` and the queue. Not a second store, not `blob-store`. | Settles [task.md](task.md)'s second open design point. `blob-store` is content-addressed and right-shaped for immutable entries, but an entry must be inserted atomically with the `messages` row it decrypts into and with a queue row — which needs one transaction on one connection (F1). B4's `D-B4-13` already made this call for the same reason. |
| **D-B5-4** | **The entry, not the plaintext, is what is stored and relayed.** `dag_entries` holds the signed header, the AES-GCM ciphertext, and the signature exactly as received; `messages` holds the decrypted body under the service DEK, written when (and only when) the epoch key is available. | A relay must forward the *original* bytes — re-encrypting would invalidate the author's signature, and the signature is the whole of B5's attribution (§0.3). It also makes "store now, decrypt when the key arrives" representable, which ADR-0013 §5 calls an expected transient convergence window. |
| **D-B5-5** | **Membership entries are signed but not encrypted.** A `membership` entry's payload (`action`, `subject_address`, `subject_sig_key`, `new_epoch`, `member_list_hash`) is plaintext inside the DAG. Message entries are encrypted. | F6: the DAG only circulates among members, so this is not a public leak. It is what lets a joiner verify the chain that admitted it and reach the *identical membership history* R4's acceptance test demands — impossible if the join event were encrypted under an epoch the joiner may not read. Accepted cost, stated in the ADR amendment (§13): a just-removed member that a lagging peer still pushes to can read later membership metadata (never message content). |
| **D-B5-6** | **The group key rides B4's 1:1 ratchet**, as an ordinary outbox message with `content_type = "application/vnd.syneroym.group-key+json"` and a new `system` flag that hides it from `history` and `outbox`. It gets no second channel, no second session, no second retry policy. | Settles [task.md](task.md)'s fourth open design point, which asks for this to be answered rather than inherited. ADR-0013 Amendment 1 says the owner distributes the key *"over the 1:1 channel from Decision 3"*. The objection worth naming — key material sharing a ratchet with content — is not a real distinction here: anyone who breaks that ratchet already reads the content it carries. What reuse buys is exactly what key distribution needs and would otherwise be rebuilt: durability across restart, `pending`/`delivered` state, the unreachable-peer defer rule, and an already-pinned peer signing key. |
| **D-B5-7** | **The owner is the single trust root for a group.** A membership `add` entry carries the joiner's `sig_key`, signed by the owner. Members verify every group author against the key the owner vouched for. Members do **not** independently trust-on-first-use each other's keys. | Settles B4 §15 item 7, which required an explicit answer. Amendment 1 already makes the owner the single point of trust for key distribution; making it also the vouching authority for signing keys adds no new trust and removes N-way TOFU, whose failure mode (one member pins a wrong key and silently diverges) is undetectable. The joiner's own root is the owner's `sig_key`, pinned by the 1:1 session that delivered the epoch key — a key it already pinned under B4's rules. |
| **D-B5-8** | **Spread is three mechanisms, each cheap: author fan-out (durable), relay push (per-tick, best-effort, fanout 3), peer pull (per-tick, one peer, cursor-based).** | ADR-0013 §4 literally describes push-from-author plus pull-from-peers; the relay push is what makes the group converge while the author is offline. Keeping relay push non-durable (a `relay_pending` flag on the entry, cleared after one pass) is what keeps it from becoming N² durable rows — the pull path is the durable guarantee, so the push does not need to be. |
| **D-B5-9** | **Sync is a per-peer cursor over the responder's local insertion sequence** (`dag_entries.seq`, `INTEGER PRIMARY KEY AUTOINCREMENT`), never over the total sort order. | A late-arriving entry sorts *before* the requester's total-order cursor and would be skipped forever. Local insertion order is monotonic on the responder, so a cursor over it can never miss an entry that peer holds, whatever order it arrived in. The cursor is stored per `(conversation, peer)` because `seq` is local to each store. |
| **D-B5-10** | **Every peer verb carries a `PeerAssertion`** — `{address, sig_key, timestamp_ms, nonce, signature}` over a domain-tagged canonical encoding — verified against the vouched key for `address` in that group's membership. A non-member is refused. | F5: the transport caller DID cannot identify a service. This is the same signature-based authorization B4 established, applied to the two new verbs. |
| **D-B5-11** | **An entry whose `sender_timestamp` exceeds the receiver's clock by more than `conversation_max_clock_skew_secs` is refused outright — not stored, and not forwarded.** | Settles B4 §15 item 6. Forwarding what you refuse guarantees divergence: some members accept the entry and some do not, and the transcripts stop being byte-identical, which is exit criterion 7. Uniform refusal is the only rule that preserves convergence. Accepted limit, recorded in the backlog: two members whose clocks differ by more than the bound will diverge on entries near it. |
| **D-B5-12** | **No new worker task and no new `RolesConfig` role.** The relay pass, the sync pass, and the scheduled-rekey pass run inside the existing `ConversationService::run_worker` loop, on their own interval counters. | `D-B4-19`'s reasoning unchanged: the knobs live on `AppSandboxRole` beside the `conversation_*` ones and the worker already has the right lifecycle, cancellation, and shutdown handling in `runtime.rs`. A second task would need all of that duplicated for no gain. |
| **D-B5-13** | **`CLAIM_LIMIT_PER_TICK` rises from 16 to 64.** | F3. Group fan-out multiplies outbox rows by member count; 16 makes a 20-member group take two ticks per message for no reason. 64 keeps the same "one service cannot spend the whole tick budget" property at a group-shaped scale. |
| **D-B5-14** | **A group message is `delivered` only when every current member's fan-out row has completed; `pending` until then; `failed` if any row gives up.** Per-member state lives in a new `message_recipients` table. | Failure-matrix row 3 (*"never shown as delivered while pending"*) is the same promise for a group as for 1:1, and D4's "no third party holds it" means an unreachable member genuinely leaves the message undelivered. Reporting `delivered` on first ack would be the lie row 3 forbids. |
| **D-B5-15** | **The same-service exemption is bounded by the signature, not by an origin check** — `group-push` refuses any entry whose `author` equals this service's own address, and `group-sync` refuses a `PeerAssertion` whose `address` equals it. | `D-B4-26`'s rule, extended verbatim. `NativeInvocation` still carries no origin, and a signature invariant survives into a relayed entry where an origin check never could. |
| **D-B5-16** | **`create-group` takes no title and `conversation-summary` gains no field.** A group's display metadata is the guest's own business, held in `data-layer`. | B4 §15 item 4 predicted B5 adds verbs, not record changes. A title is the one field everyone wants and the one that starts an unbounded list (icon, description, pinned message). M06C can hold it in its own collection at zero cost to the host interface. Revisit only if M06C shows a reason the host must know it. |
| **D-B5-17** | **`membership-event` is a new WIT record**, and the only one B5 adds. | R4's acceptance test is *"every member's membership history is identical after sync"*, which needs the history readable. A fingerprint hash would satisfy the test and leave M06C unable to render "Alice added Bob" — the wrong trade for one record. |
| **D-B5-18** | **A direct conversation auto-created solely to distribute a group key is flagged `system` and hidden from `conversations()`** until the guest itself calls `open-direct` on that peer, which clears the flag. | Distributing a key to a member the guest has never messaged would otherwise make an unrequested 1:1 conversation appear in the guest's list. The flag is one column and one predicate; the alternative (a second session table for key traffic) is a parallel B4. |
| **D-B5-19** | **No `on-membership-change` guest export.** `guest-api` keeps its two functions. | A membership change is not a message and no acceptance test needs a push for it; `membership-history` is a poll away and M06C already polls `conversations`. Adding a third export costs a new `notify_guest_*` path plus its four-attempt retry in `engine.rs` on both builds, for a notification nobody has asked for. Recorded as a backlog row with a pickup trigger (§13). |
| **D-B5-20** | **`sync-now` runs one *budgeted* round, not a complete one, and says so in the WIT.** `call_peer` gains a `timeout: Duration` parameter; `sync-now` passes 2 s per peer under a 3 s total budget (`conversation_sync_now_budget_ms`), visits members in address order until the budget is spent, logs and skips a failing peer, and returns `Ok`. The background pass (§6.16) passes 10 s per peer. | **A guest entry point is bounded by `dispatch_epoch_timeout_secs`, which defaults to 5 seconds** ([config.rs:449](../../../../crates/core/src/config.rs#L449)), and `call_peer` currently hardcodes a 30-second timeout ([transport.rs:91](../../../../crates/conversation/src/transport.rs#L91)). A `sync-now` that waited for every member would trap the calling guest on the first unreachable one — the exact shape backlog row 253 already records for `proxy.call`, and the same local workaround B4 applied for `step`. Making the *contract* "one bounded round" rather than "a complete round" is what keeps the guarantee honest: convergence is the periodic pass's job, and `sync-now` only makes it prompt. |

---

## §3 The WIT package

**File:** `crates/wit_interfaces/wit/conversation/conversation.wit` (edit).
Additive only — no existing type, function, or world changes.

Add inside `interface conversation`, after the `history-page` record:

```wit
    /// One membership change, as it appears in the group's own DAG. Signed
    /// by the group's owner and never encrypted: a member that has just
    /// joined must be able to verify the chain that admitted it, which it
    /// could not do if the entry were sealed under an epoch it may not
    /// read.
    record membership-event {
        /// The DAG entry this change came from. Stable, content-derived,
        /// and identical on every member's substrate.
        entry: string,
        /// `add` or `remove`. A string, not an enum: a future action
        /// (`transfer-ownership`) must not break a guest's match, and this
        /// value is already carried inside a signed entry a guest cannot
        /// influence.
        action: string,
        /// The member being added or removed, as a routing service id --
        /// the same namespace `open-direct` takes.
        subject: string,
        /// The epoch this change opened. Every message entry at or after
        /// it is encrypted under a different key than the one before.
        epoch: u64,
        /// The owner's own clock when it issued the change, in Unix
        /// milliseconds. Ordered by the same rule as messages.
        sender-timestamp: s64,
    }
```

Add, after `retry`:

```wit
    /// Creates a group owned by this service and returns its id. The
    /// caller is its first and, until `add-member`, only member.
    create-group: func() -> result<conversation-id, conversation-error>;

    /// Owner-only. Adds `member-address` and opens a new epoch, whose key
    /// reaches every member -- including the joiner -- over the 1:1
    /// channel. The joiner cannot read anything from before the change:
    /// earlier entries are sealed under earlier epochs.
    add-member: func(
        conversation: conversation-id,
        member-address: string,
    ) -> result<_, conversation-error>;

    /// Owner-only. Removes `member-address` and opens a new epoch whose
    /// key reaches everyone except them.
    remove-member: func(
        conversation: conversation-id,
        member-address: string,
    ) -> result<_, conversation-error>;

    /// Current members, sorted. Same namespace as
    /// `conversation-summary.participants`, which reports the same list.
    members: func(conversation: conversation-id)
        -> result<list<string>, conversation-error>;

    /// Every membership change this substrate has observed, oldest first,
    /// ordered by the same (sender-timestamp, author, entry) rule messages
    /// use -- one ordering rule for the whole conversation, not two.
    /// Identical on every member once synced.
    membership-history: func(conversation: conversation-id)
        -> result<list<membership-event>, conversation-error>;

    /// Pulls whatever peers hold and this substrate does not, now, instead
    /// of waiting for the next scheduled pass. Runs one round under a
    /// short time budget and returns `ok` whether or not that round
    /// reached every member -- a caller's own time budget is small, and an
    /// unreachable member must not spend it. Convergence is the periodic
    /// pass's guarantee; this only makes it prompt. Call it again to make
    /// further progress.
    sync-now: func(conversation: conversation-id) -> result<_, conversation-error>;
```

**Thirteen functions total** (seven from B4, six new). The three worlds are
unchanged. `guest-api` is unchanged (`D-B5-19`).

**Type-vocabulary check (`D-B3-2`):** every new type is `string`, `u64`,
`s64`, `list`, `record`, or `result`. No resources.

**Deliberately not in the WIT:** the group key, the epoch number on a
`message`, the entry id of a message, `parents`, and the signature. All are
Layer 3 concerns a guest cannot act on; the guest's window on verification
stays the existing `message.verified` bool.

---

## §4 Bindings modules

No hand-written change. `crates/wit_interfaces/src/conversation.rs`
(`wit_bindgen::generate!`) and `conversation_host.rs`
(`wasmtime::component::bindgen!`) regenerate from the `.wit` file. Verify after
editing the WIT that `cargo build -p syneroym-wit-interfaces` is clean before
touching anything downstream.

One edit, in `crates/app_host/src/types.rs`:

```rust
pub mod conversation {
    pub use syneroym_wit_interfaces::conversation::syneroym::conversation::conversation::{
        ConversationError, ConversationKind, ConversationSummary, DeliveryState, HistoryPage,
        MembershipEvent, Message,
    };
}
```

---

## §5 `syneroym-rpc` — the plain-type surface

**File:** `crates/rpc/src/conversation.rs` (edit).

New type, beside `ConversationSummary`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMembershipEvent {
    pub entry: String,
    pub action: String,
    pub subject: String,
    pub epoch: u64,
    pub sender_timestamp: i64,
}
```

Six new guest-facing methods and two new peer-facing methods on
`ConversationHost`:

```rust
    async fn create_group(&self, service_id: &str) -> Result<String, ConversationError>;

    async fn add_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError>;

    async fn remove_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError>;

    async fn members(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<Vec<String>, ConversationError>;

    async fn membership_history(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<Vec<ConversationMembershipEvent>, ConversationError>;

    async fn sync_now(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<(), ConversationError>;

    /// Peer-facing: accepts DAG entries pushed by another member. The
    /// bytes are a serde-encoded `GroupPushRequest`; both ends agree on
    /// the encoding, so this trait need not name it.
    async fn group_push(
        &self,
        service_id: &str,
        requester_did: &str,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError>;

    /// Peer-facing: serves entries this substrate holds past the
    /// requester's cursor. Bytes are a serde-encoded `GroupSyncRequest`
    /// / `GroupSyncResponse`.
    async fn group_sync(
        &self,
        service_id: &str,
        requester_did: &str,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError>;
```

`ConversationNotifier` is unchanged (`D-B5-19`).

---

## §6 Host implementation — `syneroym-conversation`

### 6.1 New modules

| File | Contents |
|---|---|
| `src/group.rs` (new) | `ConversationService`'s six guest-facing group methods, the epoch/rekey logic, and the apply-entry pipeline. |
| `src/dag.rs` (new) | `DagEntry`, its canonical byte encoding, sign/verify, entry-id derivation, and the `GroupPushRequest`/`GroupSyncRequest`/`GroupSyncResponse`/`PeerAssertion` wire types. |
| `src/store.rs` (edit) | Six new tables, six new columns, ~14 new methods. |
| `src/transport.rs` (edit) | `group_push_impl`, `group_sync_impl`, `push_entries_to`, `sync_with_peer`. |
| `src/outbox.rs` (edit) | Group fan-out completion in `drain_item`; the relay/sync/rekey passes in `run_worker`. |
| `src/lib.rs` (edit) | The `ConversationHost` impl's eight new methods; `send` branches on conversation kind; `send_system`. |
| `src/ids.rs` (edit) | `derive_group_id`, `derive_entry_id`. |

New dependencies in `crates/conversation/Cargo.toml`: `aes-gcm.workspace =
true`. (`blake3`, `rand`, `ed25519-dalek`, `serde_json`, `hex` are already
there. `hkdf` is **not** needed — the epoch key is used directly as the
AES-256-GCM key, not derived from a shared secret.)

### 6.2 Schema (`store.rs::init_schema`)

Two distinct operations on the same `execute_batch`, and they are not the same
edit: **new columns are written into the existing `CREATE TABLE` statements in
place**, and **new tables are appended to the end of the batch**. **There is no
migration and none is wanted** — the product is unreleased. A developer holding
an older `conversation.db` deletes it; state this in the commit message, because
`CREATE TABLE IF NOT EXISTS` will silently leave an existing file without the
new columns and the failure surfaces later as a query error.

Changes to existing tables:

```sql
-- conversations: three new columns
    owner_address TEXT,                        -- NULL for direct
    current_epoch INTEGER NOT NULL DEFAULT 0,
    system        INTEGER NOT NULL DEFAULT 0   -- D-B5-18

-- messages: two new columns
    system   INTEGER NOT NULL DEFAULT 0,       -- D-B5-6, hidden from history/outbox
    entry_id TEXT                              -- the DAG entry, NULL for direct
```

`idx_conversations_direct_peer` is unchanged: it is `WHERE kind = 'direct'`, so
group rows (with `peer_address IS NULL`) never enter it.

New tables:

```sql
CREATE TABLE IF NOT EXISTS group_members (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    member_address  TEXT NOT NULL,
    sig_key         BLOB NOT NULL,
    joined_epoch    INTEGER NOT NULL,
    removed_epoch   INTEGER,
    PRIMARY KEY (conversation_id, member_address)
);

CREATE TABLE IF NOT EXISTS group_epochs (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    epoch           INTEGER NOT NULL,
    key             BLOB NOT NULL,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, epoch)
);

CREATE TABLE IF NOT EXISTS dag_entries (
    seq              INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id         TEXT NOT NULL UNIQUE,
    conversation_id  TEXT NOT NULL,
    author           TEXT NOT NULL,
    sender_timestamp INTEGER NOT NULL,
    epoch            INTEGER NOT NULL,
    kind             TEXT NOT NULL,   -- 'message' | 'membership'
    header           BLOB NOT NULL,   -- the canonical signed bytes; also the AEAD AAD
    ciphertext       BLOB,            -- NULL for 'membership'
    nonce            BLOB,            -- 12 bytes; NULL for 'membership'
    payload          TEXT,            -- membership JSON; NULL for 'message'
    signature        BLOB NOT NULL,
    applied          INTEGER NOT NULL DEFAULT 0,
    relay_pending    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_dag_order
    ON dag_entries(conversation_id, sender_timestamp, author, entry_id);
CREATE INDEX IF NOT EXISTS idx_dag_unapplied
    ON dag_entries(conversation_id, applied);
CREATE INDEX IF NOT EXISTS idx_dag_relay
    ON dag_entries(relay_pending);

CREATE TABLE IF NOT EXISTS dag_parents (
    child_entry_id  TEXT NOT NULL,
    parent_entry_id TEXT NOT NULL,
    PRIMARY KEY (child_entry_id, parent_entry_id)
);
CREATE INDEX IF NOT EXISTS idx_dag_parents_parent
    ON dag_parents(parent_entry_id);

CREATE TABLE IF NOT EXISTS sync_cursors (
    conversation_id TEXT NOT NULL,
    peer_address    TEXT NOT NULL,
    last_seq        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, peer_address)
);

CREATE TABLE IF NOT EXISTS message_recipients (
    message_id     TEXT NOT NULL REFERENCES messages(id),
    member_address TEXT NOT NULL,
    state          TEXT NOT NULL,   -- 'pending' | 'delivered' | 'failed'
    last_error     TEXT,
    PRIMARY KEY (message_id, member_address)
);
CREATE INDEX IF NOT EXISTS idx_message_recipients_state
    ON message_recipients(message_id, state);
```

**`seq` and `AUTOINCREMENT`:** the keyword matters. Without it SQLite may reuse
a rowid freed by a delete, and a reused `seq` would make a sync cursor skip an
entry. With it, `sqlite_sequence` guarantees monotonicity.

#### 6.2.1 Four existing queries change, and every one of them is a leak if missed

Adding a column changes nothing on its own. These four `SELECT`s already exist,
already return the wrong answer once group and system rows are in the tables,
and each is a *silent* wrong answer — nothing fails, the guest just sees rows it
should not. Name them in the diff rather than trusting "~14 new methods" to
cover them.

| Function | Today | Change | Why it is not cosmetic |
|---|---|---|---|
| [`ConversationStore::history`](../../../../crates/conversation/src/store.rs#L473) | filters on `conversation_id` and the cursor only | add `AND system = 0` | Otherwise every group-key distribution payload appears in the owner's own 1:1 history with the peer, as an opaque JSON blob the guest cannot render (`D-B5-6`). |
| [`ConversationStore::outbox_messages`](../../../../crates/conversation/src/store.rs#L525) | `WHERE state IN ('pending','failed')` | add `AND system = 0` | R1's user-visible outbox would show one entry per member per rekey. On a 20-member group that is 19 rows the user never sent. |
| [`ConversationStore::list_conversations`](../../../../crates/conversation/src/store.rs#L277) | no `WHERE` at all; selects five columns | add `WHERE system = 0`; also select `owner_address` and `current_epoch` into `ConversationRow` | `D-B5-18`. Without the filter, a conversation the guest never opened — created only to carry a key — shows up in its list. |
| [`ConversationService::conversations`](../../../../crates/conversation/src/lib.rs#L269) | hardcodes `participants = [service_id, peer_address]` | branch on `kind`: `Direct` keeps the existing synthesis, `Group` reads `store.current_members(row.id)` (already sorted) | §0.8. A group row has `peer_address = NULL`, so the existing code would report a one-element participant list for every group. |

`ConversationRow` gains `owner_address: Option<String>` and
`current_epoch: u64`. `get_conversation` selects them too — §6.7 and §6.8 both
read `conv.owner_address` / `conv.current_epoch` off it.

#### 6.2.2 `open_direct` clears the `system` flag (`D-B5-18`)

The flag's whole point is that it is temporary. `get_or_create_direct` gains one
statement, run whether the row was found or created:

```sql
UPDATE conversations SET system = 0 WHERE id = ?1 AND system = 1
```

`enqueue_direct` (§6.9) is what *sets* it, and only when it creates the row —
never on an existing one, so a conversation the guest already opened is not
re-hidden by the next rekey. Tested in §11.1 and §11.2; without the `UPDATE` a
key-distribution conversation stays invisible forever and the guest can never
message that peer's own list entry into existence.

### 6.3 Ids (`ids.rs`)

```rust
/// Minted by the owner, adopted verbatim by every other member. Not
/// derived from the member list: membership changes and the id must not.
#[must_use]
pub fn derive_group_id(owner_address: &str, created_at_ms: i64, nonce: &[u8; 16]) -> String {
    let mut h = Hasher::new();
    h.update(b"group");
    h.update(&[0u8]);
    h.update(owner_address.as_bytes());
    h.update(&[0u8]);
    h.update(&created_at_ms.to_be_bytes());
    h.update(nonce);
    format!("conv:{}", hex::encode(h.finalize().as_bytes()))
}

/// Content-derived over the entry's own signed header, so two substrates
/// that hold the same entry compute the same id and dedup on it.
#[must_use]
pub fn derive_entry_id(header: &[u8]) -> String {
    format!("ent:{}", hex::encode(blake3::hash(header).as_bytes()))
}
```

### 6.4 The entry and its canonical bytes (`dag.rs`)

```rust
/// One DAG entry, exactly as it travels. `header` is the signed byte
/// string; every other field is a decoded view of it, and the receiver
/// re-encodes and compares rather than trusting the decoded copy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WireEntry {
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub epoch: u64,
    pub kind: EntryKind,            // Message | Membership
    pub parents: Vec<String>,       // entry ids, sorted, at most MAX_PARENTS
    /// `Message`: AES-256-GCM ciphertext under the epoch key.
    /// `Membership`: absent; `payload` carries the plaintext instead.
    pub ciphertext: Option<Vec<u8>>,
    pub nonce: Option<[u8; 12]>,
    pub payload: Option<MembershipPayload>,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MembershipPayload {
    pub action: String,             // "add" | "remove"
    pub subject_address: String,
    pub subject_sig_key: [u8; 32],
    pub new_epoch: u64,
    /// blake3 over the sorted member list after this change, so a member
    /// can detect that it applied a different set than the owner meant.
    pub member_list_hash: String,
}

pub const MAX_PARENTS: usize = 8;
```

`canonical_entry_bytes` — the signed string and the AEAD AAD, in one function
so the two can never drift:

```
b"syneroym:conversation:dag:v1"  0
conversation_id                  0
author                           0
be(sender_timestamp_ms)          0
be(epoch)                        0
kind_tag ("message"|"membership")0
be(parents.len() as u64)
for each parent (already sorted): be(len) ‖ parent_bytes
be(ciphertext_or_payload.len())  ‖ bytes          // ciphertext for message,
                                                  // canonical JSON for membership
be(nonce.len()) ‖ nonce                           // 0-length for membership
```

Every variable-length field is length-prefixed, for the reason
`envelope::canonical_bytes` already gives: a bare `0x00` separator lets an
attacker who chooses two adjacent fields' bytes move the boundary between them.

`sign_entry(signing_key, header) -> [u8;64]` and
`verify_entry(verifying_key, header, sig) -> bool` are one-liners over
`ed25519_dalek`.

**The genesis entry.** `create-group` writes a `membership` entry with
`action = "add"`, `subject_address = owner`, `epoch = 1`, `parents = []`. It is
the root every member's chain walks back to, and it is why the owner's own
membership is a DAG fact rather than an assumption.

### 6.5 Encryption

```rust
fn seal(epoch_key: &[u8; 32], header: &[u8], plaintext: &[u8])
    -> Result<(Vec<u8>, [u8; 12])>
{
    // AES-256-GCM, random 12-byte nonce per entry, AAD = header.
    // The header is not yet complete when this is called -- see the
    // ordering note below.
}
```

**Ordering note (this is the subtle part).** The header contains the
ciphertext length and the ciphertext contributes to the entry id, while the AAD
must be the header. Resolve it by building the header in two passes:

1. Build `header_prefix` = everything up to and including the parents block.
   This is the AAD.
2. `seal(key, header_prefix, plaintext)` → `(ciphertext, nonce)`.
3. `header = header_prefix ‖ be(ciphertext.len()) ‖ ciphertext ‖ be(12) ‖ nonce`.
4. `entry_id = derive_entry_id(&header)`; `signature = sign(header)`.

The receiver reverses it: split the stored `header` at the prefix boundary
(which it can, because every field is length-prefixed), use the prefix as AAD,
`open` the ciphertext. Write `canonical_entry_prefix` and
`canonical_entry_bytes` as two functions in `dag.rs`, with the second calling
the first, so the split point is defined once.

### 6.6 `create-group` (pseudo-code, `group.rs`)

```
create_group(service_id):
    store  = store_for(service_id)
    ident  = store.local_identity_or_generate(generate_identity_bytes)
    sk     = SigningKey::from_bytes(ident.sig_secret)
    now    = now_ms()
    nonce  = random 16 bytes
    gid    = derive_group_id(service_id, now, nonce)

    epoch_key = random 32 bytes

    payload = MembershipPayload {
        action: "add",
        subject_address: service_id,
        subject_sig_key: sk.verifying_key().to_bytes(),
        new_epoch: 1,
        member_list_hash: hash_members([service_id]),
    }
    entry = build_membership_entry(&sk, gid, service_id, now, 1, parents=[], payload)

    store.queue().transaction(|tx, _| {
        insert conversations
            (id=gid, kind='group', peer_address=NULL, owner_address=service_id,
             current_epoch=1, created_at=now, last_activity=now, system=0)
        insert group_epochs (gid, 1, epoch_key, now)
        insert group_members (gid, service_id, sk.verifying_key(), joined_epoch=1,
                              removed_epoch=NULL)
        insert_entry(tx, gid, &entry, applied=1, relay_pending=0)
        Ok(())
    })
    return gid
```

No network. `create-group` cannot fail on reachability.

### 6.7 `add-member` / `remove-member` (pseudo-code)

```
change_membership(service_id, conversation, member_address, action):
    store = store_for(service_id)
    conv  = store.get_conversation(conversation)? or NotFound
    if conv.kind != Group:            return InvalidArgument("not a group")
    if conv.owner_address != service_id: return PermissionDenied
    if member_address == service_id:  return InvalidArgument("the owner is
                                          always a member")

    members = store.current_members(conversation)
    if action == add:
        if members.contains(member_address): return Ok        // idempotent
        if members.len() >= config.max_group_members: return QuotaExceeded
        // D-B5-7: the owner vouches for the joiner's key, so it must learn
        // it first. The prekey bundle is the one place a service publishes
        // it, and it is already authenticated (self-signed, D-B4-15).
        bundle = fetch_prekey_bundle(service_id, member_address)?   // Unreachable on failure
        subject_sig_key = bundle.sig_key
    else:
        if !members.contains(member_address): return Ok            // idempotent
        subject_sig_key = store.member_sig_key(conversation, member_address)?

    new_epoch = conv.current_epoch + 1
    next_members = members with member_address added/removed
    payload = MembershipPayload { action, member_address, subject_sig_key,
                                  new_epoch, hash_members(next_members) }
    now   = now_ms()
    heads = store.heads(conversation)                 // capped at MAX_PARENTS
    entry = build_membership_entry(&sk, conversation, service_id, now,
                                   new_epoch, heads, payload)
    new_key = random 32 bytes

    store.queue().transaction(|tx, txq| {
        insert_entry(tx, conversation, &entry, applied=1, relay_pending=1)
        apply_membership(tx, conversation, &payload)   // upsert group_members,
                                                       // set removed_epoch on remove
        insert group_epochs (conversation, new_epoch, new_key, now)
        update conversations set current_epoch = new_epoch, last_activity = now
        // Distribute to everyone in `next_members` except self. On a remove,
        // that excludes the removed member by construction -- which is the
        // whole of failure-matrix row 7.
        for m in next_members, m != service_id:
            enqueue_system_message(tx, txq, m, GROUP_KEY_CONTENT_TYPE,
                                   key_payload(conversation, new_epoch, new_key,
                                               next_members, service_id))
        Ok(())
    })
```

`fetch_prekey_bundle` is the existing `call_peer(svc, member, "prekey-bundle",
{}, None)` from `transport.rs`, mapping `Disposition` onto
`ConversationError::Unreachable`. **This makes `add-member` require the joiner
to be reachable**, which Amendment 1 already accepts (*"the owner must be
online for a join or removal to take effect"*) and which is the honest failure —
the alternative is admitting a member whose signing key nobody can verify.
`remove-member` needs no network.

`enqueue_system_message(tx, txq, peer, content_type, body)` is `send`'s body
factored out (§6.9) with `system = 1`, going through the owner↔member *direct*
conversation, creating it with `system = 1` if absent (`D-B5-18`).

### 6.8 `send` on a group conversation

`ConversationHost::send` ([lib.rs:274](../../../../crates/conversation/src/lib.rs#L294))
branches after loading the conversation row:

```
send(service_id, conversation, content_type, body):
    ... existing body-size check ...
    conv = store.get_conversation(conversation)? or NotFound
    match conv.kind:
      Direct: <unchanged: existing code path>
      Group:  send_group(service_id, store, conv, content_type, body)

send_group(service_id, store, conv, content_type, body):
    members = store.current_members(conv.id)
    if members.len() <= 1: return InvalidArgument("a group with no other
                                member has nowhere to deliver")
    (epoch, key) = store.epoch_key(conv.id, conv.current_epoch)? or
                   Internal("no key for the current epoch")
    now    = now_ms()
    heads  = store.heads(conv.id)
    entry  = build_message_entry(&sk, conv.id, service_id, now, epoch, heads,
                                 key, encode_body(content_type, body))
    // `message_id` IS `entry.entry_id`. One id, not two: the entry id is
    // already content-derived and unique, and a second id would need its
    // own dedup index for nothing.
    store.queue().transaction(|tx, txq| {
        enforce per-conversation pending and message bounds   // as today
        insert_entry(tx, conv.id, &entry, applied=1, relay_pending=0)
        insert messages row (id=entry_id, entry_id=entry_id, author=service_id,
                             state='pending', outgoing=1, verified=1, system=0)
        for m in members, m != service_id:
            insert message_recipients (entry_id, m, 'pending', NULL)
            txq.enqueue(tx, group_key = conv.id,
                        queue_key = format!("{entry_id}:{m}"),
                        payload = OutboxItem { message_id: entry_id,
                                               peer_address: m,
                                               group: Some(conv.id) },
                        now)
        Ok(())
    })
    return entry_id
```

`relay_pending = 0` on the author's own entry: the author already fans out to
everyone durably, so a relay pass on top would double the traffic.

**`OutboxItem` gains `group: Option<String>`** (`#[serde(default)]`, since the
field is absent on every 1:1 item). That one field is what `drain_item` reads to
decide whether it is settling a message or a recipient.

### 6.9 `send_system` (`lib.rs`)

The existing `send` body from `let identity = ...` to
`insert_outgoing_and_enqueue` extracted into

```rust
async fn enqueue_direct(
    &self,
    store: &ConversationStore,
    service_id: &str,
    peer_address: &str,
    conversation_id: &str,
    content_type: &str,
    body: &[u8],
    system: bool,
) -> Result<String, ConversationError>
```

`send` calls it with `system = false`; group key distribution with
`system = true`. `insert_outgoing_and_enqueue` gains a `system: bool` parameter
and writes it to the new column.

### 6.10 Delivery of a group message (`transport.rs`)

`deliver_one` is unchanged for 1:1. A new sibling for the group case, chosen by
`OutboxItem.group`:

```
deliver_group_one(svc, peer_address, entry_id):
    entry = store.wire_entry(entry_id)?          // exactly as stored
    req   = GroupPushRequest {
                from: sign_peer_assertion(&sk, svc, group_id, now),
                group: group_id,
                entries: vec![entry],
            }
    ack: GroupPushAck = call_peer(svc, peer_address, "group-push",
                                  json(req), Some(entry_id))?
    Ok(())
```

Same `Disposition` classification (`classify`) as B4 — no change.

### 6.11 The receiving side — `group_push_impl` (`transport.rs`)

```
group_push_impl(svc, requester_did, req) -> GroupPushAck:
    if requester_did.is_empty(): return PermissionDenied
    store = store_for(svc)
    conv  = store.get_conversation(req.group)? or NotFound
    if conv.kind != Group: return InvalidArgument

    // D-B5-10: the pusher must be a current member, proven by signature,
    // not by the transport caller (F5).
    verify_peer_assertion(&store, &conv, &req.from)?          // PermissionDenied
    // D-B5-15: a guest reaching this arm on its own service id cannot
    // masquerade as a peer.
    if req.from.address == svc: return PermissionDenied

    if req.entries.len() > config.max_sync_entries_per_call:
        return QuotaExceeded

    accepted = []
    for entry in req.entries:
        match validate_and_insert(&store, svc, &conv, &entry):
            Ok(true)  => accepted.push(entry.entry_id)     // newly stored
            Ok(false) => accepted.push(entry.entry_id)     // already had it;
                                                           // still an ack, so
                                                           // the sender stops
            Err(e)    => return e                          // refuse the batch
    // Gap detection: an entry naming a parent we do not hold means we are
    // behind this peer. Ask now rather than at the next tick.
    if any accepted entry has a missing parent:
        spawn a bounded sync_with_peer(svc, conv.id, req.from.address)
    return GroupPushAck { accepted }
```

`validate_and_insert` is the single choke point every entry passes through, on
every path (push, sync, and the author's own write):

```
validate_and_insert(store, svc, conv, entry) -> Result<bool>:
    // 1. Re-encode. Never trust the decoded view.
    header = canonical_entry_bytes(entry)
    if derive_entry_id(&header) != entry.entry_id: InvalidArgument

    // 2. Author must be a member vouched for by the owner (D-B5-7), and
    //    must have been one at this entry's epoch: joined_epoch <= epoch,
    //    and removed_epoch is NULL or > epoch. This is failure-matrix
    //    row 8 -- a key or a message from a party absent from the
    //    membership history is not merely refused, it has no key to
    //    verify against.
    sig_key = store.member_sig_key_at(conv.id, entry.author, entry.epoch)?
              or return PermissionDenied
    if !verify_entry(sig_key, &header, entry.signature): PermissionDenied

    // 3. A membership entry is only valid from the owner.
    if entry.kind == Membership && entry.author != conv.owner_address:
        return PermissionDenied

    // 4. D-B5-11: refuse a future timestamp outright, do not store, do not
    //    forward. Past timestamps are accepted unbounded.
    if entry.sender_timestamp_ms > now + max_clock_skew_ms:
        return InvalidArgument("sender timestamp implausibly far in the future")

    // 5. Bounds -- failure-matrix row 12, per conversation.
    if store.dag_entry_count(conv.id) >= config.max_dag_entries_per_conversation:
        return QuotaExceeded
    if entry.parents.len() > MAX_PARENTS: return InvalidArgument

    store.queue().transaction(|tx, _| {
        inserted = insert_entry_if_absent(tx, conv.id, entry,
                                          applied=0, relay_pending=1)
        if !inserted { return Ok(false) }
        apply_entry(tx, svc, conv, entry)     // §6.12
        Ok(true)
    })
```

**`apply_entry` never fails the transaction on a missing key.** If the epoch key
is absent it leaves `applied = 0` and returns; the entry is stored and will be
applied by `apply_pending_entries` when the key arrives (§6.13). That is the
convergence window ADR-0013 §5 names.

### 6.12 `apply_entry` (`group.rs`)

```
apply_entry(tx, svc, conv, entry):
    match entry.kind:
      Membership:
        // D-B5-5: plaintext, so it applies whatever epoch we hold.
        apply_membership(tx, conv.id, entry.payload)
        if entry.payload.new_epoch > conv.current_epoch:
            update conversations set current_epoch = entry.payload.new_epoch
        // The owner also sends us the key over the 1:1 channel; if it has
        // already arrived, entries at the new epoch become applicable now.
        mark applied = 1
      Message:
        key = epoch_key(tx, conv.id, entry.epoch)
        if key is None: leave applied = 0; return          // wait for the key
        plaintext = open(key, header_prefix(entry), entry.nonce, entry.ciphertext)
                    or -> leave applied = 0 and record nothing
                         (a real member with the right key will succeed;
                          a wrong key is indistinguishable from a wrong
                          epoch and must not delete the entry)
        (content_type, body) = decode_body(plaintext)
        if body.len() > config.max_body_bytes: return QuotaExceeded
        insert OR IGNORE into messages
            (id=entry.entry_id, conversation_id=conv.id, author=entry.author,
             sender_timestamp=entry.sender_timestamp_ms, received_at=now,
             content_type, body, signature=entry.signature,
             outgoing=0, verified=1, state='delivered', system=0,
             entry_id=entry.entry_id)
        mark applied = 1
        // notify after commit, never inside the transaction
```

The `notify_message` call happens in the caller, after the transaction commits —
exactly as `peer_deliver_impl` already does
([transport.rs:263](../../../../crates/conversation/src/transport.rs#L281)).

### 6.13 Receiving a group key (`transport.rs`, inside `peer_deliver_impl`)

**Exact placement.** The branch goes *after* every check the existing function
already performs — signature verification, the `author == svc` refusal, the
`author == session.peer_address` refusal, the clock-skew bound, the
`expected_conv_id` comparison, and the `max_body` check — and *replaces* the
`store.queue().transaction(...)` block at
[transport.rs:252](../../../../crates/conversation/src/transport.rs#L252). It
must not be inserted earlier: those checks are what make `author` trustworthy,
and `author` is the only thing identifying the group's owner here.

The `expected_conv_id` check passes unchanged for a key message, and that is not
an accident: a key-distribution message **is** an ordinary 1:1 message in the
owner↔member direct conversation. Only its `body` is about a group.

```
if payload.content_type == GROUP_KEY_CONTENT_TYPE:
    key_msg: GroupKeyPayload = serde_json::from_slice(&payload.body)?
    // The sender of this 1:1 message is `author`, whose signing key the
    // 1:1 session already pinned (B4). The owner is whoever that is; a
    // key claiming a different owner than the group already has is a
    // hard refusal, never a silent owner change.
    store.queue().transaction(|tx, _| {
        conv = get_or_create_group_shell(tx, key_msg.group_id, author,
                                         key_msg.epoch)
        if conv.owner_address != author: return PermissionDenied
        insert OR IGNORE group_epochs (group_id, epoch, key, now)
        // MANDATORY, and the easiest thing in this plan to leave out.
        // `decrypt` has already advanced this session's ratchet in memory;
        // `commit_in` is the only thing that persists it. Skipping it on
        // this path -- while the ordinary message path at transport.rs:267
        // does commit -- leaves the receiver's session behind the sender's
        // by one message key, and every later 1:1 message from the owner
        // fails to decrypt. On first contact it is worse: the session row
        // is never written at all, so `session_for_envelope` finds nothing
        // next time and demands a pre-key message the owner will not send
        // again.
        self.crypto.commit_in(tx, &session)?
        Ok(())
    })
    apply_pending_entries(&store, svc, &key_msg.group_id)   // after commit
    return DeliveryAck { message_id: payload.message_id }   // never stored
                                                            // as a message
```

The `DeliveryAck` is returned with `payload.message_id` exactly as the ordinary
path does, so the owner's outbox row completes and the RPC `idempotency_key`
dedup still fences a redelivery. `notify_message` is **not** called — a key is
not a message and no guest export should see it.

`get_or_create_group_shell` is what admits a **joiner** into a group it has
never heard of: the first thing a new member receives is the epoch key, and
that call creates the `conversations` row (kind `group`, owner = the sender)
plus a `group_members` row for itself. Everything else — the membership
history, the earlier messages it may not read — arrives by sync afterward.

`apply_pending_entries(store, svc, group_id)` selects
`dag_entries WHERE conversation_id = ? AND applied = 0` in `seq` order and runs
`apply_entry` on each, notifying the guest for every message it newly writes.

**Why a joiner still cannot read pre-join messages** (exit criterion 8): it
receives only the key for epoch N+1 and later. Sync hands it the earlier
entries, `apply_entry` finds no key for their epochs, and they stay
`applied = 0` forever. They are on disk and unreadable — which is the honest
representation, and is what the test asserts on (`history` is empty for them).

### 6.14 `group_sync_impl` — the responder (`transport.rs`)

```
group_sync_impl(svc, requester_did, req) -> GroupSyncResponse:
    store = store_for(svc)
    conv  = store.get_conversation(req.group)? or NotFound
    verify_peer_assertion(&store, &conv, &req.from)?
    if req.from.address == svc: return PermissionDenied      // D-B5-15
    limit = min(req.limit, config.max_sync_entries_per_call)
    rows  = store.entries_after_seq(conv.id, req.after_seq, limit)
    return GroupSyncResponse {
        entries:  rows.map(to_wire),
        next_seq: rows.last().map(|r| r.seq).unwrap_or(req.after_seq),
        has_more: rows.len() as u32 == limit,
    }
```

The responder serves entries it holds **whether or not it could apply them** —
a member on an older epoch still relays newer ones. That is what keeps a lagging
peer from becoming a hole in the group's availability.

### 6.15 `sync_with_peer` — the requester (`transport.rs`)

```
sync_with_peer(svc, group_id, peer) -> Result<u32 /* entries taken */>:
    store  = store_for(svc)
    cursor = store.sync_cursor(group_id, peer)            // 0 if none
    taken  = 0
    for _ in 0..MAX_SYNC_ROUNDS (8):
        resp = call_peer(svc, peer, "group-sync",
                         json(GroupSyncRequest { group: group_id,
                                                 after_seq: cursor,
                                                 limit: config.max_sync_entries_per_call,
                                                 from: sign_peer_assertion(..) }),
                         None)?
        for entry in resp.entries:
            match validate_and_insert(&store, svc, &conv, &entry):
                Ok(true)  => taken += 1
                Ok(false) => {}
                // One bad entry must not stop the round: a peer holding a
                // refused entry (a future timestamp, D-B5-11) would
                // otherwise wedge our cursor behind it forever.
                Err(_)    => log and continue
        cursor = resp.next_seq
        store.set_sync_cursor(group_id, peer, cursor, now)
        if !resp.has_more: break
    if taken > 0: apply_pending_entries(&store, svc, group_id)
    Ok(taken)
```

`MAX_SYNC_ROUNDS = 8` bounds one pass at `8 × 64 = 512` entries per peer per
tick. A member returning after a long absence catches up over several ticks
rather than in one unbounded call — which is what row 12 asks for.

**Advancing the cursor past a refused entry is deliberate.** Skipping it is
correct precisely because `D-B5-11` says nobody should accept it; a member that
did accept it has already diverged, and stalling our own sync would spread the
damage.

### 6.16 The worker passes (`outbox.rs`)

`run_worker` gains two counters and keeps one `sleep(tick)`:

```rust
pub async fn run_worker(self: Arc<Self>, tick: Duration, cancel: CancellationToken) {
    let mut ticks: u64 = 0;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(tick) => {
                ticks = ticks.wrapping_add(1);
                self.drain_once().await;                  // unchanged
                self.relay_once().await;                  // every tick
                if ticks % self.sync_every_ticks() == 0 {
                    self.group_sync_once().await;
                }
                if ticks % self.rekey_every_ticks() == 0 {
                    self.scheduled_rekey_once().await;
                }
            }
        }
    }
}
```

`sync_every_ticks() = max(1, group_sync_secs / tick_secs)`, likewise for rekey.
Both are computed from the `ConversationConfig` values the service already
holds, so `runtime.rs` passes nothing new.

**`relay_once`** — `D-B5-8`'s epidemic push:

```
for svc in candidate_service_ids():
    store = store_for(svc)
    for entry in store.claim_relay_pending(limit = RELAY_LIMIT_PER_TICK (32)):
        // claim_relay_pending sets relay_pending = 0 in the same statement,
        // so a crash mid-pass costs one relay round, never a repeat storm.
        members = store.current_members(entry.conversation_id)
        targets = members minus {svc, entry.author}, shuffled, take RELAY_FANOUT (3)
        for t in targets:
            let _ = push_entries_to(svc, t, entry.conversation_id, &[entry]).await;
            // best effort by design: the durable guarantee is the author's
            // own fan-out plus the pull pass, so a failure here is not
            // retried and not recorded.
```

**`group_sync_once`:**

```
for svc in candidate_service_ids():
    for conv in store.group_conversations():
        members = current_members minus {svc}
        if members.is_empty(): continue
        // Round-robin, one peer per pass, so a node with G groups makes G
        // calls per pass rather than G x N.
        peer = members[(tick_counter as usize) % members.len()]
        let _ = self.sync_with_peer(svc, &conv.id, peer).await;
```

**`scheduled_rekey_once`** — failure-matrix row 10:

```
for svc in candidate_service_ids():
    for conv in store.group_conversations() where conv.owner_address == svc:
        (epoch, created_at) = store.current_epoch_row(conv.id)
        if now - created_at < config.group_rekey_secs: continue
        // Identical to the rekey half of change_membership, with no
        // membership entry: a rekey with stable membership still opens a
        // new epoch and still distributes a new key. That is what bounds an
        // undetected compromise.
        rekey(svc, conv, reason = scheduled)
```

**A rekey with unchanged membership writes no DAG entry.** It only inserts a
`group_epochs` row and distributes the key 1:1. Members learn of it when the
first entry at the new epoch arrives — for which they already hold the key.
This keeps membership entries meaning membership, which is what rows 8 and 10
each depend on.

### 6.17 `drain_item` — settling a group fan-out row

In `outbox.rs::drain_item`, after `parsed` is decoded:

```
match parsed.group {
    None => <existing 1:1 path, unchanged>
    Some(group_id) => {
        // The message row's own state is derived, never set per recipient.
        match deliver_group_one(svc, &parsed.peer_address, &parsed.message_id).await {
            Ok(()) | Err(Delivered) => {
                store.set_recipient_state(&parsed.message_id, &parsed.peer_address,
                                          Delivered, None);
                queue.complete(item.id);
                // D-B5-14: `delivered` only when nobody is still pending.
                if store.recipients_remaining(&parsed.message_id)? == 0 {
                    let final_state = if store.any_recipient_failed(..)? { Failed }
                                      else { Delivered };
                    store.set_state(&parsed.message_id, final_state, ..);
                    notify_state(svc, message_id, final_state).await;
                }
            }
            Err(Unreachable) => queue.defer(item.id, now + backoff_for_age(age)),
            Err(Terminal(r)) => settle_recipient_failed(.., &r),
            Err(Retry)       => on DeadLettered -> settle_recipient_failed(..),
        }
    }
}
```

`settle_recipient_failed` marks that one recipient `failed` and then applies the
same "nobody still pending" rule, so one unreachable member fails the message
once every other member has settled — never before.

### 6.18 The three remaining guest methods (`group.rs`)

**`members`** — current members only, sorted, so it is byte-comparable across
substrates (§11.3 test 7 rests on that):

```sql
SELECT member_address FROM group_members
 WHERE conversation_id = ?1 AND removed_epoch IS NULL
 ORDER BY member_address ASC
```

A removed member keeps its row — `removed_epoch` is what
`member_sig_key_at(conv, author, epoch)` reads to decide whether an author was a
member *at that entry's epoch* (§6.11 step 2). Deleting the row instead would
make every entry the removed member ever wrote unverifiable, and the group's own
history would stop converging.

**`membership_history`** — every change, oldest first:

```sql
SELECT entry_id, payload, sender_timestamp, author FROM dag_entries
 WHERE conversation_id = ?1 AND kind = 'membership'
 ORDER BY sender_timestamp ASC, author ASC, entry_id ASC
```

Ordered on the same three-part key as messages (`D-B4-17`, §0.2), **not** on
`(sender_timestamp, subject, entry)`. Correct the WIT doc comment in §3 to match:
`subject` is not a column of the entry, it lives inside `payload`, and sorting on
a decoded field would be a second ordering rule for no reason. `action`,
`subject`, and `epoch` are read out of `payload`.

`applied` is deliberately not filtered: a membership entry is plaintext
(`D-B5-5`), so it applies on arrival whatever epoch this member holds, and one
that failed to apply would be a bug worth seeing rather than hiding.

**`sync_now`** (`D-B5-20`) — one budgeted round:

```
sync_now(service_id, conversation):
    store = store_for(service_id)
    conv  = store.get_conversation(conversation)? or NotFound
    if conv.kind != Group: return InvalidArgument("not a group")

    deadline = Instant::now() + config.sync_now_budget    // 3 s default
    // Address order, not shuffled: two members calling `sync-now` at the
    // same moment should not both start on the same random peer, and a
    // deterministic order makes a test's second call resume where the
    // first ran out of budget.
    for peer in store.current_members(conversation) minus {service_id}:
        if Instant::now() >= deadline: break
        // Sequential, not concurrent. Concurrency here would need its own
        // bound to avoid N simultaneous proxy calls from one guest
        // invocation, and the budget already caps the work; one at a time
        // is the version with no second knob.
        match self.sync_with_peer(service_id, conversation, peer,
                                  per_peer_timeout = 2 s).await:
            Ok(_)  => {}
            // A peer that is down must not fail the call: `sync-now`
            // promises a bounded round, not a complete one.
            Err(e) => debug!(peer, error = %e, "group sync pass skipped a peer")
    Ok(())
```

`sync_with_peer` (§6.15) gains the `per_peer_timeout` parameter and passes it
through to `call_peer`, which gains a `timeout: Duration` parameter of its own.
**All four existing `call_peer` call sites pass `Duration::from_secs(30)`** so
B4's behaviour is byte-identical; only the two sync paths pass anything else
(2 s from here, 10 s from the background pass).

### 6.19 Authorization (`§5.3`'s rule, extended)

Every guest-facing method is keyed by `service_id` taken from
`HostState.component_id` / `SynSvcNativeService.service_id`, never from a
caller-supplied argument. No cross-service group access exists and no interface
for one. The two peer verbs authorize on `PeerAssertion` + membership
(`D-B5-10`), which is strictly narrower than B4's `deliver` (any peer with a
pinned session) because a group is a closed set.

---

## §7 The dual-build surface

### 7.1 `crates/app_host/src/lib.rs`

`AppConversation` gains six methods, mirroring the WIT one for one:

```rust
    fn create_group(&self) -> impl Future<Output = Result<String, ConversationError>> + Send;
    fn add_member(&self, conversation: String, member_address: String)
        -> impl Future<Output = Result<(), ConversationError>> + Send;
    fn remove_member(&self, conversation: String, member_address: String)
        -> impl Future<Output = Result<(), ConversationError>> + Send;
    fn members(&self, conversation: String)
        -> impl Future<Output = Result<Vec<String>, ConversationError>> + Send;
    fn membership_history(&self, conversation: String)
        -> impl Future<Output = Result<Vec<MembershipEvent>, ConversationError>> + Send;
    fn sync_now(&self, conversation: String)
        -> impl Future<Output = Result<(), ConversationError>> + Send;
```

`AppHost`, `MessageSink`, and `ConversationSink` are unchanged.

### 7.2 `crates/app_host/src/guest.rs`

Six thin delegations to `conv::create_group()` etc., in the style of the
existing seven.

### 7.3 `crates/app_host_native/src/host.rs` + `convert.rs`

Six delegations through `HostConversation::*` on the locked `HostState`, and
one new converter `membership_event_out(HostMembershipEvent) ->
GuestMembershipEvent` with a round-trip unit test beside the existing six.

### 7.4 `crates/sandbox_wasm/src/host_capabilities.rs`

- `conversation_wire::map_membership_event`, beside `map_summary`/`map_message`.
- Six new methods on `impl wit_conversation::Host for HostState`, each
  `self.conversation.upgrade().ok_or_else(no_capability)?` then delegate with
  `self.component_id`.
- `add-member`/`remove-member`/`create-group`/`sync-now` are **writes** and must
  carry the same `read_only` refusal `send` already applies
  ([host_capabilities.rs:1933](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L1933)).
  `members`/`membership-history` are reads and must not.
- `empty_conversation_host()` gains eight `unreachable!` arms.

### 7.5 `crates/control_plane/src/synsvc_native.rs`

- `dispatch_conversation` gains two arms:

```rust
    "group-push" => {
        let bytes = serde_json::to_vec(&invocation.params).map_err(internal)?;
        let ack = conversation
            .group_push(&self.service_id, &invocation.caller.caller_did, bytes)
            .await
            .map_err(conversation_error)?;
        to_payload(&serde_json::from_slice::<Value>(&ack).map_err(internal)?)
    }
    "group-sync" => { ...identical shape, `group_sync`... }
```

- `empty_conversation_host()` in this file gains the same eight arms.
- **The seven-plus-six guest verbs stay unreachable through this arm.** Do not
  add them — a remote caller must never reach `send`/`add-member` on someone
  else's service.

---

## §8 The fixture

`test-components/dual-build-fixture/`:

- `wit/deps/conversation/conversation.wit` is a **file symlink** to the real
  package ([status.md §5](status.md)) — nothing to update.
- `wit/world.wit` unchanged: the world already imports
  `syneroym:conversation/conversation@0.1.0`.
- `src/app.rs`: six new `Request` variants and their `dispatch` arms —
  `CreateGroup`, `AddMember { conversation, member_address }`,
  `RemoveMember { conversation, member_address }`, `Members { conversation }`,
  `MembershipHistory { conversation }`, `SyncNow { conversation }`. Each returns
  a plain JSON view; `membership-history` returns
  `[{entry, action, subject, epoch, sender_timestamp}]`.
- `src/app.rs`: **a seventh variant, `ListConversations`, over the *existing*
  `conversations` verb.** Checked against the tree while writing this plan:
  `conversations` has **no fixture variant and no parity scenario today** —
  `rg 'list-conversations|ListConversations' test-components/ crates/app_host_native/tests/`
  returns nothing. It is the one B4 verb the shim never exercised, and B5
  changes its behaviour twice (the group participant branch and the `system`
  filter, §6.2.1). Shipping those two changes with no parity coverage would
  put a divergence in exactly the verb that has never been compared.
  Returns `{id, kind, participants}` per row — no timestamps, per the note in
  §11.1.
- `src/app.rs`: **delete the four `M06B slice B4:` doc comments** (F10).
- `src/guest.rs` / `src/native.rs`: no change — both already route every
  `Request` through the same `app::run`.

---

## §9 Substrate wiring and config

### 9.1 `crates/core/src/config.rs`

Seven new `AppSandboxRole` fields, beside the eight `conversation_*` ones, each
with a `#[serde(default = "...")]` and a `const fn` default:

| Field | Default | Meaning |
|---|---|---|
| `conversation_group_sync_secs: u64` | 60 | Pull-sync pass interval. |
| `conversation_group_rekey_secs: u64` | 604_800 (7 days) | Scheduled rekey bound (failure-matrix row 10). |
| `conversation_max_group_members: u32` | 256 | `add-member` past it is `quota-exceeded`. |
| `conversation_max_dag_entries_per_conversation: u32` | 100_000 | Row 12, per conversation. |
| `conversation_max_sync_entries_per_call: u32` | 64 | One sync/push response's ceiling. |
| `conversation_relay_fanout: u32` | 3 | Peers one relay pass pushes to. |
| `conversation_sync_now_budget_ms: u64` | 3_000 | Total wall-clock budget for one guest `sync-now` call (`D-B5-20`). Must stay well under `dispatch_epoch_timeout_secs` (5 s), which bounds the calling guest. |

All seven are added to `Default for AppSandboxRole`.

### 9.2 `crates/conversation/src/store.rs`

`ConversationConfig` gains the same seven fields (plain values, converted by
`runtime.rs` — this crate does not depend on `syneroym-core::config`, and the
existing doc comment says so). `Default` gains matching values so every unit
test keeps compiling unchanged.

### 9.3 `crates/substrate/src/runtime.rs`

One edit: the `syneroym_conversation::store::ConversationConfig { .. }`
literal at [runtime.rs:1173](../../../../crates/substrate/src/runtime.rs#L1173)
gains seven field initialisers from the new `AppSandboxRole` fields. **Nothing
else in `runtime.rs` changes** — no new task, no new cancellation token, no new
`select!` arm (`D-B5-12`).

---

## §10 Every call site that changes

| # | File | Change |
|---|---|---|
| 1 | `crates/wit_interfaces/wit/conversation/conversation.wit` | §3 — one record, six functions. |
| 2 | `crates/app_host/src/types.rs` | Re-export `MembershipEvent`. |
| 3 | `crates/rpc/src/conversation.rs` | `ConversationMembershipEvent`; eight `ConversationHost` methods. |
| 4 | `crates/conversation/Cargo.toml` | `aes-gcm.workspace = true`. |
| 5 | `crates/conversation/src/ids.rs` | `derive_group_id`, `derive_entry_id`. |
| 6 | `crates/conversation/src/dag.rs` | **New.** Wire types, canonical bytes, sign/verify, seal/open. |
| 7 | `crates/conversation/src/group.rs` | **New.** The six guest methods, epochs, rekey, `apply_entry`, `apply_pending_entries`. |
| 8 | `crates/conversation/src/store.rs` | Six tables, five columns, ~14 new methods, `ConversationConfig` +7. **Plus three existing queries (§6.2.1): `history` and `outbox_messages` gain `AND system = 0`; `list_conversations` gains `WHERE system = 0` and two more columns. Plus `get_or_create_direct`'s flag-clearing `UPDATE` (§6.2.2) and `ConversationRow` +2 fields.** |
| 9 | `crates/conversation/src/transport.rs` | `group_push_impl`, `group_sync_impl`, `push_entries_to`, `sync_with_peer`, `deliver_group_one`, `sign_peer_assertion`/`verify_peer_assertion`; the group-key branch inside `peer_deliver_impl` (**including its `commit_in`**, §6.13); **`call_peer` gains `timeout: Duration`, all four existing call sites passing 30 s unchanged (`D-B5-20`)**. |
| 10 | `crates/conversation/src/outbox.rs` | `CLAIM_LIMIT_PER_TICK` 16→64; `run_worker` counters; `relay_once`, `group_sync_once`, `scheduled_rekey_once`; `drain_item`'s group arm; `OutboxItem.group`. |
| 11 | `crates/conversation/src/lib.rs` | `mod dag; mod group;`; `enqueue_direct`; `send` branch; eight `ConversationHost` methods. **Plus `conversations`' `kind` branch for group participants (§6.2.1).** |
| 12 | `crates/sandbox_wasm/src/host_capabilities.rs` | `map_membership_event`; six `Host` methods; `empty_conversation_host` +8 arms. |
| 13 | `crates/control_plane/src/synsvc_native.rs` | Two `dispatch_conversation` arms; `empty_conversation_host` +8 arms. |
| 14 | `crates/app_host/src/lib.rs` | `AppConversation` +6. |
| 15 | `crates/app_host/src/guest.rs` | `impl AppConversation for GuestHost` +6. |
| 16 | `crates/app_host_native/src/host.rs` | `impl AppConversation for NativeAppHost` +6. |
| 17 | `crates/app_host_native/src/convert.rs` | `membership_event_out` + round-trip test. |
| 18 | `crates/core/src/config.rs` | Seven `AppSandboxRole` fields + defaults. |
| 19 | `crates/substrate/src/runtime.rs` | Seven field initialisers. |
| 20 | `test-components/dual-build-fixture/src/app.rs` | **Seven** `Request` variants (six group verbs plus `ListConversations` over B4's untested `conversations`, §8); delete four slice-id comments. |
| 21 | `crates/app_host_native/tests/dual_build_parity.rs` | §11.1. |
| 22 | `crates/substrate/tests/group_conversation_e2e.rs` | **New.** §11.3. |

**Not changed, and worth checking that they stayed unchanged:**
`NATIVE_CAPABILITY_INTERFACES` (still 7 — `group-push`/`group-sync` are
*methods* on the existing `conversation` interface, not new interfaces);
`check_native_capability_gate`; `crates/wit_interfaces/src/*.rs`; the
`guest-api` world; `AppSandboxEngine`'s `notify_guest_*`; every
`ConversationNotifier` impl.

---

## §11 Tests

### 11.1 The parity suite (`crates/app_host_native/tests/dual_build_parity.rs`)

Added to the deterministic `SCENARIOS` table (F8 — these are byte-comparable):

| Name | Request |
|---|---|
| `members-unknown-conversation` | `{"op":"members","conversation":"conv:does-not-exist"}` |
| `membership-history-unknown` | `{"op":"membership-history","conversation":"conv:does-not-exist"}` |
| `add-member-unknown-conversation` | `{"op":"add-member","conversation":"conv:does-not-exist","member_address":"peer-x"}` |
| `add-member-on-a-direct-conversation` | opens a direct conversation, then adds — must be `invalid-argument` on both |
| `sync-now-unknown-conversation` | `{"op":"sync-now","conversation":"conv:does-not-exist"}` |
| `list-conversations` | `{"op":"list-conversations"}` — placed **after** `open-conversation` in the table, whose id is derived and therefore stable |

> **`list-conversations` only belongs in the byte-comparison table because
> the fixture projects it.** `conversation-summary` carries `created-at` and
> `last-activity-at`, both wall-clock `now_ms()` values that differ between
> the two builds by however long the harness took to reach them. The fixture
> variant must return `{id, kind, participants}` and drop both timestamps —
> the same rule that keeps `send-message` out of the table (F8). A future
> scenario over any timestamped record needs the same projection.

Named per-build tests (non-deterministic id, so outside the table):

- `create_group_yields_a_group_with_one_member_on_both_builds` — `create-group`,
  then `members` returns exactly `[SERVICE_ID]`, and `membership-history` has
  one `add` event whose `subject` is `SERVICE_ID` and whose `epoch` is 1.
- `sending_to_a_group_with_no_other_member_is_refused_on_both_builds` —
  `invalid-argument`, identical on both.
- `remove_member_by_a_non_owner_is_permission_denied_on_both_builds` — build a
  group whose owner is a different address (via the store directly in the
  harness), then call `remove-member`.
- `a_key_distribution_conversation_is_hidden_until_open_direct_on_both_builds`
  (`D-B5-18`, §6.2.2) — create a group, add a member the fixture has never
  messaged, assert `list-conversations` does **not** show the auto-created
  direct conversation with that member, then `open-direct` on the same peer
  and assert it now does, with the same id. This is the one state
  transition in B5 whose omission is silent in both directions: no `UPDATE`
  and the conversation is invisible forever; no `WHERE system = 0` and it
  was never hidden.
- `a_group_key_message_is_absent_from_history_and_outbox_on_both_builds`
  (`D-B5-6`, §6.2.1) — after the same setup, `read-history` on that direct
  conversation and `read-outbox` both return nothing.
- `the_parity_comparison_detects_a_divergence` must still fail on a corrupted
  group field — extend `Mutant` to touch one of the new responses, per B4's
  own §16 item 2.

### 11.2 Unit tests inside `syneroym-conversation`

| Module | Test |
|---|---|
| `ids` | `group_id_differs_for_the_same_owner_at_the_same_millisecond` (nonce). `entry_id_is_stable_for_identical_headers`. |
| `dag` | `a_one_bit_change_anywhere_in_the_header_fails_verification` (each field in turn, matching `envelope.rs`'s existing test). `moving_a_byte_across_the_parents_length_prefix_cannot_produce_a_collision`. `sealed_ciphertext_does_not_open_under_a_different_epoch_key`. `sealed_ciphertext_does_not_open_when_the_header_aad_is_altered`. |
| `store` | `heads_are_the_entries_with_no_child`. `the_sync_cursor_never_skips_an_entry_inserted_out_of_timestamp_order` — insert three entries with descending timestamps and assert a `seq` walk returns all three. `dag_entry_quota_is_per_conversation` (row 12). `recipients_remaining_reaches_zero_only_when_every_member_settles` (`D-B5-14`). `history_and_outbox_exclude_system_messages` and `list_conversations_excludes_system_conversations` (§6.2.1 — three queries, asserted directly at the store level where the SQL lives, not only through the fixture). `get_or_create_direct_clears_the_system_flag_on_an_existing_row` (§6.2.2). |
| `group` | `an_entry_authored_before_the_author_joined_is_refused`. `an_entry_authored_after_the_author_was_removed_is_refused` (row 7). `a_membership_entry_not_signed_by_the_owner_is_refused` (row 8). `an_entry_whose_epoch_key_is_absent_stays_unapplied_and_applies_when_the_key_arrives`. `a_scheduled_rekey_with_unchanged_membership_still_changes_the_key` (row 10). `a_joiner_cannot_open_entries_from_before_its_join` (row 7). `members_excludes_a_removed_member_but_the_row_survives_for_signature_checks` — the pair that must hold together: `members()` omits it while `member_sig_key_at(conv, addr, epoch_before_removal)` still resolves, which is what keeps its earlier entries verifiable and the transcript converging. `membership_history_orders_on_the_same_three_part_key_as_messages` — two changes at one millisecond, asserted stable. `conversations_reports_group_members_as_participants` (§6.2.1 — a group row has no `peer_address`, so the unbranched code reports a one-element list). |
| `transport` | `a_peer_assertion_from_a_non_member_is_refused`. `a_peer_assertion_signed_by_the_wrong_key_is_refused`. `a_self_addressed_peer_assertion_is_refused` (`D-B5-15`). `an_entry_more_than_the_skew_bound_in_the_future_is_neither_stored_nor_forwarded` (`D-B5-11`). |
| `outbox` | `a_group_message_is_not_delivered_until_every_recipient_settles` (row 3, group form). |

### 11.3 Cross-node e2e — `crates/substrate/tests/group_conversation_e2e.rs` (new)

Three real `syneroym-substrate` instances, built on `conversation_e2e.rs`'s
`Node`/`publish_endpoint`/`deploy_fixture` helpers (copy, do not refactor them
into `common/` in this slice — that is a separate change touching a passing
test). **Claim a fresh port block**: `conversation_e2e.rs` holds 14_000-14_102;
take **14_200-14_302** and add the third node's triple.

Serialize the tests in this binary with a `static TEST_LOCK: Mutex<()>`, per
B4's F20.

| # | Test | Asserts | Criterion / row |
|---|---|---|---|
| 1 | `three_members_converge_to_byte_identical_transcripts` | A creates a group, adds B and C; all three post with deliberately skewed `sender_timestamp` (set through the fixture by driving each node's clock offset via `conversation_max_clock_skew_secs` and posting in a scrambled order); after `sync-now` on each, `read-history` returns the **same JSON string** on all three. | 7, row 9 |
| 2 | `an_offline_member_pulls_the_gap_from_a_peer_not_the_author` | C is stopped; A posts three messages; B receives them; **A is stopped**; C restarts and `sync-now`s; C's history matches B's. Stopping A is what makes this a *pull* test rather than a redelivery test. | 9 |
| 3 | `a_removed_member_cannot_read_messages_sent_after_the_removal` | A removes C, then posts; C `sync-now`s, receives the entry (it is still pushed by a lagging peer in the same test via a direct `group-push`), and `read-history` does **not** contain it. | 8, row 7 |
| 4 | `a_joiner_cannot_read_messages_from_before_the_join` | A and B talk; A adds C; C `sync-now`s and holds the earlier entries on disk but `read-history` shows only post-join messages. | 8, row 7 |
| 5 | `no_group_content_traverses_the_broker` | A wildcard (`#`) MQTT subscription over the whole run sees no message body — the same assertion `conversation_e2e.rs` already makes for 1:1, extended over the group flow. | 6, row 6 |
| 6 | `a_group_message_survives_a_restart_on_every_node` | Post with C down; restart A and B; state is still `pending`, not duplicated and not reset; bring C up; state becomes `delivered`. | 4, row 4 |
| 7 | `membership_history_is_identical_on_every_member` | After add and remove, `membership-history` returns the same JSON on A, B, and C. | row 8 |

### 11.4 Failure-and-security-matrix ledger

Add to §13's status entry, one row per matrix line, naming the test:

| Row | Covered by |
|---|---|
| 6 (broker) | §11.3 test 5 |
| 7 (removed member / joiner) | §11.3 tests 3, 4; `group` unit tests |
| 8 (key reaches a non-member) | `a_membership_entry_not_signed_by_the_owner_is_refused`; `an_entry_authored_before_the_author_joined_is_refused` |
| 9 (skewed clocks) | §11.3 test 1 |
| 10 (scheduled rekey) | `a_scheduled_rekey_with_unchanged_membership_still_changes_the_key` |
| 12 (bounded growth) | `dag_entry_quota_is_per_conversation`; `MAX_SYNC_ROUNDS`; `conversation_max_group_members` |
| 13 (shim disagreement) | §11.1, including the extended `Mutant` |

**Exit criterion 11 is milestone-wide, not slice-wide, and B5 is the slice
that closes it.** Rows 1–2 are B1's (gateway person identity), rows 3–5 are
B4's (never-delivered-before-ack, restart survival, the unreachable
recipient), row 11 is B2's (undeclared visibility). All are already tested
and recorded in [status.md](status.md). The table above covers only what B5
adds. **§13's `status.md` edit must assert the full 1–13 matrix, citing the
owning slice and test for every row** — a per-slice ledger that never gets
summed is how a milestone-level criterion goes unmet while every slice
reports done.

---

## §12 Ordering inside B5

Each step compiles and its own tests pass before the next begins.

1. **WIT + bindings + plain types** — §3, §4, §5. `cargo build -p
   syneroym-wit-interfaces -p syneroym-rpc` clean. Nothing implements the new
   trait methods yet, so the workspace does not build; that is expected and
   ends at step 3.
2. **`dag.rs` + `ids.rs`** — pure functions, no store. All §11.2 `dag`/`ids`
   tests pass here, before anything can depend on a wrong canonical encoding.
3. **`store.rs`** — schema and methods, with the §11.2 `store` tests. **This
   step includes §6.2.1's four existing-query edits and §6.2.2's
   flag-clearing `UPDATE`** — do them here, with the schema, not later as a
   cleanup: every one of them is a silent wrong answer rather than a
   compile error, so nothing downstream will remind you. Add the eight
   `unreachable!` arms to both `empty_conversation_host()`s now so the
   workspace builds again.
4. **`group.rs`** — `create-group`, membership, epochs, `apply_entry`. Unit
   tests, no network.
5. **`transport.rs`** — the four peer functions, `call_peer`'s new `timeout`
   parameter (`D-B5-20`), and the key branch **with its `commit_in`** (§6.13).
6. **`outbox.rs`** — fan-out settlement and the three passes.
7. **The host surfaces** — `host_capabilities.rs`, `synsvc_native.rs`.
8. **The dual-build surface + fixture** — §7, §8; parity suite green.
9. **Cross-node e2e** — §11.3.
10. **The completion pass** — §13.

Step 2 before step 3 is the one ordering that matters: the canonical encoding
is what every stored signature is checked against, and changing it after
entries exist in a test fixture produces failures that look like authorization
bugs.

---

## §13 The completion pass (AGENTS.md)

In this order:

1. `cargo +nightly fmt --all`
2. `cargo clippy --workspace --all-targets --all-features`
3. `cargo test --workspace` *(sandbox **on** — disabling it stalls on real
   network)*
4. `cargo audit`
5. `cargo deny check licenses` — `aes-gcm` is Apache-2.0 OR MIT and already in
   the workspace graph (F7), so this should be a no-op; confirm rather than
   assume.
6. `mise run build:test-components` then `mise run test:e2e` *(sandbox
   **off** — needs real port binds)*
7. **Import cleanup pass** over all 22 files in §10: types via plain `use`,
   functions qualified by parent module, no inline multi-`::` paths. `dag.rs`
   is the file most at risk — it touches `aes_gcm`, `blake3`, `ed25519_dalek`,
   and `serde`.
8. **Slice-id sweep**: `rg -n 'D-B5-|M06B|Slice B|B5' crates/ test-components/`
   over the diff must return nothing in code, doc comments, or test names.
   This includes deleting F10's four existing violations.
9. **Component-link proof (exit criterion 1):**
   `wasm-tools component wit target/wasm32-wasip2/release/syneroym_test_dual_build_fixture.wasm`
   and paste the import/export list into `status.md`, as B3 and B4a both did.

### Documents and backlog owed, in the same change

| Document | Edit |
|---|---|
| [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §4 | An implementation note recording `D-B5-8` (author fan-out + relay push + cursor pull) and `D-B5-9` (the cursor is local insertion order, not the total sort — and why). |
| ADR-0013 Amendment 1 | A note recording `D-B5-6` (the key rides the 1:1 ratchet, with the reasoning) and `D-B5-7` (the owner vouches for member signing keys), since both are questions the amendment left to the implementing milestone. |
| ADR-0013 §5 | Extend B4's tie-break note: B5's DAG sorts on the same `(sender_timestamp, author, entry_id)`. |
| ADR-0013 §4 or Consequences | `D-B5-11`'s convergence limit: members whose clocks differ by more than `conversation_max_clock_skew_secs` diverge on entries near the bound. This is a security-relevant property and must not live only in a slice plan — B4's §16 item 12 is the precedent for why. |
| [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) status | Proposed → **Accepted**, now that both halves are built. Currently still `Proposed`, which is stale (§14.1). |
| [task.md](task.md) | B5's row → Complete with a date and a link to this plan; the "Owed as slices land" table gains a B5 row. |
| [status.md](status.md) | A `B5 — What shipped` section and a `B5 — Verification Evidence` section, in B4a's shape, including §11.4's ledger **and a full failure-matrix table covering rows 1–13, naming the owning slice and the test for each** — B5 is the last slice, so this is where milestone exit criterion 11 is actually demonstrated rather than assumed. |
| [deferred-backlog.md](../../deferred-backlog.md) | Row 170 (*"No durable messaging host interface exists (group half)"*) moves to **Recently resolved**. Row 169 (MLS trade-offs) **stays open** — it is a standing revisit trigger, not a deferral B5 closes. Row 181 (partial cross-node e2e coverage) is re-scoped, not closed: B5 adds seven cross-node cases and does not retire B4's thirteen missing ones. |
| [deferred-backlog.md](../../deferred-backlog.md), new rows | (a) `D-B5-11`'s clock-skew divergence limit. (b) `D-B5-19`'s missing `on-membership-change` export, with the pickup trigger *"M06C needs a membership change to reach a guest without a poll"*. (c) `D-B5-5`'s accepted leak: a just-removed member a lagging peer still pushes to sees later membership metadata. (d) F9's dead `conversation_prekey_pool_size` field. (e) Relay push is best-effort and unrecorded, so a group whose author dies before any fan-out completes never converges — the honest consequence of D4, worth stating. **(f) `delivery-status` reports one aggregate state for a group message, and `message_recipients` holds the per-member detail it does not expose** (`D-B5-14`). A group of five with one unreachable member reports `pending` with no way for a guest to learn it is 4-of-5 — M06C's UI will want exactly that. Deliberately not added here: it is a new WIT verb plus a new record, and B5's own gates do not need it. Pickup trigger: *M06C renders group delivery progress*. **(g) `conversation_sync_now_budget_ms` is a local workaround for the same 5-second guest epoch backlog rows 230 and 253 already describe** (`D-B5-20`) — the third such local budget in the tree after B2's `enqueue` probe and B4's `step`. Add B5 to row 253's list rather than opening a fourth row. |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | The Messaging section's **Group** bullet gains the three spread mechanisms; G1's slice-owner note marks B5 done. |

---

## §14 Ambiguities and staleness in the input documents

Flagged rather than guessed. Items 14.1–14.3 want an answer before or during
implementation; the rest are notes.

### 14.1 ADR-0013's status line is stale — **decide before writing the ADR edits**

The file still reads `Status: Proposed — amended 2026-08-13`, yet
[task.md](task.md)'s dependency-gate table lists *"ADR-0013 + Amendment 1
(messaging architecture, group key) — **Accepted**"*, and B4 shipped against
it. One of the two is wrong. The plan above assumes the ADR should be moved to
`Accepted` with B5, matching how ADR-0018 was moved with B2. **Confirm that
reading** — if the intent was that ADR-0013 stays `Proposed` until the product
proves it in M06C, say so and the edit drops out.

### 14.2 "Total order by `(sender_timestamp, sender_did)`" is not what the tree sorts on

[task.md](task.md)'s B5 row and R4's acceptance table both say
`(sender_timestamp, sender_did)`. The tree sorts on
`(sender_timestamp, author, id)`, and B4 amended ADR-0013 §5 to say so, for the
stated reason that two keys are not a total order. **This plan follows the
tree.** `task.md`'s B5 row and the spec's R4 row still carry the two-key
wording and should be corrected in the §13 documentation pass — a byte-identical
transcript is exactly the property that a non-total sort key silently breaks.

### 14.3 "Sender DID" versus "address" — the same conflation B4's F16 found

R4, ADR-0013 §5, and task.md all say `sender_did`. B4 established
(`D-B4-5`) that a message is attributed to the sender's **routing service id**,
not to any DID, because the transport-visible DID is the owner's Master DID and
cannot distinguish two services one owner runs. B5 inherits `address`
throughout. The documents are not wrong so much as pre-dating the finding; the
§13 pass should make the same correction ADR-0013 §3 already carries for 1:1.

### 14.4 `task.md`'s reference scenario step 9 says "adds B and C" in one step

*"A creates a group, adds B and C; owner distributes the epoch key."* Under
`D-B5-7` each `add-member` opens its own epoch, so adding two members produces
two epochs and two key distributions, not one. That is a stricter reading of
*"rekey on every join"* than the sentence implies, and it is the reading
Amendment 1's own words require. Noted rather than changed; the e2e test is
written against two adds.

### 14.5 `conversation_prekey_pool_size` is declared and never read (F9)

Not B5's, but B5 edits the same config block and adding six fields beside a
dead one makes the block harder to trust. Backlog row (d) above; the cheap fix
is to make `prekey_bundle` top the pool up to this size instead of generating
one key on demand.

### 14.6 Task.md's fourth open design point asks a question whose answer changes nothing observable

*"Whether the group key rides the 1:1 ratchet or its own channel."* `D-B5-6`
answers it, but the answer is not testable from outside — both choices produce
the same guest-visible behaviour. The decision is recorded on the ADR (§13)
precisely because no test will ever defend it.

### 14.7 The failure matrix's row 12 says "bounded per conversation", and one path is not

`MAX_SYNC_ROUNDS × max_sync_entries_per_call` bounds one sync pass, and
`max_dag_entries_per_conversation` bounds the store. But **`relay_once` iterates
every group of every service on every tick**, so a node in many large groups
does `groups × fanout` proxy calls per tick with no global ceiling. Bounded per
conversation, unbounded per node. `RELAY_LIMIT_PER_TICK = 32` in §6.16 is a
per-service cap that makes it finite; whether it is the right number is not
something this plan can know without a benchmark. Flagging it rather than
inventing a figure — if a soak run shows it matters, it wants a node-level
budget, which is a different shape.

### 14.8 `roymctl` has no operator surface for this queue's dead letters

B4 left this open (backlog, "the DLQ operator surface"). B5 multiplies the
dead-letter population by the member count, since one unreachable member per
message now produces its own row. Still out of scope here — it is one `roymctl`
command and a WIT verb, and folding it in would widen B5 past its own gates —
but it is closer to mattering than it was.

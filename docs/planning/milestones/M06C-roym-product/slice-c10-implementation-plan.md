# Slice C10 — Private group chat in the product (R4)

**Milestone:** [task.md](task.md) (row C10, `D-06C-5`, `D-06C-8`, the "Carried forward from M06B" table)
**Spec:** [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) — R4 (all five rows), D5, D10, "What is encrypted, and who can see what", Messaging, O1.
**ADR:** [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §5 (ordering) and Amendment 1 (owner-distributed key).
**Status:** Plan only. Written 2026-09-29 against `feat/m06c-slice-c9-trust` at `adecc34f`.
**Depends on:** C5 (complete). C9 (**partial** — see Q9).

This plan is written so that a different session can execute it without this
session's reasoning. Every file path is repo-relative. Line numbers are from
the commit above and must be re-checked before editing.

---

## §0 Read this first — decisions this plan needs you to confirm

The plan below assumes the **recommended** answer to each question. If you
choose differently, the section in the right column changes.

| # | Question | Recommendation | Changes |
|---|---|---|---|
| Q1 | R4 row 1 says: *"With no coordinator reachable, members who can reach each other still exchange and order messages."* **The substrate cannot do this today** (§3). Every cross-node call looks the peer up in the registry again (no cache), and a published record carries only the relay as a dial path. With the registry and relay down, every group push and every sync fails with `ServiceNotFound`. | **Spike first, with a stop gate (WO0, 3 days).** If the fix stays inside the proxy/registry client (a last-good lookup cache plus dialing with direct paths the long-lived iroh endpoint already learned), build it in C10. If it needs direct IP addresses **published** in registry records, stop: that is a privacy decision for an ADR. Then narrow the acceptance test with a spec edit (the `D-06C-2` pattern) and add a backlog row. Either way, record **Gap 10** in `task.md`. | §3, §11.3 test 5, §13 |
| Q2 | The product must show who owns a group (spec: *"The group's UI should say who the owner is"*), and a test must see that a scheduled rekey changed the key. The WIT exposes neither the owner nor the epoch. | **Add one additive host function `group-info`** to `syneroym:conversation` (same pattern as C8's `create`). Deriving the owner from `membership-history`'s first event is unreliable: a new member may not have synced the genesis entry when its first message arrives. | §4 |
| Q3 | A group needs a name every member sees. The host has no group name. | **The owner sends the name as a message of a reserved content type** (`application/vnd.roym.group-profile+json`) inside the group. It is honoured only when the author is the owner. The newest one by the ADR-0013 sort key wins. The owner sends it again after every `add-member`, because a new member cannot read anything from before it joined. | §2 D-C10-4, §7.4 |
| Q4 | The host joins you to a group as soon as its owner sends you a key. **Anybody who can reach your address can add you to a group.** Nothing asks you. | **Treat being added as a first contact from the owner.** The first time Roym sees a group, it calls `contacts.admit-first-contact` for the owner (same rule and same budget as 1:1). A refused group is stored as `refused` and not shown. Add `group.hide` / `group.unhide` so a person can hide a group they do not want. You stay a member underneath until the owner removes you; the UI says so. | §2 D-C10-6, §7.3, §7.5 |
| Q5 | Membership changes must be visible (R4 row 4), and the transcript must be identical everywhere (row 2). The host never sends membership changes to `on-message`. | **Copy every membership event into Roym's own message store as a row** of a reserved content type (`application/vnd.roym.membership-event+json`), with the entry id as row id and the owner as author. Then history, export, import, and the one sort rule all work with no second list. | §2 D-C10-5, §7.2 |
| Q6 | The host reports **one** delivery state for a group message: `pending` until every recipient settles, `failed` if any recipient failed. A member who was offline longer than `max_pending_age_secs` (30 days) makes the message `failed`, **even though that member may have received it from another member by sync**. | **Do not add a per-recipient host verb in C10.** Use honest group words: *"Not yet delivered to every member"*, *"Delivered to every member"*, *"Not delivered to every member"*, plus one notice that members also pass messages to each other. Backlog row for a per-recipient view. | §2 D-C10-8, §12 |
| Q7 | R4 row 2 needs posts from **deliberately skewed clocks**. All test nodes run in one process with one system clock. | **A `test-support` cargo feature on `syneroym-conversation`** with one hook: a per-service clock offset used only where the sender signs a group entry. Same feature and same guard as C9's planned WO7 hooks (C9 plan §9.3, `D-C9-10`). If C9 WO7 has not landed, C10 creates the feature with only this hook. | §5 |
| Q8 | Transaction cards are 1:1 by design. What happens to a card in a group? | **`conversation.send` refuses a card into a group. `transaction.sync` refuses a group conversation.** An incoming card in a group is stored as a message and the Hub shows a neutral block ("cards are not used in groups"). | §7.6, §9 |
| Q9 | The spec says *"Each release must pass its acceptance tests before the next begins."* R3 is **not** passed: C9 is partial (WO6–WO8 not built). | **Start C10's code now, but do not mark R4 passed before R3 is passed.** C10 needs C9's merged code only, not C9's e2e or Hub. The C10 `status.md` section and the spec's R4 row must say this. | §13 |
| Q10 | How does a person (or a test) check "every member has the same transcript"? | **Add `conversation.transcript-digest`**: a hash over the ordered list of `(id, author, sender_timestamp_ms, content_type)` of the rows this installation holds. The Hub shows it in the group's info panel as a short "transcript check" code. Members compare it by eye. It is also exactly what the acceptance test compares. | §2 D-C10-9, §6.2 |
| Q11 | `add-member` always fetches the new member's prekey bundle over the network (`crates/conversation/src/group.rs:207`), so **the owner can add only a person who is online at that moment**. | **When this node already holds a 1:1 session with the new member, use the key pinned in that session and skip the fetch.** Small host change with a host unit test. A person you have never talked to must still be online when you add them; the UI says so. | §4.3 |

---

## §1 What C10 must deliver

From `task.md` row C10 and the spec's R4 table:

1. **R4 row 1 — no central chat server.** Members exchange group messages
   directly (gossip DAG). With no coordinator reachable, members who can
   reach each other still exchange and order messages. Depends on Q1.
2. **R4 row 2 — same order everywhere.** Every member's transcript is
   byte-identical after sync, whatever order messages arrived in, including
   posts from skewed clocks.
3. **R4 row 3 — only members can read.** A joiner cannot read messages from
   before the join. A removed member cannot read messages after the removal.
   A scheduled rekey with stable membership still changes the key.
4. **R4 row 4 — membership is visible.** Join and removal are events in the
   transcript. Every member's membership history is identical after sync. No
   key reaches a party absent from that history.
5. **R4 row 5 — offline catch-up.** A member offline for a while pulls the
   gap from **any** online peer, not only from the author, and converges.
6. **Product surface** (`task.md` row C10): group naming and roster UI, the
   owner's read access stated in the UI, and the carried-forward B5 limits
   surfaced honestly (§2 D-C10-8, D-C10-10).
7. **Backlog row** "An inbound group message is recorded as `unsupported-kind`"
   (deferred-backlog §5, target C10) closed.

For how the finished feature looks to a person, read §16 (the demo script)
first.

**Not in C10:** MLS; member-initiated invites or approval votes; leaving a
group (only hiding it, Q4); message search inside groups as a feature (the
existing search keeps working, §7.6); pinned messages; attachments;
multi-device; group continuity after a restore onto a new machine (§12);
a per-recipient delivery view (Q6); a conversation dead-letter operator
surface; any change to the DAG wire format or `MAX_PARENTS`.

**The carried-forward `heads()`/`MAX_PARENTS` limit.** No C10 feature reads
DAG parent links. Ordering comes only from `(sender_timestamp, author, id)`.
Keep it that way; a reviewer should reject any change that reads
`parents`.

---

## §2 Decisions (D-C10-n)

| # | Decision | Why |
|---|---|---|
| D-C10-1 | **Group verbs live in the `conversation` service under a new `group.` method prefix.** `group.create`, `group.rename`, `group.add-member`, `group.remove-member`, `group.info`, `group.sync`, `group.hide`, `group.unhide`. Sending, reading, deleting, and exporting reuse the existing `conversation.*` verbs, which already accept a group conversation id. | The spec's service table puts group keys in Conversation. A separate prefix keeps `conversation.*` meaning "any conversation" and makes the group-only verbs easy to find. |
| D-C10-2 | **One additive host function, `group-info`, and nothing else on the WIT** (Q2). The stale `conversation-kind` comment is corrected at the same time. | Additive functions keep old components deployable. The owner and the epoch are facts only the host holds. |
| D-C10-3 | **A Roym `ConversationRow` gains `kind` (required) and `group: Option<GroupMeta>`.** For a group, `peer_address` holds the **owner's** address and `peer_person_did` the owner's person DID when a contact carries it. `SCHEMA_VERSION` of the `conversation` service goes 2 → 3. No migration: pre-release, and an older bundle fails at the existing version gate (`backup.rs` `import`). | One collection for both kinds keeps `conversation.list`, export, and import unchanged in shape. The owner is the one party a group's trust hangs on, and the one the first-contact decision is made against (D-C10-6). |
| D-C10-4 | **Group name = the newest owner-authored `group-profile` message by sort key** (Q3). Body is exactly `{"name": "<text>", "version": 1}`. Name is 1–80 characters after trimming, with no control characters. It is always displayed as text. A profile message from a non-owner is refused (`refused_messages` reason `not-owner`) and changes nothing. | Every member that can read the message computes the same name. The owner is the only writer, so there is no conflict rule to design. Text-only display follows the card rule (`D-06C-3`). |
| D-C10-5 | **Membership events are rows in Roym's own copy** (Q5): id = DAG entry id, author = owner, `sender_timestamp_ms` = the event's own timestamp, content type `application/vnd.roym.membership-event+json`, body `{"action","subject","epoch"}` (canonical key order). They are written by `sync_membership_rows` (§7.2), which runs on every group message, on `conversation.history` of a group, and after every group verb. These rows cannot be deleted and do not count in `message_count`. | Keeps one list and one sort rule (`roym_core::conversation::sort_key`). Export and restore carry them for free. |
| D-C10-6 | **Being added to a group is a first contact from its owner** (Q4). On the first message Roym sees in a group it has no row for: owner = self → `shown`; otherwise one `contacts.admit-first-contact` call with `sender_address = owner` → `allow` = `shown`, `blocked` = `refused{blocked}`, anything else = `refused{rate-limited}`. The decision is stored, so it runs once per group. After that, each message's **author** is checked with `block.check`, exactly like 1:1 (`D-06C-8`). | Same rule and same budget as 1:1, so a stranger cannot bypass the first-contact limit by using a group. Storing the decision stops a refused group from spending the budget again on every message. |
| D-C10-7 | **`group.hide` sets `admission = hidden`. New messages in a hidden group are refused into `refused_messages` (reason `group-hidden`). `group.unhide` sets it back to `shown`. Messages that arrived while hidden are not shown after unhide.** | The host keeps storing them underneath (Gap 4); Roym has no way to read one host message by id. Saying so is cheaper and more honest than a backfill that pages the whole host history. |
| D-C10-8 | **Group delivery words** (Q6): `pending` → "Not yet delivered to every member"; `delivered` → "Delivered to every member"; `failed` → "Not delivered to every member" + Retry. The words never name a member, never say "trying to reach", and never say "read" or "verified". | The host's group state is an aggregate. The carried-forward limit (a removed member's pending item settles `failed` only after the age window) must not be shown as progress toward a current member. These words are true in every case. |
| D-C10-9 | **Transcript check** (Q10): `transcript_digest(rows)` = `content_digest("roym-transcript:", [ {id, author, sender_timestamp_ms, content_type} ... ])` over the rows sorted by `sort_key`. Bodies are left out: a group row id is the DAG entry id, which is already a hash over the ciphertext, and leaving the body out means a local delete does not change the check. Rows refused locally (a blocked author) are not in the list, so the check differs for a member who blocked someone; the UI says so. | One number both a person and a test can compare. Reuses `syneroym_signed_record::content_digest`, already a `roym_core` dependency. |
| D-C10-10 | **Member signing keys are trust-on-first-use, and the UI says so in different words than a signed record.** Group messages are shown with no "verified" word. The info panel carries `GROUP_KEY_TRUST_NOTICE`. | Carried-forward limit 3 in `task.md`: two strengths, two words. |
| D-C10-11 | **Cards are 1:1 only** (Q8). `conversation.send` refuses `application/vnd.roym.card+json`, the group-profile type, and the membership-event type when the target is a group, and refuses the membership-event type everywhere. `transaction.sync` refuses a group conversation. | The transaction single-writer model is between two parties. Refusing at the one send choke point covers every producer. |
| D-C10-12 | **A restored group is history only.** After an import onto a clean node, the host does not know the group. `group.info` then returns `restored_only: true` and the Hub shows `GROUP_RESTORED_NOTICE`. | Group keys and the DAG live in the host store, which no backup carries (backlog §5, "Conversations cannot continue after a substrate moves to a new machine"). |
| D-C10-13 | **The Hub gets a separate Groups tab** (`crates/roym_web/ui/src/screens/groups.ts`). The Messages tab lists only direct conversations. | `messages.ts` is already 846 lines. A group thread also needs a roster panel the 1:1 thread does not. |
| D-C10-14 | **(Q1) Gap 10 fix, only if the spike keeps it inside `crates/core` / `crates/router`.** Otherwise the acceptance row is narrowed by a spec edit. | See §3. |

---

## §3 Gap 10 — cross-node calls need the registry and relay on every call (WO0, spike with stop gate)

### 3.1 What the code does today (verified 2026-09-29)

- `ProxyRouter::invoke_inner` looks in the local `EndpointRegistry` first. It
  knows only services on this node. On a miss it calls `invoke_remote`
  (`crates/router/src/proxy/router.rs:468-473`).
- `invoke_remote` calls `net_iroh::resolve_iroh_addr` (`router.rs:300-304`),
  which calls `RegistryClient::lookup(id, true)` **on every call**
  (`crates/router/src/net_iroh.rs:129-146`). Any error becomes
  `ProxyError::ServiceNotFound`.
- `RegistryClient` has no cache of resolved records
  (`crates/core/src/dht_registry/client.rs:23-30`). HTTP lookup times out
  after 10 s (`client.rs:20`). The DHT fallback is off in tests
  (`crates/substrate/tests/common/node.rs:246`).
- A published record carries only the node id and the relay URL. Direct
  addresses are removed on purpose (`crates/substrate/src/runtime/publish.rs:169-186`).
- The iroh `Endpoint` is **long-lived** (built once,
  `crates/router/src/connection_router.rs:91`), but every call opens a new
  connection (`crates/router/src/proxy/hop.rs:70-88`).
- `classify` maps `ServiceNotFound` to `Disposition::Unreachable`
  (`crates/conversation/src/transport.rs:480-493`), so pushes back off and
  retry forever; nothing is lost, but nothing moves either.

So with the registry and relay down, group push and group sync both fail at
lookup. R4 row 1, read literally, fails.

### 3.2 Spike steps (time box: 3 working days)

1. Add `CoordinatorNode` to the test harness (§11.3 setup): a plain
   `SubstrateNode` that hosts the registry and relay, and three
   `RoymNode`s pointing at it with `shared_registry` **and** `shared_relay`.
2. Form a group of three and exchange one round of messages (proves direct
   paths exist). Tear down the coordinator node. Member X sends one
   message. Wait 60 s. Record whether Y receives it, and the error in the
   `conversation` outbox (`conversation.outbox`) and in the node log
   (`RUST_LOG=warn,syneroym_router=debug`, redirected to a file).
3. **Fix A1 — last-good lookup.** In `RegistryClient` add
   `last_good: Mutex<HashMap<String, (EndpointInfo, Instant)>>` (bounded:
   1 024 entries, drop oldest). On a successful `lookup`, store the result.
   On a transport failure (connect error or timeout, **not** a clean
   "not found" answer), return the cached value if it is younger than
   24 h and its `not_after` has not passed. Add a short circuit breaker:
   after one transport failure, skip the HTTP attempt for 30 s and answer
   from the cache, so a dead registry does not cost 10 s per call.
   Re-run step 2.
4. If step 3 now fails at dial (relay down), check whether iroh 0.97's
   `Endpoint` keeps direct paths it learned for a node id, and whether
   `connect` uses them when the given relay is dead. If yes, **Fix A2**:
   after each successful call, save the peer's known direct addresses
   from the endpoint's own remote info into the same cache entry, and pass
   them in the `EndpointAddr` on the next dial. Re-run step 2.
5. **Stop gate.** If A1 + A2 pass step 2 within the time box and touch
   only `crates/core/src/dht_registry/client.rs`, `crates/router/src/net_iroh.rs`,
   and `crates/router/src/proxy/*`, keep them (with unit tests for the
   cache and breaker) and go to §11.3 test 5. Otherwise revert and take
   **branch B**.

### 3.3 Branch B (only if the stop gate fails)

- Spec edit (R4 row 1 acceptance test), recorded as a new milestone
  decision `D-06C-14` in `task.md`: *"With the registry and relay
  reachable but carrying no message content, members exchange and order
  messages member to member; no node other than members stores or orders
  a group message."* State the reason: the substrate needs the registry to
  find a peer and the relay to dial it.
- Backlog row §1 "Cross-node calls need the registry and relay on every
  call", trigger "a group must keep talking through a registry outage",
  with the spike's findings.
- §11.3 test 5 becomes "no member-to-member message passes through any
  non-member's storage": assert the coordinator node's own conversation
  store has no row for the group (it has no Roym conversation service
  in that group at all) and that `members` never includes it.

---

## §4 Host: `group-info` and the pinned-key add (WO1)

### 4.1 WIT

Edit `crates/wit_interfaces/wit/conversation/conversation.wit` and copy the
result, byte for byte, to all seven vendored copies:

```
crates/roym_catalog/wit/deps/conversation/conversation.wit
crates/roym_conversation/wit/deps/conversation/conversation.wit
crates/roym_directory/wit/deps/conversation/conversation.wit
crates/roym_profile/wit/deps/conversation/conversation.wit
crates/roym_transaction/wit/deps/conversation/conversation.wit
crates/roym_web/wit/deps/conversation/conversation.wit
test-components/dual-build-fixture/wit/deps/conversation/conversation.wit
```

Check with `for f in <the seven>; do diff -q crates/wit_interfaces/wit/conversation/conversation.wit $f; done`.
No gate checks these copies today (§15 item 17).

Changes:

1. Replace the stale comment on `conversation-kind` (lines 27-29):
   ```wit
   /// `direct` for a 1:1 conversation, `group` for a group created by
   /// `create-group` or joined by receiving its key.
   enum conversation-kind { direct, group }
   ```
2. Add after `record membership-event`:
   ```wit
   /// What this substrate knows about one group, in one call.
   record group-info {
       /// The group's owner, as a routing service id. Fixed at creation.
       owner: string,
       /// True when this service is the owner.
       is-owner: bool,
       /// True when this service is in the current member list.
       is-member: bool,
       /// Current members, sorted. The same list `members` returns.
       members: list<string>,
       /// The newest epoch this substrate has seen named, by a key or by
       /// a membership entry.
       epoch: u64,
       /// The newest epoch this substrate holds a key for. Lower than
       /// `epoch` after this service was removed: it saw the removal but
       /// was not given the new key.
       key-epoch: u64,
       /// When this substrate stored the key for `key-epoch`, in Unix
       /// milliseconds on its own clock. For the owner, when the key was
       /// made; for a member, when the key arrived.
       key-stored-at: s64,
   }
   ```
3. Add after `sync-now`:
   ```wit
   /// `invalid-argument` for a direct conversation, `not-found` for an
   /// unknown id.
   group-info: func(conversation: conversation-id) -> result<group-info, conversation-error>;
   ```

### 4.2 Call sites (every one must change, or the workspace does not build)

| File | Change |
|---|---|
| `crates/rpc/src/conversation.rs` | New `pub struct ConversationGroupInfo { pub owner: String, pub is_owner: bool, pub is_member: bool, pub members: Vec<String>, pub epoch: u64, pub key_epoch: u64, pub key_stored_at: i64 }` (derive `Debug, Clone, PartialEq, Eq`). New trait method on `ConversationHost` after `sync_now` (`:167`): `async fn group_info(&self, service_id: &str, conversation: &str) -> Result<ConversationGroupInfo, ConversationError>;`. Re-export from `crates/rpc/src/lib.rs` beside the other `Conversation*` types. |
| `crates/conversation/src/lib.rs` | `impl ConversationHost for ConversationService` (`:360`): `async fn group_info(..) { self.group_info_impl(service_id, conversation).await }`. |
| `crates/conversation/src/group.rs` | New `pub(crate) async fn group_info_impl` (pseudo-code below). |
| `crates/control_plane/src/synsvc_native/conversation.rs` | `NeverConstructed` (`:19`): add `group_info` → `unreachable!(..)` like its siblings. |
| `crates/sandbox_wasm/src/host_capabilities.rs` | `NeverConstructedConversationHost` (`:124`): same. |
| `crates/sandbox_wasm/src/host_capabilities/capabilities_messaging.rs` | `impl wit_conversation::Host for HostState` (`:102`): `async fn group_info(&mut self, conversation: String) -> Result<wit_conversation::GroupInfo, wit_conversation::ConversationError>` — upgrade `self.conversation` exactly like `membership_history` (`:228`), call `conv.group_info(&self.component_id, &conversation)`, map with a new `conversation_wire::map_group_info` beside `map_membership_event` (`:89`). Read-only is **not** checked (it is a read). |
| `crates/app_host/src/types.rs` | Add `GroupInfo` to the `conversation` re-export list (`:20-24`). |
| `crates/app_host/src/lib.rs` | `trait AppConversation` (`:293`): `fn group_info(&self, conversation: String) -> impl Future<Output = Result<GroupInfo, ConversationError>> + Send;` |
| `crates/app_host/src/guest.rs` | `impl AppConversation for GuestHost` (`:228`): `async fn group_info(&self, conversation: String) -> Result<GroupInfo, ConversationError> { conv::group_info(&conversation) }`. |
| `crates/app_host_native/src/host.rs` | `impl AppConversation for NativeAppHost` (`:382`): lock state, `HostConversation::group_info(&mut *state, conversation).await.map(convert::group_info_out).map_err(convert::conversation_error_out)`. |
| `crates/app_host_native/src/convert/conversation.rs` | New `pub(crate) fn group_info_out(v: HostGroupInfo) -> GuestGroupInfo`, beside `membership_event_out` (`:63`). |
| `crates/roym_core/src/signing.rs` | `#[cfg(test)] impl AppConversation for TestHost` (`:506`): add `group_info` returning `Err(ConversationError::NotFound)`. |
| `test-components/dual-build-fixture/src/app.rs` | New request variant `GroupInfo { conversation: String }` (serde `op = "group-info"`), beside `SyncNow` (`:163`). |
| `test-components/dual-build-fixture/src/app/dispatch.rs` | Arm beside `SyncNow` (`:258`): return `{"ok": {"owner", "is_owner", "is_member", "members", "epoch", "key_epoch"}}` (leave `key_stored_at` out; it is a clock read). |
| `crates/app_host_native/tests/dual_build_parity/conversation.rs` | After the existing `sync_now` check (`:198-202`), call `group-info` on the created group on both builds; assert `is_owner`, `members == [self]`, `epoch == 1`, `key_epoch == 1`, both builds equal. |
| `crates/app_host_native/tests/dual_build_parity/helpers.rs` | Add `("group-info-unknown", r#"{"op":"group-info","conversation":"conv:does-not-exist"}"#)` to the error-parity list beside `sync-now-unknown` (`:836`). |

Rebuild every component after the WIT change: `mise run build:test-components`
and `mise run build:roym`. The dual-build parity suites load pre-built
`wasm32-wasip2` artifacts; a stale artifact fails on the WASM side only
(the C9 status section's verification note).

Pseudo-code for the host function (`crates/conversation/src/group.rs`):

```rust
pub(crate) async fn group_info_impl(
    &self,
    service_id: &str,
    conversation: &str,
) -> Result<ConversationGroupInfo, ConversationError> {
    let store = self.store_for(service_id).await.map_err(internal)?;
    let conv = store.get_conversation(conversation).map_err(internal)?
        .ok_or(ConversationError::NotFound)?;
    if conv.kind != ConversationKind::Group {
        return Err(ConversationError::InvalidArgument("not a group conversation".into()));
    }
    let owner = conv.owner_address
        .ok_or_else(|| ConversationError::Internal("group has no owner".into()))?;
    let members = store.current_members(conversation).map_err(internal)?;
    let (key_epoch, key_stored_at) =
        store.current_epoch_row(conversation).map_err(internal)?.unwrap_or((0, 0));
    Ok(ConversationGroupInfo {
        is_owner: owner == service_id,
        is_member: members.iter().any(|m| m == service_id),
        owner, members,
        epoch: conv.current_epoch,
        key_epoch, key_stored_at,
    })
}
```

Host unit tests in `crates/conversation/src/group/tests.rs`:
`group_info_reports_owner_members_and_epoch` (owner view after create),
`group_info_is_invalid_for_a_direct_conversation`,
`group_info_after_scheduled_rekey_shows_a_higher_key_epoch` (reuse
`service_for_rekey_test`, `:449`).

### 4.3 Add a member you already talk to while they are offline (Q11)

`resolve_membership_change` (`crates/conversation/src/group.rs:191-225`),
`add` branch. Replace the unconditional fetch:

```rust
// A member this node already holds a 1:1 session with has a pinned
// signing key; the group key travels over that same session, so the
// network fetch adds nothing but a requirement that they be online now.
let sig_key = match store.session(member_address).map_err(internal)? {
    Some(sess) => sess.pinned_sig_key,
    None => self.fetch_prekey_bundle(service_id, member_address).await?.sig_key,
};
```

Check first that `store.session(addr)` returns the same key the bundle would
(`apply_incoming_group_key` already trusts `session.peer_sig_key` for the
owner, `transport.rs:305`). If the field names differ, use the session's
pinned peer signing key. Host unit test:
`add_member_uses_the_pinned_session_key_when_one_exists` (seed a session,
give the service a proxy that fails every call, assert `add-member`
succeeds and the membership entry's `subject_sig_key` equals the pinned
key).

---

## §5 Host: the clock-offset test hook (WO1, Q7)

`crates/conversation/Cargo.toml`:

```toml
[features]
test-support = []
```

(If C9 WO7 already added it, only add the new functions below.)

New `crates/conversation/src/test_support.rs`, declared in `lib.rs` as
`#[cfg(feature = "test-support")] pub mod test_support;`:

```rust
//! One-process test hooks for cross-node tests. Compiled only with the
//! `test-support` feature, which only `syneroym-substrate`'s dev build
//! enables. Keyed by service id: every test node runs as a task inside one
//! process, so an unkeyed hook would reach the wrong node.

static CLOCK_OFFSETS: LazyLock<Mutex<HashMap<String, i64>>> = ...;

/// From now on, `service_id` signs group entries as if its clock were
/// `offset_ms` ahead (negative: behind). The receiver's own checks use
/// the real clock, as they would against a real skewed peer.
pub fn set_clock_offset_ms(service_id: &str, offset_ms: i64);
pub fn clear_clock_offsets();
pub(crate) fn clock_offset_ms(service_id: &str) -> i64; // 0 when unset
```

New helper in `crates/conversation/src/store.rs`, beside `now_ms` (`:252`):

```rust
/// The clock a *sender* signs with. The real clock, except in a
/// `test-support` build where a test has skewed this service on purpose.
pub(crate) fn sender_now_ms(service_id: &str) -> i64 {
    #[cfg(feature = "test-support")]
    { now_ms() + crate::test_support::clock_offset_ms(service_id) }
    #[cfg(not(feature = "test-support"))]
    { let _ = service_id; now_ms() }
}
```

Use it in exactly three places in `crates/conversation/src/group.rs`:
`create_group_impl` (`:134`), `change_membership_impl` (`:264`),
`send_group` (`:359`). Do **not** use it in `scheduled_rekey_once`, in any
receive path, or in any validation.

`crates/substrate/Cargo.toml` `[dev-dependencies]`:
`syneroym-conversation = { workspace = true, features = ["test-support"] }`.
Note: with `cargo nextest run --workspace`, feature unification turns the
feature on for every test build in that run. That is harmless (the offset
map is empty unless a test writes it) and is the same trade-off `D-C9-10`
accepted.

---

## §6 `roym_core` vocabulary (WO2)

### 6.1 `crates/roym_core/src/conversation.rs`

- Delete `CONVERSATION_SCHEMA_VERSION` (`:12`). It has no reader; the real
  version is `roym_conversation::app::SCHEMA_VERSION` (§15 item 13).
- New enum, beside `ConversationRow`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "kebab-case")]
  pub enum ConversationRowKind { Direct, Group }
  ```
- `ConversationRow` (`:20`) gains two fields and a doc change:
  ```rust
  pub struct ConversationRow {
      pub id: String,
      pub kind: ConversationRowKind,          // required, no default
      /// Direct: the other party. Group: the group's owner.
      pub peer_address: String,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub peer_person_did: Option<String>,
      pub opened_at_secs: u64,
      pub last_activity_ms: i64,
      pub message_count: u64,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub group: Option<group::GroupMeta>,
  }
  ```
- Add `pub mod group;` (file `crates/roym_core/src/conversation/group.rs`,
  tests in `crates/roym_core/src/conversation/group/tests.rs` — module
  layout rule: `group.rs` beside `group/`).
- Update the existing `row()` test helper and every `ConversationRow { .. }`
  literal in the workspace (`git grep -n "ConversationRow {"`) to set
  `kind` and `group`.

### 6.2 New `crates/roym_core/src/conversation/group.rs`

```rust
//! The group half of Roym's conversation vocabulary: who may name a group,
//! how a membership change is written into Roym's own copy, how a first
//! sight of a group is admitted, and the one transcript check.

pub const GROUP_PROFILE_CONTENT_TYPE: &str = "application/vnd.roym.group-profile+json";
pub const MEMBERSHIP_EVENT_CONTENT_TYPE: &str = "application/vnd.roym.membership-event+json";
pub const GROUP_PROFILE_VERSION: u32 = 1;
pub const MAX_GROUP_NAME_CHARS: usize = 80;
pub const TRANSCRIPT_DIGEST_PREFIX: &str = "roym-transcript:";

/// Verbatim copies live in `crates/roym_web/ui/src/groups/words.ts`; a test
/// in `group/tests.rs` compares them character for character.
pub const OWNER_CAN_READ_NOTICE: &str = "The owner of this group makes and shares the group's \
    key, so the owner can read every message sent while they own it. Adding or removing a \
    member is shown to everyone in the group.";
pub const GROUP_KEY_TRUST_NOTICE: &str = "Each member's messages are signed with a key this \
    installation first saw when they joined. That is a weaker check than a signed record, so \
    messages here are never marked as verified.";
pub const GROUP_DELIVERY_NOTICE: &str = "\"Not yet delivered to every member\" means at least \
    one member has not received it directly. Members also pass messages to each other, so a \
    member may still receive it later.";
pub const GROUP_JOIN_BOUNDARY_NOTICE: &str = "You can read messages sent after you joined. \
    Messages from before you joined are not shared with you.";
pub const GROUP_REMOVED_NOTICE: &str = "You were removed from this group. You can still read \
    what you received before. You cannot read or send new messages.";
pub const GROUP_RESTORED_NOTICE: &str = "This group's history was restored from a backup. \
    This installation is not a member, so it cannot send or receive new messages here. Ask \
    the owner to add your new address.";
pub const GROUP_ADD_UNREACHABLE_MESSAGE: &str = "Could not reach this person to add them. \
    Someone you have not talked to before must be online when you add them.";
pub const GROUP_HIDDEN_NOTICE: &str = "A hidden group is not shown, and its new messages are \
    not kept here. This installation still receives them underneath, and you stay a member \
    until the owner removes you.";
pub const TRANSCRIPT_CHECK_NOTICE: &str = "Members who see the same code hold the same \
    messages in the same order. A member who blocked someone in the group sees a different \
    code.";
pub const CARDS_NOT_IN_GROUPS_MESSAGE: &str = "Cards are sent only in a 1:1 conversation.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum GroupAdmission {
    Shown,
    Hidden,
    Refused { reason: String },   // "blocked" | "rate-limited"
}

/// Which message set the current name. Compared by `sort_key` order, so
/// every member that read the same messages picks the same name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameSource { pub sender_timestamp_ms: i64, pub author: String, pub message_id: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_source: Option<NameSource>,
    pub admission: GroupAdmission,
    /// How many host membership events are already copied into Roym's
    /// own store. Lets `sync_membership_rows` skip work when nothing new
    /// arrived.
    pub membership_events_copied: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupNameError {
    #[error("a group name must not be empty")] Empty,
    #[error("a group name must be at most {MAX_GROUP_NAME_CHARS} characters")] TooLong,
    #[error("a group name must not contain control characters")] ControlCharacter,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupProfileError {
    #[error("group profile body is not valid JSON: {0}")] Json(String),
    #[error("group profile must be exactly {{\"name\": <string>, \"version\": 1}}")] Shape,
    #[error(transparent)] Name(#[from] GroupNameError),
}

/// Trims, then checks length in chars and refuses control characters.
pub fn validate_group_name(raw: &str) -> Result<String, GroupNameError>;

/// `{"name": <validated>, "version": 1}` as bytes. Callers validate first.
pub fn group_profile_body(name: &str) -> Vec<u8>;

/// Strict: exactly two keys, `version == 1`, and a name that passes
/// `validate_group_name`.
pub fn parse_group_profile(body: &[u8]) -> Result<String, GroupProfileError>;

/// Applies a profile message to `meta` if it sorts after the current
/// source. Returns true when the name changed.
pub fn apply_group_profile(meta: &mut GroupMeta, name: String, src: NameSource) -> bool;

/// Canonical body for a membership row: `{"action","epoch","subject"}`.
pub fn membership_event_body(action: &str, subject: &str, epoch: u64) -> String;

/// D-C10-6 as a pure function. `admit_answer` is the `admission` string
/// `contacts.admit-first-contact` returned, or None if it was not asked.
pub fn admission_for_new_group(is_owner: bool, admit_answer: Option<&str>) -> GroupAdmission;

/// D-C10-9. Sorts a copy of `rows` by `sort_key`.
pub fn transcript_digest(rows: &[MessageRow]) -> Result<String, EnvelopeError>;

/// True for the two reserved types a person never authors through
/// `conversation.send`, and which `delete-message`/`search` skip.
pub fn is_group_system_type(content_type: &str) -> bool;
```

Pseudo-code for the non-trivial ones:

```rust
pub fn validate_group_name(raw: &str) -> Result<String, GroupNameError> {
    let name = raw.trim();
    if name.is_empty() { return Err(Empty) }
    if name.chars().count() > MAX_GROUP_NAME_CHARS { return Err(TooLong) }
    if name.chars().any(char::is_control) { return Err(ControlCharacter) }
    Ok(name.to_string())
}

pub fn apply_group_profile(meta: &mut GroupMeta, name: String, src: NameSource) -> bool {
    let newer = match &meta.name_source {
        None => true,
        Some(cur) => (src.sender_timestamp_ms, &src.author, &src.message_id)
                   > (cur.sender_timestamp_ms, &cur.author, &cur.message_id),
    };
    if !newer { return false }
    let changed = meta.name.as_deref() != Some(name.as_str());
    meta.name = Some(name);
    meta.name_source = Some(src);
    changed
}

pub fn admission_for_new_group(is_owner: bool, admit_answer: Option<&str>) -> GroupAdmission {
    if is_owner { return GroupAdmission::Shown }
    match admit_answer {
        Some("allow") => GroupAdmission::Shown,
        Some("blocked") => GroupAdmission::Refused { reason: "blocked".into() },
        _ => GroupAdmission::Refused { reason: "rate-limited".into() },
    }
}

pub fn transcript_digest(rows: &[MessageRow]) -> Result<String, EnvelopeError> {
    let mut sorted: Vec<&MessageRow> = rows.iter().collect();
    sorted.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    let lines: Vec<Value> = sorted.iter().map(|r| json!({
        "id": r.id, "author": r.author,
        "sender_timestamp_ms": r.sender_timestamp_ms, "content_type": r.content_type,
    })).collect();
    content_digest(TRANSCRIPT_DIGEST_PREFIX, &Value::Array(lines))
}
```

Unit tests (`group/tests.rs`): name validation (empty, whitespace only,
exactly 80, 81, a `\u{0007}`, an emoji counts as one char); strict profile
parse (extra key, `version: 2`, number name, empty name); `apply_group_profile`
is order-independent (apply A then B equals B then A) and ties break by
author then id; `admission_for_new_group` table; `transcript_digest` is
arrival-order independent, changes when a row is added, and does **not**
change when a row is tombstoned; the verbatim-notice test reads
`../roym_web/ui/src/groups/words.ts` the same way `membership/tests.rs:547`
reads `membership.ts`.

### 6.3 `crates/roym_core/src/router.rs`

Add `("group.", CONVERSATION, MethodAuth::Owner),` right after the
`conversation.` row (`:23`). The existing tests
(`no_prefix_is_a_prefix_of_another`, `reachable_services_set_equals_siblings`)
must still pass unchanged.

---

## §7 `roym_conversation` (WO3)

### 7.1 Files

| File | What |
|---|---|
| `crates/roym_conversation/src/app.rs` | `SCHEMA_VERSION` 2 → 3 (`:31`). `pub mod group;`. New arms in `invoke`'s match (`:179`), one line each: `"group.create" => group::create(host, &req).await`, and likewise `rename`, `add-member`, `remove-member`, `info`, `sync`, `hide`, `unhide`; plus `"conversation.transcript-digest" => messages::transcript_digest(host, &req).await`. |
| `crates/roym_conversation/src/app/group.rs` (**new**, target ≤ 450 lines) | The eight `group.*` verbs and `sync_membership_rows`. |
| `crates/roym_conversation/src/app/inbox.rs` | Replace the `unsupported-kind` branch (`:80-87`) with a dispatch to `inbox::group::on_group_message`. `pub(crate) mod group;`. `upsert_conversation` (`:171`) sets `kind: Direct, group: None`. |
| `crates/roym_conversation/src/app/inbox/group.rs` (**new**, target ≤ 250 lines) | The group branch of the inbox (§7.3). |
| `crates/roym_conversation/src/app/messages.rs` | §7.6 changes. |
| `crates/roym_conversation/src/app/backup.rs` | No code change beyond the version gate (rows carry the new fields). Confirm `import` puts rows as-is (`:123-160`). |

Every function ≤ 100 lines; every match arm ≤ 10 lines (AGENTS.md).

### 7.2 `sync_membership_rows` (in `app/group.rs`)

```rust
/// Copies host membership events Roym does not hold yet into its own
/// `messages` collection (D-C10-5). Cheap when nothing changed: compares
/// the host's event count with `meta.membership_events_copied`.
pub(crate) async fn sync_membership_rows<H: AppHost>(
    host: &H,
    row: &mut ConversationRow,
    info: &GroupInfo,
) -> Result<(), String> {
    let events = AppConversation::membership_history(host, row.id.clone()).await
        .map_err(|e| format!("{e:?}"))?;
    let meta = row.group.as_mut().ok_or("not a group row")?;
    if events.len() as u32 == meta.membership_events_copied { return Ok(()) }
    let direction = if info.is_owner { Direction::Outgoing } else { Direction::Incoming };
    for ev in &events {
        if load_message(host, &ev.entry).await?.is_some() { continue }
        put_message(host, &MessageRow {
            id: ev.entry.clone(),
            conversation: row.id.clone(),
            author: info.owner.clone(),
            direction,
            sender_timestamp_ms: ev.sender_timestamp,
            content_type: MEMBERSHIP_EVENT_CONTENT_TYPE.into(),
            body_encoding: BodyEncoding::Utf8,
            body: Some(membership_event_body(&ev.action, &ev.subject, ev.epoch)),
            state: StoredState::Delivered,
            last_error: None, deleted_at_secs: None,
            stored_at_secs: clock::now_secs(),
        }).await?;
        row.last_activity_ms = row.last_activity_ms.max(ev.sender_timestamp);
    }
    meta.membership_events_copied = events.len() as u32;
    put_conversation(host, row).await     // new small helper, see below
}
```

Add `pub(crate) async fn put_conversation<H: AppHost>(host, row: &ConversationRow)`
to `app.rs` beside `put_message`, and make `upsert_conversation`,
`upsert_conversation_open`, and `upsert_conversation_activity` use it
(they each repeat the same `AppDataLayer::put` today; the duplication gate
will count a fourth copy).

### 7.3 The inbox's group branch (`app/inbox/group.rs`)

```rust
pub(super) async fn on_group_message<H: AppHost>(
    host: &H, msg: &Message, now: u64,
) -> Result<(), String> {
    let info = AppConversation::group_info(host, msg.conversation.clone()).await
        .map_err(|e| format!("group-info: {e:?}"))?;

    // 1. First sight: admit once, against the owner (D-C10-6).
    let mut row = match load_conversation(host, &msg.conversation).await? {
        Some(r) => r,
        None => new_group_row(host, &msg.conversation, &info, now).await?,
    };
    let admission = row.group.as_ref().map(|g| g.admission.clone());
    match admission {
        Some(GroupAdmission::Shown) => {}
        Some(GroupAdmission::Hidden) => return record_refused(host, msg, "group-hidden", now).await,
        Some(GroupAdmission::Refused { reason }) => return record_refused(host, msg, &reason, now).await,
        None => return Err("group row without group meta".into()),
    }

    // 2. Per-author block, exactly as 1:1 (D-06C-8).
    let person_did = person_did_for_address(host, &msg.author).await;
    if is_blocked(host, &msg.author, person_did.as_deref()).await? {   // extract from inbox.rs
        return record_refused(host, msg, "blocked", now).await;
    }

    // 3. Reserved content types.
    if msg.content_type == DELETION_REQUEST_CONTENT_TYPE {
        return honour_deletion_request(host, msg, now).await;           // extract from inbox.rs
    }
    if msg.content_type == MEMBERSHIP_EVENT_CONTENT_TYPE {
        return record_refused(host, msg, "reserved-type", now).await;  // never sent as a message
    }
    if load_message(host, &msg.id).await?.is_some() { return Ok(()) }  // idempotent retry
    if msg.content_type == GROUP_PROFILE_CONTENT_TYPE {
        if msg.author != info.owner { return record_refused(host, msg, "not-owner", now).await }
        let Ok(name) = parse_group_profile(&msg.body) else {
            return record_refused(host, msg, "bad-group-profile", now).await;
        };
        let meta = row.group.as_mut().ok_or("no meta")?;
        apply_group_profile(meta, name, NameSource {
            sender_timestamp_ms: msg.sender_timestamp,
            author: msg.author.clone(), message_id: msg.id.clone(),
        });
        // falls through: the profile message is stored, and shown as an event
    }

    // 4. Store, then catch up membership rows.
    put_message(host, &incoming_row(msg, now)).await?;                  // extract from inbox.rs
    row.message_count += 1;
    row.last_activity_ms = row.last_activity_ms.max(msg.sender_timestamp);
    sync_membership_rows(host, &mut row, &info).await                   // also writes `row`
}

async fn new_group_row<H: AppHost>(host: &H, id: &str, info: &GroupInfo, now: u64)
    -> Result<ConversationRow, String>
{
    let owner_did = person_did_for_address(host, &info.owner).await;
    let answer = if info.is_owner { None } else {
        let resp = profile_call(host, "contacts.admit-first-contact",
            json!({ "sender_address": info.owner, "sender_person_did": owner_did })).await?;
        resp.result.and_then(|v| v.get("admission").and_then(Value::as_str).map(str::to_string))
    };
    let row = ConversationRow {
        id: id.into(), kind: ConversationRowKind::Group,
        peer_address: info.owner.clone(), peer_person_did: owner_did,
        opened_at_secs: now, last_activity_ms: 0, message_count: 0,
        group: Some(GroupMeta { name: None, name_source: None,
            admission: admission_for_new_group(info.is_owner, answer.as_deref()),
            membership_events_copied: 0 }),
    };
    put_conversation(host, &row).await?;
    Ok(row)
}
```

**Refactors in `inbox.rs` so the two branches share code** (the duplication
gate counts shape, not text): extract `is_blocked`, `honour_deletion_request`,
and `incoming_row(msg, now) -> MessageRow` from `on_message_inner`
(`:74-169`). The direct branch calls the same three.

A group row's `peer_address` is the owner, so the direct branch's
`load_conversation(...).is_none()` first-contact test is never reached for a
group id — the dispatch at `:80` happens first.

### 7.4 The `group.*` verbs (`app/group.rs`)

Every verb returns `Response`. `resolve_member_address` is the existing
`resolve_open_address` (`messages.rs:27`) made `pub(crate)` and reused as is
(it already takes `address` or `person_did`).

| Verb | Params | Result | Rules |
|---|---|---|---|
| `group.create` | `name?` | `{conversation_id, owner_address}` | Validate `name` if given. `AppConversation::create_group`. `group_info`. Write the row (`kind: Group`, admission `Shown`, `name_source: {now_ms, owner, "local"}` if named). `sync_membership_rows` (writes the genesis row). |
| `group.rename` | `conversation`, `name` | `{name, sent: bool, send_error?}` | Owner only (`info.is_owner`, else `invalid_params("only the group's owner can rename it")`). Validate. If the group has more than one member, send the profile message via `send_and_record` (§7.6) and use its id/timestamp as the `NameSource`; else a local `NameSource`. Store the row. |
| `group.add-member` | `conversation`, `address` or `person_did` | `{added, epoch, name_sent: bool, send_error?}` | Owner only. `AppConversation::add_member`. Map `Unreachable(_)` → `invalid_params(GROUP_ADD_UNREACHABLE_MESSAGE)`, `QuotaExceeded` → `invalid_params("this group is full")`. `sync_membership_rows`. If the row has a name, re-send the profile (Q3); a send failure is reported, not fatal. |
| `group.remove-member` | same | `{removed, epoch}` | Owner only. `AppConversation::remove_member`. `sync_membership_rows`. |
| `group.info` | `conversation` | see below | Works for owner and members. If the host answers `NotFound` and a Roym row exists → `{restored_only: true, name, ...row fields}`. |
| `group.sync` | `conversation` | `{synced: true, events_copied}` | `AppConversation::sync_now`, then `sync_membership_rows`. |
| `group.hide` / `group.unhide` | `conversation` | `{admission}` | Any member. `hide` sets `Hidden`; `unhide` sets `Shown` (also from `Refused`: this is the person's own choice to accept). |
| `conversation.transcript-digest` | `conversation` | `{digest, rows}` | `messages_of` then `transcript_digest`. |

`group.info` result:

```json
{
  "conversation_id": "...", "name": "Street Garden" | null,
  "owner_address": "...", "owner_person_did": "..." | null,
  "is_owner": true, "is_member": true, "restored_only": false,
  "admission": {"state": "shown"},
  "epoch": 4, "key_epoch": 4, "key_stored_at_ms": 1790000000000,
  "members": [ {"address": "...", "person_did": "..." | null, "is_owner": false} ],
  "can_read_new_messages": true,            // key_epoch >= epoch && is_member
  "notices": { "owner_can_read": OWNER_CAN_READ_NOTICE,
               "key_trust": GROUP_KEY_TRUST_NOTICE,
               "delivery": GROUP_DELIVERY_NOTICE,
               "join_boundary": GROUP_JOIN_BOUNDARY_NOTICE,
               "removed": GROUP_REMOVED_NOTICE | null,     // only when !is_member
               "restored": GROUP_RESTORED_NOTICE | null }  // only when restored_only
}
```

`person_did` per member comes from one `contacts.list` call mapped by
`conversation_address` (the logic of `person_did_for_address`, `app.rs`,
changed to build the whole map once — do not call it per member).

### 7.5 Who can do what

- Owner only: `rename`, `add-member`, `remove-member`. The host already
  refuses non-owners (`PermissionDenied`, `group.rs:240-242`); Roym checks
  first to give a clear message.
- The owner cannot remove itself (host: "the owner is always a member").
  Roym passes the host message through as `invalid_params`.
- A removed member keeps its rows. `conversation.send` into that group
  returns the host's refusal as `invalid_params(GROUP_REMOVED_NOTICE)`.

### 7.6 Changes to existing `conversation.*` verbs (`app/messages.rs`)

1. **Extract `send_and_record`** from `send` (`:133-208`):
   `pub(crate) async fn send_and_record<H: AppHost>(host, conversation: &str, content_type: &str, body: &[u8]) -> Result<MessageRow, Response>`
   — everything from `AppConversation::send` through `upsert_conversation_activity`.
   `send` (the verb) keeps param parsing and the new guard:
   ```rust
   let row = load_conversation(host, &conversation).await?;
   let is_group = row.as_ref().is_some_and(|r| r.kind == ConversationRowKind::Group);
   if content_type == MEMBERSHIP_EVENT_CONTENT_TYPE
       || (is_group && (content_type == CARD_CONTENT_TYPE
                        || content_type == GROUP_PROFILE_CONTENT_TYPE)) {
       return Response::invalid_params(if content_type == CARD_CONTENT_TYPE
           { CARDS_NOT_IN_GROUPS_MESSAGE } else { "this content type is reserved" });
   }
   ```
   Map host errors for groups: `InvalidArgument(m)` → `invalid_params(m)`
   (this carries "a group with no other member has nowhere to deliver");
   `PermissionDenied` → `invalid_params(GROUP_REMOVED_NOTICE)`.
2. **`list`** (`:94`): optional params `kind` (`"direct"` | `"group"`) and
   `include_hidden` (default false). Skip group rows whose admission is not
   `Shown` unless `include_hidden`.
3. **`history`** (`:261`): load the row; if it is a group, call
   `group_info` and `sync_membership_rows` before reading (ignore a
   `NotFound` from the host: a restored group). Add `"kind"` to the result:
   `{ "messages": [...], "kind": "direct" | "group" }`.
4. **`delete_message`** (`:361`): refuse a row whose content type is a
   group system type (`is_group_system_type`) with
   `invalid_params("this row records a group change and cannot be deleted")`.
   For a group message, use a new note:
   `DELETE_NOTE_GROUP = "The local copy is removed and a deletion record kept. A request to delete it was sent to the other members; whether their clients honour it is theirs to decide, and this cannot check. Every member already holds the key this message was sent under."`
   (the last sentence is the spec's own point about groups).
5. **`search`** (`:415`): add `"content_type": {"$nin": [MEMBERSHIP_EVENT_CONTENT_TYPE, GROUP_PROFILE_CONTENT_TYPE]}`
   to the filter. The filter DSL supports `$nin` (`crates/data_db/src/filter.rs:239`).
6. **`transcript_digest`** — the new verb from §7.4.

---

## §8 `roym_web`

No code change: `web` forwards by the `router.rs` table (§6.3). Check that
`crates/roym_web/tests/dual_build_parity/wire_origin.rs` still refuses every
`group.*` method from the wire (`-32013`) — add `("group.info", json!({"conversation": "x"}))`
to its method list (`:113`).

---

## §9 `roym_transaction` — refuse a group (Q8)

`crates/roym_transaction/src/app/sync.rs`: make `fetch_sync_messages`
(`:118`) return `(Vec<Value>, Option<String>)` — the messages and the
result's `kind`. In `sync` (`:40`), after the first fetch, if the kind is
`"group"`, return `Response::invalid_params(CARDS_NOT_IN_GROUPS_MESSAGE)`
without writing any sync state. `request.set` / `quote.set` need no change:
their card send goes through `conversation.send`, which now refuses a group
(§7.6).

---

## §10 `roymctl roym group` (WO4)

New `apps/roymctl/src/commands/roym/group.rs` (≤ 300 lines) with
`GroupCommands`, registered as `RoymCommands::Group { #[command(subcommand)] command: GroupCommands }`
in `apps/roymctl/src/commands/roym.rs` (enum at `:39`, `handle` at `:117`).
Every variant takes `--gateway-url` (default `DEFAULT_GATEWAY_URL`) and
`--host`, and calls `directory::call_and_print` with `RpcCtx`
(`directory.rs:209-232`), like `trust.rs`.

| Subcommand | Method | Params |
|---|---|---|
| `create [--name N]` | `group.create` | `{name}` |
| `rename --group G --name N` | `group.rename` | |
| `add --group G (--address A \| --person-did D)` | `group.add-member` | |
| `remove --group G (--address A \| --person-did D)` | `group.remove-member` | |
| `list [--include-hidden]` | `conversation.list` | `{kind: "group", include_hidden}` |
| `info --group G` | `group.info` | |
| `send --group G --body B` | `conversation.send` | `{conversation, body}` |
| `history --group G [--limit N]` | `conversation.history` | |
| `sync --group G` | `group.sync` | |
| `check --group G` | `conversation.transcript-digest` | |
| `hide --group G` / `unhide --group G` | `group.hide` / `group.unhide` | |

CLI parse tests in `apps/roymctl/src/commands/roym/tests.rs` for every
subcommand, and that `--address` and `--person-did` are mutually exclusive
(`#[arg(group = "who")]`).

---

## §11 Tests

### 11.1 Unit tests

- `crates/conversation/src/group/tests.rs`: §4.2 (three) and §4.3 (one).
- `crates/roym_core/src/conversation/group/tests.rs`: §6.2.
- `crates/roym_core/src/router.rs` tests: unchanged, must pass.

### 11.2 Dual-build parity (WO5) — new `crates/roym_web/tests/dual_build_parity/group.rs`

Register `mod group;` in `crates/roym_web/tests/dual_build_parity.rs`.
Numbering starts at 201. The harness is one node per build, so it can only
build groups **this node owns**; the joined-group path (admission against
another owner) is proven by the unit test of `admission_for_new_group` and
by §11.3. Group ids differ per build (random nonce), so compare shapes after
`strip_volatile`, and compare ids only within one build.

| # | Scenario | Assert (both builds, identical after normalising ids) |
|---|---|---|
| 201 | `group.create` with a name | `conversation.list {kind: group}` has one row, `kind: group`, `name` set; `group.info` → `is_owner`, members `[self]`, `epoch 1`, `key_epoch 1`, all notices present; `conversation.history` has exactly one row, content type membership-event, body `{"action":"add","epoch":1,"subject":<self>}` |
| 202 | `group.rename` validation | empty, whitespace, 81 chars, a control char → the same `invalid_params` text on both builds; 80 chars accepted |
| 203 | send into a group with no other member | `invalid_params` naming "nowhere to deliver"; nothing stored in Roym's copy |
| 204 | an inbound message injected into this node's own group (`h.deliver`, author `did:key:zPeer204`) | stored, `message_count 1`, `history` shows membership row then the message in sort order |
| 205 | a blocked author in a shown group | `refused_messages` reason `blocked`; the group row stays shown; history unchanged |
| 206 | group profile from a non-owner author | refused `not-owner`, name unchanged; an injected profile authored by the owner (own service id) at a newer timestamp changes the name; an older one injected afterwards does not |
| 207 | a card into a group | `conversation.send` with the card content type → `CARDS_NOT_IN_GROUPS_MESSAGE`; `transaction.sync {conversation: group}` → the same message |
| 208 | delete | `delete-message` on the membership row → refused; on an injected group message → ok, note is `DELETE_NOTE_GROUP` |
| 209 | transcript digest | stable across two calls; changes after one more injected message; unchanged after deleting that message |
| 210 | hide / unhide | after `group.hide`, an injected message → refused `group-hidden`; `list` hides the group, `include_hidden` shows it; after `unhide` the next message is stored and the hidden-time message is still absent |
| 211 | search | a query matching text in a membership body and a group profile body returns neither |
| 212 | export / import round trip into a second, empty installation (the `scenario_188` pattern) | bundle `schema_version 3`; the group row (kind, meta) and membership rows are restored; `group.info` on the import side → `restored_only: true` with `GROUP_RESTORED_NOTICE` |
| 60 (rewrite) | rename to `scenario_60_group_message_is_stored_in_roym_copy_parity` | an injected message into a host group that has no Roym row yet (owned by self) → row created with admission shown, message stored, `refused_messages` empty |

Also update `scenario_8_status_on_all_six_services` for the conversation
service's `schema_version: 3`.

### 11.3 Cross-installation e2e (WO6)

**Shared setup — new `crates/substrate/tests/common/roym_group.rs`**
(declared in `common/mod.rs`):

- `pub struct CoordinatorNode(SubstrateNode)` booted with the default
  roles (it hosts the registry and relay). `registry_url()`, `relay_url()`,
  `async fn teardown(self)`.
- Extend `RoymNode::make_builder` (`common/roym.rs:211`) with an optional
  `shared_relay_url` field on `RoymNode` (threaded the same way as
  `shared_registry_url`, including `resume`, `:475`). Add
  `RoymNode::boot_on(label, base_path, coord: &CoordinatorNode, owner, role)`.
- `pub fn group_role(rekey_secs: u64) -> AppSandboxRole` built on
  `common::roym::fast_conversation_role` with `conversation_group_sync_secs: 1`
  and `conversation_group_rekey_secs: rekey_secs`. Do not copy
  `fast_conversation_role` or `wait_until`.
- `pub async fn form_group(owner: &RoymNode, name: &str, members: &[&RoymNode]) -> String`:
  profiles and contacts set up both ways (each member adds the owner as a
  contact and the owner adds each member), `group.create`, one
  `group.add-member` per member, then `wait_until` every member's
  `group.info` has `is_member` and the name.
- `pub async fn projection(node: &RoymNode, gid: &str) -> Vec<Value>`:
  `conversation.history` rows mapped to
  `{id, author, sender_timestamp_ms, content_type, body}`.
- `pub async fn digest(node: &RoymNode, gid: &str) -> String`.
- `pub async fn converge(nodes: &[&RoymNode], gid: &str, expect_rows: usize)`:
  loop `group.sync` on every node until all digests are equal and each
  history has `expect_rows` rows (budget 60 s).

Every test starts with `let _serial_guard = common::serial_guard().await;`,
uses only harness-allocated ports, and stays ≤ 100 lines (or carries
`#[expect(clippy::too_many_lines, reason = "...")]`). Split across two files
so each stays ≤ 800 lines.

**`crates/substrate/tests/roym_group_e2e.rs`**

| Test | Steps | R4 row |
|---|---|---|
| `three_members_see_one_order_from_skewed_clocks` | C + Z (owner), X, Y. `form_group`. `test_support::set_clock_offset_ms`: X +90 000, Y −90 000, Z 0. All three send two messages with `join_all` (six sends at once). `converge`. Assert: the three projections serialise to identical bytes; the order equals sort by `(sender_timestamp_ms, author, id)`; the three digests are equal; three membership rows (genesis, add X, add Y) are identical everywhere. `clear_clock_offsets` at the end. | 2, 4 |
| `a_joiner_reads_nothing_before_joining` | Form Z, X. Two messages. Z adds W. `converge` on Z, X, W. W's history has no message row with a sender timestamp before the `add W` event and no id of the two early messages; W reads a message sent after the join. Membership rows identical on Z, X, W. | 3, 4 |
| `a_removed_member_reads_nothing_after_removal` | Form Z, X, Y. Z removes Y. Z sends "after removal". `converge` on Z, X. Y's `group.sync`, then: Y's history has the removal row, not the new message; Y's `group.info` → `is_member: false`, `key_epoch < epoch`, `removed` notice set; Y's `conversation.send` → error with `GROUP_REMOVED_NOTICE`. Scheduled rekey later (§ below) does not raise Y's `key_epoch`. | 3, 4 |
| `a_scheduled_rekey_changes_the_key_with_stable_membership` | Nodes with `group_role(5)`. Form Z, X, Y. Record Z's `epoch`. Wait until Z's `epoch` grows and X's and Y's `key_epoch` equal it. Membership rows unchanged in count. A message sent after the rekey is read by all three. | 3 |

**`crates/substrate/tests/roym_group_offline_e2e.rs`**

| Test | Steps | R4 row |
|---|---|---|
| `an_offline_member_pulls_the_gap_from_another_member` | Form Z, X, Y. `Y.stop(None)`. X sends two, Z sends two. `Z.stop(None)` (the owner and one author is now offline). `Y.resume(None)`, `Y.republish_registry()`, login. Y `group.sync` until its digest equals X's. Y holds all four messages, so it pulled Z's messages from X, not from Z. | 5 |
| `members_talk_with_no_coordinator_reachable` | **Branch A only** (Q1). Form Z, X, Y on C. One round of messages. `C.teardown()`. X sends; wait until Y and Z hold it; Y sends; wait until X and Z hold it. Digests equal. **Branch B**: the §3.3 assertion instead. | 1 |
| `a_stranger_adding_you_is_a_first_contact` | X sets `contacts.set-limits {max_per_window: 0}`. W (not a contact of X) creates a group and adds X. W sends. X's `conversation.list {kind: group}` is empty; with `include_hidden` the group shows `refused: rate-limited`; `refused_messages` has W's message. X `group.unhide`s; W sends again; X now holds that message only. | safety (Q4) |

Put both binaries in nextest's `substrate-e2e` group (check the filter in
`.config/nextest.toml`; if it matches `roym_*_e2e` by name, no edit).

### 11.4 Hub (vitest) — see §12.

---

## §12 Hub (WO7)

### 12.1 Files

| File | What |
|---|---|
| `crates/roym_web/ui/src/groups/words.ts` (**new**) | The nine notice constants from §6.2, verbatim, plus `deliveryWords(state: "pending" \| "delivered" \| "failed"): string` (D-C10-8) and `membershipEventWords(event, nameOf): string` ("Z added X", "Z removed Y", "Z created the group"). |
| `crates/roym_web/ui/src/groups/words.test.ts` (**new**) | Every state maps to its words; no output ever contains "verified", "read", or "trying"; membership words use display names when known and the short address otherwise. |
| `crates/roym_web/ui/src/screens/groups.ts` (**new**, ≤ 600 lines) | `renderGroups(container)`: left = "New group" (name input + button) and the group list (`conversation.list {kind: "group"}`), plus a collapsed "Hidden groups (N)" section (`include_hidden`) with Unhide buttons. Right = thread (`conversation.history`) and an info panel (`group.info`). |
| `crates/roym_web/ui/src/screens/groups.test.ts` (**new**) | Rendering helpers: a name with markup becomes a text node; membership rows render as events, profile rows as "Z named the group …", card rows as the neutral block. |
| `crates/roym_web/ui/src/screens/messages.ts` | `reloadList` (`:188-221`) calls `conversation.list` with `{kind: "direct"}`. `ConversationRow` interface (`:33`) gains `kind`. No other change. |
| `crates/roym_web/ui/src/main.ts` | Tab list (`:137-147`): add `{ name: "Groups", render: () => renderGroups(tabContainer) }` after Messages. |

### 12.2 What the Groups screen shows

- **Thread.** One list in server order (the server already sorts). A
  message row: author display name (from `group.info.members`), text as a
  text node, time, and `deliveryWords(state)` for own messages. A
  membership row: a centred event line. A profile row: "Z named the group
  “…”". A card row: `renderRefusedCard("card", 1, CARDS_NOT_IN_GROUPS_MESSAGE)`
  or a new neutral block; never `renderCard`. Delete button on messages
  only (not on event rows). Retry button when `failed`.
- **Composer.** Disabled with `GROUP_REMOVED_NOTICE` when
  `!is_member`, and with `GROUP_RESTORED_NOTICE` when `restored_only`.
- **Info panel.** Name (editable by the owner: "Rename"). "Owner: <name>"
  with "(you)". `OWNER_CAN_READ_NOTICE`, always visible, not collapsible.
  Members list with owner marked. Owner-only: "Add member" (address or a
  contact picker from `contacts.list`) and "Remove" per member. "Group key
  changed here: <date>" from `key_stored_at_ms` and the epoch number.
  `GROUP_KEY_TRUST_NOTICE`. `GROUP_JOIN_BOUNDARY_NOTICE`. "Transcript
  check: <first 12 chars of digest>" with `TRANSCRIPT_CHECK_NOTICE`.
  `GROUP_DELIVERY_NOTICE` under the thread when any own message is
  `pending` or `failed`. "Hide this group" with `GROUP_HIDDEN_NOTICE` in the
  confirm dialog. "Sync now" button (`group.sync`).
- **Errors** show the server's message text as is (it already carries the
  exact notice, e.g. `GROUP_ADD_UNREACHABLE_MESSAGE`).

### 12.3 Playwright — new `crates/substrate/tests/e2e/tests/roym-groups.spec.ts`

One installation (the existing global setup). Cases 40–45; 33–39 stay
reserved for the transaction action panel row in the backlog.

| # | Case |
|---|---|
| 40 | Groups tab: create "Street Garden" → it is listed; the info panel says the person is the owner and shows `OWNER_CAN_READ_NOTICE` character for character |
| 41 | A group named `<img src=x onerror=alert(1)>` renders as literal text: no `img` element, no request to `x` (the pattern of case 4) |
| 42 | Add member with an address that never answers → the page shows `GROUP_ADD_UNREACHABLE_MESSAGE` verbatim; the member list is unchanged |
| 43 | Sending with no other member shows the "nowhere to deliver" text; nothing appears in the thread |
| 44 | The info panel never contains the word "verified"; it shows the key epoch and the transcript check |
| 45 | The Messages tab does not list the group; the Groups tab does not list a 1:1 conversation opened in case 10's way |

---

## §13 Backlog and documents (WO8)

### `docs/planning/deferred-backlog.md`

**Move to "Recently resolved":**
- §5 "An inbound group message is recorded as `unsupported-kind`…" —
  resolved by the group branch (§7.3).

**Restate (keep open, add C10 evidence):**
- §5 "No durable messaging host interface exists (group half)" — the
  product now uses it; list what the three-installation tests proved.
- §5 "A group message to a member removed while still pending settles
  `Failed` after the age limit" — the product shows it with D-C10-8's
  words, never as progress toward a current member (§12.2).
- §5 "`heads()` truncates a wide concurrent frontier to 8 parents" — C10
  added no reader of parent links (§1).
- §5 "A conversation peer's signing key is bound only by trust-on-first-use" —
  add that the group screen says so in its own words (D-C10-10).
- §5 "Conversations cannot continue after a substrate moves to a new
  machine" — add: a restored group is history only, shown with
  `GROUP_RESTORED_NOTICE` (D-C10-12).

**New rows:**

| Section | Row | Target |
|---|---|---|
| §5 | **A group message's delivery state is one aggregate** — `failed` can mean one member was offline past `max_pending_age_secs` even if it later got the message by sync. No per-recipient view. Trigger: "a person needs to know which member has a message". | TBD |
| §5 | **A member cannot leave a group; they can only hide it here** (D-C10-7). Trigger: "a member-initiated leave is designed (it needs an owner-side rekey)". | TBD |
| §5 | **Messages that arrived while a group was hidden are not shown after unhide** (D-C10-7). | TBD |
| §5 | **Adding a person you have never talked to needs them online** (Q11 narrows but does not remove it). | TBD |
| §5 | **Anybody who can reach your address can add you to a group underneath**; Roym hides it by the first-contact rule (D-C10-6) but the host has joined. Same shape and same fix as the admit-hook row (`D-06C-8`). | TBD (with the admit-hook row) |
| §13 | **Eight hand-kept copies of `conversation.wit`**; no gate checks they match (§15 item 17). | TBD |
| §1 | Only in branch B of §3: **Cross-node calls need the registry and relay on every call.** | TBD |

**"Open in-code markers":** none planned. If a `TODO` is added, add its row.

### Other documents

- `task.md`: add **Gap 10** after Gap 9 (§3 facts; "closed by C10" or
  "narrowed by `D-06C-14`"); the C10 row gets a Status (like C9's) and a
  link to this plan; the "C10 completes" row in "Documents this milestone
  edits" filled in; reference scenario step 19 corrected (§15 item 6).
- `status.md`: new "C10 — What shipped" section, with the evidence lines
  and every permitted WASM/native difference added (expected: none).
- `roym-integrated-experience-spec.md`: R4 marked **Passed** only after R3
  is marked passed (Q9); the service table's Conversation API column gains
  `group.*` and `transcript-digest`; R4 row 2's order wording corrected
  (§15 item 3); branch B's edit if taken.
- `CLAUDE.md`/`AGENTS.md` architecture bullet for Roym: add that
  `conversation` also serves groups under `group.*`. (Both files carry the
  same text; edit `AGENTS.md`, which `CLAUDE.md` includes.)

---

## §14 Work orders and order of execution

| WO | Scope | Needs | Done when |
|---|---|---|---|
| **WO0** | Gap 10 spike (§3) | — | Branch A or B chosen and written in `status.md` |
| **WO1** | Host: `group-info` (§4.1–4.2), pinned-key add (§4.3), clock hook (§5) | — | `cargo nextest run -p syneroym-conversation -p syneroym-app-host-native` green; both fixture builds rebuilt |
| **WO2** | `roym_core` (§6) | WO1 | `cargo nextest run -p syneroym-roym-core` green |
| **WO3** | `roym_conversation` (§7), router (§6.3), `wire_origin` (§8), transaction guard (§9) | WO2 | `mise run build:roym` then parity suite green |
| **WO4** | `roymctl roym group` (§10) | WO3 | parse tests green |
| **WO5** | Parity 201–212 + 60 rewrite (§11.2) | WO3 | `cargo nextest run -p syneroym-roym-web --test dual_build_parity` green, both builds identical |
| **WO6** | e2e (§11.3) | WO3, WO0 | both binaries green |
| **WO7** | Hub + vitest + Playwright (§12) | WO3 | `mise run test:roym-ui` and `mise run test:e2e` green |
| **WO8** | Backlog and docs (§13); import cleanup pass; `cargo dupes` recipe on changed files | all | `mise run verify` (no `--skip`) green |

WO0 and WO1 can run in parallel. WO4, WO5, WO6, WO7 can run in parallel
after WO3. Remember after every `roym_*` or `roym_core` change: rebuild the
WASM components before trusting a parity result.

---

## §15 Things in the docs that look stale or ambiguous

1. **R4 row 1's "no coordinator reachable" does not match the substrate.**
   Every remote call re-resolves through the registry and dials through the
   relay (§3.1). Nothing in the spec or task.md names this. → Q1, Gap 10.
2. **`conversation.wit`'s `conversation-kind` comment is stale.** It says
   `group` "is reserved for the group slice and is never returned by this
   version". B5 shipped groups; `conversations()` returns `group` today
   (`crates/conversation/src/lib.rs:376-406`). Fixed in §4.1.
3. **The spec's order key is not the code's.** R4 row 2 and the Messaging
   section say `(sender_timestamp, sender_did)`. The code, the WIT, and
   ADR-0013's implementation use `(sender-timestamp, author, id)`, where
   `author` is a routing **service id**, not a DID (Gap 5, `D-B4-5`). Same
   rule in spirit; the spec wording should be corrected.
4. **The spec's Conversation API column has no group verbs.** It lists
   `open`, `send`, `history`, … only. C10 adds `group.*`.
5. **The rekey schedule has no value in the spec.** The host default is
   7 days (`conversation_group_rekey_secs: 604_800`,
   `crates/conversation/src/store.rs:74`). The product shows when the key
   last changed; it does not show the schedule. Say it in the spec if a
   number is wanted.
6. **`task.md` reference scenario step 19 contradicts itself.** "X, Y, and Z
   form a private group … The owner removes Z." If Z (the SynOrg owner) is
   also the group owner, the host refuses that removal ("the owner is
   always a member", `group.rs:243-247`). The step never says who owns the
   group. → Edit to "X creates the group and adds Y and Z … X removes Z".
7. **R4 rows 2 and 3 together.** A joiner cannot read messages before its
   join (row 3), so "every member's transcript is byte-identical" (row 2)
   can only hold among members who joined before those messages. The tests
   check row 2 with stable membership and check row 3 separately. The spec
   should say this.
8. **Carried-forward limit 2 in `task.md`** says a removed member's pending
   message settles `failed` "only after the full age window". That window
   is 30 days by default (`max_pending_age_secs: 2_592_000`). The product
   words (D-C10-8) cover it; the doc should state the number.
9. **A new finding next to limit 2:** a group message can show `failed`
   because one member was offline past the age window, even if that member
   later received it by sync (the aggregate rule, `outbox.rs:236-270`). →
   Q6, backlog row.
10. **Adding a member needs that member online.** `add-member` fetches the
    member's prekey bundle every time (`group.rs:207`). The spec says only
    that the **owner** must be online for joins and leaves (O1 table). →
    Q11 narrows it; backlog row for the rest.
11. **`messages.ts` is 846 lines.** The 800-line rule is enforced only for
    `.rs` files (`xtask/src/file_lengths.rs:388`). C10 does not grow it
    (D-C10-13).
12. **Found in passing — a Hub bug, not C10 scope.** The Messages search
    reads `res.hits[].snippet` (`crates/roym_web/ui/src/screens/messages.ts:156-172`),
    but `conversation.search` returns `{ "matches": [MessageRow] }`
    (`crates/roym_conversation/src/app/messages.rs:463-469`). No test
    covers it. The search box would throw on any query.
13. **`roym_core::conversation::CONVERSATION_SCHEMA_VERSION` (= 1) has no
    reader.** The live version is `roym_conversation::app::SCHEMA_VERSION`
    (= 2). Two constants that disagree. Deleted in §6.1.
14. **C9 is partial, and the spec says each release must pass before the
    next begins.** → Q9.
15. **Scheduled rekeys are not DAG events.** The spec says membership
    changes are DAG events and says nothing about rekeys. So members see
    "key changed here" at their own receipt time, and different members
    see different times. That is correct, but the UI must not present it as
    a shared event.
16. **The spec's journey has no group steps.** Phases 1–3 cover only the
    transaction flow. The demo script (§16) is the only step-by-step account
    of group chat.
17. **Eight copies of `conversation.wit` are kept by hand** (§4.1). Nothing
    checks that they match.
18. **"Relay / coordinator node"** in the spec's encryption table is one
    row; the code has three separate things (community registry, iroh
    relay, `syneroym-coordinator`). R4 row 1 says "coordinator". This plan
    reads it as "the node that hosts the registry and the relay", which is
    what the test harness can take down.

---

## §16 From-scratch demo script

This section is independent of the plan above. It describes, in a few
bullet points per use case, what a person sees. Three people each run their
own Syneroym installation with Roym deployed: **Zara** (the group owner),
**Xavi**, and **Yuki**. A fourth, **Wen**, joins later. Every step works the
same from the Hub's Groups tab or from `roymctl roym group …`.

### 16.1 Setup

- Each person installs Syneroym, deploys Roym, logs in to the Hub, and
  enrols signing (as in earlier releases).
- Each person fills in their profile. The profile shows their conversation
  address.
- Zara adds Xavi and Yuki as contacts, and they add Zara. The Groups tab is
  empty and says so.

### 16.2 Zara creates a group and names it

- Zara opens **Groups**, types "Street Garden", and presses **New group**.
- The group appears in her list. The thread shows one event: "Zara created
  the group".
- The info panel says **Owner: Zara (you)**, and under it, always visible:
  "The owner of this group makes and shares the group's key, so the owner
  can read every message sent while they own it. Adding or removing a
  member is shown to everyone in the group."
- Zara tries to send a message. The Hub says there is nobody to deliver to
  yet.

### 16.3 Zara adds Xavi and Yuki

- Zara presses **Add member**, picks Xavi from her contacts, then Yuki.
- Her thread shows "Zara added Xavi" and "Zara added Yuki". The key number
  in the info panel goes up with each change.
- A few seconds later, Xavi's and Yuki's Groups tabs show "Street Garden"
  with Zara as the owner, the same owner notice, and the same member list.
- Xavi's thread starts at "Zara added Xavi". It says: "You can read
  messages sent after you joined."

### 16.4 Everyone talks, and everyone sees the same order

- All three send a message at about the same moment. Xavi's computer clock
  is a minute and a half fast, and Yuki's is a minute and a half slow.
- Each message first shows **Not yet delivered to every member**, then
  **Delivered to every member**. It never says "read" or "verified".
- The three threads list the messages in exactly the same order. (Times
  shown may look odd for the skewed clocks. The order never differs.)
- Each info panel shows a **Transcript check** code. All three codes are
  the same. The panel explains that equal codes mean the same messages in
  the same order.

### 16.5 Yuki is away and catches up from Xavi

- Yuki shuts down her computer.
- Xavi and Zara each send two messages. Then Zara shuts down too.
- Yuki starts again, logs in again (sessions end on restart), and opens the
  group. She presses **Sync now**.
- All four messages appear, including Zara's, even though Zara is offline.
  Xavi's installation passed them on. Yuki's transcript check now matches
  Xavi's.

### 16.6 The group keeps talking when the coordinator is down

(Shown this way only if the §3 spike keeps the fix; otherwise this use
case is left out of the demo.)

- The shared registry and relay go offline.
- Xavi sends a message. Yuki and Zara still receive it, and the order is
  still the same for all three.

### 16.7 Wen joins and cannot read the past

- Zara adds Wen. Everyone sees "Zara added Wen".
- Wen's thread starts at that event. The earlier messages are not there,
  and the panel says why.
- Zara's current group name, "Street Garden", appears for Wen, because
  Zara's installation sends it again when someone joins.
- Wen's next message reaches everyone.

### 16.8 Zara removes Yuki

- Zara presses **Remove** next to Yuki. Everyone, Yuki included, sees
  "Zara removed Yuki".
- Zara sends "Planning the next meeting". Xavi and Wen see it. Yuki does
  not.
- Yuki's group shows: "You were removed from this group. You can still read
  what you received before. You cannot read or send new messages." Her
  message box is turned off.

### 16.9 The key changes on a schedule

- A week later (or after the short interval set for a demo), nobody has
  joined or left.
- Each member's info panel shows a higher key number and a new "Group key
  changed here" date. The member list is the same.
- The next message is read by every current member as before. Yuki's key
  number does not change.

### 16.10 Renaming the group

- Zara presses **Rename** and types "Street Garden Crew".
- Everyone sees "Zara named the group “Street Garden Crew”", and the new
  name in their list.
- A name full of HTML shows up as plain text, never as a picture or a link.

### 16.11 Deleting a message in a group

- Xavi deletes one of his own messages. His copy shows "(message deleted)".
- The Hub tells him that a request went to the other members, that their
  clients decide whether to honour it, and that every member already holds
  the key it was sent under.

### 16.12 A stranger adds you to a group

- Wen, who is not Xavi's contact, creates "Cheap Deals" and adds Xavi. Xavi
  has set his first-contact limit to zero.
- Nothing appears in Xavi's Groups list. Under **Hidden groups (1)** he
  sees "Cheap Deals", marked as not shown because of his first-contact
  limit.
- If Xavi presses **Unhide**, new messages from then on appear. The ones
  sent while it was hidden do not.
- Xavi can also **Hide** any group he no longer wants to see. The Hub tells
  him that he stays a member underneath until the owner removes him.

### 16.13 The same flow without a browser

- Zara runs `roymctl roym group create --name "Street Garden"`, then
  `roymctl roym group add --group <id> --person-did <Xavi's DID>`.
- Xavi runs `roymctl roym group list`, `… group history --group <id>`, and
  `… group send --group <id> --body "hello"`.
- Each person runs `roymctl roym group check --group <id>` and gets the
  same transcript check code the Hub shows.

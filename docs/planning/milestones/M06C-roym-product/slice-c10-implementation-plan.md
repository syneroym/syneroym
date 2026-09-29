# Slice C10 — Private group chat in the product (R4)

**Milestone:** [task.md](task.md) (row C10, `D-06C-5`, `D-06C-8`, the "Carried forward from M06B" table)
**Spec:** [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) — R4 (all five rows), D5, D10, "What is encrypted, and who can see what", Messaging, O1.
**ADR:** [ADR-0013](../../../decisions/0013-p2p-messaging-architecture.md) §5 (ordering) and Amendment 1 (owner-distributed key).
**Status:** Plan only. Written 2026-09-29 against `feat/m06c-slice-c9-trust` at `adecc34f`; re-checked against `7fa7efee` after C9 finished (WO5–WO8), and anchors updated.
**Depends on:** C5 (complete). C9 (complete; R3 passed 2026-09-29).

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
| Q2 | The product must show who owns a group (spec: *"The group's UI should say who the owner is"*), and a test must see that a scheduled rekey changed the key. The WIT exposes neither the owner nor the epoch. | **Add one additive host function `group-info`** to `syneroym:conversation` (same pattern as C8's `create`). Deriving the owner from `membership-history`'s first event is unreliable: a new member may not have synced the genesis entry when its first message arrives. (Q13 adds a second function, `get-message`.) | §4 |
| Q3 | A group needs a name every member sees. The host has no group name. | **The owner sends the name as a message of a reserved content type** (`application/vnd.roym.group-profile+json`) inside the group. It is honoured only when the author is the owner. The newest one by the ADR-0013 sort key wins. The owner sends it again after every `add-member`, because a new member cannot read anything from before it joined. | §2 D-C10-4, §7.4 |
| Q4 | The host joins you to a group as soon as its owner sends you a key. **Anybody who can reach your address can add you to a group.** Nothing asks you. | **Treat being added as a first contact from the owner.** The first time Roym sees a group, it calls `contacts.admit-first-contact` for the owner (same rule and same budget as 1:1). A refused group is stored as `refused` and not shown. Add `group.hide` / `group.unhide` so a person can hide a group they do not want. You stay a member underneath until the owner removes you; the UI says so. | §2 D-C10-6, §7.3, §7.5 |
| Q5 | Membership changes must be visible (R4 row 4), and the transcript must be identical everywhere (row 2). The host never sends membership changes to `on-message`. | **Copy every membership event into Roym's own message store as a row** of a reserved content type (`application/vnd.roym.membership-event+json`), with the entry id as row id and the owner as author. Then history, export, import, and the one sort rule all work with no second list. | §2 D-C10-5, §7.2 |
| Q6 | The host reports **one** delivery state for a group message: `pending` until every recipient settles, `failed` if any recipient failed. A member who was offline longer than `max_pending_age_secs` (30 days) makes the message `failed`, **even though that member may have received it from another member by sync**. | **Do not add a per-recipient host verb in C10.** Use honest group words: *"Not yet delivered to every member"*, *"Delivered to every member"*, *"Not delivered to every member"*, plus one notice that members also pass messages to each other. Backlog row for a per-recipient view. | §2 D-C10-8, §12 |
| Q7 | R4 row 2 needs posts from **deliberately skewed clocks**. All test nodes run in one process with one system clock. | **Add one hook to the existing `test-support` feature on `syneroym-conversation`** (C9 created it, with `drop_next_ack` and `override_next_send`): a per-service clock offset used only where the sender signs a group entry. | §5 |
| Q8 | Transaction cards are 1:1 by design. What happens to a card in a group? | **`conversation.send` refuses a card into a group. `transaction.sync` refuses a group conversation.** An incoming card in a group is stored as a message and the Hub shows a neutral block ("cards are not used in groups"). | §7.6, §9 |
| Q9 | The spec says *"Each release must pass its acceptance tests before the next begins."* | **Resolved.** R3 was marked passed on 2026-09-29, after C9's WO5–WO8 landed. Nothing gates C10 now. | — |
| Q10 | How does a person (or a test) check "every member has the same transcript"? | **Add `conversation.transcript-digest`**: a hash over the ordered list of `(id, author, sender_timestamp_ms, content_type)` of the rows this installation holds. The Hub shows it in the group's info panel as a short "transcript check" code. Members compare it by eye. It is also exactly what the acceptance test compares. | §2 D-C10-9, §6.2 |
| Q11 | `add-member` always fetches the new member's prekey bundle over the network (`crates/conversation/src/group.rs:207`), so **the owner can add only a person who is online at that moment**. | **When this node already holds a 1:1 session with the new member, use the key pinned in that session and skip the fetch.** Small host change with a host unit test. A person you have never talked to must still be online when you add them; the UI says so. | §4.3 |
| Q12 | **A removed member can miss its own removal forever.** The owner pushes the removal entry to the removed member once and ignores the result (`crates/conversation/src/outbox.rs:99-123`). `group-sync` refuses a removed member, because `member_sig_key` reads only rows with `removed_epoch IS NULL` (`crates/conversation/src/store/dag_store.rs:32-45`, then `PermissionDenied` at `transport/group_sync.rs:363`). A member offline at removal time keeps `is_member = true` and a working-looking composer. | **Small host change: `group-sync` answers a removed member with the membership entries up to and including its removal, and the message entries that were both sealed under an earlier epoch *and* signed no later than the removal entry's own timestamp.** So a member that was offline before its removal can still fill in the messages it was entitled to read. The timestamp condition matters: a member that has not applied the removal yet still seals new messages under the old epoch, and without it every member would serve those post-removal messages to the removed member on every sync. | §4.4 |
| Q13 | Roym's own row for a message it just sent takes `sender_timestamp` and `author` from the host outbox. If the message has already left the outbox, it falls back to the local clock in whole seconds and possibly to author `"self"` (`crates/roym_conversation/src/app/messages.rs:162-179`). Under a skewed clock that row differs from every receiver's row, so the R4 row 2 check fails sometimes. | **Add a second additive host function, `get-message`** (read one message by id). `send` reads its own row through it, with no fallback. It also lets `group.unhide` fill in messages that arrived while a group was hidden. | §4.1, §7.6, D-C10-7 |

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
| D-C10-2 | **Two additive host functions, `group-info` (Q2) and `get-message` (Q13), and nothing else on the WIT.** The stale `conversation-kind` comment is corrected at the same time. Two host behaviour changes ride with them: the pinned-key add (Q11, §4.3) and removal catch-up (Q12, §4.4). | Additive functions keep old components deployable. The owner, the epoch, and a sent message's signed timestamp are facts only the host holds. |
| D-C10-3 | **A Roym `ConversationRow` gains `kind` (required) and `group: Option<GroupMeta>`.** For a group, `peer_address` holds the **owner's** address and `peer_person_did` the owner's person DID when a contact carries it. `SCHEMA_VERSION` of the `conversation` service goes 2 → 3. No migration: pre-release, and an older bundle fails at the existing version gate (`backup.rs` `import`). | One collection for both kinds keeps `conversation.list`, export, and import unchanged in shape. The owner is the one party a group's trust hangs on, and the one the first-contact decision is made against (D-C10-6). |
| D-C10-4 | **Group name = the newest owner-authored `group-profile` message by sort key** (Q3). Body is exactly `{"name": "<text>", "version": 1}`. Name is 1–80 characters after trimming, with no control characters. It is always displayed as text. A profile message from a non-owner is refused (`refused_messages` reason `not-owner`) and changes nothing. | Every member that can read the message computes the same name. The owner is the only writer, so there is no conflict rule to design. Text-only display follows the card rule (`D-06C-3`). |
| D-C10-5 | **Membership events are rows in Roym's own copy** (Q5): id = DAG entry id, author = owner, `sender_timestamp_ms` = the event's own timestamp, content type `application/vnd.roym.membership-event+json`, body `{"action","subject","epoch"}` (canonical key order). They are written by `sync_membership_rows` (§7.2), which runs on every group message, when `conversation.list` or `group.info` adopts a group Roym has not seen before, on `conversation.history` of a group, on `group.info`, and after every group verb. `conversation.list` does **not** run it for groups Roym already holds (that would be one host call per group on every list); a group's membership rows catch up when it is opened or when a message arrives. These rows cannot be deleted and do not count in `message_count`. | Keeps one list and one sort rule (`roym_core::conversation::sort_key`). Export and restore carry them for free. |
| D-C10-6 | **Being added to a group is a first contact from its owner** (Q4). The first time Roym sees a group it has no row for — either a message arrives in it, or `conversation.list` / `group.info` finds it in the host's `conversations()` (`adopt_new_groups`, §7.2) — it decides once: owner = self → `shown`; otherwise one `contacts.admit-first-contact` call with `sender_address = owner` → `allow` = `shown`, `blocked` = `refused{blocked}`, anything else = `refused{rate-limited}`. The decision is stored. After that, each message's **author** is checked with `block.check`, exactly like 1:1 (`D-06C-8`). | Same rule and same budget as 1:1, so a stranger cannot bypass the first-contact limit by using a group. Storing the decision stops a refused group from spending the budget again. Adopting from the host list means a new member sees the group as soon as it is added, not only after the first message (review finding 12). |
| D-C10-7 | **`group.hide` sets `admission = hidden`. New messages in a hidden group are refused into `refused_messages` (reason `group-hidden`). `group.unhide` sets it back to `shown` and then fills in the messages refused while hidden or unadmitted**, reading each one from the host with `get-message` (Q13). Messages refused because their **author** is blocked are never filled in. | The host keeps storing them underneath (Gap 4). With `get-message` the fill-in costs one host read per refused row, so there is no reason to lose them. |
| D-C10-8 | **Group delivery words** (Q6): `pending` → "Not yet delivered to every member"; `delivered` → "Delivered to every member"; `failed` → "Not delivered to every member" + Retry. The words never name a member, never say "trying to reach", and never say "read" or "verified". | The host's group state is an aggregate. The carried-forward limit (a removed member's pending item settles `failed` only after the age window) must not be shown as progress toward a current member. These words are true in every case. |
| D-C10-9 | **Transcript check** (Q10): `transcript_digest(rows)` = `content_digest("roym-transcript:", [ {id, author, sender_timestamp_ms, content_type} ... ])` over the rows sorted by `sort_key`. Bodies are left out: a group row id is the DAG entry id, which is already a hash over the ciphertext, and leaving the body out means a local delete does not change the check. Rows refused locally (a blocked author) are not in the list, so the check differs for a member who blocked someone; the UI says so. | One number both a person and a test can compare. Reuses `syneroym_signed_record::content_digest`, already a `roym_core` dependency. |
| D-C10-10 | **Member signing keys are trust-on-first-use, and the UI says so in different words than a signed record.** Group messages are shown with no "verified" word. The info panel carries `GROUP_KEY_TRUST_NOTICE`. | Carried-forward limit 3 in `task.md`: two strengths, two words. |
| D-C10-11 | **Cards are 1:1 only** (Q8). `conversation.send` refuses `application/vnd.roym.card+json`, the group-profile type, and the membership-event type when the target is a group, and refuses the membership-event type everywhere. `transaction.sync` refuses a group conversation. | The transaction single-writer model is between two parties. Refusing at the one send choke point covers every producer. |
| D-C10-12 | **A restored group is history only.** After an import onto a clean node, the host does not know the group. `group.info` then returns `restored_only: true` and the Hub shows `GROUP_RESTORED_NOTICE`. | Group keys and the DAG live in the host store, which no backup carries (backlog §5, "Conversations cannot continue after a substrate moves to a new machine"). A **full-state move** of a running node (developer guide, "Moving a Substrate to a New Machine") copies `conversation.db` with everything else, so groups keep working there and `restored_only` stays false. Only a `roymctl roym backup` restore gives history only. |
| D-C10-13 | **The Hub gets a separate Groups tab** (`crates/roym_web/ui/src/screens/groups.ts`). The Messages tab lists and searches only direct conversations. The Groups tab has no search box (R4 excludes message search). | `messages.ts` is already 842 lines. A group thread also needs a roster panel the 1:1 thread does not. |
| D-C10-14 | **(Q1) Gap 10 fix, only if the spike keeps it inside `crates/core` / `crates/router` and passes the regression gate in §3.2.** Otherwise the acceptance row and every other "no coordinator" claim are narrowed by a spec edit (§3.3). | See §3. |
| D-C10-15 | **Roym checks membership before it sends into a group.** `conversation.send` and the deletion request in `delete-message` call `group_info` first. When `is_member` is false, they refuse with `GROUP_REMOVED_NOTICE` and send nothing. | The host does not return `PermissionDenied` for a removed member. It returns `Internal("no key for the current epoch")` (`group.rs:347-356`), which says nothing a person can use (review finding 1). |
| D-C10-16 | **A repeated group name is shown once.** The rows stay (the transcript must stay identical), but the Hub renders a profile row only when its name differs from the name before it in the thread. | The owner re-sends the name after each add (Q3), so without this every join adds a "named the group" line with no change (review finding 14). |
| D-C10-17 | **Group chat across installations is proven on the WASM build; both builds are proven by the parity suite.** Same rule and same reason as `D-C9-13`. A natively linked Roym service still has no instance certificate (backlog §3, "The natively linked Roym services carry no instance certificate…"), and the conversation delivery worker refuses to send without one (`check_outbound_identity`, `crates/conversation/src/transport.rs`). So a native `conversation` cannot push or sync a group entry to another installation. `status.md` and the R4 "Passed" note must both say "WASM build across installations; both builds in parity". | Exit criterion 1 asks for both builds. Saying it out loud stops that criterion being read as met across installations for native. Fixing it is substrate work outside R4. |

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
  (`crates/conversation/src/transport.rs:521-543`), so pushes back off and
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
5. **Regression gate.** `RegistryClient` serves every registry lookup in
   the node, not only conversation calls. C9's directory resolution and
   the visibility rules go through it too. So Fix A1 must follow three
   rules, each with a unit test in `crates/core/src/dht_registry/`:
   - The cache is read **only** after a transport failure. While the
     registry answers, every lookup goes to it, as today.
   - A clean "not found" (or "not visible to this caller") answer is never
     replaced by a cached value, and it removes the cached entry.
   - A successful lookup always overwrites the cached entry, so a
     republished record (higher `generation`) wins at once.

   Then run, and require green, the existing cross-node suites that
   resolve through the registry:
   `cargo nextest run -p syneroym-substrate --test roym_app_e2e`
   (it holds `an_unaffiliated_caller_resolves_directorys_public_record_but_not_profiles`),
   `--test roym_directory_e2e`, `--test conversation_e2e`,
   `--test group_conversation_e2e`, `--test proxy_outbox_e2e`,
   `--test roym_trust_e2e` (C9's three-installation test, which resolves
   the directory, the provider, and the consumer through the registry), and
   `--test conversation_cross_node_e2e`.
6. **Stop gate.** If A1 + A2 pass step 2 and step 5 within the time box and
   touch only `crates/core/src/dht_registry/client.rs`,
   `crates/router/src/net_iroh.rs`, and `crates/router/src/proxy/*`, keep
   them and go to §11.3 test 5. Otherwise revert and take **branch B**.

### 3.3 Branch B (only if the stop gate fails)

- Spec edit (R4 row 1 acceptance test), recorded as a new milestone
  decision `D-06C-14` in `task.md`: *"With the registry and relay
  reachable but carrying no message content, members exchange and order
  messages member to member; no node other than members stores or orders
  a group message."* State the reason: the substrate needs the registry to
  find a peer and the relay to dial it.
- The same narrowing, in the same commit, everywhere else the claim is
  made (review finding 8):
  - `task.md` Goal paragraph (`:37-38`, *"a private group of at least
    three members holds a conversation with no server in the path"*) — add
    "other than the registry and relay it uses to find and reach members,
    which never hold a message".
  - `task.md` exit criterion 9 (*"no server in the path"*) — same wording.
  - `task.md` reference scenario step 19 (*"with no coordinator
    reachable"*) — replace with "with the registry and relay reachable
    and holding no message".
  - `task.md` slice table, row C10 (*"no server in the path"*) — same.
  - The spec's R4 row 1 goal column (*"no central chat server"*) stays: it
    is still true. Only the acceptance column changes.
  - This plan's §1 item 1 and §16.6 (drop §16.6 from the demo).
- Backlog row §1 "Cross-node calls need the registry and relay on every
  call", trigger "a group must keep talking through a registry outage",
  with the spike's findings.
- §11.3 test 5 becomes "no member-to-member message passes through any
  non-member's storage": assert the coordinator node's own conversation
  store has no row for the group (it has no Roym conversation service
  in that group at all) and that `members` never includes it.

---

## §4 Host: `group-info`, `get-message`, the pinned-key add, and removal catch-up (WO1)

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
       /// `epoch` once this service has learned it was removed: it holds
       /// the removal entry but was not given the new key. Until it learns
       /// (it may have been offline), `is-member` stays true.
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

   /// One message this service stored, sent or received, in any state.
   /// `not-found` for an unknown id and for a system message (a group key).
   get-message: func(message: message-id) -> result<message, conversation-error>;
   ```

### 4.2 Call sites (every one must change, or the workspace does not build)

Each row below is for `group-info`. **`get-message` follows the same rows**
with these differences: the rpc trait method is
`async fn get_message(&self, service_id: &str, message: &str) -> Result<ConversationMessage, ConversationError>`
(no new rpc type); the host impl is
`store.get_message(message)?.filter(|m| !m.system).map(StoredMessage::into_wire).ok_or(NotFound)`
in `crates/conversation/src/lib.rs`; the WASM and native mappings reuse the
existing message converters (`conversation_wire`'s message mapper and
`convert::conversation::message_out`, or whatever the `outbox` path uses
today); the fixture op is `get-message`; the parity check compares a sent
message read back on both builds.

| File | Change |
|---|---|
| `crates/rpc/src/conversation.rs` | New `pub struct ConversationGroupInfo { pub owner: String, pub is_owner: bool, pub is_member: bool, pub members: Vec<String>, pub epoch: u64, pub key_epoch: u64, pub key_stored_at: i64 }` (derive `Debug, Clone, PartialEq, Eq`). New trait method on `ConversationHost` after `sync_now` (`:167`): `async fn group_info(&self, service_id: &str, conversation: &str) -> Result<ConversationGroupInfo, ConversationError>;`. Re-export from `crates/rpc/src/lib.rs` beside the other `Conversation*` types. |
| `crates/conversation/src/lib.rs` | `impl ConversationHost for ConversationService` (`:409`): `async fn group_info(..) { self.group_info_impl(service_id, conversation).await }`. |
| `crates/conversation/src/group.rs` | New `pub(crate) async fn group_info_impl` (pseudo-code below). |
| `crates/control_plane/src/synsvc_native/conversation.rs` | `NeverConstructed` (`:19`): add `group_info` → `unreachable!(..)` like its siblings. |
| `crates/sandbox_wasm/src/host_capabilities.rs` | `NeverConstructedConversationHost` (`:124`): same. |
| `crates/sandbox_wasm/src/host_capabilities/capabilities_messaging.rs` | `impl wit_conversation::Host for HostState` (`:102`): `async fn group_info(&mut self, conversation: String) -> Result<wit_conversation::GroupInfo, wit_conversation::ConversationError>` — upgrade `self.conversation` exactly like `membership_history` (`:228`), call `conv.group_info(&self.component_id, &conversation)`, map with a new `conversation_wire::map_group_info` beside `map_membership_event` (`:89`). Read-only is **not** checked (it is a read). |
| `crates/app_host/src/types.rs` | Add `GroupInfo` to the `conversation` re-export list (`:20-24`). |
| `crates/app_host/src/lib.rs` | Add `GroupInfo` to the `types::conversation::{..}` import (`:25-28`). `trait AppConversation` (`:293`): `fn group_info(&self, conversation: String) -> impl Future<Output = Result<GroupInfo, ConversationError>> + Send;` |
| `crates/app_host/src/guest.rs` | `impl AppConversation for GuestHost` (`:228`): `async fn group_info(&self, conversation: String) -> Result<GroupInfo, ConversationError> { conv::group_info(&conversation) }`. |
| `crates/app_host_native/src/host.rs` | `impl AppConversation for NativeAppHost` (`:382`): lock state, `HostConversation::group_info(&mut *state, conversation).await.map(convert::group_info_out).map_err(convert::conversation_error_out)`. |
| `crates/app_host_native/src/convert.rs` | Add `GroupInfo as GuestGroupInfo` to the guest import list (`:8-13`) and `GroupInfo as HostGroupInfo` to the host import list (`:37-42`). |
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
owner, `transport.rs:359`). If the field names differ, use the session's
pinned peer signing key. Host unit test:
`add_member_uses_the_pinned_session_key_when_one_exists` (seed a session,
give the service a proxy that fails every call, assert `add-member`
succeeds and the membership entry's `subject_sig_key` equals the pinned
key).

### 4.4 A removed member learns its removal by sync (Q12)

Today a removed member learns its removal only from the one relay push
(`crates/conversation/src/outbox.rs:99-123`, result ignored). After that,
every peer refuses its `group-sync` (`transport/group_sync.rs:161-162` →
`pinned_member_sig_key` → `PermissionDenied`, because `member_sig_key`
reads only `removed_epoch IS NULL` rows).

Change `group_sync_impl` (`transport/group_sync.rs:133-184`):

```rust
let (sender_sig_key, removed_at_epoch) =
    match pinned_member_sig_key(&store, &conv.id, &req.from.address, req.from.sig_key) {
        Ok(k) => (k, None),
        Err(ConversationError::PermissionDenied) => {
            // A removed member may still fetch what it was entitled to
            // read: messages sealed under an earlier epoch *and* signed no
            // later than its removal, plus the membership entries up to and
            // including the one that removed it. The epoch alone is not
            // enough: a member that has not applied the removal yet still
            // seals new messages under the old epoch.
            let (k, removed_epoch) = store
                .removed_member_sig_key(&conv.id, &req.from.address)
                .map_err(internal)?
                .ok_or(ConversationError::PermissionDenied)?;
            (k, Some(removed_epoch))
        }
        Err(e) => return Err(e),
    };
// ... verify the peer assertion under `sender_sig_key`, unchanged ...
let entries = match removed_at_epoch {
    None => store.entries_after_seq(&req.group, req.after_seq, limit + 1),
    Some(removed_epoch) => {
        // The removal entry's own signed timestamp: identical on every
        // member, so every member serves the same set.
        let Some(removed_at_ms) = store
            .removal_entry_timestamp(&conv.id, &req.from.address, removed_epoch)
            .map_err(internal)?
        else {
            // The removal is recorded in `group_members` but its DAG entry
            // is not here yet: serve nothing, let the requester try another
            // member or try later.
            return Ok(empty_sync_response(req.after_seq));
        };
        store.entries_after_seq_for_removed(
            &req.group, req.after_seq, removed_epoch, removed_at_ms, limit + 1)
    }
}.map_err(internal)?;
```

**Why messages from before the removal are served at all.** A member that
was offline before its removal can have missed messages sent **before** the
removal. It was entitled to read those, and the spec allows it. A
membership-only rule would be safe too (it holds back more, not less), but
it would leave a gap in that member's history for no protective reason.

**Why the timestamp condition is needed (the epoch is not enough).** The
epoch shows which key sealed a message, not when it was sent. When the
owner removes Y, a member X that has not applied the removal entry yet
still has the old `current_epoch`, so a message X sends *after* the
removal is sealed under a key Y holds. Filtering on `epoch < removed_epoch`
alone would let every member that stored X's message serve it to Y on every
sync, with no time limit. So a message entry is served only if its
`sender_timestamp_ms` is no later than the removal entry's own
`sender_timestamp_ms` — the same idea the host already applies to a
removed **author** (`removed_epoch_created_at`, `group/entry.rs`).

**What this does and does not promise.** It trusts the sender's signed
timestamp. An honest lagging member signs its real time, which is after
the removal, so its message is held back. Two cases remain:
- **Clock difference — a small leftover of this rule.** If X's clock runs
  behind the owner's, a message X sent just after the removal can carry a
  timestamp just before it, and is then served. This is partly new in
  C10: before, such a message reached Y only through X's one direct push
  (the next case), which fails if Y is offline; with §4.4, Y can also get
  it by sync later. It still needs a lagging sender, and the exposure is
  bounded by the clock difference between X and the owner (at most
  `max_clock_skew_secs`), so the risk stays small.
- **The existing direct-push race (from B5).** A lagging X still lists Y
  as a member, so X's own outbox pushes its new message to Y directly,
  once, until X applies the removal. That is unchanged by C10, and a
  dishonest member could send to Y directly in any case.

Both go into the removed-member backlog row (§13).

New store functions in `crates/conversation/src/store/dag_store.rs`:

- `removed_member_sig_key(conv, address) -> Result<Option<([u8; 32], u64)>>`:
  the row with `removed_epoch IS NOT NULL`, returning its real key (never a
  zero placeholder: return `None` for `zeroblob`) and `removed_epoch`.
- `removal_entry_timestamp(conv, address, removed_epoch) -> Result<Option<i64>>`:
  the `sender_timestamp` column (milliseconds) of the membership entry at `removed_epoch`
  whose payload action is `remove` and subject is `address`. Read it from
  the stored DAG entry (the same row `membership_history` reads), not from
  `group_epochs.created_at`, which is local time and differs per member.
- `entries_after_seq_for_removed(conv, after_seq, removed_epoch, removed_at_ms, limit)`:
  the same query as `entries_after_seq` plus
  ```sql
  AND (
    (kind = 'membership' AND epoch <= ?removed_epoch)
    OR (kind = 'message' AND epoch < ?removed_epoch
                         AND sender_timestamp <= ?removed_at_ms)
  )
  ```
  Column names are the real ones in `dag_entries`
  (`crates/conversation/src/store/schema.rs:125-140`): the timestamp column
  is `sender_timestamp` (milliseconds; the Rust field is
  `sender_timestamp_ms`), and `kind` is stored as the text `'message'` /
  `'membership'` (`dag_store.rs:254-255`).
  The only membership entry at `removed_epoch` is the removal itself,
  because each membership change opens its own epoch. Membership entries
  keep the epoch-only rule: they are unencrypted and owner-signed, and the
  ones up to the removal are the history Y belonged to. Filtering in SQL,
  not after the page is read, keeps the requester's cursor moving: its
  `highest_applied_seq` then steps over only the seqs it was sent
  (`group_sync.rs:295-328`).
- `empty_sync_response(after_seq)`: a `GroupSyncResponse` with no entries,
  `next_seq = after_seq`, `has_more = false` (a small helper in
  `group_sync.rs`).

`group_push_impl` is **not** changed: a removed member cannot push.

On the removed member's side nothing changes: `run_group_sync` still calls
every member in its own (stale) member list until the removal applies,
then `current_members` no longer lists it and it keeps calling the others
at the periodic rate, which answer with nothing new.

Host unit tests in `crates/conversation/src/transport/tests.rs`:
`a_removed_member_can_sync_its_own_removal` (owner removes B; B's sync
request gets the removal entry and no message entry from the removal
epoch or later, even with such a message sitting between them in `seq`
order);
`a_removed_member_catches_up_on_messages_from_before_its_removal` (A sends
M while B is offline, then the owner removes B; B's sync answer contains M
and the removal, and B can read M);
`a_removed_member_gets_no_old_epoch_message_signed_after_its_removal`
(the owner removes B at time T; member X has **not** applied the removal
and sends N under the old epoch, signed at T + 1 s — build it with
`build_message_entry` at the old epoch and insert it into the serving
store directly, the same way the host would store X's push; B's sync
answer contains the removal entry and **not** N, while an entry signed at
T − 1 s in the same old epoch **is** served);
`a_removed_member_is_served_nothing_before_the_removal_entry_arrives`
(`group_members` records the removal but the DAG entry is absent → empty
answer, `next_seq` unchanged);
`a_removed_member_gets_no_membership_entry_after_its_removal` (owner
removes B then adds C; B's sync answer stops at B's removal);
`a_stranger_is_still_refused` (an address never in the group gets
`PermissionDenied`).

---

## §5 Host: the clock-offset test hook (WO1, Q7)

C9 already built the feature and the file. Nothing changes in either
`Cargo.toml`:

- `crates/conversation/Cargo.toml` has `[features] test-support = []`.
- `crates/substrate/Cargo.toml` `[dev-dependencies]` already enables it.
- `crates/conversation/src/test_support.rs` exists, is declared in `lib.rs`
  (`#[cfg(feature = "test-support")] pub mod test_support;`), keys every
  hook by the *sending* service id, and has a poison-tolerant `locked()`
  helper. Its hooks today are `drop_next_ack` and `override_next_send`.

**Add to `test_support.rs`** (reuse `locked()`; same style as the existing
hooks):

```rust
static CLOCK_OFFSETS: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Mutex::default);

/// From now on, `service_id` signs group entries as if its clock were
/// `offset_ms` ahead (negative: behind). The receiver's own checks use
/// the real clock, as they would against a real skewed peer.
pub fn set_clock_offset_ms(service_id: &str, offset_ms: i64) {
    locked(&CLOCK_OFFSETS).insert(service_id.to_string(), offset_ms);
}

pub fn clear_clock_offsets() {
    locked(&CLOCK_OFFSETS).clear();
}

pub(crate) fn clock_offset_ms(service_id: &str) -> i64 {
    locked(&CLOCK_OFFSETS).get(service_id).copied().unwrap_or(0)
}
```

Unlike the two existing hooks, this one is **not one-shot**: a skewed clock
stays skewed for the whole test, so the test must call
`clear_clock_offsets()` at its end.

**Add the sender clock** in `crates/conversation/src/group.rs`, in the same
shape C9 used for `claimed_send_fields` in `lib.rs` (two whole functions,
one per `cfg`, not a `cfg` block inside one function):

```rust
/// The clock a sender signs a group entry with: the real clock, except
/// where a test has skewed this service on purpose.
#[cfg(feature = "test-support")]
fn sender_now_ms(service_id: &str) -> i64 {
    now_ms() + crate::test_support::clock_offset_ms(service_id)
}

#[cfg(not(feature = "test-support"))]
fn sender_now_ms(_service_id: &str) -> i64 {
    now_ms()
}
```

**Use it only for the value that gets signed.** Each of the three
functions below reads one `now` today and feeds it to both the signed
timestamp and to local times. Split it into two values:
`let now = now_ms();` (the real clock, for every local time) and
`let signed_at = sender_now_ms(service_id);` (only for what is signed).
This is the rule C9 set for 1:1 messages: `received_at` is always this
node's clock, because the outbox measures a message's age from it
(`store/message.rs:34-36`), and `claimed_send_fields` keeps the real `now`
for the queue time.

| Function (`crates/conversation/src/group.rs`) | `signed_at` (skewable) | `now` (real clock, unchanged) |
|---|---|---|
| `create_group_impl` (`:134`) | the genesis entry's `sender_timestamp_ms` (`build_membership_entry`) | `derive_group_id`, `get_or_create_group_shell` (`created_at`, `last_activity`), `group_epochs.created_at` (`:175`) |
| `change_membership_impl` (`:264`) | the membership entry's `sender_timestamp_ms` | `group_epochs.created_at` (`:300`), `conversations.last_activity` (`:305`) |
| `send_group` (`:359`) | the message entry's `sender_timestamp_ms` (`build_message_entry`) and the `messages.sender_timestamp` column | `messages.received_at`, the outbox queue time (`txq.enqueue(.., now)`), `touch_conversation` |

In `send_group`, the `messages` INSERT binds one value to both
`sender_timestamp` and `received_at` today (`VALUES (?1, ?2, ?3, ?4, ?4, …`,
`:397`). Give `received_at` its own parameter bound to `now`, the same
change C9 made in `insert_outgoing_and_enqueue`. Without it, a member with
a −90 s offset would see its own message start 90 s "old" in its outbox.

`group_epochs.created_at` must stay on the real clock for two reasons: the
new WIT field `key-stored-at` says so ("on its own clock"), and the host's
removed-author cutoff (`removed_epoch_created_at`) reads that column. Do
not change `scheduled_rekey_once`, any receive path, any validation, or
`enqueue_direct` (the 1:1 path; its test override is C9's
`override_next_send`, which stays as it is).

Host unit test in `crates/conversation/src/group/tests.rs`,
`a_clock_offset_changes_only_signed_times` (behind
`#[cfg(feature = "test-support")]`; run it with
`cargo nextest run -p syneroym-conversation --features test-support`, and
add that command to WO1's "done when"): set an offset of −90 000 ms on the
owner, create a group, add a member (with a seeded session, as in §4.3's
test), and send one message. Then assert, against a real-clock reading
taken just before:
- the genesis entry, the membership entry, and the message's
  `sender_timestamp` are about 90 s behind that reading;
- the message's `received_at`, both `group_epochs.created_at` rows, and
  `conversations.last_activity` are within a few seconds of it.
Call `clear_clock_offsets()` at the end.

This is not caught by the skew e2e alone: there the owner Z has offset 0,
and 90 s is small next to the 30-day age limit.

Feature unification: with `cargo nextest run --workspace` the feature is on
for every test build in that run. That is harmless (the offset map is empty
unless a test writes it) and is the trade-off `D-C9-10` already accepted.

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

### 7.2 `sync_membership_rows` and `adopt_new_groups` (in `app/group.rs`)

**`sync_membership_rows` changes `row` in memory and never writes it.**
Every caller writes the row itself with `put_conversation`, once, after all
its own changes (message count, activity time, a new name). An earlier
draft wrote the row inside this function and returned early when nothing
had changed, which lost the caller's changes in the common case (review
finding 3).

```rust
/// Copies host membership events Roym does not hold yet into its own
/// `messages` collection (D-C10-5), and updates `row` in memory. The
/// caller writes `row`. Cheap when nothing changed: compares the host's
/// event count with `meta.membership_events_copied`.
pub(crate) async fn sync_membership_rows<H: AppHost>(
    host: &H,
    row: &mut ConversationRow,
    info: &GroupInfo,
) -> Result<u32, String> {                       // number of rows copied
    let events = AppConversation::membership_history(host, row.id.clone()).await
        .map_err(|e| format!("{e:?}"))?;
    let seen = row.group.as_ref().ok_or("not a group row")?.membership_events_copied;
    if events.len() as u32 == seen { return Ok(0) }
    let direction = if info.is_owner { Direction::Outgoing } else { Direction::Incoming };
    let mut copied = 0;
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
        copied += 1;
    }
    if let Some(meta) = row.group.as_mut() { meta.membership_events_copied = events.len() as u32 }
    Ok(copied)
}
```

A unit-sized check for this rule lives in parity scenario 204 (§11.2): after
two injected messages with no membership change, `message_count` is 2 and
`last_activity_ms` is the second message's timestamp.

**`adopt_new_groups`** — a new member must see the group as soon as it is
added, not only after the first message (review finding 12).

```rust
/// Gives every host group that has no Roym row one, with the same
/// once-only admission decision the inbox makes (D-C10-6). Called from
/// `conversation.list` (unless `kind == "direct"`) and from `group.info`
/// when the id has no row. Best effort: one group that cannot be adopted
/// now (its `group_info` fails, or `profile` is briefly unavailable for
/// `admit-first-contact`) is skipped and logged, and tried again on the
/// next call. It must never make the whole list fail.
pub(crate) async fn adopt_new_groups<H: AppHost>(host: &H) {
    let Ok(all) = AppConversation::conversations(host).await else {
        eprintln!("roym conversation: host conversation list unavailable; adoption skipped");
        return;
    };
    for c in all.into_iter().filter(|c| c.kind == ConversationKind::Group) {
        if let Err(e) = adopt_one(host, &c.id).await {
            // No `tracing` in this crate's wasm build (see `log_inbox_error`).
            eprintln!("roym conversation: group {} not adopted yet: {e}", c.id);
        }
    }
}

async fn adopt_one<H: AppHost>(host: &H, id: &str) -> Result<(), String> {
    if load_conversation(host, id).await?.is_some() { return Ok(()) }
    let info = AppConversation::group_info(host, id.to_string()).await
        .map_err(|e| format!("{e:?}"))?;
    let mut row = new_group_row(host, id, &info, clock::now_secs()).await?;
    sync_membership_rows(host, &mut row, &info).await?;
    put_conversation(host, &row).await
}
```

Nothing is written for a group whose adoption failed, so the next call
retries the admission decision from the start. The first-contact budget is
spent only when `admit-first-contact` answers, and then the row is written
in the same call; a failure after that answer (a `put_conversation` fault)
can spend the budget twice. That is the same unfenced shape as the
existing read-modify-write backlog row, and is accepted with it.

`new_group_row` (§7.3) moves to `app/group.rs` so the inbox and
`adopt_new_groups` share it, and it no longer writes the row (the caller
does). Cost: one host `conversations()` call per `conversation.list`, plus
one `group_info` per group seen for the first time. The host list is
already read once per inbound message today (`inbox.rs:80`).

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

    // 1. First sight: admit once, against the owner (D-C10-6). The row is
    //    written at the end (step 4), or right here when the message is
    //    refused, so a refused group keeps its decision.
    let mut row = match load_conversation(host, &msg.conversation).await? {
        Some(r) => r,
        None => {
            let mut r = new_group_row(host, &msg.conversation, &info, now).await?;
            sync_membership_rows(host, &mut r, &info).await?;
            put_conversation(host, &r).await?;
            r
        }
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

    // 4. Store the message, catch up membership rows, then write the
    //    conversation row once with every change (count, activity, name).
    put_message(host, &incoming_row(msg, now)).await?;                  // extract from inbox.rs
    row.message_count += 1;
    row.last_activity_ms = row.last_activity_ms.max(msg.sender_timestamp);
    sync_membership_rows(host, &mut row, &info).await?;
    put_conversation(host, &row).await
}

// In app/group.rs (shared with `adopt_new_groups`). Does not write the row.
pub(crate) async fn new_group_row<H: AppHost>(host: &H, id: &str, info: &GroupInfo, now: u64)
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
    Ok(row)
}
```

The same `message_count` / `last_activity_ms` loss can happen between two
concurrent inbound messages (read-modify-write with no fence). That is the
existing backlog row "Three read-modify-write sequences in Roym's
`conversation` / `catalog` are unfenced", and C10 does not make it worse.

**Refactors in `inbox.rs` so the two branches share code** (the duplication
gate counts shape, not text): extract `is_blocked`, `honour_deletion_request`,
and `incoming_row(msg, now) -> MessageRow` from `on_message_inner`
(`:74-169`). The direct branch calls the same three.

**Steps 2–4 above become one function**,
`pub(crate) async fn store_group_message(host, row: &mut ConversationRow, info: &GroupInfo, msg: &Message, now) -> Result<Stored, String>`
(`Stored` = `Kept | Refused(reason) | Ignored`), which does not write
`row`. `on_group_message` calls it after the admission check and then
writes `row`; `group.unhide`'s fill-in (§7.4) calls it for each message it
reads back with `get-message`, and writes `row` once at the end.

A group row's `peer_address` is the owner, so the direct branch's
`load_conversation(...).is_none()` first-contact test is never reached for a
group id — the dispatch at `:80` happens first.

### 7.4 The `group.*` verbs (`app/group.rs`)

Every verb returns `Response`. `resolve_member_address` is the existing
`resolve_open_address` (`messages.rs:27`) made `pub(crate)` and reused as is
(it already takes `address` or `person_did`).

| Verb | Params | Result | Rules |
|---|---|---|---|
| `group.create` | `name?` | `{conversation_id, owner_address}` | Validate `name` if given. `AppConversation::create_group`. `group_info`. `new_group_row` (admission `Shown`: owner), set the name with `name_source: {now_ms, owner, "local"}` if named, `sync_membership_rows` (copies the genesis row), `put_conversation`. Every verb below that changes the row ends with one `put_conversation`. |
| `group.rename` | `conversation`, `name` | `{name, sent: bool, send_error?}` | Owner only (`info.is_owner`, else `invalid_params("only the group's owner can rename it")`). Validate. If the group has more than one member, send the profile message via `send_and_record` (§7.6) and use its id/timestamp as the `NameSource`; else a local `NameSource`. Store the row. |
| `group.add-member` | `conversation`, `address` or `person_did` | `{added, epoch, name_sent: bool, send_error?}` | Owner only. `AppConversation::add_member`. Map `Unreachable(_)` → `invalid_params(GROUP_ADD_UNREACHABLE_MESSAGE)`, `QuotaExceeded` → `invalid_params("this group is full")`. `sync_membership_rows`. If the row has a name, re-send the profile (Q3); a send failure is reported, not fatal. |
| `group.remove-member` | same | `{removed, epoch}` | Owner only. `AppConversation::remove_member`. `sync_membership_rows`. |
| `group.info` | `conversation` | see below | Works for owner and members. If Roym has no row but the host knows the group, call `adopt_new_groups` first. If the host answers `NotFound` and a Roym row exists → `{restored_only: true, name, ...row fields}`. Also runs `sync_membership_rows`, so a removal learned by sync shows at once. |
| `group.sync` | `conversation` | `{synced: true, events_copied}` | `AppConversation::sync_now`, then `sync_membership_rows`, then `put_conversation`. |
| `group.hide` | `conversation` | `{admission}` | Any member. Sets `Hidden`. |
| `group.unhide` | `conversation` | `{admission, filled_in}` | Any member. Sets `Shown` (also from `Refused`: this is the person's own choice to accept). Then fills in (D-C10-7): page `refused_messages` filtered on `{"conversation": id, "reason": {"$in": ["group-hidden", "rate-limited"]}}`; for each, `AppConversation::get_message(id)` (skip `NotFound`), run the same author block check and reserved-type rules as the inbox (so call a shared `store_group_message(host, &mut row, &info, &msg)` that §7.3 steps 2–4 are also refactored into), and delete the `refused_messages` row once stored. Reason `blocked` (an author block) is never filled in. `refused_messages` gains an index on `conversation` (`ensure_refused`, `app.rs`). |
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
- A removed member keeps its rows. Roym refuses its sends itself
  (D-C10-15), before the host is called: the host would answer
  `Internal("no key for the current epoch")`, not `PermissionDenied`
  (`group.rs:347-356`, review finding 1).
- **A removed member that has not learned its removal yet** (it was offline
  at the time) still has `is_member: true`, so Roym lets it send. Its own
  host signs the entry in the last epoch it belonged to, and the other
  members **accept** it if its timestamp is within `max_clock_skew_secs`
  (1 day) of the removal (`group/entry.rs`, the `removed_epoch_created_at`
  check). This is existing host behaviour from B5, not something C10
  changes. With §4.4 the removed member learns the removal on its next sync
  (at most `conversation_group_sync_secs`, 60 s by default, after it comes
  back online and reaches any member). Until then its UI cannot know. The
  window is stated in the backlog (§13), not hidden. R4 row 3 is about
  reading: the removed member never gets the new key, and §4.4 serves it
  no message signed after the removal. Two narrow exceptions remain: a
  lagging member's one direct push (from B5), and a small leftover of
  §4.4's timestamp rule when a member's clock runs behind the owner's.
  Both are in §4.4 and the same backlog row.

### 7.6 Changes to existing `conversation.*` verbs (`app/messages.rs`)

1. **Extract `send_and_record`** from `send` (`:133-208`):
   `pub(crate) async fn send_and_record<H: AppHost>(host, conversation: &str, content_type: &str, body: &[u8]) -> Result<MessageRow, Response>`
   — everything from `AppConversation::send` through `upsert_conversation_activity`.
   **Read the sent message back with `AppConversation::get_message(id)`**
   and take `author`, `sender_timestamp`, and `state` from it. Delete the
   outbox read (`host_message`) and both fallbacks (`now * 1000`, author
   `"self"` / profile address) from this path (review finding 13). A
   `get_message` error is an `internal_error`: the host just stored the
   message, so a miss is a real fault, not a race. `host_message` stays for
   `host_last_error` (`inbox.rs`), which needs `last_error`; switch that to
   `get_message` too and delete `host_message` if nothing else uses it.
   `send` (the verb) keeps param parsing and the new guards:
   ```rust
   let row = load_conversation(host, &conversation).await?;
   let is_group = row.as_ref().is_some_and(|r| r.kind == ConversationRowKind::Group);
   if content_type == MEMBERSHIP_EVENT_CONTENT_TYPE
       || (is_group && (content_type == CARD_CONTENT_TYPE
                        || content_type == GROUP_PROFILE_CONTENT_TYPE)) {
       return Response::invalid_params(if content_type == CARD_CONTENT_TYPE
           { CARDS_NOT_IN_GROUPS_MESSAGE } else { "this content type is reserved" });
   }
   if is_group {
       let info = group_info(...)?;                     // D-C10-15
       if !info.is_member { return Response::invalid_params(GROUP_REMOVED_NOTICE) }
   }
   ```
   Map host errors for groups: `InvalidArgument(m)` → `invalid_params(m)`
   (this carries "a group with no other member has nowhere to deliver").
   No `PermissionDenied` mapping: the host does not produce one here.
2. **`list`** (`:94`): optional params `kind` (`"direct"` | `"group"`) and
   `include_hidden` (default false). Unless `kind == "direct"`, call
   `adopt_new_groups` first (§7.2). Skip group rows whose admission is not
   `Shown` unless `include_hidden`.
3. **`history`** (`:261`): load the row; if it is a group, call
   `group_info` and `sync_membership_rows` before reading (ignore a
   `NotFound` from the host: a restored group). Add `"kind"` to the result:
   `{ "messages": [...], "kind": "direct" | "group" }`.
4. **`delete_message`** (`:378`). Rules, in order (review finding 5):
   - A row whose content type is a group system type
     (`is_group_system_type`) → `invalid_params("this row records a group change and cannot be deleted")`.
   - An **incoming** message, 1:1 or group → unchanged:
     tombstone, `asked_peer: false`, `DELETE_NOTE_NO_PEER`.
   - An **outgoing 1:1** message → unchanged (`DELETE_NOTE`).
   - An **outgoing group** message:
     - `group_info`; if `!is_member` or the group has no other member,
       tombstone, send nothing, `asked_peer: false`, note
       `DELETE_NOTE_GROUP_ALONE = "The local copy is removed and a deletion record kept. No request was sent: nobody else in this group can receive one from you now."`.
     - otherwise send the request; on success `asked_peer: true`, note
       `DELETE_NOTE_GROUP = "The local copy is removed and a deletion record kept. A request to delete it was sent to the other members; whether their clients honour it is theirs to decide, and this cannot check. Every member already holds the key this message was sent under."`
       (the last sentence is the spec's own point about groups);
     - on a send failure, keep the tombstone and answer **success** with
       `asked_peer: false` and `send_error`, not `internal_error`. The local
       copy is already gone, so an error would be false. (This is backlog
       nit N-4, fixed here for groups only; the 1:1 path keeps its current
       behaviour and its row.)
   - Where tested: the incoming group case in parity (208). Parity cannot
     make an **outgoing** group message: the host refuses a send in a
     one-member group (203), and the harness has one node. So both outgoing
     cases — "nobody to ask" and "asked the other members" — are tested in
     e2e (§11.3, `a_removed_member_reads_nothing_after_removal`).
5. **`search`** (`:432`): add `"content_type": {"$nin": [MEMBERSHIP_EVENT_CONTENT_TYPE, GROUP_PROFILE_CONTENT_TYPE]}`
   to the filter. The filter DSL supports `$nin` (`crates/data_db/src/filter.rs:239`).
   New optional param `kind` (`"direct"` | `"group"`), review finding 9:
   load the conversation rows of that kind (the `list` query, no
   adoption) and add `"conversation": {"$in": [ids...]}` to the filter; an
   empty id list returns `{matches: []}` without querying. The Hub's
   Messages search passes `kind: "direct"` (§12.1). R4 excludes message
   search, so the Groups tab has no search box.
6. **`transcript_digest`** — the new verb from §7.4.

---

## §8 `roym_web`

No code change: `web` forwards by the `router.rs` table (§6.3).

Tests (review finding 4):
- **Local path.** `scenario_73` in
  `crates/roym_web/tests/dual_build_parity/wire_origin.rs` (list ends at
  `:115`) asserts each listed method is neither `-32601` (no match arm) nor
  `-32013` (wire-refused) on the local path. Add all nine new methods with
  valid-looking params: `group.create`, `group.rename`, `group.add-member`,
  `group.remove-member`, `group.info`, `group.sync`, `group.hide`,
  `group.unhide`, `conversation.transcript-digest`. This catches a missing
  match arm in `app.rs`.
- **Wire path.** No new test is needed. `require_internal` runs first in
  every `invoke` (`app.rs`), so every `group.*` method is refused from the
  wire already; `scenario_67` over `WIRE_REFUSED_VERBS`
  (`dual_build_parity/fixtures.rs:465`) proves that rule per service. Do
  **not** add `group.info` to `WIRE_REFUSED_VERBS`: its doc comment says it
  holds "one representative verb each of the six services", and
  `conversation` already has one.

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
- `crates/conversation/src/transport/tests.rs`: §4.4 (three).
- `crates/conversation/src/store/tests.rs`: `get_message_returns_a_sent_message_with_its_signed_timestamp`,
  `get_message_hides_system_messages` (a group-key message is `NotFound`).
- `crates/core/src/dht_registry/`: the three cache rules in §3.2 step 5
  (branch A only).
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
| 204 | two inbound messages injected into this node's own group (`h.deliver`, author `did:key:zPeer204`), no membership change between them | both stored; `message_count 2` and `last_activity_ms` = the second message's timestamp (guards the lost-write bug of review finding 3); `history` shows the membership row, then the two messages, in sort order |
| 205 | a blocked author in a shown group | `refused_messages` reason `blocked`; the group row stays shown; history unchanged |
| 206 | group profile from a non-owner author | refused `not-owner`, name unchanged; an injected profile authored by the owner (own service id) at a newer timestamp changes the name; an older one injected afterwards does not |
| 207 | a card into a group | `conversation.send` with the card content type → `CARDS_NOT_IN_GROUPS_MESSAGE`; `transaction.sync {conversation: group}` → the same message |
| 208 | delete, incoming | `delete-message` on the membership row → refused; on an injected (incoming) group message → ok, `asked_peer: false`, note `DELETE_NOTE_NO_PEER` |
| 209 | transcript digest | stable across two calls; changes after one more injected message; unchanged after deleting that message |
| 210 | hide / unhide | after `group.hide`, an injected message A → refused `group-hidden`; `list` hides the group, `include_hidden` shows it. `group.unhide` → admission `shown`, `filled_in: 0`: `h.deliver` injects through the notifier only, so the host store never held A and `get-message` answers `NotFound`, which the fill-in skips; A's `refused_messages` row stays. The next injected message is stored. (The fill-in itself, with a message the host really stored, is proven in e2e: `a_stranger_adding_you_is_a_first_contact`.) |
| 211 | search | a query matching text in a membership body and a group profile body returns neither; `kind: "direct"` returns no group message; `kind: "group"` returns no 1:1 message |
| 213 | adoption without a message | create a host group directly (`h.wasm_conversation.create_group` / `h.native_conversation.create_group`, as scenario 60 does). With no message at all, `conversation.list {kind: group}` shows it (admission `shown`, genesis membership row present). `conversation.list {kind: "direct"}` does not |
| 214 | a sent row carries the host's own values | open a 1:1 conversation to a never-answering address; `conversation.send`; read the host's row with the host service directly (`h.*_conversation.get_message`); Roym's row has the same `author` and `sender_timestamp_ms` (to the millisecond), never the `now * 1000` fallback |
| 212 | export / import round trip into a second, empty installation (the `scenario_188` pattern) | bundle `schema_version 3`; the group row (kind, meta) and membership rows are restored; `group.info` on the import side → `restored_only: true` with `GROUP_RESTORED_NOTICE` |
| 60 (rewrite) | rename to `scenario_60_group_message_is_stored_in_roym_copy_parity` | an injected message into a host group that has no Roym row yet (owned by self) → row created with admission shown, message stored, `refused_messages` empty |

Also update `scenario_8_status_on_all_six_services` for the conversation
service's `schema_version: 3`.

### 11.3 Cross-installation e2e (WO6)

**Where the shared setup goes (review finding 15).** There are three test
helper files with three different jobs. Keep them separate, and say so in
each file's module comment:

| File | Job | Owner |
|---|---|---|
| `crates/substrate/tests/common/roym.rs` (exists) | Booting and driving one Roym installation (`RoymNode`). **All node-boot changes go here**, including the new shared relay and `CoordinatorNode`. | shared |
| `crates/substrate/tests/common/roym_flow.rs` (exists, from C9) | Multi-node transaction and directory flow steps (`run_client_loop`, `open_request_conv`, `send_quote`, …). Nothing about groups; C10 does not call it. | C9 |
| `crates/substrate/tests/common/conversation_fixture.rs` (exists, from C9) | The dual-build fixture as a conversation peer (`deploy_fixture`, `fixture_run`, `publish_endpoint`, …). | C9 |
| `crates/substrate/tests/common/roym_group.rs` (new) | Group flow steps only. | C10 |

Run the `cargo dupes` recipe (AGENTS.md) over these files before WO6 is
done.

**Move one private helper out of `roym_trust_e2e.rs` first.** C9's
`roym_trust_e2e.rs` has its own `boot_node(label, dir, registry, owner)`
(boot a `RoymNode` with `fast_conversation_role(3600)`, then
`full_bring_up`) and `boot_trio`, where the first node hosts the registry
and the others share it. Its comment says why: several registry servers
in one process starve each other's registration window. C10's tests need
exactly this boot, so move `boot_node` into `common/roym.rs` as
`RoymNode::boot_ready(label, dir, registry, owner, role)` (the role becomes
a parameter), and make `roym_trust_e2e.rs` call it. Do not copy it. C10's
tests boot the same way (first node hosts the registry), except the
no-coordinator test (§3, test 5), which boots on a `CoordinatorNode`.

**Optional, small, same WO:** `crates/substrate/tests/group_conversation_e2e.rs`
still has private copies of `publish_endpoint`, `deploy_fixture`,
`fixture_run`, `fast_conversation_role`, and `wait_until` that C9 moved into
`common/conversation_fixture.rs` and `common/roym.rs` for the other
conversation tests. C10 runs this binary as a regression gate (§3.2, WO1),
so switching it to the shared helpers is cheap and lowers the duplication
count. The shared `deploy_fixture` takes one more argument than the
private copy, `mode: Deploy` (`Deploy::Certified` / `Deploy::NoCertificate`,
`common/conversation_fixture.rs:79-95`); the group test passes
`Deploy::Certified`.

**Additions to `common/roym.rs`:**

- `RoymNode::boot_ready` (moved, above).
- `pub struct CoordinatorNode(SubstrateNode)` booted with the default
  roles (it hosts the registry and relay). `registry_url()`, `relay_url()`,
  `async fn teardown(self)`.
- Extend `RoymNode::make_builder` (`common/roym.rs:211`) with an optional
  `shared_relay_url` field on `RoymNode` (threaded the same way as
  `shared_registry_url`, including `resume`, `:475`). Add
  `RoymNode::boot_on(label, base_path, coord: &CoordinatorNode, owner, role)`.

**`common/roym_group.rs`** (declared in `common/mod.rs`):

- `pub fn group_role(rekey_secs: u64, max_pending_age_secs: u64) -> AppSandboxRole` built on
  `common::roym::fast_conversation_role(max_pending_age_secs)` with
  `conversation_group_sync_secs: 1` and `conversation_group_rekey_secs: rekey_secs`.
  Do not copy `fast_conversation_role` or `wait_until`. Unless a test
  below names another role, its nodes use
  `group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs)`,
  so no scheduled rekey runs during the test.
- `pub async fn form_group(owner: &RoymNode, name: &str, members: &[&RoymNode]) -> String`:
  profiles and contacts set up both ways (each member adds the owner as a
  contact and the owner adds each member), `group.create`, one
  `group.add-member` per member, then `wait_until` every member's
  **`conversation.list {kind: "group"}`** contains the group with the name
  (review finding 12: `group.info` reads host state, so waiting on it
  would hide a missing Roym row).
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
| `three_members_see_one_order_from_skewed_clocks` | C + Z (owner), X, Y. `form_group`. `test_support::set_clock_offset_ms`: X +90 000, Y −90 000, Z 0. All three send two messages with `join_all` (six sends at once). `converge`. Assert: the three projections serialise to identical bytes — this includes each author's **own** row, which must carry the skewed timestamp read back through `get-message` (review finding 13); the order equals sort by `(sender_timestamp_ms, author, id)`; the three digests are equal; three membership rows (genesis, add X, add Y) are identical everywhere. Run the send-and-compare step 5 times in a loop inside the test, because the old failure was timing-dependent. `clear_clock_offsets` at the end. | 2, 4 |
| `a_joiner_reads_nothing_before_joining` | Form Z, X. Two messages. Z adds W. `converge` on Z, X, W. W's history has no message row with a sender timestamp before the `add W` event and no id of the two early messages; W reads a message sent after the join. Membership rows identical on Z, X, W. | 3, 4 |
| `a_removed_member_reads_nothing_after_removal` | Form Z, X, Y. Y sends one message M. **`Y.stop(None)`** (Y is offline at removal: the one relay push to Y fails, review finding 2). X sends P (before the removal, while Y is offline). `wait_until` Z holds P. Z removes Y. Z sends "after removal". `converge` on Z, X. `Y.resume(None)`, login. Y's `group.info` loop (`group.sync` each time) until `is_member: false` — proves §4.4 (sync-based catch-up). Then: Y's history has the removal row and P (sent before the removal, fetched by §4.4's rule), not the new message; `key_epoch < epoch`; `removed` notice set; Y's `conversation.send` → `GROUP_REMOVED_NOTICE` and nothing is queued in Y's `conversation.outbox`; Y's `delete-message` on M → `asked_peer: false`, `DELETE_NOTE_GROUP_ALONE`. X's `delete-message` on one of X's own messages → `asked_peer: true`, `DELETE_NOTE_GROUP`, and after `converge` Z's copy of that message is tombstoned. | 3, 4 |
| `a_scheduled_rekey_changes_the_key_with_stable_membership` | Nodes with `group_role(5, AppSandboxRole::default().conversation_max_pending_age_secs)`. Form Z, X, Y. Record Z's `epoch`. Wait until Z's `epoch` grows and X's and Y's `key_epoch` equal it. Membership rows unchanged in count. A message sent after the rekey is read by all three. | 3 |

**`crates/substrate/tests/roym_group_offline_e2e.rs`**

| Test | Steps | R4 row |
|---|---|---|
| `an_offline_member_pulls_the_gap_from_another_member` | Form Z, X, Y. `Y.stop(None)`. X sends two, Z sends two. **`wait_until` X's `conversation.history` holds all four** (Z's pushes to X run later from Z's outbox; stopping Z first would leave X without Z's two messages, and Y could never converge). Then `Z.stop(None)` (the owner and one author is now offline). `Y.resume(None)`, `Y.republish_registry()`, login. Y `group.sync` until its digest equals X's. Y holds all four messages, so it pulled Z's messages from X, not from Z. | 5 |
| `members_talk_with_no_coordinator_reachable` | **Branch A only** (Q1). Form Z, X, Y on C. One round of messages. `C.teardown()`. X sends; wait until Y and Z hold it; Y sends; wait until X and Z hold it. Digests equal. **Branch B**: the §3.3 assertion instead. | 1 |
| `a_stranger_adding_you_is_a_first_contact` | X sets `contacts.set-limits {max_per_window: 0}`. W (not a contact of X) creates a group and adds X. W sends A. X's `conversation.list {kind: group}` is empty; with `include_hidden` the group shows `refused: rate-limited`; `refused_messages` has A. X `group.unhide`s → `filled_in: 1`, A is in X's history (read back from the host with `get-message`). W sends B; X holds A and B. | safety (Q4) |
| `a_message_to_a_member_removed_while_pending_settles_failed_after_the_age_window` | Failure-matrix row 16 (review finding 6). Nodes with `group_role(3600, 60)` (a 60 s `max_pending_age_secs`; the window must leave room for a slow removal step in CI). Form Z, X, Y. `Y.stop(None)`. Z sends M (X gets it; Y's item stays pending). Z removes Y. Take every sample **relative to M's own `sender_timestamp_ms`**, not to the test's clock: sample M's state in Z's `conversation.history` at M + 20 s and M + 30 s (sleep until each point; if the removal step already ran past M + 30 s, fail with a message that the step was too slow, instead of asserting a state). Both samples are `pending`. Then `wait_until` (budget: until M + 60 s + 30 s) M becomes `failed`, and assert it did not become `failed` before M + 60 s. `group.info` on Z never lists Y during the window. The words the Hub would show for each state are checked by the vitest in §12.1 (`deliveryWords` never names a member and never says "trying"). | matrix row 16 |

Put both binaries in nextest's `substrate-e2e` group (check the filter in
`.config/nextest.toml`; if it matches `roym_*_e2e` by name, no edit).

### 11.4 Hub (vitest) — see §12.

---

## §12 Hub (WO7)

### 12.1 Files

| File | What |
|---|---|
| `crates/roym_web/ui/src/groups/words.ts` (**new**) | The nine notice constants from §6.2, verbatim, plus `deliveryWords(state: "pending" \| "delivered" \| "failed"): string` (D-C10-8) and `membershipEventWords(event, nameOf): string` ("Z added X", "Z removed Y", "Z created the group"). |
| `crates/roym_web/ui/src/groups/words.test.ts` (**new**) | Every state maps to its words; no output ever contains "verified", "read", "trying", or a member's name or address (failure-matrix row 16: a pending or failed state is never shown as progress toward a member); membership words use display names when known and the short address otherwise. |
| `crates/roym_web/ui/src/screens/groups.ts` (**new**, ≤ 600 lines) | `renderGroups(container)`: left = "New group" (name input + button) and the group list (`conversation.list {kind: "group"}`, label = name or "Unnamed group"), plus a collapsed "Hidden groups (N)" section (`include_hidden`) with Unhide buttons. Right = thread (`conversation.history`) and an info panel (`group.info`). No search box. |
| `crates/roym_web/ui/src/screens/groups.test.ts` (**new**) | Rendering helpers: a name with markup becomes a text node; membership rows render as events; a profile row renders as "Z named the group …" **only when its name differs from the one before it** (D-C10-16), so two profile rows with the same name render one line; card rows render as the neutral block; an `RpcError` of type `NotSignedIn` renders the "log in again" state (§12.2), not an error line. |
| `crates/roym_web/ui/src/screens/messages.ts` (842 lines now) | `reloadList` (`:183-213`) calls `conversation.list` with `{kind: "direct"}`. The search call (`:158`) passes `{query: q, kind: "direct"}` (review finding 9). `ConversationRow` interface (`:34`) gains `kind`. No other change. `message_search.ts` (the Hub search fix already on this branch) is unchanged. |
| `crates/roym_web/ui/src/main.ts` | Tab list (`:138-150`, now with C9's Memberships tab): add `{ name: "Groups", render: () => renderGroups(tabContainer) }` right after Messages. |

`groups.ts` imports `text`, `errText`, and `field` from
`crates/roym_web/ui/src/dom.ts` (added by C9). It must not define its own
`text()` the way `messages.ts` still does.

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
- **Session ended** (carried-forward limit 5, review finding 7). The Hub
  checks the session only at page load (`main.ts:92-122`). If the substrate
  restarts while the Groups tab is open, the next call fails with
  `RpcError` type `NotSignedIn` (`rpc.ts`). The Groups screen then replaces
  its content with "Your session ended, for example because this
  installation restarted. Log in again." and a button that reloads the
  page. It is not shown as an error, and nothing typed in the composer is
  sent twice (the send was refused, not queued). Put the helper in
  `groups/words.ts` (`SESSION_ENDED_NOTICE`) so other tabs can adopt it
  later; C10 does not change the other tabs.
- **Repeated names** (D-C10-16): render a profile row only when its name
  differs from the previous profile row's name in the same thread.

### 12.3 Playwright — new `crates/substrate/tests/e2e/tests/roym-groups.spec.ts`

One installation (the existing global setup). Follow C9's
`roym-trust.spec.ts`: descriptive test names with **no case numbers**
(`roym-hub.spec.ts` case `40` is now taken by the search fix), and log in
and make setup calls with `loginWithDelegatedKey` / `rpcCall` from
`crates/substrate/tests/e2e/hub-helpers.ts`. **Add the new file to
`testMatch` in `crates/substrate/tests/e2e/playwright.config.ts`**, beside
`'**/roym-trust.spec.ts'`; a spec file not listed there never runs.

**Check the suite's time limits before WO7 is done.** The same config sets
`globalTimeout: 300_000` (5 minutes for the **whole** run), `timeout:
60_000` per test, and `workers: 1`, so every spec file adds to one
5-minute budget. These six tests add to it, and "adding a member who
cannot be reached" waits for the host's prekey fetch to give up (a 2 s
timeout, `fetch_prekey_bundle` in `crates/conversation/src/lib.rs`, plus
the proxy's own lookup of an address the registry does not know). After
adding the file:
1. Run `mise run test:e2e > target/e2e-run.log 2>&1` twice and read the
   total time Playwright prints at the end.
2. If the total is above about 80% of `globalTimeout` (4 minutes), raise
   `globalTimeout` in `playwright.config.ts`, in the same commit, with a
   one-line comment that says what the budget covers (not a count of
   tests or a number of minutes used, per AGENTS.md's comment rule).
3. If the unreachable-member test alone takes more than 20 s, give it a
   shorter wait by using an address that fails at lookup (a well-formed
   `did:key` with no registry record) rather than one that resolves but
   never answers.
Record the measured time in `status.md`.

| Test name | What it checks |
|---|---|
| `creating a group lists it and states that the owner can read it` | Create "Street Garden" → it is listed; the info panel says the person is the owner and shows `OWNER_CAN_READ_NOTICE` character for character |
| `a group name with markup renders as literal text` | A group named `<img src=x onerror=alert(1)>`: no `img` element, no request to `x` (the pattern of `roym-hub.spec.ts` case 4) |
| `adding a member who cannot be reached shows why, and changes nothing` | An address that never answers → `GROUP_ADD_UNREACHABLE_MESSAGE` verbatim; the member list is unchanged |
| `sending with no other member says there is nobody to deliver to` | The "nowhere to deliver" text; nothing appears in the thread |
| `the group info panel never says verified` | Shows the key epoch and the transcript check; the word "verified" appears nowhere in the panel |
| `groups and 1:1 conversations stay in their own tabs` | The Messages tab does not list the group; the Groups tab does not list a 1:1 conversation |

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
- §5 "No `roymctl` operator surface for conversation dead letters"
  (carried-forward limit 4, review finding 7) — restate with C10 evidence:
  a group message that gives up shows as "Not delivered to every member"
  with Retry in the Hub and in `roymctl roym group history`; the operator
  still has no node-level view. Still TBD.
- The "Person sessions are in-memory" limit (carried-forward limit 5) has
  no backlog row of its own (it is a decision, ADR-0024 §2). Record in
  `status.md`, not the backlog, that the Groups screen shows the ordinary
  "log in again" state (§12.2) and that no group work is lost on a
  restart: the host's outbox holds pending sends, and the Roym rows are
  durable.
- `task.md` "C10 completes" row: all five carried-forward limits
  re-examined — four restated above with evidence, the `MAX_PARENTS` row
  unchanged, the sessions limit recorded in `status.md`. None is closed by
  C10.

**New rows:**

| Section | Row | Target |
|---|---|---|
| §5 | **A group message's delivery state is one aggregate** — `failed` can mean one member was offline past `max_pending_age_secs` even if it later got the message by sync. No per-recipient view. Trigger: "a person needs to know which member has a message". | TBD |
| §5 | **A member cannot leave a group; they can only hide it here** (D-C10-7). Trigger: "a member-initiated leave is designed (it needs an owner-side rekey)". | TBD |
| §5 | **A removed member that was offline at removal can still post until it learns of the removal, and the other members accept those posts** while their timestamp is within `max_clock_skew_secs` (1 day) of the removal. They are signed in the last epoch the author belonged to, so `member_sig_key_at` accepts them, and only the `removed_epoch_created_at + max_skew` cutoff (`group/entry.rs`) stops them later. It is never given a key after the removal, and §4.4's sync rule serves it only messages signed no later than the removal. Two narrow ways it can still read a message sent just after the removal: a member that has not applied the removal yet pushes its new old-epoch message to it directly, once (B5's race, from before C10); and, as a small leftover of §4.4's timestamp rule, a member whose clock runs behind the owner's can sign such a message with a time just before the removal, which the sync rule then serves even if the removed member was offline for the direct push (bounded by that clock difference, at most `max_clock_skew_secs`). With §4.4 it learns the removal on its first sync after it comes back (≤ `conversation_group_sync_secs`); until then its own UI still shows it as a member. Trigger: "a removed member must stop posting at once" (it needs a durable, retried removal notice over the 1:1 channel, and a tighter cutoff than the clock-skew allowance). | TBD |
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
- `roym-integrated-experience-spec.md`: R4 marked **Passed** when C10's
  gates pass (R3 is already passed, so Q9 no longer blocks), with the
  `D-C10-17` note "WASM build across installations; both builds in
  parity"; the service table's Conversation API column gains
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
| **WO1** | Host: `group-info` and `get-message` (§4.1–4.2), pinned-key add (§4.3), removal catch-up (§4.4), clock hook (§5) | — | `cargo nextest run -p syneroym-conversation -p syneroym-app-host-native` green; `cargo nextest run -p syneroym-conversation --features test-support` green (the clock-offset test, §5); both fixture builds rebuilt; `cargo nextest run -p syneroym-substrate --test group_conversation_e2e` still green |
| **WO2** | `roym_core` (§6) | WO1 | `cargo nextest run -p syneroym-roym-core` green |
| **WO3** | `roym_conversation` (§7), router (§6.3), `wire_origin` (§8), transaction guard (§9) | WO2 | `mise run build:roym` then parity suite green |
| **WO4** | `roymctl roym group` (§10) | WO3 | parse tests green |
| **WO5** | Parity 201–214 + 60 rewrite + `scenario_73` additions (§11.2, §8) | WO3 | `cargo nextest run -p syneroym-roym-web --test dual_build_parity` green, both builds identical |
| **WO6** | e2e (§11.3) | WO3, WO0 | both binaries green |
| **WO7** | Hub + vitest + Playwright (§12) | WO3 | `mise run test:roym-ui` and `mise run test:e2e` green, and the suite's total time checked against `globalTimeout` (§12.3) |
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
   (`crates/conversation/src/lib.rs:425-455`). Fixed in §4.1.
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
11. **`messages.ts` is 842 lines.** The 800-line rule is enforced only for
    `.rs` files (`xtask/src/file_lengths.rs:388`). C10 changes three lines
    in it (§12.1) and does not grow it (D-C10-13).
12. **Fixed on this branch, separately from C10.** The Messages search read
    `res.hits[].snippet` while `conversation.search` returns
    `{ "matches": [MessageRow] }`. The separate Hub fix (new
    `crates/roym_web/ui/src/screens/message_search.ts`, and `messages.ts:158-159`)
    reads `matches`. C10 only adds `kind: "direct"` to that call (§12.1).
13. **`roym_core::conversation::CONVERSATION_SCHEMA_VERSION` (= 1) has no
    reader.** The live version is `roym_conversation::app::SCHEMA_VERSION`
    (= 2). Two constants that disagree. Deleted in §6.1.
14. **Resolved.** C9 was partial when this plan was first written, and the
    spec says each release must pass before the next begins. R3 was marked
    passed on 2026-09-29 (Q9).
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
19. **Three statements the review corrected in this plan's first draft**,
    kept here so a later reader knows they were checked, not assumed:
    a removed member's send fails with `Internal("no key for the current
    epoch")`, not `PermissionDenied` (`group.rs:347-356`); a removed member
    that is offline at removal is never told unless something changes
    (`outbox.rs:99-123`, `group_sync.rs:363`); and a sent message's row
    could take a local-clock timestamp (`messages.rs:162-179`). §4.4,
    §7.6, and D-C10-15 handle them.

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
- A few seconds later, Xavi's and Yuki's Groups tabs show the group, with
  Zara as the owner, the same owner notice, and the same member list. It
  appears as soon as they are added, before anyone has written anything.
  The name "Street Garden" arrives a moment later. Until then the list says
  "Unnamed group".
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
  Zara's installation sends it again when someone joins. The others do not
  see a second "named the group" line, because the name did not change.
- Wen's next message reaches everyone.

### 16.8 Zara removes Yuki

- Zara presses **Remove** next to Yuki. Everyone, Yuki included, sees
  "Zara removed Yuki".
- Zara sends "Planning the next meeting". Xavi and Wen see it. Yuki does
  not.
- Yuki's group shows: "You were removed from this group. You can still read
  what you received before. You cannot read or send new messages." Her
  message box is turned off.
- If Yuki's computer was off when Zara removed her, she sees all of this
  within about a minute after she starts it again. Her installation learns
  the removal from any member it can reach.

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
- If Xavi presses **Unhide**, the group appears with the messages that
  arrived while it was hidden, and new ones from then on. Messages from a
  person Xavi has blocked stay out.
- Xavi can also **Hide** any group he no longer wants to see. The Hub tells
  him that he stays a member underneath until the owner removes him.

### 16.13 The same flow without a browser

- Zara runs `roymctl roym group create --name "Street Garden"`, then
  `roymctl roym group add --group <id> --person-did <Xavi's DID>`.
- Xavi runs `roymctl roym group list`, `… group history --group <id>`, and
  `… group send --group <id> --body "hello"`.
- Each person runs `roymctl roym group check --group <id>` and gets the
  same transcript check code the Hub shows.

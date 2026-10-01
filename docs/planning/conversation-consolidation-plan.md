# Conversation consolidation: one message store, in the base crate

Status: **proposed** (2026-10-01). Needs sign-off on §3 and §9 before work starts.

Tracked as the *Conversation capability consolidation* interstitial in
[meta-implementation-plan.md](meta-implementation-plan.md), after M06C and
before Milestone 7.

## 1. Goal

Today every Roym message is stored twice on the same machine:

1. in the host store, `conversation.db`, owned by `syneroym-conversation`
   (the base crate, reached through the WIT interface `syneroym:conversation`);
2. in Roym's own data layer (`messages`, `conversations`, `refused_messages`
   collections), owned by `syneroym-roym-conversation`.

The base crate is the platform piece every SynApp uses. `roym_conversation`
is Roym only. This plan moves the general chat features into the base crate,
removes Roym's second store, and leaves `roym_conversation` as a thin product
layer.

**Principle.** A feature that any chat app would need lives in the base
crate. `roym_conversation` keeps only what depends on Roym: its profile
service (block list, contacts, first-contact limit), its card rule, and its
Hub verbs. Transaction, negotiation and agreement logic stays in
`roym_transaction`.

## 2. Why: problems caused by two stores

The second store exists because the base crate has no import, no delete, no
search, no export, and no way for an app to refuse an incoming message
(M06C `task.md`, Gaps 3 and 4). Keeping two stores in step is where most of
the current bugs come from:

| Problem | Cause |
|---|---|
| A message can be lost from Roym for good | The host tells the app about a new message once. On WASM it retries only when the guest *crashes*; when `on_message` *returns* an error, the host only logs it (`crates/sandbox_wasm/src/engine/auth.rs`, `notify_guest_message`). The native path never retries. Roym's comment in `inbox.rs` says the host retries; it does not. Nothing repairs the gap (backlog row "`conversation.history` does not reconcile Roym's copy"). |
| Delivery state is copied with three different rules | `on_delivery_state`, the `history` re-check, and `mark_retried` each update Roym's copy differently. The host already writes the state before it notifies, so the host is always right. |
| The transcript check code can differ between members | `transcript_digest` reads Roym's copy without first copying new membership events. |
| `open` can reset a chat's message count | `upsert_conversation_open` loads, then saves, with no protection against a parallel write. |
| A deleted message is still readable on disk | Roym deletes only its own copy. The host copy keeps the full body. Roym's delete note says so. |
| Every message costs double the disk space | Two full copies (backlog row "Roym's own copy ... doubles the on-disk cost"). |
| Repeated, slightly different code | Group refresh at 9 call sites, ad-hoc error mapping at 21 places, "is this a group?" decided 6 ways. |

**How this plan closes the two gaps.**

| Gap (M06C `task.md`) | Closed by | What stays open, on purpose |
|---|---|---|
| Gap 3: history cannot be written back (no import) or removed (no delete) | H2 `delete-message`; H5 `export-history` / `import-history` | Continuing an old chat after a move to a new machine. It has its own backlog row. |
| Gap 4: no block list and no way to refuse; `on-message` is only a notice after storing | H1: the host waits for the app's answer (accept / hold / drop), shows nothing until it is accepted, erases the body on `drop`, and asks again until it gets an answer | (1) The block list and the first-contact limit stay in Roym, because they are Roym data from its profile service; the host gives the refuse mechanism. (2) The host still writes the message to disk before asking, so it can confirm receipt to the sender and survive a crash. A dropped body is erased right after the answer. |

**Correction to the first analysis.** An incoming card in a *group* is stored
and shown as a neutral block on purpose (decision D-C10-11, question Q8). It is
not a bug. Only *sending* a card into a group is refused.

## 3. Decisions this plan replaces or changes

| Decision | What it said | What happens now |
|---|---|---|
| **D-06C-5** (M06C `task.md`) | Roym keeps its own copy of every message. Export, search, delete and restore act on it. Adding import and delete to the host was rejected as "more substrate work for a problem the app can solve". | **Replaced.** The host gains import, export, delete and search. Roym's signed backup includes the host's export. |
| **D-06C-8** (M06C `task.md`) | Block is enforced only in Roym's inbox; the blocked message stays in the host store. A host admit hook was put off until "a second app wants inbound filtering". | **Changed.** The host gains the mechanism (the app answers accept / hold / drop). Roym keeps the policy (who is blocked, the first-contact limit). |
| **D-C5-7** (slice C5 plan) | Roym's copy stores full bodies; disk cost is 2×, and the product says so. | **Replaced.** No second copy. |
| **D-C5-10** (slice C5 plan) | Delivery state is never Roym's to invent; `history` re-asks the host for every row not yet delivered. | **Rule kept, mechanism removed.** Roym reads the state straight from the host. Nothing is copied, so nothing can go stale. |
| **D-C5-11** (slice C5 plan) | A refused message goes into `refused_messages`, body dropped, invisible everywhere. Block is checked on every message; the first-contact limit only on the first one. | **Changed.** The host hides held and dropped messages from every read. The two checking rules stay as Roym policy. |
| **D-C10-4** (slice C10 plan) | The group name is the newest owner-sent `group-profile` message in Roym's copy. | **Replaced.** The name is an owner-signed entry in the host's group log (§5, D-CV-5). |

These are recorded decisions, so the change needs an ADR: ADR-0025 (draft in
the appendix). It builds on ADR-0013 §6, which already says the substrate
handles the core protocol and chat apps are light wrappers over it. This plan
moves storage features into that core layer, where §6 places them.

## 4. Target split

| Responsibility | Lives in |
|---|---|
| Message storage, ordering, delivery state, outbox, retry | Base crate (already there) |
| App answer to a new message: accept / hold / drop, re-asked until answered; bring held messages back later | Base crate (new) |
| Delete the local copy; send and honour a "please delete" request | Base crate (new) |
| Group events (membership, name) inside `history`; transcript digest | Base crate (new) |
| Group name | Base crate (new) |
| Search | Base crate (new) |
| Export and import of history | Base crate (new) |
| Message count and name in conversation summaries | Base crate (new) |
| Who is blocked; first-contact limit (both from Roym's profile service) | `roym_conversation` |
| Which groups are shown or hidden; first-sight admission of a group | `roym_conversation` |
| Open a chat by person DID; show person DID next to addresses (Roym contacts) | `roym_conversation` |
| "Cards are sent only in 1:1 chats" at the one send gate | `roym_conversation` (rule text and content type stay in `roym_core::card`) |
| Hub JSON-RPC verbs, product notice texts, error wording | `roym_conversation` |
| Roym's signed backup bundle (wraps the host export) | `roym_conversation` |
| Sending cards, reading cards back, request/quote/agreement state | `roym_transaction` (unchanged) |

**Why `roym_conversation` still exists as its own service.** The host keeps one
store per *service*, and a chat address is a service address. Roym's chats
belong to the `conversation` service's address. `roym_transaction` sends its
cards through `conversation.send` because sending from its own address would
start a different chat. So `roym_conversation` stays as the one service that
owns Roym's chat address, even though its code becomes small (expected
600–800 lines, down from about 2,260).

## 5. Design decisions

**D-CV-1: The app answers every incoming message, and the host keeps asking
until it gets an answer.**
- Each incoming message row gets an `admission` value: `pending`, `accepted`,
  `held(reason)` or `dropped(reason)`.
- `guest-api.on-message` returns an answer: `accept`, `hold(reason)` or
  `drop(reason)`.
- If the call fails (error, crash, timeout), the row stays `pending`. The
  outbox worker asks again on later ticks with growing gaps (the same curve
  as `backoff_for_age`, capped at 5 minutes). A metric counts pending rows.
- A component that does not export `guest-api`, or a native service with no
  sink, gets `accept` automatically.
- `history`, `get-message`, `search`, counts, the digest and `outbox` show
  only `accepted` rows (and the service's own sent messages).
- `drop` removes the body but keeps the row, so a repeat delivery is still
  recognised and not asked about again.
- New verb `readmit(conversation, reasons)`: sets matching `held` rows back
  to `pending`, so they are asked about again. Roym's "unhide" uses it.
- **Why:** this fixes lost messages for every app, not only Roym. It also
  makes block real at the store level: a blocked person's message never
  becomes readable through any verb.

**D-CV-2: The WIT `message` record gains `outgoing: bool` and
`deleted-at: option<s64>`.**
- The host already stores `outgoing`; it was not exposed. Roym needs it for
  its `direction` field, because no host verb tells a service its own address.

**D-CV-3: Delete and the "please delete" request move to the host.**
- `delete-message(message, ask-others: bool)`: replaces the body with an
  empty value and sets `deleted-at`. The row stays, so ordering and duplicate
  detection still work.
- With `ask-others`, the host sends a reserved system message to the peer or
  the group. System messages are never shown and never counted.
- The receiving host honours a request only when the target message is in the
  same conversation and has the same author as the request. Membership and
  name entries are never deleted. This is Roym's current rule.
- **Group messages:** the encrypted entry in the group log stays, because
  other members sync from it. The readable body is removed. The delete note
  says this plainly.

**D-CV-4: `history` returns group events between messages; the host computes
the transcript digest.**
- `history-page` becomes a list of `history-item`:
  `message(message)`, `membership(membership-event)`, `group-name(name-event)`.
  All are ordered by the one rule (sender timestamp, author, id). The cursor
  stays a string.
- New verb `transcript-digest(conversation)`: computed by the host over the
  same items, with the same fields Roym uses today (id, author, sender
  timestamp, content type), including deleted messages and excluding held and
  dropped ones.
- **Why:** every group chat needs to show "X was added". Doing the merge in
  one place removes Roym's copying step and its stale-digest bug.

**D-CV-5: The group name is an owner-signed entry in the group log.**
- New log entry kind `profile`, carrying `name`. It is signed by the owner and
  not encrypted, the same as membership entries, so a new member can read it
  at once.
- New verb `set-group-name(conversation, name)`: owner only. The host checks
  the name: 1–80 characters after trimming, no control characters (today's
  `validate_group_name`).
- `group-info` and `conversation-summary` gain `name: option<string>`.
- **Why:** today Roym must send the name again every time a member is added,
  because a new member cannot read messages from before they joined. A log
  entry reaches new members through normal sync.
- **Cost:** the name is visible to whoever receives the group log, which is
  the same group of people who already see the member list.

**D-CV-6: Search moves to the host.**
- New verb `search(query, conversation: option, limit)`. It uses an FTS5
  (SQLite full-text search) index over text bodies of `accepted`, not-deleted
  messages, inside the same encrypted `conversation.db`.
- FTS5 is available. It is a compile flag, not a cargo feature: the
  `bundled-sqlcipher` build of `libsqlite3-sys` 0.36.0 (the version in
  `Cargo.lock`) compiles SQLite with `-DSQLITE_ENABLE_FTS5`. No code in the
  project uses it yet. H4 still adds a unit test that creates an FTS5 table,
  so a future dependency change that drops the flag fails a test instead of
  failing in production.
- C6 could not use FTS5 because app code reaches SQLite only through the
  data-layer filter language, which has no raw SQL. The base crate uses
  `rusqlite` directly, so that limit does not apply here.

**D-CV-7: Export and import of history move to the host, in pages.**
- `export-history(cursor) -> (chunk, next-cursor)` and `import-history(chunk)`.
  Pages keep large histories out of one WASM call.
- An export holds conversations, messages (with signatures, `deleted-at`,
  `admission`), group members and group log entries. It does **not** hold
  session keys or group epoch keys.
- Imported rows are marked `restored`. They are never sent again. A restored
  message that was still pending becomes `failed` with the reason "restored
  from a backup".
- `group-info` gains `restored: bool`. It replaces Roym's "restored only" rule
  (today: Roym holds the row, the host says `not-found`).
- The format is the host's own, with a version number. Roym's signed bundle
  carries it as one section.
- Out of scope: continuing an old chat after a move to a new machine (backlog
  row "Conversations cannot continue after a substrate moves"). This plan
  restores *history* only, as today.

**D-CV-8: Summaries carry the message count; last activity is the host's own
clock.**
- `conversation-summary` gains `message-count` (accepted, not system).
- `last-activity-at` stays the host's local time of the last change. Today
  Roym uses the newest sender timestamp instead. The local time is simpler and
  cannot be pushed into the future by a peer's wrong clock.

**D-CV-9: What Roym stores after the switch.**
- One small collection, `admissions`: conversation id → kind and admission
  (`shown`, `hidden`, `refused(reason)`). It answers "is this a chat Roym has
  already accepted?" (first-contact limit) and "is this group hidden?".
- Person DIDs are looked up from contacts when a list is built. They are not
  stored.
- Roym's JSON answers keep today's field names (`direction`,
  `body_encoding`, `state`, `deleted_at_secs`, and so on), so the Hub and
  `roym_transaction` need few changes. One change: the `history` cursor
  becomes the host's string cursor instead of a number. `roym_transaction`'s
  `sync.rs` must pass `next_cursor` back.
- Roym no longer exports `on-delivery-state`; it reads the state on demand.

**D-CV-10: "Cards are sent only in 1:1 chats" stays.**
- A card is one step in a two-party agreement (request → quote → agreement
  receipt). Each party signs and keeps its own record. A group has no clear
  other party, for example no rule for who may accept a quote.
- In a group, every member, and the owner who holds the group key, would see
  prices and terms.
- If someone in a group wants to do business, they open a 1:1 chat with that
  member.
- Group buying or split payments would need their own multi-party agreement
  design. They should not come in by simply allowing cards into groups.
- The check stays at the one send gate (`conversation.send`), and
  `transaction.sync` keeps refusing a group conversation. An incoming card in
  a group stays visible as a neutral block.

## 6. Interface changes (WIT sketch)

In `crates/wit_interfaces/wit/conversation/conversation.wit`. Final names are
settled in phase H1–H5 reviews.

```wit
record message {
    // ... existing fields ...
    outgoing: bool,
    deleted-at: option<s64>,
}

variant admission { accept, hold(string), drop(string) }

record name-event { entry: string, name: string, sender-timestamp: s64 }

variant history-item {
    message(message),
    membership(membership-event),
    group-name(name-event),
}

record history-page { items: list<history-item>, next-cursor: option<string> }

record conversation-summary { /* ... */ message-count: u32, name: option<string> }
record group-info { /* ... */ name: option<string>, restored: bool }

record export-chunk { data: list<u8>, next-cursor: option<string> }

// new functions in `interface conversation`
delete-message: func(message: message-id, ask-others: bool) -> result<_, conversation-error>;
readmit: func(conversation: conversation-id, reasons: list<string>) -> result<u32, conversation-error>;
search: func(query: string, conversation: option<conversation-id>, limit: u32)
    -> result<list<message>, conversation-error>;
set-group-name: func(conversation: conversation-id, name: string) -> result<_, conversation-error>;
transcript-digest: func(conversation: conversation-id) -> result<string, conversation-error>;
export-history: func(cursor: option<string>) -> result<export-chunk, conversation-error>;
import-history: func(data: list<u8>) -> result<u32, conversation-error>;

interface guest-api {
    on-message: func(msg: message) -> result<admission, string>;
    on-delivery-state: func(msg: message-id, state: delivery-state) -> result<_, string>;
}
```

A WIT change touches all of these:

- WIT copies: `crates/wit_interfaces/wit/conversation/` plus each
  `wit/deps/conversation/` under `roym_conversation`, `roym_catalog`,
  `roym_web`, `roym_directory`, `roym_transaction`, `roym_profile`, and
  `test-components/dual-build-fixture`.
- RPC trait and types: `crates/rpc/src/conversation.rs` (`ConversationHost`,
  `ConversationNotifier`).
- Host: `crates/conversation`.
- WASM side: `crates/sandbox_wasm/src/host_capabilities/capabilities_messaging.rs`,
  `crates/sandbox_wasm/src/engine/auth.rs` (reads the answer from `on-message`).
- Native side: `crates/app_host/src/{lib,guest,types}.rs` (`AppConversation`,
  `ConversationSink`), `crates/app_host_native/src/{host,factory}.rs`,
  `crates/app_host_native/src/convert/conversation.rs`.
- Native RPC: `crates/control_plane/src/synsvc_native/conversation.rs`.
- Fixture and parity: `test-components/dual-build-fixture`,
  `crates/app_host_native/tests/dual_build_parity/conversation.rs`.

## 7. Phases

Each phase is its own PR on a feature branch and passes `mise run verify`
(run `mise run build:roym` first; the parity suite loads pre-built WASM).
Schema changes are made in place, with no migration code (the product is not
released). Host phases H2–H5 do not change Roym's behaviour; Roym switches in
R1. H1 must touch Roym, because the `on-message` signature changes.

### H1: admission answers (size L)

Base crate:
- `store/schema.rs`: add `admission TEXT NOT NULL DEFAULT 'accepted'` and
  `admission_reason TEXT` to `messages`; add `notify_attempts` and
  `next_notify_at` for re-asking.
- New incoming rows start as `pending`. Outgoing and system rows start as
  `accepted`.
- All readers (`history`, `get_message`, `outbox_messages`, `message_count`,
  `list_conversations` activity) filter on `accepted` or `outgoing`.
- `transport.rs`, `group.rs`, `transport/group_sync.rs`: after storing, call
  the notifier and store its answer. If there is no answer, leave the row
  `pending`.
- `outbox.rs`: new `renotify_pending_once`, run from `run_worker`.
- `readmit` verb.

Interface: `admission` variant, `on-message` result, `readmit`
(all files in §6). `ConversationNotifier::notify_message` returns an outcome
(`Answered(admission)` or `NoAnswer`). `ConversationSink::on_message` returns
`Result<Admission, String>`.

Roym (still keeps its copy in this phase):
- `on_message` returns `accept` after storing or ignoring a duplicate.
- It returns `hold("group-hidden" | "rate-limited")` and
  `drop("blocked" | "reserved-type" | "not-owner" | "bad-group-profile")`.
- A storage fault or an unavailable profile service returns `Err`, so the
  host asks again.
- `unhide` calls `readmit` instead of reading held messages with `get_message`.

Tests:
- Base crate unit tests: pending, accepted, held and dropped rows are visible
  only when they should be; re-asking after a failed answer; readmit.
- Parity (fixture): the answer travels the same way on WASM and native.
- New Roym parity scenario: a profile outage during delivery does not lose
  the message.

### H2: message fields, delete, "please delete" request (size M)

- WIT `message`: add `outgoing` and `deleted-at`.
- Store: `deleted_at` column; `delete_message` keeps the row and empties the
  body.
- Reserved system content type for the delete request. The receiving side
  honours it in `transport.rs` and in group entry apply, using the
  same-author rule from D-CV-3.
- Tests:
  - unit: a deleted row keeps its position and is recognised as a duplicate
  - cross-node in `transport/tests.rs` and `group/tests.rs`: a request is
    honoured only for the author's own message; a membership entry is never
    deleted.

### H3: group events in history, group name, digest, summary fields (size L)

- `dag.rs`: `EntryKind::Profile` with a signed `name`. Accept only entries
  from the owner. Apply rule: newest by (sender timestamp, author, entry id).
- `set-group-name` verb, with the name check moved from
  `roym_core::conversation::group::validate_group_name`.
- `history`: merge `messages` and the membership/profile log entries in one
  ordered read (a `UNION ... ORDER BY sender_timestamp, author, id`). The
  cursor encodes the last item's ordering key.
- `transcript-digest` verb. `group-info` gains `name`; `conversation-summary`
  gains `message-count` and `name`.
- Tests:
  - unit: ordering across both tables; cursor paging across both
  - cross-node: a new member sees the name without a resend, and two members
    get the same digest after a membership change.

### H4: search (size M)

- A unit test that creates an FTS5 table, guarding the compile flag
  (§5, D-CV-6).
- An FTS5 table over text bodies, kept in step by the same transactions that
  insert, delete or change admission.
- `search` verb. Query text is passed as an FTS5 phrase, so special
  characters are searched for literally (today's `escape_regex` rule).
- Tests: unit tests for matching, held/dropped/deleted rows not found, and
  the conversation filter.

### H5: export and import (size L)

- Versioned export format and paged `export-history` / `import-history`.
- Restored rows; `group-info.restored`.
- Tests:
  - unit: round trip on one store
  - cross-node: restore on a clean node shows the same history and digest
  - pending messages come back as `failed` with the "restored" reason.

### R1: Roym switches to the host (size L, three PRs)

- **R1a, reads.**
  - `history`, `search`, `list`, `group.info` and `transcript-digest` read
    from the host and map the result into today's JSON fields.
  - `list` joins host summaries with the `admissions` collection and a
    contacts lookup.
  - Roym still writes its old copy, which is harmless for one PR.
  - `roym_transaction/src/app/sync.rs` switches to the string cursor.
- **R1b, writes.**
  - `delete-message` → host `delete-message(ask_others)`.
  - `group.rename` → `set-group-name`.
  - `export`/`import` → Roym bundle with the host export section plus
    `admissions`.
  - `send` keeps the content-type gate (cards only in 1:1; reserved types)
    and calls the host.
  - Error mapping goes through one `from_host(ConversationError) -> Response`
    table.
- **R1c, removal.**
  - Delete the `messages`, `conversations` and `refused_messages`
    collections, and every writer and reader of them.
  - Delete the `on-delivery-state` export, `sync_membership_rows`, the group
    adoption code and the message counter code.
  - Shrink `roym_core::conversation`: keep `encode_body`, `Direction` and the
    JSON row type used for the answers. Remove `GroupMeta.name_source` and
    `membership_events_copied`.
  - Bump the backup bundle's schema version.

Tests to rewrite (they test the old copy's internals):
- `roym_web/tests/dual_build_parity/conversation.rs`: scenarios 56 (delivery
  state), 60, 62 (deletion request) and 63–65 (export/import).
- `group.rs` and `group_lifecycle.rs`: 201–215, especially 210 (hide/unhide),
  211 (search), 212 (restore), 213 (adoption) and 214.
- Substrate e2e: `roym_conversation_e2e.rs`, `roym_group_e2e.rs`,
  `roym_group_offline_e2e.rs`, `roym_restore_e2e.rs`.
- Hub: `crates/roym_web/ui/src/screens/{messages,groups}.ts` only if a field
  changes. Keep the fields stable.

### R2: documents (size S)

- Write `docs/decisions/0025-conversation-capability-owns-history.md` from
  the appendix. Add a line to ADR-0013's status pointing to it.
- In M06C `task.md`, mark D-06C-5 replaced and D-06C-8 changed, with a link
  to ADR-0025. Slice plans are history and are not edited.
- `docs/roym-integrated-experience-spec.md` §"Retention and deletion": remove
  "each message is stored twice". Say that delete removes the readable body
  on this machine, and that a group's encrypted log entry remains.
- `profile.policy` text and the Hub Backup screen: remove the "two copies"
  statement.
- `docs/planning/deferred-backlog.md`:
  - move to "Recently resolved": "`conversation.history` does not reconcile
    Roym's copy", "Roym's own copy ... doubles the on-disk cost",
    "`conversation.search` is a `$regex` scan", and the inbound-filter
    (admit hook) row
  - add rows for anything left open in H4 (index) or H5 (size limits).

## 8. Order and size

H1 → H2 → H3 → H4 → H5 → R1a → R1b → R1c → R2. H2, H3 and H4 do not depend
on each other and can run in parallel after H1. H5 depends on H2 and H3,
because it exports deleted rows and log entries. Sizes are relative:
S < M < L.

A separate change is moving the data-layer page loop and backup helpers
into `roym_core`. If it lands first, `roym_conversation` uses those helpers
in H1 and R1 (for the `admissions` collection and the backup bundle). R1c
deletes most of the page loops it would have touched in this crate. Neither
change blocks the other.

## 9. Open questions (recommendation first)

1. **Group message delete:** keep the encrypted log entry so other members
   can still sync it, and say so in the delete note. *Recommended.* The
   alternative, dropping the encrypted body from the log, would stop serving
   it to members who have not received it yet.
2. **Honouring a "please delete" request:** the receiving host does it
   automatically, author-only. *Recommended.* The alternative is to pass it
   to the app as a new callback.
3. **Last activity time:** the host's local clock (D-CV-8). *Recommended.*
4. **Export format:** the host's own versioned format inside Roym's signed
   bundle (D-CV-7). *Recommended.* D-06C-5 worried that Roym's export would
   depend on a host format. A version number in the host format, plus Roym's
   own signature and manifest, covers that.

## 10. Risks

- **Size of the WIT change.** Each WIT change touches about 15 files across
  8 crates, plus 8 WIT copies. Keep each phase's WIT change small, and land
  it in the same PR as its host code.
- **Re-asking forever.** If an app always fails to answer, its messages stay
  `pending` and invisible. This is safer than losing them, but it must be
  visible: add a metric and a warning after N attempts.
- **History merge cost.** The `UNION` read must use the existing order
  indexes on both tables (`idx_messages_order`, `idx_dag_order`). Check the
  query plan in H3.
- **The Roym parity and e2e rewrite is the largest test cost.** Write each
  new scenario before deleting the old one.

## Appendix: ADR-0025 draft

**Title:** ADR 0025: Inbound admission, deletion, search, history export and
group names belong to the conversation capability

**Related:** ADR-0013 (§5 ordering rule, §6 layers), ADR-0023 (durable async
primitives: the outbox the re-ask loop runs in).

**Context.** `syneroym:conversation` stored and delivered messages but offered
no way to refuse, delete, search, export or import them. Roym worked around
this with a second full copy of every message in app storage (M06C D-06C-5,
D-06C-8; slice C5 D-C5-7, D-C5-10, D-C5-11; slice C10 D-C10-4). Keeping two
stores in step caused lost messages, stale delivery states and a transcript
check that could differ between members. It also doubled disk use and left
"deleted" messages readable in the host store. Every future chat SynApp would
face the same gaps.

**Decision.** The conversation capability provides:
- a durable app answer for each incoming message (accept, hold, drop), asked
  again until answered, with readmit
- local delete and an author-only "please delete" request
- group events and names inside history, and the transcript digest
- full-text search
- paged, versioned export and import of history.

Apps keep only their policy (who to accept) and their product data.

**Consequences.**
- One store per service, and no app-side copy.
- Deleting removes the readable body from the host store.
- Restore of history is a capability feature, not an app feature.
- The WIT interface grows by seven functions and three record changes.
- Continuing a conversation after a move to a new machine is still open. It
  has its own backlog row.

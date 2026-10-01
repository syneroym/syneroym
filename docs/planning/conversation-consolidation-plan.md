# Conversation consolidation: one message store, in the base crate

Status: **proposed** (2026-10-01; revised the same day after review).
Needs sign-off on §3 and §9 before work starts.

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
| A message can be lost from Roym for good | The host tells the app about a new message once. On WASM it retries (4 tries, 50 ms apart) only when the component fails to start or the guest *crashes*. When `on_message` *returns* an error, the host only logs it (`crates/sandbox_wasm/src/engine/auth.rs`, `notify_guest_message`). The native path never retries. Roym's comment in `inbox.rs` says the host retries on an error; it does not. Nothing repairs the gap (backlog row "`conversation.history` does not reconcile Roym's copy"). |
| Delivery state is copied with three different rules | `on_delivery_state`, the `history` re-check, and `mark_retried` each update Roym's copy differently. The host already writes the state before it notifies, so the host is always right. |
| The transcript check code can differ between members for the wrong reason | `transcript_digest` reads Roym's copy without first copying new membership events. |
| `open` can reset a chat's message count | `upsert_conversation_open` loads, then saves, with no protection against a parallel write. |
| A deleted message is still readable on disk | Roym deletes only its own copy. The host copy keeps the full body. Roym's delete note says so. |
| Every message costs double the disk space | Two full copies (backlog row "Roym's own copy ... doubles the on-disk cost"). |
| Repeated, slightly different code | Group refresh at 9 call sites, ad-hoc error mapping at 21 places, "is this a group?" decided 6 ways. |

**How this plan closes the two gaps.**

| Gap (M06C `task.md`) | Closed by | What stays open, on purpose |
|---|---|---|
| Gap 3: history cannot be written back (no import) or removed (no delete) | H2 `delete-message`; H5 `export-history` / `import-history` | Continuing an old chat after a move to a new machine. It has its own backlog row. |
| Gap 4: no block list and no way to refuse; `on-message` is only a notice after storing | H1: the host waits for the app's answer (accept / hold / drop), shows nothing until it is accepted, erases the readable body on `drop`, and asks again until it gets an answer | (1) The block list and the first-contact limit stay in Roym, because they are Roym data from its profile service; the host gives the refuse mechanism. (2) The host still writes the message to disk before asking, so it can confirm receipt to the sender and survive a crash. (3) For a group message, the encrypted log entry and the group key stay on the node after a `drop` (D-CV-1). |

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
| **D-C10-4** (slice C10 plan) | The group name is the newest owner-sent `group-profile` message in Roym's copy. | **Replaced.** The name is an owner-signed entry in the host's group log (D-CV-5). |
| **Transcript check notice** (`TRANSCRIPT_CHECK_NOTICE`, slice C10) | "A member who blocked someone in the group sees a different code." | **Changed (decided 2026-10-01, §9 question 1).** The code covers every message in the group log, whatever each member's local answer was. It proves "we hold the same log", and blocking no longer changes it (D-CV-4). |

These are recorded decisions, so the change needs an ADR: ADR-0025 (draft in
the appendix). It builds on ADR-0013 §6, which already says the substrate
handles the core protocol and chat apps are light wrappers over it. This plan
moves storage features into that core layer, where §6 places them.

## 4. Target split

| Responsibility | Lives in |
|---|---|
| Message storage, ordering, delivery state, outbox, retry | Base crate (already there) |
| App answer to a new message: accept / hold / drop, re-asked until answered; bring held messages back later; time limit on held bodies | Base crate (new) |
| Delete the local copy; send and honour a "please delete" request | Base crate (new) |
| Group events (membership, name) inside `history`; transcript digest | Base crate (new) |
| A feed of newly visible messages, in local order | Base crate (new) |
| Group name | Base crate (new) |
| Search | Base crate (new) |
| Export and import of history | Base crate (new) |
| Message count and name in conversation summaries | Base crate (new) |
| Who is blocked; first-contact limit (both from Roym's profile service) | `roym_conversation` |
| Which chats Roym has accepted; which groups are shown or hidden (`admissions`) | `roym_conversation` |
| Open a chat by person DID; show person DID next to addresses (Roym contacts) | `roym_conversation` |
| "Cards are sent only in 1:1 chats" at the one send gate | `roym_conversation` (rule text and content type stay in `roym_core::card`) |
| Hub JSON-RPC verbs, product notice texts, error wording | `roym_conversation` |
| Roym's signed backup bundle (wraps the host export) | `roym_conversation` |
| Sending cards, reading cards back, request/quote/agreement state | `roym_transaction` (reads cards through the new feed) |

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
- **Three outcomes, not two.** The notifier returns `Answered(admission)`,
  `NoHandler` or `NoAnswer`:
  - `NoHandler` means the app *declares* it has no inbox: a WASM component
    whose exports do not include `guest-api.on-message`, or a native service
    registered without a conversation sink. It comes from what the app
    exports or registers, never from a value that is simply not set yet. The
    host treats `NoHandler` as `accept`.
  - `NoAnswer` covers everything else: the call failed, crashed or timed
    out; the default notifier is not wired yet at boot; or a declared native
    sink is not set yet. The row stays `pending`. Messages that arrive during
    startup therefore wait for Roym; they never skip its block list.
  - Base-crate tests that need messages to become visible install a test
    notifier that answers `accept`. A test with no notifier checks that rows
    stay `pending`.
- **One asker at a time.** The row is inserted with `next_notify_at` already
  set a short time ahead (a claim window, for example 30 seconds). The answer
  request made during delivery runs inside that window. The worker re-asks
  only rows whose `next_notify_at` has passed, then moves it forward with
  growing gaps (the `backoff_for_age` curve, capped at 5 minutes).
- **The app's answer must be safe to repeat** for the same message id: a
  crash between "app decided" and "host stored the answer" causes a second
  ask. Roym makes this safe by writing its per-chat decision to `admissions`
  before it answers, and by reading `admissions` first on every ask
  (D-CV-9), so `contacts.admit-first-contact` is charged once per chat, not
  once per ask.
- `history`, `get-message`, `search`, counts, the feed (D-CV-11) and
  `outbox` show only `accepted` rows and the service's own sent messages.
  The transcript digest is the one exception (D-CV-4).
- `drop` empties the readable body but keeps the row (id, author, timestamp,
  content type), so a repeat delivery is still recognised and not asked
  about again. **For a group message, the encrypted entry in the group log and
  the group key stay on the node**, because other members sync from that
  entry. The node could still decrypt it. The product says this plainly.
- New verb `readmit(conversation, reasons)`: sets matching `held` rows back
  to `pending`, so they are asked about again. Roym's "unhide" uses it.
- **Held bodies have a time limit.** A row held longer than
  `conversation_max_held_age_secs` (new config field, default 30 days)
  becomes `dropped("expired")`.
- **Why:** this fixes lost messages for every app, not only Roym. It also
  makes block real at the store level: a blocked person's message never
  becomes readable through any verb.

**D-CV-2: The WIT `message` record gains `outgoing: bool`,
`deleted-at: option<s64>` and `restored: bool`.**
- The host already stores `outgoing`; it was not exposed. Roym needs it for
  its `direction` field, because no host verb tells a service its own address.
- `restored` marks rows that came from `import-history` (D-CV-7).
- The WIT doc for `verified` changes to: true for every row this node sent or
  received itself; **false for restored rows**, whose keys were not restored
  and whose signatures cannot be checked again.

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
the transcript digest over the whole log.**
- `history-page` becomes a list of `history-item`:
  `message(message)`, `membership(membership-event)`, `group-name(name-event)`.
  All are ordered by the one rule (sender timestamp, author, id). The cursor
  stays a string. `history` is for reading a chat; it is not a feed of new
  messages (see D-CV-11).
- `membership-history` is removed once Roym stops calling it (R1b), so there
  is one way to read membership changes.
- New verb `transcript-digest(conversation)`: computed by the host over the
  same fields Roym uses today (id, author, sender timestamp, content type),
  for **every non-system row in the conversation, whatever its admission**
  (pending, accepted, held or dropped), plus deleted rows and all membership
  and name entries. The host keeps those four fields even after a drop, so
  this is always possible.
- **Why:** the code then depends only on the synced log, not on each
  member's local rules. Two members with the same log always see the same
  code, even if one of them blocked someone. This changes the product notice
  (§3, last row; §9 question 1).

**D-CV-5: The group name is an owner-signed entry in the group log.**
- New log entry kind `profile`, carrying `name`. It is signed by the owner and
  not encrypted, the same as membership entries, so a new member can read it
  at once.
- New verb `set-group-name(conversation, name)`: owner only. The host checks
  the name: 1–80 characters after trimming, no control characters (today's
  `validate_group_name`).
- **The same check runs when an incoming `profile` entry is applied.** An
  entry with a bad name, or not signed by the owner, is not applied, and the
  name does not change. This stops a misbehaving owner client from sending a
  huge name.
- `group-info` and `conversation-summary` gain `name: option<string>`.
- **Why:** today Roym must send the name again every time a member is added,
  because a new member cannot read messages from before they joined. A log
  entry reaches new members through normal sync.
- **Cost:** the name is visible to whoever receives the group log, which is
  the same group of people who already see the member list.

**D-CV-6: Search moves to the host and keeps substring matching.**
- New verb `search(query, conversation: option, limit)`.
- It uses an FTS5 (SQLite full-text search) table with the **`trigram`
  tokenizer**, inside the same encrypted `conversation.db`. Today's search
  matches any substring (an escaped `$regex`). The default FTS5 tokenizer
  matches whole words only, so "ell" would stop finding "hello"; `trigram`
  keeps substring matching.
- `trigram` needs at least 3 characters. A shorter query uses a `LIKE` scan
  over the same visible rows, limited by `conversation` when given.
- **What is searched:** the body of `accepted`, not-deleted messages whose
  content type is text: the same rule as today's `encode_body` (`text/*`,
  `application/json`, `*+json`). Card JSON stays searchable, as today.
  Membership and name entries are not searched.
- FTS5 and `trigram` are available. FTS5 is a compile flag, not a cargo
  feature: the `bundled-sqlcipher` build of `libsqlite3-sys` 0.36.0 (the
  version in `Cargo.lock`) compiles SQLite with `-DSQLITE_ENABLE_FTS5`. Its
  bundled SQLite (3.50.4) includes the `trigram` tokenizer. No code in the
  project uses FTS5 yet. H4 adds a unit test that creates a `trigram` FTS5
  table, so a future dependency change that drops it fails a test instead
  of failing in production.
- C6 could not use FTS5 because app code reaches SQLite only through the
  data-layer filter language, which has no raw SQL. The base crate uses
  `rusqlite` directly, so that limit does not apply here.

**D-CV-7: Export and import of history move to the host, in pages.**
- `export-history(cursor) -> (chunk, next-cursor)` and `import-history(chunk)`.
  Pages keep large histories out of one WASM call.
- An export holds conversations, messages (with signatures, `deleted-at`,
  `admission`), group members and group log entries. It does **not** hold
  session keys or group epoch keys.
- **Restored rows are history only.** Imported messages and conversations
  carry a `restored` flag:
  - Restored messages have `verified = false` (D-CV-2). The host does not
    trust the bundle: an app could put any author in it.
  - A restored message that was still pending becomes `failed` with the
    reason "restored from a backup". **`retry` refuses a restored message**
    (`invalid-argument`), so it can never be sent.
  - **A restored group is never synced, relayed or re-keyed.** The worker
    loops (sync, relay, scheduled re-key, outbox) skip restored
    conversations. Imported `group_members` rows are for display only.
  - `group-info` gains `restored: bool`. It replaces Roym's "restored only"
    rule (today: Roym holds the row, the host says `not-found`).
- **Direct chats.** A direct conversation id is a hash of both addresses, and
  today the store allows one direct chat per peer
  (`idx_conversations_direct_peer`):
  - If the restored node has the same address as before (its member master
    key was restored), the imported conversation has the same id. Its rows
    merge in, and duplicate message ids are skipped.
  - If the address changed, the imported chat has a different id. It is
    imported as a separate, read-only restored conversation. The unique index
    becomes "one *live* direct chat per peer" (`WHERE kind = 'direct' AND
    restored = 0`).
  - `open-direct(peer)` always returns or creates the live chat, never a
    restored one. The Hub shows the restored chat as old history.
- The format is the host's own, with a version number. Roym's signed bundle
  carries it as one section.
- Out of scope: continuing an old chat after a move to a new machine (backlog
  row "Conversations cannot continue after a substrate moves"). This plan
  restores *history* only, as today.

**D-CV-8: Summaries carry the message count; only accepted messages move a
chat up the list.**
- `conversation-summary` gains `message-count` (accepted and own sent
  messages, not system).
- `last-activity-at` is the host's local time of the last **visible** change:
  an own sent message, or an incoming message becoming `accepted`. A pending,
  held or dropped message does not update it. Today the incoming insert
  updates it on every message (`store/message.rs`,
  `insert_incoming_if_absent`); that update moves to the accept step. So a
  blocked sender cannot push their chat to the top of the list.
- **Which chats `conversations()` lists:**
  - every group this service owns or is a member of, and every restored
    group;
  - a direct chat only if this service opened it (`open-direct`), sent in
    it, or has at least one accepted message in it.
  A stranger whose messages are all held or dropped has no visible chat.
- Today Roym uses the newest sender timestamp for activity. The local time
  is simpler and cannot be pushed into the future by a peer's wrong clock.

**D-CV-9: What Roym stores after the switch: one small `admissions`
collection.**
- `admissions`: conversation id → kind and decision (`shown`, `hidden`,
  `refused(reason)`). It answers "has Roym already accepted this chat?"
  (first-contact limit) and "is this group hidden?".
- **Who writes it, and when:**
  - `conversation.open` writes `shown` for the chat it opens, so a reply from
    someone you contacted first is never charged against the first-contact
    limit. (Today `messages::open` creates Roym's conversation row for the
    same reason.)
  - `conversation.send` writes `shown` if no row exists yet.
  - The inbox writes the first-contact decision before it answers the host
    (D-CV-1).
  - `group.create`, the first sight of a group, `group.hide` and
    `group.unhide` write the group's decision.
  - It is introduced and written from H1 onward, beside Roym's old rows, so
    it is complete before R1 starts reading it.
- **Answers per kind:**
  - 1:1 chat, sender blocked → `drop("blocked")`.
  - 1:1 chat, first contact over the limit → `drop("rate-limited")`. Today
    such a message has no way back either; it should not keep a body on disk
    forever.
  - Group hidden, or a group first seen from an owner over the limit →
    `hold("group-hidden" | "rate-limited")`. `group.unhide` calls `readmit`.
    Held bodies expire after the time limit in D-CV-1.
  - Reserved content type, non-owner name change, bad name payload →
    `drop(reason)`.
- Person DIDs are looked up from contacts when a list is built. They are not
  stored.
- Roym's JSON answers keep today's field names (`direction`,
  `body_encoding`, `state`, `deleted_at_secs`, and so on), so the Hub needs
  few changes. The `history` cursor becomes the host's string cursor.
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

**D-CV-11: A feed of newly visible messages, for `roym_transaction`'s card
sync.**
- **Problem.** `roym_transaction/src/app/sync.rs` keeps a number,
  `scanned_count`. It steps back by `SYNC_OVERLAP`, and rewinds to the
  position of the first card it declined. An opaque history cursor cannot do
  either. Worse, under D-CV-1 a message can become visible later: a
  `pending` row is accepted after a re-ask, or `readmit` brings a held row
  back. History sorts by sender timestamp, so such a message lands *behind*
  a position sync has already passed, and sync would skip that card forever.
- **Decision.** The host gives each message a local, increasing number,
  `visible-seq`, at the moment it becomes visible: on insert for an own sent
  message, at the accept step for an incoming one. A readmitted message that
  is accepted later gets a new, higher number.
- New verb `changes(conversation, after-seq: u64, limit) -> (messages,
  last-seq)`: visible messages with `visible-seq > after-seq`, in
  `visible-seq` order.
- `visible-seq` is a number, so sync keeps its current logic: it stores the
  last number it read, steps back by `SYNC_OVERLAP` (re-reading at most that
  many rows, because numbers may have gaps), and rewinds to the number of the
  first declined card.
- Roym exposes it as `conversation.changes`. `roym_transaction` switches from
  `conversation.history` + offset to `conversation.changes` + `visible-seq`,
  and renames `scanned_count` to `last_seq`.
- **Why a separate feed:** `history` answers "show me this chat in order".
  Sync asks "what is new since I last looked". One ordering cannot answer
  both once messages can become visible late.

## 6. Interface changes (WIT sketch)

In `crates/wit_interfaces/wit/conversation/conversation.wit`. Final names are
settled in phase H1–H5 reviews.

```wit
record message {
    // ... existing fields ...
    outgoing: bool,
    deleted-at: option<s64>,
    restored: bool,
    visible-seq: u64,
}

variant admission { accept, hold(string), drop(string) }

record name-event { entry: string, name: string, sender-timestamp: s64 }

variant history-item {
    message(message),
    membership(membership-event),
    group-name(name-event),
}

record history-page { items: list<history-item>, next-cursor: option<string> }

record change-page { messages: list<message>, last-seq: u64 }

record conversation-summary { /* ... */ message-count: u32, name: option<string>, restored: bool }
record group-info { /* ... */ name: option<string>, restored: bool }

record export-chunk { data: list<u8>, next-cursor: option<string> }

// new functions in `interface conversation`
delete-message: func(message: message-id, ask-others: bool) -> result<_, conversation-error>;
readmit: func(conversation: conversation-id, reasons: list<string>) -> result<u32, conversation-error>;
changes: func(conversation: conversation-id, after-seq: u64, limit: u32)
    -> result<change-page, conversation-error>;
search: func(query: string, conversation: option<conversation-id>, limit: u32)
    -> result<list<message>, conversation-error>;
set-group-name: func(conversation: conversation-id, name: string) -> result<_, conversation-error>;
transcript-digest: func(conversation: conversation-id) -> result<string, conversation-error>;
export-history: func(cursor: option<string>) -> result<export-chunk, conversation-error>;
import-history: func(data: list<u8>) -> result<u32, conversation-error>;

// removed in R1b: membership-history (history now carries membership items)

interface guest-api {
    on-message: func(msg: message) -> result<admission, string>;
    on-delivery-state: func(msg: message-id, state: delivery-state) -> result<_, string>;
}
```

In total: four changed records (`message`, `history-page`,
`conversation-summary`, `group-info`), five new types (`admission`,
`name-event`, `history-item`, `change-page`, `export-chunk`), eight new
functions, one removed function, and a changed `on-message` result.

A WIT change touches all of these:

- WIT copies: `crates/wit_interfaces/wit/conversation/` plus each
  `wit/deps/conversation/` under `roym_conversation`, `roym_catalog`,
  `roym_web`, `roym_directory`, `roym_transaction`, `roym_profile`, and
  `test-components/dual-build-fixture`.
- RPC trait and types: `crates/rpc/src/conversation.rs` (`ConversationHost`,
  `ConversationNotifier`).
- Host: `crates/conversation`.
- WASM side: `crates/sandbox_wasm/src/host_capabilities/capabilities_messaging.rs`,
  `crates/sandbox_wasm/src/engine/auth.rs` (reads the answer from
  `on-message`, and reports `NoHandler` when the export is missing).
- Native side: `crates/app_host/src/{lib,guest,types}.rs` (`AppConversation`,
  `ConversationSink`), `crates/app_host_native/src/{host,factory}.rs`
  (the factory declares at registration whether a sink will be set),
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
- `store/schema.rs`: add to `messages`:
  `admission TEXT NOT NULL DEFAULT 'accepted'`, `admission_reason TEXT`,
  `admission_changed_at INTEGER`, `notify_attempts INTEGER`, and
  `next_notify_at INTEGER`.
- New incoming rows start as `pending`, with `next_notify_at` set to the
  claim window (D-CV-1). Outgoing and system rows start as `accepted`.
- All readers (`history`, `get_message`, `outbox_messages`, `message_count`,
  `list_conversations`) filter on `accepted` or `outgoing`.
- `last_activity` updates move from the incoming insert to the accept step
  (D-CV-8). `list_conversations` applies the visibility rule from D-CV-8.
- `transport.rs`, `group.rs`, `transport/group_sync.rs`: after storing, call
  the notifier and store its answer. `NoHandler` → `accepted`; `NoAnswer` →
  the row stays `pending`.
- `outbox.rs`: new `renotify_pending_once` and `expire_held_once`, run from
  `run_worker`. `ConversationConfig` gains `conversation_max_held_age_secs`.
- `readmit` verb.

Interface: `admission` variant, `on-message` result, `readmit`
(all files in §6). `ConversationNotifier::notify_message` returns
`Answered(admission)`, `NoHandler` or `NoAnswer`.
`ConversationSink::on_message` returns `Result<Admission, String>`.
`NativeHostFactory` records at registration whether the service has a sink.

Roym (still keeps its copy in this phase):
- Add the `admissions` collection and write it everywhere D-CV-9 lists,
  beside the old rows.
- `on_message` reads `admissions` first. It returns `accept` after storing or
  ignoring a duplicate, and the answers listed in D-CV-9 otherwise.
- A storage fault or an unavailable profile service returns `Err`, so the
  host asks again.
- `unhide` calls `readmit` instead of reading held messages with
  `get_message`.

Tests:
- Base crate unit tests:
  - pending, accepted, held and dropped rows are visible only when they
    should be
  - `NoAnswer` leaves rows pending; a re-ask after the claim window
    succeeds; the worker does not re-ask inside the window
  - `NoHandler` accepts
  - readmit; held rows expire into `dropped("expired")`
  - a held or dropped message does not change `last_activity` or make a
    direct chat appear in `conversations()`.
- Parity (fixture): the answer travels the same way on WASM and native,
  including `NoHandler` for a component without the export.
- New Roym parity scenarios:
  - a profile outage during delivery does not lose the message
  - a message that arrives before the native sink is set waits and is
    checked against the block list
  - a reply in a chat you opened is not charged against the first-contact
    limit
  - after the H1 switch, nothing in Roym reads a held row through
    `get-message` (a test asserts `get-message` returns `not-found` for a
    held row and every Roym verb still passes).

### H2: message fields, delete, "please delete" request (size M)

- WIT `message`: add `outgoing`, `deleted-at` and `restored` (always false
  until H5).
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

### H3: group events in history, the feed, group name, digest, summary fields (size L)

- `dag.rs`: `EntryKind::Profile` with a signed `name`. Accept only entries
  from the owner whose name passes the check; skip others without changing
  the name. Apply rule: newest by (sender timestamp, author, entry id).
- `set-group-name` verb, with the name check moved from
  `roym_core::conversation::group::validate_group_name`.
- `history`: merge `messages` and the membership/profile log entries in one
  ordered read (a `UNION ... ORDER BY sender_timestamp, author, id`). The
  cursor encodes the last item's ordering key.
- `visible-seq` column and a store-level counter. Set on insert for own sent
  messages and at the accept step for incoming ones. `changes` verb (D-CV-11).
- `transcript-digest` verb over every non-system row and every log entry
  (D-CV-4). `group-info` gains `name`; `conversation-summary` gains
  `message-count` and `name`.
- Tests:
  - unit: ordering across both tables; cursor paging across both
  - unit: a message accepted after a re-ask, or readmitted, appears in
    `changes` after the last number already read
  - unit: an incoming `profile` entry with a bad name or a non-owner author
    is not applied
  - cross-node: a new member sees the name without a resend; two members get
    the same digest after a membership change, and still the same digest
    when one of them dropped a message from a blocked author.

### H4: search (size M)

- A unit test that creates a `trigram` FTS5 table, guarding the compile flag
  (D-CV-6).
- A `trigram` FTS5 table over text bodies, kept in step by the same
  transactions that insert, delete or change admission.
- `search` verb. Queries of 3+ characters use FTS5, passed as a quoted
  phrase so special characters are searched for literally (today's
  `escape_regex` rule). Shorter queries use the `LIKE` fallback.
- Tests: unit tests for substring matching ("ell" finds "hello"), the
  short-query fallback, held/dropped/deleted rows not found, card JSON found,
  and the conversation filter.

### H5: export and import (size L)

- Versioned export format and paged `export-history` / `import-history`.
- `restored` flag on conversations and messages; `verified = false` on
  restored messages; `retry` refuses restored messages.
- Worker loops (outbox, sync, relay, scheduled re-key) skip restored
  conversations.
- Direct-chat rule from D-CV-7: same id merges; a different id becomes a
  separate restored chat; the unique index covers live chats only;
  `open-direct` never returns a restored chat.
- `group-info.restored` and `conversation-summary.restored`.
- Tests:
  - unit: round trip on one store; pending messages come back as `failed`
    with the "restored" reason, and `retry` refuses them; restored messages
    have `verified = false`
  - unit: import into a node that already has a live chat with the same
    peer (same id merges; different id stays separate and read-only)
  - cross-node: restore on a clean node shows the same history and digest;
    no sync, relay or re-key traffic is sent for a restored group.

### R1: Roym switches to the host (size L, two PRs)

- **R1a, all reads and writes in one PR.** Switching reads and writes
  separately would leave `main` broken: a message deleted in Roym's old copy
  would come back in host-backed history and search.
  - `history`, `search`, `list`, `group.info` and `transcript-digest` read
    from the host and map the result into today's JSON fields. `list` joins
    host summaries with `admissions` (written since H1) and a contacts
    lookup.
  - New `conversation.changes` verb. `roym_transaction/src/app/sync.rs`
    switches to it (D-CV-11).
  - `delete-message` → host `delete-message(ask_others)`.
  - `group.rename` → `set-group-name`.
  - `export`/`import` → Roym bundle with the host export section plus
    `admissions`.
  - `send` keeps the content-type gate (cards only in 1:1; reserved types)
    and calls the host.
  - Error mapping goes through one `from_host(ConversationError) -> Response`
    table.
  - `TRANSCRIPT_CHECK_NOTICE` in `roym_core` and its verbatim copy in
    `crates/roym_web/ui/src/groups/words.ts` change together (a test
    compares them). Decided in §9 question 1.
  - Roym still writes its old copy in this PR, but nothing reads it.
- **R1b, removal.**
  - Delete the `messages`, `conversations` and `refused_messages`
    collections, and every writer of them.
  - Delete the `on-delivery-state` export, `sync_membership_rows`, the group
    adoption code and the message counter code.
  - Remove `membership-history` from the WIT (D-CV-4).
  - Shrink `roym_core::conversation`: keep `encode_body`, `Direction` and the
    JSON row type used for the answers. Remove `GroupMeta.name_source` and
    `membership_events_copied`.
  - Bump the backup bundle's schema version.

Tests to rewrite (they test the old copy's internals):
- `roym_web/tests/dual_build_parity/conversation.rs`: scenarios 56 (delivery
  state), 60, 62 (deletion request) and 63–65 (export/import).
- `group.rs` and `group_lifecycle.rs`: 201–215, especially 210 (hide/unhide),
  211 (search), 212 (restore), 213 (adoption) and 214.
- `roym_web/tests/dual_build_parity/transaction*.rs` and
  `substrate/tests/roym_transaction_e2e.rs`: card sync through
  `conversation.changes`, including a card that becomes visible late.
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
  "each message is stored twice". Say that delete and drop remove the
  readable body on this machine, and that a group's encrypted log entry and
  key remain.
- `profile.policy` text and the Hub Backup screen: remove the "two copies"
  statement.
- `docs/planning/deferred-backlog.md`:
  - move to "Recently resolved": "`conversation.history` does not reconcile
    Roym's copy", "Roym's own copy ... doubles the on-disk cost",
    "`conversation.search` is a `$regex` scan", and the inbound-filter
    (admit hook) row
  - add rows for anything left open in H4 or H5 (for example export size
    limits).

## 8. Order and size

H1 → H2 → H3 → H4 → H5 → R1a → R1b → R2. H2, H3 and H4 do not depend on each
other and can run in parallel after H1. H5 depends on H2 and H3, because it
exports deleted rows and log entries. Sizes are relative: S < M < L.

A separate change is moving the data-layer page loop and backup helpers
into `roym_core`. If it lands first, `roym_conversation` uses those helpers
in H1 and R1 (for the `admissions` collection and the backup bundle). R1b
deletes most of the page loops it would have touched in this crate. Neither
change blocks the other.

## 9. Open questions (recommendation first)

1. **Transcript digest scope — decided 2026-10-01:** count every non-system
   row whatever its local admission, so blocking no longer changes a
   member's code (D-CV-4). The product notice changes with it in R1a.
2. **Group message delete and drop:** keep the encrypted log entry so other
   members can still sync it, and say so in the product. *Recommended.* The
   alternative, removing the encrypted body from the log, would stop serving
   it to members who have not received it yet.
3. **Honouring a "please delete" request:** the receiving host does it
   automatically, author-only. *Recommended.* The alternative is to pass it
   to the app as a new callback.
4. **Last activity time:** the host's local clock, visible changes only
   (D-CV-8). *Recommended.*
5. **Export format:** the host's own versioned format inside Roym's signed
   bundle (D-CV-7). *Recommended.* D-06C-5 worried that Roym's export would
   depend on a host format. A version number in the host format, plus Roym's
   own signature and manifest, covers that.
6. **1:1 first-contact over the limit:** `drop`, not `hold` (D-CV-9).
   *Recommended*, because there is no way back for it today. If a later
   "accept this person" action is wanted, switch to `hold` then.
7. **Held-body time limit:** 30 days by default (D-CV-1). *Recommended.*

## 10. Risks

- **Size of the WIT change.** Each WIT change touches about 15 files across
  8 crates, plus 8 WIT copies. Keep each phase's WIT change small, and land
  it in the same PR as its host code.
- **Re-asking forever.** If an app always fails to answer, its messages stay
  `pending` and invisible. This is safer than losing them, but it must be
  visible: add a metric and a warning after N attempts.
- **Answer during delivery delays the sender's receipt.** Today the inline
  notify is already awaited before the delivery receipt is sent
  (`transport.rs`). With answers, a slow app adds its time to that wait. The
  claim window bounds it: if the app does not answer in time, the receipt is
  sent and the worker asks later.
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
check that could miss membership changes. It also doubled disk use and left
"deleted" messages readable in the host store. Every future chat SynApp would
face the same gaps.

**Decision.** The conversation capability provides:
- a durable app answer for each incoming message (accept, hold, drop), asked
  again until answered, with readmit and a time limit on held bodies; an app
  with no inbox declares it, and only then are its messages accepted
  without asking
- local delete and an author-only "please delete" request
- group events and owner-signed group names inside history, and a
  transcript digest over the whole synced log
- a feed of newly visible messages in local order
- full-text search with substring matching
- paged, versioned export and import of history; imported rows are marked
  restored, are not verified, and are never sent, synced or relayed.

Apps keep only their policy (who to accept) and their product data.

**Consequences.**
- One store per service, and no app-side copy.
- Deleting or dropping removes the readable body from the host store. For a
  group, the encrypted log entry and the group key remain on the node.
- Restore of history is a capability feature, not an app feature.
- The WIT interface changes four records, adds five types and eight
  functions, removes `membership-history`, and changes the `on-message`
  result.
- Continuing a conversation after a move to a new machine is still open. It
  has its own backlog row.

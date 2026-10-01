# Conversation consolidation: one message store, in the base crate

Status: **proposed** (2026-10-01; revised twice the same day after review).
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
| A refused first contact is silent to the sender | The host stores and acknowledges a message before Roym's inbox runs, so the sender never learns it was rate-limited (M06C failure-matrix row 12; backlog §5 "no inbound admit/reject hook"). |
| Repeated, slightly different code | Group refresh at 9 call sites, ad-hoc error mapping at 21 places, "is this a group?" decided 6 ways. |

**How this plan closes the two gaps.**

| Gap (M06C `task.md`) | Closed by | What stays open, on purpose |
|---|---|---|
| Gap 3: history cannot be written back (no import) or removed (no delete) | H2 `delete-message`; H5 `export-history` / `import-history` | Continuing an old chat after a move to a new machine. It has its own backlog row. A restored group stays read-only (D-CV-7). |
| Gap 4: no block list and no way to refuse; `on-message` is only a notice after storing | H1: the host asks the app (accept / hold / drop), shows nothing until it is accepted, erases the readable body on `drop`, and asks again until it gets an answer. A rate-limit refusal is reported back to the sender (D-CV-12) | (1) The block list and the first-contact limit stay in Roym, because they are Roym data from its profile service; the host gives the refuse mechanism. (2) The host still writes the message to disk before asking, so it can survive a crash. (3) For a group message, the encrypted log entry and the group key stay on the node after a `drop` (D-CV-1). (4) A block is never reported to the sender (D-06C-8). |

**Correction to the first analysis.** An incoming card in a *group* is stored
and shown as a neutral block on purpose (decision D-C10-11, question Q8). It is
not a bug. Only *sending* a card into a group is refused.

## 3. Decisions this plan replaces or changes

| Decision | What it said | What happens now |
|---|---|---|
| **D-06C-5** (M06C `task.md`) | Roym keeps its own copy of every message. Export, search, delete and restore act on it. Adding import and delete to the host was rejected as "more substrate work for a problem the app can solve". | **Replaced.** The host gains import, export, delete and search. Roym's signed backup includes the host's export. |
| **D-06C-8** (M06C `task.md`) | Block is enforced only in Roym's inbox; the blocked message stays in the host store. A host admit hook was put off until "a second app wants inbound filtering". | **Changed.** The host gains the mechanism (the app answers accept / hold / drop). Roym keeps the policy (who is blocked, the first-contact limit). A block stays invisible to the sender. |
| **D-C5-7** (slice C5 plan) | Roym's copy stores full bodies; disk cost is 2×, and the product says so. | **Replaced.** No second copy. |
| **D-C5-10** (slice C5 plan) | Delivery state is never Roym's to invent; `history` re-asks the host for every row not yet delivered. | **Rule kept, mechanism removed.** Roym reads the state straight from the host. Nothing is copied, so nothing can go stale. |
| **D-C5-11** (slice C5 plan) | A refused message goes into `refused_messages`, body dropped, invisible everywhere. Block is checked on every message; the first-contact limit only when no admitted chat exists. | **Changed.** The host hides held and dropped messages from every read. Both checking rules stay as Roym policy, unchanged (D-CV-9). |
| **D-C10-4** (slice C10 plan) | The group name is the newest owner-sent `group-profile` message in Roym's copy. | **Replaced.** The name is an owner-signed entry in the host's group log (D-CV-5). |
| **Transcript check notice** (`TRANSCRIPT_CHECK_NOTICE`, slice C10) | "A member who blocked someone in the group sees a different code." | **Changed (decided 2026-10-01, §9 question 1).** The code covers every entry in the group log, whatever each member's local answer was. It proves "we hold the same log", and blocking no longer changes it (D-CV-4). |
| **M06C failure-matrix row 12** ("refusal is visible to the sender") | Not met for an inbound refusal: C5 did not claim it. | **Closed by D-CV-12** for rate-limit refusals. A block is still never reported. |

These are recorded decisions, so the change needs an ADR: ADR-0025 (draft in
the appendix). It builds on ADR-0013 §6, which already says the substrate
handles the core protocol and chat apps are light wrappers over it. This plan
moves storage features into that core layer, where §6 places them.

## 4. Target split

| Responsibility | Lives in |
|---|---|
| Message storage, ordering, delivery state, outbox, retry | Base crate (already there) |
| App answer to a new message: accept / hold / drop, asked again until answered; bring held messages back later; time limit on held bodies | Base crate (new) |
| Report a refusal back to the sender when the app asks for it | Base crate (new) |
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
- Each incoming message row gets an `admission` value: `undecided`,
  `accepted`, `held(reason)` or `dropped(reason)`. The name `undecided` is
  chosen so it never clashes with the delivery state `pending`.
- `guest-api.on-message` returns an answer: `accept`, `hold(reason)` or
  `drop(reason, report)` (see D-CV-12 for `report`).
- **Three outcomes, not two.** The notifier returns `Answered(admission)`,
  `NoHandler` or `NoAnswer`:
  - `NoHandler` means the app *declares* it has no inbox: a WASM component
    whose exports do not include `guest-api.on-message`, or a native service
    registered without a conversation sink. It comes from what the app
    exports or registers, never from a value that is simply not set yet. The
    host treats `NoHandler` as `accept`.
  - `NoAnswer` covers everything else: the call failed, crashed or timed
    out; the default notifier is not wired yet at boot; or a declared native
    sink is not set yet. The row stays `undecided`. Messages that arrive
    during startup therefore wait for Roym; they never skip its block list.
  - Base-crate tests that need messages to become visible install a test
    notifier that answers `accept`. A test with no notifier checks that rows
    stay `undecided`.
- **Ask once per message, never per delivery.** `insert_incoming_if_absent`
  already returns whether the row is new; today `transport.rs` ignores it.
  The host asks during delivery **only on a fresh insert**. A repeat
  delivery of the same message (the sender's retry) never asks again; the
  worker handles any row still `undecided`.
- **Two timers, kept separate.**
  - The ask during delivery has its own short timeout,
    `admission_ask_timeout_ms` (default 3 seconds). It stays well under the
    sender's call timeout (`DEFAULT_PROXY_CALL_TIMEOUT`, 30 seconds), so a
    slow app never makes the sender time out and retry. If it expires, the
    outcome is `NoAnswer` and the delivery receipt is sent.
  - A claim window: the row is inserted with `next_notify_at` set a little
    beyond the ask timeout (default 10 seconds). The worker re-asks only rows
    whose `next_notify_at` has passed, then moves it forward with growing
    gaps (the `backoff_for_age` curve, capped at 5 minutes). So the delivery
    ask and the worker never ask about the same row at the same time.
- **The app's answer must be safe to repeat** for the same message id: a
  crash between "app decided" and "host stored the answer" causes a second
  ask. D-CV-9 says how Roym makes this safe without caching block or
  rate-limit decisions per chat.
- `history`, `get-message`, `search`, counts, the feed (D-CV-11) and
  `outbox` show only `accepted` rows and the service's own sent messages.
  The transcript digest is the one exception (D-CV-4).
- `drop` empties the readable body but keeps the row (id, author, timestamp,
  content type), so a repeat delivery is still recognised and not asked
  about again. **For a group message, the encrypted entry in the group log and
  the group key stay on the node**, because other members sync from that
  entry. The node could still decrypt it. The product says this plainly.
- New verb `readmit(conversation, reasons)`: sets matching `held` rows back
  to `undecided`, so they are asked about again. Roym's "unhide" uses it.
- **Held bodies have a time limit.** A row held longer than
  `max_held_age_secs` (new `ConversationConfig` field, default 30 days)
  becomes `dropped("expired")`. Roym's hidden-group notice says that a
  group hidden for longer than this loses those messages when it is shown
  again.
- **Why:** this fixes lost messages for every app, not only Roym. It also
  makes block real at the store level: a blocked person's message never
  becomes readable through any verb.

**D-CV-2: The WIT `message` record gains `outgoing: bool`,
`deleted-at: option<s64>`, `restored: bool`, `visible-seq: u64` and
`refused: option<string>`.**
- The host already stores `outgoing`; it was not exposed. Roym needs it for
  its `direction` field, because no host verb tells a service its own address.
- `restored` marks rows that came from `import-history` (D-CV-7).
- `refused` is set on an *own sent* message when the recipient reported a
  refusal (D-CV-12).
- The WIT doc for `verified` changes to: true for every row this node sent or
  received itself; **false for restored rows**, whose keys were not restored
  and whose signatures cannot be checked again.

**D-CV-3: Delete and the "please delete" request move to the host, and the
deleted text leaves the file.**
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
- **Freed pages are wiped.** SQLite normally leaves the old bytes of a
  deleted value in free pages, readable by anyone who holds the database key
  (the DEK, which the node holds). `conversation.db` is opened with
  `PRAGMA secure_delete = ON`, so freed pages are overwritten with zeros.
- **The WAL file is wiped in batches.** A plain checkpoint copies WAL frames
  into the database but leaves the old frames in the WAL file until they are
  reused. So the host runs `PRAGMA wal_checkpoint(TRUNCATE)`, which also
  truncates the WAL file to zero length. It does not run after every
  operation: the outbox worker runs it at most once per tick, and only when
  a delete or drop happened since the last one. `TRUNCATE` can complete only
  when no reader is still using an old part of the WAL; if it cannot, it
  reports "busy" and the worker tries again on the next tick. So an emptied
  body normally remains in the WAL file for one worker tick, and longer only
  while a long read is open.
- **Cost.** Deletes are rare, but **drops are not**: every blocked,
  rate-limited or expired message is a drop, and a flood of them is the case
  this feature exists for. `secure_delete` adds a page overwrite to each
  drop. The batched checkpoint keeps the WAL cost at one truncate per tick,
  however many drops happened. H2 measures both under a flood of drops.
- The same applies to a `drop` (D-CV-1) and to the search index (D-CV-6).

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
  (undecided, accepted, held or dropped), plus deleted rows, plus **every
  entry stored in the group log**. The host keeps those four fields even
  after a drop, so this is always possible.
- **Which log entries count:** an entry that fails the log's own checks
  (bad signature, or a membership or name entry not signed by the owner) is
  never stored, on any member, so it never counts. An owner-signed name entry
  whose name fails the name check *is* stored but not applied (D-CV-5); it
  counts. Every member makes the same choice for the same entry, so the code
  stays equal.
- **Why:** the code then depends only on the synced log, not on each
  member's local rules. Two members with the same log always see the same
  code, even if one of them blocked someone. (Decided 2026-10-01.)

**D-CV-5: The group name is an owner-signed entry in the group log.**
- New log entry kind `profile`, carrying `name`. It is signed by the owner and
  not encrypted, the same as membership entries, so a new member can read it
  at once.
- New verb `set-group-name(conversation, name)`: owner only. The host checks
  the name: 1–80 characters after trimming, no control characters (today's
  `validate_group_name`).
- **The same check runs when an incoming `profile` entry is applied.** An
  entry not signed by the owner is refused by the log and not stored. An
  owner-signed entry with a bad name is stored but not applied, and the name
  does not change. This stops a misbehaving owner client from sending a huge
  name.
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
- **External content.** The FTS table is created with
  `content='messages', content_rowid='rowid'`, so the message text is not
  stored a third time; only the index is. A `trigram` index is still several
  times the size of the text it covers, and the product's storage notice
  says so.
- With external content, the FTS row must be removed (the FTS5 `'delete'`
  command, which needs the old text) **before** the body is emptied by a
  delete or a drop. Both steps run in the same transaction.
  `secure_delete` (D-CV-3) wipes the freed index pages too.
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
  session keys or group epoch keys, and it does not hold `visible-seq`.
- **Restored rows are history only.** Imported messages and conversations
  carry a `restored` flag:
  - Restored messages have `verified = false` (D-CV-2). The host does not
    trust the bundle: an app could put any author in it.
  - A restored own message whose delivery state was still `pending` becomes
    delivery state `failed`, with the reason "restored from a backup".
    **`retry` refuses a restored message** (`invalid-argument`), so it can
    never be sent.
  - `group-info` gains `restored: bool`. It replaces Roym's "restored only"
    rule (today: Roym holds the row, the host says `not-found`).
- **Direct chats.** A direct conversation id is a hash of both addresses, and
  today the store allows one direct chat per peer
  (`idx_conversations_direct_peer`):
  - If the restored node has the same address as before (its member master
    key was restored), the imported conversation has the same id. Its rows
    merge in, duplicate message ids are skipped, and the chat stays live.
  - If the address changed, the imported chat has a different id. It is
    imported as a separate, read-only restored conversation. The unique index
    becomes "one *live* direct chat per peer"
    (`WHERE kind = 'direct' AND restored = 0`). **The direct-chat upsert's
    `ON CONFLICT(peer_address) WHERE kind = 'direct'` clause
    (`store/message.rs`, `insert_incoming_if_absent`) and the same clause in
    `get_or_create_direct` change in the same commit**, because SQLite
    refuses an upsert whose conflict target does not match a unique index.
  - `open-direct(peer)` always returns or creates the live chat, never a
    restored one. The Hub shows the restored chat as old history.
- **Groups.**
  - If the importing node already holds that group live (it is a current
    member, with keys), the imported rows merge in, duplicates are skipped,
    and the group is **not** marked `restored`. It keeps syncing.
  - If the node does not hold the group, it is created as a `restored`
    group. A restored group is never synced, relayed or re-keyed: the worker
    loops (sync, relay, scheduled re-key, outbox) skip restored
    conversations, and imported `group_members` rows are for display only.
  - So after a same-address restore, direct chats continue but groups stay
    read-only. That is the intended scope: continuing a group needs its epoch
    keys, which are never exported. It stays part of the backlog row
    "Conversations cannot continue after a substrate moves".
- **Sequence numbers.** Imported rows get new `visible-seq` numbers, in
  import order (D-CV-11). Numbers from the old node mean nothing on the new
  one. `roym_transaction`'s import **does not restore its card-sync position**;
  card sync starts again from 0 after a restore. Card filing already skips
  duplicates (parity 134, "sync is idempotent"), so the rescan is safe.
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
  an own sent message, or an incoming message becoming `accepted`. An
  undecided, held or dropped message does not update it. Today the incoming
  insert updates it on every message (`store/message.rs`,
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

**D-CV-9: What Roym stores after the switch, and how its answers stay
correct when asked twice.**
- **`admissions`**: conversation id → kind and decision. It records only
  two kinds of fact, both stable choices:
  - "this chat is accepted": set when `conversation.open` opens it, when
    `conversation.send` first sends in it, or when a first contact passes the
    limit. It answers one question: *should the first-contact limit apply to
    this chat?*
  - a group's visibility: `shown` or `hidden` (`group.create`, first sight of
    a group, `group.hide`, `group.unhide`).
  It never stores a block decision or a 1:1 rate-limit refusal.
- **Block is checked live on every message**, whatever `admissions` says
  (today's D-C5-11 rule). Blocking someone in an existing chat takes effect
  at once, and unblocking takes effect for their next message.
- **First-contact limit, charged once per message.** When a message arrives
  in a chat that is not accepted, Roym calls `contacts.admit-first-contact`.
  It first writes the answer to a small `first_contact_charges` collection,
  keyed by **message id**, then answers the host. A re-ask for the *same*
  message reuses that stored answer instead of charging the limit again. A
  *new* message from the same stranger gets a fresh check, so a stranger who
  was over the limit gets a new chance once the limit refills (today's
  behaviour). Rows older than the limit's own window are pruned.
- **Answers per case:**
  - Sender blocked, in an accepted 1:1 chat or in a shown group →
    `drop("blocked", report = false)`.
  - 1:1 first contact (chat not accepted), blocked or not: the limit is
    charged per message. Over the limit →
    `drop("rate-limited", report = true)` (D-CV-12). Under the limit →
    accept if not blocked, `drop("blocked", report = false)` if blocked.
    This is what stops a blocked stranger from detecting the block
    (D-CV-12). A rate-limited message keeps no body on disk; the sender can
    try again later.
  - Group hidden → `hold("group-hidden")`. `group.unhide` calls `readmit`.
  - A group first seen from an owner over the limit: the group's visibility
    becomes `hidden` (today's per-group decision, kept), and its messages are
    `hold("group-hidden")`. The person can show it later. Held bodies expire
    after the time limit in D-CV-1.
  - **A group whose owner you blocked.** Today such a group is refused as a
    whole when first seen (`GroupAdmission::Refused { blocked }`), and the
    refusal is permanent. New behaviour, stated here because it changes:
    - A group first seen whose owner is blocked gets visibility `hidden`,
      a visibility choice and not a stored block decision. So, as today,
      nothing from it shows by default; unlike today, the person can show it
      later, for example after unblocking the owner.
    - In a group that is shown, only the blocked person's own messages are
      dropped (`drop("blocked", report = false)`), checked live per message.
      Other members' messages are accepted. This also applies when you block
      the owner of a group you already show: the group stays shown.
  - Reserved content type, non-owner name change, bad name payload →
    `drop(reason, report = false)`.
- Person DIDs are looked up from contacts when a list is built. They are not
  stored.
- Roym's JSON answers keep today's field names (`direction`,
  `body_encoding`, `state`, `deleted_at_secs`, and so on), so the Hub needs
  few changes. The `history` cursor becomes the host's string cursor, and
  sent messages gain `refused`.
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
  either. Worse, under D-CV-1 a message can become visible later: an
  `undecided` row is accepted after a re-ask, or `readmit` brings a held row
  back. History sorts by sender timestamp, so such a message lands *behind*
  a position sync has already passed, and sync would skip that card forever.
- **Decision.** The host gives each message a local, increasing number,
  `visible-seq`, at the moment it becomes visible: on insert for an own sent
  message, at the accept step for an incoming one. A readmitted message that
  is accepted later gets a new, higher number. The numbers are local to one
  store and are not exported (D-CV-7).
- New verb `changes(conversation, after-seq: u64, limit) -> (messages,
  last-seq)`: visible messages with `visible-seq > after-seq`, in
  `visible-seq` order.
- Sync stores the last number it read, and rewinds to the number of the first
  declined card. It resumes exactly, so **`SYNC_OVERLAP` is removed**: it
  existed only because an offset into a sorted list could shift.
- Roym exposes it as `conversation.changes`. `roym_transaction` switches from
  `conversation.history` + offset to `conversation.changes` + `visible-seq`,
  and renames `scanned_count` to `last_seq`.
- **Why a separate feed:** `history` answers "show me this chat in order".
  Sync asks "what is new since I last looked". One ordering cannot answer
  both once messages can become visible late.

**D-CV-12: A rate-limit refusal is reported back to the sender; a block never
is.**
- **Problem.** M06C failure-matrix row 12 requires that a refusal for
  flooding or a contact limit is "visible to the sender, not silent". Today
  the sender's message just shows `delivered`.
- **Decision.** `drop(reason, report: bool)`. When `report` is true:
  - If the answer arrives within the delivery ask (D-CV-1), the host puts
    the reason in the delivery receipt (`DeliveryAck` gains
    `refused: option<string>`).
  - If the answer comes later (the worker's re-ask), the receiving host sends
    the sender a reserved system message carrying the message id and the
    reason. System messages are never shown or counted.
  - The sending host stores the reason on its own message (`refused` in
    D-CV-2). The delivery state stays `delivered`: the message did arrive.
    Roym shows the reason in plain words ("not accepted: this person limits
    messages from new contacts; try again later").
- **A block is never reported** (`report = false`), so the product keeps
  D-06C-8's promise: it never tells a blocked person they were blocked, and
  never claims the sender was prevented from sending.
- **A blocked sender must not be able to detect the block by flooding.** If
  blocked strangers never got a rate-limit report, a stranger could send
  until a report "should" appear and learn they are blocked when none comes.
  So in a chat that is not yet accepted, Roym applies the first-contact
  limit to a blocked stranger exactly as to any other stranger: it charges
  the limit for each message (once per message id), and when the limit is
  exceeded it answers `drop("rate-limited", report = true)`. Under the limit
  it answers `drop("blocked", report = false)`. From the sender's side this
  looks the same as an unblocked stranger: silence under the limit, a
  rate-limit report over it. In an accepted chat no message is ever
  reported, so a block there is silent like everything else.
- **Only the chat's peer can refuse your message.** The sending host stores
  a refusal only when all of these hold:
  - it arrives in the delivery receipt for that message, on the connection
    to that peer; or it arrives as a refusal notice whose transport-verified,
    session-authenticated author is the direct chat's own peer address;
  - the named message is in that same direct chat, and is this service's own
    sent message;
  - the chat is a direct chat. A refusal notice in a group, or naming a
    message in another chat, is ignored.
- **A lost receipt does not lose the refusal.** The receiving host stores the
  answer on the row, including the `report` flag and the reason. A repeat
  delivery of a message whose row was dropped with `report = true` (the
  sender's retry after a lost receipt) gets the same `refused` in its
  receipt, without asking the app again. A refusal notice sent after a late
  answer goes through the outbox, so it is retried until delivered, like any
  message.
- **Group messages are never reported, and the host enforces it.** A group
  message reaches many members, and a per-member refusal would reveal each
  member's local rules. The host ignores `report = true` in an answer for a
  group message and stores it as `false`; this does not depend on the app
  getting it right.
- **The refusal notice is invisible to chat bookkeeping.** It is a system
  message, like the delete request. Sending one does not count as "sent in
  it" for D-CV-8's visibility rule, does not update `last-activity-at`, and
  does not make a direct chat appear in `conversations()`. (The host already
  marks a conversation `system` when its only messages are system messages;
  that rule keeps applying.)
- **Why:** this closes row 12 for the case it names (contact limits), with no
  new information for a blocked person. The refusal reveals only that the
  recipient limits new contacts, which is what the row asks the product to
  say.

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
    refused: option<string>,
}

variant admission {
    accept,
    hold(string),
    drop(drop-answer),
}
record drop-answer { reason: string, report: bool }

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
`conversation-summary`, `group-info`), six new types (`admission`,
`drop-answer`, `name-event`, `history-item`, `change-page`, `export-chunk`),
eight new functions, one removed function, and a changed `on-message`
result. The peer-facing `DeliveryAck` (not WIT; `crates/conversation`) gains
`refused`.

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
  `on-message`, applies the ask timeout, and reports `NoHandler` when the
  export is missing).
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

### H1: admission answers and refusal reports (size L)

Base crate:
- `store/schema.rs`: add to `messages`:
  `admission TEXT NOT NULL DEFAULT 'accepted'`, `admission_reason TEXT`,
  `admission_changed_at INTEGER`, `notify_attempts INTEGER`,
  `next_notify_at INTEGER`, and `refused TEXT`.
- New incoming rows start as `undecided`, with `next_notify_at` set to the
  claim window (D-CV-1). Outgoing and system rows start as `accepted`.
- `transport.rs`, `group.rs`, `transport/group_sync.rs`: read the `bool` from
  `insert_incoming_if_absent` and ask **only on a fresh insert**, under
  `admission_ask_timeout_ms`. Store the answer. `NoHandler` → `accepted`;
  `NoAnswer` → the row stays `undecided`.
- All readers (`history`, `get_message`, `outbox_messages`, `message_count`,
  `list_conversations`) filter on `accepted` or `outgoing`.
- `last_activity` updates move from the incoming insert to the accept step
  (D-CV-8). `list_conversations` applies the visibility rule from D-CV-8.
- Refusal reports (D-CV-12):
  - `messages` gains `report_refusal INTEGER` beside `admission_reason`, so
    the answer's `report` flag is stored on the row.
  - `DeliveryAck.refused`, filled for *every* delivery of a row dropped with
    `report_refusal = 1`, including a repeat delivery.
  - The reserved refusal-notice system message, sent through the outbox
    after a late answer.
  - The host stores `report = false` for any group message, whatever the
    app answered.
  - The sending host stores `refused` only from the chat's own peer, only on
    its own sent message, only in a direct chat.
  - Refusal notices do not count for D-CV-8's visibility rule or
    `last-activity-at`.
- `outbox.rs`: new `renotify_undecided_once` and `expire_held_once`, run from
  `run_worker`. `ConversationConfig` gains `max_held_age_secs`,
  `admission_ask_timeout_ms` and `admission_claim_secs`.
- `readmit` verb.

Interface: `admission` variant, `on-message` result, `readmit`, the new
`message` field `refused` (all files in §6).
`ConversationNotifier::notify_message` returns `Answered(admission)`,
`NoHandler` or `NoAnswer`. `ConversationSink::on_message` returns
`Result<Admission, String>`. `NativeHostFactory` records at registration
whether the service has a sink.

Roym (still keeps its copy in this phase):
- Add the `admissions` and `first_contact_charges` collections and write them
  as D-CV-9 says, beside the old rows.
- `on_message` checks block live, consults `admissions` only for "is this
  chat accepted", charges the first-contact limit once per message id, and
  returns the answers listed in D-CV-9. It still stores accepted messages in
  its old copy.
- A storage fault or an unavailable profile service returns `Err`, so the
  host asks again.
- `unhide` calls `readmit` instead of reading held messages with
  `get_message`. The hidden-group notice gains the time-limit sentence.

Tests:
- Base crate unit tests:
  - undecided, accepted, held and dropped rows are visible only when they
    should be
  - a repeat delivery of the same message does not ask again
  - `NoAnswer` leaves rows undecided; the worker does not re-ask inside the
    claim window; a re-ask after it succeeds
  - the delivery ask stops at `admission_ask_timeout_ms` and the receipt is
    still sent
  - `NoHandler` accepts
  - readmit; held rows expire into `dropped("expired")`
  - a held or dropped message does not change `last_activity` or make a
    direct chat appear in `conversations()`.
- Cross-node (`transport/tests.rs`):
  - a `drop(report = true)` reaches the sender in the receipt and, when
    answered late, as a later notice; a `drop(report = false)` never does
  - with the first receipt lost, the sender's retry still receives `refused`
  - a refusal notice from a third party, for a message in another chat, or
    in a group, is ignored by the sending host
  - `report = true` on a group message is stored as `false`
  - sending a refusal notice to a stranger does not make that chat appear in
    the receiving node's `conversations()`.
- Parity (fixture): the answer travels the same way on WASM and native,
  including `NoHandler` for a component without the export.
- New Roym parity scenarios:
  - a profile outage during delivery does not lose the message
  - a message that arrives before the native sink is set waits and is
    checked against the block list
  - a reply in a chat you opened is not charged against the first-contact
    limit
  - blocking someone in an existing chat refuses their next message;
    unblocking accepts the one after
  - a re-ask for the same message does not charge the first-contact limit
    twice; a stranger refused for the limit is accepted after it refills
  - a rate-limited first contact shows the refusal on the sender's side; a
    blocked sender in an accepted chat sees nothing (M06C failure-matrix
    rows 11 and 12)
  - a blocked stranger flooding first contacts gets the rate-limit report at
    the same message count as an unblocked stranger, so the block cannot be
    detected
  - a group first seen from a blocked owner is hidden, and can be shown;
    in a shown group only the blocked person's messages are dropped
  - after the H1 switch, nothing in Roym reads a held row through
    `get-message` (a test asserts `get-message` returns `not-found` for a
    held row and every Roym verb still passes).

### H2: message fields, delete, "please delete" request (size M)

- WIT `message`: add `outgoing`, `deleted-at` and `restored` (always false
  until H5).
- Store: `deleted_at` column; `delete_message` keeps the row and empties the
  body. `PRAGMA secure_delete = ON` at open. The outbox worker runs
  `wal_checkpoint(TRUNCATE)` at most once per tick, only after a delete or
  drop, and retries next tick on "busy" (D-CV-3).
- Measure `secure_delete` and the batched checkpoint under a flood of drops
  (for example 10,000 messages from a blocked sender), and record the cost.
- Reserved system content type for the delete request. The receiving side
  honours it in `transport.rs` and in group entry apply, using the
  same-author rule from D-CV-3.
- Tests:
  - unit: a deleted row keeps its position and is recognised as a duplicate
  - unit: after a delete, and after a drop, followed by the worker's
    `TRUNCATE` checkpoint, the body's bytes are not found anywhere in the
    database or WAL file (searched with the key applied); and a checkpoint
    that reports "busy" is retried on the next tick
  - cross-node in `transport/tests.rs` and `group/tests.rs`: a request is
    honoured only for the author's own message; a membership entry is never
    deleted.

### H3: group events in history, the feed, group name, digest, summary fields (size L)

- `dag.rs`: `EntryKind::Profile` with a signed `name`. Refuse entries not
  signed by the owner. Store but do not apply an owner entry whose name fails
  the check. Apply rule: newest by (sender timestamp, author, entry id).
- `set-group-name` verb, with the name check moved from
  `roym_core::conversation::group::validate_group_name`.
- `history`: merge `messages` and the membership/profile log entries in one
  ordered read (a `UNION ... ORDER BY sender_timestamp, author, id`). The
  cursor encodes the last item's ordering key.
- `visible-seq` column and a store-level counter. Set on insert for own sent
  messages and at the accept step for incoming ones. `changes` verb (D-CV-11).
- `transcript-digest` verb over every non-system row and every stored log
  entry (D-CV-4). `group-info` gains `name`; `conversation-summary` gains
  `message-count` and `name`.
- Tests:
  - unit: ordering across both tables; cursor paging across both
  - unit: a message accepted after a re-ask, or readmitted, appears in
    `changes` after the last number already read
  - unit: an incoming `profile` entry from a non-owner is refused; an owner
    entry with a bad name is stored, not applied, and counted in the digest
  - cross-node: a new member sees the name without a resend; two members get
    the same digest after a membership change, and still the same digest
    when one of them dropped a message from a blocked author.

### H4: search (size M)

- A unit test that creates a `trigram` FTS5 table, guarding the compile flag
  (D-CV-6).
- An external-content `trigram` FTS5 table over text bodies. It is updated in
  the same transactions that accept a message, delete it, or drop it; the FTS
  `'delete'` runs before the body is emptied.
- `search` verb. Queries of 3+ characters use FTS5, passed as a quoted
  phrase so special characters are searched for literally (today's
  `escape_regex` rule). Shorter queries use the `LIKE` fallback.
- Tests: unit tests for substring matching ("ell" finds "hello"), the
  short-query fallback, held/dropped/deleted rows not found, card JSON found,
  the conversation filter, and the index staying in step after delete and
  drop (FTS5 `integrity-check`).

### H5: export and import (size L)

- Versioned export format and paged `export-history` / `import-history`.
- `restored` flag on conversations and messages; `verified = false` on
  restored messages; `retry` refuses restored messages.
- Worker loops (outbox, sync, relay, scheduled re-key) skip restored
  conversations.
- Direct-chat rule from D-CV-7: same id merges; a different id becomes a
  separate restored chat; the partial unique index covers live chats only,
  and both upsert clauses change with it; `open-direct` never returns a
  restored chat.
- Group rule from D-CV-7: a group already live on the node merges and stays
  live; an unknown group is created restored.
- Imported rows get new `visible-seq` numbers and are added to the search
  index.
- `group-info.restored` and `conversation-summary.restored`.
- Tests:
  - unit: round trip on one store; restored own messages that were pending
    come back as `failed` with the "restored" reason, and `retry` refuses
    them; restored messages have `verified = false`
  - unit: import into a node that already has a live chat with the same
    peer (same id merges; different id stays separate and read-only)
  - unit: import into a node where the group is live keeps it live and
    syncing
  - unit: imported rows are found by `search` and appear in `changes`
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
    switches to it and drops `SYNC_OVERLAP` (D-CV-11). `roym_transaction`'s
    import stops restoring its card-sync position (D-CV-7).
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
    compares them).
  - The Hub shows `refused` on a sent message (D-CV-12).
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
  `conversation.changes`, including a card that becomes visible late, and a
  rescan from 0 after a restore.
- Substrate e2e: `roym_conversation_e2e.rs`, `roym_group_e2e.rs`,
  `roym_group_offline_e2e.rs`, `roym_restore_e2e.rs`.
- Hub: `crates/roym_web/ui/src/screens/{messages,groups}.ts` only if a field
  changes. Keep the fields stable.

### R2: documents (size S)

- Write `docs/decisions/0025-conversation-capability-owns-history.md` from
  the appendix. Add a line to ADR-0013's status pointing to it.
- In M06C `task.md`, mark D-06C-5 replaced and D-06C-8 changed, with a link
  to ADR-0025. Slice plans are history and are not edited.
- In M06C `status.md` §"Milestone exit audit", mark failure-matrix row 12
  closed by D-CV-12, with the test names.
- `docs/roym-integrated-experience-spec.md` §"Retention and deletion": remove
  "each message is stored twice". Say that delete and drop remove the
  readable body on this machine, and that a group's encrypted log entry and
  key remain. Add the search index's size to the storage statement.
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
other and can run in parallel after H1. H5 depends on H2, H3 and H4: it
exports deleted rows and log entries, and imported rows must enter the search
index. Sizes are relative: S < M < L.

A separate change is moving the data-layer page loop and backup helpers
into `roym_core`. If it lands first, `roym_conversation` uses those helpers
in H1 and R1 (for the `admissions` and `first_contact_charges` collections
and the backup bundle). R1b deletes most of the page loops it would have
touched in this crate. Neither change blocks the other.

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
6. **1:1 first contact over the limit:** `drop` with a report to the sender,
   not `hold` (D-CV-9, D-CV-12). *Recommended*: the sender learns to try
   later, and no body stays on disk.
7. **Held-body time limit:** 30 days by default (D-CV-1). *Recommended.*
8. **Refusal reports (D-CV-12):** report rate-limit refusals only, never a
   block, never in a group. *Recommended.* The alternative is to reword M06C
   failure-matrix row 12 to accept a silent refusal.

## 10. Risks

- **Size of the WIT change.** Each WIT change touches about 15 files across
  8 crates, plus 8 WIT copies. Keep each phase's WIT change small, and land
  it in the same PR as its host code.
- **Asking forever.** If an app always fails to answer, its messages stay
  `undecided` and invisible. This is safer than losing them, but it must be
  visible: add a metric and a warning after N attempts.
- **The delivery ask adds latency to the sender's receipt.** Today the inline
  notify is already awaited before the delivery receipt is sent
  (`transport.rs`). `admission_ask_timeout_ms` (3 seconds) bounds the extra
  time, well under the sender's 30-second call timeout.
- **History merge cost.** The `UNION` read must use the existing order
  indexes on both tables (`idx_messages_order`, `idx_dag_order`). Check the
  query plan in H3.
- **Search index size.** A `trigram` index is several times the text it
  covers. External content avoids a third copy of the text, but the index
  itself still counts toward disk use; measure it in H4 and state it.
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
check that could miss membership changes. It also doubled disk use, left
"deleted" messages readable in the host store, and left a rate-limited
sender with no sign of the refusal. Every future chat SynApp would face the
same gaps.

**Decision.** The conversation capability provides:
- a durable app answer for each incoming message (accept, hold, drop), asked
  once per message and again only until answered, with readmit and a time
  limit on held bodies; an app with no inbox declares it, and only then are
  its messages accepted without asking
- a refusal report to the sender when the app asks for one (never for a
  block)
- local delete with wiped free pages, and an author-only "please delete"
  request
- group events and owner-signed group names inside history, and a
  transcript digest over the whole synced log
- a feed of newly visible messages in local order
- full-text search with substring matching
- paged, versioned export and import of history; imported rows are marked
  restored, are not verified, and are never sent, synced or relayed.

Apps keep only their policy (who to accept) and their product data.

**Consequences.**
- One store per service, and no app-side copy.
- Deleting or dropping removes the readable body from the host store and its
  free pages. For a group, the encrypted log entry and the group key remain
  on the node.
- Restore of history is a capability feature, not an app feature. A restored
  group is read-only.
- The WIT interface changes four records, adds six types and eight
  functions, removes `membership-history`, and changes the `on-message`
  result.
- Continuing a conversation after a move to a new machine is still open. It
  has its own backlog row.

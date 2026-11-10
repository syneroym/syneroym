# M08B slice B3b — Language tags on chats: implementation plan

*Reader: the developer (or AI) who implements slice B3b. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan review round 1 done, 2026-11-10. Owner questions answered 2026-11-10. P-items folded in.**

**Size and PR.** M. One PR on `feat/m08b-b3b-chat-lang-tags`, with the
commits of section 6 in that order. The branch starts from `main` at
`4830bdf7` (B3a is merged,
[PR #354](https://github.com/syneroym/syneroym/pull/354)) and already holds
the plan commit.

B3b lets a chat message say which languages its text is in: up to 3
language tags, main language first. The tags travel inside the signed and
encrypted message, between nodes, for 1:1 chats and group chats. The
conversation host (the substrate crate `crates/conversation`) checks them
and stores them. The Roym `conversation` service takes them in
`conversation.send` and returns them in every message it shows. In the Hub,
each chat box gets a short language list that starts at the Hub language.
The Hub marks the message text with the main language (`lang`), and shows
"Language not stated" for a text message that has no tag. B3b adds no
JSON-RPC method, no WIT package and no record type.

**Base.** Every file path and line number below was checked on `main` at
`4830bdf7`. Links are relative to this folder.

**Parallel work.** B4a (substrate only) may run beside B3b. Both touch
`crates/text_lang`: B4a adds `nfc()`, B3b adds a tag-list check. The two
additions do not touch the same lines. B4b must wait for B3b: both change
`crates/conversation` and the Hub messages screen ([task.md](task.md)
section 4).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - No planning ids in code or comments: no "M08B", "B3b", "ML-14" or
    "D-B3b-3" in `.rs`, `.ts`, `.ftl`, `.wit`, `.json` files or test names.
    Do not copy review ids into code, comments or test names (B2 R5, B3a
    R7).
  - Comments never cite size rules. B3b moves one block of
    `apply_message_entry` into a helper (D-B3b-13); the comment says what
    the helper is for, not why it moved.
  - **File size.** Production files at most 800 lines. Test files at most
    800 lines, and a file in `xtask/oversized-test-files.txt` is capped at
    its recorded count. Section 7.5 lists every file B3b touches that is
    close to a limit.
  - **Function size.** `clippy::too_many_lines` at 100. Two functions B3b
    touches are close (section 2.2 point 12).
  - The import clean-up pass over the Rust files you edit, the deferred
    backlog, the living-docs close-out and the demo close-out.
  - A `roym_*` crate and a WIT file change, so run `mise run build:roym`
    and `mise run build:test-components` before you trust a parity run
    ([task.md](task.md) section 2, "Records").
  - Every new Hub string goes through `t()` and the build check (B1b).
- [task.md](task.md): section 1 (Y3 and "Where Y3 and Y4 live"), section 2
  ("Host capabilities": "Chat tags go next to it"), "Component size limit",
  "B3b", section 4, section 8.
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  "Rules for every layer" (rule 2 "Translation runs on the reader's own
  node", rule 4 "No migrations"), "C. Language-tagged free text" (the
  "Chat messages" paragraph, "Tag validation" and the two Hub rules, and the
  B3a decision list), criteria ML-14 and ML-25, and `## Deviations` (the
  first entry is about `syneroym-text-lang`). Every B3b decision and
  deviation is written there, and only there (section 3).
- [demo-stories](../../changes/demo-stories/change.md), read for context
  only: B3b extends story 40 (Q6) under its rule "Link between slice plans,
  the demo and AGENTS.md". B3b meets no criterion of that doc and writes
  nothing there.
- [slice-b3a-implementation-plan.md](slice-b3a-implementation-plan.md) (the
  layout of this plan) and the five review files. Section 7.8 lists how B3b
  avoids their kinds of findings.

**Change docs.** The prompt lists one change doc, roym-multilingual.
[task.md](task.md) names no other change doc for B3b. Both criteria of B3b
(ML-14, ML-25) are in roym-multilingual.

### Terms used in this plan

- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **Conversation host:** the substrate crate `crates/conversation`
  (package `syneroym-conversation`). It keeps every chat: the encrypted
  log, the outbox, delivery, ordering, search and export
  ([ADR-0025](../../../decisions/0025-conversation-capability-owns-history.md)).
  A WASM guest reaches it through the WIT package `syneroym:conversation`.
- **Roym `conversation` service:** the Roym service
  (`crates/roym_conversation`) that the Hub calls. It calls the host.
- **Both builds:** a Roym service runs as a `wasm32-wasip2` component, or
  linked into the substrate as native code through `syneroym-app-host-native`.
  The two must behave the same; the dual-build parity tests check this.
- **Language tag:** a BCP 47 code in canonical syntax form, checked by
  `syneroym-text-lang` (`LangTag`): `hi`, `en`, `hi-Latn`.
- **Message languages:** the ordered list of 0 to 3 language tags on one
  chat message. The first one is the **main language**. An empty list means
  "language not stated".
- **1:1 payload:** `DeliveryPayload`
  ([envelope.rs:13](../../../../crates/conversation/src/envelope.rs#L13)),
  the signed message that a ratchet session encrypts for one peer.
- **Group entry:** a message in a group's log (DAG). Its plaintext is
  `encode_body(...)` ([dag.rs:292](../../../../crates/conversation/src/dag.rs#L292)),
  sealed with the group's epoch key; the entry signature covers the
  ciphertext ([dag.rs:160](../../../../crates/conversation/src/dag.rs#L160)
  to `:171`).
- **Card:** a chat message with content type
  `application/vnd.roym.card+json` that carries a signed Roym record
  (request, quote, receipt). The transaction service sends it.

## 1. What B3b builds, in one paragraph

`syneroym-text-lang` gains a tag-list check (`check_tag_list`) and the
limit `MAX_MESSAGE_LANGUAGES = 3`. The WIT `message` record gains
`languages: list<string>` next to `content-type`, and `send` gains a
`languages` parameter. The same field goes through every layer of both
builds: `syneroym-rpc`'s `ConversationHost`, the WASM host in
`sandbox_wasm`, the native shim in `app_host_native`, and `app_host`'s
`AppConversation`. The conversation host checks the list on `send`, signs it
inside the 1:1 payload (new field in the canonical bytes) and seals it
inside the group plaintext (new field in `encode_body`), stores it in a new
`messages.languages` column, refuses a received message whose list breaks
the rule, and keeps it in a backup. The Roym `conversation.send` method
takes an optional `languages`, and every `MessageView` (history, changes,
search) returns it. `roymctl roym group send` gets an optional, repeatable
`--lang`. In the Hub, both chat boxes (1:1 and group) get a language list
(the profile's list, moved into a shared module), the message text gets
`lang` from the main language (`lang=""` when there is none), a text
message with no tag shows "Language not stated", and a search result sets
`lang` the same way.

## How this is shown to an end user

- Alice uses the Hub in Hindi. She opens her chat with Bob. Under the
  message box she sees her message languages: "हिन्दी".
- She writes a message that mixes Hindi and English words. She adds
  English to the list, and she sends the message.
- Bob sees the text exactly as Alice wrote it. His browser knows that the
  text is Hindi, so it picks a Hindi font, and a screen reader reads it as
  Hindi.
- A message from a program that did not say its language shows a small line
  "Language not stated" under its text.
- Group chats work the same way.

How a developer can see it work:

- `cargo nextest run -p syneroym-conversation` runs the host tests: a
  message sent with `["hi", "en"]` comes back with the same list, on the
  receiver too; `["HI"]`, 4 tags or a repeated tag is refused.
- A `conversation.send` with `"languages": ["hi", "en"]` returns a message
  id, and `conversation.history` shows `"languages": ["hi", "en"]` on it. A
  send with `"languages": ["hi_IN"]` answers `-32602` with
  `data.field = "languages[0]"`.
- `mise run demo` runs story `demo/40-roym-languages.spec.ts`. Its new step
  shows the language list at Hindi in Alice's chat with Bob, and her sent
  message with `lang="hi"` (Q6).

**Demo:** story 40 gains one step (Q6): in a Hindi Hub, Alice opens her
chat with Bob, adds English to the message languages, and sends a message
with Hindi and English words. Her message shows with `lang="hi"`.

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. What does a receiver do with a message whose language list breaks
the rule? (blocks Step 3)**

Owner answer: (a), 2026-11-10. Refuse the whole message. Add a log line for refused group entries (P4).

The rule (D-B3b-1): at most 3 tags, each in canonical form, no tag twice.
The sender's host checks it before it signs, so only a node that does not
follow the rule sends a bad list. For records, the owner chose "canonical
form only, everywhere", also on verify (B3a Q3 (a)).

- (a) **Refuse the whole message, as records are refused.** A 1:1 message:
  the receiver answers `invalid-argument`, so the sender's outbox marks it
  `failed` with "callee refused" (the same path as a sender timestamp too
  far in the future,
  [transport.rs:368](../../../../crates/conversation/src/transport.rs#L368)
  to `:375`). A group entry: it is not applied, the same as an entry whose
  body cannot be decoded today
  ([entry.rs:146](../../../../crates/conversation/src/group/entry.rs#L146)
  to `:149`). One rule on both sides; what is stored is what was signed.
  *Recommended.*
- (b) **Keep the message and drop the bad list.** No message is lost if a
  later `icu_locale_core` prints a tag differently (the B3a P10 coupling).
  But the stored row then differs from what the sender signed, and the rule
  has two forms.
- (c) **Keep the message and the list after a well-formedness check only**
  (`LangTag::parse`, not `parse_canonical`). Lenient, but a non-canonical
  tag reaches the Hub, and records and chats follow different rules.

**Q2. How does the WIT carry the list? (blocks Step 3)**

Owner answer: (a), 2026-11-10. A languages parameter on send.

The WIT is `syneroym:conversation@0.1.0`
([conversation.wit:1](../../../../crates/wit_interfaces/wit/conversation/conversation.wit#L1)).
`send` takes `(conversation, content-type, body)` (`:185` to `:189`), and
the `message` record has `content-type` at `:67`.

- (a) **A `languages: list<string>` parameter on `send`, between
  `content-type` and `body`, and a `languages: list<string>` field in
  `message`, right after `content-type`.** The version stays `0.1.0`: the
  product is not released, and no WIT version moved before (change doc rule
  4). Every caller of `send` changes once (section 2.2 point 6).
  *Recommended.*
- (b) **A new record `outgoing-message { content-type, languages, body }`
  as the one parameter of `send`.** Easier to extend later, but it changes
  every caller the same way and adds a type.
- (c) **A second function `send-tagged`.** No caller changes, but two
  functions do the same work.

**Q3. What does the chat box show for the message languages? (blocks
Step 5)**

Owner answer: (a), 2026-11-10. The shared list component. One risk for the owner: the default tag is the Hub language, so Hindi typed in an English Hub goes out as en. The list also resets each time a 1:1 thread is redrawn.

ML-14 allows 3 tags, main language first, and ML-25 says a mixed message
lists both languages. The profile screen already has an ordered language
list with add, "move up" and remove
([profile.ts:37](../../../../crates/roym_web/ui/src/screens/profile.ts#L37)
to `:111`).

- (a) **The profile's list, moved into one shared component, at most 3
  rows.** It starts with one row, the Hub language. "Move up" makes another
  row the main language. Removing every row sends no tag ("language not
  stated"). One component for profile, 1:1 chat and group chat.
  *Recommended.*
- (b) **One main-language `<select>` and one "also in" `<select>`** (at
  most 2 tags). Smaller, but a new control, and it cannot hold 3 tags.
- (c) **One main-language `<select>` only.** Extra tags through the API
  only. The Hub then cannot send what ML-25 describes.

**Q4. What does the Hub show for a message that has tags? (blocks Step 5)**

Owner answer: (a), 2026-11-10. No language name under a tagged message.

- (a) **Only `lang` on the text.** Nothing more is visible. A reader sees
  the script, and B5 adds the translate action at the chat hook point,
  where the source language will matter. *Recommended:* ML-14 asks only for
  the "not stated" note.
- (b) **Also the language names** ("Tamil, English") under the text when
  the main language is not the Hub language. Useful before B5, but it adds
  a line to many messages and one more rule.

**Q5. Do cards carry message languages? (blocks Step 4)**

Owner answer: (a), 2026-11-10. Cards carry no message languages.

The transaction service sends a card with `SendInput { conversation, body,
content_type }`
([app.rs:538](../../../../crates/roym_transaction/src/app.rs#L538),
[agreement_ops.rs:482](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L482)).
Since B3a, the record inside the card has its own `lang`.

- (a) **No.** A card sends no message languages. The record's `lang` (and,
  for a quote, `terms.lang`) covers its text, and the Hub already sets
  `lang` from it (B3a). The Hub never shows "Language not stated" for a
  card. The sibling-call JSON of the transaction service does not change
  (D-B3b-6). *Recommended.*
- (b) **Yes, the record's tag as the one message tag.** Two places then say
  the same thing, and they can disagree.

**Q6. What does story 40 show for B3b? (blocks Step 6)**

Owner answer: (a), 2026-11-10. One send step in story 40.

Story 40 runs on the demo nodes after story 20, so Alice's chat with Bob
exists and the story reads `BOB_ADDR` from the same demo env
([20-roym-chat.spec.ts:28](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts#L28)).

- (a) **One step in Alice's chat with Bob:** with the Hub in Hindi, she
  opens Messages and the chat with Bob; the language list shows Hindi; she
  adds English, sends a message with Hindi and English words (with a run
  tag, as story 20 does), and her message shows with `lang="hi"`. Bob gets
  one more message each run, as in story 20. *Recommended.*
- (b) **The language list only, no send.** Nothing reaches Bob, but the
  story does not show the tags on a sent message.
- (c) **No story step.** Only the Playwright test shows it.

### 2.2 Points found while reading

1. **The WIT today.** The `message` record
   ([conversation.wit:50](../../../../crates/wit_interfaces/wit/conversation/conversation.wit#L50)
   to `:80`) has `content-type: string` (`:67`) and `body: list<u8>`
   (`:68`). `send` is at `:185`. The guest export `on-message` (`:318`)
   receives a `message`. The dual-build fixture links the same file
   (`test-components/dual-build-fixture/wit/deps/conversation/conversation.wit`
   is a symlink), so the fixture builds against the new WIT.
2. **The layers of `send`, in both builds.**
   - WIT host for WASM guests:
     [capabilities_messaging.rs:182](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_messaging.rs#L182)
     calls `ConversationHost::send`; `map_message` (`:72`) builds the WIT
     `message`.
   - `ConversationHost` trait:
     [rpc/conversation.rs:200](../../../../crates/rpc/src/conversation.rs#L200);
     `ConversationMessage` at `:77` to `:93`.
   - The host itself:
     [host_impl.rs:76](../../../../crates/conversation/src/host_impl.rs#L76)
     to `:109` (1:1 through `enqueue_direct`,
     [lib.rs:430](../../../../crates/conversation/src/lib.rs#L430); group
     through `send_group`,
     [group.rs:452](../../../../crates/conversation/src/group.rs#L452)).
   - `AppConversation` for Roym services:
     [app_host/lib.rs:324](../../../../crates/app_host/src/lib.rs#L324);
     the guest impl
     [guest.rs:241](../../../../crates/app_host/src/guest.rs#L241); the
     native impl
     [app_host_native/host.rs:399](../../../../crates/app_host_native/src/host.rs#L399),
     which calls the same WIT host trait (`HostConversation::send`).
   - Converters: `message_out` and `rpc_message_to_guest`
     ([convert/conversation.rs:49](../../../../crates/app_host_native/src/convert/conversation.rs#L49),
     `:160`); the `on-message` JSON for a WASM guest,
     [engine/auth.rs:170](../../../../crates/sandbox_wasm/src/engine/auth.rs#L170)
     to `:186`, which names every field of the WIT record.
   - Never-built stubs that must still compile:
     [synsvc_native/conversation.rs:33](../../../../crates/control_plane/src/synsvc_native/conversation.rs#L33),
     [host_capabilities.rs:143](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L143),
     and one test host in
     [roym_core/signing.rs:516](../../../../crates/roym_core/src/signing.rs#L516)
     (`:686` is WebSocket).
3. **The signed 1:1 payload.** `canonical_bytes`
   ([envelope.rs:30](../../../../crates/conversation/src/envelope.rs#L30)
   to `:61`) writes the domain `syneroym:conversation:v1`, then the fields
   with a `0` between them, and the body with a `u64` length in front
   (`:58`). The sender builds the payload twice: once to sign it
   ([lib.rs:470](../../../../crates/conversation/src/lib.rs#L470)) and once,
   later, from the stored row to encrypt it
   ([transport.rs:211](../../../../crates/conversation/src/transport.rs#L211)
   to `:219`). So the stored row must hold the tags exactly as signed. The
   receiver checks the signature
   ([transport.rs:362](../../../../crates/conversation/src/transport.rs#L362))
   and then the timestamp and the conversation id (`:368` to `:381`). A
   mailbox deposit carries the same sealed envelope (`outbound_envelopes`,
   [transport.rs:205](../../../../crates/conversation/src/transport.rs#L205)),
   so it needs no change of its own.
4. **The group plaintext.** `encode_body(content_type, body)` and
   `decode_body`
   ([dag.rs:292](../../../../crates/conversation/src/dag.rs#L292) to
   `:316`). The sender calls it in `send_group`
   ([group.rs:476](../../../../crates/conversation/src/group.rs#L476)); the
   receiver in `apply_message_entry`
   ([entry.rs:146](../../../../crates/conversation/src/group/entry.rs#L146)).
   Tests call it in `group/tests.rs` (`:194`, `:406`, `:452`),
   `group/tests/deletion.rs:63` and `host_impl/tests/events.rs:164`.
5. **The message id does not depend on the tags.** `derive_message_id`
   ([ids.rs:32](../../../../crates/conversation/src/ids.rs#L32)) hashes the
   author, conversation, timestamp, content type, body and a random nonce.
   The receiver never derives it again (`ids.rs:5` to `:7`). A group
   message's id is its entry id, a hash of the signed header. So the tags
   need not enter the id (D-B3b-3).
6. **Callers of `ConversationHost::send`.** About 20 test calls in
   `crates/conversation` (`host_impl/tests.rs`, `host_impl/tests/events.rs`,
   `group/tests.rs`, `outbox/tests/deposit.rs`, `transport/tests.rs`), one
   in `sandbox_wasm`, and the fixture
   (`test-components/dual-build-fixture/src/app/dispatch.rs:194`). The
   system sends inside the host use `enqueue_direct` with `system = true`
   (group keys `group.rs:350`, `:631`; deletion request
   `host_impl.rs:351`; refusal notice `outbox.rs:598`); they carry no tags.
7. **The store.** The `messages` table
   ([schema.rs:78](../../../../crates/conversation/src/store/schema.rs#L78)
   to `:103`) is made with `CREATE TABLE IF NOT EXISTS`, and the crate has
   no `ALTER TABLE` or schema version. Seven `SELECT` lists name every
   column for `row_to_message`
   ([message.rs:698](../../../../crates/conversation/src/store/message.rs#L698)):
   `message.rs:253`, `:571`, `:646`; `history.rs:31`, `:63`;
   `search.rs:29`, `:52` (with the alias `m.`). The backup export has its
   own list ([backup.rs:205](../../../../crates/conversation/src/store/backup.rs#L205)).
   Rows are written at `message.rs:94`, `:150`, `:225`, `group.rs:403`,
   `entry.rs:166`, `:286` (a withdrawn row, no body) and `backup.rs:397`.
   Two store tests insert rows by hand (`store/tests.rs:67`, `:462`); a
   column with a default needs no change there.
8. **Store function callers.** `insert_outgoing_and_enqueue`,
   `insert_outgoing_without_enqueue` and `insert_incoming_if_absent` have
   about 45 test callers (`store/tests*.rs`, `host_impl/tests*.rs`,
   `outbox/tests.rs`, `transport/tests/delivery.rs`, `group/tests.rs`,
   parity `card_sync.rs:27`, `group_lifecycle.rs:258`).
9. **The transcript digest does not read the body or the tags.** It hashes
   `id`, `author`, `sender_timestamp` and `content_type` only
   ([history.rs:139](../../../../crates/conversation/src/store/history.rs#L139)
   to `:190`). So two members agree on it whatever the tags.
10. **The Roym side.** `SendInput` (`conversation`, `body`, `content_type?`,
    `deny_unknown_fields`;
    [method/conversation.rs:10](../../../../crates/roym_core/src/method/conversation.rs#L10))
    is read by `messages::send`
    ([messages.rs:262](../../../../crates/roym_conversation/src/app/messages.rs#L262)
    to `:313`), which calls `AppConversation::send` (`:295`). `MessageView`
    ([roym_core/conversation.rs:141](../../../../crates/roym_core/src/conversation.rs#L141))
    is built `From<&Message>` (`:162`) and by hand for membership and
    group-name rows (`messages.rs:320`, `:342`) and in two tests
    (`conversation.rs:246`, `conversation/group/tests.rs:7`). `from_host`
    maps `InvalidArgument` to `-32602` without `data.field`
    ([app.rs:134](../../../../crates/roym_conversation/src/app.rs#L134)).
    The key test `message_view_writes_the_same_keys_as_the_old_json`
    ([method/tests.rs:437](../../../../crates/roym_core/src/method/tests.rs#L437))
    lists the keys of a `MessageView`.
11. **Callers of the Roym `conversation.send`.** The Hub
    ([messages.ts:293](../../../../crates/roym_web/ui/src/screens/messages.ts#L293),
    [groups.ts:426](../../../../crates/roym_web/ui/src/screens/groups.ts#L426)),
    `roymctl roym group send`
    ([group.rs:192](../../../../apps/roymctl/src/commands/roym/group.rs#L192)
    to `:195`), the transaction service (two card sends, Q5), the developer
    guide block `roym-client=open-send`
    ([developer-guide.md:2440](../../../developer-guide.md#L2440)), which
    story `30-roym-client.sh` runs, about 30 Rust test calls
    (parity, `method_contract`, `crates/substrate/tests/roym_*_e2e.rs`,
    `common/roym_flow.rs:72`), and none in the Playwright specs (they use
    the Hub). Every one stays valid: `languages` is optional (D-B3b-6).
12. **Two functions near the clippy line limit.** `apply_message_entry`
    ([entry.rs:110](../../../../crates/conversation/src/group/entry.rs#L110)
    to `:216`) has 103 lines that are not blank or a comment, signature
    included. `ingest_verified`
    ([transport.rs:408](../../../../crates/conversation/src/transport.rs#L408)
    to `:510`) has 89. B3b adds to the first and not to the second: the
    receive check goes into `verify_incoming_envelope` (D-B3b-4), and
    `apply_message_entry` first moves its row literal into a helper
    (D-B3b-13).
13. **The Hub today.** Two message types: `MessageRow`
    ([messages.ts:41](../../../../crates/roym_web/ui/src/screens/messages.ts#L41))
    and `GroupMessageRow`
    ([groups.ts:55](../../../../crates/roym_web/ui/src/screens/groups.ts#L55)).
    The 1:1 text is drawn at `messages.ts:529` to `:534`, the group text at
    `groups.ts:114`, a search hit at `messages.ts:152` (from
    `searchHits`, [message_search.ts:30](../../../../crates/roym_web/ui/src/screens/message_search.ts#L30)).
    The chat boxes: `messages.ts:279` to `:302`, `groups.ts:405` to `:444`.
    `textLangSelect`, `langAttr` and `languageName` are in
    [text_lang.ts](../../../../crates/roym_web/ui/src/i18n/text_lang.ts).
14. **e2e steps that use the chat boxes.** `sendMessage` fills
    `.compose-input` and clicks the button named "Send" (exact)
    ([hub-helpers.ts:83](../../../../crates/substrate/tests/e2e/hub-helpers.ts#L83)
    to `:87`); `openMessages` and `openConversation` click "Messages" and
    "Open conversation" (`:66` to `:80`).
    [roym-groups.spec.ts:100](../../../../crates/substrate/tests/e2e/tests/roym-groups.spec.ts#L100)
    fills `.composer-box input`. So the new controls are a `<select>` and
    `<button type="button">` elements whose names are not "Send", and no new
    `<input>` goes into `.composer-box`.
    [roym-hub.spec.ts:387](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L387)
    reads the thread text and checks that "delivered" is not in it; the
    language names do not contain it.
15. **The method shape snapshots** (`crates/roym_web/tests/method_shapes/`,
    rewritten with `UPDATE_METHOD_SHAPES=1`). These hold a `MessageView`
    and change: `conversation_history_direct`, `conversation_changes`,
    `conversation_search`, `conversation_history_deleted_message`,
    `conversation_history_group_membership`. `conversation_export` may
    change too, because a backup message gains a field. Review the diff:
    only `languages` keys may appear.
16. **No ADR changes.** ADR-0013 and ADR-0025 do not name the payload
    fields or the WIT parameters, so neither needs an amendment.

## 3. What B3b meets

| Criterion | What B3b does | Test |
| --- | --- | --- |
| ML-14 | A chat message carries 0 to 3 language tags, main language first, on both builds and between nodes, 1:1 and group. The host refuses 4 tags, a repeated tag and a tag not in canonical form, on send and on receipt. The Hub shows "Language not stated" for a text message with no tag. | `text_lang` (`check_tag_list`); `host_impl/tests.rs`, `transport/tests/delivery.rs`, `group/tests/languages.rs`, `envelope.rs`, `dag.rs`, `backup_roundtrip.rs`; `method_contract`; parity `conversation.rs` and `group.rs`; `message_text.test.ts`, `messages.test.ts`, `groups.test.ts`, `hindi-screens.test.ts` |
| ML-25 | A message with Hindi and English words keeps its text byte for byte, its tags list `hi` and `en`, and the Hub sets `lang` from the first tag. | `host_impl/tests.rs` (body and tags read back), `roym_conversation_e2e.rs` (two nodes), parity `conversation.rs`; `message_text.test.ts`; `roym-hub.spec.ts` ("a message in two languages keeps its text and has its main language"); story 40 |

The exact test names go into the change doc's Test column at close-out,
after they exist (B3a R2).

**Where decisions and deviations are written.** Both criteria are in the
roym-multilingual change doc, so every B3b decision and deviation goes
there and only there, at close-out (Step 7): decisions into "C.
Language-tagged free text" as a new list "Decided <date>, chat message
languages", after the "Chat messages" paragraph; deviations under
`## Deviations`. The deviations, if the owner takes the recommendations:
cards carry no message languages (Q5); the chat box offers only
`RECORD_LANGUAGES` (D-B3b-10); a message with no tag has `lang=""`
(D-B3b-11); a tagged message shows no language name (Q4); a receiver
refuses a whole message whose list breaks the rule (Q1); the WIT version
stays `0.1.0` (Q2).

## 4. Behaviour-change audit

### Wire changes

No JSON-RPC method, record type, record version, WIT package or config key
is added or removed.

| # | Where | Before | After |
| --- | --- | --- | --- |
| W1 | WIT `syneroym:conversation@0.1.0`, `message` record | no tags | `languages: list<string>` after `content-type` (Q2) |
| W2 | WIT `send` | `(conversation, content-type, body)` | `(conversation, content-type, languages, body)`. A list that breaks the rule (D-B3b-1) is `invalid-argument("message languages: …")` |
| W3 | `syneroym-rpc` `ConversationHost::send`, `ConversationMessage`; `app_host` `AppConversation::send` | — | the same parameter and field, `Vec<String>` |
| W4 | 1:1 payload between nodes (`DeliveryPayload`, canonical bytes) | — | `languages` field; the canonical bytes hold it between `content_type` and the body (D-B3b-2). The domain stays `syneroym:conversation:v1` |
| W5 | Group plaintext (`encode_body`) | `ct_len, ct, body` | `ct_len, ct, tag list, body` (D-B3b-2) |
| W6 | Receipt of a 1:1 message or a group entry | — | a list that breaks the rule is refused (Q1): 1:1 answers `invalid-argument`, the sender marks the message `failed`; a group entry is not applied |
| W7 | Store `messages` table | — | column `languages TEXT NOT NULL DEFAULT ''` (D-B3b-5) |
| W8 | Backup bundle message (`conversation.export`, `conversation.import`) | — | `languages` (missing means none); an import with a bad list is `invalid-argument`, like an unknown state |
| W9 | Roym `conversation.send` input | `conversation`, `body`, `content_type?` | also `languages?` (0 to 3). `"languages": ["hi_IN"]` is `-32602` with `data.field = "languages[0]"`; 4 tags or a repeated tag is `-32602` with `data.field = "languages"` |
| W10 | Roym `MessageView` (`conversation.history`, `.changes`, `.search`) | — | `languages` always present, `[]` when none (D-B3b-7) |
| W11 | `roymctl roym group send` | no language flag | optional `--lang <tag>`, up to 3 times, put in canonical form by `LangTag::parse` (D-B3b-9) |
| W12 | WIT guest export `on-message` | — | the `message` it gets has `languages` |

W4, W5 and W7 mean that a node without B3b and a node with B3b cannot
exchange messages: the old node cannot read the new payload, and a 1:1
signature made over the new canonical bytes does not verify there. A
developer node made before B3b has a `conversation.db` without the new
column; its conversation host fails until the data folder is reset. The
product is not released, so there is no migration (change doc rule 4,
D-S0-5). The PR text says so.

What the Hub sends changes too:

| # | Field | Before | After |
| --- | --- | --- | --- |
| H1 | `languages` of `conversation.send` from the 1:1 chat box and the group chat box | — | the box's language list, in its order; `[]` when the person removed every row |

### Behaviour changes (Hub only)

| # | Before | After |
| --- | --- | --- |
| 1 | A chat box has a text input and Send | Also a language list under it (Q3), starting at the Hub language |
| 2 | Message text has no `lang` | `lang` is the main language; `lang=""` when the message has no tag or its tag does not parse |
| 3 | — | A text message with no tag shows "Language not stated" under its text. A card, a deleted message, a binary message and a group system row do not |
| 4 | A search hit has no `lang` | The snippet has `lang` from the main language |

### Stays the same

- Every method name, every record type and `version`, every content type.
- Message ids, ordering, delivery states, admission, the event feed and the
  transcript digest (section 2.2 points 5 and 9).
- Chat search: it still indexes the raw body (B4b changes it).
- Cards: the transaction service's sibling call to `conversation.send`
  sends the same JSON as before (D-B3b-6), and a card message has no tags
  (Q5).
- Every English Hub string a test or a Playwright step reads, except the
  new messages.
- `roymctl` output (it prints the JSON it gets; `languages` appears in it).

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3). The ones
that depend on an owner answer say so.

- **D-B3b-1. The rule, in `syneroym-text-lang`** (file
  `crates/text_lang/src/lib.rs`; tests in `src/tests.rs`).

  ```rust
  /// Most language tags a chat message carries, main language first.
  pub const MAX_MESSAGE_LANGUAGES: usize = 3;

  #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
  pub enum TagListError {
      #[error("more than {0} languages")]
      TooMany(usize),
      #[error("the language '{0}' is listed twice")]
      Duplicate(String),
  }

  /// An ordered list of tags: at most `max`, no tag twice. Tags are
  /// compared as canonical strings, so `hi` and `hi-IN` are two tags.
  pub fn check_tag_list(tags: &[LangTag], max: usize) -> Result<(), TagListError>;

  /// Parses each string with `LangTag::parse_canonical` and checks the list
  /// against MAX_MESSAGE_LANGUAGES. The one check every holder of message
  /// languages calls.
  pub fn parse_message_languages(tags: &[String]) -> Result<Vec<LangTag>, MessageLanguagesError>;

  #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
  pub enum MessageLanguagesError {
      #[error(transparent)]
      Tag(#[from] LangTagError),
      #[error(transparent)]
      List(#[from] TagListError),
  }
  ```

  The profile's own check
  ([person.rs:80](../../../../crates/roym_core/src/person.rs#L80) to
  `:88`) calls `check_tag_list(&self.languages, MAX_LANGUAGES)` and maps
  the two errors to its existing `TooManyLanguages` and
  `DuplicateLanguage`, so there is one copy of the loop. The profile's
  behaviour and error text do not change. The rule lives here and not in
  `crates/conversation` because both the host and `roym_core` read it, and
  `roym_core` cannot depend on the host crate ([task.md](task.md) section
  1).
- **D-B3b-2. The tags on the wire** (Q2 (a)).
  - `DeliveryPayload` gains `pub languages: Vec<String>` after
    `content_type`. `canonical_bytes`, `sign` and `verify` take
    `languages: &[String]` after `content_type`. In the bytes, after
    `content_type` and its `0`, comes the list, then the body as today.
    `sign` gains `#[allow(clippy::too_many_arguments)]` (7 to 8 arguments,
    P2).
  - One helper writes the list, in `envelope.rs`, and `dag.rs` uses it
    too:

    ```rust
    /// `u64` count, then each tag as a `u64` length and its bytes. Lengths,
    /// not separators, so no byte can move between two tags or into the body.
    pub(crate) fn put_languages(out: &mut Vec<u8>, languages: &[String]);
    /// The reverse of `put_languages`, reading from `bytes[*at..]` and moving
    /// `at`. Fails on a short buffer, a count over MAX_MESSAGE_LANGUAGES,
    /// an offset overflow or a tag that is not UTF-8.
    pub(crate) fn take_languages(bytes: &[u8], at: &mut usize) -> Result<Vec<String>, String>;
    ```

    `take_languages` refuses a count over 3 before it allocates, so a bad
    entry cannot ask for a large `Vec`. Safe length parsing rule (P1): no `+`
    or `-` on a length read from the wire. Read each length with
    `usize::try_from(u64)`, add offsets with `checked_add`, and take slices
    with `get(a..b)`; any failure is `Err`. Refuse a tag longer than
    `MAX_TAG_LEN` before slicing.
  - `encode_body(content_type, languages, body)` writes `u64 ct_len`, the
    content type, `put_languages`, then the body. `decode_body` returns
    `(String, Vec<String>, Vec<u8>)` and also runs
    `parse_message_languages` on the list (Q1 (a)): a bad list is an
    `Err`, so the entry takes the path of a body that cannot be decoded.
    `decode_body` applies the same safe length parsing rule (P1) for
    `ct_len` and offsets, closing an existing panic/overflow hole on hostile
    lengths.
  - The domain string of the 1:1 bytes stays `syneroym:conversation:v1`:
    nothing reads the old form, as with record versions (D-S0-5).
- **D-B3b-3. The message id does not change.** Section 2.2 point 5.
- **D-B3b-4. Where the host checks the list** (Q1 (a)).
  - `send` (`host_impl.rs:76`): first thing after the reserved-type check,
    `parse_message_languages(&languages)`; an error is
    `InvalidArgument(format!("message languages: {e}"))`. The host passes
    the list on as `&[String]` (the strings it got, already checked).
  - 1:1 receipt: in `verify_incoming_envelope`
    ([transport.rs:318](../../../../crates/conversation/src/transport.rs#L318)),
    after the conversation-id check, the same call; an error is
    `InvalidArgument`. Not in `ingest_verified`, which is near the clippy
    limit (section 2.2 point 12). The test hook `deliver_inbound`
    ([lib.rs:374](../../../../crates/conversation/src/lib.rs#L374)) skips
    this check, as it already skips the signature check.
  - Group receipt: `decode_body` (D-B3b-2). When a group entry is refused
    because of a bad language list, emit a `tracing::warn!` with entry id
    and author (never the body text, P4). The entry stays unapplied and retry
    on each pass is the accepted cost (deferred backlog).
  - Backup import (`import_messages`,
    [backup.rs:358](../../../../crates/conversation/src/store/backup.rs#L358)):
    the same call per message; an error fails the chunk with
    `invalid-argument`, as an unknown state does today.
- **D-B3b-5. The stored column.** `languages TEXT NOT NULL DEFAULT ''` at
  the end of the `messages` table: the canonical tags joined by one space
  (a tag has no space). `StoredMessage` gains `pub languages: Vec<String>`
  (after `content_type`). One constant names the column list:

  ```rust
  /// Every column `row_to_message` reads, in its order.
  pub(crate) const MESSAGE_COLUMNS: &str = "id, conversation_id, author, \
      sender_timestamp, received_at, content_type, body, signature, outgoing, \
      verified, state, last_error, system, entry_id, admission, admission_reason, \
      admission_changed_at, notify_attempts, next_notify_at, report_refusal, \
      refused, deleted_at, restored, visible_seq, languages";
  ```

  The five unaliased `SELECT`s (`message.rs:253`, `:571`, `:646`,
  `history.rs:31`, `:63`) use it through `format!`. The two in `search.rs`
  join `messages_fts`, which also has a `body` column, so they keep their
  `m.` list and append `m.languages`; B4b rewrites both queries.
  `row_to_message` reads index 24 and splits it with
  `split_ascii_whitespace`. The three insert functions
  (`insert_outgoing_and_enqueue`, `insert_outgoing_without_enqueue`,
  `insert_incoming_if_absent`) and `record_outgoing_group_message` take
  `languages: &[String]` right after `content_type`, and write
  `languages.join(" ")`. Three raw `INSERT` statements are updated too (P3):
  `apply_message_entry` (`group/entry.rs:166`) adds `languages` to the
  statement and binds `content.1.join(" ")`; the backup import
  (`store/backup.rs:397`) adds `languages` and binds `m.languages.join(" ")`;
  the withdrawn row (`group/entry.rs:286`) stores no list (`''` default, as the
  body is gone). No `ALTER TABLE` and no schema version: pre-release rule (W7).
- **D-B3b-6. The Roym input.** `SendInput` gains

  ```rust
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub languages: Vec<LangTag>,
  ```

  `LangTag`'s `Deserialize` refuses a tag that is not canonical, so a bad
  tag answers `-32602` with `data.field = "languages[0]"` through
  `parse_value` ([method.rs:107](../../../../crates/roym_core/src/method.rs#L107)).
  `messages::send` then calls `check_tag_list(&input.languages,
  MAX_MESSAGE_LANGUAGES)`; an error is returned as `ParamError { field:
  Some("languages".into()), message: e.to_string() }.into_response()` (P6),
  giving `-32602` with `data.field = "languages"` before the host call. It
  passes `as_str()` strings to `AppConversation::send`. With
  `skip_serializing_if`, the transaction service's two card sends (Q5 (a))
  add `languages: Vec::new()` and send the same JSON as before.
- **D-B3b-7. The Roym output.** `MessageView` gains `pub languages:
  Vec<String>` after `content_type`, always serialized (`[]` when none), so
  a client reads one shape. `From<&Message>` copies it; the membership and
  group-name rows (`messages.rs:320`, `:342`) set `Vec::new()`. The values
  were checked by the host, so the view does not parse them again.
- **D-B3b-8. Both builds.** The WIT change (W1, W2) is followed in every
  layer of section 2.2 point 2, in one step (Step 3), because the generated
  types change at once. The `on-message` JSON in `engine/auth.rs` gains
  `"languages": msg.languages`. The dual-build fixture's `SendMessage`
  passes `Vec::new()`; the parity of the tags is tested through the Roym
  services (section 7.2).
- **D-B3b-9. `roymctl roym group send --lang`.** `GroupCommands::Send`
  gains `#[arg(long = "lang")] langs: Vec<String>`. Each value goes through
  `LangTag::parse` (lenient: `HI-latn` becomes `hi-Latn`), and the params
  get `"languages"` only when the list is not empty. A bad tag stops before
  the call with the `LangTagError` text, as `--lang` does for requests
  (B3a D-B3a-7). The count check is the service's. The params are built in
  a function `build_send_params(group, body, langs) -> Result<Value>` so a
  unit test can call it.
- **D-B3b-10. Hub: one language list component** (Q3 (a)). New file
  `src/i18n/language_list.ts`:

  ```ts
  export interface LanguageListLabels { add: string; moveUp: string; remove: string }
  export interface LanguageList {
    /** A <div> that holds the <ul> of rows and the "add" <select>. */
    root: HTMLElement;
    /** The chosen tags, in order. */
    value(): string[];
    /** Replaces the chosen tags and draws the rows again. */
    set(tags: string[]): void;
  }
  /// An ordered list of language tags from RECORD_LANGUAGES, at most `max`:
  /// each row has the language name, "move up" (not on the first row) and
  /// "remove"; the "add" <select> offers the languages not yet chosen and is
  /// disabled at `max`. Every button has type="button". The class names are
  /// `${prefix}-languages-list`, `${prefix}-language-row`,
  /// `${prefix}-language-name`, `${prefix}-language-move-up`,
  /// `${prefix}-language-remove` and `${prefix}-languages-add`.
  export function languageList(
    prefix: string, max: number, initial: string[], labels: LanguageListLabels,
    groupLabel: string,
  ): LanguageList;
  ```

  Accessibility (P8): `root` gets `role="group"` and `aria-label` set to
  `groupLabel` (`t("messages-languages-label")` or `t("group-languages-label")`,
  or the profile's label). The "add" `<select>` also gets `aria-label` set to
  `groupLabel`.
  The profile screen uses it with the prefix `profile`, `max` 8, and its own
  label, so its class names and its rows stay as they are (`profile.test.ts`
  and the e2e step `saveProfile` pass unchanged); its stored list goes through
  `set`.
  The 1:1 chat box uses the prefix `messages` and the group chat box the
  prefix `group`, both with `max` 3 and `initial` `[currentLanguage()]`.
  The list lives as long as the open thread: it keeps its rows between
  sends and starts again at the Hub language when the thread is drawn
  again. The chat boxes offer only `RECORD_LANGUAGES`, the list the owner
  chose for records (B3a Q6).
- **D-B3b-11. Hub: drawing the message text.** New file
  `src/chat/message_text.ts`:

  ```ts
  /// The text of a chat message: a `tag` element (default "span") of class "message-body" with the
  /// body, and `lang` from the first tag (langAttr), or "" when there is no
  /// tag. When `languages` is empty, a second <span class="message-language-note">
  /// with "Language not stated". Returns the nodes to append.
  export function messageText(body: string, languages: readonly string[], tag?: string): Node[];
  /// The `lang` value for a message: langAttr of the first tag, or "".
  export function messageLang(languages: readonly string[]): string;
  ```

  `messageText` is used only when `body_encoding === "utf8"` (P5). A binary row
  (`body_encoding === "base64"`) is drawn as today, with no `lang` attribute
  and no note.
  `messages.ts` (`:533`, with `span`) and `groups.ts` (`:114`, with `div`) call
  `messageText` for text messages; the search hit (`messages.ts:152`) sets
  `lang = messageLang(h.languages)` on the snippet. `MessageRow`,
  `GroupMessageRow` and `SearchMatch` gain `languages?: string[]`; a missing
  field is read as `[]`. A card, a deleted message, a binary message and a
  group system row (membership, group name) are not drawn through
  `messageText`, so they get no note. With Q4 (a), a tagged message shows no
  language name. `lang=""` is the HTML way to say "the language is not known";
  a message with no tag must not inherit the page language, which would mark
  a Tamil text as Hindi.
- **D-B3b-12. Hub messages.** New ids, English and Hindi, each added in the
  step that first uses it: `messages-languages-add`,
  `messages-languages-move-up`, `messages-languages-remove`,
  `group-languages-add`, `group-languages-move-up`,
  `group-languages-remove`, and `chat-text-not-stated` ("Language not
  stated"). The prefix `chat-text-` names a shared part, as
  `record-text-` does (B3a D-B3a-10). Each screen has its own add, move-up
  and remove ids (B1b "Shared messages"). A visible label for the list
  is the `field()` label `messages-languages-label` / `group-languages-label`
  ("Message languages").
- **D-B3b-13. A helper for the incoming group row, with no change of
  behaviour.** The `StoredMessage` literal of `apply_message_entry`
  (`entry.rs:186` to `:210`) moves into
  `fn incoming_group_row(conv_id: &str, entry: &StoredDagEntry, content: Content, now: i64, next_notify_at: i64) -> StoredMessage`
  in `entry.rs`, where `Content` is `(String, Vec<String>, Vec<u8>)` as
  `decode_body` returns it. Step 3.1 does this first, before any new field.
- **D-B3b-14. Test data.** Hub fixtures that draw an incoming text message
  in `messages.test.ts`, `groups.test.ts` and `hindi-screens.test.ts` get
  `languages` where a test is about tags, and keep no `languages` where it
  is not (they then show the note, which the Hindi sweep also checks).
  `cards/samples.json` does not change.

## 6. Step-by-step changes

Each step builds and its tests pass. Rust: `cargo nextest run -p <crate>`
for the crates the step touches. Hub: `npm run build` and
`mise run test:roym-ui` in `crates/roym_web/ui`. Paths are from the
repository root.

### Step 1 — Commit this plan

Done in the planning session: this file and the B3b row of
[status.md](status.md).

### Step 2 — The rule in `syneroym-text-lang`

1. `crates/text_lang/src/lib.rs`: `MAX_MESSAGE_LANGUAGES`, `TagListError`,
   `check_tag_list`, `MessageLanguagesError`, `parse_message_languages`
   (D-B3b-1).
2. `crates/roym_core/src/person.rs`: `validate` calls `check_tag_list` and
   maps its errors (D-B3b-1). The person tests run unchanged.
3. Tests of section 7.2 ("text_lang").
4. `cargo nextest run -p syneroym-text-lang -p syneroym-roym-core`.

### Step 3a — Host internals and storage

1. `crates/conversation/src/group/entry.rs`: `incoming_group_row`
   (D-B3b-13), with no new field yet. `cargo nextest run -p
   syneroym-conversation`: the same tests pass, the same number.
2. `crates/conversation`:
   - `Cargo.toml`: `syneroym-text-lang.workspace = true`.
   - `envelope.rs`: `put_languages`, `take_languages` (with length overflow
     checks, D-B3b-2), the field in `DeliveryPayload`, `canonical_bytes`,
     `sign` (`#[allow(clippy::too_many_arguments)]`, P2), `verify` (D-B3b-2);
     its tests.
   - `dag.rs`: `encode_body`, `decode_body` (safe lengths with `checked_add`
     and slice `get`, D-B3b-2, P1); tests in its existing `mod tests`
     (`dag.rs:374`).
   - `store/schema.rs`, `store.rs` (`StoredMessage`, `into_wire`),
     `store/message.rs` (`MESSAGE_COLUMNS`, the three insert functions,
     `row_to_message`), `store/history.rs`, `store/search.rs`,
     `store/backup.rs` (named insert in `backup.rs:397`, D-B3b-5, P3).
   - `lib.rs`: `enqueue_direct` (`#[allow(clippy::too_many_arguments)]`, P2;
     with `languages: &[String]`; the system sends pass `&[]`) and
     `deliver_inbound`, `group.rs` `send_group` and
     `record_outgoing_group_message`, `group/entry.rs` `apply_message_entry`
     (raw `INSERT` writes `languages`, P3; `tracing::warn!` on bad tag refusal,
     P4; withdrawn row stores `''`, P3), `transport.rs` `sealed_envelope`
     (`:211`) and `verify_incoming_envelope` (D-B3b-4).
   - Every internal test caller in `crates/conversation`: pass `vec![]` /
     `&[]` unless the test is about tags. New tests of section 7.2 ("host").
3. `cargo nextest run -p syneroym-conversation`.

### Step 3b — The WIT and both builds

1. WIT (W1, W2): `crates/wit_interfaces/wit/conversation/conversation.wit`
   `message` and `send`, with a doc line on each: "Language tags of the
   text, main language first, at most 3, each in canonical form. Empty
   when not stated."
2. `syneroym-rpc`: `ConversationMessage.languages`, `ConversationHost::send`
   (W3).
3. `crates/conversation`: `host_impl.rs` `send` takes `languages: Vec<String>`,
   runs `parse_message_languages` (D-B3b-4); tests in `host_impl/tests.rs`.
4. `sandbox_wasm`: `capabilities_messaging.rs` (`send`, `map_message`),
   `host_capabilities.rs:143`, `engine/auth.rs` (D-B3b-8).
5. `app_host`: `lib.rs:324`, `guest.rs:241`. `app_host_native`:
   `host.rs:399`, `convert/conversation.rs` (`message_out`,
   `rpc_message_to_guest`). `control_plane`:
   `synsvc_native/conversation.rs:33`. `roym_core`: the one test host in
   `signing.rs:516` (P7), the `Message` literal in `method/tests.rs:438`.
6. `roym_conversation` `messages.rs:295`: pass `Vec::new()` for now
   (Step 4 sends the input's list).
7. `test-components/dual-build-fixture/src/app/dispatch.rs:194`: pass
   `Vec::new()`. `app_host_native/tests/dual_build_parity/conversation.rs`
   (the `DeliveryPayload` literal, `:368` to `:385`).
8. `mise run build:test-components`, `mise run build:roym`, then
   `cargo nextest run -p syneroym-conversation -p syneroym-rpc
   -p syneroym-sandbox-wasm -p syneroym-app-host -p syneroym-app-host-native
   -p syneroym-control-plane -p syneroym-roym-core -p syneroym-roym-web`
   (output to a file, AGENTS.md "Context Budget").

### Step 4 — Roym input, output and `roymctl`

1. `roym_core`: `SendInput.languages` (D-B3b-6), `MessageView.languages`
   (D-B3b-7), its tests (`conversation.rs:246`,
   `conversation/group/tests.rs:7`, `method/tests.rs:437`).
2. `roym_conversation` `messages.rs`: the `check_tag_list` call returning
   `ParamError` with `field = "languages"` (P6), and the list passed to the
   host (D-B3b-6); the two hand-built views (D-B3b-7).
3. `roym_transaction` `app.rs:538`, `agreement_ops.rs:482`:
   `languages: Vec::new()` (Q5 (a)).
4. `roymctl` (D-B3b-9) and its unit test.
5. `method_contract` (section 7.2) and
   `UPDATE_METHOD_SHAPES=1 cargo nextest run -p syneroym-roym-web --test
   method_contract`; read the diff of `crates/roym_web/tests/method_shapes/`
   (section 2.2 point 15); run again without the variable.
6. Parity tests and the two-node test of section 7.2.
7. `mise run build:roym`, then
   `cargo nextest run -p syneroym-roym-core -p syneroym-roym-conversation
   -p syneroym-roym-transaction -p syneroym-roym-web -p roymctl`, then the
   substrate tests `roym_conversation_e2e` and `roym_group_e2e` (output to
   a file).

### Step 5 — Hub

1. `src/i18n/language_list.ts` (D-B3b-10, with `role="group"` and
   `aria-label`s, P8); `screens/profile.ts` uses it. `profile.test.ts`
   passes unchanged. New `language_list.test.ts`.
2. `src/chat/message_text.ts` (D-B3b-11, for `body_encoding === "utf8"`, P5)
   and `chat-text-not-stated`.
3. `screens/messages.ts`: the chat box list, `languages` in the send, the
   text through `messageText`, the search snippet `lang`. `screens/groups.ts`:
   the same for the group chat box and `renderMessageRow`.
   `screens/message_search.ts`: `SearchMatch.languages`, carried into the
   hit.
4. Messages of D-B3b-12 in `en.ftl` and `hi.ftl`.
5. Tests of section 7.1.

### Step 6 — e2e and story 40

1. `crates/substrate/tests/e2e/hub-helpers.ts`: `openMessages`,
   `openConversation` and `sendMessage` take an optional `tag = "en"` and
   find their buttons by `hubMessage(tag, …)` (`tab-messages`,
   `messages-open-button`, `messages-send-button`). A new
   `addMessageLanguage(page, lang, tag = "en")` picks `lang` in
   `.messages-languages-add`. Existing callers do not change.
2. `roym-hub.spec.ts`: the new test of section 7.3.
3. Story 40 (Q6 (a)): one more step (section 7.3).
4. `mise run test:e2e` and `mise run demo`, output redirected; keep the
   summaries for the PR text.

### Step 7 — Sizes, docs and close-out

1. Size (section 7.6): before (on `main`) and after, in the same build
   folder: `npm run build`, and for each `dist/assets/*.js` the raw bytes
   and `gzip -9 -c | wc -c`; the six Roym `.wasm` sizes from
   `mise run build:roym`. Write the numbers into task.md (B3b result line).
2. Change doc: decisions into "C. Language-tagged free text" as "Decided
   <date>, chat message languages" (every D-B3b item that a reader of the
   change doc needs: D-B3b-1, -2, -4, -5, -6, -7, -10, -11, -12); the
   deviations of section 3; the Test column of ML-14 and ML-25 with the
   real test names; the "Chat messages" paragraph says "`languages`, at
   most 3"; `demo:` stays story 40; `status` stays `draft` (B6 sets
   `done`); `living-docs-touched` keeps its list (all files of section 9
   are on it already).
3. Living docs (section 9), deferred backlog (section 8), task.md B3b
   result line, status.md.
4. The import clean-up pass over every Rust file you edited.
5. `cargo dupes report` over the changed Rust files (AGENTS.md recipe).
   `put_languages` is the one writer of the list; no new duplicate group.
6. `mise run verify` with no `--skip`, as a background job.

## 7. Tests

### 7.1 Hub unit tests (vitest, `TZ=UTC`)

`src/i18n/language_list.test.ts`

- Starts with `initial`; `value()` gives the tags in order.
- "Move up" on row 2 makes it row 1; the first row has no "move up".
- "Remove" drops the row; removing every row gives `[]`.
- The add list offers only `RECORD_LANGUAGES` not yet chosen, and is
  disabled at `max` (3 and 8).
- Every button has `type="button"`; a click inside a `<form>` does not
  submit it.
- `set(["ta", "en"])` draws two rows in that order.
- The class names follow the prefix (`messages-language-row`).
- Accessibility: root has `role="group"` and `aria-label` matching
  `groupLabel`; "add" `<select>` has matching `aria-label` (P8).

`src/chat/message_text.test.ts`

- `["hi", "en"]`: the body span has `lang="hi"` and the text as given (a
  mixed Hindi and English string, byte for byte); no note.
- `[]`: `lang=""` and the note "Language not stated"; in Hindi, the note
  has a Devanagari letter.
- `["hi_IN"]` (a tag `Intl.Locale` refuses): `lang=""`, no note.
- `messageLang([])` is `""`; `messageLang(["ta-Latn"])` is `ta-Latn`.

`src/screens/messages.test.ts`

- The 1:1 chat box shows one language row at the UI language; Send calls
  `conversation.send` with `languages: ["en"]`; after "add Hindi" and
  "move up", with `["hi", "en"]`; after removing every row, with `[]`.
- After a send, the list keeps its rows.
- An incoming text message with `languages: ["ta"]` has `lang="ta"`; one
  without `languages` shows the note; a card message and a deleted message
  show no note; a binary message (`body_encoding: "base64"`) has no note
  and no `lang` (P5).
- A search hit with `languages: ["hi"]` has a snippet with `lang="hi"`.

`src/screens/groups.test.ts`

- The same for the group chat box and `renderMessageRow`; a membership row
  and a group-name row show no note; a binary message
  (`body_encoding: "base64"`) has no note and no `lang` (P5).
- The existing test "retry handler … never calls conversation.send" passes
  unchanged.

`src/screens/profile.test.ts` runs unchanged (the list moved, D-B3b-10).

`src/i18n/hindi-screens.test.ts`

- The messages screen and the groups screen, each with one tagged and one
  untagged text message and the chat box list, drawn in Hindi: no Latin
  word is left (language names come from `Intl.DisplayNames` in Hindi; if
  one comes back in Latin letters in Node's ICU, stop and tell the owner,
  B3a D-B3a-15).

`src/i18n/catalog.test.ts` runs unchanged: every new id is used, and
`hi.ftl` has the ids of `en.ftl`.

### 7.2 Rust

`crates/text_lang/src/tests.rs` ("text_lang")

- `check_tag_list`: `[]`, `["hi"]` and 3 tags pass; 4 tags give
  `TooMany(3)`; `["hi", "hi"]` gives `Duplicate("hi")`; `["hi", "hi-IN"]`
  passes.
- `parse_message_languages`: `["hi", "en"]` gives two tags; `["HI"]` gives
  `Tag(NotCanonical)`; `["hi_IN"]` gives `Tag(Malformed)`; 4 tags give
  `List(TooMany(3))`.

`crates/roym_core` — the person tests run unchanged (Step 2).

Host (`crates/conversation`)

- `envelope.rs` tests: a payload with `["hi", "en"]` verifies; changing one
  tag, removing one, or swapping their order fails verification;
  `canonical_bytes` for `["ab"]` and for `["a", "b"]` differ, and for
  `["a"]` with body `b"b"` and `["ab"]` with an empty body differ.
- `dag.rs` tests: `encode_body` / `decode_body` round trip with 0, 1 and 3
  tags; a count of 4, a truncated tag and a non-canonical tag are `Err`;
  `decode_body` with `ct_len = u64::MAX`, with `ct_len = usize::MAX - 7`,
  a tag length of `u64::MAX`, and a count of `u64::MAX` returns `Err` and
  does not panic (P1).
- `host_impl/tests.rs`: `send` with `["hi", "en"]` and a body that mixes
  Devanagari and Latin letters; `get_message` and `history` give the same
  body bytes and the same list. `send` with `["HI"]`, with 4 tags and with
  `["hi", "hi"]` is `InvalidArgument` whose text starts with
  "message languages:", and stores nothing.
- `transport/tests/delivery.rs` (not `transport/tests.rs`, which is near
  800 lines): a 1:1 message with `["ta", "en"]` reaches the receiver's
  store with the same list; a payload correctly signed with
  `["HI"]` is refused with `InvalidArgument` and not stored.
- `group/tests/languages.rs` (new, `mod languages;` in `group/tests.rs`):
  a group message with `["hi", "en"]` reaches a member with the same list;
  reading the stored row from the database (not just the notification)
  confirms `languages` is stored (P3); an entry whose plaintext has a
  non-canonical tag is not applied and logs a warning (P4).
- `store/tests/backup_roundtrip.rs`: the list survives export and import;
  an import with `"languages": ["HI"]` is refused.
- `store/tests.rs`: a row inserted by hand without the column reads back
  `languages: []`.

Services and parity

- `method_contract` `contract.rs`: `conversation.send` with
  `"languages": ["hi_IN"]` answers `-32602` with
  `data.field = "languages[0]"`; with 4 tags and with `["hi", "hi"]`,
  `-32602` with `data.field = "languages"`; with `[]` and without the
  field, a message id. `shapes.rs:324`: the first send gets
  `"languages": ["en"]`, so the history shapes show a non-empty list.
- `roym_core`: `SendInput` with no `languages` serializes without the key
  (the card JSON does not change, D-B3b-6); `MessageView` with an empty
  list writes `"languages": []`.
- Parity `crates/roym_web/tests/dual_build_parity/conversation.rs`: a send
  with `["hi", "en"]` gives the same `conversation.history` on both builds,
  with the list. `group.rs`: a group message with `["ta"]` gives the same
  history on both builds.
- `crates/substrate/tests/roym_conversation_e2e.rs` (two nodes, Step 5 of
  that test, `:150` to `:156`): the send gets `"languages": ["hi", "en"]`,
  and B's copy has `languages == ["hi", "en"]` (ML-25 between nodes).
- `roymctl`: `build_send_params` with `--lang HI-latn --lang en` sends
  `["hi-Latn", "en"]`; with no `--lang`, no `languages` key; with
  `--lang hi_IN`, the `Malformed` text and no call.

### 7.3 Playwright

- Every spec runs with the Hub sending `languages` (Step 5): no step
  changes for it (section 2.2 point 14).
- `roym-hub.spec.ts`, new test "a message in two languages keeps its text
  and has its main language" (ML-25, ML-14). The Hub stays in English. Open
  a conversation with the never-answering peer, as the test at `:376`
  does. The language list shows English. `addMessageLanguage(page, "hi")`,
  then click "move up" on the Hindi row. Send `नमस्ते, see you at 5 ${tag}`.
  The message's `.message-body` has the text as typed and `lang="hi"`, and
  no `.message-language-note`. Reload, open the same conversation: still
  `lang="hi"` (read back from the store, not from the box).
- Story 40 (Q6 (a)): after the listing step, Alice (Hub in Hindi) calls
  `openMessages(page, "hi")` and `openConversation(page, bobAddr, "hi")`;
  the first `.messages-language-row` shows the Hindi name of `hi`; she
  calls `addMessageLanguage(page, "en", "hi")` and
  `sendMessage(page, text, "hi")` with a text of Hindi and English words
  and a run tag (as story 20 does); her message has `lang="hi"`. The story
  reads `BOB_ADDR` with `requireEnv`.

### 7.4 Not tested

- A screen reader reading a `lang="hi"` message as Hindi, and font choice
  per script (change doc, "Fonts").
- A message between a node with B3b and a node without it (W4: it fails by
  design; no test keeps old nodes).

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `crates/conversation/src/crypto.rs` | 786 | +1 (a `DeliveryPayload` literal in its tests). Limit 800 |
| `crates/conversation/src/transport/tests.rs` | 781 | about +4 (three literals, one call). New tests go to `transport/tests/delivery.rs` (279). Limit 800 |
| `crates/conversation/src/group/tests.rs` | 766 | about +3 (`mod languages;`, calls). New tests go to `group/tests/languages.rs`. Limit 800 |
| `crates/conversation/src/transport.rs` | 752 | about +8. Limit 800 |
| `crates/conversation/src/store/message.rs` | 737 | about -10 (`MESSAGE_COLUMNS`), then +8. Limit 800 |
| `crates/conversation/src/group.rs` | 735 | about +4. Limit 800 |
| `crates/conversation/src/store/tests.rs` | 717 | about +10 (one `&[]` per call). Limit 800 |
| `crates/conversation/src/host_impl/tests/events.rs` | 715 | about +12 (wrapped send calls). Limit 800 |
| `crates/conversation/src/group/entry.rs` | 469 | about +10 (helper). `apply_message_entry` gets shorter (D-B3b-13) |
| `crates/roym_web/tests/dual_build_parity/fixtures.rs` | 798 | +1 (`languages: vec![]` in `inbound`). Limit 800 — put nothing else here |
| `crates/roym_core/src/method/tests.rs` | 727 | about +2 |
| `crates/roym_web/ui/src/screens/messages.ts` | 931 | about +15 (TypeScript has no length gate) |
| `crates/roym_web/ui/src/screens/groups.ts` | 694 | about +12 |
| `crates/roym_web/ui/src/screens/profile.ts` | 171 | about -60 (the list moves out) |
| `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` | 1420 | about +35 (TypeScript, no gate) |

None of the capped files of `xtask/oversized-test-files.txt` changes
(`app_host_native` and `roym_web` parity `helpers.rs`, `directory.rs`,
`profile.rs`, `transaction.rs`, `sandbox_wasm/src/engine/tests.rs`). If
one gets shorter anyway, set its new count (AGENTS.md "Mandatory Ratchet
Update"). New Rust code adds to the workspace size, so the duplication
percent may fall; if `cargo xtask check-duplication` says so, lower
`MAX_EXACT_DUPLICATE_PERCENT` as its message says. Never raise it.

### 7.6 Bundle and component size

- Hub: the main script gains `language_list.ts` (moved, not new),
  `message_text.ts` and 9 English messages. Expected under 2 KB more after
  `gzip -9` (after B3a it was 51,902 bytes). The Hindi catalog grows by
  under 1 KB (14,377 after B3a). If the main script grows by more than
  5 KB, stop and tell the owner.
- Guests ([task.md](task.md) "Component size limit"): conversation gets the
  new input and output field and the list check, so it is a record-code
  component: stop if it grows by more than 15% (769,589 bytes after B3a).
  Transaction gets one empty field in a struct literal, and web, profile,
  catalog and directory get nothing new: each of these is a shared-code
  component, stop if one grows by more than 40 KB.

### 7.7 Hindi text

As in B1b to B3a: common words a Hindi speaker reads on a phone, the same
Hindi word for the same thing as `hi.ftl` already uses (the profile's
"add a language", "move up" and "remove"). The owner reviews the new Hindi
lines before the merge; the PR text lists them (9 messages), so no review
file is needed.

### 7.8 How B3b avoids the kinds of findings S0 to B3a had

- **A function that crosses the clippy line limit** (B3a P3):
  `apply_message_entry` gets its helper first (Step 3.1), and the receive
  check goes where there is room (D-B3b-4).
- **Two copies of one helper** (B1b R5, R7; B2 R7; B3a R8, R14): one tag
  list check (`check_tag_list`, also used by the profile), one list writer
  (`put_languages`, for the 1:1 bytes and the group plaintext), one column
  list (`MESSAGE_COLUMNS`), one Hub list component (moved from the profile),
  one message text helper for both chat screens.
- **A step that breaks a later step or the e2e suite** (B1b P4, P10; B3a
  P4): the WIT and every layer change in one step (Step 3); the Hub adds no
  `<input>` and no "Send" button to a chat box, and every new button has
  `type="button"` (section 2.2 point 14).
- **A refused tag gives no `lang`** (B3a R6): `lang=""` for a refused tag
  and for no tag, tested (section 7.1).
- **The Hindi sweep never draws the new parts** (B3a R4): it draws both
  chat boxes, a tagged and an untagged message (section 7.1).
- **The change doc's Test column names tests that do not exist** (B3a R2):
  the names are written at close-out from the real files (section 3).
- **A claim not checked in the code** (S0 P1): every line here is from
  `4830bdf7`; caller counts are from `git grep`.
- **A test name or comment with a planning id** (B2 R5, B3a R7): none.
- **Component size over the stop limit** (B3a R13): the two-part limit of
  task.md, applied per component (section 7.6).
- **The change doc missing decisions or deviations** (B2 R8, B3a R10):
  Step 7.2 names which D-B3b items and deviations go there.

## 8. Deferred backlog

In [deferred-backlog.md](../../deferred-backlog.md):

- **§10, row "Language tags are not CLDR-canonicalised"** (line 536): add
  one sentence: chat messages follow the same rule; a receiver refuses a
  message whose tags are not in canonical form (Q1 (a)), so an
  `icu_locale_core` upgrade that changes printer output also needs an owner
  decision for chats. Add the link `crates/conversation/src/transport.rs`
  (`verify_incoming_envelope`).
- **§10, new row "The chat box offers only the record languages"**
  (D-B3b-10): a chat language outside `RECORD_LANGUAGES` can be sent
  through the API only. Target `TBD`. Link
  `crates/roym_web/ui/src/i18n/language_list.ts`.
- **§10, new row "A tagged chat message shows no language name"** (Q4
  (a)): the reader sees the language only through the script; B5 adds the
  translate action at the chat hook point. Target M08B B5 (decide there).
  Link `crates/roym_web/ui/src/chat/message_text.ts`. Drop this row if the
  owner picks Q4 (b).
- **§10, new row "A group entry with a bad language list is retried on
  every pass"** (P4): an entry whose plaintext has a bad tag stays unapplied
  and is decrypted on every pass. Target `TBD`. Link
  `crates/conversation/src/group/entry.rs`.
- **§11, new row "Chat search does not use the message languages"**: B4b
  builds search keys from the body; the tags are not used to pick a
  transliteration table. Target M08B B4b (decide there). Link
  `crates/conversation/src/store/search.rs`.

The row "Accessibility (WCAG 2.1 AA) & localisation/i18n" (§10, line 509)
stays: B6 moves its localisation part. No new `TODO` or `FIXME` marker. If
an owner answer is not the recommendation, drop or change the matching
row.

## 9. Living docs to update at close-out

- **`docs/system-requirements-spec.md:1462`** ("Hub localisation (built)"):
  one sentence: a chat message carries up to 3 language tags, main
  language first; the Hub sets them from a list in each chat box, marks
  the message text with the main language, and shows "Language not stated"
  for a message without one.
- **`docs/system-architecture.md`**: the Messaging section (`:964`), one
  sentence: each message carries its language tags inside the signed 1:1
  payload or the sealed group entry, and the host checks them on send and
  on receipt. The crate list (`:568`): `syneroym-text-lang` is used by
  `roym_core` and `conversation`; `data_db` will use it.
- **`docs/planning/traceability-matrix.md:68`** (`[APP-A11Y]`): add the chat
  tags and their evidence (`host_impl/tests.rs`, `message_text.test.ts`).
- **`docs/developer-guide.md`**: the block `roym-client=open-send`
  (`:2440`) sends `"languages":["en"]` (story 30 runs it, so the block
  proves the field); the sentence at `:2446` names the optional
  `languages` input (0 to 3 tags in canonical form, main language first);
  the list of message fields at `:2448` adds `languages`. Near "Record
  languages" (`:1781`), a paragraph "Chat languages": `languageList`,
  `messageText`, and that every new place that draws a chat message uses
  `messageText`.
- **`docs/demo/demo.md`**: R3b (`:1260`) adds the chat step of story 40;
  R6 (`:1360`) may add `--lang en` to `roym group send`.
- **`docs/TERMINOLOGY.md`**: the row "Language tag" (`:72`) adds that a chat
  message carries up to 3.
- **The change doc**, **task.md**, **status.md**: Step 7.

`docs/roym-integrated-experience-spec.md`: no change in B3b; B6 updates it
(task.md, B6).

## 10. Exit criteria

B3b is done when all of these are true:

1. The owner answered Q1 to Q6, and sections 3 to 9 match the answers.
2. A message with 0 to 3 tags is sent, delivered and read back with the
   same list, 1:1 and group, on both builds, and between two nodes.
3. The host refuses 4 tags, a repeated tag and a non-canonical tag on send,
   on receipt (1:1 and group) and on backup import.
4. `conversation.send` takes `languages`; `MessageView` returns it; the
   card JSON of the transaction service is unchanged.
5. `roymctl roym group send` takes `--lang`.
6. The Hub has the language list in both chat boxes (and the profile still
   works the same), sends the list, sets `lang` from the first tag (`""`
   when none), and shows "Language not stated" for a text message with no
   tag.
7. The Hindi sweep passes with the new parts.
8. The new Playwright test and story 40 pass.
9. The method shape diff holds only `languages` changes.
10. The bundle and component sizes are reported and within section 7.6.
11. The owner reviewed the new Hindi lines.
12. The change doc, the living docs, the backlog, task.md and status.md are
    updated as in sections 8 and 9.
13. `mise run verify` passes with no `--skip`.
14. One PR.

## 11. What B3b does not do

- No search over message languages, no search keys, no short-word index
  (B4a, B4b).
- No machine translation, no translate action at the chat hook point (B5).
- No language name shown for a tagged message (Q4 (a)).
- No tags on cards or system messages (Q5 (a)).
- No language detection: the Hub never guesses a tag from the text.
- No CLDR form: `iw` stays `iw` (backlog row).
- No migration of a `conversation.db` made before B3b, and no exchange of
  messages with a node without B3b (W4, W7).
- No change to message ids, ordering, delivery states or the transcript
  digest.
- No right-to-left layout.

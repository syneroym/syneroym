# M08C slice C7 — Message forwarding: implementation plan

*Reader: the developer (or AI) who implements slice C7, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-25, by Claude Opus 5.5. Not reviewed yet.
The owner has not answered section 2.1 yet.**

**Size and PRs.** [task.md](task.md) sizes C7 as M. Checked against the
code, the forward of signed records (FW-7) is a second, separate piece of
work: it has its own content type, three sources in the Hub (a quote card,
a directory hit, a profile) and its own checks for three record types. So
this plan splits C7 in two (Q2). Each part is one PR that passes
`mise run verify`:

- **C7a — Forward mode, origin and forward of a text message (M).** The
  `forward_mode` in each message the Hub sends, the default-mode setting,
  the `message-origin` record, the check at read time,
  `conversation.forward`, and in the Hub the composer control, the
  first-message question, the forward action, the display rules and the
  group notice; the demo step. Criteria FW-1 to FW-6, FW-8 to FW-15.
- **C7b — Forward of signed records (S).** A `listing`, `profile` or
  `quote` travels as itself in a record-forward body. Criterion FW-7.

Branch: `feat/m08a-c7-msg-forwarding` (this branch; it holds the plan
commit; the owner chose the name). C7b gets its own branch, cut from `main`
after C7a merges.

**Base.** Every file path and line number below was checked on `main` at
`aa25961b` (2026-11-25), unless it names the C4 branch. Links are relative
to this folder.

**Dependencies.**

- C0 (the ADR) is done: [ADR-0030](../../../decisions/0030-signed-message-origins.md),
  accepted 2026-11-16 ([status.md](status.md), row C0).
- C2 is merged (C2a PR #370, C2b PR #371). C7b's forwarded quote needs it.
- **C4 is not merged.** [task.md](task.md) section 4: "C7 changes
  `conversation` and the Hub chat, like C4. So C7 and C4 must not run at
  the same time." C4 is being built now on `feat/m08c-c4-widgets`. Its plan
  is on that branch, not on `main`. C4 moves the body of
  `conversation.send` into a new function `send_body`, reserves two more
  content types, adds three `conversation` methods, and changes
  `messageElement`, `fillThread`, the group composer and story 20. These
  are the same places C7a changes. This plan is written so that **the C7a
  code starts after C4 merges** (Q1). Section 2.2, point 1 lists each place
  where C7a builds on C4.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - functions at most 100 lines; match arms at most about 10 lines;
  - production `.rs` files at most 800 lines, so new code goes in new
    sibling files (`crates/roym_conversation/src/app/messages.rs` is 537
    lines; `crates/roym_core/src/conversation.rs` is 371);
  - module layout: `foo.rs` beside `foo/`, never `mod.rs`;
  - comments say why, never which plan, slice or review finding;
  - import rules: types by `use`, functions through their parent module;
  - no hard-coded ports in tests (`common::alloc_ports`, `boot_trio`);
  - parity runs need a fresh `mise run build:roym`;
  - every new or changed Hindi message is listed for the owner's review.
- [The forwarding change doc](../../changes/roym-message-forwarding/change.md):
  criteria FW-1 to FW-15, and its `## Deviations`, which win over its main
  text.
- [ADR-0030](../../../decisions/0030-signed-message-origins.md). Its twelve
  decisions are fixed. This plan does not decide them again. It decides
  only what the ADR leaves to "the C7 plan": the body format, where the
  default is stored, and the display name of an author.
- [task.md](task.md) section 3 (C7) and section 1 (the component size rule).
- The C4 plan on branch `feat/m08c-c4-widgets`
  (`slice-c4-implementation-plan.md`), point 14: a widget and a widget
  answer carry no forward mode, so they are not forwardable. "The C7 plan
  decides whether that stays." It stays (D-C7-13).

### Terms used in this plan

- **Chat body:** the new JSON body of a Roym text message, content type
  `application/vnd.roym.message+json` (`ROYM_MESSAGE_CONTENT_TYPE`). It
  holds the text, the forward mode, the origin (for `signed`) and the
  forward count (for a forward).
- **Inner content:** what the origin names: the content type `text/plain`,
  the message's language tags and the UTF-8 text. It is not the chat body
  itself, because the chat body holds the origin and the count.
- **Origin:** a `message-origin` record, version 1 (ADR-0030, decision 6).
- **Check:** verify the origin envelope, check that it is a
  `message-origin` version 1 signed as a person, and check that its
  `content_hash` matches the inner content. An origin that passes is
  **checked**.
- **Forward view:** the new `forward` field of `MessageView`: what the node
  found when it read a chat body.
- **Source message:** the message a person forwards.
- **Settings row:** the person's default forward mode, and whether the Hub
  has asked the first-message question. `conversation` keeps it.

## 1. What C7 builds, in one paragraph

Each message the Hub sends gets a forward mode, `signed` or
`not-forwardable`, inside a new chat body. For a `signed` message, the
`conversation` service signs a `message-origin` record as the person, with
its existing record-signing certificate, and puts it in the body. When any
node reads a chat body (`conversation.history`, `.changes`, `.search`), it
checks the origin and reports a forward view. `conversation.forward` sends
a checked, `signed` source message to another chat with the same text, the
same origin and the count plus one. It refuses anything else. The Hub shows
the mode in the composer, asks for a default before the person's first
message, offers "Forward" only on checked `signed` messages, and shows
"Forwarded. First written by … on …. Forwarded N times." A message sent
without a mode (another client, `roymctl`, the client guide) is sent as
`text/plain`, exactly as today, and counts as `not-forwardable` (ADR-0030,
decision 3). C7b then lets a person forward an offering, a profile or a
quote as the signed record itself.

## How this is shown to an end user

C7a:

- The first time a person sends a message, the Hub asks one question:
  "Can others forward your messages with your name?" It explains the two
  answers in plain words. The person picks one, or skips. Skip means "not
  forwardable".
- Next to the send button, the person sees the mode for this message,
  "Signed" or "Not forwardable", and can change it for this one message.
  Their default is on the Profile tab.
- In a group, when the mode is "Signed", the composer also says: "Any
  member can show this message to anyone, with your name."
- On a signed message, the person sees "Forward". They pick a chat. The
  other person sees: "Forwarded. First written by Alice on 25 Nov 2026.
  Forwarded 1 time." From 5 forwards on it says "Forwarded many times".
- On a message that is not forwardable, there is no "Forward" button, and
  a short note says the author did not allow it. If a changed app forwards
  it anyway, the receiver sees it as the forwarder's own words, with the
  note "origin could not be checked".

C7b:

- On a quote card, an offering in the directory, or a contact's profile,
  the person sees "Forward". The receiver sees the offering, profile or
  quote card, checked as today. For a quote, the Hub first warns that the
  forward shows both people of the deal. The receiver sees no prices.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-core chat` (C7a): the body format,
  the content hash, the origin check and every display rule, with no node.
- `cargo nextest run -p syneroym-substrate --test roym_forward_e2e` (C7a):
  three nodes. Alice sends a signed message to Bob. Bob forwards it into a
  group with Carol. Carol's node reports a checked origin that names Alice,
  and `forward_count` 1. `conversation.forward` refuses a not-forwardable
  message.
- `mise run demo` (C7a): story 20 shows Bob's forward in a group on
  Alice's screen, with the line "Forwarded. First written by Alice …".

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation, marked **(recommended)**.

#### Q1. When does the C7a code start?

C4 changes the same functions (section 2.2, point 1).

- (a) **(recommended)** After C4 merges. This branch then rebases on
  `main`, and the implementer re-checks the line numbers that section 2.2,
  point 1 names. This is the rule of task.md section 4.
- (b) Beside C4, with a merge plan for `messages.rs`, `api.rs`,
  `messages.ts`, `groups.ts`, the `method_contract` count, scenario 73 and
  story 20. Each of these conflicts, and C4's `send_body` is the place
  where the C7a send belongs.

#### Q2. One PR or two?

- (a) **(recommended)** Two: C7a (FW-1 to FW-6, FW-8 to FW-15) and C7b
  (FW-7). C7b shares no code with C7a except the conversation picker of
  the forward button. It touches the quote check (`roym_core::transaction`),
  the profile check (`roym_profile`) and the directory screen, which C7a
  does not.
- (b) One PR, size L.

#### Q3. The chat body: which messages use it?

ADR-0030 leaves the body format to this plan. A forward mode and an origin
cannot ride in a `text/plain` body, so a new content type is needed.

- (a) **(recommended)** A message gets the chat body when the caller names
  a mode (`conversation.send` with `forward_mode`). The Hub always names
  one, so every Hub message has the mark (FW-1 and FW-13 say "marked"). A
  call with no mode is sent as `text/plain`, exactly as today. That keeps
  `roymctl roym group send`, the client guide (developer guide §6,
  `conversation.send` at line 2558), story 30 and every test that sends
  with no mode unchanged. Such a message counts as `not-forwardable`
  (ADR-0030, decision 3).
- (b) Only `signed` messages use the chat body. A `not-forwardable`
  message stays `text/plain`, with no mark. Fewer changes, but FW-1 says a
  `not-forwardable` message "has the mark", and the Hub could not tell the
  person's own choice from another client's message.
- (c) The service applies the person's default whenever the caller names
  no mode, so every message of every client gets the chat body. This
  changes the content type of every message that `roymctl` and the guide
  send.

#### Q4. Search over a chat body

The host indexes the whole body of a `+json` content type
(`is_searchable_content_type`, `crates/conversation/src/store/message.rs:36`;
the index call at `:134`). For a chat body, it would index the JSON keys
and the origin envelope. A search for "text", "signed" or "message" would
then match every Hub message.

- (a) **(recommended)** `conversation.search` keeps a chat-body match only
  if the inner text matches the query by the host's own rule (D-C7-8). The
  check is pure Rust in `roym_core`, built from
  `syneroym_text_lang::query_terms` (`crates/text_lang/src/search_key.rs:145`)
  and `fts::index_keys` (`crates/text_lang/src/fts.rs:30`). This links
  `search_key` (with its ICU data) into the `conversation` component for
  the first time. Step 1 measures the size first. **Stop rule:** if
  `conversation` grows by more than 60 KB from this alone, stop and ask the
  owner (then (b) or (c)).
- (b) Accept the false matches, and record a backlog row. The Hub's
  snippet is still the inner text (D-C7-6).
- (c) A host change: the host indexes only a named JSON field of a body.
  The change doc says "the substrate conversation capability does not
  change", so this needs the owner.

#### Q5. Where is the default mode stored?

ADR-0030: "Not decided here, and left to the C7 plan: where the default
setting is stored."

- (a) **(recommended)** In `conversation`: a new collection
  `conversation_settings` with one row `forward`:
  `{ "default_mode": "signed" | "not-forwardable", "asked": bool }`. Two new
  local methods read and write it (D-C7-4). It is in the `conversation`
  backup bundle as a new section `settings`, so a restore keeps the choice
  and the Hub does not ask again. `SCHEMA_VERSION` 4 → 5
  (`crates/roym_conversation/src/app.rs:31`). The service that signs the
  origin keeps the setting, and every client of the node sees the same
  answer.
- (b) In the browser (`localStorage`). Lost on another device or browser,
  and another client never sees it.
- (c) In `profile`'s `settings` collection
  (`crates/roym_profile/src/app.rs:33`). `conversation` never needs it from
  there, so there is no reason for the extra service.

#### Q6. When does a node check an origin?

- (a) **(recommended)** At read time, in `history`, `changes` and `search`,
  with a pure function (D-C7-6). There is no new stored state, so nothing
  can go stale, and a restored message is checked the same way. Cost: two
  ed25519 checks (certificate and record) per chat body that carries an
  origin. **Stop rule:** if a `conversation.history` page of 200 `signed`
  messages takes more than 500 ms in the WASM build (the epoch deadline is
  5 s), stop and ask (then (b)).
- (b) At arrival, in `on_message`
  (`crates/roym_conversation/src/app/inbox.rs:136`), with the result stored
  in a new collection. It needs a write per message, a row for the
  person's own sent messages too, and rules for backup and restore.

#### Q7. The demo step

Story 20 (`crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts`) has two
nodes, Alice and Bob (`demo/20-roym-setup.sh:51`, `:52`). A forward needs a
second chat.

- (a) **(recommended)** At her first message, Alice answers the question
  with "Signed". Bob creates a group "Market" with Alice and forwards her
  message there. Alice's screen shows it in the group: "Forwarded. First
  written by Alice on …. Forwarded 1 time." The story needs no third node.
  The steps come after C4's poll steps.
- (b) Add a third node (Carol) to the demo setup and forward to her. More
  natural, but the demo setup, its environment file and every story start
  get slower.

#### Q8. Attachments (FW-8)

Roym chat has no attachments today. `encode_body` says "Attachments are out
of scope for the first release, so in practice every stored body takes the
`Utf8` path" (`crates/roym_core/src/conversation.rs:121`). The WIT message
has no attachment field
(`crates/wit_interfaces/wit/conversation/conversation.wit:50`). The Hub has
no attach action.

- (a) **(recommended)** The content hash input has a list of blob hashes
  (ADR-0030, decision 6), and C7 always writes an empty list. A unit test
  proves that a different blob list gives a different hash. FW-8 is met in
  the format only. A Deviation row and a backlog row record it.
- (b) Build attachments first. Not in M08C.

#### Q9. The author's name (FW-3)

ADR-0030 leaves "the display name of an author" to this plan.

- (a) **(recommended)** The contact name, if the author's DID is in the
  reader's contacts. Otherwise the first 16 characters of the DID and
  "(not in your contacts)". The name never comes from the forward itself,
  so a forwarder cannot choose what the reader sees.
- (b) Always the DID.

#### Q10. Where is the default in the Hub?

The Hub has no Settings tab (`crates/roym_web/ui/src/main.ts:180` to `:191`).

- (a) **(recommended)** A section "Forwarding of my messages" on the Profile
  tab (`crates/roym_web/ui/src/screens/profile.ts`, 109 lines). It is a
  setting of the person.
- (b) A new Settings tab.

### 2.2 Points found while reading

1. **Where C7a builds on C4** (Q1). On the C4 branch, the body of
   `conversation.send` from `messages.rs:273` moves into
   `send_body(host, conversation, content_type, languages, body, check_reserved)`.
   The C7a send with a mode and `conversation.forward` both call
   `send_body`. Then the group checks (`restored`, `is_member`) and their
   order (parity scenarios 207 and 208 pin it) stay in one place. C4 also
   reserves `application/vnd.roym.widget+json` and
   `application/vnd.roym.widget-answer+json`; C7a adds
   `ROYM_MESSAGE_CONTENT_TYPE` to the same reserved list. C4 raises the
   `conversation` method count in
   `crates/roym_web/tests/method_contract/contract.rs:39` from 24 to 27,
   adds its verbs to scenario 73
   (`crates/roym_web/tests/dual_build_parity/wire_origin.rs`), uses parity
   numbers 260 to 265, and adds three steps to story 20. C7a takes the next
   free numbers. Where C4's merged code differs from this, the implementer
   follows C4's code and writes the difference into section 5 of this plan.
2. **The requirements spec says Roym supports attachments.**
   `docs/system-requirements-spec.md:291`: "The system supports one-to-one
   text, attachments, private group chat …". The code has none (Q8). The
   C7a close-out corrects the sentence, because FW-8 depends on it.
3. **The signing refusal words have several copies.** The function
   `resolve_principal_and_owner` exists in `roym_transaction`
   (`crates/roym_transaction/src/app.rs:307`, with a `data.code` on each
   refusal) and, without `data.code`, in `roym_catalog`
   (`crates/roym_catalog/src/app/listing_ops.rs:159`). `roym_profile`,
   `roym_directory` and `roym_core::backup` (`backup.rs:313`) map the same
   errors on their own. ADR-0030, decision 7, asks for "the same words as
   every other signing verb". C7a moves the `transaction` copy (the one
   with codes) into `roym_core::signing`, and uses it from `transaction`
   and `conversation`. The other copies stay; a backlog row records them
   (section 8).
4. **`roym_conversation` has no unit tests**, and the `AppConversation` of
   `TestHost` is a stub (`crates/roym_core/src/testing/stubs.rs:169`). So
   C7a puts every rule it can into pure functions in `roym_core`
   (unit-tested), and tests the service through the parity harness (both
   builds) and the three-node test.
5. **The parity harness pins the signing clock ahead of wall-clock time**
   (`crates/roym_web/tests/dual_build_parity/fixtures.rs:67`,
   `helpers.rs:1029`). A check at read time uses `clock::now_secs()` with
   `VerifyOptions::new(now)`, which refuses a record dated more than 300 s
   in the future (`crates/signed_record/src/verify.rs:91`, `:184`). Step 3
   confirms that the pinned clock is within 300 s of "now". If not, the
   service passes the pinned clock to the view; `forward_view` takes
   `now_secs` as a parameter for that reason.
6. **An origin must still check after its certificate expires.** The
   verifier checks that the record was signed inside the certificate's
   window, not that the window is still open (`verify.rs:206`; test at
   `verify.rs:463`). So a forward of an old message still checks. A unit
   test pins this for `message-origin` (section 7.1).
7. **`messages.ts` is 1,174 lines and `groups.ts` 727.** TS files are not
   checked by `check-file-lengths` (it reads only `*.rs`,
   `xtask/src/file_lengths.rs:398`), but an earlier review asked that
   `messages.ts` not grow (C1 R15). New Hub code goes into
   `crates/roym_web/ui/src/chat/`, which holds `message_text.ts` today.

## 3. What C7 meets

All criteria are in the [forwarding change doc](../../changes/roym-message-forwarding/change.md).
Its Deviations (from ADR-0030) win over its main text.

| Criterion | Part | How | Main test |
| --- | --- | --- | --- |
| FW-1 | C7a | A mode in every Hub message; an origin for `signed`; the payload as the Deviations say (no `author`, no `sent_at_secs`; `languages`) | `chat` unit tests; parity B, C |
| FW-2 | C7a | `conversation.forward` checks mode and origin itself; the Hub hides the action | parity G; `roym_forward_e2e` step 5 |
| FW-3 | C7a | Check at read; Hub line with name and date | `forward_view` unit tests; Hub unit test; story 20 |
| FW-4 | C7a | Failed check: the forwarder's own words plus a note; no author | `chat` unit tests; Hub unit test |
| FW-5 | C7a | Count ≥ 5 shows "Forwarded many times" | Hub unit test |
| FW-6 | C7a | Any change of text, languages or content type breaks the hash | `chat` unit tests |
| FW-7 | C7b | A record-forward body; `listing`, `profile`, `quote` only | C7b tests (section 6.2) |
| FW-8 | C7a | A blob-hash list in the hash input, always empty (Q8) | `chat` unit test; Deviation row |
| FW-9 | C7a | Forward to a one-to-one chat or a group, one method | `roym_forward_e2e` steps 3, 4 |
| FW-10 | C7a | First-message question; settings text | Hub unit test; Hub case; story 20 |
| FW-11 | C7a | Group composer notice; no group rule | Hub unit test |
| FW-12 | C7a | Composer mode control; one change does not change the default | Hub unit test |
| FW-13 | C7a | No forward action and a reason; a forged forward has no checked origin | Hub unit test; `chat` unit tests |
| FW-14 | C7a | The mode is in the body, under the sender's message signature | parity B (raw body holds the mode); D-C7-1 |
| FW-15 | C7a | A forward copies the author's mode and origin; the forwarder's default is not read | parity F |

## 4. Behaviour-change audit

### Wire changes (between nodes)

1. **New content type** `application/vnd.roym.message+json` for every
   message the Hub sends (Q3 (a)). A node on an older build shows it as an
   unknown JSON message. No compatibility path: the product is not
   released.
2. **New record type** `message-origin` version 1, inside a chat body.
   Signed as the person by the `conversation` service key, with the
   certificate in the envelope's `delegation` field (ADR-0030, decision 11).
3. **Body size:** a `signed` chat body is about 1 KB larger than its text
   (ADR estimate: about 940 bytes for the origin). C7a measures it
   (step 4). A short message still fits the mailbox estimate used for cards
   (`MAILBOX_CARD_BODY_BYTES` = 13 KiB, `crates/roym_core/src/card.rs:40`).

Nothing else on the wire changes: `text/plain` messages, cards, group
system messages, deletion requests and the conversation envelope
(`crates/conversation/src/envelope.rs`) stay the same.

### Local method changes (one node)

4. `conversation.send` takes an optional `forward_mode` (`"signed"` or
   `"not-forwardable"`). With it, `content_type` must be absent or
   `"text/plain"`, else `-32602` on field `content_type`. With no
   `forward_mode`, nothing changes.
5. `conversation.send` refuses `content_type`
   `"application/vnd.roym.message+json"` as reserved, like the card type
   (`messages.rs:287` to `:299`).
6. New methods `conversation.forward`, `conversation.settings` and
   `conversation.set-settings` (D-C7-4, D-C7-7). The `conversation` method
   table grows by 3.
7. `MessageView` (`crates/roym_core/src/conversation.rs:141`) gains
   `forward: Option<ForwardView>`. It is set only for a chat body, and left
   out of the JSON when `None`. **For a chat body, `body` is the inner
   text**, not the raw JSON (D-C7-6). `content_type` stays the wire content
   type.
8. `conversation.search` drops a chat-body match whose inner text does not
   match the query (Q4 (a)). So it can return fewer rows than `limit`.
9. The `conversation` backup bundle gets a section `settings`;
   `SCHEMA_VERSION` 4 → 5. A bundle of version 4 is refused on import, as
   for every schema change.
10. `RECORD_TYPES` (`crates/roym_core/src/record.rs:18`) gains
    `("message-origin", 1)`.
11. A `signed` send with no usable certificate is refused with `-32602` and
    `data.code` `signing-not-enrolled`, `signing-certificate-expired`,
    `signing-certificate-stale` or `signing-no-owner`: the words
    `transaction` uses today (`crates/roym_transaction/src/app.rs:307` to
    `:337`). The refusals of `transaction` do not change: the function
    moves, its words stay.

### Visible behaviour changes (Hub)

12. Composer: a mode control in the one-to-one and group composers; a
    group notice when the mode is `signed`.
13. A first-message dialog, once per person (per node).
14. Profile tab: a section with the default mode.
15. On each chat-body message: a "Signed" tag, a "Forward" button (checked
    `signed` only) or a "Not forwardable" note; the forwarded line; the
    "origin could not be checked" note.
16. A `text/plain` message (another client, or a message from before C7)
    shows the "Not forwardable" note too. Before C7 it showed no note.
17. The delete dialog of a `signed` message adds one sentence: the signed
    origin stays with everyone who has the message (ADR-0030, decision 12:
    "It cannot be taken back").
18. Search results show the inner text of a chat body (D-C7-6).

### Stays the same

- The substrate conversation capability, its WIT and its host search.
- `transaction` filing: it reads only card messages
  (`crates/roym_transaction/src/app/sync.rs:263`).
- Events: `conversation.message` events carry ids only
  (`crates/conversation/src/lib.rs:184`).
- `conversation.retry` resends the stored body unchanged. A retried
  `signed` message carries its first origin and is not signed again.
- Delete, admission, first-contact limits, blocks, group visibility.
- Signing enrolment: `conversation` is already in the Hub's
  `SIGNING_SERVICES` (`crates/roym_web/ui/src/session/enrolment.ts:7`) and
  in the list of `roymctl` (`apps/roymctl/src/commands/roym/signing.rs:13`).
- `roymctl`: no command changes (D-C7-12).
- Widgets and widget answers (C4) have no mode and are not forwardable
  (D-C7-13).

## 5. Decisions this plan makes

Every new input struct is `#[serde(deny_unknown_fields)]` (all new methods
are local-only). Every new output is a typed struct. Error codes go in
`data.code`, never only in the message text.

### D-C7-1. The chat body

New module `crates/roym_core/src/chat.rs`, with its tests in
`crates/roym_core/src/chat/tests.rs` (module layout rule). Callers write
`chat::…` after `use syneroym_roym_core::chat;`.

```rust
pub const ROYM_MESSAGE_CONTENT_TYPE: &str = "application/vnd.roym.message+json";
pub const CHAT_BODY_VERSION: u32 = 1;
/// The only inner content type in this version.
pub const INNER_TEXT_CONTENT_TYPE: &str = "text/plain";
/// An origin envelope longer than this is not read.
pub const MAX_ORIGIN_BYTES: usize = 8 * 1024;
/// "Forwarded many times" from this count on. The view carries it, so the
/// Hub does not keep its own copy of the number.
pub const MANY_FORWARDS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ForwardMode { Signed, NotForwardable }
// Deserialize by hand: "signed" -> Signed, every other string -> NotForwardable.

/// As it travels. Unknown fields are ignored: another node wrote it, and a
/// later version may add fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatBody {
    pub v: u32,
    pub content_type: String,
    pub text: String,
    pub forward_mode: ForwardMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_count: Option<u32>,
}

pub enum ChatBodyError { Malformed(String), Unsupported(String) }

impl ChatBody {
    pub fn parse(bytes: &[u8]) -> Result<ChatBody, ChatBodyError>;
    pub fn to_bytes(&self) -> Vec<u8>; // serde_json::to_vec, compact
}
```

Rules in `ChatBody::parse`:

- A body that is not JSON, or misses a field → `Malformed`.
- `v != 1` → `Unsupported`.
- `content_type != "text/plain"` → `Unsupported`.
- An unknown `forward_mode` string reads as `NotForwardable`. A later mode
  never makes a message forwardable on this build.
- `forward_count == Some(0)` reads as `None`.

Rules applied later, in the check (D-C7-3) and the view (D-C7-6):

- On a `NotForwardable` body, any `origin` is ignored.
- An `origin` longer than `MAX_ORIGIN_BYTES` fails the check with the
  reason "origin too large". The length is tested before any parse.

The sender's message signature covers the chat body bytes (one-to-one: the
`DeliveryPayload` over the plain body, `crates/conversation/src/envelope.rs:92`;
group: the ciphertext, `crates/conversation/src/dag.rs:156`). So the mode
is covered (FW-14), and no new signature is needed for it.

### D-C7-2. The content hash

```rust
pub fn content_hash(
    content_type: &str,
    languages: &[String],
    body: &[u8],
    blob_hashes: &[[u8; 32]],
) -> String
```

```text
input =  b"roym-message-origin-v1"
      || u32be(len(content_type)) || content_type
      || u32be(count(languages))  || for each tag: u32be(len(tag)) || tag
      || u32be(len(body))         || body
      || u32be(count(blobs))      || for each blob: its 32 bytes
content_hash = "sha256:" + lowercase hex of SHA-256(input)
```

- `body` is the UTF-8 text. `languages` are the message's tags as stored
  (canonical form, 0 to 3, checked by `syneroym_text_lang::check_tag_list`
  in `send`, `messages.rs:263`). C7 always passes an empty blob list (Q8).
- Hex through the existing `roym_core::backup::bytes_to_hex`
  (`crates/roym_core/src/backup.rs:232`). SHA-256 through the `sha2`
  crate: `roym_core` gains `sha2.workspace = true` (workspace version
  `0.10`, root `Cargo.toml:164`). Every Roym component already links
  `sha2` through `syneroym-signed-record` (`envelope.rs:227`), so the size
  cost is the new code only.

### D-C7-3. The origin record

```rust
// crates/roym_core/src/record.rs
pub const RECORD_MESSAGE_ORIGIN: &str = "message-origin";

// crates/roym_core/src/chat.rs
pub const MESSAGE_ORIGIN_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageOriginPayload {
    pub content_type: String,
    pub languages: Vec<String>,
    pub content_hash: String,
}

pub fn origin_draft(payload: &MessageOriginPayload) -> RecordDraft;
pub fn verify_message_origin(envelope: &str, now_secs: u64)
    -> RecordVerdict<MessageOriginPayload>;
pub fn check_origin(body: &ChatBody, languages: &[String], now_secs: u64) -> OriginCheck;

pub enum OriginCheck {
    Checked { author_did: String, written_at_secs: u64, record_id: String },
    Failed { reason: String },
    Absent,
}
```

- `record.rs`: add the constant, and `("message-origin", 1)` to
  `RECORD_TYPES` (`record.rs:18`). If the wire-word test
  (`crates/roym_core/src/record/tests/wire_words.rs`) lists record types,
  it gets the new word.
- `origin_draft`: `version` 1, `record_type` `message-origin`, `subject`
  `""` (ADR-0030, decision 6; `signed_record/src/envelope.rs:61` checks
  only the maximum subject length), `expires_at_secs` `None`, `supersedes`
  `None`.
- `verify_message_origin`, in the shape of `verify_step`
  (`crates/roym_core/src/step.rs:235`) and `verdict::verify_party_record`
  (`crates/roym_core/src/verdict.rs:73`): `record::verify_json` with
  `VerifyOptions::new(now_secs)`, then refuse when: the type or version
  differs; the payload does not parse; `subject` is not `""`;
  `expires_at_secs` is set; the envelope has no `delegation` (an origin
  signed by a service key alone names no person, so "an origin must be
  signed as a person"). The verdict's `issuer` is then the person's master
  DID.
- `check_origin`: `Absent` when the body is `NotForwardable` or has no
  origin; `Failed { "origin too large" }` over `MAX_ORIGIN_BYTES`; else
  `verify_message_origin`, then compare the payload's `content_type` with
  the body's and its `languages` with the message's, then recompute
  `content_hash` over the inner content and compare. Each failure has its
  own short reason.

### D-C7-4. The settings row and its methods

In `roym_conversation`, new file `src/app/settings.rs`. Types in
`crates/roym_core/src/method/conversation.rs`, beside `SendInput` (`:11`),
so the method shapes and parity see one type:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardSettings { pub default_mode: ForwardMode, pub asked: bool }
// The default, when there is no row: NotForwardable, asked = false
// (ADR-0030, decision 4).

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSettingsInput {
    pub default_mode: Option<ForwardMode>,
    pub asked: Option<bool>,
}
```

```rust
// crates/roym_conversation/src/app.rs
pub const SETTINGS: &str = "conversation_settings";
// crates/roym_conversation/src/app/settings.rs
const FORWARD_ROW: &str = "forward";
```

- `conversation.settings` (`NoParams => ForwardSettings`): no row reads as
  the default. A storage error is `-32603`, never "the default".
- `conversation.set-settings` (`SetSettingsInput => ForwardSettings`):
  merges the given fields into the row, writes it and returns it.
  `asked: false` is refused (`-32602`, field `asked`): the Hub asks only
  once (ADR-0030, decision 5).

The service never reads the setting when it sends (Q3 (a)). The Hub reads
it and passes the mode. The node keeps the setting for the Hub.

### D-C7-5. Sending with a mode

`SendInput` (`method/conversation.rs:11`) gains
`#[serde(default, skip_serializing_if = "Option::is_none")] pub forward_mode: Option<ForwardMode>`.

New file `crates/roym_conversation/src/app/chat_send.rs`:

```text
pub(crate) async fn send_with_mode(host, input, mode) -> Reply<SendOutput>
  if input.content_type is Some(ct) and ct != "text/plain":
      -32602, field content_type
  (languages are checked in send before this, messages.rs:263)
  origin = None
  if mode == Signed:
      payload = MessageOriginPayload {
          content_type: "text/plain", languages,
          content_hash: chat::content_hash("text/plain", &languages, text, &[]) }
      (principal, _owner) = signing::resolve_person_principal(host, now)?   // D-C7-11
      origin = Some(AppSigning::sign_record(host, chat::origin_draft(&payload), principal)
                    .await.map_err(internal_error)?)
  body = ChatBody { v: 1, "text/plain", text, mode, origin, forward_count: None }
  send_body(host, conversation, ROYM_MESSAGE_CONTENT_TYPE, languages, body.to_bytes(), ..)
```

- `messages::send` calls `send_with_mode` when `forward_mode` is `Some`,
  and keeps its path unchanged when it is `None`.
- The group checks must run **before** the signing call, so a send to a
  group the person left signs nothing. If C4's `send_body` runs them only
  inside itself, `send_with_mode` first calls
  `AppConversation::group_info` the same way `send` does today
  (`messages.rs:273` to `:285`), and the implementer moves that check into
  a small shared helper rather than copying it.
- A signing failure sends nothing. There is no fallback to
  `not-forwardable` (ADR-0030, decision 7).

### D-C7-6. The forward view (read side)

Pure part, in `roym_core::chat`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardView {
    pub mode: ForwardMode,
    /// `signed`, and the origin is checked. The Hub offers "Forward"
    /// exactly when this is true.
    pub forwardable: bool,
    /// Only when the origin is checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginView>,
    /// Why a `signed` body's origin did not check, or why the body could
    /// not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_error: Option<String>,
    /// 0 for a message that is not a forward.
    pub forward_count: u32,
    pub many_forwards: bool,  // forward_count >= MANY_FORWARDS
    /// Draw the "Forwarded" line.
    pub forwarded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginView { pub author_did: String, pub written_at_secs: u64, pub record_id: String }

pub fn forward_view(
    body: &ChatBody,
    languages: &[String],
    sender_person_did: Option<&str>,
    now_secs: u64,
) -> ForwardView;

/// The view of a chat body that could not be read.
pub fn unreadable_view(reason: &str) -> ForwardView;
```

Rules of `forward_view` (each row is a unit test, section 7.1):

| Body and check | `forwardable` | `origin` | `origin_error` | `forwarded` |
| --- | --- | --- | --- | --- |
| `not-forwardable` | false | none | none | false |
| `signed`, checked, count 0, sender DID = author | true | yes | none | false |
| `signed`, checked, count ≥ 1 | true | yes | none | true |
| `signed`, checked, count 0, sender DID known and ≠ author | true | yes | none | true |
| `signed`, checked, count 0, sender DID unknown | true | yes | none | false |
| `signed`, failed or absent (any count) | false | none | yes | false |

The last row is FW-4: the Hub shows the message as the sender's own words,
with the note. `origin` is `None`, so no author name can be drawn. The
fourth row covers a forward that a changed client sent with no count: it
still names its author.

Service part, new file `crates/roym_conversation/src/app/view.rs`:

- `pub(crate) async fn message_views<H: AppHost>(host: &H, rows: Vec<Message>) -> Vec<MessageView>`.
  For a row whose `content_type` is `ROYM_MESSAGE_CONTENT_TYPE` and that is
  not deleted: parse the body. On success, `view.body = Some(text)` and
  `view.forward = Some(forward_view(..))`. On an error, keep the raw body
  and set `view.forward = Some(unreadable_view("message body could not be read"))`.
- `sender_person_did`: for an outgoing row, `signing::owner_did(host)`
  (`crates/roym_core/src/signing.rs:240`); for an incoming row, the
  `contacts_map` (`crates/roym_conversation/src/app.rs:143`). Each is read
  **once per page**, and only if the page holds a chat body.
- `now_secs`: `clock::now_secs()`.
- `history` (`messages.rs:373`), `changes` (`:398`) and `search` (`:491`)
  build their rows through `message_views`. `history_item_to_view`
  (`messages.rs:322`) keeps its two literal rows for membership and
  group-name events, with `forward: None`.

### D-C7-7. `conversation.forward`

New file `crates/roym_conversation/src/app/forward.rs`. The input type is
in `method/conversation.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ForwardInput {
    pub message_id: String,     // the source message
    pub conversation: String,   // where the forward goes
}
// Output: SendOutput, the same as conversation.send.
```

Steps, in this order. Each refusal is `-32602` with the `data.code` named:

1. `AppConversation::get_message(host, message_id)`. `NotFound` →
   `message-not-found`. (It returns only accepted or own messages, so a
   held or dropped message is "not found".) Other host errors →
   `from_host` (`crates/roym_conversation/src/app.rs:131`).
2. Deleted (`deleted_at` set) → `message-deleted`.
3. `content_type != ROYM_MESSAGE_CONTENT_TYPE` → `not-forwardable` (a
   `text/plain` message, a card, a widget, a system message; ADR-0030,
   decision 3).
4. The body does not parse → `not-forwardable`.
5. `forward_mode != Signed` → `not-forwardable`.
6. `check_origin` is not `Checked` → `origin-not-checked`, with the reason
   in the message.
7. Build `ChatBody { v: 1, content_type, text, Signed, origin: <the same
   string, byte for byte>, forward_count: Some(old.unwrap_or(0).saturating_add(1)) }`.
8. `send_body(host, input.conversation, ROYM_MESSAGE_CONTENT_TYPE, source.languages, body, ..)`.
   The group checks of the target run there.

- The forwarder's own default is never read (FW-15).
- The source may be in any chat, and the target may be the same chat
  (FW-9: "inside the same group").
- A restored source message may be forwarded: the check needs only the
  body and the languages.
- Nothing is signed with the forwarder's person key. The forwarder's own
  message signature covers the forward, as for any message (ADR-0030,
  decision 8).

### D-C7-8. Search (Q4 (a))

In `roym_core::chat`:

```rust
/// True when `text` matches `query` by the host's own search rule.
pub fn text_matches(text: &str, query: &str) -> bool
```

```text
terms = syneroym_text_lang::query_terms(query)
keys  = syneroym_text_lang::fts::index_keys(text)
every long term t:  keys.joined contains t
every short term s: some word w of keys.words (split on ' ') starts with s
```

This is the rule of the two FTS5 tables (`fts.rs:56`): the trigram table
finds a long term anywhere in the joined key, and the word table with
`prefix = '1 2'` finds a short term as a word prefix. In `messages::search`,
after the host call, a chat-body match is kept only if
`chat::text_matches(inner_text, &query)`. The method's doc comment says
that `matches` may hold fewer rows than `limit`.

### D-C7-9. Backup

`crates/roym_conversation/src/app/backup.rs` (`:19`, `:66`): add
`SECTION_SETTINGS = "settings"` and `(SECTION_SETTINGS, SETTINGS)` to
`collect_sections`. Import: the section may hold zero rows, or one row with
id `forward`. Any other id, or a payload that does not parse as
`ForwardSettings`, refuses the import (`-32602`, the row id in the
message). Chat bodies travel in `conversation_history` unchanged, and are
checked again at read after a restore.

`SCHEMA_VERSION` 4 → 5 (`crates/roym_conversation/src/app.rs:31`). Its doc
comment gains a "why" line: the settings section.

### D-C7-10. Hub

New files in `crates/roym_web/ui/src/chat/`:

- `forward_mode.ts`: `loadForwardSettings()`, `saveForwardSettings(..)`
  and `modeControl(defaultMode, inGroup)`. The control is a `<select>`
  ("Signed", "Not forwardable") set to the default, with a group notice
  element shown when `inGroup` is true and the value is `signed` (FW-11).
  `reset()` puts it back to the default after each send (FW-12).
- `first_message.ts`: `askOnce(anchor): Promise<ForwardMode>`. If
  `settings.asked` is true, it returns the default at once. Otherwise it
  draws a dialog (class `forward-mode-dialog`) with the two choices and
  "Skip", and the text of ADR-0030, decision 12, in plain words. It saves
  `{ default_mode, asked: true }` (skip saves `asked: true` only) and
  returns the mode for this send.
- `forward_view.ts`: `forwardLine(view, contacts)`, `forwardActions(m, refresh)`
  and the notes. The line is "Forwarded. First written by {name} on
  {date}. Forwarded {count} time(s)." or "… Forwarded many times." when
  `many_forwards` (FW-5). `{name}` by Q9 (a). `{date}` from
  `written_at_secs`, through the Hub's date format (`i18n/format.ts`).
- `forward_picker.ts`: a list of the person's chats from
  `conversation.list` (direct and groups, not hidden). "Forward" calls
  `conversation.forward`. The picker has the class `forward-picker`.

Changes to existing files:

- `screens/messages.ts`: add `.forward-mode-dialog, .forward-picker` to
  `OPEN_WORK` (`:30`), so a feed redraw waits for them. In `renderThread`
  (`:304`) and its send handler (`:341`): `askOnce` before the send, then
  `forward_mode` from the mode control. `MessageRow` (`:55`) gains
  `forward?: ForwardView`. `messageElement` (`:517`) calls `forwardLine`
  and `forwardActions` for a row with `forward`, and the "Not forwardable"
  note for a `text/plain` row.
- `screens/groups.ts`: the group composer (`doSend`, `:444`) does the same
  with `inGroup = true`. Its message rows (`renderThreadMessages`, `:151`)
  use the same helpers.
- A refused `signed` send with `data.code` `signing-not-enrolled`,
  `signing-certificate-expired` or `signing-certificate-stale` shows two
  buttons: "Send as not forwardable" (sends again with that mode) and
  "Renew signing" (opens the setup screen, `screens/setup.ts`). It never
  changes the mode on its own (ADR-0030, decision 7).
- `screens/profile.ts`: the section "Forwarding of my messages", with the
  default and the explanation (FW-10, Q10 (a)).
- The delete dialog (`openDeleteDialog`, `messages.ts:1135`) adds the
  sentence of section 4, item 17, for a `signed` message.

### D-C7-11. One signing refusal helper

Move `resolve_principal_and_owner` from
`crates/roym_transaction/src/app.rs:307` to `crates/roym_core/src/signing.rs`
as:

```rust
pub async fn resolve_person_principal<H: AppHost>(host: &H, now: u64)
    -> Result<(Principal, String), Response>
```

Its words and `data.code` values do not change. Every caller in
`roym_transaction` switches to it (find them with
`git grep -n resolve_principal_and_owner crates/roym_transaction`).
`roym_conversation` calls it in D-C7-5.

### D-C7-12. No `roymctl` change

`roymctl roym group send` (`apps/roymctl/src/commands/roym/group.rs:197`)
keeps sending with no mode (Q3 (a)). No story or test needs a
`--forward-mode` flag or a `forward` command. The developer guide
documents the raw methods instead.

### D-C7-13. Widgets stay not forwardable

A `widget` or `widget-answer` message (C4) has no forward mode, so
`conversation.forward` refuses it at step 3 of D-C7-7. Whether a vote is
provable is the C4 plan's question (ADR-0030, Consequences), not C7's.

## 6. Step-by-step changes

### 6.1 C7a

Each step builds, and its tests pass, before the next one starts.

#### Step 0 — Rebase on C4

After C4 merges (Q1 (a)): rebase this branch on `main`. Re-check the places
in section 2.2, point 1, and update section 5 of this plan where C4's code
differs. Record the component sizes on that `main` commit (one `target/`
folder, `mise run build:roym`) as the "before" column.

#### Step 1 — Pure chat code in `roym_core`

- `crates/roym_core/Cargo.toml`: `sha2.workspace = true`.
- `src/chat.rs` and `src/chat/tests.rs`: D-C7-1, D-C7-2, D-C7-3, the pure
  part of D-C7-6, and D-C7-8. `src/lib.rs`: `pub mod chat;`.
- `src/record.rs`: the record type.
- `src/conversation.rs`: `MessageView.forward`, and the struct literals at
  `conversation.rs:248`, `conversation/group/tests.rs:7`,
  `crates/roym_conversation/src/app/messages.rs:327` and `:350`.
- `src/method/conversation.rs`: `SendInput.forward_mode`, `ForwardInput`,
  `ForwardSettings`, `SetSettingsInput`. Update the `SendInput` literals in
  tests (`git grep -n "SendInput {"`).
- `src/signing.rs`: D-C7-11, and switch `roym_transaction` to it.
- **Size probe** (Q4 stop rule): a throwaway build of the `conversation`
  component with a call to `chat::text_matches` in `search`. Report the
  size change. Over 60 KB: stop and ask. Do not commit the probe.

Tests: section 7.1. `cargo nextest run -p syneroym-roym-core` and
`-p syneroym-roym-transaction`.

#### Step 2 — `conversation`: settings, send with a mode, the view

- `src/app.rs`: `SETTINGS`, `SCHEMA_VERSION` 5.
- `src/app/settings.rs`: D-C7-4.
- `src/app/chat_send.rs`: D-C7-5.
- `src/app/view.rs`: the service part of D-C7-6.
- `src/app/messages.rs`: the reserved type; `send` calls `send_with_mode`;
  `history`, `changes` and `search` go through `view.rs`; the D-C7-8 filter.
- `src/app/backup.rs`: D-C7-9.
- `src/api.rs`: rows for `conversation.settings` and
  `conversation.set-settings`.
- `crates/roym_web/tests/method_contract/contract.rs:39`: the count (C4's
  value + 2).
- Scenario 73 (`dual_build_parity/wire_origin.rs`): add the two verbs.
- Method shapes: run with `UPDATE_METHOD_SHAPES=1` and read the diff.

Tests: parity A to E and I (section 7.3). Run `mise run build:roym` first,
then the **whole** parity binary.

#### Step 3 — `conversation.forward`

- `src/app/forward.rs`: D-C7-7. `src/api.rs`: its row. Contract count + 1;
  scenario 73 + 1; method shapes.
- Confirm section 2.2, point 5 (the pinned clock) with the first forward
  scenario.

Tests: parity F to H.

#### Step 4 — Three-node test and the measurements

- New `crates/substrate/tests/roym_forward_e2e.rs` on
  `common::roym_group::boot_trio` (`crates/substrate/tests/common/roym_group.rs:39`),
  `form_group` (`:72`) and `converge` (`:193`). Section 7.4.
- It prints, and asserts under 2,048 bytes, the length of the origin
  envelope of a short text message ("Market opens at 9", one tag `en`).
  It also prints the length of the whole chat body. **Over 2 KiB: stop and
  ask the owner** (ADR-0030, decision 11). Both numbers go into the change
  doc's Deviations.
- It measures one `conversation.history` page of 200 `signed` messages on
  the WASM build (Q6 stop rule: 500 ms).

#### Step 5 — Hub

D-C7-10, with the Hub unit tests of section 7.5. Run
`TZ=UTC npx vitest run`. New `en.ftl` and `hi.ftl` keys (section 7.8).

#### Step 6 — Playwright and the demo

- `crates/substrate/tests/e2e/hub-helpers.ts`: `sendMessage` (`:211`)
  answers the first-message dialog with "Skip" when it is shown. Then every
  existing caller (`demo/20-roym-chat.spec.ts`,
  `demo/40-roym-languages.spec.ts`, `tests/roym-hub.spec.ts`) keeps
  working. A new helper `chooseForwardMode(page, mode)` answers it on
  purpose. The group send helper that `tests/roym-groups.spec.ts` uses does
  the same.
- `tests/roym-hub.spec.ts`: a new case (the next free number; 42 is the
  highest on `main`) for the dialog, the composer control and the Profile
  setting, on one node.
- Story 20 (Q7): section 7.6.
- `mise run demo`, then `mise run test:e2e`, with the output redirected
  (AGENTS.md, Context Budget).

#### Step 7 — Sizes, docs and close-out of C7a

Component sizes (section 7.7) into [task.md](task.md) section 1; the
change doc's Deviations (section 9); backlog (section 8); living docs
(section 9); the `status.md` row with the Hindi keys; `mise run verify`
with no `--skip` (in a worktree: `cargo xtask verify` with
`CARGO_TARGET_DIR`).

### 6.2 C7b — Forward of signed records

An outline only. C7b checks these points in the code again after C7a
merges, on its own branch, and adds a short section to this plan.

- New content type `application/vnd.roym.record-forward+json`, body
  `{ "v": 1, "record_type": "listing" | "profile" | "quote", "envelope": "<envelope JSON>" }`.
  It ignores the forward mode and carries no origin (ADR-0030, decision 10).
  Reserved in `conversation.send`. Allowed in groups: it is not the card
  type, so `transaction` never files it (`sync.rs:263`).
- New `conversation.forward-record` (`{ conversation, record_type, envelope }`):
  checks the envelope with the matching pure function, refuses any other
  record type, and sends. The envelope is at most 160 KiB (the card body
  limit, `MAX_CARD_BODY_BYTES` in `crates/roym_core/src/card.rs`).
- Read side: `MessageView.record_forward: Option<RecordForwardView>`, with
  the verdict, and for a quote only the visible fields that ADR-0030,
  decision 10, lists.
- Checks: `listing::verify_envelope` (`crates/roym_core/src/listing.rs:622`)
  and `transaction::verify_quote` (`crates/roym_core/src/transaction.rs:458`)
  exist. The profile check lives in `roym_profile`
  (`crates/roym_profile/src/app/contacts.rs:117`); C7b moves it into
  `roym_core` first.
- Hub: "Forward" on a quote card (with the warning that it shows both
  parties), on a directory hit (the envelope from `runEnvelope`,
  `screens/directory.ts:454`) and on a contact's profile. The receiver's
  card is drawn by the existing templates.
- Tests: `roym_core` unit tests for each type; parity; a two-node forward
  of a quote that the receiver's `transaction.thread` does not list.

## 7. Tests

### 7.1 Unit tests (`crates/roym_core/src/chat/tests.rs`)

Test names say what they prove, with no plan ids.

Body:

- `a_chat_body_round_trips_through_bytes`
- `a_body_of_another_version_is_unsupported`
- `a_body_with_another_inner_content_type_is_unsupported`
- `an_unknown_forward_mode_reads_as_not_forwardable`
- `an_origin_on_a_not_forwardable_body_is_ignored`
- `an_origin_over_the_size_limit_fails_as_too_large`
- `a_forward_count_of_zero_reads_as_none`
- `unknown_fields_in_a_chat_body_are_ignored`

Hash:

- `the_content_hash_is_stable` (a fixed vector)
- `the_hash_changes_with_the_text`, `the_hash_changes_with_one_language_tag`,
  `the_hash_changes_with_the_order_of_tags`,
  `the_hash_changes_with_the_content_type`,
  `the_hash_changes_with_the_blob_list` (FW-6, FW-8)
- `length_prefixes_keep_two_splits_apart` (`["ab","c"]` and `["a","bc"]`)

Origin. Signed with `syneroym_identity` keys and a real
`DelegationCertificate`, as the tests in `crates/signed_record/src/verify.rs`
do. Test clocks start at or after the real "now", because the certificate
lifetime is checked against it:

- `a_signed_origin_checks_and_names_the_master_did`
- `an_origin_still_checks_after_its_certificate_expired`
- `an_origin_signed_by_the_service_key_alone_is_refused`
- `an_origin_with_a_subject_is_refused`
- `an_origin_with_an_expiry_is_refused`
- `an_origin_of_another_record_type_is_refused`
- `an_origin_for_other_text_fails_the_hash` (FW-6)
- `an_origin_for_other_languages_fails`
- `a_tampered_origin_signature_fails` (FW-4)
- `a_changed_text_with_the_old_origin_is_not_forwardable` (FW-13: the body
  a changed client would send)

View (`forward_view`): one test per row of the table in D-C7-6, and:

- `a_forward_of_a_forward_counts_up`
- `many_forwards_starts_at_five` (FW-5)
- `a_failed_origin_never_names_an_author` (FW-4)
- `an_unreadable_body_is_not_forwardable`

Search (`text_matches`):

- `text_matches_a_long_term_inside_a_word`
- `text_matches_a_short_term_as_a_word_prefix`
- `text_does_not_match_a_word_that_is_only_in_the_json` (`forward_mode`,
  `signed`)
- `text_matches_devanagari_like_the_host` (one case from the host's search
  tests in `crates/conversation/src/host_impl/tests.rs`)

Signing helper (in the `signing::tests` module, `signing.rs:256`):

- `resolve_person_principal_refuses_each_certificate_state_with_its_code`:
  `TestHost` with `set_signing_id` and no `CERTIFICATES` row, then an
  expired certificate, then a stale one, then no owner.

### 7.2 Service tests

`roym_conversation` has no unit tests (section 2.2, point 4). Its rules
are tested through 7.3 and 7.4.

### 7.3 Parity (both builds agree): new file `dual_build_parity/forward.rs`

Scenario numbers: the next free numbers after C4 (C4 takes 260 to 265).
Each scenario calls `enrol_signing(&h, "conversation")`
(`dual_build_parity/fixtures.rs:59`) first, unless it tests the refusal.

- A. `conversation.settings` on a fresh node returns the default;
  `set-settings` changes it; `asked: false` is refused.
- B. `send` with `signed`: the history row has `forward.mode` `signed`,
  `forwardable: true`, `origin.author_did` = the owner DID, and `body` =
  the text. The raw stored body (read with the host's `get_message`) holds
  `"forward_mode":"signed"` (FW-1, FW-14).
- C. `send` with `not-forwardable`: no origin, `forwardable: false`.
- D. `send` with `signed` and no certificate: `data.code`
  `signing-not-enrolled`, and the outbox is empty.
- E. `send` with `forward_mode` and `content_type: "text/html"`: `-32602`,
  field `content_type`. A raw `send` of the chat content type: reserved.
- F. `forward` of B's message to a second conversation, with the default
  set to `not-forwardable`: count 1, the same origin string, `forwarded:
  true`, mode `signed` (FW-2, FW-15).
- G. `forward` refusals: a `text/plain` message, C's message, a deleted
  message, an unknown id. Each has its `data.code`.
- H. Search: a word of B's text finds it; `forward_mode` and `signed` do
  not (Q4 (a)).
- I. Backup: export after A, import into a wiped node: the settings row is
  back. A bundle with a second settings row id is refused.

The two builds share the owner identity and the pinned clock, so they
should sign the same bytes. Normalize the origin with `strip_volatile` only
if they do not, and say why in the normalizer.

### 7.4 Three-node test (`crates/substrate/tests/roym_forward_e2e.rs`)

On `boot_trio` (Alice, Bob, Carol), WASM build:

1. Alice and Bob open a one-to-one chat. Alice sends "Market opens at 9"
   with `signed`. Wait for `delivered`.
2. Bob's history: `forwardable: true`, author Alice's master DID.
3. Bob forms a group with Carol (`form_group`) and forwards Alice's
   message there. On Carol's node, after `converge`: `forwarded: true`,
   `forward_count: 1`, `origin.author_did` = Alice's master DID, `body` =
   the text (FW-3, FW-9 to a group).
4. Carol forwards it into the same group: count 2 (FW-9 inside the same
   group, FW-15).
5. Alice sends a `not-forwardable` message. Bob's `forward` of it is
   refused with `not-forwardable` (FW-2).
6. The size measurement and the history timing of step 4.

A forged forward (changed text with the old origin) cannot be sent through
`conversation.send`, because the chat content type is reserved. The view is
a pure function of the body and the languages, so the unit tests of 7.1
cover it on the receiving side.

### 7.5 Hub unit tests (`TZ=UTC npx vitest run`)

New files `chat/forward_view.test.ts`, `chat/forward_mode.test.ts`,
`chat/first_message.test.ts` and `chat/forward_picker.test.ts`:

- the forwarded line with a contact name, with an unknown DID ("(not in
  your contacts)"), and with count 1, 4 and 5 (FW-3, FW-5, Q9);
- a failed origin: no author name anywhere in the element, and the note
  (FW-4);
- no "Forward" button when `forwardable` is false, and the note says why
  (FW-13); a `text/plain` row gets the note too;
- the mode control starts at the default, changes for one send, and is
  back at the default after it; the saved default is unchanged (FW-12);
- the group notice shows only for `signed` in a group (FW-11);
- the first-message dialog: shown when `asked` is false; each choice saves
  `asked: true`; skip keeps `not-forwardable`; not shown again (FW-10);
- a refused `signed` send shows the two buttons and does not resend on its
  own (ADR-0030, decision 7).

Existing tests: `screens/messages.test.ts`, `screens/groups.test.ts` and
`screens/messages_feed.test.ts` mock `conversation.send`. Update their
expected params, and add one assertion each that `forward_mode` is sent.

### 7.6 Playwright and the demo

- A Hub case (the next free number) in `tests/roym-hub.spec.ts`: dialog,
  composer, Profile default.
- Story 20, after C4's poll steps (Q7 (a)):
  1. "Alice's screen: she chooses that her messages can be forwarded with
     her name": at her first send, `chooseForwardMode(page, 'signed')`.
     This is part of today's step "send Bob a message".
  2. "Bob's screen: he forwards Alice's message to a new group with
     Alice": create the group "Market", then forward.
  3. "Alice's screen: the group shows 'Forwarded. First written by
     Alice …'". Wait `DELIVERY_WAIT_MS`.
- `crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts` goes into the
  change doc's `demo:` list.

### 7.7 Component size

Release `wasm32-wasip2`, one `target/` folder, before (step 0) and after.
`conversation` gets new record code, so its limit is +15% of its "before"
size (on `aa25961b` it is 813,818 bytes in the C3c2 table, so about
122 KB; C4 changes the base). `transaction` changes only by the moved
helper: +40 KB limit. The other components link `roym_core` but call none
of the new code: +40 KB limit each. Over a limit: stop and tell the owner.

### 7.8 Hindi text

Every new key goes into `en.ftl` and `hi.ftl`, and the `status.md` row
lists them for the owner's review. Expected keys (names may change):
`forward-mode-label`, `forward-mode-signed`, `forward-mode-not-forwardable`,
`forward-mode-group-notice`, `forward-first-title`,
`forward-first-signed-explain`, `forward-first-not-forwardable-explain`,
`forward-first-skip`, `forward-settings-heading`, `forward-settings-explain`,
`forward-button`, `forward-picker-title`, `forward-sent`, `forward-line`,
`forward-line-many`, `forward-not-in-contacts`, `forward-not-forwardable-note`,
`forward-origin-not-checked`, `forward-signed-tag`,
`forward-send-not-forwardable`, `forward-renew-signing`,
`messages-delete-signed-note`. The tests `i18n/hindi-screens.test.ts` and
`i18n/catalog.test.ts` check that every key has both languages.

### 7.9 Not tested

- A changed client that lowers the forward count: accepted by design (the
  change doc, "What the forward count can and cannot say").
- Attachments: none exist (Q8).

### 7.10 Size and time risks

- The ICU data of `search_key` in `conversation` (Q4 stop rule, step 1).
- The origin size (ADR-0030 stop rule, step 4).
- The read-time check (Q6 stop rule, step 4).
- `messages.ts` keeps its length: new code is in `chat/`.

### 7.11 How C7 avoids the kinds of findings in earlier reviews

- Every line number above was read on `aa25961b`. Step 0 reads the
  C4-touched ones again (C4 P1, C3c P8).
- Callers of changed types: the `MessageView` literals, `SendInput` (Hub;
  `roymctl` tests at `apps/roymctl/src/commands/roym/tests.rs:384`;
  parity) and `resolve_principal_and_owner` (all of `roym_transaction`).
  `git grep` each before step 1 ends (C3c P3, C2 P2).
- Hard-coded counts: the `conversation` contract count, scenario 73, the
  method shapes (C4 P5, C3b R1).
- Every free field has a limit checked before it is parsed: origin 8 KiB,
  languages 3, text by the store limit of 262,144 bytes
  (`crates/conversation/src/store.rs:92`) (C3a P4, C2 P5).
- The signer is compared with what is expected: the origin must be signed
  as a person (delegation present), and the display name comes only from
  the checked `issuer` (C2 R14, R25).
- The signing refusals are planned and tested (C3b P5, C2 R5).
- A storage error is never read as "no row" (C2 R22).
- New Hindi keys are listed (C1 R15).

## 8. Deferred backlog

Rows to add to [deferred-backlog.md](../../deferred-backlog.md) in C7a
(section 5, Messaging, unless noted):

- **Roym chat has no attachments, so the origin's blob-hash list is always
  empty.** FW-8 is met in the format only (Q8). Target: TBD.
- **Four places map the signing errors without a `data.code`:**
  `roym_catalog`, `roym_profile`, `roym_directory` and `roym_core::backup`
  (section 2.2, point 3). Section 10 (Product surfaces). Target: TBD.
- **A default forward mode per conversation, and a group owner's limit on
  `signed` messages** (change doc, open questions; ADR-0030,
  Consequences). Target: TBD.
- If the owner answers Q4 (b): **search matches the JSON keys of a chat
  body.** Target: TBD.
- Update the row "More demo stories" (§13): C7 extends story 20.

## 9. Living docs and the change doc at close-out

Change doc [roym-message-forwarding](../../changes/roym-message-forwarding/change.md),
`## Deviations` (this change doc owns these, and they are written only
there):

- the chat body content type and format (Q3), and that a message sent with
  no mode stays `text/plain`;
- the inner content and its hash layout (D-C7-2), the `sha256:` hex form;
- FW-8 met in the format only (Q8);
- where the setting lives (Q5), and the first-message flag on the node;
- the check at read time (Q6);
- the display name rule (Q9);
- the measured origin and body sizes (step 4);
- the split into C7a and C7b (Q2), until C7b merges.

Also in the change doc: tick the C7a lines of `## Tasks`. `status: done`
and `living-docs-touched` wait for C8 (task.md section 3, C8).

Living docs (C7a):

- `docs/roym-integrated-experience-spec.md`: "Messaging" (`:690`) — the
  chat body and the forward; "What is encrypted, and who can see what"
  (`:576`) — what a `signed` message gives up (ADR-0030, decision 12);
  "Records and what each one proves" (`:468`) — `message-origin`.
- `docs/TERMINOLOGY.md`: forward mode, origin (`message-origin`), forward,
  forward count.
- `docs/developer-guide.md` §6 "Build a Roym client": `forward_mode` on
  `conversation.send`, `conversation.forward`, `conversation.settings`,
  and the `forward` field of a message (near line 2566). The tagged code
  blocks run in story 30, so a new tagged block must run there too.
- `docs/system-requirements-spec.md:291`: remove "attachments"; add the
  forward with a signed origin.
- `docs/planning/traceability-matrix.md`: the messaging row gains the
  forward and its tests.
- [task.md](task.md): the C7a result line and the size table.

## 10. Exit criteria

C7a is done when all of these hold:

- [ ] Every C7a criterion in section 3 names a test that exists on the
      branch, and the test passes.
- [ ] The `roym_core` `chat` unit tests, the whole parity binary (after
      `mise run build:roym`), `roym_forward_e2e`, the Hub unit tests, the
      new Hub case and story 20 pass.
- [ ] The origin of a short text message is under 2 KiB, measured and
      recorded, or the owner answered the stop.
- [ ] A history page of 200 `signed` messages takes under 500 ms in the
      WASM build, or the owner answered the stop.
- [ ] Component sizes are in task.md section 1, each within its limit.
- [ ] Deviations, backlog rows, living docs and the `status.md` row (with
      the Hindi keys) are done.
- [ ] `mise run verify` with no `--skip` passes.

C7b is done when FW-7 has its tests, and its sizes, docs and verify pass.

## 11. What C7 does not do

- No chain of forwarders, no forward limit, no copy-paste detection (the
  change doc's non-goals).
- No enforcement of `not-forwardable` against copy, a screenshot or a
  changed client.
- No mode for messages sent without one (`roymctl`, other clients). They
  stay `text/plain` and are not forwardable (Q3 (a)).
- No attachments (Q8).
- No default per conversation, and no group rule (ADR-0030).
- No forward of a quote's hidden fields, and no disclosures in a forward
  (ADR-0030, decision 10; agreements doc, open question 11).
- No forward of widgets, widget answers (D-C7-13), cards other than a
  quote, or system messages.
- No new `roymctl` command (D-C7-12).
- No change to the substrate conversation capability, its WIT or the host
  search, unless the owner picks Q4 (c).

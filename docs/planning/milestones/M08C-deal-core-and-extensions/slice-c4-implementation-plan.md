# M08C slice C4 — Widgets: implementation plan

*Reader: the developer (or AI) who implements slice C4, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan reviewed, 2026-11-25 (round 2 by Claude Sonnet 5.5, all items
folded). The owner accepted the recommended answer of every question in
section 2.1 on 2026-11-25.**

**Size and PRs.** [task.md](task.md) sizes C4 as L. Checked against the
code, it is about 1,500 lines of production code (Rust and TypeScript) and
as many lines of tests. The plan recommends two PRs (Q1):

- **C4a — Widgets on the node (M).** The widget and answer formats in
  `roym_core`, the manifest kind `chat` with the `poll` extension, the
  three `widget.*` methods on `conversation`, `roymctl roym widget`, the
  parity scenarios and a two-node test. Nothing on the Hub screen is new.
  Branch: `feat/m08c-c4-widgets` (this branch; it holds the plan commit,
  cut from `main` at `aa25961b`).
- **C4b — Widgets in the Hub (M).** The poll card, the generic widget
  card, the poll composer, the 1:1 and group threads, the Hindi messages,
  the Playwright case and the demo step in story 20. Branch:
  `feat/m08c-c4b-widgets-hub`, cut from `main` after C4a merges.

**Base.** Every file path and line number below was checked on
`1920cee3` (2026-11-25), the tip of the C3c2 branch. `main` at `aa25961b`
(C3c2 merged, PR #382) has the same tree, so every line holds on `main`.
Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: C4 comes after C3c, and
after C3a for the generic field view. [status.md](status.md) says "Merged"
for C3a1, C3a2, C3c1 and C3c2. C4 and C7 must not run at the same time:
both change `conversation` and the Hub chat (task.md section 4).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments, test names, or `.wit`, `.toml`,
    `.json`, `.ftl` files: no "M08C", "C4a", "EX-12", "ML-26", "D-C4-3",
    "Q2" or review ids. ADR numbers are allowed.
  - Comments never cite size rules.
  - **File size.** Production `.rs` files at most 800 lines; new test `.rs`
    files at most 800 lines. Put new code in new files (section 7.8).
  - **Function size.** `clippy::too_many_lines` at 100. Add no
    suppression.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - **Ports.** Two-node tests use the shared harness
    (`crates/substrate/tests/common/roym.rs`, `roym_group.rs`), never a
    port literal.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1).
  - Do not commit on `main`.
- [task.md](task.md): "C4", section 1 (the component size limit), section 4
  (C4 and C7), section 5 (the EX-11 and ML-26 rows), section 6 question 4
  (larger definitions are refused), section 7 (the widget rows).
- [roym-extensions](../../changes/roym-extensions/change.md): the terms
  "Widget" and "Widget answer" (`:28`, `:30`); EX-10 to EX-13 (`:84` to
  `:87`); "Step records" (`:142` to `:148`, the widget-answer paragraph and
  the card count); "Widgets" (`:150` to `:154`); "Deviations" (`:213` to
  `:277`, the EX-11 line at `:216`, and "`[[widgets]]` and `[service]` are
  not built … Widgets come with C4" at `:225`).
- [roym-multilingual](../../changes/roym-multilingual/change.md) (it is not
  in this slice's prompt list, but task.md names it for C4): ML-26 (`:133`),
  ML-28 (`:136`), "Commitment kinds and extension fields" (`:221`),
  "Translation hook points" (`:473` to `:484`, the row "Widget answer" at
  `:482`), and the Deviations at `:615` and `:616`.
- [ADR-0030](../../../decisions/0030-signed-message-origins.md):
  "Consequences", the `widget-answer` line (`:82`). It leaves the signature
  of a vote to this plan (Q2). Its "Context" also describes how a 1:1 and a
  group message are signed today (`:25` to `:27`).
- [roym-agreements](../../changes/roym-agreements/change.md) `:1026` and
  [roym-value-exchange](../../changes/roym-value-exchange/change.md): no
  criterion of either is built here. The agreements doc only repeats the
  card count (point 7 in section 2.2).
- The C3a plan ([slice-c3a-implementation-plan.md](slice-c3a-implementation-plan.md))
  for the manifest, the registry, the message ids and the field view that
  this slice reuses.
- The C1, C2, C3a, C3b and C3c review files. Section 7.11 says how this
  plan avoids the kinds of findings they hold.

### Terms used in this plan

- **Widget:** a small interactive item in a chat, such as a poll. It is
  data, never code. In this plan a widget is one chat message of the
  content type `application/vnd.roym.widget+json`. Its body holds the
  **widget definition**.
- **Widget answer:** one person's answer to one widget, for example a
  vote. It is one chat message of the content type
  `application/vnd.roym.widget-answer+json`. It names the widget by the
  widget's message id.
- **Widget type:** a name such as `poll/single`, declared in an extension
  manifest of the new kind `chat` (D-C4-3). A node knows the types of the
  manifests it ships.
- **Semantic tag:** a short code that says what a widget is for (`poll`,
  `form`, `checklist`, …). The Hub has a richer card for `poll`. Every
  other widget is drawn by the generic card.
- **Message signature:** the signature that every chat message already
  carries. In a 1:1 chat the sender signs the plain message. In a group
  the author signs the conversation entry. The key is the conversation
  store's own key, not the person's master key (ADR-0030, "Context").
- **Person signature:** a Roym signed record signed through the `signing`
  host interface as the person (`principal::delegated`), like a quote.
- **Projection:** the node's checked view of a received item, which the
  Hub draws. The Hub never draws a widget from the raw message body.
- **Scan:** one pass of `widget.list` over the messages of one
  conversation, paging forward from the oldest asked widget (or from the start
  when listing without ids).

## 1. What C4 builds, in one paragraph

Today a chat has text messages and, in a 1:1 chat only, deal cards. C4
adds widgets. A person opens a chat, taps "Poll", types a question and two
to twelve options, and sends it. Everyone in the chat sees the poll with a
count for each option. Each person taps one option to vote, and may tap
another one later to change the vote. The poll works in a 1:1 chat and in
a group. A widget and an answer are ordinary chat messages with their own
content types. The chat's own message signature says who sent each one.
They are not deal cards, so `transaction` never sees them, and they are not
person-signed records, so a vote stays as deniable as a chat message (Q2).
The node checks every widget and answer: the shape, the 8 KiB limit, and,
for a type it knows, the type's rules. `widget.list` counts the newest
valid answer of each person. The widget's fixed labels (its name, its field
labels) come from the string catalog. The author's question and options are
free text in the message's language. A free-text field of an answer gets a
translate action at the new "widget answer" hook point. A widget of a type
this build does not know, but built from the field types it does know, is drawn
by the generic card with "Not checked on this device". A widget with an unknown
field type or unknown key is refused (`widget-shape`).

### Why C4 is split in two (Q1)

C4a is all node code: formats, checks, methods, counting and their tests.
A reviewer can read it with no Hub. Its result is visible through `roymctl`
and the two-node test. C4b is all Hub code and the demo. It can rely on a
merged, tested API. The C3c2 code review needed six rounds, and most of its
late findings were in the Hub and the stories; a separate Hub PR keeps
those rounds away from the node code.

## How this is shown to an end user

C4a:

- Nothing in the Hub is new. A developer can send a poll with `roymctl`
  and read the counts.

C4b:

- In a chat within the messages the Hub shows (the first 200 messages today),
  Alice opens her chat with Bob and taps "Poll". She types "Which day
  shall we meet?" and two options, "Saturday" and "Sunday", and taps
  "Send poll".
- Both chats show the poll card: the question, the two options, "0 votes"
  next to each, and the line "Everyone in this chat can see each vote."
- Bob taps "Sunday". His chat shows "You chose: Sunday" and "1 vote" next
  to Sunday. (If sending his vote fails, Bob sees the failure state and a retry
  button). A few seconds later, Alice's chat shows "1 vote" next to
  Sunday too.
- Bob changes his mind and taps "Saturday". The counts move to Saturday on
  both chats.
- In a group, every member sees the same poll and the same counts.
- If a newer app sends a widget of a type this app does not know but built from
  the field types it does know, the chat shows its fields as plain labelled
  lines with "Not checked on this device", and the person can still answer it.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-core widget` (C4a): every shape
  rule, the 8 KiB limit (exactly 8,192 bytes passes, one byte more is
  refused), the poll type's rules and the answer check.
- `cargo nextest run -p syneroym-substrate --test roym_widget_e2e` (C4a):
  a poll in a 1:1 chat and in a group of three; each node counts the same
  newest vote of each person.
- `roymctl roym widget poll --conversation <id> --question "Which day?"
  --option Saturday --option Sunday`, then `roymctl roym widget list
  --conversation <id>` (C4a): the poll with `"count": 0` for each option.
- `mise run demo` (C4b): story 20 ends with Alice's poll and Bob's vote.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for
the recommended answer.

**Owner answers, 2026-11-25:** Q1 to Q9: the recommended answer of each
(Q1 (b), Q2 (b), Q3 (a), Q4 (a), Q5 (a), Q6 (a), Q7 (a), Q8 (a), Q9 (a)).
The same day, the owner asked for the simple `widget.list` of D-C4-7 (the
Hub sends the ids of the widgets it shows, and nothing is stored), not a
stored count (section 7.8).

#### Q1. How many PRs?

Owner answer: (b), 2026-11-25.

- (a) One PR. About 3,000 lines with tests. One close-out.
- **(b) Two PRs: C4a (node), then C4b (Hub and demo).** Recommended. Each
  PR has one reviewer question. C4a can be checked with `roymctl` and the
  two-node test alone. The C3c2 review showed that a large PR with node and
  Hub code together needs many rounds.

#### Q2. What carries a widget and an answer?

Owner answer: (b), 2026-11-25.

EX-12 says "a signed `widget-answer` record". ADR-0030 ("Consequences",
`:82`) leaves open whether that signature is a person signature. The
[backlog row](../../deferred-backlog.md) "A poll vote is signed by the
person, so it is provable outside the chat" (`:317`) asks this plan to
decide it with the owner.

- (a) **Person-signed records in deal cards.** A widget and an answer are
  Roym signed records (`principal::delegated`), carried in the card
  wrapper. Every voter's `conversation` service must hold a signing
  certificate. Every vote is provable to anyone, anywhere, forever, with
  the person's master DID (ADR-0030 decision 12 describes this loss for a
  `signed` message, which the person chooses; here nobody chooses). About
  1 KB and one signing call per vote. Deal cards are refused in a group
  today ([messages.rs:290](../../../../crates/roym_conversation/src/app/messages.rs#L290)),
  and `transaction` files every message of the card content type and counts
  it against the 2,000-card cap
  ([sync.rs:263](../../../../crates/roym_transaction/src/app/sync.rs#L263),
  [:280](../../../../crates/roym_transaction/src/app/sync.rs#L280)). So
  (a) also needs its own content type and changes in the group rule.
- **(b) Ordinary chat messages with their own content types.**
  Recommended. A widget and an answer are Roym message bodies. The
  message signature that every chat message carries says which member
  sent it, inside the chat. Nothing more is needed to count votes in that
  chat. A vote is as deniable as any message today. No signing
  certificate is needed to vote. `transaction` never sees them. They work
  in a group with no change to the card rule. The card table and
  `RECORD_TYPES` do not grow, so task.md's "twelve card types" and the
  extensions doc's "ten" become "ten deal card types, and two widget
  message types" (deviations, section 9).
- (c) The widget is a chat message, and only an answer is person-signed.
  It keeps the deniability problem of (a) and adds a second format.

The cost of (b): an answer cannot be shown to someone outside the chat as
proof. No widget in M08C needs that. A deal widget that must be evidence
(for example a slot picker that books) can add a signed form later, as its
own decision.

#### Q3. Widgets in groups?

Owner answer: (a), 2026-11-25.

The extensions doc's example is "A poll in a family chat" (`:146`).

- **(a) Yes, in every chat, 1:1 and group.** Recommended. With Q2 (b) it
  needs no new rule: the group refuses only the card content type.
- (b) 1:1 only in M08C. A family poll then waits for a later slice.

#### Q4. May a person change an answer?

Owner answer: (a), 2026-11-25.

- **(a) Yes. The newest valid answer of each person counts.** Recommended.
  The Hub lets the person tap another option. An earlier answer stays in
  the chat history and is not counted. There is no "take back my vote"
  (backlog row, section 8).
- (b) No. The first answer of each person is final. A wrong tap cannot be
  fixed.

"Newest" is by the sender's own clock, then by message id (D-C4-7). A
person's own answers are all stamped by that person's own node, so the
order of one person's answers does not depend on another node's clock.
If a voter's local clock steps back (e.g. time sync), a later vote carries
an older timestamp than the earlier one, so the earlier vote continues to
count (backlog row, section 8).

#### Q5. Which widget types ship?

Owner answer: (a), 2026-11-25.

- **(a) Only `poll/single`.** Recommended. The generic card draws any
  other type, from a newer build, from its field types (EX-11's last
  sentence), and a person can answer it. Tests reach the generic card and
  the answer hook point with a fixture of an unknown type.
- (b) Also a "question" widget with one free-text answer. The answer hook
  point and the generic card are then reachable by a person in the Hub. It
  adds a second composer and about six more Hindi messages.

#### Q6. Where is the poll type declared?

Owner answer: (a), 2026-11-25.

- **(a) A new manifest kind `chat`, and a new manifest
  `crates/roym_core/extensions/poll.json`.** Recommended. A `chat`
  manifest holds only `widgets`. The deal and listing paths see only
  `workflow` manifests
  ([extension.rs:180](../../../../crates/roym_core/src/extension.rs#L180),
  [:198](../../../../crates/roym_core/src/extension.rs#L198)), so a
  listing or a commitment cannot name `poll`, as for an `insight`.
- (b) A `widgets` key on a `workflow` manifest, as the sketch in the
  extensions doc shows for `scheduling`. A poll has no deal, so it would
  need a `workflow` extension with no resource kinds that a listing could
  still name.
- (c) A constant in `roym_core`, with no manifest. The Hub and the node
  then keep two lists, and the message-id rules of the manifests do not
  cover the poll.

#### Q7. What does a poll show?

Owner answer: (a), 2026-11-25.

Every member's node holds every answer message, so a vote is never secret
inside the chat.

- **(a) The count for each option, the person's own choice, and the line
  "Everyone in this chat can see each vote."** Recommended. EX-12 asks for
  the count. The line tells the truth about what the chat holds.
- (b) Also the names of the people under each option. Useful for "who is
  coming", but more screen work. A backlog row records it.

#### Q8. Demo

Owner answer: (a), 2026-11-25.

- **(a) Story 20 gets three steps: Alice sends a poll to Bob, Bob votes,
  Alice sees the count.** Recommended. task.md (Demo line) names story 20.
  A group poll is shown by the Playwright case in `roym-groups.spec.ts`
  and the two-node test, because the demo harness has two nodes.
- (b) No demo step. The Hub unit tests and Playwright cover it.

#### Q9. Which size limit applies to `conversation`?

Owner answer: (a), 2026-11-25.

task.md section 1: a component with new record code may grow by 15%, any
other by 40 KB. `conversation` is 813,818 bytes (C3c2 table).

- **(a) The widget and answer formats are new record code: the limit is
  +15% (122,072 bytes).** Recommended. They are new message formats with
  their own checks. The plan still aims at less than 40 KB (D-C4-12), and
  the first build of C4a step 3 measures it before any more work.
- (b) +40 KB. If the first build is over, C4a stops and asks.

### 2.2 Points found while reading

1. **Deal cards are refused in a group.** `conversation.send` refuses the
   card content type in a group with "Cards are sent only in a 1:1
   conversation." ([messages.rs:287](../../../../crates/roym_conversation/src/app/messages.rs#L287)
   to `:298`, [group.rs:45](../../../../crates/roym_core/src/conversation/group.rs#L45)).
   The Hub's group thread draws any card as a refused block
   ([groups.ts:188](../../../../crates/roym_web/ui/src/screens/groups.ts#L188)).
   This is why a widget must not use the card content type (Q2, Q3).
2. **`transaction` files every card message.** `classify_sync_message`
   files every message of `CARD_CONTENT_TYPE`
   ([sync.rs:263](../../../../crates/roym_transaction/src/app/sync.rs#L263)),
   counts it against `MAX_CARDS_PER_CONVERSATION` = 2,000
   ([transaction.rs:87](../../../../crates/roym_core/src/transaction.rs#L87),
   [sync.rs:280](../../../../crates/roym_transaction/src/app/sync.rs#L280)),
   and refuses a known type that it does not file
   ([sync.rs:448](../../../../crates/roym_transaction/src/app/sync.rs#L448)).
   A `widget` card type in `CARD_TYPES` would be filed and refused there.
   With their own content types, widget messages are skipped at `:263`,
   and `transaction` needs no change (task.md: "not `transaction`").
3. **A message id is the same on every node.** In a 1:1 chat the message
   id is inside the signed delivery payload
   ([envelope.rs:92](../../../../crates/conversation/src/envelope.rs#L92)
   to `:111`). In a group the message id is the entry id
   ([group.rs:439](../../../../crates/conversation/src/group.rs#L439)),
   derived from the signed header
   ([group.rs:85](../../../../crates/conversation/src/group.rs#L85),
   `:125`). So an answer can name its widget by message id.
4. **The order of history is by the sender's clock, and `visible_seq` is
   given when a message is accepted**
   ([message.rs:319](../../../../crates/conversation/src/store/message.rs#L319)
   to `:323`). So an answer's `visible_seq` is not always larger than its
   widget's. `widget.list` does not rely on it (D-C4-7).
5. **The field types have no list, and a `choice` field names only codes**
   ([schema.rs:23](../../../../crates/roym_core/src/extension/schema.rs#L23),
   [:55](../../../../crates/roym_core/src/extension/schema.rs#L55),
   `check_choices` at `:220`). A poll's options are the author's own
   words, not catalog codes. So a widget definition carries each choice as
   a code with its author text (D-C4-2), and the code, not the text, is
   what an answer holds and what `check_fields`
   ([schema.rs:108](../../../../crates/roym_core/src/extension/schema.rs#L108))
   checks.
6. **`roym_conversation` has no tests in its own crate.** Its methods are
   tested by the parity harness
   ([dual_build_parity/conversation.rs](../../../../crates/roym_web/tests/dual_build_parity/conversation.rs),
   [group.rs](../../../../crates/roym_web/tests/dual_build_parity/group.rs))
   and the substrate tests. The harness can deliver an inbound message
   (`h.deliver`, scenario 54 at
   [conversation.rs:50](../../../../crates/roym_web/tests/dual_build_parity/conversation.rs#L50)).
   C4 follows that pattern; the pure rules get unit tests in `roym_core`.
7. **Three docs state a card count that Q2 (b) makes false.** task.md
   (C4: "the card table has twelve types"), the extensions doc (`:148`:
   "three new generic card types … grows from seven types to ten") and the
   agreements doc (`:1026`: "grows from seven types to twelve"). The
   design is the extensions doc's, so its `## Deviations` gets the record.
   task.md gets the C4 result lines. The agreements sentence is fixed at
   C8 with that doc's "Differences" pass, as the other cross-doc changes
   are; this plan does not write a deviation into the agreements doc,
   because the design it describes is not the agreements doc's.
8. **The extensions doc names no test for EX-10 to EX-13**, and ML-26 and
   ML-28 name "Later: … (M08C C4)" in their test columns. C4 fills them.
9. **The search index takes every `+json` body**
   ([message.rs:36](../../../../crates/conversation/src/store/message.rs#L36)),
   and the Hub's search shows a snippet of the raw body
   ([message_search.ts:32](../../../../crates/roym_web/ui/src/screens/message_search.ts#L32)).
   A widget would show as raw JSON there, as a deal card does today.
   C4b draws a widget hit from its question and skips answer hits
   (D-C4-10).
10. **The manifest count is fixed in two tests.**
    [extension/tests.rs:52](../../../../crates/roym_core/src/extension/tests.rs#L52)
    and `:332` assert two manifests. The Hub list is compared with
    `MANIFESTS` in order
    ([extension/tests.rs:373](../../../../crates/roym_core/src/extension/tests.rs#L373)).
    The catalog test wants exactly the derived `ext-` ids in English and
    Hindi ([catalog.test.ts:178](../../../../crates/roym_web/ui/src/i18n/catalog.test.ts#L178)).
    So `poll.json`, `registry.ts` and both `.ftl` files change in the same
    step (C4a step 1).
11. **A group has at most 256 members**
    ([store.rs:100](../../../../crates/conversation/src/store.rs#L100)).
    So the newest answer per person is a short list.
12. **The Hub's thread loads the oldest 200 messages**
    (`conversation.history` with no `limit` and no cursor fetches in ascending
    chronological order,
    [messages.ts:386](../../../../crates/roym_web/ui/src/screens/messages.ts#L386);
    the default limit of 200 at
    [messages.rs:375](../../../../crates/roym_conversation/src/app/messages.rs#L375);
    store query `ORDER BY sender_timestamp ASC, author ASC, id ASC` at
    [store/history.rs:65](../../../../crates/conversation/src/store/history.rs#L65)).
    There is no paging in the Hub today. `widget.list` receives the ids of
    widgets the caller wants, reads each with `get_message`, and pages forward
    from the oldest asked widget to collect answers (D-C4-7).
13. **A widget message fits the mailbox.** A sealed mailbox message holds
    a body of about 13 KiB (`MAILBOX_CARD_BODY_BYTES`,
    [card.rs:40](../../../../crates/roym_core/src/card.rs#L40)). A widget
    body is at most 8 KiB, so a poll reaches a peer that is offline.
14. **C7 adds a forward mode to every message a person writes** (ADR-0030,
    decision 1). A message with no mode counts as `not-forwardable`
    (decision 3). A widget and an answer carry no mode in C4, so they
    cannot be forwarded. The C7 plan decides whether that stays.
15. **A vote in a 1:1 chat wakes `transaction`.** Its wake runs on
    `conversation.message` (ADR-0031). `transaction.drive` then reads the
    conversation and skips the message at
    [sync.rs:263](../../../../crates/roym_transaction/src/app/sync.rs#L263).
    No card is filed. This costs one sync per vote, as a text message does
    today.

## 3. What C4 meets

| Change doc | Criterion | Part | Where it is proved (section 7) |
| --- | --- | --- | --- |
| roym-extensions | EX-10 | C4a: a widget definition has an extension, a type, a semantic tag, typed fields and derived label ids; no markup, no script (every value is drawn as text) | unit `widget/tests.rs`, `extension/tests.rs`; Hub `widgets/*.test.ts` (C4b) |
| roym-extensions | EX-11 | Inline only. C4a: a definition up to 8 KiB is sent and shown; a larger one is refused at send and shown as refused at receive. The generic card draws an unknown type from its field types (C4b) | unit `widget/tests.rs`; parity 262, 264; Hub `widgets/generic.test.ts` |
| roym-extensions | EX-12 | With Q2 (b): an answer is a chat message that names the widget, not an agreement; the conversation is the one both are in. A poll shows a count per option to the people in the chat | parity 261, 263; two-node `roym_widget_e2e`; Hub `widgets/poll.test.ts`; Playwright |
| roym-extensions | EX-13 | The type name and field labels come from the catalog (derived ids) | unit `extension/tests.rs`; Hub `catalog.test.ts`, `hindi-screens.test.ts` |
| roym-multilingual | ML-26 | The widget-answer hook point: a free-text field of an answer gets the translate action, with the three cases of every hook point | Hub `widgets/generic.test.ts` (C4b) |
| roym-multilingual | ML-28 | Widget labels from the catalog, English as fallback | Hub `widgets/poll.test.ts`, `hindi-screens.test.ts`, `catalog.test.ts` |

Not met here: EX-11's fetch by hash (backlog row `:550`). ML-26's opinion
(M08G) and offering, want and storefront hook points (M08D).

## 4. Behaviour-change audit

### Wire changes (between nodes)

- **Two new chat message content types**:
  `application/vnd.roym.widget+json` and
  `application/vnd.roym.widget-answer+json` (D-C4-2). They travel as
  ordinary messages: in a 1:1 chat inside the signed delivery payload, in
  a group inside the signed, sealed entry. No WIT, envelope, record or
  card change.
- An older node shows them as messages of an unknown content type, as
  for any other content type it does not know.

### Local method and manifest changes (one node)

- **New local-only methods on `conversation`:** `widget.send`,
  `widget.answer`, `widget.list` (D-C4-6). The router gets one row,
  `("widget.", CONVERSATION, MethodAuth::Owner)`
  ([router.rs:25](../../../../crates/roym_core/src/router.rs#L25)).
  `api.invoke` stays local-only, as for every method of `conversation`.
- **`conversation.send` refuses the two widget content types** with "this
  content type is reserved"
  ([messages.rs:287](../../../../crates/roym_conversation/src/app/messages.rs#L287)).
  A widget or answer can then be sent only through the checked methods.
- **Extension manifests:** a third kind, `chat`, with one key, `widgets`
  (D-C4-3). `workflow` and `insight` manifests may not have `widgets`. A
  new file `crates/roym_core/extensions/poll.json`. `FieldType` gains
  `Serialize`.
- **Message ids:** two new derived ids per widget type (D-C4-4).
- **`roymctl roym widget poll | answer | list`** (D-C4-9).
- **Backup:** none. Widgets and answers are messages, so they are in the
  `conversation_history` section already
  ([backup.rs](../../../../crates/roym_conversation/src/app/backup.rs)).
  No schema version changes.

### Behaviour changes

- A vote is a chat message. It bumps the chat's last activity and wakes
  `transaction` in a 1:1 chat (point 15), as a text message does.
- The Hub (C4b) draws a widget from `widget.list`, draws no bubble for an
  answer, and draws a widget search hit from its question.

### Stays the same

- `transaction`, `catalog`, `scheduling`, `directory`, `profile` and
  `web` code. Every deal record, card and state.
- `CARD_TYPES` ([card.rs:11](../../../../crates/roym_core/src/card.rs#L11))
  and `RECORD_TYPES` ([record.rs:18](../../../../crates/roym_core/src/record.rs#L18)).
- The group rule for deal cards, and every other reserved content type.
- Every text message, its languages, its translate action and its
  deletion.
- Stories 10, 30, 40, 50 and 60. Story 20 gains steps (C4b).

## 5. Decisions this plan makes

### D-C4-1. Widgets are chat messages (Q2 (b), Q3 (a))

A widget and an answer are Roym message bodies of their own content types,
in any chat the person is a member of. The message signature authenticates
the sender inside the chat. No person signature, no signing certificate.

### D-C4-2. The formats

In a new module `crates/roym_core/src/widget.rs` (with `widget/tests.rs`):

```rust
pub const WIDGET_CONTENT_TYPE: &str = "application/vnd.roym.widget+json";
pub const WIDGET_ANSWER_CONTENT_TYPE: &str = "application/vnd.roym.widget-answer+json";
pub const WIDGET_BODY_VERSION: u32 = 1;
pub const ANSWER_BODY_VERSION: u32 = 1;

/// The whole message body, so the limit is what travels.
pub const MAX_WIDGET_BODY_BYTES: usize = 8 * 1024;
pub const MAX_ANSWER_BODY_BYTES: usize = 8 * 1024;
pub const MAX_WIDGET_FIELDS: usize = 16;
pub const MAX_WIDGET_CHOICES: usize = 32;
pub const MAX_WIDGET_TYPE_LEN: usize = 64;
pub const MAX_SEMANTIC_LEN: usize = 32;
pub const MAX_PROMPT_LEN: usize = schema::MAX_TEXT_LEN; // 512
pub const MAX_WIDGET_LABEL_LEN: usize = 128;
pub const MAX_CHOICE_TEXT_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetDefinition {
    pub extension: ExtensionRef,
    /// `<extension id>/<name>`.
    #[serde(rename = "type")]
    pub widget_type: String,
    /// A code: `poll`, `form`, `checklist`, ...
    pub semantic: String,
    /// The author's question, in the message's first language.
    pub prompt: String,
    /// What an answer holds.
    pub fields: Vec<DefinedField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinedField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// The author's label. A known type's field is labelled from the
    /// catalog instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `choice` only. Each choice is a code and the author's text for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<DefinedChoice>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinedChoice {
    pub code: String,
    pub text: String,
}

/// A widget message body: `{"widget_version": 1, "widget": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetBody {
    pub widget_version: u32,
    pub widget: WidgetDefinition,
}

/// An answer message body:
/// `{"answer_version": 1, "widget_id": "<message id>", "data": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerBody {
    pub answer_version: u32,
    pub widget_id: String,
    pub data: Value,
}
```

The free text (`prompt`, `label`, `text`) is in the language that the
message's `languages` names first, as for a text message. The definition
has no `lang` of its own: `conversation.send` already takes and checks
`languages` ([messages.rs:262](../../../../crates/roym_conversation/src/app/messages.rs#L262)).

A body with another `widget_version` or `answer_version` is refused, as a
card wrapper of an unknown version is ([card.rs:85](../../../../crates/roym_core/src/card.rs#L85)).

### D-C4-3. The manifest kind `chat` and `poll.json` (Q6 (a))

`crates/roym_core/extensions/poll.json`:

```json
{
  "id": "poll",
  "version": 1,
  "kind": "chat",
  "widgets": [
    {
      "type": "poll/single",
      "semantic": "poll",
      "fields": [
        { "name": "option", "type": "choice", "required": true, "min_choices": 2, "max_choices": 12 }
      ]
    }
  ]
}
```

In `crates/roym_core/src/extension.rs`:

```rust
pub enum ManifestKind { Workflow, Insight, /// Adds widgets to any chat.
    Chat }

pub struct Manifest {
    // ... the keys of today ...
    /// The widget types of a `chat` extension. Empty for the other kinds.
    #[serde(default)]
    pub widgets: Vec<WidgetType>,
}
```

In a new file `crates/roym_core/src/extension/widget.rs`:

```rust
/// One widget type that a `chat` extension declares.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetType {
    #[serde(rename = "type")]
    pub widget_type: String,
    pub semantic: String,
    pub fields: Vec<WidgetFieldRule>,
}

/// A field of a widget type. A `choice` field names no choices: the
/// author of each widget gives them, between `min_choices` and
/// `max_choices`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetFieldRule {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)] pub required: bool,
    #[serde(default)] pub max_len: Option<usize>,
    #[serde(default)] pub min: Option<i64>,
    #[serde(default)] pub max: Option<i64>,
    #[serde(default)] pub min_choices: Option<usize>,
    #[serde(default)] pub max_choices: Option<usize>,
}
```

Load rules (`load.rs`, a new `ManifestKind::Chat => check_chat(m)` arm at
[load.rs:36](../../../../crates/roym_core/src/extension/load.rs#L36)):

- A `chat` manifest has only `widgets`, and at least one. It has no
  `resource_kinds`, `hooks`, `service`, `details`, `steps` or `reads`.
- `check_workflow` and `check_insight` refuse a non-empty `widgets`.
- Each `type` is `<id>/<name>`, with `name_after_prefix`
  ([load.rs:105](../../../../crates/roym_core/src/extension/load.rs#L105)),
  at most `MAX_WIDGET_TYPE_LEN`, and unique in the manifest.
- `semantic` is a code (`listing::valid_token`, at most
  `MAX_SEMANTIC_LEN`).
- `fields`: 1 to `MAX_WIDGET_FIELDS`, unique names. Each field passes the
  name, `max_len` and number rules of `schema::check_definition`
  ([schema.rs:182](../../../../crates/roym_core/src/extension/schema.rs#L182));
  make `check_len_limit` and `check_number_limits` `pub(super)` and call
  them on a `Field` built from the rule. A `choice` field has
  `2 <= min_choices <= max_choices <= MAX_WIDGET_CHOICES`; any other
  field has neither. A `money` field is refused: a chat has no currency.

`Registry` gets:

```rust
/// A `chat` manifest's widget type.
pub fn widget_type(&self, ext: &ExtensionRef, widget_type: &str) -> Option<&WidgetType>;
```

`workflow()` is not changed. So `poll` is unknown to `listing.set`,
`quote.set` and the deal checks, as an `insight` is: `listing.set` refuses
it with `extension-unknown`, and a commitment naming it is "not checked".

**A small reader for `conversation` (D-C4-12).** `conversation` must not
link the whole registry (point: C3a1 measured +66 KB in `catalog` for
that, task.md section 1). In `extension/widget.rs`:

```rust
pub struct WidgetTypes { types: Vec<(ExtensionRef, WidgetType)> }

impl WidgetTypes {
    pub fn get(&self, ext: &ExtensionRef, widget_type: &str) -> Option<&WidgetType>;
}

/// The widget types of the manifests that ship with this build. It reads
/// only `chat` manifests, so a component that needs only widgets does not
/// link the deal schema checks. It always agrees with
/// `Registry::widget_type`.
pub fn widget_types() -> &'static WidgetTypes;
```

It deserializes each source of `MANIFESTS` into a header
`{id, version, kind, #[serde(default)] widgets}` (no `deny_unknown_fields`
on the header, as `known_workflow_in` at
[extension.rs:202](../../../../crates/roym_core/src/extension.rs#L202)),
keeps the `chat` ones, and is built once (`LazyLock`). It does not run the
load rules; `registry()` and the test `every_shipped_manifest_loads` do.

`MANIFESTS` becomes `[scheduling.json, one-shot.json, poll.json]`
([extension.rs:38](../../../../crates/roym_core/src/extension.rs#L38)),
and the Hub's `MANIFESTS` the same, in order
([registry.ts:66](../../../../crates/roym_web/ui/src/extensions/registry.ts#L66)).

### D-C4-4. Message ids (EX-13, ML-28)

In [ids.rs](../../../../crates/roym_core/src/extension/ids.rs), and the
same in the Hub's `registry.ts`:

```rust
/// The name of a widget type: `poll/single` -> `ext-poll-widget-single`.
pub fn widget_name_id(ext_id: &str, widget_type: &str) -> String;
/// The label of a field of a widget type:
/// `ext-poll-widget-single-option`.
pub fn widget_field_label_id(ext_id: &str, widget_type: &str, field: &str) -> String;
```

`manifest_message_sources` adds them, with the places
`"{ext}/widget/{name}"` and `"{ext}/widget/{name}/{field}"`, so
`check_message_ids` ([load.rs:163](../../../../crates/roym_core/src/extension/load.rs#L163))
refuses a clash. The Hub's `manifestMessageIds`
([registry.ts:126](../../../../crates/roym_web/ui/src/extensions/registry.ts#L126))
adds the same, so `catalog.test.ts` asks for them in both catalogs.

New catalog messages: `ext-poll-widget-single = Poll`,
`ext-poll-widget-single-option = Your choice`, and Hindi (section 7.10).

The author's `prompt`, `label` and choice `text` are content, not labels.
They are shown as written, with the message's language. The deviation of
ML-28 says so (section 9).

### D-C4-5. The checks, and their error codes

In `widget.rs`. Each error has `code()`, as `StepError` has
([step.rs:72](../../../../crates/roym_core/src/step.rs#L72)), and the
methods answer `-32602` with `data.code`, as `step.add` does
([step_ops.rs:103](../../../../crates/roym_transaction/src/app/step_ops.rs#L103)).

```rust
pub enum WidgetError {
    TooLarge { bytes: usize },          // "widget-too-large"
    UnknownVersion(u32),                // "widget-version"
    Json(String),                       // "widget-json"
    Shape(String),                      // "widget-shape"
    TypeUnknown(String),                // "extension-unknown"
    TypeRules(String),                  // "widget-rules"
    AnswerTooLarge { bytes: usize },    // "answer-too-large"
    Answer(String),                     // "answer-invalid"
    NotFound,                           // "widget-not-found"
    NotAWidget,                         // "not-a-widget"
    Deleted,                            // "widget-deleted"
    Refused(String),                    // "widget-refused" (the widget itself fails)
}

pub enum WidgetCheck { Checked, NotChecked }

/// Size first, then parse to `serde_json::Value`, check `widget_version`
/// (returns `UnknownVersion` / "widget-version"), and deserialize the
/// definition (mapping any serde failure to `Shape` / "widget-shape").
pub fn parse_widget_body(body: &[u8]) -> Result<WidgetDefinition, WidgetError>;
/// Shape, then size of the serialized body.
pub fn widget_body(def: &WidgetDefinition) -> Result<String, WidgetError>;
/// The rules for every widget, known or not.
pub fn check_shape(def: &WidgetDefinition) -> Result<(), WidgetError>;
/// The type's rules, when this build knows the type.
pub fn check_type(types: &WidgetTypes, def: &WidgetDefinition) -> Result<WidgetCheck, WidgetError>;
/// `data` against the definition's own fields.
pub fn check_answer(def: &WidgetDefinition, data: &Value) -> Result<(), WidgetError>;
pub fn parse_answer_body(body: &[u8]) -> Result<AnswerBody, WidgetError>;
pub fn answer_body(widget_id: &str, data: &Value) -> Result<String, WidgetError>;
/// The one poll type, with the codes `o1`, `o2`, ... in order.
pub fn poll_definition(prompt: &str, options: &[String]) -> WidgetDefinition;
```

`check_shape`:

1. `extension.id` is an extension id (`is_extension_id`) and `version >= 1`.
2. `widget_type` is `<extension.id>/<name>`, name a code, at most
   `MAX_WIDGET_TYPE_LEN`.
3. `semantic` is a code of at most `MAX_SEMANTIC_LEN`.
4. `prompt` is not blank and at most `MAX_PROMPT_LEN` bytes.
5. `fields`: 1 to `MAX_WIDGET_FIELDS`, unique names, no `money`. Each
   field, turned into a `schema::Field` (choices = the codes), passes
   `schema::check_definition` (make it `pub(crate)`). A `choice` field has
   1 to `MAX_WIDGET_CHOICES` choices; each `text` is not blank and at most
   `MAX_CHOICE_TEXT_LEN` bytes. A `label`, when present, is not blank and
   at most `MAX_WIDGET_LABEL_LEN` bytes.
   All structs deny unknown fields (`deny_unknown_fields`), and `field_type`
   must deserialize into one of today's eight variants of `FieldType`.
   In `parse_widget_body`, the body is parsed to `serde_json::Value` first,
   `widget_version` is checked, and deserialization into `WidgetBody` maps
   any serde failure (unknown field type like `location` or unknown key) to
   `Shape("widget-shape")` (P3).
   For `widget.send`, input deserialization of `WidgetSendInput` with an
   unknown field type fails at the RPC layer as invalid params (`-32602`)
   without `data.code` (P3).

`check_type`: `types.get(&def.extension, &def.widget_type)`. `None` gives
`NotChecked`. Otherwise `semantic` equals the type's; the field names, in
order, equal the type's; each field has the type's `field_type`,
`required`, `max_len`, `min` and `max`; a `choice` field's choice count is
within `min_choices..=max_choices`; a field of a known type has no
`label`. Any mismatch is `TypeRules`.

`check_answer`: `AnswerBody` size first (`MAX_ANSWER_BODY_BYTES`, checked
by `answer_body` and `parse_answer_body`), then
`schema::check_fields(&def.answer_fields(), data)`
([schema.rs:108](../../../../crates/roym_core/src/extension/schema.rs#L108)),
where `answer_fields()` turns each `DefinedField` into a `schema::Field`.
This is the only check of what an answer holds. It works for a type this
build does not know.

### D-C4-6. The methods

Input and output types in a new `crates/roym_core/src/method/widget.rs`
(so `roymctl`, the parity tests and the service share them). The service
code in a new `crates/roym_conversation/src/app/widget.rs`, and the rows in
[api.rs](../../../../crates/roym_conversation/src/api.rs#L23).

```rust
pub struct WidgetSendInput {
    pub conversation: String,
    pub widget: WidgetDefinition,
    #[serde(default)] pub languages: Vec<LangTag>,
}
pub struct WidgetAnswerInput {
    pub conversation: String,
    pub widget_id: String,
    pub data: Value,
    #[serde(default)] pub languages: Vec<LangTag>,
}
pub struct WidgetListInput {
    pub conversation: String,
    /// The widgets the caller draws, at most `MAX_LISTED_WIDGET_IDS`.
    /// Empty: every widget in the scan (`roymctl`).
    #[serde(default)] pub widget_ids: Vec<String>,
}
// widget.send and widget.answer return SendOutput (method/conversation.rs:20).
```

**Shared send path.** Move the body of `conversation.send` from `messages.rs:273`
into a shared function in `messages.rs`:

```rust
pub(crate) async fn send_body<H: AppHost, F>(
    host: &H,
    conversation: String,
    content_type: &str,
    languages: Vec<String>,
    body: Vec<u8>,
    check_reserved: F,
) -> Reply<SendOutput>
where
    F: FnOnce(bool /* is_group */, &str /* content_type */) -> Option<&'static str>;
```

`send_body` performs the group checks once (`group_info`, `GROUP_RESTORED_NOTICE`,
`GROUP_REMOVED_NOTICE`), then invokes `check_reserved(is_group, content_type)`.
`conversation.send` passes today's reserved check (including cards not in groups)
plus the two widget content types ("this content type is reserved").
`widget.send` and `widget.answer` pass a check that returns `None`.
This preserves the exact error precedence: a removed group member receives
`GROUP_REMOVED_NOTICE` rather than a reserved content type error (scenarios 207
and 208 remain unchanged).

```text
widget.send(input):
  languages = check_tag_list(input.languages)          // as conversation.send
  check_shape(input.widget)                              -> -32602 code
  match check_type(widget_types(), input.widget):
    NotChecked -> -32602 "extension-unknown"             // a node sends only types it knows
    Checked    -> ok
  body = widget_body(input.widget)                       -> "widget-too-large"
  send_body(host, conversation, WIDGET_CONTENT_TYPE, languages, body, |_, _| None)

widget.answer(input):
  languages = check_tag_list(input.languages)
  m = AppConversation::get_message(input.widget_id)
        NotFound -> "widget-not-found"
  if m.conversation != input.conversation           -> "widget-not-found"
  if m.content_type != WIDGET_CONTENT_TYPE          -> "not-a-widget"
  if m.deleted_at.is_some()                         -> "widget-deleted"
  def = parse_widget_body(m.body)                    -> "widget-refused" (with the inner code in the message)
  check_answer(def, input.data)                      -> "answer-invalid"
  body = answer_body(input.widget_id, input.data)    -> "answer-too-large"
  send_body(host, conversation, WIDGET_ANSWER_CONTENT_TYPE, languages, body, |_, _| None)
```

`widget.answer` accepts a widget of a type this node does not know: the
answer is checked against the widget's own fields (Q5 (a)).

### D-C4-7. `widget.list`: the scan and the count

```rust
/// A cap for one call: messages scanned forward from the oldest asked widget.
pub const MAX_WIDGET_SCAN_MESSAGES: usize = 1_000;
/// The Hub's page is 200 messages, so it never asks for more widgets.
pub const MAX_LISTED_WIDGET_IDS: usize = 200;
/// Output cap when listing widgets without ids (e.g. roymctl).
pub const MAX_UNFILTERED_WIDGETS: usize = 50;

pub struct WidgetListOutput {
    pub widgets: Vec<WidgetView>,
    /// False when the scan stopped at the cap before the end of chat.
    pub complete: bool,
}
pub struct WidgetView {
    pub widget_id: String,
    pub author: String,
    pub direction: Direction,
    pub sender_timestamp_ms: i64,
    pub languages: Vec<String>,
    /// `None` when `refused` is set.
    pub definition: Option<WidgetDefinition>,
    /// The code of `WidgetError` when this widget fails its checks.
    pub refused: Option<String>,
    /// True when this build knows the type and its rules pass.
    pub checked: bool,
    /// One row per choice of each `choice` field, in definition order,
    /// with 0 for a choice nobody picked.
    pub counts: Vec<ChoiceCount>,
    /// The newest valid answer of each author, newest first.
    pub answers: Vec<AnswerView>,
    /// Answers that named this widget and failed `check_answer`.
    pub ignored_answers: u32,
    pub state: StoredState,
    pub last_error: Option<String>,
}
pub struct ChoiceCount { pub field: String, pub choice: String, pub count: u32 }
pub struct AnswerView {
    pub message_id: String,
    pub author: String,
    pub direction: Direction,
    pub sender_timestamp_ms: i64,
    pub languages: Vec<String>,
    pub data: Value,
    pub state: StoredState,
    pub last_error: Option<String>,
}
```

```text
widget.list(conversation, widget_ids):
  if widget_ids.len() > MAX_LISTED_WIDGET_IDS: -32602
  widgets = []; answers = []; complete = true

  if widget_ids is not empty:
    // P1 & P4: Fetch asked widgets directly; drop missing, deleted, foreign or non-widget
    valid_widgets = []
    for wid in widget_ids:
      m = AppConversation::get_message(wid)
      if m is NotFound or m.conversation != conversation: continue
      if m.content_type != WIDGET_CONTENT_TYPE: continue
      if m.deleted_at.is_some(): continue
      valid_widgets.push(m)
    if valid_widgets is empty:
      return { widgets: [], complete: true }

    widgets = valid_widgets
    // Oldest asked widget by (sender_timestamp, author, id)
    oldest = min(widgets, by: (sender_timestamp, author, id))
    cursor = Some(oldest.id)
    seen = 0
    loop:
      page = AppConversation::history(conversation, 200, cursor)
      for item in page.items:
        if item is not a Message: continue
        seen += 1
        if m.deleted_at.is_some(): continue
        if m.content_type == WIDGET_ANSWER_CONTENT_TYPE:
          answers.push(m)
        if seen == MAX_WIDGET_SCAN_MESSAGES:
          complete = false; break
      if not complete or page.next_cursor is None: break
      cursor = page.next_cursor

  else:
    // Empty widget_ids (roymctl): page forward from start, capped at MAX_UNFILTERED_WIDGETS
    cursor = None; seen = 0
    loop:
      page = AppConversation::history(conversation, 200, cursor)
      for item in page.items:
        if item is not a Message: continue
        seen += 1
        if m.deleted_at.is_some(): continue
        if m.content_type == WIDGET_CONTENT_TYPE:
          if widgets.len() < MAX_UNFILTERED_WIDGETS:
            widgets.push(m)
          else:
            complete = false
        if m.content_type == WIDGET_ANSWER_CONTENT_TYPE:
          answers.push(m)
        if seen == MAX_WIDGET_SCAN_MESSAGES:
          complete = false; break
      if not complete or page.next_cursor is None: break
      cursor = page.next_cursor

  views = widgets.map(project)            // parse_widget_body, check_type; carries m.state and m.last_error; refused code on error
  by_widget = answers.filter_map(parse_answer_body).group_by(widget_id)
  for view with a definition:
    valid = by_widget[view.widget_id].filter(check_answer ok)
    view.ignored_answers = the others' count
    latest = per author: max by (sender_timestamp_ms, message_id)
    view.answers = latest sorted newest first (carries state and last_error)
    view.counts = for each choice field, each choice: count of latest whose data[field] == code
  return { widgets: views, complete }
```

Rules, each with a test (section 7):

- **Newest valid answer per author** (Q4 (a)). An invalid answer is
  dropped before the newest is chosen, so a bad message never hides a good
  earlier one. The tie-break by message id is the same on every node
  (point 3).
- **Own answer state and retry** (P2). An own answer in `failed` or
  `pending` state is counted locally on the voter's node (as history returns
  own outgoing messages regardless of admission). `AnswerView` carries
  `state` and `last_error` so the Hub displays the state word and retry
  button for the person's own answer.
- **Own widget state and retry** (P10). `WidgetView` carries `state` and
  `last_error` from the widget's message row. For an own widget
  (`direction == Direction::Outgoing`), the Hub shows its state word when
  not `delivered`, and if `failed`, shows a retry button calling
  `conversation.retry` with `view.widget_id`.
- **A deleted answer is not counted.** The author's earlier answer then
  counts again. The Hub offers no delete for an answer (it draws no bubble
  for it), so this happens only through `conversation.delete-message`.
- **Deleted, missing or foreign widget ids** (P4). An asked id that is
  deleted, missing, not of `WIDGET_CONTENT_TYPE`, or belonging to another
  conversation is omitted from `widgets` and does not start a scan. The Hub
  does not ask for ids of messages with `deleted_at_secs` (drawn as "message
  deleted"). A drawn widget whose id is missing from `widget.list` draws
  `widget-refused`.
- **Every message history returns counts**: own and peers', and restored
  rows. History returns only the person's own messages and accepted ones
  ([history.rs:64](../../../../crates/conversation/src/store/history.rs#L64)),
  ordered by sender time, author and id.
  A restored row has `verified: false`, but it was accepted before it was
  backed up; skipping it would empty every poll after a restore.
- **An answer whose widget is not in the scan is not counted.** An answer
  never names a widget in another conversation in the count, because only
  this conversation's history is read.
- **The scan pages forward from the oldest asked widget** (P1). The Hub passes
  the ids of the widgets in the messages it loaded (today within the oldest
  200, point 12). The scan fetches those definitions directly, finds the
  oldest by `(sender_timestamp, author, id)`, and pages forward with
  `history(conversation, 200, cursor = Some(oldest.id))` up to 1,000 messages.
  If the cap is hit before reaching the end of chat, `complete: false`.
- **Clock skew and ordering** (P9). History is in sender-time order (point 4).
  Answers sort after their widget in history. If a sender's clock is behind
  such that an answer sorts before its widget, it is missed by the forward
  scan and not counted (backlog row, section 8). If a voter's local clock
  steps back, their subsequent vote carries an older timestamp and will not
  replace their earlier vote in `max by (sender_timestamp_ms, message_id)`
  (backlog row, section 8).
- **Size of the output** (P8). When `widget_ids` is specified, at most 200
  widgets are returned. When `widget_ids` is empty, at most
  `MAX_UNFILTERED_WIDGETS = 50` widgets are returned (at most ~400 KiB of
  definitions, well below message size limits, avoiding up to 8 MB
  unbounded replies).

### D-C4-8. Groups

`widget.send` and `widget.answer` go through `send_body`, so a removed
member and a restored group are refused with today's notices
(`GROUP_REMOVED_NOTICE`, `GROUP_RESTORED_NOTICE`,
[messages.rs:275](../../../../crates/roym_conversation/src/app/messages.rs#L275)
to `:281`). In a group, `author` is the member's address, as in history.
The answers of a member who has left still count: their messages stay in
the group's history.

### D-C4-9. `roymctl`

A new `apps/roymctl/src/commands/roym/widget.rs`, wired in `roym.rs` like
`Step` ([roym.rs:98](../../../../apps/roymctl/src/commands/roym.rs#L98)):

```text
roymctl roym widget poll   --conversation <id> --question <text> --option <text> (2 to 12 times) [--lang <tag>]...
roymctl roym widget answer --conversation <id> --widget <message id> --data <json> [--lang <tag>]...
roymctl roym widget list   --conversation <id>
```

`poll` builds the definition with `roym_core::widget::poll_definition`, so
`roymctl` and the Hub build the same body as the node checks.

### D-C4-10. The Hub (C4b)

New folder `crates/roym_web/ui/src/widgets/`:

- `view.ts`: the `WidgetView`, `AnswerView`, `ChoiceCount` types, and
  `pollDefinition(question, options)`, which builds the same definition as
  `poll_definition` (codes `o1`, …). A test compares it with a fixture
  that the Rust test writes the same way.
- `poll.ts`: `renderPoll(view, opts)`. The heading is the catalog name
  for a known type, else `widget-unknown-heading` with the type code;
  "Not checked on this device" (`ext-not-checked`) when `checked` is false
  (P7). The prompt with `messageText(prompt, view.languages, "p", {
  translatable: incoming })` (the chat message hook point,
  [message_text.ts:33](../../../../crates/roym_web/ui/src/chat/message_text.ts#L33));
  the line `widget-votes-visible`; one button per option with the choice
  text (`textContent`, `lang` from the first language) and the count
  (`widget-poll-count`, a Fluent plural); and "You chose: …" for the
  person's own newest answer. If the person's newest own answer is not
  `delivered`, show its state word (`messages-state-*`), and if `failed`,
  show a retry button calling `conversation.retry` with `AnswerView.message_id`
  (P2). A tap on an option finds the choice field name from the definition
  (`const fieldName = view.definition.fields[0].name;`) and calls `widget.answer`
  with `{ [fieldName]: code }` (P7), then redraws. A tap on the person's
  current choice does nothing.
- `generic.ts`: `renderGenericWidget(view, opts)`. The heading is the
  catalog name for a known type, else `widget-unknown-heading` with the
  type code; "Not checked on this device" (`ext-not-checked`) when
  `checked` is false. One input per field, by type: `text` and `code` an
  `<input>` with `maxlength`; `long-text` a `<textarea>`; `number` an
  `<input type="number">` with `min` and `max`; `date-time` an `<input
  type="datetime-local">`, sent as seconds; `duration` minutes, sent as
  seconds; `choice` a `<select>` of the author texts. The label is the
  catalog label for a known type, else the author `label`, else the field
  name. If the person's newest own answer is not `delivered`, show its state
  word, and if `failed`, show a retry button (P2). "Send answer" calls
  `widget.answer`. Under the form, the answers: each author's name and its
  fields, drawn with `fieldView`
  ([fields.ts:65](../../../../crates/roym_web/ui/src/cards/fields.ts#L65))
  for the non-text fields, and each `text` or `long-text` value with
  `messageText(value, answer.languages, "span", { translatable: incoming
  })`. **This is the widget-answer hook point** (ML-26).
- `render.ts`: `renderWidget(view, opts)`. It picks `renderPoll` when
  `semantic` is `poll` and the definition has exactly one required
  `choice` field, else `renderGenericWidget`. A `refused` view draws
  `renderRefusedCard`-like text: `widget-too-large` for that code, else
  `widget-refused` with the reason. For an own widget (`view.direction === 'outgoing'`),
  it shows its state word when not `delivered` (`messages-state-*`), and if
  `failed`, shows a retry button calling `conversation.retry` with `view.widget_id`
  (P10). The Hub ignores `complete: false` when rendering widgets (it draws
  the counts collected up to the scan cap), because the Hub thread shows only
  the oldest 200 messages today; recorded in the backlog row (P11).
- `composer.ts`: the "Poll" button beside the composer, and its form:
  the question, two option inputs, "Add option" up to 12, "Remove" down to
  2, "Send poll". It calls `widget.send` with `pollDefinition` and the
  composer's language list. A blank question, a blank option or two equal
  options keep the button off, with the reason.

Wiring:

- `messages.ts` `fillThread`
  ([messages.ts:363](../../../../crates/roym_web/ui/src/screens/messages.ts#L363)):
  `conversation.history` loads the oldest 200 messages today (P1). Call
  `widget.list` next to `transaction.thread`, with the ids of non-deleted
  widget messages in the loaded history (so `conversation.history` comes
  first), and keep a map by `widget_id`. A deleted message is drawn as
  "message deleted" and its id is not sent to `widget.list` (P4). With no
  widget message in the page, it makes no `widget.list` call. In
  `messageElement`
  ([messages.ts:517](../../../../crates/roym_web/ui/src/screens/messages.ts#L517)):
  a widget message draws `renderWidget` from the map, or `widget-refused`
  when the map has none; an answer message is not drawn (as a `repeat`
  card is not, `:396`). The state word and retry action for the widget message
  are rendered on the widget card (P10).
- `groups.ts` `reloadThread` (`:656`) calls `widget.list`, and
  `renderThreadMessages` (`:151`) takes the map and does the same.
- Both composers get the "Poll" button.
- `message_search.ts` `searchHits` (`:32`): a widget hit's snippet is its
  prompt; an answer hit is skipped.
- `registry.ts`: `kind` adds `"chat"`, `widgets?` on `Manifest`,
  `widgetType(ext, type)`, and the two id functions (C4a step 1).

Every value is drawn with `textContent` and every attribute from data is
`lang` only, as for every card template
([fields.ts:7](../../../../crates/roym_web/ui/src/cards/fields.ts#L7)).

### D-C4-11. Translation

- The prompt of a peer's widget: the chat message hook point (existing).
- A free-text field of a peer's answer: the widget-answer hook point
  (new, ML-26). It uses `messageText`, so the three cases (no engine,
  exact params, `unavailable`) behave as for a chat message; C4b adds the
  three tests for this hook point.
- The choice texts and the author labels: drawn with their `lang`, not
  offered to machine translation. Backlog row (section 8), like the one
  for free text in `details` (extensions doc, `:231`).

### D-C4-12. The size of `conversation`

`conversation` links `widget.rs`, `extension/widget.rs` (the small reader)
and `schema::check_fields`, and not `Registry`. C4a step 3 builds
`conversation` before the Hub and the tests and records the size. Stop
rule: over +40 KB, look for what linked more than this list (as C3a1 did
for `catalog`); over the Q9 limit, stop and tell the owner.

### D-C4-13. What C4 does not keep

No collection, no ledger row, no backup section. The count is computed
from history on each `widget.list`. A restore brings back the messages,
so the polls and their counts come back.

## 6. Step-by-step changes

Each step builds, and its tests pass, before the next starts.

### 6.1 C4a — Widgets on the node

#### Step 1 — The manifest kind and `poll.json` (`roym_core`, Hub registry)

- `ManifestKind::Chat`, `Manifest.widgets`, `extension/widget.rs`
  (`WidgetType`, `WidgetFieldRule`, `WidgetTypes`, `widget_types()`), the
  `check_chat` rules and the refusals in `check_workflow` and
  `check_insight`, `Registry::widget_type`.
- `ids.rs`: the two id functions and the sources.
- `poll.json`, `MANIFESTS` with three entries; the two count asserts
  (point 10) become 3.
- Hub: `registry.ts` (import `poll.json`, `kind`, `widgets`, `widgetType`,
  the id functions, `manifestMessageIds`), `en.ftl` and `hi.ftl` (the two
  `ext-poll-…` messages). These are here because the Rust test of the Hub
  list and the catalog test fail otherwise.
- Tests: section 7.1, the manifest part.

#### Step 2 — The formats and checks (`roym_core`, pure)

- `widget.rs` and `widget/tests.rs` (D-C4-2, D-C4-5), `FieldType:
  Serialize`, `schema::check_definition` to `pub(crate)`.
- `method/widget.rs` (D-C4-6, D-C4-7 types).
- Router row `widget.`; the router test that each prefix routes to its
  service.
- Tests: section 7.1, the format part.

#### Step 3 — The methods (`roym_conversation`)

- `send_body` in `messages.rs`; `conversation.send` reserves the two
  content types.
- `app/widget.rs` with `send`, `answer`, `list`; the three rows in
  `api.rs`.
- `mise run build:roym`. Record the size of `conversation` (D-C4-12) and
  apply the stop rule **before step 4**.

#### Step 4 — Parity and method contract (`roym_web` tests)

- Parity scenarios 260 to 265 in a new
  `crates/roym_web/tests/dual_build_parity/widget.rs`, registered in
  `dual_build_parity.rs`.
- `UPDATE_METHOD_SHAPES=1` for the three new methods; the probes in
  `method_contract/shapes.rs`.
- Update `contract.rs:40` assertion `table.len() == 24` to 27 for
  `conversation`; `every_conversation_method_has_typed_input_and_output`
  ([contract.rs:38](../../../../crates/roym_web/tests/method_contract/contract.rs#L38))
  passes with the new file (P5).
- Add `widget.send`, `widget.answer`, and `widget.list` to the scenario 73
  method list in `wire_origin.rs:116` (creating a widget first for
  `widget.answer`), asserting they are wired and locally admitted (P5).

#### Step 5 — `roymctl roym widget`

- D-C4-9, with a parse test in `apps/roymctl/src/commands/roym/tests.rs`.

#### Step 6 — The two-node test

- `crates/substrate/tests/roym_widget_e2e.rs` (section 7.4).

#### Step 7 — Sizes, docs and close-out of C4a

- Sizes of all seven components (section 7.9) into task.md section 1.
- Deviations (section 9), backlog (section 8), living docs of the node
  part (section 9), the C4a row in status.md. Demo line: `- none: no
  visible change; roymctl and the two-node test show it`.
- `mise run verify` with no `--skip`.

### 6.2 C4b — Widgets in the Hub

#### Step 1 — The renderers

- `widgets/view.ts`, `poll.ts`, `generic.ts`, `render.ts` and their tests
  (section 7.5), with the Hub messages (section 7.10).

#### Step 2 — The composer and the threads

- `widgets/composer.ts`; `messages.ts` and `groups.ts` wiring;
  `message_search.ts`.

#### Step 3 — Hindi and the hook point

- `hi.ftl`; `hindi-screens.test.ts` ("draws a poll in Hindi"); the three
  hook-point cases in `widgets/generic.test.ts`.

#### Step 4 — Playwright and the demo

- `roym-groups.spec.ts`: one case, a poll in a group.
- Story 20: three steps (Q8). New helpers in `hub-helpers.ts`:
  `sendPoll(page, question, options)`, `votePoll(page, question, option)`,
  `pollCount(page, question, option)`.
- `mise run demo`.

#### Step 5 — Close-out of C4

- Living docs of the Hub part, demo.md, the multilingual deviations, the
  owner's review of the Hindi messages, status.md, `mise run verify` with
  no `--skip`.

## 7. Tests

### 7.1 Unit tests (`roym_core`)

`crates/roym_core/src/extension/tests.rs` (or a new
`extension/widget/tests.rs` if the file grows past 800 lines):

- `a_chat_manifest_loads_and_its_widget_type_is_found`
- `a_chat_manifest_with_a_deal_key_is_refused` (one case each for
  `resource_kinds`, `hooks`, `details`, `steps`, `reads`)
- `a_workflow_or_insight_manifest_with_widgets_is_refused`
- `a_widget_type_must_start_with_its_extension_id`
- `a_widget_choice_rule_needs_min_and_max_choices` (and both bounds)
- `a_widget_field_of_type_money_is_refused`
- `a_listing_cannot_name_a_chat_extension` (`is_known_workflow` is false)
- `a_commitment_naming_a_chat_extension_is_not_checked` (beside the
  insight case in `deal/tests/extension.rs`)
- `widget_types_agree_with_the_registry`
- `widget_ids_are_derived_and_a_clash_is_refused`

`crates/roym_core/src/widget/tests.rs`:

- `a_poll_definition_passes_every_check`
- One test per `check_shape` rule (1 to 5 in D-C4-5), each with the code.
- `a_body_of_exactly_the_limit_passes_and_one_byte_more_is_refused`
  (8,192 bytes)
- `an_unknown_body_version_is_refused`
- `a_body_with_unknown_version_and_extra_keys_returns_widget_version` (P3)
- `a_body_with_unknown_field_type_returns_widget_shape` (P3)
- `a_poll_with_one_or_thirteen_options_breaks_its_type_rules`
- `a_known_type_with_another_semantic_or_field_breaks_its_rules`
- `an_unknown_type_is_not_checked_but_its_shape_is`
- `an_answer_with_an_unknown_choice_an_extra_key_or_no_required_field_is_refused`
- `an_answer_to_an_unknown_type_is_checked_against_its_own_fields`
- `poll_definition_builds_the_hub_fixture` (writes the fixture that the
  Hub's `view.test.ts` reads, or compares with it)

### 7.2 Parity (both builds agree), `dual_build_parity/widget.rs`

- `scenario_260_widget_send_and_list_parity`: own poll, listed with zero
  counts, `checked: true`.
- `scenario_261_inbound_answers_count_the_newest_of_each_author_parity`:
  an inbound poll and inbound answers from two authors, one author
  answers twice; a deleted answer brings back the earlier one; an invalid
  answer is in `ignored_answers` and does not hide an earlier valid one;
  own answer in `failed` state is counted locally with its state and last_error
  in `AnswerView` (P2).
- `scenario_265_widget_list_forward_scan_parity`: with `widget_ids`, only
  those widgets are listed, a missing or deleted id is left out; the scan
  pages forward from the oldest asked widget to collect answers and stops
  after 1,000 messages (`complete: false`) (P1, P4); with empty `widget_ids`,
  at most `MAX_UNFILTERED_WIDGETS = 50` widgets are returned, with
  `complete: false` when more exist (P8).
- `scenario_262_widget_refusals_parity`: `widget.send` with an unknown
  type (`extension-unknown`), a body over 8 KiB (`widget-too-large`), a
  definition breaking a `check_shape` rule (`widget-shape`, P3; `widget.send`
  with an unknown field type is rejected as invalid params at RPC parse);
  `conversation.send` with each widget content type (reserved);
  `widget.answer` to a text message (`not-a-widget`), to a widget of
  another conversation (`widget-not-found`), to a deleted widget
  (`widget-deleted`), with a bad choice (`answer-invalid`).
- `scenario_263_a_poll_in_a_group_parity`: a group poll and answers in
  the group (the group helpers of `group.rs`); a removed member cannot
  answer (`GROUP_REMOVED_NOTICE`).
- `scenario_264_inbound_bad_or_unknown_widgets_parity`: an inbound widget
  over 8 KiB (`widget-too-large`), one with a bad shape, and one with an
  unknown field type (e.g. `location`, P3) are listed with `refused` codes
  (`widget-too-large`, `widget-shape`); an inbound unknown type (built from
  known field types) is listed with `checked: false`, and an answer to it
  is sent and counted.

### 7.3 Method contract

- Shapes for `widget.send`, `widget.answer` and `widget.list`
  (`UPDATE_METHOD_SHAPES=1`), and their probes.
- `contract.rs:40` assertion updated from 24 to 27 for `conversation` (P5).
- Scenario 73 in `wire_origin.rs` updated to include all three methods,
  asserting they are wired and locally admitted (neither `-32601` nor
  `-32013`) (P5).

### 7.4 Two-node and three-node (`crates/substrate/tests/roym_widget_e2e.rs`)

- `a_poll_in_a_direct_chat_counts_each_persons_newest_vote`: Alice's node
  sends a poll with `widget.send`. Bob's node waits until `widget.list`
  shows it (`wait_until`, `common/roym.rs:683`). Bob answers `o2`; both
  nodes show `o2: 1`. Bob answers `o1`; both show `o1: 1, o2: 0`. Use the
  1:1 setup of `roym_conversation_e2e.rs` and `deliver_one_message`
  (`common/roym_flow.rs:68`).
- `a_poll_in_a_group_shows_the_same_counts_on_every_member`:
  `boot_trio`, `form_group` (`common/roym_group.rs:39`, `:72`). The owner
  sends a poll; two members answer; `converge`; all three nodes return the
  same `counts`.

### 7.5 Hub unit tests (`npm test`)

- `widgets/view.test.ts`: `pollDefinition` equals the Rust fixture.
- `widgets/poll.test.ts`: heading from catalog for known type, else generic
  code (P7); `ext-not-checked` drawn when `checked: false` (P7); zero counts;
  the person's own choice; a tap calls `widget.answer` with `{ [fieldName]: code }`
  derived from the definition (P7); a tap on the current choice makes no call;
  a choice text with markup is drawn as text; the `lang` of the options;
  state word and retry button drawn when an own poll or own answer has state
  `failed` (P2, P10).
- `widgets/generic.test.ts`: an unknown type draws its fields and "Not
  checked on this device"; each field type's input; labels (catalog, then
  author, then name); state word and retry button for own widget or own answer
  with state `failed` (P2, P10); "Send answer"; the answers list. The
  widget-answer hook point: "a node with no engine draws no translate action
  and makes no call", the exact `translation.translate` params, and the
  `unavailable` stub (`-32020`), as in `chat/message_text.test.ts`.
- `widgets/render.test.ts`: poll versus generic choice; the two refused
  texts; `renderWidget` renders normally and without error when `complete`
  is `false` (P11).
- `widgets/composer.test.ts`: 2 to 12 options; the blank and repeated
  option rules; the `widget.send` params.
- `messages.test.ts` and `groups.test.ts`: a widget message draws the
  widget; an answer message draws nothing; a widget missing from the map
  draws `widget-refused`.
- `message_search.test.ts`: a widget hit shows the prompt; an answer hit
  is skipped.
- `catalog.test.ts`: the two `ext-poll-…` ids (step 1 of C4a).
- `hindi-screens.test.ts`: "draws a poll in Hindi".

### 7.6 Playwright and the demo

- `roym-groups.spec.ts`: "a poll in a group counts the owner's vote".
- Story 20 (Q8): "Alice's screen: send Bob a poll", "Bob's screen: vote
  for Sunday", "Alice's screen: the poll shows Bob's vote". Use
  `RUN_TAG` in the question, as the story does for its messages
  ([20-roym-chat.spec.ts:12](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts#L12)).

### 7.7 Not tested

- A scan that reaches 1,000 messages with a real two-node history: the
  parity harness checks `complete: false` with 1,001 inbound messages
  instead.
- A peer that sends a widget body between 8 KiB and the 256 KiB message
  limit through a modified client is covered by scenario 264; a modified
  client that skips the shape check is the same case.

### 7.8 Size and time risks

- **Files near the cap:** `messages.rs` (537 lines) gains `send_body` and
  loses the same lines from `send`. `extension/tests.rs` is 414 lines;
  the new manifest tests may go to a new `extension/widget/tests.rs`.
  `extension/load.rs` (182) and `ids.rs` (70) stay small. TypeScript files
  have no line gate, but `messages.ts` is 1,174 lines, so every new Hub
  part goes into `widgets/`.
- **Time of `widget.list`:** with `widget_ids` from today's Hub,
  `get_message` fetches the definitions, and history pages forward from the
  oldest asked widget. For 200 messages, it reads at most one or two history
  pages of 200. The cap case (1,000 messages, five pages) bounds scan work.
  Listing without `widget_ids` is capped at `MAX_UNFILTERED_WIDGETS = 50`
  widgets to avoid returning up to 8 MB (P8). C4a step 3 records both on the
  WASM build (as C3c1 recorded `transaction.drive`). Stop rule: the Hub case
  over 200 ms, stop and tell the owner.
- **The Hub calls `widget.list` on a thread redraw only when the page
  holds a widget.** It is one call next to the two calls of today
  (`transaction.thread`, `conversation.history`).
- **Not chosen: a stored count.** Keeping each person's newest answer in
  a collection, with a marker of the last message read, would make every
  poll's count complete however old it is. It needs a collection, a
  catch-up across calls, a reset after restore, and a rule that deleting
  a vote message does not take back the vote (a deletion does not change
  `visible_seq`, `store/message.rs:402` to `:422`). Too much for this
  slice; the backlog row records it.

### 7.9 Component size

Report all seven components, before and after, from one `target/`
folder, as task.md section 1 asks. Expected: `conversation` grows (Q9);
`transaction`, `catalog` and `scheduling` grow by a few KB (the third
manifest string and the `widgets` key in their `registry()`); `web`,
`profile` and `directory` stay the same within a few hundred bytes.

### 7.10 Hindi text

New messages, for the owner to review before the merge of each PR:

- C4a: `ext-poll-widget-single` (English "Poll"), `ext-poll-widget-single-option`
  ("Your choice").
- C4b: `widget-poll-button` ("Poll"), `widget-poll-question` ("Question"),
  `widget-poll-option` ("Option { $n }"), `widget-poll-add-option`,
  `widget-poll-remove-option`, `widget-poll-send` ("Send poll"),
  `widget-poll-needs-two`, `widget-poll-options-differ`,
  `widget-poll-count` (Fluent plural: "{ $count } vote" / "{ $count }
  votes"), `widget-poll-your-choice` ("You chose: { $choice }"),
  `widget-votes-visible` ("Everyone in this chat can see each vote."),
  `widget-answer-send` ("Send answer"), `widget-answers-heading`
  ("Answers"), `widget-unknown-heading` ("A widget this app does not
  know: { $type }"), `widget-refused` ("This widget cannot be shown: {
  $reason }"), `widget-too-large` ("This widget is too large for this
  app."), `widget-send-failed`, `widget-answer-failed`.

Hub words do not start with `ext-`, so the catalog test of the manifest
ids does not see them.

### 7.11 How C4 avoids the kinds of findings in earlier reviews

- **Facts in the plan that are wrong** (C1 P1, C2 P3, C3c P8): every line
  was checked on `1920cee3`, the same tree as `main` at `aa25961b`;
  section 2.2 names the facts the design rests on.
- **Missed callers** (C1 P7, C3b P2, R1): point 10 lists the count
  asserts and the Hub list test; D-C4-6 moves the one send path instead of
  copying it.
- **Silent failures** (C2 R21, R22): a refused widget carries its code to
  the Hub; an invalid answer is counted in `ignored_answers`, not dropped
  without a trace.
- **Errors classed by text** (C1 R3, C3c R24): every refusal has a
  `code()`, and the Hub branches on `data.code`.
- **Hub state that does not match the service** (C1 R38, C2 R2): the Hub
  never counts votes itself; it draws `widget.list`.
- **Tests that prove less than their names** (C3a R3, R4; C3c R2, R3):
  the limit test checks both sides of 8,192 bytes; the "newest answer"
  test changes a vote and checks both counts; the hook-point tests check
  the call params.
- **Living docs left false** (C1 R6, C3c R23): section 9 names the lines,
  including `AGENTS.md`.
- **Ratchets and planning ids** (C1 R16, C3c R27, R34, R36): no
  suppression, no planning id in test names, and the import clean-up pass
  on every new file.
- **Stories that time out in CI** (C3c R35): the demo steps wait with
  `DELIVERY_WAIT_MS`, as story 20's other steps do.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C4 action |
| --- | --- |
| "A poll vote is signed by the person, so it is provable outside the chat" (`:317`) | Resolved by Q2 (b): move to "Recently resolved" (C4a). |
| "A widget definition larger than 8 KB is refused" (`:550`) | Unchanged. C4a builds the refusal; add the test names. |
| "Widget labels do not come from the catalog yet" (`:578`) | Resolved (C4b): move to "Recently resolved". |
| "Translation hook points for widget answers, opinions, …" (`:580`) | The widget answer part is built (C4b). The row keeps M08D and M08G. |
| "`RECORD_TYPES` is not read by any verifier" (`:665`) | Unchanged. C4 adds no record type. |
| "More demo stories" (`:705`) | C4b extends story 20. |
| New: "Poll options and widget labels are not offered to machine translation" | C4b adds it (D-C4-11). Target: TBD. |
| New: "`widget.list` scans at most 1,000 messages forward from the oldest asked widget, misses an answer whose sender's clock puts it before its widget, and caps unfiltered listings at 50 widgets" | C4a adds it (D-C4-7). The Hub draws the counts it has and shows no sign when `complete` is false, so a count in a long chat can be too low. The fix is a stored count (section 7.8). Target: TBD. |
| New: "The Hub thread shows only the oldest 200 messages" | C4b adds it (P1). The fix is thread paging. Target: TBD. |
| New: "A voter's local clock stepping back can hide a change of vote" | C4a adds it (P9, Q4). Target: TBD. |
| New: "A person cannot take back a vote" | C4b adds it (Q4). Target: TBD. |
| New: "A poll does not show who voted for what" | C4b adds it (Q7). Target: TBD. |
| New: "Only the poll can be made in the Hub" | C4b adds it (Q5): other widget types come with the extensions that need them. Target: TBD. |

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass.

- `AGENTS.md`, "Architecture": the `conversation` sentence gains
  `widget.*` (C4a).
- `docs/system-architecture.md`: `:1141` and `:3130` (the card types):
  add that widgets and answers are chat messages of their own content
  types, not deal cards (C4a); the Roym conversation part.
- `docs/developer-guide.md`: the `conversation` method table
  (`widget.send`, `.answer`, `.list`, near `:2564`); the two content types
  and their bodies; the reserved content types of `conversation.send`; the
  manifest kind `chat` and `widgets`; the derived widget ids (C4a).
- `docs/roym-integrated-experience-spec.md`: the chat part, a poll in any
  chat; `:745` (ten card types) stays true and gains the widget note (C4b).
- `docs/system-requirements-spec.md`: `:1409` ("interactive forms,
  booking widgets") — say what is built (C4b).
- `docs/TERMINOLOGY.md`: "Widget", "Widget answer"; the "Roym extension"
  and "Extension manifest" rows (`:88`, `:89`) gain the `chat` kind (C4a).
- `docs/demo/demo.md`: story 20's new steps (C4b).
- `docs/planning/traceability-matrix.md`: the rows of section 3.
- Change docs (not living docs, but owned here):
  - roym-extensions `## Deviations`: Q2 (b) (a widget and an answer are
    chat messages, not signed records or cards; EX-12's "signed" is the
    message signature; the card table stays at ten and two widget content
    types are added, against `:148`); Q3 (groups); Q4 (newest valid answer
    counts); Q5 (only the poll ships); Q6 (the `chat` kind and `poll.json`;
    `[[widgets]]` is in a `chat` manifest, not in `scheduling`'s);
    `WidgetFieldRule` with `min_choices` and `max_choices`; no `money` in a
    widget; the author's texts are content; the test column of EX-10 to
    EX-13; the "Tasks" checkbox for widgets.
  - roym-multilingual `## Deviations` (`:615`, `:616`) and the test
    columns of ML-26 and ML-28: the widget-answer hook point and the
    widget labels are built; the author's texts are not catalog labels and
    are not offered to machine translation (backlog row).
  - [task.md](task.md): the C4 result lines (and the card count line of
    C4), the split, section 1's size tables.
- ADR-0030 is not edited (ADRs are permanent). Its open line is answered
  by the extensions deviation and the backlog row.

## 10. Exit criteria

C4a:

- [x] The owner answered section 2.1 (2026-11-25).
- [ ] Every C4a row of section 3 names a test that exists and passes.
- [ ] `conversation` measured after step 3 and within the Q9 limit.
- [ ] One `widget.list` with the Hub's `widget_ids` on the WASM build under 200
  ms, recorded.
- [ ] `roym_widget_e2e` passes (both tests).
- [ ] Parity 260 to 265 (including forward scan, missing/deleted id handling,
  and 50-widget unfiltered cap) and the method contract (count 27, scenario 73)
  pass after `mise run build:roym`.
- [ ] `conversation.send` refuses both widget content types, and every
  other content type behaves as before (scenarios 53 to 61 unchanged).
- [ ] No file of `roym_transaction` changed.
- [ ] Sizes recorded; Deviations, backlog and living docs of C4a;
  `mise run verify` with no `--skip`.

C4b:

- [ ] Every C4b row of section 3 names a test that exists and passes.
- [ ] The three hook-point cases for the widget answer pass.
- [ ] `roym-groups.spec.ts`'s poll case and story 20 pass (`mise run
  demo`).
- [ ] The owner reviewed the Hindi messages.
- [ ] Living docs, multilingual deviations, demo.md, status.md;
  `mise run verify` with no `--skip`.

## 11. What C4 does not do

- Fetch a definition larger than 8 KiB by hash (backlog row `:550`).
- Person-signed widgets or answers, and widgets inside a deal (a slot
  picker that books).
- Any widget type other than the poll in the Hub (Q5).
- Names of voters, anonymous polls, polls with several choices, a closing
  time, or taking back a vote.
- A forward mode on widget messages (C7).
- Any change to `transaction`, the deal cards, `CARD_TYPES` or
  `RECORD_TYPES`.
- Any WIT change or wire-reachable method.

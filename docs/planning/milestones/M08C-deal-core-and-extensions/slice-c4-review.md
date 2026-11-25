# M08C slice C4 — review

*Reader: the developer who fixes the C4 plan or code, and the project owner.
This file holds every review finding for slice C4 (parts C4a and C4b). Plan
review findings are P1, P2, …; code review findings are R1, R2, …. Ids are
never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-25

Checked [slice-c4-implementation-plan.md](slice-c4-implementation-plan.md) on
branch `feat/m08c-c4-widgets` at `a7ab43a9` (on top of `main` at
`aa25961b`). This is the main checkout, not a worktree (`git rev-parse
--git-dir` is `.git`). `git pull` had nothing to pull: the local branch had no
upstream set (the branch was already on the remote at `a7ab43a9`); the push
of this round set it. The plan was written by
Claude Opus 5.5. This review is by Claude Sonnet 5.5.

Read: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); the plan (all 1,519 lines);
task.md and status.md of M08C; the one change doc in CHANGE_DOCS
(roym-extensions: terms, EX-10 to EX-13, "Step records", "Widgets",
Deviations). task.md's C4 section names two change docs: roym-extensions
and **roym-multilingual** (ML-26, ML-28). roym-multilingual is not in
CHANGE_DOCS for this round. The plan reads it itself (section 0) and names
it for ML-26 and ML-28 in section 3, so I checked the plan against task.md
and the plan's own quotes of it, not against that doc's full text. There is
no earlier round, so no earlier item needed re-checking. I also read the
review files of C3c (round headings and the kinds of findings) for what
earlier plan reviews looked for.

**Summary: 9 findings: 1 blocker, 2 major, 6 minor. 0 open, 9 fixed.**

### What was checked and found correct

- **The decision itself (Q2 (b)).** A widget and an answer are chat
  messages with their own content types. `transaction` skips them:
  `classify_sync_message` returns `Skip` for any content type that is not
  the card type ([sync.rs:263](../../../../crates/roym_transaction/src/app/sync.rs#L263)).
  `CARD_TYPES` and `RECORD_TYPES` do not change. `conversation.send` takes
  any other content type today ([messages.rs:286](../../../../crates/roym_conversation/src/app/messages.rs#L286)
  to `:298`), so the two new types need the "reserved" rule that the plan
  adds.
- **Body size.** The store limit is 262,144 bytes for a body
  ([store.rs:92](../../../../crates/conversation/src/store.rs#L92), checked
  at send in `host_impl.rs:99` and on receive in `transport.rs:425`). An 8
  KiB widget body is far below it, and the plan's own 8 KiB check is what
  limits widgets.
- **Clock handling.** An inbound message that is too far in the future is
  refused ([transport.rs:371](../../../../crates/conversation/src/transport.rs#L371),
  [group/entry.rs:426](../../../../crates/conversation/src/group/entry.rs#L426)).
  So a peer cannot put a vote far in the future. (A peer can still put a
  vote in the past; that only hurts the peer's own later votes.)
- **`get_message` only returns accepted or own messages**
  ([host_impl.rs:313](../../../../crates/conversation/src/host_impl.rs#L313)),
  so `widget.answer` cannot answer a widget that history hides.
- **Deleting a message** keeps its row, empties the body and sets
  `deleted_at` ([store/message.rs:402](../../../../crates/conversation/src/store/message.rs#L402)
  to `:422`), so the content type stays and the plan's rule "a deleted
  answer is not counted" needs `deleted_at`, as the plan says.
- **Field checks.** `schema::check_fields` refuses unknown keys, a missing
  required key and a choice not in the list
  ([schema.rs:108](../../../../crates/roym_core/src/extension/schema.rs#L108)
  and following). `check_definition` is `pub(super)` today, so the plan's
  change to `pub(crate)` is needed for a module at the crate root. The text
  field rule "max_len is required" in `check_len_limit` applies to every
  widget text field; the plan's field examples already follow it.
- **Backlog rows.** The four rows the plan cites exist at the cited lines
  (`:317`, `:550`, `:578`, `:580` of deferred-backlog.md).
- **Router and API tables.** `conversation.` and `group.` are the existing
  prefixes of `conversation` ([router.rs:25](../../../../crates/roym_core/src/router.rs#L25));
  a `widget.` row is new and does not clash. `api.rs` has the method rows
  the plan edits.
- **"How this is shown to an end user"** is present, right after the goal,
  with a C4a part and a C4b part, and the developer lines name real
  commands. See P3 for one bullet that is not true as written.
- **Exit criteria, backlog rows, living-docs list and the change-doc
  deviations** are complete for what the plan builds. The plan names the
  right change doc for each criterion: EX-10 to EX-13 in roym-extensions,
  ML-26 and ML-28 in roym-multilingual.
- **Size rules.** `messages.rs` is 537 lines and `load.rs` is 182, so the
  planned additions fit; new code goes to new files.

### Views on the owner questions

The owner accepted the recommended answer of Q1 to Q9 on 2026-11-25. My view
of each, for the record:

- **Q1 (two PRs).** Agree. The node part and the Hub part have different
  risks. Note that P1 changes the node design before C4a starts.
- **Q2 (chat messages, not signed records).** Agree. A vote that anyone can
  prove for ever, chosen by nobody, is a worse default. The cost is stated
  and the backlog keeps the signed form for a widget that must be evidence.
- **Q3 (groups).** Agree. It needs no new rule once Q2 is (b).
- **Q4 (the newest answer counts).** Agree. See P9: "newest" depends on the
  voter's own clock.
- **Q5 (only the poll).** Agree. P3 says the unknown-widget path is
  narrower than the plan says.
- **Q6 (manifest kind `chat`).** Agree. It keeps `poll` out of
  `listing.set` and the deal checks, and the small reader keeps
  `conversation` small.
- **Q7 (counts, not names).** Agree. The line "Everyone in this chat can
  see each vote." is the honest one.
- **Q8 (three steps in story 20).** Agree.
- **Q9 (+15% for `conversation`).** Agree. The plan's own stop rule (+40 KB
  look at what linked more) is a good guard.

### Findings

#### [x] P1 — `history` pages oldest first; the whole `widget.list` scan, its early stop and its cost claims are inverted (blocker)

*Where:* D-C4-7 (the pseudo-code, the rules "The scan stops early" and "The
order is not taken from `visible_seq`"), point 12 of section 2.2, section
7.8 ("Time of `widget.list`"), parity scenario 265, the exit criterion
"under 200 ms", and the Hub part (`fillThread` passes "the ids of the
widgets in the loaded history").

*Problem:* The plan says history is "newest first" and that the Hub loads
"the newest 200". It is the other way round.

- The host interface says history pages "in chronological order"
  ([conversation.wit:193](../../../../crates/wit_interfaces/wit/conversation/conversation.wit#L193)).
- The store query is `ORDER BY sender_timestamp ASC, author ASC, id ASC`
  with the cursor as an exclusive lower bound `(…) > (cursor)` and no
  cursor meaning "from the beginning"
  ([store/history.rs:48](../../../../crates/conversation/src/store/history.rs#L48)
  to `:66`). `next_cursor` points forward.
- `conversation.history` passes no reversal
  ([messages.rs:375](../../../../crates/roym_conversation/src/app/messages.rs#L375)).
- The Hub calls it with no `limit` and no `cursor`
  ([messages.ts:386](../../../../crates/roym_web/ui/src/screens/messages.ts#L386)),
  so it gets the **oldest** 200 messages (the default limit is 200). There
  is no paging in the Hub.

Every rule built on "newest first" is therefore wrong:

- "The scan reads back from the newest message … 1,000 messages" reads the
  oldest 1,000. In a chat with more than 1,000 messages, a recent poll is
  never found and its answers are never counted, and `complete` is false.
- "Once every asked widget is found, the pages before this one hold nothing
  more" stops at the page of the *newest* asked widget. Its answers come
  after it, so on a chat of more than one page the scan stops before the
  answers and every count is 0.
- Parity scenario 265 would test the wrong direction ("the scan stops at the
  page of the oldest asked widget").
- The Hub only shows the first 200 messages of a chat. A poll sent after
  message 200 is not drawn, and the user story ("Bob taps Sunday … Alice's
  chat shows 1 vote") does not hold in any chat with more than 200
  messages. This limit exists today for text messages, but the plan assumes
  the opposite.

Small tests hide all of this, because a chat of a few messages fits in one
page whatever the order.

*Failure case:* Alice and Bob chat for a month (300 messages) and Alice
sends a poll. The Hub does not draw it, because the poll is message 301. A
`roymctl roym widget list` reads the oldest 1,000, finds the poll, and finds
no answer if the answer is on a later page and the early stop fired.

*Fix:* Redesign the scan for forward order, and state the facts correctly in
points 4 and 12 and in section 7.8.

1. Fetch each asked widget with `AppConversation::get_message` (it is
   already used by `widget.answer`, and it applies the same accepted-or-own
   filter). Check that it is in this conversation and is a widget.
2. Take the oldest asked widget by `(sender_timestamp, author, id)`. Page
   forward with `history(conversation, 200, cursor = <that id>)`. The cursor
   is exclusive, so the widget itself is not repeated. Every answer sorts
   after its widget, except the clock case in the next point. Collect the
   answers of all asked widgets until the end of the chat or the cap.
3. Keep `complete: false` when the cap stops the scan, and set the cap in
   messages after the oldest asked widget, not from the start of the chat.
   Say how an answer that sorts *before* its widget (its sender's clock is
   behind) is handled: it is not counted, and the backlog row says so.
4. With no `widget_ids` (`roymctl`), scan forward from the start and stop at
   the cap, and say that old chats show the oldest widgets only; or let
   `roymctl widget list` take `--from <message id>`. Choose one.
5. Fix the Hub part: either the Hub pages the thread (its own change, a
   backlog row), or the plan states that C4b draws a poll only when it is
   within the first 200 messages, adds a backlog row "The Hub thread shows
   only the oldest 200 messages", and the end-user bullets say "in a chat
   that the Hub shows". The owner decides; the plan must not claim the
   newest 200.
6. Rewrite parity scenario 265 (a forward scan, a poll after message 201,
   the cap case with 1,001 messages after the oldest asked widget) and the
   200 ms measurement to match.

*Status:* Fixed (c4199a05). Agreed. Redesigned `widget.list` for forward paging from the oldest asked widget using `get_message`; corrected point 12 to oldest 200; scenario 265, section 7.8, and backlog updated.

#### [x] P2 — A vote that fails to send is invisible, and the voter's own count still includes it (major)

*Where:* D-C4-7 (`AnswerView`), D-C4-10 (`messageElement`: "an answer
message is not drawn"), Q4.

*Problem:* A message the person sends has a state word and, when it fails, a
retry button (`messageStateWord`, `canRetry`,
[messages.ts:506](../../../../crates/roym_web/ui/src/screens/messages.ts#L506)
and `:599` to `:609`, calling `conversation.retry`). An answer is a message,
but the plan draws no bubble for it, so it has no state and no retry.
`AnswerView` has no `state` field. `history` returns own messages in any
state ([store/history.rs:63](../../../../crates/conversation/src/store/history.rs#L63):
`outgoing = 1 OR admission = 'accepted'`), so a `pending` or `failed` own
answer is counted on the voter's node at once.

*Failure case:* Bob taps "Sunday" while the transport fails for this message
(state `failed`). Bob sees "You chose: Sunday" and "1 vote" next to Sunday.
Alice sees 0 votes. Nothing on Bob's screen says the vote was not sent, and
there is no retry button. Bob believes he voted.

*Fix:* Add `state` (and `last_error` when `failed`) to `AnswerView`. In the
poll card, show the state word of the person's own newest answer when it is
not `delivered` (the existing `messages-state-*` messages, `canRetry` and
`conversation.retry` with `AnswerView.message_id`). Add a Hub test for a
`failed` answer (word shown, retry calls `conversation.retry`) and a node
test that `widget.list` returns the state. Decide in the plan whether a
`failed` own answer is counted (recommended: yes, as the chat counts it, with
the state shown) and write it down.

*Status:* Fixed (c4199a05). Agreed. `AnswerView` gains `state: StoredState` and `last_error: Option<String>`; own answer state word and retry button added to poll and generic cards; parity scenario 261 and unit tests updated.

#### [x] P3 — A widget from a newer build with a new field type or new key is refused, not drawn by the generic card (major)

*Where:* Section 1 and the last "How this is shown" bullet ("If a newer app
sends a widget this app does not know, the chat shows its fields … and the
person can still answer it"), D-C4-2 (`#[serde(deny_unknown_fields)]` on
`WidgetDefinition`, `DefinedField` and `DefinedChoice`; `field_type:
FieldType`), tests `an_unknown_type_is_not_checked_but_its_shape_is`,
scenario 264.

*Problem:* The plan makes an unknown **type name** drawable. The likely
change in a newer build is a new **field type** (`location`, `file-pointer`
and `record-pointer` are already in the backlog as not built) or a new key
on a field. Both fail to parse here: `FieldType` has eight variants, and
every struct denies unknown keys. The result is `widget-json` and the card
shows "This widget cannot be shown". The generic card, the "Not checked on
this device" line and the answer hook point are then reachable only for a
new type name built from today's eight field types, which no shipped build
sends (Q5: only `poll/single`). So the headline story for the unknown path
is untestable except through a fixture the plan writes itself.

*Failure case:* Build N+1 ships a `location-share` widget with a `location`
field. A person on build N sees "This widget cannot be shown: widget-json".

*Fix:* Pick one and write it in the plan.

- (a) Keep the strict parse and say so: a widget with a field type or key
  this build does not know is refused (`widget-shape`, not `widget-json`).
  Change the user bullet and section 1 to say "a widget of a type this app
  does not know but built from the field types it does know". Add a test
  that a new field type is refused with that code. Recommended: it is the
  smallest change and the version number (`widget_version`) is the place to
  grow the format.
- (b) Accept an unknown field type for display (`FieldType::Other(String)`)
  and refuse only to *answer* that field. This is more code in `conversation`
  (size limit) and in the Hub.

*Status:* Fixed (ec5139bb). Reopened in round 2 and fixed in ec5139bb: `parse_widget_body` parses to `Value` first, checks `widget_version`, and maps serde errors to `Shape("widget-shape")`; `widget.send` rejects unknown field types at RPC parse; scenarios 262 and 264 updated.

#### [x] P4 — What `widget.list` does with an asked id that is deleted, not a widget, or from another chat is not defined (minor)

*Where:* D-C4-7 pseudo-code and rules, D-C4-10 (`messageElement`).

*Problem:* The pseudo-code skips a message with `deleted_at` before it
counts the widget as found, so an asked id that is deleted is never found.
With the early stop as written, the scan then never stops early and reads to
the cap on every Hub redraw. An asked id of a message that is not a widget,
or of another conversation, is also never found. The plan says the Hub
draws `widget-refused` "for an id that is not found", but a deleted widget
should draw "message deleted" as today, not "cannot be shown".

*Failure case:* Alice deletes her poll. Every redraw of the chat reads the
whole scan window to look for a widget that cannot be found, and the poll's
bubble says "This widget cannot be shown" instead of "message deleted".

*Fix:* With P1's design (each asked id read with `get_message`) say: a
deleted, missing, non-widget or other-conversation id is left out of
`widgets` and does not start a scan; the Hub does not ask for the id of a
message it draws as deleted; and a missing id, not a deleted one, draws
`widget-refused`. Add the case to scenario 265.

*Status:* Fixed (c4199a05). Agreed. An asked id that is deleted, missing, non-widget, or foreign is omitted from `widgets` and does not start a scan; Hub skips deleted messages and draws `widget-refused` for missing ids; scenario 265 updated.

#### [x] P5 — Two guard tests list every conversation method and are not in the plan (minor)

*Where:* Step 4 (parity and method contract), section 7.3.

*Problem:* The plan says the method contract test "passes with the new
file". It does not pass without a count change. [contract.rs:40](../../../../crates/roym_web/tests/method_contract/contract.rs#L40)
asserts `table.len() == 24` for `conversation`; the three new rows make 27.
[wire_origin.rs:116](../../../../crates/roym_web/tests/dual_build_parity/wire_origin.rs#L116)
(scenario 73, "every catalog and conversation verb is wired and locally
admitted") lists each `conversation` and `group` verb with params and fails
if one is `-32601` or `-32013`. A new verb is missing from that list, and the
plan's claim "`api.invoke` stays local-only" for `widget.*` is exactly what
that test is for.

*Fix:* Add both to step 4: the count 24 to 27, and `widget.send`,
`widget.answer` and `widget.list` in the scenario 73 list (a widget id is
needed for `widget.answer`; create one first, as it does for
`conversation.send`). Run `git grep` for other fixed counts and lists of
`conversation` methods (`group.unhide`, `conversation.delivery-status`) and
write the result in the plan, as the C3b review asked for the service lists.

*Status:* Fixed (c4199a05). Agreed. Step 4, section 7.3, and exit criteria update `contract.rs` table length assertion from 24 to 27, and add `widget.send`, `widget.answer`, `widget.list` to scenario 73 in `wire_origin.rs`.

#### [x] P6 — The `send_body` extraction does not say where the reserved-type check goes (minor)

*Where:* D-C4-6, "Shared send path", and the behaviour audit line "`conversation.send` refuses the two widget content types".

*Problem:* The plan says to move "the body of `conversation.send` after the
content-type check ([messages.rs:273 to :318])". Lines 273 to 285 are the
group checks (`group_info`, the restored and removed notices) and lines 286
to 298 are the content-type check, which needs `is_group` from the group
checks. So the content-type check is inside the range the plan moves, and
it comes **after** the notices today. If the new reserved check for the two
widget types is written before `group_info` (the simple place), a removed
member who sends one now gets "this content type is reserved" instead of
`GROUP_REMOVED_NOTICE`. If `send_body` calls `group_info` and `send` also
calls it, there are two host calls per send.

*Fix:* Say it: `send_body` does the group checks once, then calls a
`reserved: impl Fn(bool /* is_group */, &str) -> Option<&'static str>`
check that the caller gives (`conversation.send` passes today's list plus the
two widget types; the widget methods pass none). The order of errors stays
as today, and scenarios 207 and 208 do not change.

*Status:* Fixed (c4199a05). Agreed. `send_body` takes a `check_reserved` closure invoked after group checks, preserving error precedence for removed members (scenarios 207 and 208 unchanged).

#### [x] P7 — The Hub poll card hard-codes the field name and the heading (minor)

*Where:* D-C4-10 (`poll.ts`, `render.ts`).

*Problem:* `renderWidget` picks the poll card when `semantic` is `poll` and
the definition has one required choice field. `renderPoll` then sends
`{ option: code }` and draws the heading `ext-poll-widget-single`. A
definition from a peer (an unknown type, `checked: false`) that has semantic
`poll` and a field named `vote` gets the poll card, which sends `{ option:
… }`, which the node refuses (`answer-invalid`, unknown field `option`).
The card also says "Poll" and does not say "Not checked on this device".

*Fix:* Use the definition's own field name in the answer. Draw
`ext-not-checked` on any card whose `checked` is false, and the heading from
the catalog only when the type is known (the generic card's rule). Add a
`poll.test.ts` case.

*Status:* Fixed (c4199a05). Agreed. `renderPoll` dynamically uses the definition's choice field name in the answer payload, draws `ext-not-checked` when `checked` is false, and uses catalog heading only for known types; `poll.test.ts` updated.

#### [x] P8 — `widget.list` with no ids can return about 8 MB, and the output-size claim is wrong (minor)

*Where:* D-C4-7, rule "Size of the output".

*Problem:* The plan says every scanned message appears once at most, so the
output is no larger than the scanned bodies, and compares this with
`conversation.history`. For answers the output is smaller (only the newest
answer of each author is returned). For widgets it is not: each `WidgetView`
carries the whole definition. With no `widget_ids`, up to 1,000 widgets of 8
KiB come back in one reply (about 8 MB), plus `counts` rows (up to 16
fields × 32 choices each). `conversation.history` is capped by the 200-row
page, which `widget.list` is not.

*Fix:* Cap the widgets returned when `widget_ids` is empty (for example the
50 oldest after `--from`, with `complete: false`), and replace the size
sentence with the real bound. Add the bound to scenario 265.

*Status:* Fixed (c4199a05). Agreed. Capped unfiltered widget listings at `MAX_UNFILTERED_WIDGETS = 50` widgets (~400 KiB) with `complete: false` when more exist; updated size rules, scenario 265, and backlog.

#### [x] P9 — "Newest answer" uses the voter's own clock, and a clock that steps back makes a vote do nothing (minor)

*Where:* Q4 and D-C4-7 ("per author: max by (sender_timestamp_ms,
message_id)").

*Problem:* The plan notes that one person's answers are stamped by one node,
so another node's clock does not matter. A person's own clock can still
step back (a time sync, a changed setting). Then the new vote carries an
older stamp than the earlier one, and the earlier vote still counts on every
node. The Hub tells the person "You chose: Saturday", and nobody sees the
count move.

*Fix:* Record it as a known limit in the backlog row of D-C4-7 (it is rare
and the fix needs a counter in the message, not the clock), and state in Q4
that a clock step back can hide a change of vote. No code change needed in
C4.

*Status:* Fixed (c4199a05). Agreed. Recorded as a known limit in Section 8 deferred backlog and D-C4-7; Q4 updated to state that local clock step-backs preserve earlier votes.

## Plan review round 2 — Claude Sonnet 5.5, 2026-11-25

Closure check of round 1, on branch `feat/m08c-c4-widgets` at `7fa0af92` (the
fix is `c4199a05`, the close-out is `7fa0af92`). I read the plan diff
between `7d508103` and `7fa0af92` and the new D-C4-5, D-C4-7 and D-C4-10
text, and checked the stale wording by search ("newest 200", "newest 1,000",
"newest first", "read back": no match is left). I did not re-read the whole
plan, only what the fixes touched.

**Summary: 8 of 9 round 1 items confirmed closed. 1 reopened (P3). 2 new
items (P10 major, P11 minor). All 3 addressed in ec5139bb: 0 open, 3 fixed.**

### Round 1 items

- **P1 — Confirmed closed.** `widget.list` now fetches each asked widget
  with `get_message`, takes the oldest by `(sender_timestamp, author, id)`
  and pages forward with that id as the exclusive cursor, up to 1,000
  messages, with `complete: false` at the cap. The unfiltered path scans
  from the start. Points 4 and 12, section 7.8, scenario 265, the backlog
  rows and the first user bullet ("within the first 200 messages") agree.
  One gap is left as P11.
- **P2 — Confirmed closed for answers.** `AnswerView` has `state` and
  `last_error`; the poll card and the generic card show the state word and a
  retry button (`conversation.retry` with `message_id`) for the person's own
  newest answer; tests are listed. The same fix lost something: see P10.
- **P3 — Reopened.** The user text, section 1 and the strict-parse choice
  (a) are right. The error code is not: see below.
- **P4 — Confirmed closed.** Deleted, missing, other-chat and non-widget ids
  are dropped before any scan; the Hub does not send ids of deleted messages
  and draws `widget-refused` for a missing one; scenario 265 covers it.
- **P5 — Confirmed closed.** Count 24 to 27 and the scenario 73 list are in
  step 4 and section 7.3. One wording nit for the fixer: section 7.3 says
  scenario 73 asserts the verbs are "refused on the wire". Scenario 73 only
  checks that each verb is wired and locally admitted (not `-32601`, not
  `-32013`); say that, or add the wire check where it exists.
- **P6 — Confirmed closed.** `send_body` does the group checks once, then
  calls `check_reserved(is_group, content_type)`; the error order for a
  removed member stays.
- **P7 — Confirmed closed.** The answer key comes from the definition, the
  heading from the catalog only for a known type, and `ext-not-checked` is
  drawn when `checked` is false.
- **P8 — Confirmed closed.** `MAX_UNFILTERED_WIDGETS = 50` (about 400 KiB),
  at most 200 widgets with ids, `complete: false` when more exist.
- **P9 — Confirmed closed.** The limit is in Q4, D-C4-7 and a backlog row.

#### [x] P3 check, round 2 — the code `widget-shape` is not reachable (major)

*Where:* D-C4-5 (`parse_widget_body`, `check_shape` item 5), scenario 262
(`widget.send` with an unknown field type gives `widget-shape`), scenario
264, section 1.

*Problem:* The fix puts the rule under `check_shape` ("a definition from a
peer with an unknown field type … is refused as `Shape("widget-shape")`").
But `check_shape` takes an already parsed `WidgetDefinition`, and a value
with an unknown `FieldType` variant or an unknown key cannot be built. The
failure happens earlier, in serde, inside `parse_widget_body`, whose own
comment says "Size first, then JSON, version and shape". A serde failure is
`Json(String)`, code `widget-json`. For `widget.send` it happens even
earlier, when the method input `WidgetSendInput` is parsed, and the answer is
the generic invalid-params error with no `data.code`. So scenario 262 as
written cannot pass, and the Hub shows `widget-json` for a widget from a
newer build.

The same order problem hides the version check: a body with
`widget_version: 2` and a new key fails `deny_unknown_fields` as `Json`
before `UnknownVersion` is reached, so `widget-version` is not reachable
either for the case it is meant for.

*Failure case:* Build N+1 sends a widget with a `location` field. Build N
shows "This widget cannot be shown: widget-json", and the test that expects
`widget-shape` fails (or is written to pass on the wrong code).

*Fix:* In `parse_widget_body`, parse to `serde_json::Value` first. Read
`widget_version` and answer `UnknownVersion` (`widget-version`). Then
deserialize the definition and map any serde error to `Shape` (`widget-shape`).
Say that `widget.send` of an unknown field type is refused as invalid
params from the typed input (no `data.code`), and change scenario 262 to
use a definition that parses but breaks a `check_shape` rule instead, and
scenario 264 to carry the unknown field type as an inbound body. Add a test
that a version 2 body with a new key gives `widget-version`.

*Status:* Fixed (ec5139bb). Agreed. `parse_widget_body` parses to `Value` first, checks `widget_version`, and maps serde errors to `Shape("widget-shape")`; `widget.send` rejects unknown field types at RPC parse without `data.code`; scenarios 262 and 264 updated.

#### [x] P10 — The widget's own message has no state word or retry any more (major)

*Where:* D-C4-10, the `messages.ts` wiring; `WidgetView` in D-C4-7.

*Problem:* Before the fix, the wiring text ended: "an answer message is not
drawn … The state word and the retry action stay." That sentence was about
the widget's own bubble, and the fix removed it. `WidgetView` has no `state`
either (only `AnswerView` got one). A person's own poll is a message that can
`fail` like any other, and `renderWidget` replaces the bubble. This is the
same problem as P2, for the author of the poll.

*Failure case:* Alice sends a poll. The send fails. Her screen shows the
poll with "0 votes" and no failure word and no retry. Bob never gets it.

*Fix:* Put the sentence back, or better, add `state` and `last_error` to
`WidgetView` and have `renderWidget` show the state word (not `delivered`)
and the retry button for an own widget. Add a Hub test for a `failed` own
poll (word shown, retry calls `conversation.retry` with the widget id), next
to the P2 tests.

*Status:* Fixed (ec5139bb). Agreed. `WidgetView` gains `state: StoredState` and `last_error: Option<String>`; `renderWidget` shows state word and retry button calling `conversation.retry` for own widgets; Hub unit tests added.

#### [x] P11 — What the Hub shows when `widget.list` says `complete: false` is not defined (minor)

*Where:* D-C4-7 (`complete`), D-C4-10 (`messages.ts` wiring).

*Problem:* After the P1 fix the count is read from at most 1,000 messages
after the oldest asked widget. The Hub only shows the oldest 200 messages, so
in a long chat the poll is drawn while its answers may be later than the
scan. `complete: false` is returned, but no Hub text uses it. The counts
are then too low with no sign.

*Fix:* Say what the Hub does: for example show a line "Votes after this
point may be missing" (a new Hindi message) when `complete` is false for a
drawn poll, or state that the Hub ignores it and add the limit to the
backlog row text. Add a Hub test for the chosen behaviour.

*Status:* Fixed (ec5139bb). Agreed. Hub ignores `complete: false` today because threads only load the first 200 messages; limit recorded in deferred backlog row; Hub unit test added.

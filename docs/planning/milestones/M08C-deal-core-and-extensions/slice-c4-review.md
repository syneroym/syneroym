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

*Status:* Fixed (1a4d4e54) after round 3 (backlog row added). Was: Fixed (ec5139bb). Agreed. Hub ignores `complete: false` today because threads only load the first 200 messages; limit recorded in deferred backlog row; Hub unit test added.

## Plan review round 3 — Claude Sonnet 5.5, 2026-11-25

Closure check of the three open items of round 2, on `feat/m08c-c4-widgets`
at `7406924d` (fix `ec5139bb`). I read the plan diff between `8f4a3f6f` and
`7406924d` and the places it touches.

**Summary: P3 and P10 confirmed closed. P11 stays open (minor). Open: 1.**

- **P3 — Confirmed closed.** `parse_widget_body` now parses to
  `serde_json::Value`, checks `widget_version` (`widget-version`), then
  deserializes and maps any serde failure to `Shape` (`widget-shape`). The
  plan says `widget.send` of an unknown field type is refused as invalid
  params at the typed input, with no `data.code`. Scenario 262 now uses a
  definition that breaks a `check_shape` rule. Scenario 264 carries the
  unknown field type (`location`) as an inbound body. The two tests
  `a_body_with_unknown_version_and_extra_keys_returns_widget_version` and
  `a_body_with_unknown_field_type_returns_widget_shape` are listed.
- **P10 — Confirmed closed.** `WidgetView` has `state` and `last_error`
  (D-C4-7, next to `ignored_answers`), copied from the message row. D-C4-10
  says an own widget shows its state word when not `delivered` and a retry
  button calling `conversation.retry` with the widget id. The Hub tests
  (`poll.test.ts`, `generic.test.ts`) name a `failed` own poll.
- **P11 — Open.** The plan now defines the behaviour: the Hub ignores
  `complete: false` and draws the counts it has, and `render.test.ts` checks
  that a `false` value renders without error. That is a valid choice. But the
  text says this is "recorded in the backlog row", and it is not. The two
  new rows in section 8 say the scan stops at 1,000 messages and that the Hub
  shows the oldest 200 messages. Neither says that a poll can show counts that
  are too low, with no sign in the Hub. The review fix asked for this to be
  written down.

#### P11 check, round 3

*Fix:* Add to the section 8 row "`widget.list` scans at most 1,000
messages …" a sentence: "The Hub draws the counts it has and shows no sign
when `complete` is false, so a count in a long chat can be too low." Then
the statement in D-C4-10 is true.

*Status:* Fixed (1a4d4e54). The section 8 row now says the Hub draws the counts it has and shows no sign when `complete` is false. Checked in the plan.

## Code review round 1 — Claude Opus 5.5, 2026-11-25

This is the first code review of **part C4a** (widgets on the node): plan
section 6.1 and D-C4-1 to D-C4-9, D-C4-12 and D-C4-13. PR #383 (not a
draft), branch `feat/m08c-c4-widgets` at `92863fd4`, main checkout (not a
worktree), `git pull` up to date. Diff: `git diff origin/main...HEAD`
(45 files). The code was written by Gemini 3.8 Flash. This file has only
plan review rounds before this one, so there was no earlier code finding to
check. New findings start at R1.

**CI.** `gh pr checks 383` on `92863fd4`: **"Lints, licenses & checks"
failed** (R1). fmt, clippy with `-D warnings`, `cargo audit`, `cargo deny`,
`check-roym-deps`, `check-file-lengths`, `check-lint-suppressions`,
`check-module-layout`, `check-change-docs`, `check-dep-versions` and
`check-duplication` passed in that job before the failing step
(`mise run test:roym-ui`). The six test shards and Playwright E2E passed.
So CI is proof for the Rust suite and Playwright, not for the Hub unit
tests.

**Tests I ran** (targeted only): `cargo nextest run -p syneroym-roym-core
widget::tests`: 16 passed (status.md says 14, R15). `cargo nextest run -p
roymctl widget`: 4 passed (status.md says 4). I did not build the roym
WASM, so I did not run parity again (CI's test shards ran it), and I did
not measure the component sizes again.

### Checked and found correct

- **Widgets are chat messages (D-C4-1, Q2 (b)).** Two new content types
  in `roym_core::widget`. No change to `CARD_TYPES`, `RECORD_TYPES`, WIT,
  the envelope or any file of `roym_transaction` (the diff has none).
- **The formats (D-C4-2).** `WidgetDefinition`, `DefinedField`,
  `DefinedChoice`, `WidgetBody` and `AnswerBody` match the plan field for
  field, with `deny_unknown_fields` and the same `skip_serializing_if`.
  The constants have the plan's values.
- **`parse_widget_body` (P3).** Size first, then `Value`, then
  `widget_version` (`widget-version`, also with extra keys), then
  `from_value` mapped to `widget-shape`, then `check_shape`. Tests
  `a_body_with_unknown_version_and_extra_keys_returns_widget_version` and
  `a_body_with_unknown_field_type_returns_widget_shape` show both orders.
- **`check_shape`** follows rules 1 to 5 of D-C4-5, and calls
  `schema::check_definition` on a `schema::Field` built from the codes, so
  a repeated choice code is refused there (`check_choices`). One test per
  rule exists.
- **`check_type`** checks semantic, field names in order, type,
  `required`, `max_len`, `min`, `max`, no `label`, and the choice count
  against `min_choices..=max_choices`. `None` gives `NotChecked`.
- **`check_answer`** is `schema::check_fields` over `answer_fields()`, so
  an unknown key, a missing required field and a wrong choice are refused,
  for a known or an unknown type.
- **The manifest kind `chat` (D-C4-3).** `check_chat` refuses every deal
  key and an empty `widgets`; `check_workflow` and `check_insight` refuse
  `widgets`; type prefix, length and uniqueness, semantic, field count,
  unique names, no `money`, the `min_choices`/`max_choices` rules.
  `Registry::widget_type` reads only `chat` manifests; `workflow()` is not
  changed, so `listing.set` and the deal checks do not know `poll`
  (`a_listing_cannot_name_a_chat_extension`,
  `a_commitment_naming_a_chat_extension_is_not_checked`).
  `widget_types()` reads a header only and agrees with the registry
  (`widget_types_agree_with_the_registry`).
- **Message ids (D-C4-4).** `widget_name_id` and `widget_field_label_id`
  in Rust and TypeScript give `ext-poll-widget-single` and
  `ext-poll-widget-single-option`; `manifest_message_sources` adds them
  with their places, so a clash is refused
  (`widget_ids_are_derived_and_a_clash_is_refused`). Both `.ftl` files
  have the two messages.
- **The shared send path (D-C4-6).** I compared `send_body` with the body
  of `conversation.send` on `main`: the group check
  (`GROUP_RESTORED_NOTICE`, `GROUP_REMOVED_NOTICE`) still comes first, then
  the reserved check, then `AppConversation::send` and the same
  `SendOutput`. The reserved list is today's list plus the two widget
  content types, and the card message in a group is unchanged. So
  scenarios 207 and 208 keep their errors.
- **`widget.send` and `widget.answer`** follow the D-C4-6 pseudocode in
  order: languages, shape, type (`extension-unknown` for an unknown type),
  body size; for an answer: `widget-not-found` (missing or other
  conversation), `not-a-widget`, `widget-deleted`, `widget-refused` with
  the inner code in the message, `answer-invalid`, `answer-too-large`.
  Every refusal is `-32602` with `data.code`.
- **`widget.list` (D-C4-7).** The `widget_ids` path fetches each id with
  `get_message`, drops a missing, foreign, non-widget or deleted id, and
  pages forward with `history(…, 200, Some(oldest.id))`; the store's
  cursor is exclusive and ascending by `(sender_timestamp, author, id)`
  (`store/history.rs:49`), so the plan's order holds. The newest valid
  answer per author is chosen after invalid answers are dropped, by
  `(sender_timestamp, id)`; deleted answers are skipped; the views carry
  `state` and `last_error` for the widget and each answer (P2, P10).
- **Access.** One router row `widget.` → `conversation`, `Owner`. The
  three methods use the default `require_internal`, and the contract test
  that drives every table row over the wire (`contract.rs:203`) checks
  `-32013` for them. Scenario 73 drives the three methods on the local
  path.
- **Both builds.** Parity scenarios 260 to 265 compare the WASM and the
  native answers; CI's test shards passed them.
- **Two-node and three-node tests (section 7.4).** The direct-chat test
  changes Bob's vote and checks both counts on both nodes; the group test
  checks the same counts on all three nodes. Shared harness, no port
  literal.
- **Rules of AGENTS.md checked by tools:** file and function sizes,
  module layout, suppressions, duplication and change docs all passed in
  CI. No planning id in code, comments or test names (the only `Q1` is a
  poll's question text in a test).
- **Living docs:** `AGENTS.md`, `system-architecture.md`,
  `TERMINOLOGY.md` and the traceability matrix say what C4a built, without
  over-claims. The developer guide has false lines (R9).

**Counts of this round's new findings: 1 blocker, 2 major, 13 minor.
Open: 16 (R1 to R16).**

### R1 — CI is red: the Hub registry test expects three message ids (blocker)

*Where:* [registry.test.ts:60](../../../../crates/roym_web/ui/src/extensions/registry.test.ts#L60)
to `:64` at `92863fd4`.

*Problem:* `allManifestMessageIds()` now returns five ids, because
`MANIFESTS` holds `poll.json`. The test still lists three. Plan point 10
missed this test, but `mise run verify` (hub-ui gate) shows it. The new
TypeScript functions `widgetType`, `widgetNameId` and `widgetFieldLabelId`
have no Hub test at all.

*Failure case:* CI "Lints, licenses & checks" fails in
`mise run test:roym-ui`: "expected [ …(5) ] to deeply equal [ …(3) ]",
with `+ "ext-poll-widget-single"` and `+ "ext-poll-widget-single-option"`.
The PR cannot merge.

*Fix:* add the two ids to the expected list, in `MANIFESTS` order. Add one
test in `registry.test.ts` that `widgetNameId("poll", "poll/single")` and
`widgetFieldLabelId("poll", "poll/single", "option")` give the Rust ids,
and that `widgetType(POLL_REF, "poll/single")` is found and
`widgetType({id: "scheduling", version: 1}, "poll/single")` is not. Then
run `mise run verify` with no `--skip`, as the C4a exit criteria ask.

*Status:* Open.

### R2 — The change doc and the backlog name tests that do not exist (major)

*Where:* [roym-extensions change.md:84](../../../changes/roym-extensions/change.md#L84)
to `:87` (EX-10 to EX-13, test column);
[deferred-backlog.md:551](../../../deferred-backlog.md#L551) (the 8 KB row).

*Problem:* the test column names eight test functions. None of them
exists anywhere in the workspace (`git grep "fn <name>"` finds nothing):
`round_trip_poll_body`, `shape_prompt_and_fields`,
`every_shipped_chat_manifest_has_valid_widgets`,
`poll_manifest_loads_and_declares_widget_type`,
`chat_kind_manifest_has_no_workflow_keys`, `eight_kib_boundary_at_send`,
`refuses_unknown_field_type` and `derived_widget_ids_match_specification`.
The backlog row "A widget definition larger than 8 KB is refused" also
names `eight_kib_boundary_at_send`. The C4a exit criterion "Every C4a row
of section 3 names a test that exists and passes" is not met.

*Failure case:* a reader who looks for the proof of EX-10, EX-11 or EX-13
finds nothing. `check-change-docs` does not check test names, so no tool
catches it.

*Fix:* replace each name with a real one, for example EX-10:
`a_poll_definition_passes_every_check`, `check_shape_rule_1_…` to
`check_shape_rule_5_fields`, `a_chat_manifest_loads_and_its_widget_type_is_found`,
`a_chat_manifest_with_a_deal_key_is_refused`; EX-11:
`a_body_of_exactly_the_limit_passes_and_one_byte_more_is_refused`,
`a_body_with_unknown_field_type_returns_widget_shape`; EX-13:
`widget_ids_are_derived_and_a_clash_is_refused`. Same in the backlog row.
Check every name with `git grep` before the commit.

*Status:* Open.

### R3 — `conversation` is over the Q9 limit, and the record names an option that does not exist (major)

*Where:* [task.md:173](task.md#L173) to `:185`;
[roym-extensions change.md:287](../../../changes/roym-extensions/change.md#L287);
[status.md](status.md) C4a row.

*Problem:* `conversation` grew by 170,873 bytes (+21.00%). The Q9 limit is
+15% (122,072 bytes), and the plan aimed at less than 40 KB (D-C4-12). The
plan's stop rule says: over +40 KB, look for what linked more than the
planned list (`widget.rs`, `extension/widget.rs`, `schema::check_fields`);
over the Q9 limit, stop and tell the owner. The docs say the owner
approved "under Q9 (Alternative 3)". Q9 has only (a) and (b); there is no
third option, no date and no owner wording in the plan, this review file or
the change doc. No document says what made the 170 KB: about 400 lines of
code would not normally add that much, so something larger is probably
linked (for example more of `listing`, `deal` or the registry than the
plan expected).

*Failure case:* a later reader cannot tell whether the owner decided this,
or what was decided. A size that no one has explained can hide a link to
code that `conversation` must not carry (the reason D-C4-12 exists).

*Fix:* (1) Find what is linked: build `conversation` on `main` and on
this branch from one `target/` folder and compare the symbol sizes (for
example `twiggy top` or `twiggy diff` on the two `.wasm` files). Record the
three largest new items in task.md. (2) If it can be cut back under +15%
by not linking a large part, do that. (3) If not, write the owner's real
decision in the change doc's Deviations and in task.md: the date, the
words, and the limit now in force. Remove "Alternative 3".

*Status:* Open.

### R4 — The `widget.list` time is not measured (minor)

*Where:* plan section 7.8 ("C4a step 3 records both on the WASM build");
exit criterion "One `widget.list` with the Hub's `widget_ids` on the WASM
build under 200 ms, recorded"; [status.md](status.md) C4a row.

*Problem:* no document gives a time for `widget.list`. The stop rule
(over 200 ms, stop and tell the owner) was not applied.

*Failure case:* a slow scan on the WASM build (200 `get_message` host
calls, then up to five history pages) is found only when the Hub (C4b)
calls it on every thread redraw.

*Fix:* time one `widget.list` on the WASM build with 200 `widget_ids` in a
chat of 1,000 messages (a throwaway parity test is enough, as C3c1 did for
`transaction.drive`). Record the number in status.md and task.md.

*Status:* Open.

### R5 — `complete` is `false` when the scan reads exactly the last message (minor)

*Where:* [widget.rs:138](../../../../crates/roym_conversation/src/app/widget.rs#L138)
to `:148` (`scan_history`);
[dual_build_parity/widget.rs:617](../../../../crates/roym_web/tests/dual_build_parity/widget.rs#L617)
to `:630` (scenario 265).

*Problem:* `complete` becomes `false` when `seen == 1,000`, even when that
message is the last one in the chat. D-C4-7 says `false` only "when the
scan stopped at the cap before the end of chat". Scenario 265 puts exactly
1,000 messages after the widget and asserts `false`, so the test checks the
wrong value. Plan 7.7 says the test uses 1,001 messages.

*Failure case:* a chat with a poll and exactly 1,000 later messages
reports `complete: false`, though every answer was read. A caller (the Hub
in a later slice, or `roymctl`) that shows "counts may be missing" shows
it when they are not.

*Fix:* when the cap is reached, set `complete = false` only if more
messages remain: a later item in this page, or `page.next_cursor` is
`Some`. Test both sides: 1,000 messages after the widget gives `true`,
1,001 gives `false`.

*Status:* Open.

### R6 — Without ids, the scan stops collecting answers after the 51st widget (minor)

*Where:* [widget.rs:206](../../../../crates/roym_conversation/src/app/widget.rs#L206)
to `:216` (`scan_unfiltered`) with `:150` (`if !complete … break`).

*Problem:* when a 51st widget is seen, the closure sets `complete =
false`. `scan_history` then stops at the end of that page of 200. Answers
on later pages to the 50 widgets that are listed are not read. The plan's
pseudocode has the same shape, but its goal is a cap on the number of
widgets returned, not a shorter scan.

*Failure case:* `roymctl roym widget list` in a chat with 51 polls, where
votes for poll 1 arrive later than message 200 after poll 51: the counts
of poll 1 are too low, with no sign except `complete: false`, which also
means "more widgets".

*Fix:* keep two flags in `scan_unfiltered`: `more_widgets` (the 50-widget
cap) and the scan's own `complete` (the 1,000-message cap). Only the scan
cap stops the scan. Return `complete = scan_complete && !more_widgets`.
Add a parity case: 51 widgets, then an answer to widget 1 more than 200
messages later, and the count of widget 1 is 1.

*Status:* Open.

### R7 — The 8 KiB refusal at send, and `answer-too-large`, are not tested (minor)

*Where:* [dual_build_parity/widget.rs:201](../../../../crates/roym_web/tests/dual_build_parity/widget.rs#L201)
to `:207`; [widget/tests.rs:128](../../../../crates/roym_core/src/widget/tests.rs#L128).

*Problem:* plan 7.2 says scenario 262 sends "a body over 8 KiB
(`widget-too-large`)". The test sets an 8 KiB prompt, which breaks the
512-byte prompt rule first, and asserts `widget-shape`. So `widget.send`
never reaches `widget-too-large`. The unit boundary test checks only
`parse_widget_body` (the receive side), not `widget_body` (the send
side). No test reaches `answer-too-large` (`answer_body`,
`parse_answer_body`). Both are reachable. At send, only `poll/single`
passes `check_type`, and 12 plain options stay near 2.5 KB; but JSON
escapes a control character as 6 bytes, so 12 options of 128 bytes of
`\u{1}` (which is not blank) and such a prompt give more than 8,192
bytes and pass `check_shape`. An answer is over the limit when it answers
an inbound widget of an unknown type with several long text fields.

*Failure case:* a change that drops the size check in `widget_body` or
`answer_body` passes every test.

*Fix:* in scenario 262, send a poll that passes `check_shape` and whose
body is over 8,192 bytes (12 options and a prompt made of escaped
characters, as above) and assert `widget-too-large` on both builds. Also
answer an inbound unknown-type widget with long text fields so the answer
body is over 8,192 bytes, and assert `answer-too-large`. In `widget/tests.rs`, test
`widget_body` at exactly 8,192 and 8,193 bytes, and `answer_body` and
`parse_answer_body` over 8,192 (`answer-too-large`).

*Status:* Open.

### R8 — Scenario 265 does not prove the forward scan or the deleted id (minor)

*Where:* [dual_build_parity/widget.rs:561](../../../../crates/roym_web/tests/dual_build_parity/widget.rs#L561)
to `:651`.

*Problem:* plan 7.2 says scenario 265 shows that "a missing or deleted id
is left out" and that "the scan pages forward from the oldest asked
widget to collect answers". The test asks for one missing id, but no
deleted id. No answer is placed on a later page than the first, and no
answer is placed before the oldest asked widget. So the paging and the
start point are not tested.

*Failure case:* a change that reads only the first page, or starts the
scan at the newest asked widget, passes scenario 265.

*Fix:* add to scenario 265: a deleted widget id in `widget_ids` (left
out); an answer more than 200 messages after the widget (counted); an
answer to a second, newer asked widget that is older than that widget but
newer than the oldest asked widget (counted); an answer with a sender time
before the oldest asked widget (not counted).

*Status:* Open.

### R9 — The developer guide has false and missing lines; two other doc lines are stale (minor)

*Where:* [developer-guide.md:1897](../../../../docs/developer-guide.md#L1897),
`:1919`, `:2576` to `:2578`;
[roym-extensions change.md:225](../../../changes/roym-extensions/change.md#L225);
[deferred-backlog.md:318](../../../deferred-backlog.md#L318).

*Problem:*

- `:2576` says for `widget.send`: "if unknown, structural shape rules are
  checked". In fact `widget.send` refuses an unknown type with
  `extension-unknown` (D-C4-6, scenario 262).
- `:1919` says `widget-too-large` covers "a widget definition or answer
  body". An answer over the limit gives `answer-too-large`.
- The refusal table has no row for `widget-version`, `widget-json`,
  `widget-rules`, `widget-refused` or `answer-too-large`.
- `:2578` says each widget view has `semantic`. `WidgetView` has no
  `semantic`; it is inside `definition`.
- `:1897` says a `chat` manifest declares an "author prompt". The
  manifest has no prompt; the prompt is in each widget message.
- `:2577` leaves out `languages` from `widget.answer`'s input.
- change.md `:225` still says "`[[widgets]]` … are not built … Widgets
  come with C4". The `widgets` key of a `chat` manifest is now built.
- The backlog row at `:318` says the newest answer is chosen "by message
  timestamp (`created_at_secs`)". The code uses `sender_timestamp` in
  milliseconds; there is no `created_at_secs`.

*Failure case:* a developer who follows the guide expects `widget.send`
to accept an unknown type, and a client that branches on `data.code`
looks for the wrong code for a large answer.

*Fix:* correct the six guide lines and add the five missing codes to the
table. In change.md `:225`, say that `widgets` is built in a `chat`
manifest (C4a) and `[service]` is not. In the backlog row, write
`sender_timestamp` (milliseconds, the sender's clock).

*Status:* Open.

### R10 — `widget.answer` and `widget.list` with ids accept a message that is not admitted (minor)

*Where:* [widget.rs:71](../../../../crates/roym_conversation/src/app/widget.rs#L71)
to `:85` (`answer`), `:165` to `:178` (`scan_filtered`).

*Problem:* both read the widget with `AppConversation::get_message`, which
returns any row (`store/message.rs:262`), also an inbound row whose
admission is `undecided`, held or dropped. History, and so every answer
that `widget.list` counts, returns only own rows and accepted rows
(`store/history.rs:63`). D-C4-7 says the scan counts what history returns.
So a widget that the person has not accepted (for example in a message
request from a stranger) can be listed with ids and answered. The guest
can tell: such a row has `visible_seq == 0` (the insert at
`store/message.rs:244` writes 0, and `Accept` sets it).

*Failure case:* a caller passes the id of a held widget from an unknown
sender to `widget.answer`. The node sends a vote into that conversation
before the person accepted the sender. The Hub does not do this today
(it sends ids from history), but `roymctl` and any later caller can.

*Fix:* in `answer` and `scan_filtered`, treat a row that is not outgoing
and has `visible_seq == 0` as not found (`widget-not-found`, left out of
the list). Check that restored rows keep a non-zero `visible_seq`. Add one
parity case with an undecided inbound widget.

*Status:* Open.

### R11 — Constants, checks and conversions are written twice (minor)

*Where:* [widget.rs:25](../../../../crates/roym_core/src/widget.rs#L25)
to `:28` and [extension/widget.rs:9](../../../../crates/roym_core/src/extension/widget.rs#L9)
to `:12`; [load.rs:121](../../../../crates/roym_core/src/extension/load.rs#L121)
to `:130`; [widget.rs:103](../../../../crates/roym_core/src/widget.rs#L103)
to `:119` and `:280` to `:288`;
[app/widget.rs:348](../../../../crates/roym_conversation/src/app/widget.rs#L348)
to `:362` and `:326` to `:343`.

*Problem:*

- `MAX_WIDGET_TYPE_LEN`, `MAX_SEMANTIC_LEN`, `MAX_WIDGET_FIELDS` and
  `MAX_WIDGET_CHOICES` are defined in both `widget.rs` and
  `extension/widget.rs`. The manifest load uses one copy, the message
  check the other. One edit makes them differ.
- `check_widget_fields` copies the field-name rule of
  `schema::check_definition` instead of calling it (the plan says "passes
  the name, `max_len` and number rules of `schema::check_definition`").
- The `DefinedField` → `schema::Field` conversion is written in
  `answer_fields` and again as `dummy` in `check_shape`.
- `zero_counts` repeats the count loop of
  `compute_view_answers_and_counts` with 0.

*Failure case:* a later change to one copy (a new limit, a new name rule)
leaves the other one behind, and the manifest rule and the message rule
disagree.

*Fix:* keep the four constants in one place (for example
`extension/widget.rs`) and re-export or use them from `widget.rs`. In
`check_widget_fields`, build the `Field` and call
`schema::check_definition` (it does the name, length and number rules;
`choices: None` passes `check_choices` for a non-choice field, so call it
for a choice field with one placeholder code, or keep only the name rule
in one shared helper). Add one `DefinedField::to_schema_field` and use it
in both places. Build the counts once: start from `zero_counts` and add
the latest answers.

*Status:* Open.

### R12 — `check_insight` now refuses `service`, which the plan does not say (minor)

*Where:* [load.rs:46](../../../../crates/roym_core/src/extension/load.rs#L46).

*Problem:* D-C4-3 says `check_workflow` and `check_insight` refuse a
non-empty `widgets`. The code also adds `m.service.is_some()` to the
insight refusal. On `main`, an `insight` manifest with a `service` key
loaded. This is a new rule for an existing kind, with no test and no
line in the Deviations.

*Failure case:* none today (no shipped insight manifest has `service`).
But the code does more than the plan, and no reviewer of the change doc
can see it.

*Fix:* either remove `|| m.service.is_some()` from `check_insight`, or
keep it, add a test (`an_insight_with_a_service_is_refused`) and a line in
the roym-extensions Deviations.

*Status:* Open.

### R13 — A repeated id in `widget_ids` lists the widget twice, the second time with no votes (minor)

*Where:* [widget.rs:165](../../../../crates/roym_conversation/src/app/widget.rs#L165)
to `:178` and `:247` (`by_widget.remove(&m.id)`).

*Problem:* `scan_filtered` keeps every asked id that passes, also a
repeated one. `project_widgets` takes the answers out of `by_widget` with
`remove` for the first copy. The second copy finds nothing and gets zero
counts.

*Failure case:* `widget.list` with `widget_ids: ["w1", "w1"]` returns two
views of `w1`: one with the real counts and one with all counts 0. A
client that keeps a map by `widget_id` (D-C4-10) may keep the second.

*Fix:* drop repeated ids before the fetch (keep the first, in order), and
add the case to scenario 265.

*Status:* Open.

### R14 — `widget.answer` accepts a widget that `widget.list` refuses (minor)

*Where:* [widget.rs:87](../../../../crates/roym_conversation/src/app/widget.rs#L87)
to `:89` (`answer`), against `:237` to `:244` (`project_widgets`).

*Problem:* `widget.list` runs `parse_widget_body` and then `check_type`,
and lists a known-type widget that breaks the type's rules as
`refused: "widget-rules"`, with no counts. `widget.answer` runs only
`parse_widget_body`. So a person can answer a widget that the node itself
shows as refused, and that answer is never counted. The plan's pseudocode
also has only `parse_widget_body`; the gap is the plan's and the code's.

*Failure case:* a peer sends `poll/single` with 20 options (shape passes,
type rules fail). `widget.list` shows it refused. `widget.answer` with
`{"option": "o15"}` succeeds and sends a message that no node counts.

*Fix:* in `answer`, run `check_type` after `parse_widget_body` and map an
error to `widget-refused` with the inner code, as for a parse error. Add
the case to scenario 264 or 262.

*Status:* Open.

### R15 — status.md counts and the plan's exit criteria do not match the code (minor)

*Where:* [status.md](status.md) C4a row;
[slice-c4-implementation-plan.md](slice-c4-implementation-plan.md) section 10;
[widget/tests.rs:295](../../../../crates/roym_core/src/widget/tests.rs#L295).

*Problem:* the C4a row says `roym_core::widget::tests` "14 passed";
`cargo nextest run -p syneroym-roym-core widget::tests` runs 16. The C4a
exit criteria in plan section 10 are all still unticked, and the row does
not say that `mise run verify` ran (it would have shown R1).
`poll_definition_builds_the_hub_fixture` writes no fixture and compares
with none; it only reads its own value. The plan says it "writes the
fixture that the Hub's `view.test.ts` reads, or compares with it".

*Failure case:* a reader trusts the count and the "Implemented" step,
while the hub-ui gate fails.

*Fix:* write the real counts. Tick each C4a exit criterion that holds,
with its evidence, after R1 to R4. Either rename the fixture test to what
it does (`poll_definition_has_the_hub_shape`) or make it write a JSON
fixture file now, so C4b's `view.test.ts` can read it.

*Status:* Open.

### R16 — Inline paths against the import rule, and a cast that can wrap (minor)

*Where:* [load.rs:116](../../../../crates/roym_core/src/extension/load.rs#L116)
and `:149` (`super::widget::WidgetFieldRule`), `:133` (`super::Field`);
[wire_origin.rs:82](../../../../crates/roym_web/tests/dual_build_parity/wire_origin.rs#L82)
and [shapes.rs:399](../../../../crates/roym_web/tests/method_contract/shapes.rs#L399)
(`syneroym_roym_core::widget::poll_definition`);
[dual_build_parity/widget.rs:451](../../../../crates/roym_web/tests/dual_build_parity/widget.rs#L451)
(`syneroym_roym_core::conversation::group::GROUP_REMOVED_NOTICE`);
[widget/tests.rs:147](../../../../crates/roym_core/src/widget/tests.rs#L147)
(`std::iter::repeat_n`); [widget.rs:187](../../../../crates/roym_core/src/widget.rs#L187)
and `:380` (`version as u32`).

*Problem:* AGENTS.md "Mandatory Import Cleanup": types by `use`,
functions qualified by their parent module, no inline paths with several
`::`. `load.rs` already imports `widget` and `schema`, but writes
`super::widget::WidgetFieldRule` and `super::Field`. The two `as u32`
casts turn a version such as 4,294,967,297 into `UnknownVersion(1)`, so
the error message names the wrong version.

*Failure case:* review noise only for the imports. For the cast, a
peer's body with a very large version is refused with a message that says
version 1.

*Fix:* import `WidgetFieldRule` and `Field` and use them by name; in the
tests, `use syneroym_roym_core::widget` (or the function) and
`use syneroym_roym_core::conversation::group`, and `use std::iter`. Keep
the version as `u64` in `UnknownVersion`, or use
`u32::try_from(version).unwrap_or(u32::MAX)`.

*Status:* Open.

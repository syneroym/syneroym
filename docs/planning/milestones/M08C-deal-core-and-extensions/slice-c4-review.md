# M08C slice C4 — review

*Reader: the developer who fixes the C4 plan or code, and the project owner.
This file holds every review finding for slice C4 (parts C4a and C4b). Plan
review findings are P1, P2, …; code review findings are R1, R2, …. Ids are
never renumbered.*

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-25

Checked [slice-c4-implementation-plan.md](slice-c4-implementation-plan.md) on
branch `feat/m08c-c4-widgets` at `a7ab43a9` (on top of `main` at
`aa25961b`). This is the main checkout, not a worktree (`git rev-parse
--git-dir` is `.git`). `git pull` had nothing to pull: the branch has no
upstream yet, so the push of this round creates it. The plan was written by
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

**Summary: 9 findings: 1 blocker, 2 major, 6 minor. 9 open.**

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

#### P1 — `history` pages oldest first; the whole `widget.list` scan, its early stop and its cost claims are inverted (blocker)

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

*Status:* Open

#### P2 — A vote that fails to send is invisible, and the voter's own count still includes it (major)

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

*Status:* Open

#### P3 — A widget from a newer build with a new field type or new key is refused, not drawn by the generic card (major)

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

*Status:* Open

#### P4 — What `widget.list` does with an asked id that is deleted, not a widget, or from another chat is not defined (minor)

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

*Status:* Open

#### P5 — Two guard tests list every conversation method and are not in the plan (minor)

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

*Status:* Open

#### P6 — The `send_body` extraction does not say where the reserved-type check goes (minor)

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

*Status:* Open

#### P7 — The Hub poll card hard-codes the field name and the heading (minor)

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

*Status:* Open

#### P8 — `widget.list` with no ids can return about 8 MB, and the output-size claim is wrong (minor)

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

*Status:* Open

#### P9 — "Newest answer" uses the voter's own clock, and a clock that steps back makes a vote do nothing (minor)

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

*Status:* Open

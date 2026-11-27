# M08C slice C7 — review

*Reader: the developer who fixes the C7 code, and the project owner. This
file holds the code review findings for slice C7 (parts C7a and C7b). The
two plan reviews are inside the plan, in the sections "Review on main
89d1dd23" and "Second review". Code review findings are R1, R2, …. Ids are
never renumbered.*

## Code review round 1 — Claude Opus 5.5, 2026-11-27

This is the first code review of **part C7a** (forward of a text message):
plan section 6.1, steps 0 to 7, and D-C7-1 to D-C7-13 as they apply to
C7a. PR #399 (draft), branch `feat/m08a-c7-msg-forwarding` at `25f0f6ae`,
main checkout (not a worktree), `git pull` up to date. The branch sits on
`main` at `722956b9` (`git merge-base`), so no rebase was needed. Diff:
`git diff origin/main...HEAD` (68 files). The code was written by DeepSeek
V4.1 Flash. There was no earlier code review, so new findings start at R1.

**CI.** `gh pr checks 399` on `25f0f6ae`: only "Check CLA Agreement" has
passed. "Lints, licenses & checks", the six test shards and Playwright E2E
are still **pending**. So CI is not yet proof that the full suite passes.
The next round must read `gh pr checks 399` again.

**Tests I ran** (targeted only): `cargo nextest list -p syneroym-roym-core`:
595 tests (status.md says 595), 46 of them in `chat::tests`, and
`signing::tests::resolve_principal_and_owner_refuses_each_certificate_state_with_its_code`
exists. One temporary Vitest probe of `forwardLine` (deleted after; the
tree is clean) to prove R2 and R3. I did not build the roym WASM, so I did
not run parity or `roym_forward_e2e`, and I did not measure the component
sizes again.

**Not checkable from the branch.** The owner asked the implementer to push
the node commits and report after step 4, before the Hub. The commits are
in the right order (node `1e91b478` to `caacff76`, then Hub `59f559af`),
but git cannot show whether the stop was made.

### Checked and found correct

- **The chat body (D-C7-1).** `ChatBody` has the plan's fields and
  constants. `parse` refuses another `chat_version` and another inner
  content type as `Unsupported`, and a missing field as `Malformed`. An
  unknown mode reads as `not-forwardable` (`lenient_mode`). `clamped_count`
  reads any JSON number: 0, a negative number and a fraction read as
  `None`; a number above 1,000 or above `u64::MAX` reads as 1,000; a
  string is `Malformed`. `to_bytes` builds the object by hand, so a
  failure cannot turn into an empty body. The mode is inside the body, so
  the sender's message signature covers it (FW-14).
- **The content hash (D-C7-2).** The layout is the plan's: the domain
  string, then `u32be` length prefixes for the content type, each tag, the
  body and the blob count. `sha256:` and lowercase hex through
  `backup::bytes_to_hex`. Tests: a fixed vector, a change of text, one tag,
  tag order, content type and blob list (FW-6, FW-8), and the two splits
  `["ab","c"]` and `["a","bc"]`.
- **The origin (D-C7-3).** `RECORD_MESSAGE_ORIGIN` and
  `("message-origin", 1)` in `RECORD_TYPES`. `origin_draft` has no subject,
  no expiry and no `supersedes`. `verify_message_origin` runs
  `record::verify_json` with `VerifyOptions::new(now)`, then refuses another
  type or version, a payload that does not parse (`deny_unknown_fields`), a
  subject, an expiry and an envelope with no `delegation`. `check_origin`
  tests the size before any parse, then the content type, the languages
  and the recomputed hash. The tests cover a service-key-only origin, an
  expired certificate (still checks), a certificate of another scope, an
  issuer that the certificate does not name, a `profile` record in the
  origin field, a tampered payload, a future date that checks once the
  clock catches up, and changed text with the old origin (FW-13).
- **The view (D-C7-6).** `forward_view` follows the table of D-C7-6 row
  by row: `forwardable` only for a checked `signed` body; `origin` only
  when checked, so a failed check can never name an author (FW-4);
  `forwarded` for a count of 1 or more, or for a known sender who is not
  the author; `many_forwards` from 5 (FW-5). `add_forward_views` runs in
  `history`, `changes` and `search` (the only three places that build a
  `MessageView`), reads the owner DID and the contacts once per page and
  only when the page has a chat body, replaces `body` with the inner text,
  and sets `body = None` for a body it cannot read, so raw JSON is never
  shown. `tombstone` clears `forward`.
- **Settings (D-C7-4, Q5 (a)).** One method `conversation.settings`. No
  field reads; no row reads as `not-forwardable`, `asked: false`; a storage
  error is `-32603`, never the default. `asked: false` is refused with
  field `asked`. `SettingsInput` has `deny_unknown_fields`, and a bad mode
  (`"signd"`) is `-32602` (parity 278).
- **Send with a mode (D-C7-5).** `send` keeps its old path when
  `forward_mode` is absent (Q3 (a)), and the chat type is reserved in its
  closure. `send_with_mode` refuses a content type other than `text/plain`
  on field `content_type`, then runs `check_target` **before** the signing
  call, then signs, then `deliver`. `send_body` is now `check_target`
  then `deliver`, with the same checks in the same order, so `widget.send`,
  `widget.answer` and the plain send keep their behaviour. A signing
  failure sends nothing (parity 281: the outbox is empty). Parity 285
  shows the order: a restored node with no certificate gets the group
  notice, not `signing-not-enrolled`.
- **`conversation.forward` (D-C7-7).** The steps are in the plan's order:
  not found, deleted, content type, parse, mode, origin check, then the
  copy with the same origin string and `next_forward_count`, then
  `send_body` with the target's group checks. It signs nothing and never
  reads the forwarder's default (parity 286 sets the default to
  `not-forwardable` first, FW-15). Own forward keeps the count (parity
  288, three-node step 6).
- **Access control.** Both new methods sit behind `admit::require_internal`
  in `invoke` like every other `conversation` method. Scenario 73 lists
  both, and the method table count is 29.
- **Search (D-C7-8, Q4 (a)).** `indexable_text` replaces the old test at
  all six call sites (`store/message.rs` three times, `group.rs`,
  `store/backup.rs`, `store/text_index.rs` through the same function). A
  JSON body with a top-level string `text` is indexed by that string only;
  anything else as before. The WIT doc comment of `search` says the rule.
  Host tests and parity 283 show that `signed`, `forward_mode`,
  `chat_version` and `envelope` find nothing.
- **Backup (D-C7-9).** `SCHEMA_VERSION` 5 with a "why" comment. The
  `settings` section is exported (also empty on a fresh node, a recorded
  deviation), and `import_settings` refuses another row id and a payload
  that does not parse. `import` stays short: the new arm is one call.
- **The signing helper (D-C7-11).** The body moved to
  `roym_core::signing` unchanged, word for word and code for code.
  `transaction` re-exports it under the same name, its 12 call sites do
  not change, and the now-unused imports are gone. A unit test covers the
  four refusal codes.
- **Widgets (D-C7-13).** A widget message is refused at the content-type
  step (parity 287) and the Hub draws no tag or action on it.
- **Both builds.** Every parity scenario 278 to 288 compares the WASM and
  the native answer.
- **Three-node test (section 7.4).** Steps 1 to 7 exist on `boot_trio`
  with `enrol_signing`; no port literal. It asserts the origin under
  2 KiB and the 200-message page under 500 ms, and prints the 11 KiB body
  size. The numbers in task.md and the change doc (848 bytes, 1,037
  bytes, 12,278 bytes, 100 and 164 ms) are under the owner's stop rules.
- **Hub (D-C7-10).** New code is in `chat/`; `messages.ts` and
  `groups.ts` only call it. The mode control starts at the default,
  `reset()` runs after each send (FW-12), and the group notice shows only
  for `signed` in a group (FW-11). The first-message question has no
  pre-selected choice, and Skip and closing save `asked: true` only. A
  refused `signed` send shows the two buttons and never resends by itself
  (ADR-0030, decision 7). "Forward" is drawn only when `forwardable` is
  true. The name comes only from the reader's contacts or `is_you`,
  never from the message. The Profile tab has the default (Q10 (a)).
  `OPEN_WORK` in `messages.ts` holds the dialog and the picker.
- **Rules.** No planning ids in code comments. New production files are
  short (`chat.rs` 463 lines, `chat/tests.rs` 645). `sha2` is added with
  `workspace = true` and is allowed in `check-roym-deps` (a recorded
  deviation). The backlog has the three new rows, and both revocation
  rows and "More demo stories" are extended. TERMINOLOGY, the developer
  guide, the requirements spec (attachments removed) and the traceability
  matrix are updated.

### Counts

Blocker 0, major 1, minor 9. **10 open.**

### R1 — The change doc's Deviations miss almost every item the plan lists (major)

*Where:* [change.md](../../changes/roym-message-forwarding/change.md),
`## Deviations` (lines 192 to 218) and line 28, at `25f0f6ae`.

*Problem:* Plan section 9 lists what the change doc's `## Deviations` must
record for C7a: the chat body content type and format, and that a message
with no mode stays `text/plain` (Q3); the correction of "the substrate
conversation capability does not change" (Q4 (a)); the forward count rule
(read up to 1,000, own forward keeps it); the one `conversation.settings`
method (D-C7-4); the inner content and the hash layout (D-C7-2); FW-8 in
the format only (Q8); where the setting lives (Q5); the check at read time
(Q6); the name rule and the delete sentence (Q9); the tags (section 4,
item 16); and the C7a/C7b split (Q2). Only the measured sizes are there.
The new sub-section "Different from the plan" records how the code differs
from the plan, which is useful, but it is not this list. Line 28 still
says "The substrate conversation capability does not change", which is
now false: its indexing rule changed. The plan says these Deviations
"win over its main text" and the exit criteria need them. There are also
three empty lines before `## Close-out`.

*Failure case:* C7b and C8 read the change doc, not the plan, as the
record of the design. They find no content type, no hash layout and no
count rule, and line 28 tells them the host was not touched.

*Fix:* Add one short bullet per item of plan section 9 to `## Deviations`
(above "Different from the plan"), with the same numbers as the plan
(1,000, 5, 8 KiB, the hash layout, `sha256:` hex). The Q4 bullet must say
that the capability's interface does not change but its indexing rule gets
one case. Remove the extra empty lines.

*Status:* Open.

### R2 — The name of an unknown author starts with two "…" (minor)

*Where:* [forward_view.ts:51](../../../../crates/roym_web/ui/src/chat/forward_view.ts#L51)
and [en.ftl:1036](../../../../crates/roym_web/ui/src/i18n/en.ftl#L1036)
(also `hi.ftl:1006`), at `25f0f6ae`.

*Problem:* The code passes `short` as `` `…${did.slice(-12)}` ``, and the
message is `…{ $short } (not in your contacts)`. Both add the "…".

*Failure case:* Proved with a probe of `forwardLine` for an author not in
the contacts: the text is `……3z6wm8w5e4ae (not in your contacts)`. The
unit test passes because it checks only `toContain`.

*Fix:* Pass `short: did.slice(-12)` and keep the "…" in the message (so a
translation can place it), and make the unit test compare the whole text
of `.forward-author`.

*Status:* Open.

### R3 — A contact with no display name is shown as "not in your contacts" (minor)

*Where:* [messages.ts:448](../../../../crates/roym_web/ui/src/screens/messages.ts#L448),
[groups.ts:471](../../../../crates/roym_web/ui/src/screens/groups.ts#L471)
and [forward_view.ts:44](../../../../crates/roym_web/ui/src/chat/forward_view.ts#L44),
at `25f0f6ae`.

*Problem:* Both maps store `display_name || ""` for a contact. `authorName`
tests `if (known)`, so an empty name falls through to the "not in your
contacts" text. Q9 (a) says: the contact name if the author's DID is in
the reader's contacts. A contact with no display name is still a contact.

*Failure case:* Proved with the same probe: a map with the author's DID
and `""` gives `……3z6wm8w5e4ae (not in your contacts)`. The reader is
told a saved contact is a stranger.

*Fix:* In `authorName`, test `contacts?.has(did)`. For a contact with an
empty name, show the short DID without "(not in your contacts)". Add a
unit test for it.

*Status:* Open.

### R4 — In a group, a new message closes an open forward picker (minor)

*Where:* [groups.ts:223](../../../../crates/roym_web/ui/src/screens/groups.ts#L223)
(`threadList.replaceChildren()`), and the feed handler at `groups.ts:851`
to `:873`, at `25f0f6ae`.

*Problem:* The picker is put inside the message row
(`openForwardPicker(button, …)`). Every `conversation.*` event for the open
group calls `reloadThread`, which redraws `threadList`. `groups.ts` has no
`OPEN_WORK` check like `messages.ts` has, so the picker is removed.

*Failure case:* Bob opens "Forward" in a busy group. A member sends a
message before Bob picks a chat. The picker disappears, and Bob must start
again. In an active group he may never finish.

*Fix:* In the group feed handler, skip a reload (or keep a pending one)
while `threadList` holds `.forward-picker`, as `messages.ts` does with
`OPEN_WORK`. Add a Hub unit test that a feed event leaves an open picker.

*Status:* Open.

### R5 — A group delete never shows the "signed origin stays" sentence (minor)

*Where:* the delete callback at
[groups.ts:813](../../../../crates/roym_web/ui/src/screens/groups.ts#L813)
to `:822`, at `25f0f6ae`.

*Problem:* Plan section 4, item 17 (ADR-0030, decision 12: "It cannot be
taken back"): the delete dialog of a `signed` message says that the
origin and the forwards stay. Only `openDeleteDialog` in `messages.ts`
does this. A group delete has no dialog at all, so a `signed` group
message is deleted with no note. A group is where a signed message is
most exposed (FW-11). The plan named only `messages.ts`, so this is a gap
in the plan that the code inherited.

*Failure case:* Alice deletes a `signed` message in a group and believes it
is gone. Every member still holds its origin and can forward it.

*Fix:* Before a group delete of a row with `forward.mode === "signed"`,
show the `messages-delete-signed-note` text and ask to confirm. Or record
the gap in the change doc's Deviations and a backlog row.

*Status:* Open.

### R6 — The group composer now sends the text without trimming (minor)

*Where:* [groups.ts:539](../../../../crates/roym_web/ui/src/screens/groups.ts#L539)
and `:549`, at `25f0f6ae`.

*Problem:* `doSend` computes `body = sendInput.value.trim()` and used to
send `body`. The new `send` sends `sendInput.value`. This is a behaviour
change that is not in the plan's audit (section 4).

*Failure case:* "  Market opens at 9\n" is sent with the spaces and the
new line. For a `signed` message the origin hash covers them too, so the
same words typed twice give two different hashes.

*Fix:* Pass the trimmed `body` into `send` (and use it again for "Send as
not forwardable").

*Status:* Open.

### R7 — New code imports the chat functions directly (minor)

*Where:* [chat_send.rs:9](../../../../crates/roym_conversation/src/app/chat_send.rs#L9)
to `:12`, [forward.rs:10](../../../../crates/roym_conversation/src/app/forward.rs#L10)
to `:13` and [view.rs:5](../../../../crates/roym_conversation/src/app/view.rs#L5),
at `25f0f6ae`.

*Problem:* AGENTS.md: types by `use`, functions through their parent
module. D-C7-1: "Callers write `chat::…` after `use
syneroym_roym_core::chat;`". These files import `content_hash`,
`origin_draft`, `check_origin`, `next_forward_count`, `forward_view` and
`unreadable_view` and call them bare.

*Failure case:* A reader of `forward.rs` cannot see that `check_origin` is
the shared pure check and not a local helper.

*Fix:* Import `chat` and call `chat::content_hash`, `chat::check_origin`
and so on. Keep the types in the `use` list.

*Status:* Open.

### R8 — The developer guide's new example is not run by story 30 (minor)

*Where:* [developer-guide.md:2581](../../../developer-guide.md#L2581),
at `25f0f6ae`.

*Problem:* Plan section 9: "The tagged code blocks run in story 30, so a
new tagged block must run there too." The new block is plain
```` ```bash ```` with `"…"` placeholders, so `30-roym-client.sh` never runs
it. The method names and fields in it can drift from the code with no
test failing.

*Fix:* Tag the block (`bash roym-client=forward`) with real values taken
from the earlier blocks, and run it in `30-roym-client.sh`. A `signed`
send needs an enrolled certificate; if the story node has none, use
`not-forwardable` for the send and show `conversation.settings` only, and
say why in the guide.

*Status:* Open.

### R9 — The integrated spec says the origin is signed with the master DID (minor)

*Where:* [roym-integrated-experience-spec.md:610](../../../roym-integrated-experience-spec.md#L610)
and `:735`, at `25f0f6ae`.

*Problem:* Both say the origin is "signed with the author's master DID".
The master key signs nothing here. The `conversation` service key signs
the origin, under the person's delegation certificate, and the envelope's
`issuer` is the master DID (ADR-0030, decision 11; D-C7-3). AGENTS.md:
"Never make a statement less precise to make it shorter."

*Failure case:* A reader concludes that a stolen service key cannot sign
origins as the person. The backlog row says the opposite.

*Fix:* Say "signed as the person: by the `conversation` service key, with
the person's delegation certificate, so it names the person's master DID".

*Status:* Open.

### R10 — Some tests prove less than they claim (minor)

*Where:* [forward.rs:236](../../../../crates/roym_web/tests/dual_build_parity/forward.rs#L236)
(scenario 284), [forward.rs:228](../../../../crates/roym_web/tests/dual_build_parity/forward.rs#L228)
(scenario 283), [forward.rs (service):51](../../../../crates/roym_conversation/src/app/forward.rs#L51),
and `chat/tests.rs` (`an_origin_signed_for_one_master_but_naming_another_issuer_fails`,
`an_origin_with_a_certificate_of_another_scope_is_refused`), at
`25f0f6ae`.

*Problem:*
1. Scenario 284 imports the bundle into the same node, not into a wiped
   one (plan 7.3, I). It shows an overwrite, not that a fresh install gets
   the choice back and does not ask again.
2. The last search in scenario 283 (`limit: 2`, 2 results) passes with or
   without the host rule. It does not show the lost-hit case.
3. The `origin-not-checked` refusal of `conversation.forward` has no test
   on either build. The chat type is reserved in `send`, but the parity
   harness can store a body through the host directly.
4. The two unit tests assert only `Failed { .. }`, so they pass if the
   check fails for another reason (for example the hash).

*Fix:* (1) Import into `harness().await` and read `conversation.settings`
there. (2) Remove the last search, or make it a case that fails without
the rule (for example, `limit: 1` with `signed` as the query and a newer
plain message holding "signed"). (3) Add a parity case that stores a chat
body with a broken origin through `wasm_conversation`/`native_conversation`
and asserts `data.code` `origin-not-checked`. (4) Assert the reason text.

*Status:* Open.

## Smaller points (no finding)

- `forward_view`'s `OriginCheck::Absent` arm (`chat.rs:424`) cannot be
  reached: `check_origin` returns `Absent` only for a `not-forwardable`
  body, which returns earlier, and a `signed` body with no origin is
  `Failed { "no signed origin" }`. The plan said `Absent` there. The result
  is the same; the arm can go.
- `verify_message_origin` parses the envelope twice (`Envelope::from_json`
  and `record::verify_json`) only to read `delegation`.
- `send` computes `languages` and then drops it on the mode path;
  `send_with_mode` computes it again.
- `answerFirstMessage` and `chooseForwardMode` in `hub-helpers.ts` repeat
  the same choice-to-label code.

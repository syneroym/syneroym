# M08B slice B3b — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-b3b-implementation-plan.md](slice-b3b-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-10

Checked on `feat/m08b-b3b-chat-lang-tags` at `e0de3bc6`, in the main checkout
(not a worktree: `.git` is a directory). `git pull` said "Already up to date".
The only commit after the plan's base (`4830bdf7`) is the plan itself, so the
code facts below hold on `main`.

**Who reviewed.** This session runs on Claude Sonnet 5.5. The `status.md` row
says the plan was written by Claude Opus 5.5. They are different models, as
the prompt says.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md`
names no other change doc for B3b. The plan also reads
[demo-stories](../../changes/demo-stories/change.md) for context only. I agree:
ML-14 and ML-25 are both in roym-multilingual, so every B3b decision and
deviation belongs there (plan section 3 says the same).

**Summary:** 0 blocker, 1 major, 8 minor, 9 items. 9 Fixed (6dbe9dcd), 0 Open.

Limits of this review: I read the plan, the docs, the code and the e2e
specs. I ran one experiment outside the repository: a copy of `decode_body`
compiled with `rustc` and fed a hostile length (P1). I did not run any plan
step, the Hub tests or `mise run verify`, and I changed no code.

### What was checked and found correct

- **File and line links.** Spot-checked and right: `envelope.rs:13`, `:30` to
  `:61`; `dag.rs:292` to `:316`; `host_impl.rs:76`; `lib.rs:430`, `:470`;
  `transport.rs:211`, `:318`, `:362`, `:368` to `:381`, `:712`; `group.rs:452`,
  `:476`; `group/entry.rs:110`, `:146`, `:166`, `:286`; `schema.rs:78`;
  `store/message.rs:698`; `backup.rs:205`, `:358`, `:397`; `rpc/conversation.rs`
  `send` at `:200`; `app_host` `lib.rs:324`, `guest.rs:241`;
  `app_host_native/host.rs:399`; `capabilities_messaging.rs:72`, `:182`;
  `engine/auth.rs:170` to `:186`; `method/conversation.rs:10`;
  `roym_conversation/messages.rs:262` to `:313`, `:320`, `:342`;
  `roym_core/conversation.rs:141`, `:162`; `shapes.rs:324`;
  `roym_conversation_e2e.rs:150` to `:156`; `messages.ts:279` to `:302`,
  `:529` to `:534`; `groups.ts:114`, `:405` to `:444`; `profile.ts:37`;
  `hub-helpers.ts:66` to `:91`; `roym-groups.spec.ts:100`; backlog `:536`;
  requirements `:1462`; architecture `:964`, `:568`.
- **File sizes.** `crypto.rs` 786, `transport/tests.rs` 781, `group/tests.rs`
  766, `transport.rs` 752, `message.rs` 737, `group.rs` 735, `store/tests.rs`
  717, `events.rs` 715, `fixtures.rs` 798, `method/tests.rs` 727, `messages.ts`
  931, `groups.ts` 694, `profile.ts` 171, `roym-hub.spec.ts` 1420, `entry.rs`
  469. All match section 7.5. `host_impl/tests.rs` (243) and
  `store/tests/backup_roundtrip.rs` (218) have room.
- **Callers of the host `send`.** I counted `.send(` in the host crate's tests:
  3 + 6 + 7 + 2 + 1 = 19 ("about 20" is right). `sandbox_wasm` has one real
  caller. The only `ConversationHost` implementors are `ConversationService`
  and the two never-built stubs. The only `AppConversation` implementors are
  `GuestHost`, `NativeAppHost` and one test host. The only non-Rust caller of
  the Roym `conversation.send` is the Hub, plus the developer-guide block that
  story 30 runs.
- **Every constructor of the message types.** I searched `visible_seq` and
  `ConversationMessage`: the plan names every non-test site (rpc,
  `capabilities_messaging`, `engine/auth`, `convert/conversation`,
  `roym_conversation` x2, `roym_core` x1, `entry.rs` literal).
- **Group receive path.** `apply_message_entry` returns `Ok((false, None,
  None))` on a decode error and does not mark the entry applied
  (`entry.rs:146` to `:149`), so Q1 (a) "the same as an entry whose body cannot
  be decoded" is exact. Entries are retried one by one, so one bad entry does
  not hold the others (`group.rs:520` to `:556`). Signature, membership and
  epoch checks come first (`validate_and_insert`, `entry.rs:379` to `:420`).
- **1:1 receive path.** `verify_incoming_envelope` runs before the session is
  committed, and `InvalidArgument` after decrypt already exists for the
  timestamp (`transport.rs:368`). It maps to "callee refused" on the sender
  (`transport.rs:712`). So the new check follows a working path.
- **System messages.** Group-key, deletion-request and refusal-notice sends all
  go through `enqueue_direct` with `system = true`; the Roym `send` already
  refuses the reserved content types before the host call. Passing `&[]` for
  them is right.
- **Message id and digest.** `derive_message_id` does not hash the tags, and the
  transcript digest hashes only `id`, `author`, `sender_timestamp` and
  `content_type` (`history.rs:143`). D-B3b-3 and point 9 are right.
- **The row column list.** `row_to_message` reads 0 to 23; `languages` at 24 is
  free. The seven `SELECT` lists (`message.rs:254`, `:574`, `:649`;
  `history.rs:34`, `:66`; `search.rs:34`, `:56`) and the backup export list
  (`backup.rs:205`) are all the lists the plan names. No `INSERT ... SELECT`
  or table copy exists, so no other statement reads the table by position.
- **No ADR changes.** ADR-0013 and ADR-0025 do not name the payload fields.
  Only `system-architecture.md:1646` names `DeliveryPayload`, and it lists no
  fields.
- **Hub facts.** The 1:1 thread is redrawn by `renderThread` on
  `reloadList(select)` (`messages.ts:204`, `:226`), so the plan's rule "the
  list starts again at the Hub language when the thread is drawn again" is
  what happens to the compose box today. The group chat box is built once per
  `renderGroup`, and `reloadThread` redraws only the message list, so the
  group list keeps its rows between sends. `roym-hub.spec.ts:387` and the
  `.compose-input` / "Send" / `.composer-box input` steps are not broken by a
  `<select>` and `type="button"` buttons. `messages-open-button`,
  `messages-send-button`, `tab-messages` and the three `profile-languages-*`
  ids exist in both catalogs.
- **Both builds.** `languages` is a plain list of strings, so no new WIT type is
  needed. The dual-build fixture links the same WIT file. All six Roym guests
  and the native shim go through `app_host`, so one change per layer covers
  both builds.
- **Release profile.** `Cargo.toml:42` to `:48` sets `panic = "abort"` (this
  matters for P1).
- **Plan form.** "How this is shown to an end user" has 5 bullets for the end
  user and 3 for a developer. Each question names the step it blocks. Backlog
  rows have a reason, a target and a link. Section 7.8 maps to earlier review
  kinds.

### Views on the owner's questions

(For the owner to decide. I do not answer for the owner.)

- **Q1 (what a receiver does with a bad list).** I would take (a). One rule on
  both sides, and what is stored equals what was signed. The cost is small:
  only a node that does not run the sender-side check can send a bad list. See
  P4: a refused group entry stays "unapplied" and is decrypted again on every
  pass, so the plan should say what that costs and log it.
- **Q2 (how the WIT carries the list).** I would take (a). Every caller
  changes once, and the compiler finds every one of them. (b) adds a type for
  one field. (c) leaves two functions that do the same work.
- **Q3 (what the chat box shows).** I would take (a): one component for profile
  and both chat boxes. Two risks for the owner to weigh. First, the default tag
  is the Hub language. An English-Hub user who writes Hindi in Latin letters
  sends `en` unless they change the list, and a screen reader on the other
  side then reads it as English, which is the case ML-5 wants to avoid. Second,
  the list resets each time a 1:1 thread is drawn again, so the person must
  change it again. A "remember the last choice" setting would fix both, but it
  is a new preference, so it is the owner's call. I would still take (a) for
  B3b and add a backlog row if the owner wants the memory later.
- **Q4 (language names under a tagged message).** I would take (a). ML-14 asks
  only for the "not stated" note, and B5 adds the translate action where the
  source language matters.
- **Q5 (do cards carry message languages).** I would take (a). The record
  inside a card already has its own `lang` (B3a). Two places that can disagree
  is worse than one.
- **Q6 (story 40).** I would take (a). It shows the whole path (list, send, `lang`
  on the sent message). Bob getting one more message per run is the same as
  story 20. The step needs no new state, because story 20 has already made the
  chat.

### Findings

#### P1 — `decode_body` can panic on a hostile length, and B3b rewrites it and adds a second parser like it (major)

- **Where:** D-B3b-2 (`encode_body`, `decode_body`, `take_languages`), Step 3.4
  (`dag.rs`), section 7.2 ("`dag.rs` tests"); `crates/conversation/src/dag.rs:300`
  to `:316`.
- **Problem:** Today `decode_body` reads `ct_len` as a `u64`, casts it to
  `usize`, and tests `encoded.len() < 8 + ct_len`. A `ct_len` near
  `u64::MAX` overflows the `+`. I compiled a copy of the function and fed it
  8 bytes of `0xff` and one more byte. In a release build it panics with "slice
  index starts at 8 but ends at 7". In a debug build it panics with "attempt
  to add with overflow". The workspace release profile has `panic = "abort"`
  (`Cargo.toml:46`), so a panic stops the whole substrate. The input is the
  plaintext of a group entry. A member holding an epoch key can write any
  plaintext, and the entry passes `validate_and_insert` (it is a validly signed
  entry from a member). `apply_pending_entries` then runs it, and runs it
  again on every pass, because the entry is never marked applied. B3b did not
  create this, but D-B3b-2 replaces the function and adds `take_languages`,
  which reads a count and one length per tag in the same way. The plan says
  `take_languages` "refuses a count over 3 before it allocates", but it names no
  rule for the length fields, and the planned tests (7.2: "a count of 4, a
  truncated tag and a non-canonical tag") do not cover a huge length.
- **Failure case:** A member of a group sends one entry whose plaintext starts
  with `ff ff ff ff ff ff ff ff`. Every member's node that applies the group
  aborts, and aborts again after each restart, until the group state is removed
  by hand.
- **Fix:** In D-B3b-2 add the rule: no `+` or `-` on a length read from the
  wire. Read each length with `usize::try_from(u64)`, add offsets with
  `checked_add`, and take slices with `get(a..b)`; any failure is `Err`. Use
  the same rule in `take_languages` (a tag longer than `MAX_TAG_LEN` can be
  refused before it is sliced). Add tests to section 7.2: `decode_body` with
  `ct_len = u64::MAX`, with `ct_len = usize::MAX - 7`, a tag length of
  `u64::MAX`, and a count of `u64::MAX`; each must return `Err` and not panic.
  Say in the plan (and in the PR text) that this also closes an existing hole,
  so the owner knows the fix is in this slice and no backlog row is needed.
- *Status:* Fixed (6dbe9dcd)

#### P2 — `sign` and `enqueue_direct` get an eighth argument, and clippy refuses that (minor)

- **Where:** D-B3b-2 (`sign` takes `languages: &[String]`), Step 3.4 (`lib.rs`
  `enqueue_direct` "with `languages: &[String]`"); `crates/conversation/src/envelope.rs:66`
  to `:73` (`sign`: 7 arguments today), `crates/conversation/src/lib.rs:430` to
  `:438` (`enqueue_direct`: `self` and 6 more).
- **Problem:** `clippy::too_many_arguments` fires above 7 arguments. Both
  functions have exactly 7 today and get 8. The workspace sets `complexity` to
  `warn`, and `mise run verify` runs clippy with `-D warnings`. The plan names
  the `too_many_lines` risk in two places but not this one. The crate already
  allows it in five places (`group.rs:90`, `:365`; `message.rs:52`, `:134`,
  `:184`), and the ratchet in `xtask/src/lint_suppressions.rs` counts only
  `too_many_lines`, so an `#[allow(clippy::too_many_arguments)]` is the
  accepted way.
- **Failure case:** Step 3 builds and its tests pass, but the `clippy` gate of
  the final `mise run verify` fails on `sign` and `enqueue_direct`.
- **Fix:** Say in Step 3.4 that these two functions get
  `#[allow(clippy::too_many_arguments)]`, as the other five do. `canonical_bytes`
  (6 to 7) and `send_group` (6 to 7) stay inside the limit.
- *Status:* Fixed (6dbe9dcd)

#### P3 — Three raw `INSERT` statements are not named as places that write `languages` (minor)

- **Where:** D-B3b-5 (it lists the three insert functions and
  `record_outgoing_group_message`), D-B3b-13; `group/entry.rs:166` (the incoming
  group message `INSERT OR IGNORE`), `group/entry.rs:286` (the withdrawn row),
  `store/backup.rs:397` (the import `INSERT`).
- **Problem:** Section 2.2 point 7 lists these sites, and Step 3.4 lists the
  files. But D-B3b-5 says which functions take `languages`, and D-B3b-13 moves
  only the `StoredMessage` literal. The SQL in `apply_message_entry` stays a
  raw statement with a numbered list of columns, and so does the import. A
  receiver that adds `languages` to the literal but not to the SQL stores `''`.
  The column default hides the error: the code builds, and only the new group
  test sees it. The withdrawn row has no body, and the plan does not say what
  it stores.
- **Failure case:** The Hub shows "Language not stated" on every group message
  that came from another member, while the in-memory notification had the
  list.
- **Fix:** In D-B3b-5, name the three raw statements. For
  `apply_message_entry`, the `INSERT` writes `content.1` joined by a space; for
  the import, `m.languages`; for the withdrawn row, say "no list" (the body is
  gone, and the default `''` is right). Add to section 7.2 a group test that
  reads the stored row (not the notification) after a tagged entry arrives.
- *Status:* Fixed (6dbe9dcd)

#### P4 — A group entry with a bad list is retried on every pass and nobody sees it (minor)

- **Where:** Q1 (a), D-B3b-2 (`decode_body` runs `parse_message_languages`),
  W6; `group/entry.rs:146` to `:149`, `group.rs:520` to `:556`.
- **Problem:** The plan says the entry "is not applied, the same as an entry
  whose body cannot be decoded today". That is true, and it has two costs the
  plan does not name. The entry stays `applied = 0`, so
  `unapplied_dag_entries` returns it, and `apply_pending_entries` decrypts and
  decodes it again on every pass for the life of the group. The refusal also
  writes no log line, so the person who is missing a message and the operator
  have nothing to look at (the 1:1 side at least ends as `failed` with "callee
  refused").
- **Failure case:** A buggy peer node that signs an uppercase tag sends many
  messages. Each one stays in the unapplied list. Every pass repeats all
  their decrypts. No log says why the messages are missing.
- **Fix:** Pick one and write it into D-B3b-4. (a) Add a `tracing::warn!` with
  the entry id and the author (never the text) at the refusal, and state in the
  plan that retry is the accepted cost, with a backlog row if the owner wants
  the entry marked applied later. (b) Mark the entry applied with no row, as the
  withdrawn path does, so it is not retried; then "what is stored" is
  still what was signed, and the entry is still not shown. I prefer (a) now
  because (b) changes the DAG head logic and is a separate decision.
- *Status:* Fixed (6dbe9dcd)

#### P5 — "Language not stated" is planned for every non-card, non-deleted row, including binary ones (minor)

- **Where:** D-B3b-11, behaviour change 3, section 7.1 (`message_text.test.ts`);
  `crates/roym_web/ui/src/screens/messages.ts:529` to `:534`,
  `crates/roym_web/ui/src/screens/groups.ts:114`.
- **Problem:** The plan says the note shows for "a text message with no tag",
  and that a card, a deleted message and a group system row do not. But the 1:1
  `else` branch at `messages.ts:529` draws every row that is not a card, and
  `renderMessageRow` draws every group row that is not a membership event or a
  group name. A row whose `body_encoding` is `base64` (any content type that
  `roym_core::conversation::is_text_content_type` does not call text) would get
  the note under a base64 string. Both row types already carry `body_encoding`.
- **Failure case:** A program sends `application/octet-stream` with no tag. The
  Hub shows the base64 text with "Language not stated", which is false (it is
  not text at all) and marks the blob with a `lang` attribute.
- **Fix:** Say in D-B3b-11 that `messageText` is used only when
  `body_encoding === "utf8"`; other rows are drawn as today. Add a case to
  `messages.test.ts` and `groups.test.ts`: a `base64` row has no note and no
  `lang`.
- *Status:* Fixed (6dbe9dcd)

#### P6 — D-B3b-6 does not say how to put `data.field` on the "languages" error (minor)

- **Where:** D-B3b-6, W9, section 7.2 (`method_contract`); `crates/roym_core/src/envelope.rs:62`
  (`Response::invalid_params(message)`), `crates/roym_core/src/method.rs:66`
  to `:77` (`ParamError::into_response`).
- **Problem:** The plan says the count and duplicate errors are
  `Response::invalid_params(…)` "with `data.field = "languages"` (as
  `method.rs:76` adds it)". `invalid_params` takes only a message and sets no
  data. Only `ParamError { field, message }.into_response()` sets
  `data.field`. An implementer who follows the sentence literally returns
  `-32602` with no `data`, and the `method_contract` case in 7.2 then fails.
- **Failure case:** Step 4.2 is written as `Response::invalid_params(e.to_string())`;
  Step 4.5 fails with "missing data.field".
- **Fix:** Write in D-B3b-6: `ParamError { field: Some("languages".into()), message:
  e.to_string() }.into_response()`.
- *Status:* Fixed (6dbe9dcd)

#### P7 — Point 2 names a second conversation test host that does not exist (minor)

- **Where:** Section 2.2 point 2 ("two test hosts in `roym_core/signing.rs:516`
  and `:686`"), Step 3.6; `crates/roym_core/src/signing.rs:509`, `:686`.
- **Problem:** There is one `impl AppConversation for TestHost`
  (`signing.rs:509`, `send` at `:516`). The `send` at `:686` belongs to
  `impl AppWebSocket for TestHost`. It takes `(conn, frame, kind)` and does not
  change.
- **Failure case:** An implementer looks for a second conversation host,
  finds none, and loses time, or edits the wrong function.
- **Fix:** Name one host (`signing.rs:516`). Keep `:686` out of the list.
- *Status:* Fixed (6dbe9dcd)

#### P8 — The language list needs a name and a role for screen readers (minor)

- **Where:** D-B3b-10 (`LanguageList.root`), D-B3b-12 (`field()` label
  `messages-languages-label`); `crates/roym_web/ui/src/dom.ts` (`field`).
- **Problem:** The component's root is a `<div>` that holds a `<ul>` and a
  `<select>`. A `<label>` can name a form control, not a `<div>`. So the
  "Message languages" label from `field()` does not name the group, and the
  "add" `<select>` has only its first option ("Add a language") as a name. The
  slice is about marking language for screen readers (ML-5), and its own
  control should be readable by one.
- **Failure case:** A screen reader reads "combo box, Add a language" and a
  list of rows with no heading, and the person does not know these are the
  languages of the message they are about to send.
- **Fix:** In D-B3b-10 give the root `role="group"` and `aria-label` (the
  `*-languages-label` message, through `t()`, so the visible-text check is
  satisfied), and give the "add" `<select>` the same text as an `aria-label`.
  Add a case to `language_list.test.ts`. The profile screen gets the same
  change, so it should pass its own label.
- *Status:* Fixed (6dbe9dcd)

#### P9 — Step 3 is one large step, and the host half can be checked alone first (minor)

- **Where:** Section 6, Step 3 (nine sub-steps, 8 crates, about 65 test call
  sites, the WIT and both builds).
- **Problem:** The plan's rule is "each step builds and its tests pass", and
  Step 3 does. But it changes the WIT, the generated types, `syneroym-rpc`,
  the host crate, the WASM host, the native shim and the fixture before any
  test run. A mistake in the new wire format (D-B3b-2) and one in the WIT
  plumbing then show together, and a re-run needs `mise run build:roym` for the
  WASM side each time.
- **Fix:** Split Step 3 at the point where the plan already splits for the Roym
  side (3.7). Step 3a: `text_lang` use, `envelope.rs`, `dag.rs`, the store, the
  host internals and their tests, with `host_impl::send` passing `&[]` and
  the WIT unchanged; run `cargo nextest run -p syneroym-conversation`. Step 3b:
  the WIT, `syneroym-rpc`, the hosts and converters, the host `send` taking the
  list and checking it (D-B3b-4), and both builds. Both parts stay in one PR.
- *Status:* Fixed (6dbe9dcd)

## Code review round 1 — Claude Opus 5.5, 2026-11-11

Reviewed `git diff origin/main...HEAD` on `feat/m08b-b3b-chat-lang-tags` at
`ddd3a73b` (PR #356, draft), in the main checkout (not a worktree). `git pull`
said "Already up to date". The code was written by Gemini 3.8 Flash. The
prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). Every B3b
decision and deviation belongs there. This is the first code review round, so
there are no earlier R items to confirm.

**Summary:** 0 blocker, 3 major, 5 minor (R1 to R8). 0 Open, 8 Fixed.

### What was checked and found correct

- **CI.** `gh pr checks 356` on `ddd3a73b`: "Lints, licenses & checks" and the
  CLA check pass. Playwright E2E and Test 1/6 to 6/6 were still pending when
  this round was written, so CI is **not yet proof** that the full suite passes
  on this sha. The fix session must read the checks again.
- **Run here.** `npm test` in `crates/roym_web/ui`: 39 files, 443 tests
  passed. This matches status.md. `cargo nextest run -p syneroym-text-lang
  -p syneroym-conversation`: 191 passed, 0 skipped.
- **The rule (D-B3b-1).** `MAX_MESSAGE_LANGUAGES = 3`, `TagListError`,
  `check_tag_list`, `MessageLanguagesError` and `parse_message_languages`
  match the plan. `ProfilePayload::validate` calls `check_tag_list` and maps
  both errors to the old variants, so the profile's text and order of checks
  do not change. One copy of the loop.
- **Wire (D-B3b-2, W1 to W8).** `put_languages` is the one writer of the list,
  for the 1:1 bytes and the group plaintext. The tag list sits between
  `content_type` and the body length. `take_languages` and `decode_body` use
  `checked_add`, `usize::try_from` and `get(..)`, refuse a count over 3 before
  `Vec::with_capacity`, and refuse a tag longer than `MAX_TAG_LEN` before
  slicing. The four hostile inputs of plan section 7.2 are tested and return
  `Err`. The domain string stays `syneroym:conversation:v1`. The sender builds
  the payload from the stored row (`transport.rs:217`) with the stored list,
  so signed and sent bytes agree. The message id does not include the tags.
- **Receive checks (D-B3b-4).** 1:1: in `verify_incoming_envelope`, after the
  signature, timestamp and conversation checks, `InvalidArgument`; nothing is
  stored (tested). Group: `decode_body` runs `parse_message_languages`; the
  entry is not applied and a `tracing::warn!` names the entry id and author,
  not the body. Backup import: the same check per message, `InvalidInput`.
  `deliver_inbound` skips it, as the plan says. `ingest_verified` did not grow.
- **Store (D-B3b-5).** Column `languages TEXT NOT NULL DEFAULT ''` at the end of
  `messages`, space-separated. `MESSAGE_COLUMNS` is used by the five unaliased
  `SELECT`s; the two `search.rs` queries append `m.languages` at index 24. All
  six `INSERT` statements that write a row (three store functions,
  `record_outgoing_group_message`, `apply_message_entry`, backup import) bind
  the list; the withdrawn row keeps the default. A hand-inserted row reads
  back `[]` (tested).
- **D-B3b-13.** `incoming_group_row` holds the old literal field for field,
  plus `languages`. `apply_message_entry` got shorter.
- **Both builds (D-B3b-8).** The WIT field and parameter are in the planned
  places with the planned doc line. `ConversationHost`, the WASM host, the two
  never-built stubs, `AppConversation` (guest and native), both converters,
  the `on-message` JSON and the fixture all carry the list. The converter
  tests check the field. `post_rpc` asserts WASM and native answers are equal,
  so the contract cases run on both builds.
- **Roym side (D-B3b-6, D-B3b-7).** `SendInput.languages` is `Vec<LangTag>`
  with `skip_serializing_if`, so the two card sends write the same JSON as
  before (tested). `"languages": ["hi_IN"]` answers `-32602` with
  `data.field = "languages[0]"`; 4 tags and a repeated tag answer `-32602`
  with `data.field = "languages"` (tested). `MessageView.languages` is always
  written. The membership and group-name views set `[]`. The method shape diff
  holds only `languages` keys.
- **`roymctl` (D-B3b-9).** `--lang` is repeatable, goes through
  `LangTag::parse`, and `build_send_params` adds `languages` only when the list
  is not empty. `docs/demo/demo.md` R6 has `--lang en`.
- **Two nodes.** `roym_conversation_e2e.rs` sends `["hi", "en"]` from A and
  reads the same list on A and on B.
- **Hub.** `languageList` and `messageText` match D-B3b-10 and D-B3b-11:
  every button has `type="button"`, the root has `role="group"` and an
  `aria-label`, and the "add" `<select>` has the same label. `messageText` is
  used only for `body_encoding === "utf8"`; a card, a deleted message, a
  binary message and a group system row get no note (tested). The search
  snippet gets `lang`. All text goes through `textContent`, and `lang` through
  `langAttr`. The nine new message ids exist in `en.ftl` and `hi.ftl`, and the
  Hindi uses the profile's words for "add a language", "move up" and
  "remove". The Hindi sweep draws a tagged and an untagged message on both
  chat screens.
- **e2e and story 40.** The new Playwright test moves Hindi up, sends a mixed
  text, checks `lang="hi"` and no note, and checks again after a reload. Story
  40 has the one step of Q6 and reads `BOB_ADDR` with `requireEnv`.
- **Rules.** No planning ids in code, comments or test names. Every touched
  file is under its size limit (`fixtures.rs` 799, `transport/tests.rs` 790,
  `crypto.rs` 787); no capped file changed. The two
  `#[allow(clippy::too_many_arguments)]` are the ones P2 asked for. The
  crate-level `cfg_attr(test, allow(clippy::unwrap_used, …))` in `text_lang`
  is the workspace pattern. Sizes in task.md are within section 7.6.
- **Backlog.** The CLDR row has the chat sentence and the
  `verify_incoming_envelope` link. The four new rows of plan section 8 exist,
  with a reason, a target and a link.

### Findings

#### R1 — major — a click on the list's label or on a language name removes the main language

- *Where:* `crates/roym_web/ui/src/screens/messages.ts:323`,
  `crates/roym_web/ui/src/screens/groups.ts:466`; `field()` in
  `crates/roym_web/ui/src/dom.ts:60`.
- *Problem:* Both chat boxes put the language list inside `field(...)`, which
  is a `<label>`. A `<label>` forwards a click on its text, or on any part of it
  that is not a control, to its first labelable descendant. In the list, that
  is the first row's "Remove" button (the first row has no "Move up").
- *Failure case:* Alice's chat box shows "Hindi, English". She clicks the word
  "Hindi" (to see what it is) or the label "Message languages". Hindi is
  removed without a word, and her next message goes out as `["en"]`. A second
  click removes English, and the message goes out with no tag. Proved here with
  a throw-away vitest file (deleted): `languageList("messages", 3, ["hi",
  "en"], …)` inside `field()`; a click on `.messages-language-name` of row 1
  gives `["en"]`, then a click on the label text gives `[]`.
- *Fix:* Do not wrap the list in a `<label>`. Use a `<div class="field">` with
  a `<span>` for the visible text (the root already has `role="group"` and
  `aria-label`), or a heading as the profile screen does. Add a test to
  `language_list.test.ts` or to both screen tests: a click on the visible label
  and on a language name does not change `value()`.
- *Status:* Fixed (b4ffbb2a). Proof: added click tests in `messages.test.ts` and `groups.test.ts` asserting clicks on visible label and language names do not alter `value()`; broke locally by wrapping in `<label>` and observed both tests fail with value changed, verified both pass after fix.

#### R2 — major — the profile form puts back a language the person removed

- *Where:* `crates/roym_web/ui/src/screens/profile.ts:101` to `:103`.
- *Problem:* On `main`, a loaded profile always replaced the list:
  `Array.isArray(env.payload.languages) ? [...] : []`. B3b changed this to
  "call `set` only when `languages` is an array". But
  `ProfilePayload.languages` has `skip_serializing_if = "Vec::is_empty"`
  (`crates/roym_core/src/person.rs:37`), so a stored profile with an empty
  list comes back with **no** `languages` key. The list then keeps its start
  value `[currentLanguage()]`. This reopens B3a R3. The test
  `stored profile with empty languages …` (`profile.test.ts:178`) still passes
  because its fixture sends `languages: []`, which the service never sends.
- *Failure case:* A person removes every language and saves. Later they
  change their name and save again. The form sends `languages: ["en"]`, a
  language they removed. Proved here with a throw-away vitest file (deleted):
  a `profile.get` payload without `languages` draws 1 row and `profile.set`
  gets `["en"]`.
- *Fix:* `langList.set(Array.isArray(env.payload.languages) ?
  env.payload.languages : [])`. Change the fixture of the test at
  `profile.test.ts:178` to leave out `languages` (the form the service
  writes), or add a second case for it.
- *Status:* Fixed (29926059). Proof: updated `profile.test.ts:178` fixture to omit the `languages` key as written by `ProfilePayload` serialization; broke locally with old `Array.isArray(env.payload.languages)` check and observed test fail (expected empty array, got `["en"]`), verified passes after fix.

#### R3 — major — the change doc's Test column names tests that do not exist

- *Where:* `docs/planning/changes/roym-multilingual/change.md:110` (ML-14),
  `:125` (ML-25).
- *Problem:* Plan section 3 and 7.8 say the names are written "from the real
  files" (the B3a R2 finding). `git grep` finds none of these:
  `check_tag_list_enforces_limits_and_duplicates`,
  `parse_message_languages_enforces_rules`, `host_send_languages_direct`,
  `host_send_languages_group`, `host_send_languages_validation_errors`,
  `incoming_direct_envelope_validates_languages`,
  `group_message_entry_validates_languages`,
  `export_import_message_languages_roundtrip`,
  `scenario_53_conversation_history_languages_parity`. There is no host test
  that sends a group message with tags at all (see R7).
- *Failure case:* A reader who checks ML-14 or ML-25 cannot find 9 of the
  cited tests, and ML-25 cites a group send test that was never written.
- *Fix:* Use the real names: `check_tag_list_enforces_count_and_uniqueness`,
  `parse_message_languages_validates_tags_and_list` (`text_lang`);
  `payload_with_languages_verifies_and_detects_tampering`,
  `put_and_take_languages_roundtrip_and_safety` (`envelope.rs`);
  `encode_and_decode_body_roundtrip_and_safety` (`dag.rs`);
  `send_with_valid_languages_is_stored_and_returned_by_get_and_history`,
  `send_with_invalid_languages_is_refused_with_invalid_argument_and_stores_nothing`
  (`host_impl/tests.rs`);
  `direct_message_with_languages_is_stored_and_non_canonical_is_refused`
  (`delivery.rs`); `group_message_with_languages_is_stored_in_db_and_read_back`,
  `group_message_with_non_canonical_language_tag_is_refused_and_not_stored`
  (`group/tests/languages.rs`);
  `backup_roundtrip_preserves_language_tags_and_rejects_non_canonical`;
  `scenario_53_conversation_send_is_never_optimistic_parity` (parity
  `conversation.rs`). Add the group send test of R7 and cite it, or remove
  `host_send_languages_group`. Check every name with `git grep` before the
  commit.
- *Status:* Fixed (5c53208b)

#### R4 — minor — the change doc and the developer guide state wrong facts

- *Where:* `docs/planning/changes/roym-multilingual/change.md:293`;
  `docs/developer-guide.md:1783`.
- *Problem:* The D-B3b-5 decision says the column holds "comma-separated
  canonical tags". The code joins with one space and splits with
  `split_ascii_whitespace` (`store/message.rs:715`). The developer guide gives
  `messageText(host, message, draw)`; the real function is
  `messageText(body, languages, tag?)`, and it returns nodes. The same kind
  as B3a R9.
- *Failure case:* A developer who writes a migration or a raw query from the
  change doc splits on `,` and gets one tag `"hi en"`. A developer who follows
  the guide calls a function that does not exist.
- *Fix:* Write "joined by one space (a tag has no space)". Write
  `messageText(body, languages, tag = "span")`, which returns the nodes to
  append. The D-B3b-4 decision can also name the backup import refusal, which
  the code does.
- *Status:* Fixed (5c53208b)

#### R5 — minor — the PR text is wrong in three places and leaves out two things the plan asks for

- *Where:* PR #356 description.
- *Problem:* (1) It says "`GroupEntryPlaintext` adds `languages`
  (bincode-encoded with backward-compatible format)". There is no
  `GroupEntryPlaintext`; the group plaintext is `encode_body`, and the new
  format is not compatible with an old node (plan section 4, W4/W5). (2) It
  says a bad 1:1 list fails "with `ProtocolViolation`"; the code answers
  `InvalidArgument`. (3) It says the list has "move up/down"; it has only
  "move up". Plan section 4 asks the PR text to say that a node with B3b and a
  node without it cannot exchange messages, and that an old
  `conversation.db` must be reset. Plan section 7.7 asks it to list the 9 new
  Hindi lines for the owner's review. Neither is there.
- *Failure case:* The owner reviews the PR text, believes old nodes still
  work, and finds no Hindi lines to review (exit criterion 11).
- *Fix:* Correct the three points. Add a "Not compatible with older nodes"
  paragraph (W4, W5, W7) and the 9 Hindi lines from `hi.ftl`.
- *Status:* Fixed (PR #356 description)

#### R6 — minor — the host checks the languages before the reserved-type check, not after it

- *Where:* `crates/conversation/src/host_impl.rs:84` to `:97`.
- *Problem:* D-B3b-4 says the check runs "first thing after the reserved-type
  check". The code runs it before. A send with a reserved content type and a
  bad list now answers "message languages: …" instead of "this content type is
  reserved". This is a reordered check, and the change doc's Deviations do not
  name it. The `canonical_tags` → `to_string()` step is also not needed:
  `parse_canonical` accepts only strings that are already canonical, so the
  host can pass on the `languages` it got, as the plan says.
- *Failure case:* An app that sends `DELETION_REQUEST_CONTENT_TYPE` with
  `["HI"]` gets the language error, and a test that expects the reserved-type
  error for such a call fails.
- *Fix:* Move the `parse_message_languages` call after the reserved-type
  `if`, and pass `&languages` on. Or keep the order and record it under
  Deviations.
- *Status:* Fixed (6014dad7)

#### R7 — minor — several tests prove less than their names or the plan say

- *Where:* `crates/roym_web/tests/dual_build_parity/group.rs:509` to `:538`;
  `crates/conversation/src/group/tests/languages.rs:117`;
  `apps/roymctl/src/commands/roym/tests.rs:390`;
  `crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts:72`.
- *Problem:*
  - `scenario_210_group_message_with_languages_parity` calls
    `strip_volatile` and `normalize_message_ids` on both answers and then
    never compares them. It checks one field on each side. The other parity
    scenarios end with `assert_eq!(hw, hn)`.
  - No test sends a group message with tags through the host
    (`ConversationService::send` on a group, so `send_group`,
    `encode_body` and `record_outgoing_group_message`). The group tests only
    insert an entry built by hand on the receiving side. So the sender's stored
    row for a group message is never checked, and exit criterion 2 ("group …
    between two nodes") has no test.
  - `group_message_with_non_canonical_language_tag_is_refused_and_not_stored`
    does not check the warning that P4 asked for. Its comment says the code
    "returns Ok(())"; it returns `Ok((false, None, None))`.
  - The `roymctl` test checks only `is_err()` for `INVALID_TAG!`. The plan asks
    for `--lang hi_IN` and the `Malformed` text.
  - Story 40 does not check that English was added (two rows) before it sends.
- *Failure case:* A WASM-only difference in a group history (any field but
  `languages`) passes scenario 210. A bug that drops the list in
  `record_outgoing_group_message` passes every test.
- *Fix:* End scenario 210 with `assert_eq!(hw, hn)`. Add a host test in
  `group/tests/languages.rs` that sends with `["hi", "en"]` on a group and
  reads the sender's row back. Check the warning with the crate's log capture,
  or record under Deviations that it is not tested. Use `hi_IN` and check the
  error text in the `roymctl` test. In story 40, check two
  `.messages-language-row` after `addMessageLanguage`.
- *Status:* Fixed (6014dad7, 076ecacd, 9c3294de, 53757d56, 5c53208b)

#### R8 — minor — new full paths, a length read written four times, and an unused parameter

- *Where:* `crates/conversation/src/envelope.rs:81`;
  `crates/conversation/src/dag.rs:296`, `:321`, `:543`;
  `apps/roymctl/src/commands/roym/group.rs:238`;
  `crates/roym_core/src/method/tests.rs:515`;
  `crates/conversation/src/store/tests/backup_roundtrip.rs:255`;
  `crates/substrate/tests/e2e/hub-helpers.ts:90`.
- *Problem:* AGENTS.md "Mandatory Import Cleanup": new lines use
  `std::str::from_utf8`, `crate::envelope::put_languages` /
  `take_languages`, `syneroym_text_lang::LangTag::parse` and
  `crate::store::backup::BackupBundle` inline. Import the type (`LangTag`,
  `BackupBundle`) and the parent module (`std::str`, `crate::envelope`).
  The block "8 bytes with `checked_add`, `get`, `try_into`,
  `u64::from_be_bytes`" is written three times in `take_languages` and once
  in `decode_body`. `addMessageLanguage` takes `_tag` and never uses it.
- *Failure case:* No wrong behaviour. The next hostile-length fix has to be
  made in four places, and a reader of `addMessageLanguage(page, 'en', 'hi')`
  thinks the Hindi tag matters.
- *Fix:* Fix the imports. Add one `pub(crate) fn take_u64(bytes: &[u8], at:
  &mut usize) -> Result<u64, String>` in `envelope.rs` and use it in both
  functions. Drop the `_tag` parameter and its argument in story 40, or use it.
- *Status:* Fixed (6014dad7, 076ecacd, 238deb83, 53757d56)

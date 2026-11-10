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

**Summary:** 0 blocker, 1 major, 8 minor, 9 items. 9 Open.

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

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
- *Status:* Open

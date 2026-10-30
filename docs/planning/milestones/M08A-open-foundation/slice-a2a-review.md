# M08A slice A2a — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a2a-implementation-plan.md](slice-a2a-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Opus 5.5 (Claude Code), 2026-10-30

Checked against the code at `758bbf72` on `feat/m08a-a2a-typed-methods`.
Line numbers below are at that commit.

What was checked and found correct:

- The method lists. `profile` dispatches 22 names
  ([app.rs:64-85](../../../../crates/roym_profile/src/app.rs#L64)) plus the
  two certificate verbs. `conversation` dispatches 22 names
  ([app.rs:190-211](../../../../crates/roym_conversation/src/app.rs#L190))
  plus the two certificate verbs. `on_message` and `status` are WIT exports,
  not JSON-RPC methods. No other method reaches these two services. The
  plan's tables have all 48, with the right params and results, except the
  points in P6 and P7.
- Callers. A script listed the top-level keys of every literal call to these
  48 methods in `crates/substrate/tests`, `crates/roym_web/tests`, the Hub,
  `roymctl` and the sibling crates. Calls that build the name at run time
  were read by hand. Only one caller sends a field the new structs refuse or
  a wrong type: the Hub's object `profile_envelope` (B1). No caller sends
  both `group` and `conversation`. Every `null` sent is for a field the plan
  makes `Option`.
- `serde_json` has no `preserve_order` in either build (no `indexmap` under
  it in `Cargo.lock`), so map keys are visited in sorted order in both
  builds. The native build adds only the `raw_value` feature
  (`cargo tree -e features -i serde_json`). That feature does not change
  error texts, so the WASM and native builds give the same `-32602` text.
- The macro follows the `macro_rules!` follow-set rules (`path` then `[`,
  `ty` then `=>`). `MethodInfo::of` can store `parse_value::<I>` as a
  `fn` pointer through a closure that captures nothing.

### Owner questions — reviewer view

**Q1 (method table).** Agree with (a): one list writes both the dispatch and
the table. But the plan keeps two hand-written paths beside it: the
certificate verbs (`handle_certificate_verb` plus `certificate_rows`), and
whatever the source scan of one file misses. I would add: the two
certificate verbs become normal rows of the macro list, with typed handler
functions in `roym_core::signing`, so `invoke` has no hand dispatch left.
See P2.

**Q2 (where types live).** Agree with (a). A struct has one owner, and
`roym_core` stays small. A small layout point is in P18.

**Q3 (wire half of the rule now).** Agree with (a). It is about 30 lines and
one test, and it keeps the rule defined once. Keep it a parameter of the
check only, as planned. Do not add production code for it.

**Q4 (`-32602` and the field name).** Agree with (a), `serde_path_to_error`.
The derivation of the field name in the pseudo-code is wrong for nested
missing fields (P1). Duplicate keys need the same treatment (P11). Arrays
need a decision (P9). I checked how the path looks for an unknown field:
`serde_path_to_error` records the key while it reads it, before the visitor
fails, so the path ends with the unknown key itself, not `?`.

**Q5 (outputs as named types).** Agree with (a). Wrapping the three arrays
changes the wire, and A2 does not reshape methods. A backlog row is the right
record.

**Q6 (parity scenario 2).** Agree with (a). The rule "passes unchanged in
shape" is written for A1 in task.md. Shape means the module count (24), the
scenario count (222) and the two-build comparison. Option (a) keeps all
three. It changes one expected result, which OF-18 requires, so it does not
break the rule. It must be recorded as a deviation, as the plan says.
Renaming the test is right, because "inert" is no longer true. The
expected `data.field = "auth"` depends on the sorted key order, which holds
in both builds today (see above). If that ever changes, assert that the
field is one of `auth`, `caller`, `person_did` instead.

**Q7 (closed word sets as enums).** Agree with (a). The only callers are the
Hub `<select>` values and tests, and they send valid words.

**Q8 (the five sibling call sites).** Agree with (a). It is the only way the
compiler sees drift between the services. But each site handles errors in
its own way today, and the plan does not say to keep that. Without a rule
per site, (a) changes wire text and behaviour. See P4.

**Q9 (Hub `profile_envelope`).** Agree with (a). The service, the stored row
and every test use a string. The Hub fix also makes the Hub path verify the
envelope, which is what the form promises. The Playwright helper
`addContact` sends no envelope, so no e2e test changes.

### Findings

- [x] **P1 — major.** *Where:* section 2 Q4; section 6 Step 3, the
  `parse_value` pseudo-code. *Problem:* `serde_path_to_error` gives
  different paths for different error kinds. For a wrong type or an unknown
  field, the path already ends with the field (`favourite`,
  `bundle.manifest.x`). For a missing field, the path is the parent
  (`.` at the top, `bundle.manifest` when nested), and the name is only in
  the text. The pseudo-code takes `path` whenever it has a named segment, so
  a nested missing field reports `bundle.manifest`, not
  `bundle.manifest.subject_did`. *Fix:* use this rule. If the text starts
  with `missing field` (or `duplicate field`, P11), `field` is the path
  joined with the quoted name (only the name when the path is `.`).
  Otherwise, `field` is the path when it has at least one named segment.
  Otherwise there is no field. Unit tests in Step 3: missing, unknown and
  wrong type, each at the top and nested two levels down. Remove the
  `unverified` note about `?`, but keep a test that pins the real text.
  *Status:* Fixed (2b8139af).

- [x] **P2 — major.** *Where:* section 2 Q1 (a) point 5; section 6 Step 3
  (`check.rs`), Step 4 (`certificate_rows`), Step 7. Code:
  [signing.rs:253-291](../../../../crates/roym_core/src/signing.rs#L253).
  *Problem:* the "dispatched but not listed" guard reads only `app.rs` and
  only lines shaped `"x.y" =>`. A hand-written arm in any other file, an
  `if req.method == ".."` test, or a third verb added to
  `handle_certificate_verb` (it matches on the suffix, in `roym_core`) is
  dispatched without a row, and the test still passes. `certificate_rows`
  is a second hand list that can drift from the handler. *Fix:* (1) Add
  typed handlers in `roym_core::signing`:
  `pub async fn signing_status<H: AppHost>(host: &H, _: NoParams) ->
  Reply<SigningStatusOutput>` and
  `pub async fn install_signing_certificate<H: AppHost>(host: &H, input:
  InstallCertificateInput) -> Reply<InstallCertificateOutput>`. List them as
  two normal rows in each service's `service_methods!`
  (`"profile.signing-status": signing::signing_status [NoParams =>
  SigningStatusOutput],`). `invoke` in profile and conversation no longer
  calls `handle_certificate_verb`. `handle_certificate_verb` stays for
  catalog, transaction and directory, and calls the same two functions.
  Delete `certificate_rows`. (2) The source check reads every `.rs` file
  under the service's `src/` except `bindings.rs`. It fails on any line
  matching `^"[a-z-]+\.[a-z-]+"\s*=>`, and on any use of `req.method` other
  than the single `Response::method_not_found(&req.method)` in `app.rs`.
  (3) It also collects every string literal in those files that matches
  `"<prefix><verb>"` for a prefix that `router::route` sends to profile or
  conversation, and fails when that name is not a row of the owning
  service's table. This also catches a sibling call to a method that does
  not exist. *Status:* Fixed (2b8139af).

- [x] **P3 — major.** *Where:* section 6 Step 2 (`shape`), D-A2a-14,
  section 7.2 item 5. *Problem:* the shape files have two holes. (1) An
  array keeps only the shape of its first item. A history page with a
  message, a membership item and a deleted message proves only the first.
  (2) Every string becomes `"string"`. A changed enum word (`rate-limited`
  to `rate_limited`, `incoming` to `Incoming`, `group`/`direct`) passes.
  The parity harness compares the two new builds with each other, so it
  cannot see this either. These are the two most likely mistakes when
  `json!` becomes a struct. *Fix:* (1) An array records the sorted,
  de-duplicated list of its item shapes. (2) A string records its value
  (not `"string"`) when its key is one of `kind`, `state`, `direction`,
  `body_encoding`, `admission`, `status`, `content_type`, `subject_kind`,
  `category`, `note`, or any key under `notices`. These are closed word sets
  or fixed texts. Every other value stays as its type name. *Status:* Fixed (2b8139af).

- [x] **P4 — major.** *Where:* section 2 Q8, D-A2a-12, section 4, section 6
  Steps 4 to 6. Code:
  [roym_transaction app.rs:576-596](../../../../crates/roym_transaction/src/app.rs#L576),
  [agreement_ops.rs:480-497](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L480),
  [listing_ops.rs:44](../../../../crates/roym_catalog/src/app/listing_ops.rs#L44),
  [roym_conversation app.rs:158-171](../../../../crates/roym_conversation/src/app.rs#L158).
  *Problem:* each site handles a failed sibling call in its own way, and
  moving to `call_sibling` changes that unless the plan says otherwise.
  (a) `send_card_and_file` puts the callee's `error.message` into
  `send_error`, which `request.*`, `quote.*` and `agreement.*` return to the
  client (`quote_ops.rs:143`, `request_ops.rs:114`). The planned
  `SiblingError::Refused` text adds ` (code N)`, so that wire text changes.
  (b) catalog treats a `profile.get` error as "no address" (`Ok(None)`). A
  `?` on `SiblingError` makes it an error. (c) A reply with no `result`
  gives an empty message id today, and `BadReply` after. (d) `contacts_map`
  ignores every error. Section 4 lists none of these. *Fix:* in Steps 5 and
  6, write the mapping for each site. `send_card_and_file` and the agreement
  site: every `Err` keeps the not-sent path, and `Refused(e)` uses
  `e.message` exactly. catalog: `Err(Refused(_))` maps to `Ok(None)`, and
  only `Transport` and `BadReply` become `Err`. `contacts_map`: any `Err`
  gives an empty map. `sync.rs`: `Refused(e)` gives
  `Response::internal_error(e.message)` as today. Add rows B17 to B20 to
  section 4 that say "same as today" for each. *Status:* Fixed (2b8139af).

- [x] **P5 — minor.** *Where:* section 6 Step 4; section 4; section 11.
  Code: [signing.rs:253](../../../../crates/roym_core/src/signing.rs#L253).
  *Problem:* the certificate verbs are shared by five services. Typing
  `install-signing-certificate` with `deny_unknown_fields` changes
  `catalog.`, `transaction.` and `directory.install-signing-certificate` in
  A2a too, and changes their error text. The plan does not say so. All
  current callers send only `certificate` (`roymctl` `signing.rs:159`,
  `substrate/tests/common/roym.rs:522`, parity `fixtures.rs:59-60` and
  `trust_fixtures.rs:123`), so nothing breaks. *Fix:* add a row to section
  4 and a line to section 11: the other three services get the typed verbs
  in A2a; their table rows come in A2b and A2c. *Status:* Fixed (2b8139af).

- [x] **P6 — minor.** *Where:* section 2 Q1, the shared types
  (`SigningStatusOutput`). Code:
  [record_signer.rs:52](../../../../crates/core/src/record_signer.rs#L52),
  [signing.rs:269](../../../../crates/roym_core/src/signing.rs#L269).
  *Problem:* `owner_did` is `Option<String>` and is written as `null` when
  absent. The plan lists it as `String`. That forces an `unwrap_or_default`,
  which changes `null` to `""`. The shape tests always run with an owner, so
  they would not see it. *Fix:* `owner_did: Option<String>`, with no
  `skip_serializing_if`. *Status:* Fixed (2b8139af).

- [x] **P7 — minor.** *Where:* section 2 Q1 row `block.check`; D-A2a-15.
  Code: [moderation.rs:174-185](../../../../crates/roym_profile/src/app/moderation.rs#L174).
  *Problem:* today the blocked answer writes `reason` and `since_secs` as
  `null` when absent, and the not-blocked answer is `{"blocked": false}`.
  D-A2a-15 puts "the two arms of `BlockCheckOutput`" among the
  `skip_serializing_if` fields, which would drop the nulls. Also, read back
  through `call_sibling` (`is_blocked`), an untagged enum whose first arm has
  only `Option` fields besides `blocked` matches `{"blocked": false}`.
  *Fix:* one struct, no untagged enum:
  `struct BlockCheckOutput { blocked: bool, #[serde(flatten)] since:
  Option<BlockedSince> }` with `struct BlockedSince { reason:
  Option<String>, since_secs: Option<u64> }` (no skip on these two). `None`
  writes only `blocked`. `Some` writes both keys, `null` when empty. This is
  an output type, so `flatten` does not meet `deny_unknown_fields`. Fix the
  D-A2a-15 text to match. *Status:* Fixed (2b8139af).

- [x] **P8 — minor.** *Where:* section 3.1; D-A2a-18; section 6 Step 4.
  Code: [conversation.rs:118-161](../../../../crates/roym_core/src/conversation.rs#L118),
  [conversation/group.rs:65](../../../../crates/roym_core/src/conversation/group.rs#L65).
  *Problem:* section 3.1 misses `MessageRow` (with `tombstone`, `is_deleted`
  and `sort_key`). No service uses it, and it has almost the fields of the
  new `MessageView`, so the plan would leave two message types. D-A2a-18
  leaves `GroupMeta` to the implementer. `GroupMeta` is used by
  `apply_group_profile` and its tests, and is not method input or output.
  *Fix:* Step 4 replaces `MessageRow` by `MessageView` (delete `MessageRow`
  and its helpers if only their own tests use them). Leave `GroupMeta` and
  `apply_group_profile` as they are, and remove them from D-A2a-18. When
  `ConversationListItem` replaces `ConversationRow`, use the host's types:
  `opened_at_secs` and `last_activity_ms` are `i64` (`created_at`,
  `last_activity_at` in the host `ConversationSummary`), not `u64`.
  *Status:* Fixed (2b8139af).

- [x] **P9 — minor.** *Where:* section 2 Q4; D-A2a-7; Step 3
  `parse_value`. *Problem:* serde's derived struct visitor also reads a JSON
  array, by position. `{"params": ["did:key:zA"]}` would parse as
  `ContactKeyInput`. Today an array means every field is absent. This is a
  new accepted wire form, not documented, and the probe does not check it.
  *Fix:* `parse_value` accepts only an object or `null`. Any other JSON type
  gives `-32602 "invalid params: expected an object"` with no `data`. Add
  the unit test `parse_refuses_an_array`. *Status:* Fixed (2b8139af).

- [x] **P10 — minor.** *Where:* D-A2a-6. *Problem:* serde's text contains
  the bad value (`invalid type: string "<the whole value>"`). A large string,
  or a pasted certificate, is copied back into the error and into logs.
  *Fix:* cut the message to 256 bytes on a char boundary, with the same
  rule as `envelope::truncate_method`
  ([envelope.rs:71](../../../../crates/roym_core/src/envelope.rs#L71)). Add a
  unit test with a 10 KB string. *Status:* Fixed (2b8139af).

- [x] **P11 — minor.** *Where:* section 2 Q4 ("four error kinds"); D-A2a-8;
  Step 7 test for `group.info`. *Problem:* sending both `group` and
  `conversation` is a fifth kind: serde says `duplicate field
  `conversation``, and the path is the parent. The planned helper gives no
  `data.field`, and the test checks only the code. *Fix:* treat
  `duplicate field` like `missing field` in the name rule (P1). Add it to the
  Step 3 unit tests. In Step 7, assert `data.field == "conversation"`.
  *Status:* Fixed (2b8139af).

- [x] **P12 — minor.** *Where:* section 6 Step 2 and Step 7. *Problem:*
  `fixtures.rs` uses `super::{helpers::*, trust_fixtures}`, so a new binary
  must include `fixtures.rs`, `helpers.rs`, `trust_fixtures.rs` and
  `trust_harness.rs`. Most of their functions are unused in a small binary,
  so `dead_code` warnings fail `clippy -- -D warnings`. "Add whatever the
  compiler asks for" does not say this. *Fix:* in Step 2, name the four
  `#[path]` modules and put `#[allow(dead_code)]` on each `mod` line.
  Simpler still: put the shape tests and the contract tests in one binary,
  `method_contract.rs`, with modules `shapes` and `contract`, so the
  includes are written once. *Status:* Fixed (2b8139af).

- [x] **P13 — minor.** *Where:* section 6 Step 7, the error tests; section
  7.2 item 3. *Problem:* in the WASM build the parse runs inside the guest.
  OF-14's answer must be the same in both builds, but the plan sends the
  error requests "through `/rpc`" without saying which build. *Fix:* send
  each error request to both `h.wasm_http` and `h.native_http`, assert the
  two bodies are equal, then assert the code and `data.field` (the pattern
  of parity scenario 35). *Status:* Fixed (2b8139af).

- [x] **P14 — minor.** *Where:* section 6 Steps 4, 5 and 6; section 7.4;
  section 9. *Problem:* facts to correct. (a) `dual_build_parity/profile.rs`
  has 1112 lines and is recorded as 1112 in
  [oversized-test-files.txt:23](../../../../xtask/oversized-test-files.txt#L23),
  not 1119. (b) `signing.rs` is 761 lines; with about 60 more it passes
  800, so the "if" in Step 4 will happen. (c) `docs/system-architecture.md`
  does not contain `require_internal`, so the grep in section 9 finds
  nothing. (d) The open point in Step 6: the substrate `roym_*` tests need
  no `--features roym` (no test is gated on it; they load the WASM
  artifacts). But they return early when `roym_artifacts_present()`
  ([common/roym.rs:60](../../../../crates/substrate/tests/common/roym.rs#L60))
  is false, so a green run can mean "skipped". *Fix:* (a) write 1112.
  (b) decide now: the new signing types and the two typed verb functions go
  in `crates/roym_core/src/signing/verbs.rs`, re-exported from
  `signing.rs`. (c) name the place: a short paragraph in the section
  "SynApp 1: Roym" of the architecture doc. (d) in the Step 6 proof, run
  `mise run build:roym` first and check in the log that the roym tests ran.
  *Status:* Fixed (2b8139af).

- [x] **P15 — minor.** *Where:* section 3.3. *Problem:* the list of callers
  does not name the demo stories and the Playwright helpers:
  `crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts`,
  `20-roym-setup.sh`, `lib.sh` and `crates/substrate/tests/e2e/hub-helpers.ts`.
  I checked them. The demos drive the Hub and `roymctl` (`roym enrol-signing`,
  `roym address`). `lib.sh`'s `rpc` sends `[]` by default, but only the
  greeter demo uses it. `addContact` sends no envelope. *Fix:* add them to
  section 3.3 as "checked, no change". *Status:* Fixed (2b8139af).

- [x] **P16 — minor.** *Where:* "How this is shown to an end user", third
  bullet. *Problem:* there is no `roymctl roym` verb that sends any method.
  A bare `curl` to `/rpc` also needs the Roym `Host:` header and a session.
  A developer cannot act on the bullet as written. *Fix:* give the full
  `curl` (with `-H "Host: <roym web host>"` and the session cookie from
  `roymctl session login`), or drop the bullet and keep the two test
  commands. *Status:* Fixed (2b8139af).

- [x] **P17 — minor.** *Where:* section 6 Step 5 ("`profile.get` reads the
  stored row into `ProfileRecord`"); Step 6 (`on_message`). Code:
  [profile_ops.rs:79-82](../../../../crates/roym_profile/src/app/profile_ops.rs#L79),
  [backup.rs:76-115](../../../../crates/roym_profile/src/app/backup.rs#L76),
  [inbox.rs:205](../../../../crates/roym_conversation/src/app/inbox.rs#L205).
  *Problem:* today `profile.get` returns the stored payload as it is. An
  imported profile row is stored as the bundle gave it, with only
  `envelope` checked. Reading it into `ProfileRecord` gives `-32603` for a
  row with no `record_id`. Also, `FirstContactCharge` stores `admission` as
  a string; a typed `AdmissionKind` must be written back as the same word.
  *Fix:* state two rules in Step 4: structs read from storage never use
  `deny_unknown_fields`; and the stored `admission` stays the kebab-case
  word. Add a section 4 row for the row-without-`record_id` case (only a
  hand-made bundle can make one), and keep the new `-32603`. *Status:* Fixed (2b8139af).

- [x] **P18 — minor.** *Where:* D-A2a-3; section 6 Steps 5 and 6.
  *Problem:* a method's code lives in up to four places: the macro line in
  `api.rs`, a struct in `api/<area>.rs`, the handler in `app/<area>.rs`,
  and sometimes a struct in `roym_core::method`. The `api/` tree mirrors
  `app/` and adds six files without adding a rule. *Fix:* keep `api.rs` for
  the macro list. Put each service-only struct in the handler's own file
  (`app/contacts.rs` and so on), next to the function that reads it. No file
  passes 800 lines this way: the largest, `app/messages.rs`, shrinks.
  *Status:* Fixed (2b8139af).

Count: 0 blockers, 4 major, 14 minor.

## Plan review round 2 — Claude Opus 5.5 (Claude Code), 2026-10-30

Checked the plan at `f6691087`. P1 to P18 are confirmed fixed in the plan
(`2b8139af`). The fix for P12 adds one new problem.

### Findings

- [x] **P19 — minor.** *Where:* section 6 Step 2 (the line "New test binary
  `crates/roym_web/tests/method_contract.rs` with `mod shapes;`") and Step 7
  (the line "Add `mod contract;`"). *Problem:* `tests/method_contract.rs`
  is the root file of a test crate. In a root file, `mod shapes;` makes Rust
  look for `tests/shapes.rs` (or `tests/shapes/mod.rs`), not
  `tests/method_contract/shapes.rs`. Cargo also builds every `.rs` file
  placed directly under `tests/` as its own test binary. So the plan as
  written either does not compile, or adds two extra test binaries.
  *Fix:* in both steps, write the module lines with a path, as the parity
  harness does in `crates/roym_web/tests/dual_build_parity.rs`:
  `#[path = "method_contract/shapes.rs"] mod shapes;` and
  `#[path = "method_contract/contract.rs"] mod contract;`. Name the two files
  `crates/roym_web/tests/method_contract/shapes.rs` and
  `crates/roym_web/tests/method_contract/contract.rs` in the step text.
  *Status:* Fixed (d0d7f172).

Count for this round: 0 blockers, 0 major, 1 minor.

Confirmed, 2026-10-30: P19 is fixed in the plan at `d0d7f172`. Steps 2 and
7 declare `shapes` and `contract` with `#[path = "method_contract/..."]`
and name both files under `crates/roym_web/tests/method_contract/`. No
module line without a path is left. No plan review item is open.

## Code review round 1 — Claude Opus 5.5 (Claude Code), 2026-10-30

Checked the code at `74ebebfa` on `feat/m08a-a2a-typed-methods` (PR
[#333](https://github.com/syneroym/syneroym/pull/333)), against the plan and
`main`. Line numbers below are at `74ebebfa`. The implementer was Gemini 3.8
Flash. No test suite was re-run here; CI on the PR runs it.

What was checked and found correct:

- **Method lists.** Both `api.rs` files list 24 rows, with the input and
  output types of the plan's Q1 tables. `invoke` in both services runs
  `admit::require_internal`, then `api::dispatch`, then
  `Response::method_not_found`. No hand-written arm and no
  `handle_certificate_verb` call is left in either service.
  `grep -rn "params\.get\|req\.params"` over both `src/` trees prints nothing.
- **The parser** ([method.rs:74](../../../../crates/roym_core/src/method.rs#L74)).
  `null` reads as `{}`. Any other non-object gives `-32602 "invalid params:
  expected an object"` with no `data`. The field rule for missing and
  duplicate fields (path joined with the quoted name) and for wrong type,
  wrong value and unknown field (the path) is as in D-A2a-6. The message is
  cut to 256 bytes on a char boundary. Unit tests cover the five error kinds
  at the top level and two levels down.
- **`deny_unknown_fields`.** Every input struct of the 48 rows has it, also
  `ImportInput`, `InstallCertificateInput` and `NoParams`. No struct read from
  storage has it (`ContactRow`, `ProfileRecord`, `BlockRow`, `ReportRow`,
  `FirstContactCharge`, `ContactLimits`, `MessageView`, `Bundle`). Stored
  `admission` is still the kebab-case word (`AdmissionKind::as_str`).
- **Output `Option` rules (P6, P7).** `SigningStatusOutput.owner_did` is
  `Option` with no skip. `BlockCheckOutput` uses the flattened
  `Option<BlockedSince>` with no skip inside. `MessageView` skips only `body`
  and `deleted_at_secs`. `AdmitFirstContactOutput` skips only
  `retry_after_secs`. These match the old `json!` code.
- **Closed word sets.** `KindFilter`, `ReportSubject`, `ReportCategory`,
  `AdmissionKind` and `StoredState::as_str` give the same words the old code
  accepted or wrote. `report.create` turns the enums back into the same
  strings before `content_digest`.
- **Shape files (P3).** `shape` records object keys, all item shapes of an
  array (sorted, de-duplicated) and the exact value for the closed-word keys
  of D-A2a-14 and anything under `notices`. All 48 methods have at least one
  case (63 files). The 63 files and `shapes.rs` were written in `72c2c49f` and
  are not changed by any later commit, so they come from the old code.
- **Sibling call sites (P4).** `call_sibling` sends the same request by the
  same route as the old `profile_call` and `sibling_call`:
  `host.call(CallTarget::Dependency(<service>), <interface>, "invoke",
  [<request>], None)`. Per site: `send_card_and_file` and `maybe_countersign`
  keep the not-sent path on every `Err`, and `Refused(e)` gives `e.message`
  with no ` (code N)`. `address_from_profile` maps `Refused` to `Ok(None)`.
  `contacts_map` gives an empty map on any `Err`. `fetch_sync_messages` gives
  `Response::internal_error(e.message)` on `Refused`. `resolve_open_address`
  keeps `Response::err(code, message)` for a refusal and the same transport
  text. A reply that has `"result": null` still reaches `Option<ProfileRecord>`
  as `None`, because `Response::ok(Value::Null)` writes the `result` key.
- **Parity harness.** 24 module files, 222 `fn scenario_`. In the scenario
  files only scenario 2 changed: its name and its last assertion
  (`-32602`, `data.field == "auth"`). `helpers.rs` changed too; see the
  answer below.
- **Hub (B1).** `contacts.ts` sends the trimmed envelope text and checks
  only that it parses. `contacts.test.ts` covers object, text, empty and
  address.
- **xtask.** Adding `serde_path_to_error` to `target_independent` in
  `check_roym_deps` is correct and minimal: `roym_core` builds for both
  targets, and the crate is pure Rust. The workspace and crate entries name
  `0.1`.
- **Rules.** No changed file passes 800 lines except the two parity files on
  the oversized list (`helpers.rs` was lowered to 1429 in the ratchet). No
  planning id (`A2a`, `M08`, `OF-`, `D-A2a`, `B<n>`, `P<n>`) is in code,
  comments or test names.
- **CI.** Run [41043929726](https://github.com/syneroym/syneroym/actions/runs/41043929726)
  on `74ebebfa` passed: "Lints, licenses & checks" (clippy, fmt, xtask
  checks), "Test 1/6" to "Test 6/6" and "Playwright E2E". "Benchmarks" and
  "Code Coverage" were skipped by the workflow. So no blocker comes from CI.
- **Docs.** The backlog rows of plan section 8 are all there. The task.md
  result line and the status.md row (63 shapes, 24 and 222, 11
  `method_contract` tests) are true.
  `docs/roym-integrated-experience-spec.md` has no `-32602` text, so it needed
  no change.

### Answer: why `helpers.rs` now installs a service-instance certificate

Only the test harness was wrong. Production behaviour did not change.

- The certificate was added in `72c2c49f` ("record method result shapes").
  That commit changes only test files (`git show --stat 72c2c49f`). So the
  old production code already needed it in this harness.
- The shape test needs one successful `group.add-member`. That method calls
  the host function `AppConversation::add_member`. The host fetches the new
  member's prekey bundle: `fetch_prekey_bundle` →
  `call_peer` → `peer_request` → `check_outbound_identity`
  ([transport.rs:83](../../../../crates/conversation/src/transport.rs#L83)).
  That check refuses when the service holds no instance certificate, and
  `fetch_prekey_bundle` turns the refusal into `InvalidArgument`, so the
  method answers an error.
- A real deploy installs the certificate
  ([deploy/commit.rs:120](../../../../crates/control_plane/src/service/orchestration/deploy/commit.rs#L120)).
  The `roym_web` parity harness on `main` never did. On `main`, no parity
  scenario adds a group member with success: the only `group.add-member` call
  is in `wire_origin.rs:119`, sent from off the node and expected to get
  `-32013`.
- `call_sibling` is not on this path. A sibling call is a guest-to-guest
  `host.call(CallTarget::Dependency(..))`, the same as before. The peer call
  is made by the host crate `syneroym-conversation`, which this PR does not
  change. So no sibling call goes by a different route or with a different
  identity.
- One side effect in the harness only: every service now holds a
  certificate, so the outbox delivery of every scenario now passes
  `check_outbound_identity` instead of ending with "no instance certificate".
  No scenario asserts that old path (scenario 56 sets `failed` with
  `notify_state`), and the parity run is reported green. R4 asks to record
  this.

Not done: no test was run to show the failure on `main`. The proof is the
commit content and the code path above.

### Findings

- [ ] **R1 — major.** *Where:*
  [contract.rs:20](../../../../crates/roym_web/tests/method_contract/contract.rs#L20)
  and [:32](../../../../crates/roym_web/tests/method_contract/contract.rs#L32);
  [check.rs:94](../../../../crates/roym_core/src/method/check.rs#L94).
  *Problem:* the OF-15 source scan never runs. The test passes the relative
  path `crates/roym_profile/src`. cargo and nextest run an integration test
  with the package directory (`crates/roym_web`) as the working directory,
  and `crates/roym_web/crates/roym_profile/src` does not exist.
  `scan_source_dir` then returns at once (`if !dir.exists() { return; }`),
  so the hand-written-arm, `req.method` and string-literal checks find
  nothing, whatever the source says. The Step 7 proof ("put a hand-written
  `"profile.get" =>` arm in `app.rs` and it fails") would not fail today.
  The unit tests hide this, because they pass `Path::new("/nonexistent")` and
  expect no panic from the scan. I ran the scan's rules by hand over the real
  directories: today they find no problem, so the fix should stay green.
  *Fix:* (1) In `contract.rs`, build the path from the manifest dir:
  `Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_profile/src")` (and
  `../roym_conversation/src`). (2) In `assert_service_table`, add a problem
  when `service_src_dir` does not exist, and another when no `.rs` file was
  read; do not return silently. Also report a `read_dir` error in
  `collect_rs_files` instead of ignoring it. (3) In `method/tests.rs`, give
  every test that does not test the scan a real empty temp directory in
  place of `/nonexistent`, and add `check_fails_when_the_source_dir_is_missing`.
  (4) Do the Step 7 manual proof once (add an arm, see the test fail, remove
  it) and say so in the PR. *Status:* Open.

- [ ] **R2 — minor.** *Where:* change doc
  [change.md:233-250](../../changes/roym-open-foundation/change.md#L233),
  section "Decisions"; also "Open questions" (`Resolved by D-A2a-2 and
  D-A2a-17`). *Problem:* the change doc has eighteen `D-A2a-*` decisions,
  but they are not the plan's eighteen. The numbers carry other meanings
  (for example change-doc D-A2a-3 is "Unknown fields policy", plan D-A2a-3 is
  "Where types live"; change-doc D-A2a-11 is "Dead types", plan D-A2a-11 is
  "Backup"). Some plan decisions are missing (D-A2a-3 where types live,
  D-A2a-11 backup, D-A2a-14 shape files, D-A2a-15 output rules), and the
  owner's answers to Q1–Q9 are not recorded, as plan section 9 asks. A reader
  who follows `D-A2a-2` from code review or the plan lands on a different
  decision. *Fix:* replace the eighteen bullets with the plan's D-A2a-1 to
  D-A2a-18, same numbers and same meaning, shortened if needed, and add one
  line with the owner's answers ("Q1–Q9: option (a), 2026-10-30"). Check
  that "Resolved by D-A2a-2 and D-A2a-17" still points at the method table
  and the test binary. *Status:* Open.

- [ ] **R3 — minor.** *Where:* change doc
  [change.md:262-267](../../changes/roym-open-foundation/change.md#L262),
  section "Deviations". *Problem:* (a) Only the `helpers.rs` bullet is a real
  deviation from the plan. The other five bullets are wire changes the plan
  planned (section 4 rows B1, B2, B4, B5, B7–B10); plan section 9 asked for
  them here, but mixed together a reader cannot tell "done differently from
  the plan" from "planned wire change". (b) The `helpers.rs` bullet gives no
  reason that a reader can check, and does not say that it changes the
  environment of all 222 scenarios. (c) Two planned wire changes are
  missing: B21 (the certificate verbs of `catalog`, `transaction` and
  `directory` now refuse unknown fields, and a missing `certificate` gives
  `missing field` text instead of `certificate is required`; this also holds
  for `<svc>.signing-status`, which now takes `NoParams`, and section 4 says
  only "install"), and B22 (`profile.get` answers `-32603` for a stored row
  with no `record_id`). (d) "Standardized on `-32602` with `data.field`" is
  not exact: a non-object `params` gets `-32602` with no `data`. *Fix:* split
  the A2a bullets under two labels. "Different from the plan": the
  `helpers.rs` bullet, rewritten as: "The `roym_web` parity harness did not
  install a service-instance certificate, unlike a real deploy
  (`deploy/commit.rs`). The shape test needs a successful
  `group.add-member`, whose host-side prekey fetch runs
  `check_outbound_identity` and refuses without one. The harness now
  installs one per service on both builds. This also lets outbox delivery in
  every parity scenario pass that check; no scenario asserted the old
  refusal. Production code is unchanged." "Planned wire changes (plan
  section 4)": the five current bullets, plus B21 (with `signing-status`)
  and B22, and with (d) made exact. *Status:* Open.

- [ ] **R4 — minor.** *Where:*
  [method.rs:91-97](../../../../crates/roym_core/src/method.rs#L91).
  *Problem:* the 256-byte cut applies only to `message`. `data.field` is the
  serde path, and for an unknown field the path ends with the key the client
  sent. A request with a 10 KB unknown key (`{"<10 KB>": 1}`) gets the whole
  key back in `data.field`, and in any log of the response. This is the
  reflection P10 meant to stop. *Fix:* cut `field` with the same
  `truncate_utf8(.., 256)` before it goes into `ParamError`, and add the
  unit test `unknown_field_name_in_data_is_cut_to_256_bytes`. *Status:* Open.

- [ ] **R5 — minor.** *Where:*
  [tests.rs:202-208](../../../../crates/roym_core/src/method/tests.rs#L202);
  tests at [:93-200](../../../../crates/roym_core/src/method/tests.rs#L93).
  *Problem:* (a) the truncation test uses only `"x"`, a one-byte char, so
  every cut point is a char boundary and
  `err.message.is_char_boundary(err.message.len())` is always true. The
  char-boundary half of the test cannot fail. (b) Q4 says "a unit test in
  Step 3 pins the real text". The tests only check `contains("invalid type")`
  and similar; none pins the full message, so the `invalid params: <path>:
  <text>` form (and the rule that a top-level error has no path part) is not
  tested. *Fix:* (a) build the long value from a multi-byte char (for
  example `"€".repeat(4000)`) so the 256-byte limit falls inside a char, and
  assert `message.len() <= 256` and that `message` ends with a whole `€`.
  (b) For one nested case and one top-level case, assert the whole message
  with `assert_eq!` (for example `invalid params: mid.deep: missing field
  `val_str``). *Status:* Open.

- [ ] **R6 — minor.** *Where:*
  [check.rs:40-51](../../../../crates/roym_core/src/method/check.rs#L40).
  *Problem:* two ways a method slips past the OF-15/OF-18 check. (a) The "no
  `Value`" rule reads only the outer type name, so `FooOutput { data: Value }`
  or `FooInput(Value)` passes. (b) The unknown-field probe tests only the top
  level, so a nested input struct without `deny_unknown_fields` (for example
  `struct FooInput { filter: Filter }` where `Filter` has no attribute)
  passes and accepts unknown nested keys. Today no row has either problem
  (the only nested input is `ImportInput.bundle: Bundle`, which by design has
  no attribute and carries `Value` sections inside a signed document). But
  OF-15 says the test "fails when one has no typed input or output", and
  the developer guide says the contract test verifies that "every method is
  typed". *Fix:* write the limits down where a method author reads them: a
  doc comment on `assert_service_table` naming what it checks and the two
  limits above, and one sentence in the developer guide's "Add a Roym
  method" step 1: "a nested input struct needs `deny_unknown_fields` too; the
  check sees only the outer type". No new code needed. *Status:* Open.

- [ ] **R7 — minor.** *Where:*
  [tests.rs:601](../../../../crates/roym_core/src/method/tests.rs#L601)
  (`call_sibling_with_test_host`); `TestHost::call` in `signing.rs` tests.
  *Problem:* the test checks only how replies are read. It does not check
  what `call_sibling` sends (the `Dependency` target, the interface, the
  `invoke` export, the `[{"method", "params"}]` envelope), which is the part
  that must equal the old `profile_call`. It also has no case for
  `{"result": null}` read as `Option<T>` = `None`, which `catalog`'s
  "no profile" path (B18) depends on. *Fix:* let `TestHost` record the
  arguments of its last `call`, and assert them in this test. Add a case:
  reply `{"result": null}` read as `Option<ProfileRecord>` gives `Ok(None)`.
  *Status:* Open.

- [ ] **R8 — minor.** *Where:*
  [system-architecture.md:1143](../../../system-architecture.md#L1143);
  [developer-guide.md:1687](../../../developer-guide.md#L1687), `:1689` and
  `:1702`. *Problem:* living docs state more than is true now. (a) "Each
  service owns one method table" and "Every Roym method uses typed input and
  output structs": only `profile` and `conversation` do; `catalog`,
  `transaction`, `directory` and `web` still read loose JSON. (b) Step 1 of
  "Add a Roym method" says to put `deny_unknown_fields` on the input struct,
  with no exception, but the next section says a wire-reachable method must
  ignore unknown fields. (c) "The error object includes `data`" is not true
  when `params` is not an object (no `data` then). *Fix:* (a) name the two
  services in both places ("Today `profile` and `conversation` have a method
  table; the other services still read their params by hand"). (b) Step 1:
  "on a local-only method; leave it off on a wire-reachable method, and list
  that method in the check's wire list". (c) Add: "When no field can be
  named, for example when `params` is not a JSON object, `data` is absent."
  *Status:* Open.

- [ ] **R9 — minor.** *Where:*
  [roym_conversation app.rs:85-89](../../../../crates/roym_conversation/src/app.rs#L85)
  (`serde_json::Value`, `serde_json::Value::as_str`);
  [roym_core conversation.rs:162-163](../../../../crates/roym_core/src/conversation.rs#L162)
  (`syneroym_app_host::types::conversation::Message`);
  [method.rs:82](../../../../crates/roym_core/src/method.rs#L82),
  [:195-196](../../../../crates/roym_core/src/method.rs#L195)
  (`serde_json::Map::new`, `std::any::type_name`);
  [contract.rs:14-32](../../../../crates/roym_web/tests/method_contract/contract.rs#L14)
  (`syneroym_roym_profile::api::method_table`, `std::path::Path::new`);
  `method/tests.rs` (`std::env::temp_dir`, `std::panic::catch_unwind`,
  `std::panic::resume_unwind`). *Problem:* inline paths with more than one
  `::`, against the AGENTS.md import rule. In `app.rs` the change removed
  the `Value` import and wrote the full path in its place. *Fix:* import the
  types (`use serde_json::{Map, Value};`,
  `use syneroym_app_host::types::conversation::Message;`, `use std::path::Path;`)
  and qualify functions by their parent module (`use std::{any, env, panic};`
  then `any::type_name`, `env::temp_dir`, `panic::catch_unwind`;
  `use syneroym_roym_profile::api;` then `api::method_table()`, with an alias
  for the conversation one). *Status:* Open.

- [ ] **R10 — minor.** *Where:*
  [check.rs:20-60](../../../../crates/roym_core/src/method/check.rs#L20).
  *Problem:* the comments `// 1. Duplicate name` … `// 4 & 5. Probe unknown
  fields` … `// 7, 8, 9. Source code scan` number the problem list of the
  plan's Step 3. The code has no list 1 to 9, so the numbers only make sense
  next to the plan, which gets archived ("comments explain why, not which
  plan"). *Fix:* drop the numbers. Put one doc comment on
  `assert_service_table` that lists, in words, what it checks (this can be
  the same comment as R6). *Status:* Open.

- [ ] **R11 — minor.** *Where:*
  [tests.rs:308](../../../../crates/roym_core/src/method/tests.rs#L308),
  `:369`, `:386`, `:405`. *Problem:* the scan tests write files into fixed
  names under the shared system temp directory (`test_correct_table`,
  `test_hand_written_arm`, …) and delete them with `remove_dir_all`. Two
  runs at the same time (two worktrees, or a nextest retry beside a running
  test) delete each other's files, and a test then passes or fails for the
  wrong reason. *Fix:* use `tempfile::TempDir` (already a workspace
  dependency, `tempfile = "3.10"`; add `tempfile.workspace = true` to
  `roym_core`'s `[dev-dependencies]`). The `TempDir` drop also replaces the
  `catch_unwind` / `resume_unwind` clean-up code. *Status:* Open.

- [ ] **R12 — minor.** *Where:*
  [method.rs:81-83](../../../../crates/roym_core/src/method.rs#L81).
  *Problem:* `parse_value` clones the whole `params` before it reads it.
  For `*.import` the params hold the whole backup bundle, so each import
  holds two copies in guest memory. `&Value` is already a serde
  `Deserializer`, so the copy is not needed. *Fix:* pass `params` (or a
  static empty map for `null`) by reference:
  `serde_path_to_error::deserialize(params)` with `params: &Value`, and for
  `null` a `static EMPTY` or a local `Value::Object(Map::new())` that is
  only built in that branch. Keep the existing tests green. *Status:* Open.

Count: 0 blockers, 1 major, 11 minor.

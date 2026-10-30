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

- [ ] **P19 — minor.** *Where:* section 6 Step 2 (the line "New test binary
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
  *Status:* Open.

Count for this round: 0 blockers, 0 major, 1 minor.

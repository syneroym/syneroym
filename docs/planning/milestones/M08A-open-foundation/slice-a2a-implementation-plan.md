# M08A slice A2a — Typed methods for profile and conversation: implementation plan

*Reader: the developer (or AI) who implements slice A2a. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan revised, 2026-10-30, incorporating plan review round 1.**
Steps in section 6 follow the agreed design.

**Size and PR.** L. One PR on `feat/m08a-a2a-typed-methods`, with the
commits of section 6 in that order. The branch starts from `main` at
`1d2b071d` (A1 is merged, PR #331). A2b and A2c are separate PRs and start
after this one is merged.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). In a worktree use `cargo xtask verify` with
    `CARGO_TARGET_DIR` set.
  - Functions of at most 100 lines. Match arms of at most 10 lines. Files
    of at most 800 lines.
  - No planning ids in code: no "M08A", "A2a", "OF-14" in code, comments or
    test names.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the ratchets (`check-file-lengths`, `check-duplication`).
  - Dependency versions name `major.minor` only.
  - Tests never hardcode a port. Use `SubstrateTestContext` or
    `common::alloc_ports::<N>()`.
- [task.md](task.md): section 1 (X2) and "A2 — Typed methods".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  OF-14, OF-15, OF-18, "Typed methods", the open question "Typed-method
  check".
- [deferred-backlog.md](../../deferred-backlog.md) section 10, the row
  "Roym's sibling boundary is untyped JSON".
- [slice-a1-implementation-plan.md](slice-a1-implementation-plan.md): the
  model for this plan.

### Terms used in this plan

- **Method:** one JSON-RPC verb a Roym service answers through its
  `invoke` export, for example `contacts.upsert`.
- **Input struct / output struct:** the Rust type that the `params` of a
  request are read into, and the Rust type the `result` is written from.
- **Local-only method:** a method that answers only calls made from inside
  this installation. Every method starts with `admit::require_internal`
  ([app.rs:55](../../../../crates/roym_profile/src/app.rs#L55),
  [app.rs:182](../../../../crates/roym_conversation/src/app.rs#L182)).
- **Wire-reachable method:** a method another node may call. Today only four
  `directory.*` methods are. A2a has none.
- **Unknown field:** a key in `params` that the input struct does not name.
- **Sibling call:** one Roym service calling another through
  `host.call(CallTarget::Dependency(..), .., "invoke", ..)`.
- **Certificate verbs:** `<service>.signing-status` and
  `<service>.install-signing-certificate`. In A2a, each service lists them as
  normal rows in `service_methods!`, handled by typed functions in
  `roym_core::signing::verbs`. Other services keep using
  `signing::handle_certificate_verb`.
- **Method table:** the list of every method one service answers, with the
  Rust type of its input and output. A test reads it.
- **Shape file:** a JSON file that records the key names, the JSON types,
  the null or absent state of a method's result, and the exact string values
  for closed word sets, but not other values.

## 1. What A2a builds, in one paragraph

A2a gives every method of the `profile` service (24, with the two
certificate verbs) and the `conversation` service (24) an input struct and
an output struct. A request with a wrong field type, a missing field or an
unknown field now gets `-32602`, and the error names the field. The shared
part X2 is built here once: the parser and its error, the `typed` wrapper
that every handler runs inside, the macro that writes both a service's
dispatch and its method table from one list, and the check that reads those
tables. A2b and A2c only add rows. The wire does not change for a correct
request. Only the text and the code of some wrong requests change.

## How this is shown to an end user

- An end user sees nothing new. Every screen and every step works as it
  did before.
- A developer can run `cargo nextest run -p syneroym-roym-web --test
  method_contract`. It lists every profile and conversation method and shows
  that each one has a typed input and a typed output, and that a wrong or
  unknown field gets `-32602` with the field name.
- A developer can run `cargo nextest run -p syneroym-roym-web --test
  method_contract shapes`. It shows that the JSON every method returns has the
  same keys and types as before the change.

**Demo:** none. A2a adds no command, no screen and no working path. The
change doc has no `demo:` list yet. A7 adds it at close-out.

## 2. Points for the owner

These are the open choices. Each has the options, a recommendation and the
reason. Section 6 follows the recommendations.

### Q1. The method table for OF-15: what it holds, where it lives, how the check finds every method

**What each service exports.** One row per method. A row has: the method
name, the input type, the output type, and whether the method is
local-only or wire-reachable. All 48 methods of A2a are local-only. The
tables are below. "core" means `crates/roym_core/src/method/<service>.rs`
or `roym_core::signing::verbs` (Q2). "svc" means in `src/app/<area>.rs` beside
the handler in the service crate. `Page` means the two optional fields
`offset: Option<u64>` and `limit: Option<u64>`. Every input struct has
`#[serde(deny_unknown_fields)]`. Every field marked `Option` may be absent
or `null`.

Shared by every service (in `roym_core`): `NoParams {}`,
`PingOutput { service: String }`, `ImportInput { bundle: Bundle }`
(`roym_core::backup`), `Bundle` as the output of every `*.export`,
`SigningStatusOutput { signing_did: String, pubkey_hex: String,
owner_did: Option<String>, certificate: CertificateStatus }`,
`InstallCertificateInput { certificate: String }`,
`InstallCertificateOutput { master_did: String, expires_at_secs: u64 }`.
The signing types and their typed handlers live in
`crates/roym_core/src/signing/verbs.rs` and are re-exported from
`roym_core::signing`.

**profile service (24 methods)**

| Method | Input | Output | Where |
| --- | --- | --- | --- |
| `profile.ping` | `NoParams` | `PingOutput` | core |
| `profile.policy` | `NoParams` | `PolicyOutput { statement: String, one_person_per_installation: bool, retention: String }` | svc |
| `profile.get` | `ProfileGetInput { person_did: Option<String> }` | `Option<ProfileRecord>` (`ProfileRecord { envelope: String, record_id: String, verified_at_secs: u64 }`) | core |
| `profile.set` | `ProfileSetInput { display_name: String, about: Option<String>, conversation_address: Option<String>, locale: Option<String> }` | `ProfileSetOutput { record_id: String, envelope: String }` | svc |
| `profile.export` | `NoParams` | `Bundle` | core |
| `profile.import` | `ImportInput` | `ProfileImportOutput { sections: usize, profiles_verified: usize }` | svc |
| `contacts.list` | `ContactsListInput { favourites_only: Option<bool>, offset, limit }` | `Vec<ContactRow>` | core |
| `contacts.get` | `ContactKeyInput { person_did: String }` | `Option<ContactRow>` | core |
| `contacts.upsert` | `ContactUpsertInput { person_did: String, profile_envelope: Option<String>, display_name: Option<String>, conversation_address: Option<String>, favourite: Option<bool> }` | `ContactUpsertOutput { person_did: String }` | svc |
| `contacts.remove` | `ContactKeyInput` | `ContactRemoveOutput { removed: String }` | svc |
| `contacts.resolve-address` | `ContactKeyInput` | `ResolveAddressOutput { conversation_address: String }` | core |
| `contacts.admit-first-contact` | `AdmitFirstContactInput { sender_address: String, sender_person_did: Option<String> }` | `AdmitFirstContactOutput { admission: AdmissionKind, blocked: bool, retry_after_secs: Option<u64> }` (`retry_after_secs` is absent unless `rate-limited`) | core |
| `contacts.limits` | `NoParams` | `ContactLimits` (exists, `safety.rs:20`) | core |
| `contacts.set-limits` | `ContactLimitsInput { window_secs: u64, max_per_window: u32 }` | `ContactLimits` | svc |
| `block.add` | `BlockAddInput { person_did: Option<String>, address: Option<String>, reason: Option<String> }` | `BlockAddOutput { key: String }` | svc |
| `block.remove` | `BlockTargetInput { person_did: Option<String>, address: Option<String> }` | `BlockRemoveOutput { removed: String }` | svc |
| `block.list` | `Page` | `Vec<BlockRow>` | svc |
| `block.check` | `BlockTargetInput` | `BlockCheckOutput` (`struct BlockCheckOutput { blocked: bool, #[serde(flatten)] since: Option<BlockedSince> }` where `BlockedSince { reason: Option<String>, since_secs: Option<u64> }`; `None` writes only `blocked: false`, `Some` writes both keys with `null` when empty) | core |
| `report.create` | `ReportCreateInput { subject_kind: ReportSubject, subject_id: String, category: ReportCategory, details: Option<String> }` | `ReportCreateOutput { report_id: String, status: String }` | svc |
| `report.list` | `ReportListInput { status: Option<String>, offset, limit }` | `Vec<ReportRow>` | svc |
| `report.get` | `ReportIdInput { report_id: String }` | `Option<ReportRow>` | svc |
| `report.withdraw` | `ReportIdInput` | `ReportCreateOutput` (same two fields) | svc |
| `profile.signing-status` | `NoParams` | `SigningStatusOutput` | core (`signing::signing_status`) |
| `profile.install-signing-certificate` | `InstallCertificateInput` | `InstallCertificateOutput` | core (`signing::install_signing_certificate`) |

**conversation service (24 methods)**

| Method | Input | Output | Where |
| --- | --- | --- | --- |
| `conversation.ping` | `NoParams` | `PingOutput` | core |
| `conversation.open` | `OpenInput { address: Option<String>, person_did: Option<String> }` | `OpenOutput { conversation_id: String, peer_address: String }` | svc |
| `conversation.list` | `ConversationListInput { kind: Option<KindFilter>, include_hidden: Option<bool>, offset: Option<u64>, limit: Option<u64> }` | `ConversationListOutput { conversations: Vec<ConversationListItem> }` | svc |
| `conversation.send` | `SendInput { conversation: String, body: String, content_type: Option<String> }` | `SendOutput { message_id: String, state: StoredState, sender_timestamp_ms: i64 }` | core |
| `conversation.history` | `HistoryInput { conversation: String, limit: Option<u32>, cursor: Option<String> }` | `HistoryOutput { messages: Vec<MessageView>, next_cursor: Option<String>, kind: ConversationRowKind }` | svc |
| `conversation.changes` | `ChangesInput { conversation: String, after_seq: Option<u64>, limit: Option<u32> }` | `ChangesOutput { messages: Vec<MessageView>, last_seq: u64, kind: ConversationRowKind }` | core |
| `conversation.delivery-status` | `MessageIdInput { message_id: String }` | `DeliveryStatusOutput { state: StoredState }` | svc |
| `conversation.outbox` | `NoParams` | `OutboxOutput { outbox: Vec<OutboxItem> }` | svc |
| `conversation.retry` | `MessageIdInput` | `RetryOutput { retried: bool }` | svc |
| `conversation.delete-message` | `DeleteMessageInput { message_id: String, ask_peer: Option<bool> }` | `DeleteMessageOutput { deleted: String, asked_peer: bool, note: String }` | svc |
| `conversation.search` | `SearchInput { query: String, conversation: Option<String>, limit: Option<u32>, kind: Option<KindFilter> }` | `SearchOutput { matches: Vec<MessageView> }` | svc |
| `conversation.transcript-digest` | `ConversationRef { conversation: String }` (accepts the key `group` too) | `TranscriptDigestOutput { digest: String, rows: u64 }` | svc |
| `conversation.export` | `NoParams` | `Bundle` | core |
| `conversation.import` | `ImportInput` | `ConversationImportOutput { imported: ImportCounts { messages, admissions } }` | svc |
| `group.create` | `GroupCreateInput { name: Option<String> }` | `GroupCreateOutput { conversation_id: String, owner_address: String }` | svc |
| `group.rename` | `GroupRenameInput { conversation: String (also `group`), name: String }` | `GroupRenameOutput { renamed: bool, name: String }` | svc |
| `group.add-member` | `GroupMemberInput { conversation: String (also `group`), address: Option<String>, person_did: Option<String> }` | `GroupAddOutput { added: String, epoch: u64 }` | svc |
| `group.remove-member` | `GroupMemberInput` | `GroupRemoveOutput { removed: bool }` | svc |
| `group.info` | `ConversationRef` | `GroupInfoOutput` (the 14 keys of [group.rs:148-170](../../../../crates/roym_conversation/src/app/group.rs#L148)) | svc |
| `group.sync` | `ConversationRef` | `GroupSyncOutput { synced: bool }` | svc |
| `group.hide` | `ConversationRef` | `AdmissionOutput { admission: AdmissionState { state: String } }` | svc |
| `group.unhide` | `ConversationRef` | `GroupUnhideOutput { admission: AdmissionState, filled_in: u64 }` | svc |
| `conversation.signing-status` | `NoParams` | `SigningStatusOutput` | core (`signing::signing_status`) |
| `conversation.install-signing-certificate` | `InstallCertificateInput` | `InstallCertificateOutput` | core (`signing::install_signing_certificate`) |

`MessageView` is the message object that `message_to_json`
([messages.rs:191](../../../../crates/roym_conversation/src/app/messages.rs#L191))
writes today: `id`, `conversation`, `author`, `direction` (`Direction`),
`sender_timestamp_ms`, `content_type`, `body_encoding` (`BodyEncoding`),
`state` (`StoredState`), `last_error: Option<String>` (written as `null`),
`stored_at_secs`, `refused: Option<String>` (written as `null`),
`restored`, `visible_seq`, `verified`, and two keys that are absent unless
set: `deleted_at_secs` and `body`. `ConversationListItem` is the row of
[messages.rs:106-124](../../../../crates/roym_conversation/src/app/messages.rs#L106),
with `opened_at_secs: i64` and `last_activity_ms: i64` matching the host types.

**How the check test finds every method.** Options:

- (a) **One macro writes the dispatch and the table.** Each service lists its
  methods once, one line each:
  `"profile.get": profile_ops::get [ProfileGetInput => Option<ProfileRecord>],`.
  The two certificate verbs are normal rows in that list, calling typed
  handlers in `roym_core::signing::verbs` (`signing_status` and
  `install_signing_certificate`).
  The macro (`roym_core::service_methods!`) writes `dispatch(host, req) ->
  Option<Response>` and `method_rows() -> Vec<MethodInfo>` from that list.
  A method cannot be dispatched without a row, and a row cannot exist without
  an input and an output type. `invoke` in profile and conversation has no
  `match` on method names and no call to `handle_certificate_verb`.
  `certificate_rows` is deleted. The check test then proves five things:
  1. Names are unique, and `router::route(name)` gives this service.
  2. No input or output type is `serde_json::Value` (read from
     `std::any::type_name`).
  3. A row not in the service's wire list refuses an unknown field. A row in
     it ignores one. The test probes the row's own parser with
     `{"__probe__": 1}`.
  4. Every method is refused off the node with `-32013`, before any parsing.
  5. The source check reads every `.rs` file under the service's `src/`
     except `bindings.rs`. It fails on any line matching
     `^"[a-z-]+\.[a-z-]+"\s*=>`, and on any use of `req.method` other
     than the single `Response::method_not_found(&req.method)` in `app.rs`.
     It also collects every string literal in those files that matches
     `"<prefix><verb>"` for a prefix that `router::route` sends to the
     service, and fails when that name is not a row of the service's table.
- (b) **A hand-written table plus a source scan.** Simple, but nothing makes
  the author add the row. Only the scan catches it, and only after the fact.
- (c) **An enum of methods.** No macro, and the compiler checks every arm.
  But it needs four lists to edit for each new method (variant, `ALL`, name,
  handler), and `ALL` can still miss a variant.

**Recommendation: (a).** One list per service, one line per method, and the
compiler refuses a method with no types. The macro is small (about 25 lines).
It is the one place in the workspace where a macro removes real duplication.
`unverified`: whether `clippy::too_many_lines` fires on the expanded `dispatch`
(the macro is defined in another crate, so clippy should skip it). If it
fires, split the list into two macro calls in that service.

### Q2. Where the shared X2 code and each struct live

- (a) **The owner's rule, applied one struct at a time.** X2 code lives in
  `crates/roym_core/src/method.rs` and `method/`. Shared signing types and
  verb handlers live in `crates/roym_core/src/signing/verbs.rs`. A struct lives
  in `roym_core` when a sibling service or a client (`roymctl`) also uses it.
  Otherwise it lives in the handler's file in the service crate
  (`src/app/<area>.rs`).
- (b) **Every struct in `roym_core`**, one file per service, as the single
  place a client author reads.

**Recommendation: (a).** It keeps `roym_core` (which every guest builds)
small, and a struct has one owner. The "Where" column of Q1 is the result.
The structs in `roym_core` are those the sibling calls need: the profile
methods that `conversation` and `catalog` call, and `conversation.send` and
`conversation.changes`, which `transaction` calls. The 48 methods still have
one list each, in `api.rs` of their service, so a reader finds all of a
service's methods in one file. Service-only structs live directly beside
the functions that read them in `src/app/<area>.rs`, avoiding extra files.
`roymctl` and the tests send raw JSON on purpose (D-A2a-13), so no client
uses a struct yet.

### Q3. The unknown-field rule: build the "ignore" mode now, or leave it to A2c

Both services are local-only. Every A2a input uses `deny_unknown_fields`.

- (a) **Build the rule and its check now, with a unit test.** The "ignore"
  mode needs no serde code: a struct without `deny_unknown_fields` already
  ignores. What A2a builds is the check: a row named in a service's wire
  list must ignore unknown fields. The unit test uses two synthetic types,
  one of each kind, in `roym_core`.
- (b) **Leave all of it to A2c.** A2a only checks that every row refuses.

**Recommendation: (a).** X2 is "the rule, defined once". If A2a leaves the
wire half out, A2c has to decide it again, and the check then has two
authors. The cost is about 30 lines and one test. No wire-reachable method
exists in A2a, so no production code uses the wire branch yet. Say so in
the doc comment.

### Q4. How `-32602` names the bad field, and what `data` carries

serde's own text for a wrong type is `invalid type: integer `5`, expected a
string`. It does not name the field. OF-14 needs the field name.

- (a) **`serde_path_to_error`** (version `0.1`, already in `Cargo.lock` as a
  dependency of `axum`, pure Rust, builds for `wasm32-wasip2`). It records
  the path of the failing field. Message: `invalid params: favourite:
  invalid type: string "yes", expected a bool`. `data`: `{"field":
  "favourite"}`. `parse_value` accepts only a JSON object or `null`. An array
  or other JSON type gives `-32602 "invalid params: expected an object"` with
  no `data`. For a missing field or duplicate field, serde puts the quoted
  name in backticks and the path is the parent: `field` is the path joined
  with the quoted name (only the quoted name when the path is `.`). For a
  wrong type or unknown field, `path` already ends with the bad field itself.
  Error messages are truncated to 256 bytes on a char boundary.
- (b) **Map serde's text by hand.** No new dependency, but wrong-type errors
  would still have no field name.

**Recommendation: (a).** Five error kinds, all with `data.field`: wrong
type, wrong value (for example a number out of range or an unknown enum
word), missing field, unknown field, duplicate field. When no field can be
named (for example `params` is a string or array), `data` is absent. A unit
test in Step 3 pins the real text.

### Q5. Output structs: does every output become a struct

- (a) **Every output is a named type. The wire shape does not change.** Most
  are structs. Three methods return a bare JSON array (`contacts.list`,
  `block.list`, `report.list`). Their output is `Vec<Row>`. Three return
  `null` when the thing is missing (`profile.get`, `contacts.get`,
  `report.get`). Their output is `Option<Row>`. Two return a signed bundle
  (`*.export`). No method returns nothing: every "nothing" method already
  returns an object such as `{"retried": true}`.
- (b) **Wrap the three arrays in an object**, for example `{"contacts": [..]}`,
  so each list can gain fields later.

**Recommendation: (a), and record (b) as a backlog row.** (b) changes the
wire and breaks the Hub, `roymctl` and the sibling that reads
`contacts.list`. A2 does not rename or reshape methods. A bare array cannot
gain a next-cursor, so M8 will want (b) the first time a list needs one.
Doing it then costs the same as doing it now.

### Q6. A method with no input that gets unknown fields (parity scenario 2)

`profile.policy` is public (no session) and local-only. Parity scenario 2
([profile.rs:35](../../../../crates/roym_web/tests/dual_build_parity/profile.rs#L35))
sends it `{"caller": .., "person_did": .., "auth": ..}` and asserts it
still answers with the policy text. This shows that identity claims in
`params` change nothing.

- (a) **`NoParams` refuses unknown fields** (OF-18 as written). The scenario
  keeps its name and its two-build comparison. Its last assertion changes
  from "the result has `statement`" to "both builds answer `-32602` with
  `data.field = "auth"`". The claim is still inert: it is now refused.
- (b) **`NoParams` ignores unknown fields.** The scenario stays unchanged,
  but "local-only methods refuse unknown fields" (OF-18) gets one exception,
  and the check needs a list of exceptions.

**Recommendation: (a).** OF-18 has no exceptions. The scenario count (222)
and the module count (24) stay. One assertion line changes. Record it in
the change doc's Deviations. Without this change the plan cannot meet OF-18.

### Q7. Closed sets of words: `String` or `enum`

`report.create` accepts `subject_kind` in `person|listing|message` and
`category` in five words. `conversation.list` and `conversation.search`
accept `kind` in `direct|group` (today `list` ignores any other word and
`search` answers `-32602 "unknown kind"`).

- (a) **Enums** (`ReportSubject`, `ReportCategory`, `KindFilter`). A schema
  can later list the allowed words. A wrong word answers `-32602` with
  `unknown variant`. The stored `ReportRow` keeps its `String` fields, so
  stored data and `content_digest` do not change.
- (b) **Keep `String`** and keep today's checks in the handler.

**Recommendation: (a).** This is what typing is for. Only the error text for
a wrong word changes, and `conversation.list` now refuses a wrong `kind`
instead of ignoring it. No caller sends a wrong word (section 4).

### Q8. The five caller sites in the `catalog` and `transaction` crates

`catalog` calls `profile.get` ([listing_ops.rs:32](../../../../crates/roym_catalog/src/app/listing_ops.rs#L32)).
`transaction` calls `conversation.send` twice and `conversation.changes` once.
They build `json!` by hand today. They belong to A2b, but the structs they
should use are made in A2a.

- (a) **A2a changes the five sites** to build the typed struct through one
  shared helper, `roym_core::method::call_sibling`. It replaces
  `profile_call` in `conversation` too. A2b converts the rest of
  `transaction` and `catalog`.
- (b) **A2a leaves them as `json!`.** A rename of a field in `SendInput` then
  breaks `transaction` only at run time, and only a test sees it.

**Recommendation: (a).** The point of typing is that the compiler sees a
sibling drifting. Leaving the five sites out leaves the exact bug the
backlog row names. The edits are small (five call sites). Each site preserves
its error-handling semantics as today (section 4 B17 to B20).
`roym_transaction` keeps its own `sibling_call` for its calls to `catalog`
until A2b. Add a backlog row for that (section 8).

### Q9. The Hub sends an object where the service reads a string

The Hub's contact form sends `profile_envelope` as a parsed JSON object
([contacts.ts:43-48](../../../../crates/roym_web/ui/src/screens/contacts.ts#L43)).
The service reads a string (`as_str()`,
[contacts.rs:148](../../../../crates/roym_profile/src/app/contacts.rs#L148)).
Today an object is **silently ignored**: the contact is saved with no
profile and no signature check, and the call fails with
`conversation_address is required` if the address field is empty. After A2a
it answers `-32602` with `data.field = "profile_envelope"`.

- (a) **Fix the Hub** in the same commit as the profile conversion. It sends
  the pasted text as a string, after checking that it parses as JSON.
- (b) **Make the service accept an object as well** as a string.

**Recommendation: (a).** The record in a profile row is a string
everywhere else (`envelope` in `ProfileRecord`, every test). Two accepted
shapes for one field would be a second thing to document and to test. The
current behaviour is a Hub bug: it skips the check the form promises.

## 3. Facts checked in the code

All paths are from the repository root. Line numbers are at `1d2b071d`.

### 3.1 How a request reaches a handler today

- `Request { method: String, params: Value }`, `params` has
  `#[serde(default)]`, so a missing `params` is `Value::Null`
  ([envelope.rs:14-18](../../../../crates/roym_core/src/envelope.rs#L14)).
  A typed parser must read `null` as `{}`.
- `Response::invalid_params` is `-32602`. `RpcError` has an optional `data`
  and `Response::with_data` sets it ([envelope.rs:62](../../../../crates/roym_core/src/envelope.rs#L62),
  [envelope.rs:50](../../../../crates/roym_core/src/envelope.rs#L50)).
- `web` forwards the callee's whole `RpcError`, `data` included
  ([app.rs:210-215](../../../../crates/roym_web/src/app.rs#L210)). The Hub's
  `RpcError` class keeps `data` ([rpc.ts:20](../../../../crates/roym_web/ui/src/rpc.ts#L20)).
- Both services: `invoke` runs `admit::require_internal`, then
  `signing::handle_certificate_verb`, then a `match` on 22 names
  ([profile app.rs:54-87](../../../../crates/roym_profile/src/app.rs#L54),
  [conversation app.rs:181-213](../../../../crates/roym_conversation/src/app.rs#L181)).
  Each handler reads fields with `req.params.get("x").and_then(as_str)`.
  A value of the wrong type is silently read as "absent". 69 such
  lines are in the two crates (`grep -rn "params\.get" crates/roym_profile/src
  crates/roym_conversation/src`).
- The `status` export (a WIT function, not a JSON-RPC method) returns
  `{service, schema_version}` as a string. A2a does not touch it.
- The transaction service already parses typed inputs at 24 sites
  (`request_ops.rs:50` and others) with `serde_json::from_value` and the
  message `invalid params: {e}`. None uses `deny_unknown_fields` and none
  names the field. A2b replaces them.
- `serde_path_to_error` 0.1.20 is in `Cargo.lock`. `deny_unknown_fields` is
  used in `fdae`, `passport` and config types, not in Roym.
- `ConversationRow` and `MessageRow`
  ([conversation.rs:19-42, :118-161](../../../../crates/roym_core/src/conversation.rs#L19))
  are not used by any service. `MessageRow` is replaced by `MessageView`,
  and `ConversationRow` is replaced by `ConversationListItem`. `GroupMeta`
  is used by `apply_group_profile` and its tests, and remains as an internal
  domain type. `StoredState`, `Direction` and `BodyEncoding` are used and
  have the right kebab-case words.

### 3.2 Methods, handlers and what they read

| Method | Handler | Reads from `params` (and the line) |
| --- | --- | --- |
| `profile.ping` | `app.rs:64` | nothing |
| `profile.policy` | `profile_ops.rs:21` | nothing |
| `profile.get` | `profile_ops.rs:62` | `person_did` (:66) |
| `profile.set` | `profile_ops.rs:221`, reads at :94-101 | `display_name`, `about`, `locale`, `conversation_address` |
| `profile.export` / `.import` | `backup.rs:33` / `:142` | import: `bundle`, or the whole `params` as a bundle (:143) |
| `contacts.list` | `contacts.rs:42` | `favourites_only`, `offset`, `limit` |
| `contacts.get` / `.remove` / `.resolve-address` | `:68` / `:214` / `:228` | `person_did` |
| `contacts.upsert` | `:138` | `person_did`, `profile_envelope`, `display_name`, `conversation_address`, `favourite` |
| `contacts.admit-first-contact` | `:246` | `sender_person_did`, `sender_address` |
| `contacts.limits` / `.set-limits` | `:341` / `:348` | set: `window_secs`, `max_per_window` (`as_u64() as u32`) |
| `block.add` / `.remove` / `.check` | `moderation.rs:49` / `:94` / `:157` | `person_did`, `address` (+ `reason` for add) |
| `block.list` | `:132` | `offset`, `limit` |
| `report.create` | `:188` | `subject_kind`, `subject_id`, `category`, `details` |
| `report.list` / `.get` / `.withdraw` | `:291` / `:319` / `:337` | `status`, `offset`, `limit` / `report_id` |
| `conversation.open` | `messages.rs:46`, address at :29-34 | `address`, else `person_did` |
| `conversation.list` | `:61` | `offset`, `limit`, `kind`, `include_hidden` |
| `conversation.send` | `:133` | `conversation`, `body`, `content_type` |
| `conversation.history` | `:269` | `conversation`, `limit`, `cursor` (a non-string is ignored, an empty string means none) |
| `conversation.changes` | `:304` | `conversation`, `after_seq`, `limit` |
| `conversation.delivery-status` / `.retry` | `:327` / `:358` | `message_id` |
| `conversation.outbox` | `:338` | nothing |
| `conversation.delete-message` | `:383` | `message_id`, `ask_peer` |
| `conversation.search` | `:423` | `query` (empty is refused), `conversation`, `limit`, `kind` |
| `conversation.transcript-digest` | `:472` | `conversation`, else `group` |
| `conversation.export` / `.import` | `backup.rs:20` / `:62` | import: `bundle`, or the whole `params` |
| `group.create` | `group.rs:27` | `name` |
| `group.rename` | `:57` | `conversation` or `group`; `name` |
| `group.add-member` / `.remove-member` | `:76` / `:106` | `conversation` or `group`; `address`, else `person_did` |
| `group.info` / `.sync` / `.hide` / `.unhide` | `:121` / `:173` / `:187` / `:201` | `conversation` or `group` |
| certificate verbs | `signing.rs:253-291` | install: `certificate` |

### 3.3 Callers (what each sends)

- **Hub (`crates/roym_web/ui/src`).** `messages.ts`: `conversation.open`
  `{address}` :140, `conversation.search` `{query, kind:"direct"}` :165,
  `conversation.list` `{kind:"direct"}` :194, `conversation.send`
  `{conversation, body}` :248, `conversation.history` `{conversation}` :351,
  `conversation.retry` `{message_id}` :514, `conversation.delete-message`
  `{message_id, ask_peer}` :870. `groups.ts`: `group.create` `{name}` or `{}`
  :224, `conversation.list` `{kind:"group"[, include_hidden:true]}` :262-264,
  `group.unhide` :295, `group.info` :343, `conversation.transcript-digest`
  :356, `contacts.list` :365, `conversation.send` :419, `group.rename` :460,
  `group.remove-member` `{conversation, address}` :499, `group.add-member`
  `{conversation, person_did}` or `{conversation, address}` :531-533,
  `group.sync` :571, `group.hide` :586, `conversation.history` :606,
  `conversation.delete-message` :617, `conversation.retry` :627.
  `contacts.ts`: `contacts.list` :18, `contacts.upsert` :66 (**sends an
  object**). `profile.ts`: `profile.set` :40, `profile.get` :54. `safety.ts`:
  `profile.policy` :16, `block.add` :46-48, `block.list` :63,
  `report.list` :132, `report.withdraw` :142, `report.create` :162,
  `contacts.limits` :204, `contacts.set-limits` :220. `backup.ts`:
  `<svc>.export` and `<svc>.import` with `{bundle}` :8-15, :84, :101.
  `enrolment.ts:14`: `<svc>.signing-status`. `main.ts:282`: `profile.ping`.
  The Hub never calls `conversation.changes`, `conversation.outbox`,
  `conversation.delivery-status`, `block.remove`, `block.check`,
  `contacts.get`, `contacts.remove`, `contacts.resolve-address`,
  `contacts.admit-first-contact`, `report.get`.
- **`roymctl`** (`apps/roymctl/src/commands/roym/`). `group.rs:152-224`:
  `group.create`, `group.rename`, `group.add-member` and `group.remove-member`
  (both `address` and `person_did` may be sent together), `conversation.list`
  `{kind:"group", include_hidden}`, `group.info`, `conversation.send`,
  `conversation.history` `{conversation[, limit]}`, `group.sync`,
  `conversation.transcript-digest`, `group.hide`, `group.unhide`.
  `signing.rs:84, :123, :158`: the certificate verbs.
  `backup.rs:213, :284`: `<svc>.export` with `{}` and `<svc>.import` with
  `{bundle}`.
- **Sibling services (Rust).** `conversation` to `profile`:
  `contacts.list {}` ([app.rs:160](../../../../crates/roym_conversation/src/app.rs#L160)),
  `contacts.resolve-address {person_did}` ([messages.rs:35](../../../../crates/roym_conversation/src/app/messages.rs#L35)),
  `block.check {address, person_did}` where `person_did` may be `null`
  ([inbox.rs:116](../../../../crates/roym_conversation/src/app/inbox.rs#L116)),
  `contacts.admit-first-contact {sender_address, sender_person_did}`
  ([inbox.rs:185](../../../../crates/roym_conversation/src/app/inbox.rs#L185),
  [:281](../../../../crates/roym_conversation/src/app/inbox.rs#L281)).
  `catalog` to `profile`: `profile.get {}` (`listing_ops.rs:32`).
  `transaction` to `conversation`: `conversation.send {conversation, body,
  content_type}` ([app.rs:578](../../../../crates/roym_transaction/src/app.rs#L578),
  [agreement_ops.rs:482](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L482)),
  `conversation.changes {conversation, limit, after_seq}`
  ([sync.rs:126](../../../../crates/roym_transaction/src/app/sync.rs#L126)).
  Each reads the answer with `.get("key")`, so an output with the same keys
  needs no change on the reading side.
- **`web`.** Forwards `params` unchanged ([app.rs:148, :193](../../../../crates/roym_web/src/app.rs#L148)).
- **Parity harness** (`crates/roym_web/tests/dual_build_parity/`, 24 files,
  222 `fn scenario_`). About 250 lines name these methods, in `admission.rs`,
  `conversation.rs`, `group.rs`, `group_lifecycle.rs`, `profile.rs`,
  `wire_origin.rs`, `fixtures.rs`, `catalog.rs`, `transaction*.rs`.
- **Substrate e2e** (`crates/substrate/tests/`): `roym_conversation_e2e.rs`,
  `roym_group_e2e.rs`, `roym_group_offline_e2e.rs`, `roym_refusal_e2e.rs`,
  `roym_restore_e2e.rs`, `roym_identity_e2e.rs`, `roym_trust_e2e.rs`,
  `roym_directory_e2e.rs`, `roym_booking_e2e.rs`, `roym_transaction_e2e.rs`,
  `roym_app_e2e.rs`, and the helpers `common/roym.rs`, `common/roym_flow.rs`,
  `common/roym_group.rs`. A throw-away script (kept out of the repository) listed the params of every
  call with a literal method name in the parity and e2e files. Calls that
  build the method name at run time (`format!("{svc}.import")`, the
  `for method in [..]` loops at `profile.rs:706` and `:722`) were read by hand.
- **Playwright** (`crates/substrate/tests/e2e/tests/roym-hub.spec.ts`,
  `roym-groups.spec.ts`) drives the Hub.
- **Demo stories and Playwright helpers**
  (`crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts`, `20-roym-setup.sh`,
  `lib.sh`, and `crates/substrate/tests/e2e/hub-helpers.ts`). Checked, no
  change. The demos drive the Hub and `roymctl` (`roym enrol-signing`,
  `roym address`). `lib.sh`'s `rpc` sends `[]` by default, but only the
  greeter demo uses it. `addContact` sends no envelope.

## 4. Behaviour-change audit

A correct request gets the same answer after A2a. These are the requests
that do not, found by reading every call site of section 3.3 and the
shape inventory above.

| # | What is sent today | What the service does today | After A2a | How the plan handles it |
| --- | --- | --- | --- | --- |
| B1 | Hub `contacts.upsert` with `profile_envelope` as an **object** ([contacts.ts:43-48](../../../../crates/roym_web/ui/src/screens/contacts.ts#L43)) | Ignores it (`as_str()` is `None`). Saves the contact with no profile and no check. | `-32602`, `data.field = "profile_envelope"` | Step 5 changes the Hub to send the text (Q9). A unit test covers it. |
| B2 | Parity scenario 2: `profile.policy` with `caller`, `person_did`, `auth` | Answers the policy text. | `-32602`, `data.field = "auth"` (the first unknown key in sorted order) | Step 5 changes that one assertion (Q6). |
| B3 | `group.*` and `conversation.transcript-digest` with the key `group` (all `group` e2e tests, [roym_group_e2e.rs](../../../../crates/substrate/tests/roym_group_e2e.rs), `roym_group_offline_e2e.rs`, parity `group_lifecycle.rs:438`) | `group` is read when `conversation` is absent. | The same. The field has `#[serde(alias = "group")]`. | Kept (D-A2a-8). A test covers both keys, and both together. |
| B4 | A required field is missing, for example `contacts.upsert {}` | `-32602 "person_did is required"` | `-32602 "invalid params: missing field `person_did`"`, `data.field = "person_did"` | The code stays. The text changes. No test or caller reads the old texts (checked by grep over `roym_web/tests`, `substrate/tests`, `roym_web/ui`, `apps/roymctl`). |
| B5 | A field of the wrong type, for example `about: 5`, `limit: "10"`, `ask_peer: "yes"`, `include_hidden: 1`, `cursor: 5` | Silently read as absent. | `-32602` with `data.field` | No caller sends one (every literal call in the parity and e2e files, the Hub, `roymctl`). A new test sends two (section 7). |
| B6 | `max_per_window` above `u32::MAX` | Cut to 32 bits without a message. | `-32602` | No caller does this. |
| B7 | `conversation.list` with a `kind` other than `direct` or `group` | The word is ignored. All conversations are listed. | `-32602 unknown variant` | No caller does this (Q7). |
| B8 | `report.create` with a wrong `subject_kind` or `category` | `-32602` with the old sentence. | `-32602 unknown variant`, `data.field` | Q7. Only the text changes. |
| B9 | `*.import` with the bundle as the whole `params` (no `bundle` key) | Read as the bundle. | `-32602 missing field `bundle`` | No caller does this. Every caller sends `{bundle}` (Hub `backup.ts:101`, `roymctl backup.rs:284`, all tests). |
| B10 | A bad `bundle` (not a bundle at all) | `-32602 "invalid bundle: .."` | `-32602 "invalid params: bundle: .."` | Only the text changes. Parity `conversation.rs:371, :385` and e2e `roym_conversation_e2e.rs:291` assert the code `-32602` only. |
| B11 | `profile.set` with invalid input **and** no signing certificate | `-32602 "signing-not-enrolled"` (the certificate check runs first) | `-32602` for the bad field | Both are `-32602`. The typed parse runs before the handler (D-A2a-1). Parity scenarios 12 and 35 send a valid input and still get the certificate message. |
| B12 | `params` missing or `null` | `null.get(..)` is `None`, so it works for a no-input method. | The same. The parser reads `null` as `{}`. | Kept (D-A2a-7). A unit test and a method test cover it. |
| B13 | `null` for an optional field (`block.check` from `conversation` sends `"person_did": null`) | Read as absent. | The same. `Option<T>` accepts `null`. | None needed. |
| B14 | `conversation.history` with `cursor: ""` | Same as no cursor. | The same. The handler keeps `filter(non-empty)`. | Kept. |
| B15 | Hub `contacts.set-limits` with a negative number (`Number.isInteger` allows it) | `-32602 "window_secs is required"` | `-32602 invalid value`, `data.field` | Only the text changes. |
| B16 | A non-local caller sends a bad request | `-32013` (admission runs first) | The same. The parse runs after admission. | A test proves the order (section 7). |
| B17 | Sibling call: `send_card_and_file` and `agreement_ops.rs` | Callee's `error.message` into `send_error` | Same as today. `Refused(e)` uses `e.message` directly (no ` (code N)` suffix), keeping not-sent path on any `Err`. | Step 6 maps `Refused(e)` to `e.message` directly (P4). |
| B18 | Sibling call: `catalog` `profile.get` | Error treated as "no address" (`Ok(None)`) | Same as today. `Err(Refused(_))` maps to `Ok(None)`; only `Transport` and `BadReply` become `Err`. | Step 5 preserves this mapping (P4). |
| B19 | Sibling call: `contacts_map` in `roym_conversation` | Ignores every error | Same as today. Any `Err` gives an empty map. | Step 6 preserves this mapping (P4). |
| B20 | Sibling call: `sync.rs` in `roym_transaction` | Callee error mapped to internal error | Same as today. `Refused(e)` gives `Response::internal_error(e.message)`. | Step 6 preserves this mapping (P4). |
| B21 | Certificate verbs in `catalog`, `transaction`, `directory` | Unchecked parameters | Reject unknown fields with `-32602` via shared typed `InstallCertificateInput` | Added to audit. All existing callers send only `certificate` (P5). Table rows follow in A2b/A2c. |
| B22 | Handcrafted bundle with stored profile row missing `record_id` | `profile.get` returns stored payload as-is | `-32603` internal error on deserialization into `ProfileRecord` | Added to audit. Only handcrafted malformed bundle can produce this (P17). |

No other difference was found. The parity harness keeps its 24 module files
and its 222 `fn scenario_` functions. Only the last assertion of scenario 2
changes (B2).

## 5. Decisions this plan makes

These follow the recommendations in section 2. If the owner answers
differently, the decision changes with the answer.

- **D-A2a-1. One wrapper for every handler.** A handler is
  `async fn(host: &H, input: I) -> Reply<O>` with `type Reply<T> =
  Result<T, Response>`. The `Err` side is an error `Response` (as the
  handlers already build). A function `typed` reads `req.params` into `I`,
  runs the handler, and writes `O` with `serde_json::to_value`. The parse
  runs once, before the handler. The one effect on the order of checks is
  B11.
- **D-A2a-2. One list per service writes the dispatch and the table**
  (`roym_core::service_methods!`, Q1 (a)). `invoke` keeps `require_internal`
  and then calls `dispatch`. It has no `match` on method names and no
  `handle_certificate_verb` call in profile and conversation. The certificate
  verbs are normal rows in the macro list, handled by typed functions in
  `roym_core::signing::verbs`. `certificate_rows` is deleted.
- **D-A2a-3. Where types live** (Q2 (a)). `roym_core::method` for X2.
  `roym_core::signing::verbs` for shared certificate types and verb handlers.
  `roym_core::method::profile` and `::conversation` for the structs that a
  sibling uses. `src/api.rs` for the `service_methods!` list in each service,
  and each service-only struct in `src/app/<area>.rs` beside the handler
  that reads it. No `src/api/` subdirectory is added.
- **D-A2a-4. Every A2a input refuses unknown fields.** Each input struct has
  `#[serde(deny_unknown_fields)]`. `NoParams` too (Q6 (a)). Two input
  structs are never joined with `#[serde(flatten)]`: `flatten` and
  `deny_unknown_fields` do not work together, so `Page`-like fields are
  repeated in each struct. Structs read from storage never use
  `deny_unknown_fields`.
- **D-A2a-5. The wire half of the rule is built as a check only** (Q3 (a)).
  `check::assert_service_table` takes a list of wire-reachable method names.
  A2a passes an empty list. A2c passes the directory's four.
- **D-A2a-6. The error** (Q4 (a)). `-32602`, message `invalid params:
  <path>: <serde text>` (no `<path>:` when the error is at the top), `data`
  `{"field": "<path>"}` when a field can be named. If the text starts with
  `missing field` or `duplicate field`, `field` is the path joined with the
  quoted name (only the name when the path is `.`). Otherwise, `field` is
  the path when it has at least one named segment. Otherwise there is no field.
  Error messages are truncated to 256 bytes on a char boundary.
  `serde_path_to_error` `0.1` is added to the workspace and to `roym_core`.
- **D-A2a-7. A missing or `null` `params` is `{}`.** Any non-object / non-null
  JSON value (such as an array) is rejected with `-32602 "invalid params:
  expected an object"` with no `data`.
- **D-A2a-8. The `group` key stays.** `ConversationRef`, `GroupRenameInput`
  and `GroupMemberInput` name the field `conversation` with `#[serde(alias =
  "group")]`. Both keys at once is a duplicate-field `-32602` with
  `data.field = "conversation"`. No caller sends both.
- **D-A2a-9. "One of two fields" rules stay in the handler**, with their
  old words: `conversation.open` and `group.add-member`/`remove-member`
  (`"person_did or address is required"`; `address` wins when both are
  sent), `block.add`/`remove`/`check` (`"at least one of person_did or
  address is required"`), `conversation.search` (an empty `query`:
  `"query is required"`).
- **D-A2a-10. Closed sets are enums** (Q7 (a)): `KindFilter { Direct, Group }`,
  `ReportSubject { Person, Listing, Message }`, `ReportCategory
  { Impersonation, Fraud, Harassment, UnsafeService, IllegalContent }`, all
  kebab-case. Stored rows keep their `String` fields.
- **D-A2a-11. Backup.** `*.export` returns `Bundle`. `*.import` takes
  `ImportInput { bundle: Bundle }`. The "whole `params` is the bundle"
  fallback is removed (B9).
- **D-A2a-12. Siblings use the structs** (Q8 (a)). `roym_core::method::call_sibling`
  takes the typed input and returns the typed output. It replaces
  `profile_call` in `conversation`. Five call sites in other crates move to
  it. Each site preserves its existing error handling (B17-B20).
- **D-A2a-13. Tests and clients keep sending raw JSON.** `roymctl`, the
  parity harness, the e2e tests and the Hub do not import the structs. A
  contract test that reuses the server's struct cannot find a wrong name.
- **D-A2a-14. The output shape is proved by shape files**, recorded from the
  unchanged code in Step 2 and kept green by every later step. An array records
  the sorted, de-duplicated list of its item shapes. A string records its exact
  value (not `"string"`) when its key is a closed word set (`kind`, `state`,
  `direction`, `body_encoding`, `admission`, `status`, `content_type`,
  `subject_kind`, `category`, `note`, or any key under `notices`).
- **D-A2a-15. Output rules.** An output struct reproduces today's JSON
  exactly. Where the old code wrote `null` (a `json!` with an `Option`), the
  field has no `skip_serializing_if`. Where the old code left a key out, the
  field has `skip_serializing_if = "Option::is_none"`. These places have the
  second kind: `body` and `deleted_at_secs` of `MessageView`, and
  `retry_after_secs`. `SigningStatusOutput.owner_did` is `Option<String>`
  without skip. `BlockCheckOutput` uses `struct BlockCheckOutput { blocked: bool,
  #[serde(flatten)] since: Option<BlockedSince> }` where `BlockedSince {
  reason: Option<String>, since_secs: Option<u64> }` has no skip on its
  options, preserving `null` when blocked without a reason. The three `get`
  methods write `null` for a missing row (an `Option` result, not a key).
  Stored `admission` remains kebab-case.
- **D-A2a-16. Numbers keep the width the handler already cast to**: `limit`
  of the conversation methods is `u32` (they cast `as u32`), the profile
  lists use `u64` (they cast `as usize`).
- **D-A2a-17. The shape tests and check tests live in one binary,
  `crates/roym_web/tests/method_contract.rs`.** It has modules `shapes` and
  `contract`, including the four parity harness helper modules with
  `#[allow(dead_code)]`.
- **D-A2a-18. Dead types are replaced.** Step 4 replaces `MessageRow` with
  `MessageView`. Step 6 replaces `ConversationRow` by `ConversationListItem`
  (`opened_at_secs: i64`, `last_activity_ms: i64`) and reuses
  `ConversationRowKind` for the `kind` output. `GroupMeta` and
  `apply_group_profile` remain as domain code.

## 6. Step-by-step changes

**Rule for every commit:** the workspace builds, and the checks of section
7.1 pass at the end of it. The shape files (Step 2) stay green from Step 2
to the end. Rebuild the WASM components with `mise run build:roym` before
you run the parity suite (it loads built artifacts).

### Step 1 — Commit this plan

- Commit this file and the `status.md` row: `docs(planning): plan M08A A2a
  typed methods`. Done by the planning session.

### Step 2 — Record the output shapes from the unchanged code

**Commit:** `test(roym-web): record method result shapes`.

- New test binary `crates/roym_web/tests/method_contract.rs` with `mod shapes;`.
  It is a single binary that will also host the contract tests in Step 7.
  It includes the parity harness modules with `#[allow(dead_code)]`:
  ```rust
  #[allow(dead_code)]
  #[path = "dual_build_parity/helpers.rs"]
  mod helpers;
  #[allow(dead_code)]
  #[path = "dual_build_parity/trust_fixtures.rs"]
  mod trust_fixtures;
  #[allow(dead_code)]
  #[path = "dual_build_parity/trust_harness.rs"]
  mod trust_harness;
  #[allow(dead_code)]
  #[path = "dual_build_parity/fixtures.rs"]
  mod fixtures;
  ```
  It uses the native build only, through `fixtures::harness()` and `h.native_http`.
- Signature:
  ```rust
  /// The shape of a JSON value: object keys (sorted) with the shape of each
  /// value; arrays as a sorted, de-duplicated list of item shapes; strings,
  /// numbers, booleans and null by their type name, EXCEPT closed word sets
  /// (`kind`, `state`, `direction`, `body_encoding`, `admission`, `status`,
  /// `content_type`, `subject_kind`, `category`, `note`, or any key under
  /// `notices`), which record their exact string value.
  fn shape(v: &Value) -> Value;
  /// Reads `tests/method_shapes/<case>.json`. With the env var
  /// `UPDATE_METHOD_SHAPES=1` it writes the file instead.
  fn assert_shape(case: &str, result: &Value);
  ```
- One test function per area (`profile_shapes`, `contacts_block_report_shapes`,
  `conversation_shapes`, `group_shapes`). Each builds its state in order
  and calls `assert_shape` for every case. Use the same set-up helpers the
  parity scenarios use (certificate install, `deliver_both`, `open_conv`,
  group creation). Do not copy their code. Reuse it.
- Required cases (about 60). Every method has at least one success case.
  Also these cases, because the shape differs:
  `profile.get` empty and set; `contacts.get`, `report.get` found and
  missing; `contacts.list`, `block.list`, `report.list` empty and with rows;
  `block.check` blocked and not blocked; `contacts.admit-first-contact`
  allow, rate-limited and blocked; `conversation.history` of a direct
  conversation, and of a group that has a membership item and a group-name
  item; a deleted message in `history`; `conversation.list` with a direct
  and a group row; `group.info` for the owner and for a removed member;
  `group.hide`, `group.unhide`; `*.export`; the two certificate verbs for
  both services.
- Record the files on the **unchanged** code:
  ```bash
  UPDATE_METHOD_SHAPES=1 cargo nextest run -p syneroym-roym-web --test method_contract shapes
  ```
- **Proof:** `cargo nextest run -p syneroym-roym-web --test method_contract shapes`
  passes with no env var. `git diff --stat` shows only the new test file and
  the shape files. This is the first proof of "no behaviour change": the
  files come from the old code.

### Step 3 — X2: the typed method machinery in `roym_core`

**Commit:** `feat(roym-core): add typed method helpers`.

- `Cargo.toml` (workspace): `serde_path_to_error = "0.1"`.
  `crates/roym_core/Cargo.toml`: `serde_path_to_error.workspace = true`.
- New `crates/roym_core/src/method.rs` (the entry point) and
  `crates/roym_core/src/method/check.rs`, `method/tests.rs`. Add `pub mod
  method;` to `lib.rs`.
- `method.rs` signatures:
  ```rust
  pub type Reply<T> = Result<T, Response>;

  /// Input of a method with no parameters.
  #[derive(Debug, Default, Deserialize, Serialize)]
  #[serde(deny_unknown_fields)]
  pub struct NoParams {}

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct PingOutput { pub service: String }

  /// A bad input, before it is turned into a `-32602` response.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct ParamError { pub field: Option<String>, pub message: String }
  impl ParamError { pub fn into_response(self) -> Response; }

  pub fn parse_value<T: DeserializeOwned>(params: &Value) -> Result<T, ParamError>;

  pub async fn typed<'a, H, I, O, F, Fut>(
      host: &'a H, req: &Request, handler: F,
  ) -> Response
  where I: DeserializeOwned, O: Serialize,
        F: FnOnce(&'a H, I) -> Fut, Fut: Future<Output = Reply<O>>;

  #[derive(Debug, Clone)]
  pub struct MethodInfo {
      pub name: String,
      pub input: &'static str,   // std::any::type_name::<I>()
      pub output: &'static str,
      pub probe: fn(&Value) -> Result<(), ParamError>, // parse_value::<I>, result dropped
  }
  impl MethodInfo { pub fn of<I: DeserializeOwned, O: Serialize>(name: &str) -> Self; }

  #[macro_export] macro_rules! service_methods { .. }
  ```
- `parse_value` pseudo-code:
  ```text
  if !params.is_object() && !params.is_null():
      return Err(ParamError { field: None, message: "invalid params: expected an object".into() })
  value = if params.is_null() { json!({}) } else { params }
  match serde_path_to_error::deserialize(value):
    Ok(t)  -> Ok(t)
    Err(e) -> path = e.path().to_string()          // "." at the top
              text = e.into_inner().to_string()
              name = quoted_name(text)             // text between the first pair of backticks
              is_missing_or_dup = text.starts_with("missing field") || text.starts_with("duplicate field")
              field = if is_missing_or_dup:
                          if path == "." { name } else { name.map(|n| format!("{path}.{n}")) }
                      else if path != "." && has_named_segment(path):
                          Some(path)
                      else:
                          None
              msg = if path == "." || path.starts_with("?") { format!("invalid params: {text}") }
                    else { format!("invalid params: {path}: {text}") }
              message = truncate_utf8(msg, 256)
              Err(ParamError { field, message })
  ```
  `into_response`: `Response::invalid_params(message)`, plus
  `.with_data(json!({"field": f}))` when `field` is `Some`.
  `typed` pseudo-code:
  ```text
  input  = parse_value(&req.params) or return err.into_response()
  output = handler(host, input).await or return the Err response
  match serde_json::to_value(output): Ok(v) -> Response::ok(v)
                                      Err(e) -> Response::internal_error(e.to_string())
  ```
- The macro (follow-set note: after `$handler:path` the next token must be
  `[`, `=>`, `,` and so on, not `(`; keep one row on one line):
  ```rust
  #[macro_export]
  macro_rules! service_methods {
      ($( $name:literal : $handler:path [ $input:ty => $output:ty ] ),+ $(,)?) => {
          pub fn method_rows() -> Vec<$crate::method::MethodInfo> {
              vec![ $( $crate::method::MethodInfo::of::<$input, $output>($name) ),+ ]
          }
          pub(crate) async fn dispatch<H: ::syneroym_app_host::AppHost>(
              host: &H, req: &$crate::envelope::Request,
          ) -> Option<$crate::envelope::Response> {
              match req.method.as_str() {
                  $( $name => Some($crate::method::typed::<H, $input, $output, _, _>(
                      host, req, $handler).await), )+
                  _ => None,
              }
          }
      };
  }
  ```
  A service writes `Response::method_not_found(&req.method)` when `dispatch`
  answers `None`. The macro uses `:` after the name, never `=>`, so the
  source scan can tell a macro row from a hand-written arm.
- `method/check.rs` (a normal `pub fn`, not `#[cfg(test)]`, so the test
  crate can call it):
  ```rust
  pub fn assert_service_table(
      service: Service, rows: &[MethodInfo], wire: &[&str], service_src_dir: &std::path::Path,
  );
  ```
  It panics with every problem it finds: duplicate name; `router::route`
  not `Some(service)`; a type name containing `serde_json::value::Value`;
  a row not in `wire` whose probe error does not contain `unknown field`
  (probe input: `{"__probe__": 1}`); a row in `wire` whose probe error does;
  a name in `wire` that is not a row; reading every `.rs` file under
  `service_src_dir` except `bindings.rs`, any line whose trimmed text
  matches `^"[a-z-]+\.[a-z-]+"\s*=>`; any use of `req.method` other than
  the single `Response::method_not_found(&req.method)` in `app.rs`; and any
  string literal matching `"<prefix><verb>"` for a prefix routed to the
  service that is not a row of `rows`.
- Unit tests (`method/tests.rs`, names by behaviour):
  `parse_reads_null_as_empty_object`; `parse_refuses_an_array`;
  one test for each of the five error kinds (wrong type, wrong value,
  missing field, unknown field, duplicate field) that checks the message
  and `data.field`, each at the top level and nested two levels down;
  a test that message truncation stops at 256 bytes on char boundary with
  a 10 KB string; `a_string_params_gives_an_error_with_no_field`;
  `typed_runs_the_handler_with_the_parsed_input` and
  `typed_returns_the_handler_error_unchanged` (with `TestHost`);
  `typed_gives_an_internal_error_when_the_output_cannot_be_written`;
  `service_methods_dispatches_listed_names_and_answers_none_for_others`
  (a test-only service with two rows);
  `check_accepts_a_correct_table`, and one `#[should_panic]` test per
  problem in `assert_service_table`, including a wire row that refuses and a
  refusing row listed as wire. The two synthetic types (one with
  `deny_unknown_fields`, one without) prove the wire branch (Q3).
- **Proof:** `cargo nextest run -p syneroym-roym-core` passes.
  `cargo clippy -q -p syneroym-roym-core --all-targets` is clean.
  `cargo build -p syneroym-roym-core --target wasm32-wasip2` builds
  (checks the new dependency on the guest target). Real rows are proved in
  Step 7.

### Step 4 — The certificate verbs, the backup types and `call_sibling`

**Commit:** `refactor(roym-core): type the shared verbs and sibling calls`.

- New `crates/roym_core/src/signing/verbs.rs` (re-exported from `signing.rs`):
  add `SigningStatusOutput` (`owner_did: Option<String>` with no
  `skip_serializing_if`), `InstallCertificateInput` (`deny_unknown_fields`),
  `InstallCertificateOutput`. Add typed handlers:
  ```rust
  pub async fn signing_status<H: AppHost>(
      host: &H, _: NoParams,
  ) -> Reply<SigningStatusOutput>;
  pub async fn install_signing_certificate<H: AppHost>(
      host: &H, input: InstallCertificateInput,
  ) -> Reply<InstallCertificateOutput>;
  ```
  `handle_certificate_verb` in `signing.rs` delegates to these two functions
  (serving `catalog`, `transaction`, and `directory` in A2a).
  `certificate_rows` is not needed and deleted.
- Storage rules: structs read from storage never use `deny_unknown_fields`,
  and stored `admission` stays the kebab-case word.
- `crates/roym_core/src/backup.rs`: add
  ```rust
  #[derive(Debug, Deserialize)]
  #[serde(deny_unknown_fields)]
  pub struct ImportInput { pub bundle: Bundle }
  ```
  `Bundle` itself does not get `deny_unknown_fields` (it is a signed
  document, not a method input).
- `crates/roym_core/src/method.rs` adds:
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum SiblingError {
      #[error("{0}")] Transport(String),
      #[error("{} (code {})", .0.message, .0.code)] Refused(RpcError),
      #[error("{0}")] BadReply(String),
  }
  pub async fn call_sibling<H: AppHost, I: Serialize, O: DeserializeOwned>(
      host: &H, service: Service, method: &str, input: &I,
  ) -> Result<O, SiblingError>;
  ```
  Pseudo-code: serialize `{method, params: input}`, `host.call(Dependency(service.name),
  service.interface, "invoke", json!([req]), None)`, parse a `Response`,
  turn `error` into `Refused(err)`, parse `result` into `O` (a missing `result` is
  `BadReply`). It is the same shape as `profile_call`
  ([app.rs:138](../../../../crates/roym_conversation/src/app.rs#L138)) and
  `sibling_call` ([app.rs:474](../../../../crates/roym_transaction/src/app.rs#L474)).
  `Refused` holds `RpcError` so callers can access `e.message` directly.
- New `crates/roym_core/src/method/profile.rs` and
  `method/conversation.rs`: the structs marked "core" in Q1.
  `ContactRow` moves here from `roym_profile` (re-export it from
  `roym_profile::app` so no other import breaks). `MessageView` implements
  `From<&Message>` (the host `Message`, `syneroym_app_host::types::conversation`)
  so `message_to_json` becomes `MessageView::from(&m)`. `MessageRow` and
  `ConversationRow` are replaced (D-A2a-18) in `roym_core/src/conversation.rs`.
- Unit tests: `message_view_writes_the_same_keys_as_the_old_json`
  (build a host `Message`, compare with the keys listed in Q1, with and
  without `deleted_at`); `block_check_output_blocked_and_unblocked_shapes`
  (nulls preserved when blocked);
  `admit_output_leaves_retry_out_unless_rate_limited`;
  `import_input_refuses_a_bare_bundle`;
  `signing_status_and_install_certificate_typed_handlers`;
  `call_sibling_with_test_host` (transport error, refused, bad reply).
- **Proof:** `cargo nextest run -p syneroym-roym-core` passes. The shape
  files of Step 2 still pass (nothing is wired yet, so this shows the
  shared verbs write the same JSON):
  `cargo nextest run -p syneroym-roym-web --test method_contract shapes`.

### Step 5 — Convert `profile`

**Commit:** `refactor(roym-profile): type every method`.

- New file in `crates/roym_profile/src/`: `api.rs` (entry point, with
  `service_methods!` and `method_table()`). Structs marked "svc" in Q1 live
  in `app/profile_ops.rs`, `app/contacts.rs`, `app/moderation.rs` beside
  the functions that read them. Add `pub mod api;` to `lib.rs`.
- `api.rs`:
  ```rust
  roym_core::service_methods! {
      "profile.ping": ping [NoParams => PingOutput],
      "profile.policy": profile_ops::policy [NoParams => PolicyOutput],
      "profile.get": profile_ops::get [ProfileGetInput => Option<ProfileRecord>],
      // ... all 24 methods in Q1, including profile.signing-status and profile.install-signing-certificate
  }
  pub fn method_table() -> Vec<MethodInfo> {
      method_rows()
  }
  ```
- `app.rs`: `invoke` becomes
  ```rust
  pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
      if let Some(resp) = admit::require_internal(host).await { return resp; }
      api::dispatch(host, &req).await
          .unwrap_or_else(|| Response::method_not_found(&req.method))
  }
  ```
- Every handler in `app/profile_ops.rs`, `contacts.rs`, `moderation.rs`,
  `backup.rs` changes its signature to `async fn name<H: AppHost>(host: &H,
  input: Input) -> Reply<Output>`. Error paths become `return
  Err(Response::..)`. Success becomes `Ok(Output { .. })`. Rules for the
  rewrite:
  - Delete every `req.params.get(..)`. The check at the end of the step
    greps for it.
  - Keep the order of the other checks and every error text (D-A2a-9).
  - `profile.get` reads the stored row into `ProfileRecord` (a row missing
    `record_id` returns `-32603`). `contacts.upsert` and `profile.set`
    write rows from the struct. `current_owner_*` helpers stay.
  - `contacts.set-limits` builds `ContactLimits` from `ContactLimitsInput`
    and keeps `limits.validate()`.
  - `block.add` keeps `block_keys` and builds the same rows.
  - `report.create` turns `ReportSubject` and `ReportCategory` into the
    same strings with `as_str()` before `content_digest`, so
    `report_id` is unchanged.
- `profile.policy` and `ping` become `async fn(_host: &H, _: NoParams)`.
- Sibling caller (Q8, B18): `crates/roym_catalog/src/app/listing_ops.rs:32`
  (`address_from_profile`) uses `call_sibling::<_, ProfileGetInput,
  Option<ProfileRecord>>(host, services::PROFILE, "profile.get",
  &ProfileGetInput::default())`. `Err(SiblingError::Refused(_))` maps to
  `Ok(None)` ("no address"), preserving existing behavior; only `Transport`
  and `BadReply` become `Err`. It reads `record.envelope` from the struct.
- Hub (Q9): `crates/roym_web/ui/src/screens/contacts.ts`. Add and export
  ```ts
  export function upsertParams(did: string, addr: string, envText: string): Record<string, unknown>
  ```
  that returns `{ person_did, conversation_address? , profile_envelope? }`
  with `profile_envelope` as the **trimmed text** (never a parsed object),
  and only when it is not empty. The click handler calls `JSON.parse` on
  the text only to show "Profile envelope is not valid JSON", and sends the
  text. New `contacts.test.ts` (vitest): object not sent, text sent as the
  same string, empty not sent, address trimmed and omitted when empty.
- Parity scenario 2 (Q6):
  [profile.rs:35-54](../../../../crates/roym_web/tests/dual_build_parity/profile.rs#L35).
  Rename it to `scenario_2_extra_caller_field_in_params_is_refused` (keep
  the number prefix, as every scenario has one). It still sends the same
  request to both builds and asserts `wasm_res == native_res`. The last
  lines assert `error.code == -32602` and `error.data.field == "auth"`.
  The file `profile.rs` is on the size ratchet list (recorded at 1112
  lines). The edit must not add lines. If `cargo xtask check-file-lengths`
  says it changed, record the real value.
- Delete `PROFILES`-row helpers that are now unused, if any (clippy will
  say). Rebuild WASM: `mise run build:roym`.
- **Proof:**
  ```bash
  mise run build:roym > target/a2a-build.log 2>&1; echo "exit=$?"; tail -n 3 target/a2a-build.log
  cargo clippy -q --workspace --all-targets --all-features
  cargo nextest run -p syneroym-roym-core -p syneroym-roym-profile -p syneroym-roym-catalog > target/a2a.log 2>&1; echo "exit=$?"; tail -n 5 target/a2a.log
  cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2a-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2a-par.log
  (cd crates/roym_web/ui && npm test) 2>&1 | tail -n 8
  grep -rn "params\.get\|req\.params" crates/roym_profile/src
  ```
  The last command prints nothing. The shape files and all 222 parity
  scenarios pass. Count before and after is the same:
  `grep -c "fn scenario_" crates/roym_web/tests/dual_build_parity/*.rs | awk -F: '{s+=$2} END {print s}'` prints `222`.

### Step 6 — Convert `conversation`, and the `transaction` call sites

**Commit:** `refactor(roym-conversation): type every method`.

- New file in `crates/roym_conversation/src/`: `api.rs`. Structs marked
  "svc" in Q1 live in `app/messages.rs`, `app/group.rs`, `app/backup.rs`.
  Same pattern as Step 5. The 24 rows of Q1 are the list in `service_methods!`.
- `app.rs`: `invoke` as in Step 5 (no `handle_certificate_verb`).
  `profile_call` and `contacts_map` change:
  ```rust
  pub(crate) async fn contacts_map<H: AppHost>(host: &H) -> HashMap<String, String>;
  ```
  keeps its signature but uses `call_sibling::<_, ContactsListInput,
  Vec<ContactRow>>`. Any `Err` returns an empty map (B19).
  `resolve_open_address` takes `OpenInput`-like fields
  (`address: Option<&str>`, `person_did: Option<&str>`) instead of `&Request`,
  because `group.add-member` and `group.remove-member` share it. It calls
  `ContactKeyInput`. `is_blocked` and `on_message` use `BlockCheckInput`
  (`BlockTargetInput`) and `AdmitFirstContactInput`, and read
  `BlockCheckOutput` and `AdmitFirstContactOutput` in place of
  `.get("blocked")` and string matching on `admission`. Stored `admission`
  stays kebab-case. Delete `profile_call`.
- `messages.rs` is 490 lines. It shrinks because `message_to_json` and
  `history_item_to_json` become `MessageView` conversions.
  `history_item_to_json` for membership and group-name items builds a
  `MessageView` with the same fixed values as the old `json!`
  (`state: delivered`, `verified: true`, `visible_seq: 0`, `refused: None`,
  `last_error: None`).
- `ConversationListItem`, `GroupInfoOutput` and the others are built with
  the same values and the same `Option` handling as the old `json!`
  (D-A2a-15). `conversation.search` keeps the "empty query" check after the
  parse (D-A2a-9).
- Sibling callers in `transaction` (Q8):
  - `crates/roym_transaction/src/app.rs:578` and
    `agreement_ops.rs:482`: `call_sibling::<_, SendInput, SendOutput>(host,
    services::CONVERSATION, "conversation.send", &SendInput { conversation,
    body, content_type: Some(CARD_CONTENT_TYPE.to_string()) })`. On error,
    every `Err` keeps the not-sent path, and `SiblingError::Refused(e)` uses
    `e.message` directly without the ` (code N)` suffix (B17).
    The code after it reads `output.message_id`, `output.state` and
    `output.sender_timestamp_ms` from the struct. `state` is a `StoredState`:
    where the code today compares it to a string, use `StoredState`'s kebab
    name (`StoredState` implements `Serialize`; add `as_str()` if a string is
    needed).
  - `sync.rs:126`: `ChangesInput` and `ChangesOutput`. On `SiblingError::Refused(e)`,
    it gives `Response::internal_error(e.message)` as today (B20). The loop that
    reads `messages` as `Vec<Value>` keeps working if the code needs the raw
    messages: use `serde_json::to_value(&out.messages)` there, or read the
    fields it needs from `MessageView`.
  - `conversation_call` stays only for helpers that A2b converts. Delete it
    when no call remains.
- **Proof:** build WASM artifacts with `mise run build:roym` first, run the
  commands of Step 5 for `-p syneroym-roym-conversation -p syneroym-roym-transaction`,
  then:
  ```bash
  cargo nextest run -p syneroym-substrate -E 'binary(~roym)' > target/a2a-e2e.log 2>&1; echo "exit=$?"; tail -n 5 target/a2a-e2e.log
  grep -rn "params\.get\|req\.params" crates/roym_conversation/src
  ```
  Check `target/a2a-e2e.log` to confirm the substrate `roym_*` tests ran and
  were not skipped. The grep prints nothing.

### Step 7 — The check test and the error tests

**Commit:** `test(roym-web): check every profile and conversation method`.

- Add `mod contract;` to `crates/roym_web/tests/method_contract.rs`.
- Tests (names by behaviour):
  - `every_profile_method_has_typed_input_and_output` and
    `every_conversation_method_has_typed_input_and_output`: call
    `check::assert_service_table(services::PROFILE, &roym_profile::api::method_table(),
    &[], std::path::Path::new("crates/roym_profile/src"))` and the same for
    conversation. Also assert the row count is 24 for each (a row added or
    removed changes it on purpose).
  - `every_listed_method_is_dispatched`: for each row of both tables, call
    the service's `invoke` through the harness's internal-call helper with
    `params: {}`. The answer must not be `-32601`. A name that is not in the
    table must be `-32601` (`conversation.no-such-method`).
  - `every_listed_method_is_refused_off_the_node_before_parsing`: for each
    row, call with a verified-remote caller and params `{"zz": 1}`. The
    answer must be `-32013`, not `-32602` (B16).
  - Error tests sent to both `h.wasm_http` and `h.native_http` with
    `Some(caller())` (asserting `wasm_res == native_res`, then asserting code
    and `data.field`):
    - `contacts.upsert` with `favourite: "yes"` gives `-32602`, `data.field
      == "favourite"` and the text contains `invalid type`.
    - `contacts.upsert` with an unknown key `nick` gives `-32602`,
      `data.field == "nick"`.
    - `contacts.upsert` with `profile_envelope` as an object gives `-32602`,
      `data.field == "profile_envelope"` (B1).
    - `conversation.send` with `body: 5` gives `data.field == "body"`.
    - `conversation.send` with an unknown key `colour` gives `data.field ==
      "colour"`.
    - `group.info` with `{"group": id}` works, with `{"conversation": id}`
      works, with both gives duplicate-field `-32602`, `data.field == "conversation"` (B3).
    - `profile.ping` with `params` absent works (B12).
- Ports: this file starts no listener. It uses the harness. No port number
  appears (`no_ephemeral_port_literals.rs` stays green).
- **Proof:** `cargo nextest run -p syneroym-roym-web --test method_contract`
  passes. Remove one row from `api.rs` of `profile` by hand and the first
  test fails; put a hand-written `"profile.get" =>` arm in `app.rs` and it
  fails again. (Do this once while developing. Do not commit it.)

### Step 8 — Docs and close-out

**Commit:** `docs: describe typed Roym methods`.

- Section 9 lists the files.
- Backlog: section 8.
- `crates/roym_web/tests/method_shapes/` and the new tests have no
  planning ids in names or comments.

### Step 9 — Finish

- Import clean-up pass over every edited Rust file.
- `cargo +nightly-2026-04-06 fmt --all`.
- Update the ratchets if they moved: `cargo xtask check-file-lengths`,
  `cargo xtask check-duplication` (the macro and the removed `profile_call`
  may lower the duplication value; record it).
- `cargo xtask check-module-layout`, `cargo xtask check-dep-versions`,
  `cargo shear` (the new dependency is used).
- `cargo dupes` recipe from AGENTS.md over the changed files.
- `mise run verify` as a background job, no `--skip`, every gate green.
- Open the PR with the CLA line and the attribution line. The PR text
  lists the commits, the method tables, the audit table (section 4), and
  "calls with a wrong field type now fail".

## 7. Tests

### 7.1 After every commit

```bash
cargo clippy -q --workspace --all-targets --all-features
cargo nextest run -p syneroym-roym-core -p syneroym-roym-profile -p syneroym-roym-conversation > target/a2a.log 2>&1; echo "exit=$?"; tail -n 5 target/a2a.log
cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2a-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2a-par.log
```

From Step 5 on, also the Hub unit tests (`npm test` in
`crates/roym_web/ui`). After Step 6, the substrate e2e of Step 6. After
`roym_*` code changes, rebuild with `mise run build:roym` first.

### 7.2 What each test proves

1. **X2 code (unit tests, Step 3 and 4).** The parser (null as `{}`, non-object
   refusal, 256-byte truncation, the five error kinds with `data.field`
   including nested fields), `typed`, the macro's dispatch, the check (each
   failure mode, the wire branch with synthetic types), `MessageView`,
   `BlockCheckOutput`, `AdmitFirstContactOutput`, `ImportInput`, `call_sibling`
   (with `TestHost`: transport error, refused, bad reply).
2. **OF-15 (Step 7).** Both tables have 24 rows, typed input and output, no
   `Value`, a route to the right service, a probe that refuses unknown
   fields, and no hand-written arm or `req.method` inspection in the service
   source files. A name that is not listed answers `-32601`.
3. **OF-14 (Step 7).** One wrong type, one unknown field, and duplicate
   fields per service give `-32602` with `data.field`, tested against both
   WASM and native builds to prove dual-build equality.
4. **OF-18 (Step 7).** The probe of the check, for all 48 rows, plus the
   off-node test: every method is refused with `-32013` before the parse.
5. **No behaviour change.**
   - The shape files, recorded from the old code in Step 2 and passing at
     every later step: every result has the same keys, the same JSON types,
     the same null or absent state, and exact string values for closed word sets.
   - The parity harness: 24 modules, 222 scenarios, the same two-build
     equality (`assert_eq!(stripped(&w), stripped(&n))` in `helpers.rs` is
     untouched). One assertion line changes in scenario 2 (B2).
   - The substrate e2e tests for profile and conversation, which send raw
     JSON and read results by key.
   - The Hub unit tests and the Playwright specs `roym-hub.spec.ts` and
     `roym-groups.spec.ts`.
   - `grep -rn "params\.get\|req\.params" crates/roym_profile/src
     crates/roym_conversation/src` prints nothing.
6. **The Hub fix (Step 5).** `contacts.test.ts`.

### 7.3 Not tested

- A request that sends a wrong type for every field of every method. The
  mechanism is one function (`parse_value`), tested once, and the check
  probes every row for the unknown-field attribute.
- Reading a stored row that an older build wrote. The product is not
  released (see the rename plan, which made the same choice).

### 7.4 Size risks

- `crates/roym_core/src/signing.rs` is at 761 lines. The signing types and
  typed verb handlers are placed directly in `crates/roym_core/src/signing/verbs.rs`
  and re-exported.
- `crates/roym_conversation/src/app/messages.rs` shrinks (490 lines now).
  `app/group.rs` and `app/contacts.rs` shrink a little. No new file is
  expected to pass 400 lines.
- `dual_build_parity/profile.rs` is on the oversized-test list (recorded at
  1112 lines). Step 5 must not make it longer.
- `cargo xtask check-duplication` can move in either direction. Record
  the new value if it improves.

## 8. Deferred backlog

All rows are in [deferred-backlog.md](../../deferred-backlog.md).

- **Narrow, not close, the §10 row "Roym's sibling boundary is untyped JSON,
  with no WIT-level contract between services".** New text: `profile` and
  `conversation` take and return typed structs in Rust and the sibling calls
  use them. `catalog`, `transaction`, `directory` and `web` are still on
  loose JSON until A2b and A2c. There is still no WIT contract between
  services: the method set grows with the product, and a WIT function per
  method makes each verb a deployed-interface change. The `status` export
  still returns an untyped JSON string. Target stays `TBD`. Update the
  source column to `crates/roym_core/src/method.rs`.
- **New row, §10: "Three list methods return a bare JSON array".**
  `contacts.list`, `block.list`, `report.list`. An array cannot gain a
  `next_cursor` or a total. Reason: wrapping them changes the wire, and A2
  does not reshape methods (this plan, Q5). Target: the first M8 change that
  adds a field to a list. Source: `crates/roym_profile/src/app/contacts.rs`.
- **New row, §10: "`transaction` still has its own sibling-call helper".**
  `roym_transaction::app::sibling_call` and `catalog_call`, and the direct
  `host.call` in `catalog` and `directory`, are not on
  `roym_core::method::call_sibling`. Reason: A2a moved only the five sites
  that call the A2a services. Target: A2b and A2c (close the row there).
  Source: `crates/roym_transaction/src/app.rs:474`.
- **Append to the §10 row "The Roym API contract is not published":** the
  `profile` and `conversation` tables can already feed a schema generator,
  and `roymctl`, the tests and the Hub still write their JSON by hand.
- **Resolved in code, no row needed:** the Hub's object `profile_envelope`
  (B1). It was a bug, not a deferral.
- No new open `TODO` or `FIXME` marker is added.

## 9. Living docs to update at close-out

Check each by asking: is a statement now false or incomplete? Do not cite
change docs, milestone or slice ids in these files. Link ADRs only.

- [docs/developer-guide.md](../../../developer-guide.md): in the Roym section,
  add "Add a Roym method": write the input and output struct, add one row to
  the service's `service_methods!` list, write the handler, run
  `method_contract`. Add "Bad input": the `-32602` text, `data.field`, the
  unknown-field rule (local-only refuses, wire-reachable ignores), the
  `group` key for group methods, and "send a string, not an object, for
  `profile_envelope`". Find the place with `grep -n "Roym" docs/developer-guide.md`.
- [docs/system-architecture.md](../../../system-architecture.md): in the section
  "SynApp 1: Roym", add a short paragraph describing that each service owns
  one method table, that admission runs before the input is parsed, and that
  a wrong input gets `-32602`.
- [docs/planning/traceability-matrix.md](../../traceability-matrix.md): add
  rows for typed inputs and outputs, the unknown-field rule and the table
  check, each pointing at `method_contract.rs`, and say "profile and
  conversation" until the other services join.
- [docs/roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md):
  only if its API section states how bad input is answered (grep `-32602`).
- Change doc [roym-open-foundation](../../changes/roym-open-foundation/change.md):
  - **Tasks:** the line "Typed input and output for every Roym method, the
    unknown-field rule, and the check test" stays open. Add the sub-bullet
    "- [x] profile and conversation" under it.
  - **Open questions:** mark "Typed-method check" resolved by D-A2a-2 and
    D-A2a-17.
  - **Decisions:** add D-A2a-1 to D-A2a-18 with the owner's answers to
    Q1 to Q9.
  - **Deviations:** the `NoParams` refusal changes parity scenario 2 (B2);
    the Hub's object `profile_envelope` was a bug and is fixed (B1); the
    "whole `params` is the bundle" form of `*.import` is removed (B9);
    `-32602` texts changed for missing and wrong fields (B4, B5, B10);
    `report.create` and `conversation.list` words are enums (B7, B8).
  - **`living-docs-touched`:** add the files above that changed.
  - `status` stays `draft`.
- [task.md](task.md): add the A2a result line under "A2".
- [status.md](status.md): row A2a: "Plan written" now. Later steps record
  the PR, the shape-file count, the parity count (24 and 222) and the
  `method_contract` result.

## 10. Exit criteria

A2a is done when all of these are true:

1. The owner has answered Q1 to Q9, and section 5 matches the answers.
2. Every one of the 48 methods of `profile` and `conversation` has an input
   struct and an output struct (OF-14). No `req.params.get` is left in
   `roym_profile` or `roym_conversation`.
3. `method_contract` passes: both tables have 24 rows, typed, local-only,
   with refusing inputs and no hand-written arm (OF-15, OF-18).
4. A wrong type, an unknown field, and duplicate fields give `-32602` with
   `data.field`, tested once per service (OF-14).
5. `method_contract shapes` passes with the files recorded from the unchanged code.
6. The parity harness has 24 modules and 222 scenarios and passes. Only
   scenario 2's last assertion changed.
7. The Hub sends a string `profile_envelope` and has a unit test for it.
8. `catalog` and `transaction` call the A2a services through typed structs
   at five sites.
9. The change doc, the developer guide, the architecture doc, the
   traceability matrix, task.md, status.md and the backlog are updated
   as in sections 8 and 9.
10. `mise run verify` passes with no `--skip`.
11. One PR. A2b starts after it is merged.

## 11. What A2a does not do

- No typed methods for `catalog`, `transaction`, `directory` or `web` (A2b
  and A2c), except that their certificate verbs share the typed functions in
  `roym_core::signing::verbs` in A2a; their table rows come in A2b and A2c.
  The 24 typed parse sites of `transaction` stay as they are.
- No wire-reachable method. The "ignore unknown fields" mode is only a
  check (Q3). The directory's four methods join in A2c.
- No schema is generated or published (OpenRPC, JSON Schema). No client
  package.
- No WIT contract between services. The `status` export stays untyped.
- No method renames. No reshaping of results: the three bare arrays stay
  arrays (Q5).
- No change to what a correct request does, to a stored row, to a signed
  record, or to the order of any check other than B11.
- No change to `roymctl` code. It keeps sending raw JSON.
- No new demo story.

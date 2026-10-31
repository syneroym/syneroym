# M08A slice A2c — Typed methods for directory and web: implementation plan

*Reader: the developer (or AI) who implements slice A2c. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan revised, 2026-10-31.** The owner answered Q1 to Q7 (all
option (a)) on 2026-10-31. Plan review round 1 (P1 to P12 in
[slice-a2c-review.md](slice-a2c-review.md)) is folded in. Section 5 and
section 6 follow the answers.

**Size and PR.** L. One PR on `feat/m08a-a2c-typed-methods`, with the
commits of section 6 in that order. The branch starts from `main` at
`e8c7b41d` (A2b is merged, PR #335) and has `main` at `a164e09d` merged
in (PR #336, Hub screens only). A2c is the last part of A2. A4 needs
it (task.md section 4).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). In a worktree use `cargo xtask verify` with
    `CARGO_TARGET_DIR` set.
  - Functions of at most 100 lines. Match arms of at most 10 lines. Files
    of at most 800 lines.
  - No planning ids in code: no "M08A", "A2c", "OF-18", "D-A2c", "B47" in
    code, comments or test names. No numbered comments that follow a list
    in this plan.
  - Keep every "why" comment beside code that stays. A typing slice has no
    reason to remove one (A2b R8).
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the ratchets (`check-file-lengths`, `check-duplication`).
  - Tests never hardcode a port.
- [task.md](task.md): section 1 (X2) and "A2 — Typed methods".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  OF-14, OF-15, OF-18, "Typed methods", decisions D-A2a-1 to D-A2a-18 and
  D-A2b-1 to D-A2b-17, Deviations.
- [slice-a2a-implementation-plan.md](slice-a2a-implementation-plan.md) and
  [slice-a2b-implementation-plan.md](slice-a2b-implementation-plan.md): the
  machinery this slice reuses. This plan has the same section layout as
  the A2b plan.
- [slice-a2a-review.md](slice-a2a-review.md) and
  [slice-a2b-review.md](slice-a2b-review.md): section 7.5 lists how A2c
  avoids each kind of finding.
- [deferred-backlog.md](../../deferred-backlog.md) section 10, the rows
  "Roym's sibling boundary is untyped JSON", "The Roym API contract is not
  published" and "Public contract versioning (`[G5]`)".

### Terms used in this plan

- **Method, input struct, output struct, local-only method, sibling call,
  certificate verbs, method table, shape file:** as defined in the A2a
  plan, section 0.
- **X2:** the typed-method code A2a built in `roym_core`:
  `method::parse_value`, `method::typed`, `ParamError`, `Reply<T>`,
  `NoParams`, `PingOutput`, `VerifyInput`, `MethodInfo`, the
  `service_methods!` macro, `method::call_sibling` with `SiblingError`,
  and `method::check::assert_service_table`
  ([method.rs](../../../../crates/roym_core/src/method.rs),
  [check.rs](../../../../crates/roym_core/src/method/check.rs)).
- **Wire-reachable method:** a method another node can call. Today exactly
  four, all in `directory`: `directory.search`, `directory.info`,
  `directory.publish`, `directory.standing`. `roym_core::admit::admit`
  lets them through from off the node
  ([admit.rs:56](../../../../crates/roym_core/src/admit.rs#L56)). Every
  other Roym method answers `-32013` to an off-node caller.
- **Wire list:** the names of the wire-reachable methods of one service,
  passed to `assert_service_table` as `wire`. A row in the wire list must
  ignore unknown fields. Every other row must refuse them. A2a built this
  check and passed an empty list (D-A2a-5). A2c is its first real user.
- **Server half / client half of `directory`:** one service, two jobs. The
  server half is a SynOrg's own directory (settings, roster, publications,
  search, credentials). It answers strangers. The client half is this
  person's own list of directories, the fan-out and the merge. It calls
  strangers. They share no collection
  ([app.rs:1-9](../../../../crates/roym_directory/src/app.rs#L1)).
- **SynOrg:** a group that runs a directory and vouches for its members
  (see [TERMINOLOGY.md](../../../TERMINOLOGY.md)).
- **Pass-through method:** a local-only method whose answer today is the
  answer of a remote node, returned unchanged: `directory.probe-info` and
  `directory.publish-to-source`.
- **Newer node / older node:** a node that runs a later or an earlier build
  of Roym. "Newer" here means it may send or answer a field this build does
  not know.
- **Hand-made bundle:** a backup bundle that was not written by `*.export`
  of this build. Only such a bundle can put a malformed row in storage.

## 1. What A2c builds, in one paragraph

A2c gives every method of the `directory` service (37 rows, with the two
certificate verbs and `directory.ping`) an input struct and an output
struct, in one `service_methods!` table. Four of them are wire-reachable:
their inputs ignore unknown fields, and their input and output structs move
to `roym_core::method::directory`, because both ends of the wire and the
federated query library of A4 speak them. The 33 local-only methods refuse
unknown fields, as in the other four services. `web` is the gateway of the
Hub: it checks the session and forwards every other method unchanged to
the service that owns it. It has one method of its own, `session.whoami`.
A2c types that one method, gives `web` a one-row method table, and keeps
the forwarder as it is (Q4). After A2c every Roym method is typed. The
reuse is X2 as A2a and A2b left it, plus three small additions named in
D-A2c-1. A correct request gets the same answer. Section 4 lists the
requests that do not, and every wire change.

## How this is shown to an end user

An end user sees nothing new. Every screen works as before.

- A developer can run `cargo nextest run -p syneroym-roym-web --test
  method_contract`. It checks the method table of every Roym service
  (for `web`, its one row). It shows that each method has a typed input
  and a typed output, that a wrong or
  unknown field gets `-32602` with the field name, and that the four
  directory methods other nodes call ignore an unknown field.
- A developer can run `cargo nextest run -p syneroym-roym-web --test
  method_contract shapes`. It shows that the JSON every directory method
  and `session.whoami` returns has the same keys and types as before the
  change.
- A developer can run `cargo nextest run -p syneroym-roym-web --test
  dual_build_parity newer_node`. It shows that a node answering with fields
  this build does not know is still read.

**Demo:** none. A2c adds no command, no screen and no working path. The
change doc's `demo:` list keeps its current line.

## 2. Points for the owner

Each question has the options, a recommendation and the reason. Section 5
and section 6 follow the owner's answers, which are under each question.

Three choices are not questions, because A2a or A2b already decided them
for every service: closed word sets in inputs are enums (D-A2a-10; Q5 is
only about the wire-reachable `SearchQuery`), the "whole `params` is the
bundle" form of `*.import` is removed (D-A2a-11), and record types inside
inputs keep no `deny_unknown_fields` (D-A2b-3).

### Q1. How `directory.publish` learns who called it

`directory.publish` records who published (`published_by`) and keys the
rate limit on it. Today `invoke` gets the caller from `admit::admit` and
passes it to the handler
([app.rs:184-187, :202](../../../../crates/roym_directory/src/app.rs#L184),
[publication_ops.rs:123-131, :238-246](../../../../crates/roym_directory/src/app/publication_ops.rs#L123)).
A typed handler is `fn(host, input)` (D-A2a-1). It has no third argument.

- (a) **The handler asks the host again.** Add to `roym_core::admit`:
  ```rust
  /// Who is on the other end of this guest call. The same answer `admit`
  /// reads; the host fixes it for the whole call.
  pub async fn caller<H: AppHost>(host: &H) -> Caller
  ```
  It maps `CallerOrigin::Internal` to `Caller::Internal`,
  `Verified(did)` to `Caller::Verified(did)`, `Anonymous` to
  `Caller::Anonymous`. `admit` calls it in place of its own match.
  `publish` calls it first. `invoke` keeps `admit::admit` for the
  decision only.
- (b) **Give `service_methods!` a context argument** passed to every
  handler of every service.
- (c) **Dispatch `directory.publish` by hand** before `api::dispatch`.

**Recommendation: (a).** It is one small function, and the host already
answers it for every call. Both builds fix the caller for the whole guest
call: the WASM host reads the invocation context
([guest.rs:413](../../../../crates/app_host/src/guest.rs#L413)), the
native host reads the fixed `invocation_origin` of its per-call host
([host.rs:685-698](../../../../crates/app_host_native/src/host.rs#L685)).
(b) changes about 140 handlers for one user. (c) is a hand-written arm,
which the check forbids (OF-15).

Owner answer: (a), 2026-10-31.

### Q2. `directory.set-settings`: what type the input has

Today the whole `params` is read as `SynOrgSettings`
([synorg.rs:33-48](../../../../crates/roym_directory/src/app/synorg.rs#L33)).
`SynOrgSettings` is also the stored row, the answer of
`directory.settings`, and a section of the backup bundle
([directory.rs:84-97](../../../../crates/roym_core/src/directory.rs#L84)).
A local-only input must refuse unknown fields, and a struct read from
storage never does (D-A2a-4).

- (a) **A `SynOrgSettingsInput` struct** in `synorg.rs` with the eight
  fields and the same serde attributes, `#[serde(deny_unknown_fields)]`,
  and `fn into_settings(self) -> SynOrgSettings` written as a struct
  literal, so a field added to one side only does not compile. A unit test
  fills every field and compares the JSON of both. This is what A2b did
  for `quote.set` (D-A2b-16).
- (b) **Add `deny_unknown_fields` to `SynOrgSettings`.** Then a stored row
  or a bundle section with an extra key can no longer be read, and
  `directory.info` fails for that node.
- (c) **Wrap it:** `{ "settings": { .. } }`. Every caller changes.

**Recommendation: (a).** It keeps the stored type lenient and the input
strict, with no wire change. `publication_limits` inside it stays
`PublicationLimits` (a nested type, as in D-A2b-3), so an unknown key
inside that block is ignored.

Owner answer: (a), 2026-10-31.

### Q3. The two pass-through methods

`directory.probe-info` and `directory.publish-to-source` return a remote
node's answer unchanged, result or error
([client_sources.rs:60-72, :262-265](../../../../crates/roym_directory/src/app/client_sources.rs#L60)).
A typed output cannot be "whatever the remote said".

- (a) **Read the remote result into the same struct the server half
  writes.** `probe-info` returns `Option<DirectoryInfoOutput>`,
  `publish-to-source` returns `PublishOutput` (D-A2c-11). A remote
  *error* passes through unchanged as the handler's `Err(Response)`, with
  its code, message and `data` (the Hub reads `data.admission` and
  `data.membership`,
  [membership.ts:129](../../../../crates/roym_web/ui/src/directory/membership.ts#L129)).
  A remote result that does not parse becomes `-32603`.
- (b) **Keep the raw pass-through.** The output is a
  `#[serde(transparent)]` wrapper around the remote `Value`, and the check
  gets a named exception for these two rows.

**Recommendation: (a).** It needs no exception in the check, and the same
struct is now checked on both ends. The cost is in section 3.6: an unknown
key that a newer remote adds is dropped from the local answer (the local
client is built with this node and does not know it anyway), and a remote
`directory.info` that holds a new closed word, for example a new `Area`
kind, gives `-32603` until this node is upgraded. The client half already
reads search hits the same strict way (B58), so this adds no new kind of
hole. `probe-info` is used only by `roymctl roym directory info`.

Owner answer: (a), 2026-10-31.

### Q4. `web`: what typing means for the gateway

`web` has one method of its own, `session.whoami`
([app.rs:130-132, :151-172, :243-249](../../../../crates/roym_web/src/app.rs#L130)).
Every other method is checked against the login session and forwarded,
with its `params` unchanged, to the service the router names
([app.rs:139-148, :185-233](../../../../crates/roym_web/src/app.rs#L139)).
The `/rpc` answer to `session.whoami` is built from the HTTP caller
identity, which a `(host, input)` handler cannot see. `router::route`
returns `None` for `session.whoami` on purpose
([router.rs:20-21, :104](../../../../crates/roym_core/src/router.rs#L20)),
and `assert_service_table` requires every row to route to its service.
So `web` does not fit the method-table model.

- (a) **Type the one method, keep the forwarder.** `session.whoami` reads
  `NoParams` and returns `WhoamiOutput`. One function builds the output
  for both paths. `roym_web::api::method_table()` returns that one row. A
  contract test checks it directly (D-A2c-15), not with
  `assert_service_table`. The forwarder keeps `params: Value`: the service
  that owns a method types it, and `web` must not parse a method it does
  not own (it would need every input struct of every service and would
  drift from them).
- (b) **Route `session.` to `web` in the router** and give `web` a
  `service_methods!` table. This breaks two router tests that say the
  router names only siblings
  ([router.rs:109-127](../../../../crates/roym_core/src/router.rs#L109)),
  `session.anything` would be forwarded to `web` itself, and the row still
  could not see the HTTP caller.
- (c) **Leave `web` untyped**, and say so in OF-15.

**Recommendation: (a).** It types the one thing `web` owns and says
plainly why the rest is not a table. The forwarder is safe: the check's
routing rule proves every typed row routes to its service, and `web`
forwards by the same `router::route`.

Owner answer: (a), 2026-10-31.

### Q5. `SearchQuery`'s `open_to` and `booking_mode`: `String` or enum

`SearchQuery` is the input of the wire-reachable `directory.search`, and
the `query` of the local `directory.query-source`
([directory.rs:170-184](../../../../crates/roym_core/src/directory.rs#L170)).
`open_to` and `booking_mode` are `Option<String>`. The handler turns them
into `listing::OpenTo` and `listing::BookingMode` and refuses a bad word
with its own text ("open_to must be one of: anyone, members, referral,
past-counterparties")
([search_ops.rs:298-321](../../../../crates/roym_directory/src/app/search_ops.rs#L298)).

- (a) **Enums**, as D-A2a-10 says for every closed set:
  `open_to: Option<OpenTo>`, `booking_mode: Option<BookingMode>`. The hand
  check is deleted. A bad word gets `-32602 invalid params: open_to:
  unknown variant `x`, expected one of ..` with `data.field = "open_to"`.
- (b) **Keep `String`** and the hand check.

**Recommendation: (a).** serde's text also lists the valid words, and the
code is the same `-32602`. A newer node that sends a new word is refused in
both options, so (b) protects nothing. One change: through
`directory.query-source` a bad word is now refused by this node before any
call, instead of being sent to every source and refused by each (B46). No
caller sends `open_to` or `booking_mode` except parity scenario 114, which
sends a valid word.

Owner answer: (a), 2026-10-31.

### Q6. The parity harness's two hand-made lists of directory methods

`trust_fixtures::ALL_DIRECTORY_VERBS` (37 names) and
`WIRE_REACHABLE_DIRECTORY_VERBS` (4 names) drive scenario 118
([trust_fixtures.rs:130-181](../../../../crates/roym_web/tests/dual_build_parity/trust_fixtures.rs#L130),
[directory.rs:638-671](../../../../crates/roym_web/tests/dual_build_parity/directory.rs#L638)).
The comment on the first list says: "A real guarantee would need `invoke`
to dispatch through a `const` table the test could import."

- (a) **Use the table for the long list; keep the short list.** Replace
  `ALL_DIRECTORY_VERBS` by a function that returns the names of
  `directory_api::method_table()`. Scenario 118 loops over it. Keep
  `WIRE_REACHABLE_DIRECTORY_VERBS` as an independent hand list, and assert
  in the contract test that it equals the names in
  `directory_api::WIRE_REACHABLE`.
- (b) **Leave both lists.**

**Recommendation: (a).** A method added to the table is then tested at
once. The four-name list is the security claim ("exactly these four answer
over the wire"). It must stay written by hand, so that a fifth name added
to the production list fails a test that a reviewer reads. The parity
harness keeps 222 scenarios. `dual_build_parity/directory.rs` is at its
cap on the oversized list (1135 lines), so the change there must not add a
line (D-A2c-17).

Owner answer: (a), 2026-10-31.

### Q7. The backlog row "Roym's sibling boundary is untyped JSON, with no WIT-level contract between services"

After A2c every Roym service is typed. The row
([deferred-backlog.md:509](../../deferred-backlog.md)) also says: there is
no WIT contract between services, and the `status` export returns an
untyped JSON string. Those two parts stay true.

- (a) **Split it.** Move the row to "Recently resolved" with the PR, for
  the typed half. Add a new, narrower row in §10: "Roym services have no
  WIT-level contract between them, and the `status` export is untyped".
- (b) **Close it** and drop the two remaining parts.
- (c) **Narrow it in place**, keeping its title.

**Recommendation: (a).** The title then says what is still open. (c) keeps
a title that is false. (b) loses two real gaps.

Owner answer: (a), 2026-10-31.

## 3. Facts checked in the code

All paths are from the repository root. Line numbers are at `a164e09d`.

### 3.1 What A2a and A2b left for A2c

- `directory.publish-to-source` already calls `listing.get` through
  `call_sibling` with `ListingKeyInput` and `Option<ListingGetOutput>`
  (A2b Q6,
  [client_sources.rs:221-236](../../../../crates/roym_directory/src/app/client_sources.rs#L221)).
  The rest of that handler is still a `&Request` handler.
- `directory` is the last user of `signing::handle_certificate_verb`
  ([signing.rs:258-269](../../../../crates/roym_core/src/signing.rs#L258),
  called at [app.rs:189](../../../../crates/roym_directory/src/app.rs#L189))
  and of `backup::export_signed`
  ([backup.rs:321-337](../../../../crates/roym_core/src/backup.rs#L321),
  called at [directory backup.rs:53](../../../../crates/roym_directory/src/app/backup.rs#L53)).
  `export_signed` has one unit test
  ([backup.rs:535-540](../../../../crates/roym_core/src/backup.rs#L535)).
  A router test comment names `handle_certificate_verb`
  ([router.rs:232](../../../../crates/roym_core/src/router.rs#L232)).
- The check's source scan allows `req.method` only on a line in `app.rs`
  that also holds `Response::method_not_found(&req.method)`
  ([check.rs:185-195](../../../../crates/roym_core/src/method/check.rs#L185)).
  `directory`'s `invoke` must also pass the method name to `admit::admit`
  ([app.rs:184](../../../../crates/roym_directory/src/app.rs#L184)). The
  scan would flag that line (D-A2c-16).
- `assert_service_table` already takes the wire list and checks both
  directions with the probe `{"__probe__": 1}`: a local-only row must
  refuse it with "unknown field", a wire row must not
  ([check.rs:88-111](../../../../crates/roym_core/src/method/check.rs#L88)).
- `PublicationLimitsInput { window_secs: u64, max_per_window: u32 }` with
  `deny_unknown_fields` lives in the catalog service
  ([limits.rs:16-21](../../../../crates/roym_catalog/src/app/limits.rs#L16)).
  `directory.set-limits` reads the same two fields.
- `ImportInput { bundle: Bundle }` is in `roym_core::backup`
  ([backup.rs:85-87](../../../../crates/roym_core/src/backup.rs#L85)).
- `directory`'s handlers are `pub(in crate::app)`. `api.rs` sits beside
  `app.rs` and cannot name them until they are `pub(crate)`.
- `directory` has unit tests only in `app/standing/tests.rs`. Every test of
  its methods is in the parity harness and the substrate e2e tests. `web`
  has no unit tests.

### 3.2 `directory`: methods, handlers and what they read

The dispatch is at [app.rs:183-240](../../../../crates/roym_directory/src/app.rs#L183).
`admit::admit` runs first with `WIRE_REACHABLE`
([app.rs:74-79](../../../../crates/roym_directory/src/app.rs#L74)), then
`handle_certificate_verb`, then a `match` of 35 arms. "`.get`" means
`req.params.get(..)` with `as_str` or `as_u64`: a wrong type reads as
absent. "W" marks the four wire-reachable methods.

**Server half**

| Method | Handler | Reads from `params` today |
| --- | --- | --- |
| `directory.ping` | inline `app.rs:194` | nothing |
| `directory.settings` | `synorg.rs:25` | nothing |
| `directory.set-settings` | `synorg.rs:33` | whole `params` as `SynOrgSettings` (text `invalid settings: {e}`), then `validate()` |
| `directory.info` W | `synorg.rs:55` | nothing |
| `member.add` | `synorg.rs:89` | `did` (`.get`, empty is "did is required"), `note` (`.get`, default `""`) |
| `member.remove` | `synorg.rs:105` | `did` (`.get`) |
| `member.list` | `synorg.rs:131` | nothing |
| `directory.publish` W | `publication_ops.rs:123` | caller first (`resolve_published_by`, `:238`), then `envelope` (`.get`, "envelope is required") |
| `directory.unpublish` | `publication_ops.rs:509` | `listing_id` (`.get`) |
| `directory.publications` | `publication_ops.rs:532` | nothing |
| `directory.search` W | `search_ops.rs:164` | whole `params` as `SearchQuery` (text `invalid query: {e}`), then `validate_search_query` (`:238`) and the `open_to` / `booking_mode` word check (`:298-321`) |
| `directory.limits` | inline `app.rs:206-209` | nothing |
| `directory.set-limits` | `publication_ops.rs:545` | `window_secs` (`.get`), `max_per_window` (`.get`, `as u32` at `:551`, cut without a message) |
| `directory.reindex` | `search_ops.rs:529` | nothing |
| `directory.export` | `backup.rs:41` | nothing |
| `directory.import` | `backup.rs:60` | `bundle`, or the whole `params` (`:61`) |
| `credential.issue` | `credential_ops.rs:127` | `parse_issue_params` (`:83`): `member_did` (`.get`, empty refused), `categories` (required, text `invalid categories: {e}`), `area` (`null` or absent is `[]`, text `invalid area: {e}`), `expires_at_secs` (`.get`). `note` (`.get`, default `""`) is read at `:198`, after the record is signed and stored. |
| `credential.list` | `credential_ops.rs:344` → `list_issued` `:325` | `member_did` (`.get`): a filter, or no filter |
| `revocation.issue` | `credential_ops.rs:220` | `credential_record_id` (`.get`, empty refused), `reason` (`.get`, default `""`) |
| `revocation.list` | `credential_ops.rs:348` → `list_issued` | `member_did` filter |
| `member.suspend` | `moderation_ops.rs:20` | `member_did` (`.get`, empty refused), `rule`, `reason` (`.get`, default `""`), `until_secs` (`null` or absent is `None`, anything not a `u64` is "until_secs must be a non-negative integer"), `scope` (`parse_scope` `:12`: `null` or absent is `Membership`, text `invalid scope: {e}`) |
| `member.lift` | `moderation_ops.rs:104` | `decision_record_id` (`.get`, empty refused), `reason` (`.get`, default `""`) |
| `member.decisions` | `moderation_ops.rs:203` → `list_issued` | `member_did` filter |
| `directory.standing` W | `standing.rs:248` | `member_did` (`.get`; not a string is "member_did is required"; a string that is not a `did:key` is "member_did must be a did:key") |
| `directory.signing-status`, `directory.install-signing-certificate` | `handle_certificate_verb` | typed since A2a (`NoParams`, `InstallCertificateInput`) |

**Client half**

| Method | Handler | Reads from `params` today |
| --- | --- | --- |
| `directory.add-source` | `client_sources.rs:74` | `did` (`.get`, empty refused), `issuer_did` (`:79-83`: `null` or absent is `None`, a `did:key` string is kept, anything else is "issuer_did must be a did:key"), `label` (`.get`, default `""`, `:109`) |
| `directory.probe-info` | `client_sources.rs:60` | `did` (`.get`, empty refused) |
| `directory.remove-source` | `client_sources.rs:173` | `did` (`.get`) |
| `directory.sources` | `client_sources.rs:191` | nothing |
| `directory.start-run` | `client_query.rs:68` | nothing |
| `directory.query-source` | `client_query.rs:121` | `parse_query_source_params` (`:201`): `run_id`, `source` (`.get`), `query` (absent is `SearchQuery::default()`; present is parsed, so `null` is refused with `invalid query: ..`) |
| `directory.merge` | `client_merge.rs:16` | `run_id` (`.get`) |
| `directory.run-envelope` | `client_merge.rs:261` | `run_id`, `record_id` (`.get`) |
| `directory.check-standing` | `held.rs:69` | `source`, `member_did` (`.get`, empty refused) |
| `directory.memberships` | `held.rs:220` | `member_did` filter (`.get`) |
| `directory.publish-to-source` | `client_sources.rs:211` | `source`, `listing_id` (`.get`) |

A wrong-typed filter (`member_did: 5`) to the four list methods reads as
absent, so today it lists **every** row (B43).

### 3.3 What each `directory` method returns today

"null" means the key is written with `null` when the value is missing (a
`json!` with an `Option`). "absent" means the key is left out.

- **`null` for "nothing here":** `directory.settings` (no settings row,
  [synorg.rs:28](../../../../crates/roym_directory/src/app/synorg.rs#L28)),
  `directory.info` (no settings row, `:64`), `directory.run-envelope` (no
  verified row,
  [client_merge.rs:292](../../../../crates/roym_directory/src/app/client_merge.rs#L292)).
  Their outputs are `Option`s.
- **Several shapes for one method.**
  - `directory.publish`: `{listing_id, record_id}` for a published listing
    ([publication_ops.rs:224](../../../../crates/roym_directory/src/app/publication_ops.rs#L224)),
    `{listing_id, withdrawn: true}` for a signed withdrawal (`:305`). The
    refusals carry `data`: `{admission: "not-admitted", membership}`
    (`:44-48`) and `{admission: "rate-limited", retry_after_secs}`
    (`:330-334`).
  - `directory.check-standing`: `{verdict, as_of_secs, refreshed: true}`
    ([held.rs:164](../../../../crates/roym_directory/src/app/held.rs#L164));
    `{verdict, refreshed: false}` when the issuer changed, **with no
    `as_of_secs`** (`:139-143`); `{verdict, as_of_secs, refreshed: false,
    error}` when the remote could not be used (`:209-214`).
  - `directory.query-source`: `{source, verified, refused, truncated,
    error: null}` (`client_query.rs:192-198`), or the same keys with
    `error` a `SourceError` and zeros (`:310-312`).
  - `credential.issue`, `revocation.issue`, `member.suspend`,
    `member.lift`: `{record_id, envelope}` on every path, also on the
    early "already done" returns (`credential_ops.rs:257-259`,
    `moderation_ops.rs:158-160`).
- **null keys:** `directory.info`'s `issuer_did` and `signing_did`
  (`synorg.rs:73-85`); `directory.add-source`'s `probe`
  (`client_sources.rs:170`); `directory.query-source`'s `error`;
  `directory.standing`'s `issuer_did` (`standing.rs:259-264`);
  `directory.memberships`' `issuer_did` and `last_error`
  (`held.rs:248-255`); `directory.merge`'s `sources[].membership`
  (`client_merge.rs:214-219`).
- **absent keys:** `check-standing`'s `as_of_secs` and `error` (above).
  Inside stored rows, the existing `skip_serializing_if` attributes stay
  (`SourceRow.last_ok_secs`, `.last_error`, `.issuer_did`;
  `IssuedRecordRow.until_secs`).
- **Stored rows returned as they are:** `member.list` (`members`),
  `directory.publications` (`publications`), `directory.sources`
  (`sources`), `credential.list`, `revocation.list`, `member.decisions`
  (`records`) return the stored JSON rows unchanged, read as `Value`.
  Every such row is written by serializing one struct: `Member`
  (`synorg.rs:98`, `credential_ops.rs:115-124`), `PublicationRow`
  (`publication_ops.rs:408-418`), `SourceRow` (`client_sources.rs:159-167`,
  `client_query.rs:443-454`), `IssuedRecordRow` (`credential_ops.rs:184-196`
  and the revoke and moderation paths). `directory.import` writes rows from
  the bundle without checking them, except that each publication's
  envelope must verify
  ([backup.rs:87-113](../../../../crates/roym_directory/src/app/backup.rs#L87)).
- **Maps:** `directory.import` returns `{imported: {<collection>: <count>},
  reindexed, standing}` (`backup.rs:130-143`). `serde_json` has no
  `preserve_order` here (A2a review), so a `BTreeMap<String, usize>` writes
  the same order.
- **Pass-through:** `directory.probe-info` and `directory.publish-to-source`
  return the remote `Response` as parsed (Q3).

### 3.4 `web`: what it is and which methods it has

`web` is the only inbound HTTP surface of Roym. Its manifest routes
`POST /rpc`, `GET /health` and `GET /ws`
([roym.toml:22-26](../../../../crates/roym_core/app/roym.toml#L22)). The Hub's
files are static assets of the same service (`[services.web.assets]`),
served by the host, not by `web`'s code.

- `handle_http` answers `POST /rpc`, `GET /health` (`{"status": "ok"}`),
  and 404 for anything else
  ([app.rs:93-110](../../../../crates/roym_web/src/app.rs#L93)).
- `rpc` reads a JSON-RPC envelope (`JsonRpcRequest`, `:28-36`). It answers
  `session.whoami` itself (`:130`). For every other method it checks the
  session against `router::method_auth` (`admit`, `:76-91`), finds the
  owner with `router::route`, and forwards `{method, params}` with `params`
  unchanged (`forward_rpc_call`, `:185-233`). An unrouted method is
  `-32601` from `web` (`:139-146`).
- `session.whoami` on `/rpc` returns `{did, auth, app_instance}` built from
  the HTTP caller identity. `auth` is `delegated`, `ucan`, `self-asserted`,
  or `anonymous` with `did` and `app_instance` `null` (`:151-172`). It reads
  no params.
- `invoke` (the WIT export) admits only a local dispatch path
  (`require_internal`, `:239`), answers `session.whoami` with the anonymous
  form (`:243-249`), and forwards everything else the same way
  (`:251-292`).
- The WebSocket hooks are no-ops (`:295-307`). The event feed of A5 lives
  in a host-level bus, not in `web` (D-OF-3).
- The `status` export returns an untyped JSON string, as in every service.

**What typing means for `web`.** `web` owns one method, `session.whoami`.
Everything else that passes through it is owned, and typed, by the service
it is forwarded to. The forwarder, `/health`, the WebSocket hooks and the
`status` export are not methods in the X2 sense. `web` does not fit
`service_methods!` (Q4): its one method reads the HTTP caller identity,
and the router names only siblings.

### 3.5 Callers (what each sends)

A throw-away script (kept out of the repository) listed the top-level keys
of every literal call to a `directory.`, `member.`, `credential.`,
`revocation.` or `session.` method in `crates/roym_web/tests`,
`crates/substrate/tests` (with the Playwright specs), `apps/roymctl/src`
and `crates/roym_web/ui/src`. Calls that build the params in a variable
were read by hand. **Every key sent is a field of the struct in section
5.** No caller sends an unknown key, a whole-`params` bundle, or a
non-object `params`.

- **Hub (`crates/roym_web/ui/src`).**
  `screens/synorg.ts`: `directory.settings` :33, `directory.set-settings`
  :101 (the eight settings keys; `publication_limits` from the last
  `directory.settings` answer or `{window_secs, max_per_window}`),
  `directory.set-limits {window_secs, max_per_window}` :172, `member.list`
  :205, `member.remove {did}` :216, `member.add {did, note}` :238,
  `directory.publications` :258, `directory.unpublish {listing_id}` :270.
  `screens/synorg_members.ts`: `credential.list`, `revocation.list`,
  `member.decisions` with no params :60-82, `credential.issue {member_did,
  categories, expires_at_secs}` :140, `member.suspend {member_did, rule,
  reason, scope?, until_secs?}` :192-208, `revocation.issue
  {credential_record_id, reason}` :275, `member.lift {decision_record_id}`
  :324. `screens/directory.ts`: `directory.check-standing {source,
  member_did}` :49, `directory.sources` :114, `directory.remove-source
  {did}` :140, `directory.add-source {did, label?}` :182 (`label:
  undefined` is dropped by `JSON.stringify`), `directory.merge {run_id}`
  :271. `directory/search.ts`: `directory.start-run` :69,
  `directory.query-source {run_id, source, query}` :96 (`query` holds only
  `text` and `categories`, `screens/directory.ts:250-256`),
  `directory.merge` :209, `directory.run-envelope {run_id, record_id}`
  :214. `screens/memberships.ts`: `directory.sources`,
  `directory.memberships`, `directory.check-standing`.
  `screens/listings.ts`: `directory.publish-to-source {source,
  listing_id}` :220; a refusal is shown from `data.admission` and
  `data.membership`. `screens/backup.ts`: `directory.export` and
  `directory.import {bundle}`. `rpc.ts` sends `params: {}` when none is
  given. **The Hub never calls `session.whoami`**: it reads the auth
  service's `/_syneroym/session/whoami`
  ([login.ts:281](../../../../crates/roym_web/ui/src/session/login.ts#L281)).
  **No Hub change.**
- **`roymctl`** (`apps/roymctl/src/commands/roym/`). `directory.rs:243-450`:
  `directory.sources`, `add-source {did, label?, issuer_did?}`,
  `remove-source {did}`, `publish-to-source {listing_id, source}`,
  `probe-info {did}`, `set-settings` (the eight keys, `area: []`),
  `member.add {did, note}` (`note` is a `String`, never `null`),
  `member.remove`, `member.list`, `member.suspend {member_did, rule,
  reason, scope, until_secs?}` (`scope` is always sent), `member.lift
  {decision_record_id, reason}`, `member.decisions {member_did?}`.
  `trust.rs:66-148`: `credential.issue {member_did, categories, area,
  expires_at_secs, note?}` (`area` is `[]` when not given),
  `credential.list`, `revocation.issue`, `check-standing`, `memberships`.
  `find.rs:111-284`: `start-run`, `query-source {run_id, source, query}`
  (`query` holds `categories`, `limit`, and `text` and `area` when given),
  `merge {run_id}`; it reads the answers with `.get`. **No `roymctl`
  change.**
- **Other services.** No other Roym crate calls a `directory` method or
  `session.whoami` (grep of `crates/roym_*/src`). `directory` calls
  `catalog`'s `listing.get` (typed since A2b).
- **Other nodes (the wire).** The client half calls three wire methods on
  remote directories, each with a hand-built `host.call` to
  `CallTarget::Service` with a 2 s timeout:
  - `directory.search` with `params` = this node's `SearchQuery`
    serialized (every field written, `null` when unset), and reads
    `result.truncated` (`as_bool`, default `false`) and `result.hits` as
    `Vec<SearchHit>` (default `[]`)
    ([client_query.rs:132-147, :251-303](../../../../crates/roym_directory/src/app/client_query.rs#L132));
  - `directory.info` with `{}`, and reads `result.name` and
    `result.issuer_did` with `.get` (add-source,
    [client_sources.rs:42-58, :116-143](../../../../crates/roym_directory/src/app/client_sources.rs#L42))
    or passes it through (probe-info);
  - `directory.standing` with `{member_did}`, and reads
    `result.issuer_did` with `.get` and `result.evidence` as
    `MembershipEvidence` (default `{}`)
    ([held.rs:89-152](../../../../crates/roym_directory/src/app/held.rs#L89));
  - `directory.publish` with `{envelope}`, and passes the answer through
    (`client_sources.rs:239-265`).
- **Parity harness** (`crates/roym_web/tests/dual_build_parity/`, 24
  modules, of which 19 hold scenarios, and 222 `fn scenario_`). The
  directory scenarios are in `directory.rs` (39), `trust.rs` (15),
  `trust_evidence_cap.rs` (7), `trust_sources.rs` (6), `wire_origin.rs`
  (8), `bundles.rs` (5). Every key sent is a field of the new structs.
  **Assertions on message text:** scenario 81's `contains("signature")`
  (`directory.rs:132`), scenario 84c's `contains("draft")` (`:237`) and
  scenario 112's `contains("no SynOrg")` (`:720`) read texts written after
  the parse, by checks that do not change. Scenario 110 sends
  `radius_m: u64::MAX` (`:690`) and scenario 111 sends 1000 categories
  (`:705`); both still get `-32602`, from `validate_search_query` after
  the parse (`radius_m` is a `u64`,
  [area.rs:27](../../../../crates/roym_core/src/area.rs#L27)). Scenario
  114 sends `open_to: "past-counterparties"`, a valid word (Q5).
  Scenario 106b and 118 send `{}` off the node (Q6). Scenario 3 sends
  `session.whoami` with `params: {}`
  ([profile.rs:57-80](../../../../crates/roym_web/tests/dual_build_parity/profile.rs#L57)).
  Canned remote answers are in `trust_harness.rs` (`hostile_source_response`,
  `trust_source_response`, joined by `canned_source_response` at `:297`).
  The `did:key:hTrunc` canned answer is `{"hits": [], "truncated": true}`,
  with no `directory` and no `answered_at_secs`; the client must keep
  reading it (D-A2c-13).
- **Substrate e2e** (`crates/substrate/tests/`): `roym_directory_e2e.rs`,
  `roym_trust_e2e.rs`, `roym_app_e2e.rs` (`session.whoami` with
  `params: {}` at `:390`, `:434`, `:454`), `roym_transaction_e2e.rs`, and
  the helpers `common/roym.rs`, `common/roym_flow.rs`. Every key sent is a
  field of the new structs. They assert codes, result keys and `data`
  (`roym_trust_e2e.rs:167-168` reads `data.admission` and
  `data.membership.state` of a `publish-to-source` refusal, which passes
  through unchanged under Q3 (a)). The one text assertion is
  `contains("32013")` (`roym_directory_e2e.rs:494`).
- **Playwright and demo stories.** `roym-hub.spec.ts`, `roym-trust.spec.ts`
  and `roym-groups.spec.ts` call `session.whoami` with `{}` and
  `credential.issue` with the three keys, and drive the Hub screens above.
  `roym-trust.spec.ts:125` reads the Hub's text for a `not-admitted`
  refusal, built from `data` that does not change. The demo stories
  (`crates/substrate/tests/e2e/demo/`) do not call these methods by name.
  Checked, no change.

### 3.6 Older and newer nodes: requests and answers in both directions

This is the first slice with wire-reachable methods. The rule (OF-18):
a newer node can call an older one. A removed or renamed field still
breaks a call; that waits for G5. The table reads one row per case, with
"A2c" for a node that runs this slice and "pre-A2c" for one that does not.

| # | Who calls whom | What happens | Why |
| --- | --- | --- | --- |
| C1 | A newer node (it sends an extra field) calls an A2c node's wire method | The field is ignored. Same answer. | The four wire inputs have no `deny_unknown_fields` (D-A2c-3). The check's wire list proves it for each row; the off-node contract test sends `{"zz": 1}` over the wire path (Step 8). |
| C2 | A2c node calls a pre-A2c node's wire method | Same answer as today. | A2c sends the same keys: `query-source` still sends the whole `SearchQuery`, `probe_info` sends `{}`, `check_standing` sends `{member_did}`, `publish_to_source` sends `{envelope}`. Pre-A2c handlers ignore unknown fields anyway (`.get`, and `SearchQuery` has no `deny_unknown_fields`). |
| C3 | Pre-A2c node calls an A2c node's wire method | Same answer for a correct request. | Same keys, same output JSON (the shape files prove the output, Step 2). |
| C4 | Either direction, a request with a field of the wrong type | `-32602` both before and after. Only the text changes: `invalid params: <path>: <text>` with `data.field` (B45). | The client half stores a remote refusal as `SourceError::Refused { code, message }` and shows it; no caller reads the text. |
| C5 | A newer node sends a new word in a closed set (`open_to`, `booking_mode`, a new `Area` kind) | Refused with `-32602`, before and after. | The word is unknown to this build in both options of Q5. This is a G5 question, not an A2c one. |
| C6 | An A2c node answers an older or newer client node | Same JSON as today. | Output structs reproduce today's JSON (D-A2a-15, shape files). |
| C7 | A newer node answers with an extra top-level key or an extra key inside a hit, a standing or an info | Read; the key is ignored. | The client reads with `.get` or into types with no `deny_unknown_fields` (`SearchHit`, `MembershipEvidence`). Output structs never get `deny_unknown_fields` (D-A2c-13). For the two pass-through methods the key is dropped from the local answer (Q3). The new parity scenario with a "newer node" canned source proves all four reads (Step 8). |
| C8 | A newer node answers with a new word in a closed set inside an answer (a new `AreaMatch` kind in a hit, a new verdict word, a new `Area` kind in `info`) | The whole answer is unreadable: the source is `unreadable`, the standing check falls back to the stored copy, `probe-info` answers `-32603`. | Older than A2c for the first two (`client_query.rs:284-295`, `held.rs:144-152`); new for `probe-info` under Q3 (a). Recorded as B58 and appended to the G5 backlog row (section 8). |
| C9 | A remote node answers with only some keys (the `hTrunc` canned answer: `hits` and `truncated` only; a third-party directory whose `info` has only some keys) | `directory.search`, `add-source` and `check-standing`: read as today. `probe-info` and `publish-to-source`: read strictly, but a missing key that has a natural empty value is read as empty. | A2c does not change how the client half reads `directory.search`, `directory.info` (add-source) and `directory.standing` answers (D-A2c-10). The two pass-through methods now read the whole result into a struct (Q3, D-A2c-9). The wire output structs get `#[serde(default)]` where an empty value means "not given" (D-A2c-13). An `info` without `name`, `retention_secs` or `member_count`, or a publish answer without `listing_id`, is `-32603` (B51). Every directory built from this repository sends every key, so this is a cost only for a third-party directory. |

## 4. Behaviour-change audit

A correct request gets the same answer after A2c. These are the requests
that do not, found by reading every caller of section 3.5 and every handler
of section 3.2. The numbers continue A2b's list (B23 to B40), so a number
in the change doc names one row.

| # | What is sent today | What the service does today | After A2c | How the plan handles it |
| --- | --- | --- | --- | --- |
| B41 | `credential.issue` with a `note` that is not a string | Ignored (`""`), after the credential is signed and stored | `-32602`, `data.field = "note"`, before any check | The parse runs before the handler (D-A2a-1). No caller sends a non-string `note`. |
| B42 | A missing or wrong field to a local-only directory method | `-32602` with texts such as `did is required`, `invalid settings: ..`, `invalid categories: ..`, `invalid area: ..`, `invalid scope: ..`, `until_secs must be a non-negative integer`, `issuer_did must be a did:key` (for a value that is not a string), `expires_at_secs is required`, `run_id is required`, `bundle is required`, `invalid bundle: ..`, `invalid query: ..` (query-source) | `-32602 invalid params: <path>: <text>` with `data.field` (D-A2a-6) | Only the text changes. An **empty** string keeps its old words in the handler (D-A2c-5). A string that is not a `did:key` keeps `member_did must be a did:key` and `issuer_did must be a did:key` in the handler. No test reads the changed texts (section 3.5). |
| B43 | A field of the wrong type that is read with `.get` (`note: 5`, `label: 5`, `reason: 5`, `rule: 5`, `member_did: 5` as a filter, `window_secs: "x"`) | Read as absent: `""`, "is required", or **no filter** (the four list methods then list every row) | `-32602`, `data.field` | No caller sends one. |
| B44 | An unknown top-level key, to any of the 33 local-only directory methods | Ignored | `-32602`, `data.field = <key>` | No caller sends one (section 3.5). |
| B45 | A wrong or missing field to a wire method: `directory.search` with a bad type (`invalid query: ..`); `directory.publish` with `envelope` missing or not a string (`envelope is required`); `directory.standing` with `member_did` not a string (`member_did is required`) | `-32602` with those texts | `-32602 invalid params: <path>: <text>` with `data.field`. `member_did must be a did:key` for a string that is not a `did:key` stays, in the handler. | Wire change, text only (section 3.6 C4). |
| B46 | `open_to` or `booking_mode` with a word that is not a variant: to `directory.search` (wire), or inside `query` of `directory.query-source` (local) | search: `-32602 open_to must be one of: ..`. query-source: the word is sent to every source, each refuses, and each source outcome is `refused` | search: `-32602 invalid params: open_to: unknown variant ..`, `data.field = "open_to"`. query-source: `-32602`, `data.field = "query.open_to"`, and no source is called | Q5 (a). Only parity scenario 114 sends `open_to`, with a valid word. |
| B47 | `directory.set-limits` with `max_per_window` above `u32::MAX` | Cut to 32 bits without a message (`publication_ops.rs:551`) | `-32602` | As A2b B30. No caller does this. |
| B48 | `directory.import` with the bundle as the whole `params` | Read as the bundle | `-32602 missing field `bundle`` | As A2a B9 and A2b B29. Every caller sends `{bundle}`. |
| B49 | A list method whose stored rows include a row that does not parse into its row struct (`member.list`, `directory.publications`, `directory.sources`, `credential.list`, `revocation.list`, `member.decisions`); `directory.settings` with a stored settings row that does not parse | The row is returned as it is | Lists: the row is skipped. `directory.settings`: `-32603`. | Only a hand-made bundle can write such a row (A2b B32). A row this build wrote is written by serializing the same struct, so it reads back to the same JSON (section 3.3). |
| B50 | `directory.info` with `params` that is not an object (an array, a string) | Answered (params are not read) | `-32602 invalid params: expected an object`, no `data` | D-A2a-7. Every caller (and every pre-A2c node) sends `{}`. Unknown keys inside an object are still ignored. |
| B51 | `directory.probe-info` or `directory.publish-to-source`, when the remote answers with a result | The remote `Response` unchanged | Result read into `Option<DirectoryInfoOutput>` / `PublishOutput` and written again: keys this build does not know are dropped; a result that does not parse (for example an `info` without `name`, C9) is `-32603`; a reply with no `result` key and no `error` is `-32603`. A remote **error** is unchanged, with its `data`. | Q3 (a), D-A2c-9. The read uses `method::read_reply`, the same reading code as `call_sibling`. |
| B52 | `session.whoami` with an unknown key or with `params` that is not an object, on `/rpc` or through `invoke` | Answered (params are not read) | `-32602`, `data.field = <key>` (or no `data` for a non-object). On `/rpc` this is the first error that `web` itself writes with `data` (it already passes a callee's `data` through). | Q4 (a). Every caller sends `{}` (section 3.5). |
| B53 | `directory.publish` from an anonymous caller | `admit` refuses it before the handler (`VerifiedOnly`). Inside, the caller is read before the input. | The same. The handler parses the input first, then reads the caller (Q1). | No visible change: an anonymous caller never reaches the handler. Listed because the order inside the handler changes, and the caller is now read by a new function. The contract test `a_wire_publish_is_recorded_under_the_verified_caller` (Step 8) proves, in both builds, that a wire publish is recorded and rate-limited under the verified caller's DID, not the owner's. |
| B54 | Certificate verbs of `directory` | Served by `handle_certificate_verb` with the typed functions | The same functions, now as table rows | No change (A2a B21). |
| B55 | `directory.search` over the wire with `params` absent or `null` | `-32602 invalid query: invalid type: null, expected struct SearchQuery` ([search_ops.rs:165](../../../../crates/roym_directory/src/app/search_ops.rs#L165)) | Answered as `{}`: the newest listings, as an empty query | D-A2a-7, as A2b B34. A wire change on an `Open` method: an anonymous caller now gets an answer where it got a refusal. It gets the same answer for `{}` today, so nothing new is exposed. `directory.standing` and `directory.publish` with no `params` still fail (`missing field`); only the text changes (B45). |
| B56 | A request with two bad fields | The error of the first check in the handler's order (for example `member.suspend` with an empty `member_did` and `until_secs: "x"` says `member_did is required`) | The error of the first bad field in the order serde reads them (the JSON key order), or the handler's empty-string check if the parse passes | No caller sends two errors. The error tests send exactly one (Step 8). |
| B57 | `directory.probe-info` against a node that runs no SynOrg (its `directory.info` answers `{"result": null}`); `directory.add-source` of that node | probe-info: `{}`, neither `result` nor `error`, because `serde` reads `"result": null` into `Option<Value>` as `None` (checked with a unit test on `Response`, 2026-10-31). add-source: the branch that writes the note "this address answered, but runs no directory" ([client_sources.rs:118-121](../../../../crates/roym_directory/src/app/client_sources.rs#L118)) never runs, for the same reason, so the Hub never shows the note ([directory.ts:186-190](../../../../crates/roym_web/ui/src/screens/directory.ts#L186)) | probe-info: `{"result": null}` (D-A2c-9). add-source: `probe` is the note, `last_ok_secs` is set as today (D-A2c-10) | A bug fix, found in plan review (P1). Both reads look at the raw `result` key. `roymctl roym directory info` prints `null` in place of an empty answer. The shape files of Step 5 and a parity scenario (Step 7) prove both. |

Found while checking, not changed by A2c:

- **B58.** A newer directory that adds a word to a closed set inside its
  answer (a new `AreaMatch` kind in a search hit, a new word in membership
  evidence) makes the whole answer unreadable for this node: `query-source`
  reads all hits with one `serde_json::from_value::<Vec<SearchHit>>`
  ([client_query.rs:284-295](../../../../crates/roym_directory/src/app/client_query.rs#L284)),
  so one unknown word marks the whole source `unreadable`, and
  `check-standing` falls back to the stored copy
  ([held.rs:144-152](../../../../crates/roym_directory/src/app/held.rs#L144)).
  Under Q3 (a) `probe-info` joins them. This is cross-version work (G5).
  The plan appends it to the G5 backlog row (section 8). A4, which moves
  the search read into the federated query library, is the natural place
  to read hits one by one.
- **B59.** The parity harness keeps a hand list of the 37 directory
  methods that nothing ties to the dispatch
  ([trust_fixtures.rs:130-136](../../../../crates/roym_web/tests/dual_build_parity/trust_fixtures.rs#L130)).
  Q6 replaces it with the table.

No other difference was found. The parity harness keeps its 24 modules
(19 with scenarios, and `deal_fixtures`, `fixtures`, `helpers`,
`trust_fixtures`, `trust_harness`) and 222 `fn scenario_` functions until
Step 7. Scenario 118
changes its loop (Q6). No other scenario assertion changes.

### Wire changes, in one list

What another node, a client or a test can observe:

1. Error text of every bad input to every directory method, and to
   `session.whoami`: `invalid params: <path>: <text>` with `data.field`
   (B42, B45, B46, B52). Codes do not change.
2. New `-32602` refusals: an unknown key to a local-only directory method
   (B44) or to `session.whoami` (B52); a wrong-typed field that was read as
   absent (B41, B43); `max_per_window` above `u32::MAX` (B47); a whole-params
   bundle (B48); non-object `params` to `directory.info` (B50); a bad
   `open_to` / `booking_mode` inside a local `query` (B46).
   A request with two bad fields may name a different one (B56).
3. `directory.query-source` no longer calls the sources when its `query`
   has a bad `open_to` / `booking_mode` word (B46).
4. The two pass-through methods drop unknown keys from a remote result, and
   answer `-32603` for a remote result they cannot read (B51, C9).
5. A wire `directory.search` with no `params` or `null` `params` is
   answered, as `{}` is (B55).
6. `directory.probe-info` for a node with no SynOrg answers
   `{"result": null}` in place of `{}`, and `directory.add-source` of such
   a node returns its `probe` note (B57).
7. Malformed stored rows from a hand-made bundle are skipped by list
   methods, and `directory.settings` answers `-32603` for one (B49).

Not a change: every correct request and every answer of the four wire
methods (section 3.6), and every `data` of a refusal (`admission`,
`membership`, `retry_after_secs`).

## 5. Decisions this plan makes

These follow the owner's answers to Q1 to Q7. Every A2a and A2b decision
(D-A2a-1 to D-A2a-18, D-A2b-1 to D-A2b-17) applies as it is. Each question
has its decision: Q1 → D-A2c-7, Q2 → D-A2c-8, Q3 → D-A2c-9, Q4 → D-A2c-15,
Q5 → D-A2c-6, Q6 → D-A2c-17, Q7 → section 8.

- **D-A2c-1. No new machinery beyond three small parts.** `directory` uses
  `service_methods!`, `typed`, `parse_value`, `call_sibling` and
  `assert_service_table` as A2a built them. The only additions to shared
  code are:
  1. `roym_core::method::WireNoParams` (an empty input with **no**
     `deny_unknown_fields`, for `directory.info`). `NoParams` refuses
     unknown fields, so a wire method with no params needs its own type.
  2. `roym_core::admit::caller` (Q1).
  3. One more allowed `req.method` line in the check's scan
     (D-A2c-16).
  Everything else is structs.
- **D-A2c-2. Where types live** (the rule of D-A2a-3).
  - New `roym_core::method::directory`
    (`crates/roym_core/src/method/directory.rs`): the inputs and outputs of
    the four wire methods, because the other end of the wire and A4's
    library read them: `PublishInput`, `PublishOutput`, `StandingInput`,
    `StandingOutput`, `SearchOutput`, `DirectoryInfoOutput`.
    `SearchQuery` stays in `roym_core::directory` (it is also the `query`
    of `directory.query-source`).
  - `roym_core::method` (`method.rs`): `WireNoParams`, and
    `PublicationLimitsInput` moved from `roym_catalog/src/app/limits.rs`,
    because `catalog` and `directory` both use it. The catalog `api.rs`
    and `limits.rs` import it from there.
  - Every other struct lives in `roym_directory`, beside the handler that
    reads it. Structs used by several handler files live in
    `crates/roym_directory/src/app.rs`: `DidInput { did }`,
    `MemberFilterInput { member_did: Option<String> }`,
    `RemovedOutput { removed: bool }`, `SignedRecordOutput { record_id,
    envelope }`.
  - `WhoamiOutput` and `SessionAuth` live in `roym_web` (`app.rs`).
  - The merge output types (`MergeOutput`, `MergedHit`, `HitSource`,
    `RefusedHit`) live in `client_merge.rs`. A4 moves them with the merge
    code into the federated query library.
- **D-A2c-3. Unknown fields.**
  - Every local-only input struct has `#[serde(deny_unknown_fields)]`
    (D-A2b-3), including `SynOrgSettingsInput` and `QuerySourceInput`.
  - The four wire inputs (`SearchQuery`, `WireNoParams`, `PublishInput`,
    `StandingInput`) have **no** `deny_unknown_fields`. They are listed in
    the check's wire list from the one constant `api::WIRE_REACHABLE`
    (D-A2c-16).
  - Types inside inputs keep their attributes: `SearchQuery` inside
    `QuerySourceInput.query` (so an unknown key inside a local `query` is
    ignored, as before: a misspelled filter such as `categorys` is dropped
    and the search is wider than asked; no check looks at nested types,
    and a strict copy of `SearchQuery` would be a second struct that must
    stay equal to it, so A2c only says this in the developer guide,
    section 9), `Area` inside `credential.issue`'s `area`,
    `ModerationScope` inside `member.suspend`'s `scope`,
    `PublicationLimits` and `Area` inside `SynOrgSettingsInput`.
- **D-A2c-4. Existing attributes and `null` stay as today, field by field**
  (D-A2b-4). A field that accepts `null` today is an `Option`. A field
  that today reads `null` as an error stays non-`Option`.

  | Field | Today | Type |
  | --- | --- | --- |
  | `member.add` `note`; `revocation.issue`, `member.suspend`, `member.lift` `reason`; `member.suspend` `rule`; `directory.add-source` `label`; `credential.issue` `note` | absent, `null` and a wrong type are `""` | `Option<String>`, then `.unwrap_or_default()` |
  | `credential.issue` `area` | absent or `null` is `[]` | `Option<Vec<Area>>`, then `.unwrap_or_default()` |
  | `member.suspend` `scope` | absent or `null` is `Membership` | `Option<ModerationScope>`, then `.unwrap_or(ModerationScope::Membership)` |
  | `member.suspend` `until_secs` | absent or `null` is `None` | `Option<u64>`. Keep the "why" of `moderation_ops.rs:31-33` as a comment on the field: a string or a float must not quietly become "until lifted". |
  | `directory.add-source` `issuer_did` | absent or `null` is `None` | `Option<String>`; the `did:key` check stays in the handler |
  | the `member_did` filter of the four list methods | absent or `null` is no filter | `Option<String>` |
  | `directory.query-source` `query` | absent is the default; `null` is refused | `#[serde(default)] query: SearchQuery` |
  | `credential.issue` `categories`; every other required field | absent or `null` is refused | non-`Option` |
- **D-A2c-5. "Is required" for an empty string stays in the handler**, with
  its old words (as D-A2a-9). A field that is present, a string, and empty
  still gets its old `-32602` text: `did` of `member.add`,
  `directory.add-source` and `directory.probe-info`; `member_did` of
  `credential.issue`, `member.suspend` and `directory.check-standing`;
  `source` of `directory.check-standing`; `decision_record_id` of
  `member.lift`; `credential_record_id` of `revocation.issue`. The fields
  that have no empty check today (for example `member.remove`'s `did`)
  get none.
- **D-A2c-6. Closed sets** (Q5 (a)). `SearchQuery.open_to:
  Option<listing::OpenTo>` and `SearchQuery.booking_mode:
  Option<listing::BookingMode>`. Delete the word check at
  `search_ops.rs:298-321`; `build_search_filter` pushes
  `serde_str(&v)` of the enum as today, so the index sees the same words.
  `SearchQuery` still serializes `null` for an unset field, so
  `query-source` sends the same JSON. Output: `WhoamiOutput.auth:
  SessionAuth { Delegated, Ucan, SelfAsserted, Anonymous }`, kebab-case.
  Stored strings stay strings (`SearchRunRow.status`,
  `.revocation_status`, and the same fields of `MergedHit`), as D-A2a-10.
- **D-A2c-7. `directory.publish` reads its caller itself** (Q1 (a)).
  `admit::caller` as in Q1, and `admit` calls it. `publish(host, input:
  PublishInput) -> Reply<PublishOutput>` calls `admit::caller(host)` and
  then `resolve_published_by` as today. The comment above
  `resolve_published_by` (`publication_ops.rs:226-237`) stays.
- **D-A2c-8. `directory.set-settings` takes `SynOrgSettingsInput`** (Q2
  (a)). Eight fields with the attributes of `SynOrgSettings` (`area`,
  `categories` and `publication_limits` have `#[serde(default)]`),
  `#[serde(deny_unknown_fields)]`, `fn into_settings(self) ->
  SynOrgSettings` that first destructures `self` completely (`let Self {
  name, rules, .. } = self;` with every field named, no `..`) and then
  builds a struct literal. A field added to only one of the two structs
  then does not compile. The output is `SynOrgSettings`.
  `validate()` runs on the result, as today.
- **D-A2c-9. The pass-through methods are typed** (Q3 (a)).
  - **Do not parse the reply as `Response`.** `Response.result` is
    `Option<Value>`, and serde reads `"result": null` into `None`, so a
    directory with no SynOrg (`{"result": null}`) and a reply with no
    result look the same (B57). `call_sibling` already reads the reply as
    a `Value` and looks at the `result` key for this reason.
  - In `roym_core::method`, split the reading tail of `call_sibling`
    ([method.rs:185-201](../../../../crates/roym_core/src/method.rs#L185))
    into
    ```rust
    /// Reads one `invoke` reply: an `error` is `Refused`, a `result` is read as `O`.
    pub fn read_reply<O: DeserializeOwned>(raw: &str) -> Result<O, SiblingError>
    ```
    `call_sibling` calls it, with no change to its errors or texts.
  - In `client_sources.rs`:
    ```rust
    /// One `invoke` reply from a remote directory: its result read as `O`,
    /// or its error returned unchanged.
    fn remote_reply<O: DeserializeOwned>(raw: &str) -> Reply<O>
    ```
    calls `method::read_reply::<O>`. `Refused(e)` is `Err(Response {
    result: None, error: Some(e) })`, so `data` stays. `BadReply(text)` is
    `-32603 "unreadable answer from the directory: {text}"`. (`Transport`
    does not come from `read_reply`.)
  - `probe_info_verb` returns `remote_reply::<Option<DirectoryInfoOutput>>`,
    so `{"result": null}` is `Ok(None)` and the answer is
    `{"result": null}`. `publish_to_source` returns
    `remote_reply::<PublishOutput>`. Their transport errors keep today's
    texts (`client_sources.rs:70`, `:259`).
  - `add_source` keeps reading the info answer with `.get` (D-A2c-10), but
    parses the reply as `Value`, not `Response`: `v.get("result")` is
    `Some(Value::Null)` for "runs no directory" (the note), and the other
    reads use `v.get("result")` in place of `resp.result.as_ref()`. A JSON
    parse error stays `SourceError::Unreadable`. This is the fix of B57.
- **D-A2c-10. Other client-half reads of remote answers do not change.**
  `query_source`'s read of `directory.search` (A4 moves it into the
  federated query library and can use `SearchOutput` there),
  `add_source`'s read of `directory.info`, and `handle_standing_reply`'s
  read of `directory.standing` stay as they are. Reason: no behaviour
  change (except the `null` read of B57 in `add_source`), and the A4
  move. Section 8 records this as deferred. The outbound requests stay
  hand-built `host.call`s:
  they target `CallTarget::Service` with options that `call_sibling` does
  not take, and A3/A4 replace the search call with `call-all`.
- **D-A2c-11. Outputs that have more than one shape.**
  - `PublishOutput { listing_id: String, record_id: Option<String>,
    withdrawn: Option<bool> }`, both `Option`s with
    `#[serde(default, skip_serializing_if = "Option::is_none")]`. A struct,
    not an untagged enum, because `publish-to-source` reads it back (the
    A2a P7 risk of reading an untagged enum).
  - `CheckStandingOutput { verdict: MembershipVerdict, as_of_secs:
    Option<u64>, refreshed: bool, error: Option<String> }`, both `Option`s
    with `skip_serializing_if = "Option::is_none"`. The three paths of
    section 3.3 set them so: refreshed `Some`/`None`, issuer-changed
    `None`/`None`, stale `Some`/`Some`.
  - `QuerySourceOutput { source, verified: u32, refused: u32, truncated:
    bool, error: Option<SourceError> }`, `error` with no skip (`null`).
    `refuse_source` and the normal path both build it.
  - `Option<_>` for the three "nothing here" answers: `directory.settings`
    → `Option<SynOrgSettings>`, `directory.info` and
    `directory.probe-info` → `Option<DirectoryInfoOutput>`,
    `directory.run-envelope` → `Option<RunEnvelopeOutput>`.
- **D-A2c-12. Stored rows in outputs are parsed into their row structs**
  (B49, as A2b B32). The four list paths read rows as `Value`, parse each,
  and skip a row that does not parse. `directory.settings` returns
  `-32603` for a row that does not parse. The row structs (`Member`,
  `PublicationRow`, `SourceRow`, `IssuedRecordRow`, `SynOrgSettings`) do
  not change, and none gets `deny_unknown_fields`.
- **D-A2c-13. Wire output structs are readable by an older or newer
  node.** The six types of `roym_core::method::directory` derive
  `Serialize` and `Deserialize`, never `deny_unknown_fields`. A field that
  today's client reads with a default gets `#[serde(default)]` (it changes
  nothing when written): `SearchOutput` all four fields (the `hTrunc`
  answer has only two), `StandingOutput.issuer_did` and `.evidence`,
  `DirectoryInfoOutput.issuer_did` and `.signing_did`. Also
  `DirectoryInfoOutput.rules`, `.area`, `.categories`, `.support_contact`
  and `.dispute_path`: an empty value there means "not given" (C9).
  `name`, `retention_secs` and `member_count` stay required, because a
  default (`""`, `0`) would be a false claim about the remote directory.
  `PublishOutput.listing_id` stays required. `Option` fields that are
  written as `null` today have no skip.
- **D-A2c-14. Certificate verbs, export, and two deletions.** The two
  certificate verbs are table rows (`signing_status`,
  `install_signing_certificate` from `roym_core::signing::verbs`).
  `directory.export` returns `backup::export_signed_bundle` (D-A2b-12).
  Delete `signing::handle_certificate_verb` and `backup::export_signed`:
  nothing else uses them (section 3.1). Change the unit test at
  `backup.rs:535-540` to call `export_signed_bundle` and assert that its
  error is `Response::invalid_params("signing-not-enrolled")`. Change the
  router test comment at `router.rs:232` to say the five services list
  their certificate verbs in their method tables.
- **D-A2c-15. `web` has one typed method and keeps its forwarder** (Q4
  (a)).
  ```rust
  /// The one method `web` answers itself; every other method is forwarded.
  pub const WHOAMI: &str = "session.whoami";

  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  #[serde(rename_all = "kebab-case")]
  pub enum SessionAuth { Delegated, Ucan, SelfAsserted, Anonymous }

  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  pub struct WhoamiOutput {
      pub did: Option<String>,
      pub auth: SessionAuth,
      pub app_instance: Option<String>,
  }

  fn whoami(caller: Option<&CallerIdentity>) -> WhoamiOutput
  ```
  No skip on the `Option`s (`null` today). Both paths parse `params` with
  `parse_value::<NoParams>` first. The `/rpc` path turns a `ParamError`
  into a JSON-RPC error body that carries `data`, the way
  `forward_rpc_call` writes a callee error (`app.rs:214-215`), and keeps
  the request `id`. `invoke` returns `err.into_response()`.
  `crates/roym_web/src/api.rs` holds
  `pub fn method_table() -> Vec<MethodInfo> { vec![MethodInfo::of::<NoParams, WhoamiOutput>(WHOAMI)] }`.
  `rpc` and `invoke` compare with `WHOAMI`. The forwarder, `JsonRpcRequest`
  and `/health` do not change.
- **D-A2c-16. One wire list, and the check's scan.** `WIRE_REACHABLE`
  moves from `app.rs` to `crates/roym_directory/src/api.rs` as
  `pub const WIRE_REACHABLE: &[(&str, WireRule)]`, with its comment
  (`app.rs:68-73`). `invoke` passes `api::WIRE_REACHABLE` to
  `admit::admit`. The contract test builds the check's `wire` list from it.
  In `check.rs`, a `req.method` line in `app.rs` is also allowed when it
  holds the whole call `admit::admit(host, api::WIRE_REACHABLE,
  &req.method)` (the call that decides admission by method name), and no
  other `req.method`. Match the full call text, as the existing
  `Response::method_not_found(&req.method)` allowance does, and count the
  `req.method` uses on the line: exactly one. Add the unit test
  `check_allows_req_method_only_in_the_admit_call` in
  `method/tests/directory.rs` (Step 3). It shows that the call line
  passes, that a line with the call and also `req.method.as_str()` fails,
  and that `admit::admit(host, OTHER, &req.method)` fails. Keep
  the `admit::admit` call on one line (it is about 85 characters with
  `if let Err(resp) = ` and four spaces of indent).
- **D-A2c-17. The parity lists** (Q6 (a)). In `trust_fixtures.rs`, replace
  `ALL_DIRECTORY_VERBS` and its comment by
  ```rust
  /// Every method of `directory`'s method table, so a new row is tested here at once.
  pub(crate) fn all_directory_verbs() -> Vec<String>
  ```
  (the `name`s of `syneroym_roym_directory::api::method_table()`).
  Scenario 118 changes only its loop line, the `contains` line
  (`for method in &all_directory_verbs() {`,
  `..contains(&method.as_str())`) and, in place, the text of its
  assertion message ("is in ALL_DIRECTORY_VERBS but ..." becomes "is in
  the directory method table but ...",
  [directory.rs:648](../../../../crates/roym_web/tests/dual_build_parity/directory.rs#L648)):
  `directory.rs` is at its cap of 1135 lines and must not grow. Keep `WIRE_REACHABLE_DIRECTORY_VERBS` and its
  comment. The contract test of Step 8 asserts that its sorted names equal
  the sorted names of `api::WIRE_REACHABLE`.
- **D-A2c-18. Shapes.** `auth` joins the closed-word keys of the shape
  helper (`is_closed_word_key`,
  [shapes.rs:15-42](../../../../crates/roym_web/tests/method_contract/shapes.rs#L15)).
  No other key is added. The 157 shape files of A2a and A2b must not
  change.
- **D-A2c-19. The directory conversion is two commits**, so each can be
  reviewed alone: the server half (with the certificate verbs, export and
  import), then the client half. Between them, `invoke` tries
  `api::dispatch` and falls back to the old `match` for the methods not yet
  listed (as D-A2b-15). The second commit removes the fallback. The check
  test runs only after both.

### Method tables

Every local-only input struct has `#[serde(deny_unknown_fields)]`. "svc"
means `roym_directory`, beside the handler. "core" means `roym_core`
(`m::d` is `roym_core::method::directory`). W marks a wire-reachable
method. Number types follow the code today: counts that the code holds as
`u32` stay `u32`, `usize` counts stay `usize` in outputs, seconds are
`u64`.

**directory (37 methods)**

| Method | Input | Output | Where |
| --- | --- | --- | --- |
| `directory.ping` | `NoParams` | `PingOutput` (`service: "directory"`) | core |
| `directory.settings` | `NoParams` | `Option<SynOrgSettings>` | core (exists) |
| `directory.set-settings` | `SynOrgSettingsInput` (D-A2c-8) | `SynOrgSettings` | svc `synorg.rs` |
| `directory.info` W | `WireNoParams` | `Option<DirectoryInfoOutput>` (`name`, `rules`, `area: Vec<Area>`, `categories: Vec<String>`, `support_contact`, `dispute_path`, `retention_secs: u64`, `member_count: u64`, `issuer_did: Option<String>`, `signing_did: Option<String>`) | `m::d` |
| `member.add` | `MemberAddInput { did: String, note: Option<String> }` | `Member` | svc `synorg.rs` |
| `member.remove` | `DidInput` | `RemovedOutput` | svc `app.rs` |
| `member.list` | `NoParams` | `MemberListOutput { members: Vec<Member> }` | svc `synorg.rs` |
| `directory.publish` W | `PublishInput { envelope: String }` | `PublishOutput` (D-A2c-11) | `m::d` |
| `directory.unpublish` | `ListingKeyInput` (reused from `roym_core::method::catalog`, the same `{listing_id}`) | `UnpublishOutput { listing_id, unpublished: bool }` | svc `publication_ops.rs` |
| `directory.publications` | `NoParams` | `PublicationsOutput { publications: Vec<PublicationRow> }` | svc |
| `directory.search` W | `SearchQuery` (D-A2c-6) | `SearchOutput { hits: Vec<SearchHit>, truncated: bool, directory: String, answered_at_secs: u64 }` | core / `m::d` |
| `directory.limits` | `NoParams` | `PublicationLimits` | core (exists) |
| `directory.set-limits` | `PublicationLimitsInput` | `PublicationLimits` | core (moved) |
| `directory.reindex` | `NoParams` | `ReindexOutput { rebuilt: u64, standing: u64 }` | svc `search_ops.rs` |
| `directory.export` | `NoParams` | `Bundle` | core |
| `directory.import` | `ImportInput` | `DirectoryImportOutput { imported: BTreeMap<String, usize>, reindexed: u64, standing: u64 }` | svc `backup.rs` |
| `credential.issue` | `CredentialIssueInput { member_did, categories: Vec<String>, area: Option<Vec<Area>>, expires_at_secs: u64, note: Option<String> }` | `SignedRecordOutput` | svc `credential_ops.rs` |
| `credential.list` | `MemberFilterInput` | `IssuedRecordsOutput { records: Vec<IssuedRecordRow> }` | svc |
| `revocation.issue` | `RevokeInput { credential_record_id, reason: Option<String> }` | `SignedRecordOutput` | svc |
| `revocation.list` | `MemberFilterInput` | `IssuedRecordsOutput` | svc |
| `member.suspend` | `SuspendInput { member_did, rule: Option<String>, reason: Option<String>, until_secs: Option<u64>, scope: Option<ModerationScope> }` | `SignedRecordOutput` | svc `moderation_ops.rs` |
| `member.lift` | `LiftInput { decision_record_id, reason: Option<String> }` | `SignedRecordOutput` | svc |
| `member.decisions` | `MemberFilterInput` | `IssuedRecordsOutput` | svc |
| `directory.standing` W | `StandingInput { member_did: String }` | `StandingOutput { issuer_did: Option<String>, member_did, evidence: MembershipEvidence, answered_at_secs: u64 }` | `m::d` |
| `directory.add-source` | `AddSourceInput { did, label: Option<String>, issuer_did: Option<String> }` | `AddSourceOutput { source: SourceRow, probe: Option<String> }` (`probe` null) | svc `client_sources.rs` |
| `directory.probe-info` | `DidInput` | `Option<DirectoryInfoOutput>` (D-A2c-9) | svc / `m::d` |
| `directory.remove-source` | `DidInput` | `RemovedOutput` | svc |
| `directory.sources` | `NoParams` | `SourcesOutput { sources: Vec<SourceRow> }` | svc |
| `directory.start-run` | `NoParams` | `StartRunOutput { run_id, sources: Vec<String>, max_concurrency: usize }` | svc `client_query.rs` |
| `directory.query-source` | `QuerySourceInput { run_id, source, #[serde(default)] query: SearchQuery }` | `QuerySourceOutput` (D-A2c-11) | svc |
| `directory.merge` | `RunInput { run_id }` | `MergeOutput { hits: Vec<MergedHit>, hits_truncated: bool, refused: Vec<RefusedHit>, refused_truncated: bool }` | svc `client_merge.rs` |
| `directory.run-envelope` | `RunEnvelopeInput { run_id, record_id }` | `Option<RunEnvelopeOutput { envelope }>` | svc |
| `directory.check-standing` | `CheckStandingInput { source, member_did }` | `CheckStandingOutput` (D-A2c-11) | svc `held.rs` |
| `directory.memberships` | `MemberFilterInput` | `MembershipsOutput { memberships: Vec<HeldMembershipView> }` | svc |
| `directory.publish-to-source` | `PublishToSourceInput { source, listing_id }` | `PublishOutput` (D-A2c-9) | svc / `m::d` |
| `directory.signing-status` | `NoParams` | `SigningStatusOutput` | core |
| `directory.install-signing-certificate` | `InstallCertificateInput` | `InstallCertificateOutput` | core |

The merge types copy `build_hits` and `build_refused` exactly
([client_merge.rs:196-259](../../../../crates/roym_directory/src/app/client_merge.rs#L196)):
`MergedHit { listing_id, record_id, issuer, title, summary, categories:
Vec<String>, conversation_address, status: String, verified: bool,
revocation_status: String, age_secs: u64, sources: Vec<HitSource>,
versions_differ: bool }` (`verified` is always `true`, as today),
`HitSource { directory, record_id, received_at_secs: u64, membership:
Option<MembershipVerdict> }` (no skip), `RefusedHit { listing_id, reason:
String, sources: Vec<String> }`. `HeldMembershipView { source, member_did,
issuer_did: Option<String>, as_of_secs: u64, last_error: Option<String>,
verdict: MembershipVerdict }` copies `held.rs:248-255` (no skips).

**web (1 method)**

| Method | Input | Output | Where |
| --- | --- | --- | --- |
| `session.whoami` | `NoParams` | `WhoamiOutput` (D-A2c-15) | `roym_web` `app.rs`, row in `api.rs` |

After A2c: profile 24, conversation 24, catalog 17, transaction 41,
directory 37, web 1. 144 methods.

## 6. Step-by-step changes

**Rule for every commit:** the workspace builds, and the checks of section
7.1 pass at the end of it. The shape files of Step 2 stay green from Step 2
to the end. Rebuild the WASM components with `mise run build:roym` before
you run the parity suite, the contract test or the substrate e2e tests
(they load built artifacts; a stale artifact runs the old code).

### Step 1 — Commit this plan

- Commit this file and the `status.md` row: `docs(planning): plan M08A
  slice A2c`. Done by the planning session.

### Step 2 — The output shapes from the unchanged code

**Commit:** `test(roym-web): record directory and whoami shapes`.

- `crates/roym_web/tests/method_contract.rs`: add `#[path =
  "method_contract/shapes_directory.rs"] mod shapes_directory;` (always
  with `#[path]`, A2a P19).
- `crates/roym_web/tests/method_contract/shapes.rs`: add `"auth"` to
  `is_closed_word_key` (D-A2c-18). Reuse `rpc`, `assert_shape` and the
  existing helpers; do not copy them.
- New `shapes_directory.rs`. Build state with the existing parity helpers
  (`trust_fixtures::ensure_synorg`, `enrol_signing`,
  `publish_listing_to_primary`, the credential helpers of
  `trust_fixtures.rs`, and the canned and foreign sources of
  `trust_harness.rs` and `helpers.rs`). If a set-up is needed twice, put it
  in one helper (A2b R9). One test function per area:
  `server_shapes`, `credential_shapes`, `client_shapes`,
  `standing_shapes`, `whoami_shapes`. Required cases:
  - server: `directory.ping`; `directory.settings` missing and found;
    `directory.set-settings`; `directory.info` with no SynOrg (`null`) and
    with one; `member.add`; `member.remove` found and missing;
    `member.list` empty and with rows; `directory.publish` published and
    withdrawn (a signed withdrawal); `directory.unpublish`;
    `directory.publications` empty and with a row; `directory.search` with
    no hit and with a hit; `directory.limits`; `directory.set-limits`;
    `directory.reindex`; `directory.export`; `directory.import`; both
    certificate verbs.
  - credentials: `credential.issue`; `credential.list` all and filtered;
    `revocation.issue` new and repeated (the early return);
    `revocation.list`; `member.suspend` with and without `until_secs`;
    `member.lift`; `member.decisions`.
  - client: `directory.add-source` with an issuer-mismatch `probe` note
    and without a note; `directory.probe-info` for a directory. **Do not
    record** `probe-info` or `add-source` against a node with no SynOrg
    here: the unchanged code answers `{}` and no note, which is the bug
    B57. Step 5 records those two shapes on the fixed code;
    `directory.remove-source` found and missing; `directory.sources`;
    `directory.start-run`; `directory.query-source` with hits, with a
    `SourceError` (an unbound source), and the `hTrunc` source;
    `directory.merge` with hits and refused evidence (a `hForge1` source);
    `directory.run-envelope` found and missing;
    `directory.publish-to-source` accepted.
  - standing: `directory.standing`; `directory.check-standing` refreshed,
    issuer-changed (no `as_of_secs` key) and stale (unreachable source,
    with `error`); `directory.memberships`.
  - whoami: `session.whoami` on `/rpc` with a delegated caller and with no
    caller, and through `invoke` (the anonymous form).
- Record the files on the **unchanged** code:
  ```bash
  mise run build:roym > target/a2c-build.log 2>&1; echo "exit=$?"; tail -n 3 target/a2c-build.log
  UPDATE_METHOD_SHAPES=1 cargo nextest run -p syneroym-roym-web --test method_contract shapes
  ```
- **Proof:** `cargo nextest run -p syneroym-roym-web --test method_contract
  shapes` passes with no env var. `git diff --stat
  crates/roym_web/tests/method_shapes/` shows only new files: the 157 files
  of A2a and A2b do not change with the new `auth` key. If one does change,
  find out why before you go on. The parity suite still has 222 scenarios
  and passes.

### Step 3 — Shared types and the three small parts in `roym_core`

**Commit:** `feat(roym-core): add directory wire method types`.

- `crates/roym_core/src/method.rs`: add `WireNoParams` (derive
  `Deserialize`, `Serialize`, `Default`; no `deny_unknown_fields`; a doc
  comment that says why). Move `PublicationLimitsInput` here from
  `crates/roym_catalog/src/app/limits.rs` and change its two users in the
  catalog. Add `pub mod directory;`.
- New `crates/roym_core/src/method/directory.rs` with the six types of
  D-A2c-2 and the attributes of D-A2c-11 and D-A2c-13.
- `crates/roym_core/src/directory.rs`: `SearchQuery.open_to:
  Option<OpenTo>`, `.booking_mode: Option<BookingMode>` (D-A2c-6). Keep
  `#[serde(default)]` on both. The build of `directory` then needs the
  Step 4 change of `build_search_filter` in the same commit: do that one
  edit here (delete the word check, push `serde_str(&v)`), so the
  workspace builds.
- `crates/roym_core/src/method.rs`: split `read_reply` out of
  `call_sibling` (D-A2c-9).
- `crates/roym_core/src/admit.rs`: add `caller` (Q1) and make `admit` use
  it. Unit test: `caller_maps_the_three_origins`.
- `crates/roym_core/src/method/check.rs`: the `admit::admit` call
  allowance (D-A2c-16).
- Unit tests in a new `crates/roym_core/src/method/tests/directory.rs`,
  declared from `method/tests.rs` with `mod directory;` (`method/tests.rs`
  is 725 lines; six more tests would take it past 800). Names by
  behaviour:
  `read_reply_reads_a_null_result_as_none_and_keeps_error_data`,
  `check_allows_req_method_only_in_the_admit_call`,
  `wire_no_params_ignores_an_unknown_key`,
  `search_query_ignores_an_unknown_key_and_refuses_an_unknown_open_to_word`,
  `publish_output_writes_both_shapes_and_reads_them_back`,
  `search_output_reads_an_answer_with_only_hits_and_truncated`,
  `standing_output_reads_an_answer_with_an_extra_key`,
  `directory_info_output_reads_an_answer_with_only_name_and_counts`.
- **Proof:** `cargo nextest run -p syneroym-roym-core -p
  syneroym-roym-catalog` passes. `cargo build -p syneroym-roym-core
  --target wasm32-wasip2` builds.

### Step 4 — Convert the server half of `directory`

**Commit:** `refactor(roym-directory): type the server half`.

- New `crates/roym_directory/src/api.rs` with `WIRE_REACHABLE` (moved,
  with its comment), `ping`, and `service_methods!` with the server-half
  rows of section 5: `ping`, `settings`, `set-settings`, `info`,
  `member.add`, `member.remove`, `member.list`, `publish`, `unpublish`,
  `publications`, `search`, `limits`, `set-limits`, `reindex`, `export`,
  `import`, the seven credential and moderation rows, `standing`, and the
  two certificate verbs (26 rows). `pub fn method_table() ->
  Vec<MethodInfo>`, as in the other services. Add `pub mod api;` to
  `lib.rs`.
- `app.rs` `invoke` (temporary, until Step 5):
  ```rust
  if let Err(resp) = admit::admit(host, api::WIRE_REACHABLE, &req.method).await { return resp; }
  if let Some(resp) = api::dispatch(host, &req).await { return resp; }
  match req.method.as_str() {
      // only the client-half arms not yet listed
      other => Response::method_not_found(other),
  }
  ```
  `handle_certificate_verb` is no longer called.
- Every server-half handler in `synorg.rs`, `publication_ops.rs`,
  `search_ops.rs`, `standing.rs`, `credential_ops.rs`, `moderation_ops.rs`
  and `backup.rs` changes to `pub(crate) async fn name<H: AppHost>(host:
  &H, input: Input) -> Reply<Output>`. Add the structs of section 5 beside
  them and the shared ones in `app.rs`. Rules:
  - Delete every `req.params.get(..)`, `from_value(req.params..)` and
    `parse_issue_params` / `parse_scope`.
  - Keep the order of the checks that run after the parse, every other
    error text, and every comment that explains why (A2b R8). Keep the
    empty-string checks of D-A2c-5.
  - `directory.limits`: a function in `publication_ops.rs`.
  - `directory.publish` (D-A2c-7): `let caller = admit::caller(host).await;`
    then `resolve_published_by` as today. `withdraw_publication` returns
    `PublishOutput { record_id: None, withdrawn: Some(true), .. }`; the
    normal path returns `record_id: Some(..), withdrawn: None`. The
    `Err(..)` refusals with `data` do not change.
  - `credential.issue`: `note` comes from the input, and is still passed to
    `upsert_member` at the same place (after the record is stored).
  - The list paths and `directory.settings`: D-A2c-12.
  - `directory.export`: `backup::export_signed_bundle`. `directory.import`:
    `ImportInput`; the rest as today.
  - Make `PublicationRow`, `IssuedRecordRow` and the other output types
    `pub(crate)` where `api.rs` must name them.
- `roym_core`: delete `handle_certificate_verb` and `export_signed`, and
  change their test and comment (D-A2c-14).
- `synorg.rs`: there is no `synorg/` directory today. Create
  `synorg/tests.rs` and declare it from `synorg.rs` with `#[cfg(test)] mod
  tests;` (module layout rule: `synorg.rs` + `synorg/`). Add the unit test
  `settings_input_matches_settings_fields`: fill every field of
  `SynOrgSettingsInput`, call `into_settings`, and compare the JSON of the
  input with the JSON of the result.
- Rebuild WASM: `mise run build:roym`.
- **Proof:**
  ```bash
  cargo clippy -q --workspace --all-targets --all-features
  cargo nextest run -p syneroym-roym-core -p syneroym-roym-directory > target/a2c.log 2>&1; echo "exit=$?"; tail -n 5 target/a2c.log
  cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2c-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2c-par.log
  grep -rn "handle_certificate_verb\|export_signed(" crates/ | grep -v '/target/'
  ```
  The last command prints nothing. The shape files and all 222 parity
  scenarios pass.

### Step 5 — Convert the client half of `directory`

**Commit:** `refactor(roym-directory): type the client half`.

- Add the remaining eleven rows to `api.rs` (37 in all). `invoke` becomes
  the final form:
  ```rust
  pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
      if let Err(resp) = admit::admit(host, api::WIRE_REACHABLE, &req.method).await {
          return resp;
      }
      if let Some(resp) = api::dispatch(host, &req).await {
          return resp;
      }
      Response::method_not_found(&req.method)
  }
  ```
- Handlers in `client_sources.rs`, `client_query.rs`, `client_merge.rs`
  and `held.rs` change to the typed signature. Rules as in Step 4, plus:
  - `remote_reply` and the two pass-through methods (D-A2c-9), and the
    `Value` read of the probe reply in `add_source` (B57).
  - `query_source`: `refuse_source` returns `QuerySourceOutput` (it stays
    an `Ok` answer, as today). The remote `directory.search` call and its
    read do not change (D-A2c-10).
  - `check_standing`: `handle_standing_reply` and `stale_or_unknown`
    return `CheckStandingOutput` with the three settings of D-A2c-11.
  - `merge`: `build_hits` returns `Vec<MergedHit>`, `build_refused`
    returns `Vec<RefusedHit>`.
- Delete the imports that are no longer used (`Value`, `json`, `Request`
  where they go away).
- Rebuild WASM.
- **Proof:** the commands of Step 4, and then:
  ```bash
  cargo nextest run -p syneroym-substrate -E 'binary(~roym)' > target/a2c-e2e.log 2>&1; echo "exit=$?"; tail -n 5 target/a2c-e2e.log
  grep -rn "params\.get\|req\.params\|Response::ok" crates/roym_directory/src | grep -v bindings.rs
  ```
  Check in `target/a2c-e2e.log` that the substrate `roym_*` tests ran and
  were not skipped (they return early when `roym_artifacts_present()` is
  false). The grep prints nothing.
- Record two new shape files on the fixed code (B57), with
  `UPDATE_METHOD_SHAPES=1` for these two cases only: `directory.probe-info`
  against a node with no SynOrg (`{"result": null}`) and
  `directory.add-source` of that node (with the note). Use
  `hForeignWire2` before `directory2` has a SynOrg, as in Step 7. Every shape file of
  Step 2 stays unchanged.

### Step 6 — `web`

**Commit:** `refactor(roym-web): type session.whoami`.

- `crates/roym_web/src/app.rs`: `WHOAMI`, `SessionAuth`, `WhoamiOutput`,
  `whoami` (D-A2c-15). `handle_whoami_rpc` takes the params and the id:
  it parses `NoParams` first and answers the JSON-RPC error with `data` on
  a bad input. `invoke` parses `NoParams` before it answers.
- New `crates/roym_web/src/api.rs` with `method_table()`; `pub mod api;`
  in `lib.rs`.
- No change to `forward_rpc_call`, `admit`, `handle_http`, the WebSocket
  hooks or `status`.
- Rebuild WASM.
- **Proof:** `cargo nextest run -p syneroym-roym-web --test
  dual_build_parity` passes (scenario 3 reads `did` and `auth` as before).

### Step 7 — Parity harness: the table, and a newer node

**Commit:** `test(roym-web): drive directory parity from the table`.

- `trust_fixtures.rs`: `all_directory_verbs()` in place of
  `ALL_DIRECTORY_VERBS` (Q6, D-A2c-17). `directory.rs` scenario 118: the
  three-line change of D-A2c-17, no added line. The scenario keeps its
  name.
- `trust_harness.rs`: a canned source `did:key:hNewer` in
  `trust_source_response`, the "newer node". It answers
  `directory.search` with one validly signed hit, an extra top-level key
  `"ranking": "v2"` and an extra key in the hit; `directory.info` with a
  normal info and `"extra": 1`; `directory.standing` with a normal answer
  and `"extra": 1`; `directory.publish` with `{listing_id, record_id,
  "receipt": "x"}`. Its `info` and `standing` answers name the same
  `issuer_did`, so `add-source` pins it and `check-standing` does not
  answer "issuer-changed". Reuse the signing helpers already in that
  file.
- New parity module `crates/roym_web/tests/dual_build_parity/directory_compat.rs`,
  declared in `dual_build_parity.rs` with `#[path]` like the others (not in
  `directory.rs`, which is at its cap; 7.4). Two scenarios. The first:
  `scenario_229_a_newer_node_answer_with_unknown_keys_is_read_parity`.
  It adds `hNewer` as a source and checks on both builds:
  `query-source` stores the hit (`verified: 1`, `error: null`);
  `check-standing` is `refreshed: true`; `probe-info` answers with the
  known keys and without `"extra"`; `publish-to-source` answers
  `{listing_id, record_id}` without `receipt`. 229 is the next free
  number (the highest today is 228: `grep -rho 'fn scenario_[0-9]*'
  crates/roym_web/tests/dual_build_parity | sort -t_ -k2 -n | tail -1`);
  check again before you write it.
- A second scenario in the same module,
  `scenario_230_a_source_with_no_synorg_is_noted_and_probes_as_null_parity`
  (B57): add `did:key:hForeignWire2` as a source before the second
  directory has a SynOrg (`helpers::foreign_wire_route` sends it to
  `directory2` over a real wire round trip, so that directory's
  `directory.info` answers `{"result": null}`), and check on both builds that `add-source` returns the "runs no directory"
  note and `probe-info` answers `{"result": null}`.
- The parity count becomes 224. `dual_build_parity.rs` then declares 25
  modules (20 with scenarios). Record both in `status.md` in these words.
- **Proof:** `cargo nextest run -p syneroym-roym-web --test
  dual_build_parity` passes. `grep -c "async fn scenario_"` over the
  folder gives 224.

### Step 8 — The check test and the error tests

**Commit:** `test(roym-web): check every directory and web method`.

- `crates/roym_web/tests/method_contract/contract.rs`:
  - `every_directory_method_has_typed_input_and_output`:
    `assert_service_table(services::DIRECTORY, &directory_api::method_table(),
    &wire, &Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_directory/src"))`
    with `wire` the names of `directory_api::WIRE_REACHABLE`. Assert 37
    rows, and that the sorted `wire` names equal the sorted hand list
    `trust_fixtures::WIRE_REACHABLE_DIRECTORY_VERBS` (D-A2c-17).
  - `web_has_one_typed_method_of_its_own`: `web_api::method_table()` has
    one row, named `session.whoami`; `router::route("session.whoami")` is
    `None`; neither type name contains `serde_json::value::Value`; the
    row's `probe` refuses `{"__probe__": 1}` with "unknown field". Say in
    a comment why `web` is not run through `assert_service_table` (its
    one method is not routed to a sibling, and its forwarder reads
    `req.method` on purpose).
  - Extend `every_listed_method_is_dispatched` to the directory table.
  - Extend `every_listed_method_is_refused_off_the_node_before_parsing`:
    for directory, a local-only row still answers `-32013` to `{"zz": 1}`
    over the wire; a wire row answers something else, and if it is an
    error, its `data.field` is not `"zz"` (OF-18 on the real wire path,
    both builds). Rename the test if its name then says too little, for
    example `every_method_answers_off_the_node_as_its_wire_rule_says`.
  - Error tests, each through `post_rpc` (both builds, `wasm == native`),
    or through `h.wire_invoke` for the wire cases. **Each request has
    exactly one error** (A2b R6): build it from a valid request and add or
    break one key.
    - `member.add` with an unknown key `colour` gives `data.field ==
      "colour"`.
    - `credential.issue` (valid otherwise) with `expires_at_secs: "soon"`
      gives `data.field == "expires_at_secs"`.
    - `member.suspend` (valid otherwise) with `until_secs: -1` gives
      `data.field == "until_secs"`.
    - `directory.set-settings` (valid settings) with an unknown top-level
      key gives `data.field == <key>`; the same settings with an unknown
      key inside `publication_limits` is accepted (D-A2c-3).
    - `directory.query-source` (valid run and source) with `query:
      {"open_to": "everyone"}` gives `data.field == "query.open_to"`, and
      the source was not called (B46). Pin the message text once.
    - `directory.search` over the wire with `open_to: "everyone"` gives
      `data.field == "open_to"`.
    - `directory.search` over the wire with `{"text": "hedge", "zz": 1}`
      gives a result.
    - `directory.standing` over the wire with a valid `member_did` and
      `"zz": 1` gives a result.
    - `directory.info` over the wire with `{"zz": 1}` gives a result.
    - `directory.publish` over the wire with a valid signed envelope and
      `"zz": 1` gives `{listing_id, record_id}` (OF-18's real case: a newer
      node publishes). Reuse the envelope set-up of the `hNewer` scenario.
  - `a_wire_publish_is_recorded_under_the_verified_caller` (Q1, B53), both
    builds: a wire `directory.publish` through `wire_invoke_as` with
    `foreign_searcher_caller()` (a verified DID that is not the owner),
    then a local `directory.publications`: its `published_by` is
    `did:key:zForeignSearcher`. Then the same caller publishes until the
    limit refuses it with `data.admission == "rate-limited"`, and a local
    publish by the owner after that is still accepted (the limit is keyed
    on the caller, not the owner). Today's scenario 113 checks only a local
    publish.
    - `session.whoami` with `{"zz": 1}` on `/rpc` gives `-32602`,
      `data.field == "zz"`, and the request `id`.
  - `contract.rs` is 367 lines. Put the directory and web error tests in a
    new `method_contract/contract_directory.rs` (declared with `#[path]`).
- **Proof:** `cargo nextest run -p syneroym-roym-web --test
  method_contract` passes. Once, by hand, add a hand-written
  `"directory.info" =>` arm to `crates/roym_directory/src/app.rs` and see
  the directory table test fail; remove it. Once, by hand, add
  `#[serde(deny_unknown_fields)]` to `StandingInput` and see the table test
  fail with "is in wire, but rejects unknown field"; remove it. Say both in
  the PR (A2a R1 showed why).

### Step 9 — Docs and close-out

**Commit:** `docs: describe typed directory and web methods`.

- Section 9 lists the files. Section 8 lists the backlog rows.
- No planning ids in names or comments of the new tests and shape files.

### Step 10 — Finish

- Import clean-up pass over every edited Rust file (types by `use`,
  functions by their parent module, no paths with more than one `::`
  inline; A2a R9, A2b R3). `moderation_ops.rs` and `standing.rs` use
  `use super::*;` today; keep that, but import every new type by name.
- `cargo +nightly-2026-04-06 fmt --all`.
- Ratchets: `cargo xtask check-file-lengths` (`dual_build_parity/directory.rs`
  must still be 1135 lines; `trust_fixtures.rs` is not on the oversized
  list, so its shrink records nothing),
  `cargo xtask check-duplication` (the hand list and the hand word check
  go away; record the new value if it improved).
- `cargo xtask check-module-layout`, `cargo xtask check-dep-versions`,
  `cargo shear`.
- The `cargo dupes` recipe from AGENTS.md over the changed files.
- `mise run verify` as a background job, no `--skip`, every gate green.
- Open the PR with the CLA line and the attribution line. The PR text lists
  the commits, the method tables, the audit table (section 4), the wire
  change list, the table of section 3.6, the two manual proofs of Step 8,
  and the exact counts (shape files new and total, parity scenarios).

## 7. Tests

### 7.1 After every commit

```bash
cargo clippy -q --workspace --all-targets --all-features
cargo nextest run -p syneroym-roym-core -p syneroym-roym-directory -p syneroym-roym-catalog > target/a2c.log 2>&1; echo "exit=$?"; tail -n 5 target/a2c.log
cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2c-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2c-par.log
```

After any `roym_*` code change, rebuild with `mise run build:roym` first.
After Step 5, also the substrate e2e of Step 5 and the Hub unit tests
(`npm test` in `crates/roym_web/ui`; no Hub file changes, so they prove
nothing broke in the mocked shapes).

### 7.2 What each test proves

1. **Shared types (Step 3).** `WireNoParams` and `SearchQuery` ignore an
   unknown key; `SearchQuery` refuses an unknown `open_to` word;
   `PublishOutput` writes both shapes and reads them back; the wire output
   types read a partial or a larger answer. `admit::caller` maps the three
   origins. The check allows the `admit::admit` line and still refuses
   another `req.method` use on the same line. `read_reply` reads a `null`
   result as `None` and keeps an error's `data`. `SynOrgSettingsInput` has
   every field of `SynOrgSettings`.
2. **OF-15 (Step 8).** The directory table has 37 rows, typed input and
   output, no `Value` as the outer type, a route to `directory` for each,
   the right probe answer for each of the four wire rows and the 33
   local-only rows, and no hand-written arm or stray `req.method` in the
   source. `web` has its one typed row. A name not listed answers
   `-32601`.
3. **OF-14 (Step 8).** A wrong type at the top level, a wrong type inside a
   nested input (`query.open_to`), and unknown keys give `-32602` with
   `data.field`, equal in both builds, on `/rpc`, through `invoke`, and
   over the wire.
4. **OF-18 (Step 8 and Step 7).** Every local-only row refuses `{"zz": 1}`
   off the node with `-32013` before the parse. Every wire row, called
   over the wire with `{"zz": 1}`, does not refuse the key. A newer node's
   answers with unknown keys are read (the `hNewer` scenario). The
   parity scenario 118 keeps the four-name security list by hand. A wire
   `info` and a wire `publish` with an extra key are answered.
5. **Q1, the caller of `directory.publish` (Step 8).** A wire publish is
   recorded and rate-limited under the verified caller's DID, in both
   builds; the owner's budget is untouched.
6. **B57 (Step 5 and Step 7).** A node with no SynOrg probes as `null`, and
   `add-source` of it returns the note.
7. **No behaviour change.**
   - The shape files, recorded from the old code in Step 2 and passing at
     every later step. Every shape of the multi-shape methods of section
     3.3 has its own file, including issuer-changed (no `as_of_secs`).
   - The parity harness: 222 scenarios before Step 7, 224 after, the same
     two-build equality. Only scenario 118's loop, `contains` line and
     message text change.
   - The substrate e2e tests for the directory and trust flows, which send
     raw JSON and read results, codes and `data` by key.
   - The Hub unit tests and the Playwright specs.
   - The greps of Step 4 and Step 5 print nothing.

### 7.3 Not tested

- A wrong type for every field of every method. The mechanism is one
  function, tested in A2a; the check probes every row.
- A real older or newer build. The tests send what such a node would send
  (an extra key, a partial answer) through the real wire path of the parity
  harness. There is no second build to run.
- A malformed stored row from a hand-made bundle (B49). The mappings are a
  skip and a `-32603`; a test would need a hand-made signed bundle.
- A remote answer that is not JSON, or that has neither a `result` key
  nor an `error` (B51). No directory built from this repository sends one
  (`Response::ok(Value::Null)` is written as `{"result": null}`, which is
  tested, B57). The branches are those of `read_reply`, which
  `call_sibling` already uses.
- B53 (the caller read inside `directory.publish` for an anonymous
  caller): `admit` refuses it first, and scenario 118 proves that.

### 7.4 Size risks

- `publication_ops.rs` 568 lines, `search_ops.rs` 576, `client_query.rs`
  455, `credential_ops.rs` 350. Structs add lines; deleted parse code
  removes some. If a file would pass 800, put its structs in a sibling
  module (`publication_ops/types.rs` with `mod types;`).
- `dual_build_parity/directory.rs` is 1135 lines and on the oversized
  list at exactly that cap (`xtask/oversized-test-files.txt:21`). It must
  not get one line longer: the `hNewer` scenario goes in the new module
  `directory_compat.rs` (Step 7), and scenario 118 changes three lines in
  place (D-A2c-17). `trust_fixtures.rs` shrinks by about 35 lines.
- `contract.rs` 367 lines: the new tests go in `contract_directory.rs`
  (Step 8).
- `roym_core/src/method/tests.rs` 725 lines, not on the oversized list:
  the new unit tests go in `method/tests/directory.rs` (Step 3).
- `service_methods!` with 37 rows. It did not trip `too_many_lines` with 41
  rows in A2b. If it does, split the table as A2b section 7.4 describes and
  record a deviation.

### 7.5 How A2c avoids the kinds of findings A2a and A2b had

| Earlier finding | The risk in A2c | What this plan does |
| --- | --- | --- |
| A2a P3, P6, P7 (shape holes; `null` vs absent) | multi-shape outputs; the issuer-changed answer has no `as_of_secs` | Section 3.3; D-A2c-11; a shape case for every shape (Step 2). |
| A2a P4, A2b B33 (sibling error handling) | the pass-throughs and remote errors | D-A2c-9 gives each branch; remote errors pass unchanged. |
| A2a P12, P19 (test binary modules) | new modules | Step 2 and Step 8 name each `#[path]` line. |
| A2a R1 (a check that silently does nothing) | the new wire branch; a stale WASM | Two manual proofs in Step 8; `CARGO_MANIFEST_DIR` paths; rebuild before every run; the e2e log check. |
| A2a R2, R3; A2b R1, R12 (change doc decisions and deviations) | 19 new decisions | Section 9: D-A2c-1 to D-A2c-19 with the same numbers and meaning; planned wire changes and real deviations in their two lists. |
| A2a R6 (nested types not checked) | `query` inside `query-source`; blocks inside settings | D-A2c-3 names each; Step 8 tests one nested refusal and one nested ignore. |
| A2a R8 (living docs say more than is true) | "every method is typed" after A2c | Section 9 says what `web` does and does not type. |
| A2a R9, A2b R3 (inline paths) | many edits | Step 10 import pass. |
| A2a R10, A2b R7 (numbered comments) | error tests | Named test functions; no numbers. |
| A2b P1, P2 (an order of checks the plan missed) | `publish`, `credential.issue` `note` | B41, B53. |
| A2b P3 (a typed read that drops a newer peer's keys) | the pass-throughs | Q3 states the cost; D-A2c-10 leaves the other remote reads as they are. |
| A2b R2 (wrong counts) | shapes, scenarios, modules | Step 2, Step 7 and 7.4 say what to count and where to record it. |
| A2b R5 (a behaviour change hidden in a rewrite) | the handlers of two halves | "Keep the order of the checks that run after the parse"; no rewrite beyond types; the reviewer reads `publish`, `check-standing` and the pass-throughs against `main`. |
| A2b R6 (a test that sends more than one error) | the error tests | "Each request has exactly one error" (Step 8). |
| A2b R8 (deleted why-comments) | many handlers with long comments | Section 0 rule; Step 4 rule. |
| A2b R9 (set-up written twice) | directory shapes need sources and credentials | Step 2: one helper per set-up, reuse `trust_fixtures`. |
| A2c P1 (a `null` result lost by a typed read) | every read of a remote reply | `read_reply`, shared with `call_sibling`; B57 fixed and tested. |
| A2c P2 (a security read with no direct test) | the caller of `directory.publish` | The attribution test of Step 8. |

## 8. Deferred backlog

All rows are in [deferred-backlog.md](../../deferred-backlog.md).

- **Q7 (a): split the §10 row "Roym's sibling boundary is untyped JSON,
  with no WIT-level contract between services".** Move it to "Recently
  resolved" with the PR and this text: "All six Roym services take and
  return typed structs (`web` types its one own method and forwards the
  rest to the service that owns it); sibling calls use the structs." Add a
  new §10 row: "**Roym services have no WIT-level contract between them,
  and the `status` export is untyped.** The method set grows with the
  product, and a WIT function per method makes each verb a
  deployed-interface change. Target: TBD. Source:
  `crates/roym_core/src/method.rs`."
- **Append to the §10 row "The Roym API contract is not published":** the
  `directory` and `web` tables can also feed a schema generator; the four
  wire types are in `roym_core::method::directory`.
- **Append to the §10 row "Public contract versioning (`[G5]`)":** a newer
  node that adds a word to a closed set inside an answer (a new
  `AreaMatch` kind, a new membership word, a new `Area` kind in `info`)
  makes the whole answer unreadable for an older node: the source is
  `unreadable`, `check-standing` falls back to the stored copy, and
  `probe-info` answers `-32603` (B58). A4's federated query library could
  read hits one by one.
- **New row, §10: "Local-only methods refuse unknown fields, which assumes
  a client built with the node".** OF-18 refuses an unknown field on a
  local-only method because "the caller is the node's own client, built
  with the node". Remote clients (A6) and third-party clients (A7) break
  that: a newer client that adds a field fails against an older node with
  `-32602`. Reason: the rule was set before remote clients existed.
  Target: decide in A6 or A7 (the client guide must say it). Source:
  [roym-open-foundation](../../changes/roym-open-foundation/change.md)
  "Typed methods".
- **New row, §10: "The directory's client half reads remote answers by
  hand".** `query_source` reads `directory.search`, `add_source` reads
  `directory.info` and `handle_standing_reply` reads `directory.standing`
  with `Value` and `.get`, and the outbound calls are hand-built
  `host.call`s (D-A2c-10). Reason: A4 moves the search read into the
  federated query library, and the calls use `CallTarget::Service`
  options that `call_sibling` does not take. Target: A4. Source:
  `crates/roym_directory/src/app/client_query.rs`,
  `client_sources.rs`, `held.rs`.
- **Resolved in code, no row needed:** the hand list `ALL_DIRECTORY_VERBS`
  (B59), the hand word check of `open_to` / `booking_mode`, and the
  "runs no directory" note that never showed (B57).
- No new open `TODO` or `FIXME` marker is added.

## 9. Living docs to update at close-out

Check each by asking: is a statement now false or incomplete? Do not cite
change docs, milestone or slice ids in these files. Link ADRs only.

- [docs/developer-guide.md](../../../developer-guide.md) "Add a Roym
  method" (around line 1687): replace "Today `profile`, `conversation`,
  `catalog` and `transaction` have a method table; the other services
  (`directory`, `web`) still read their params by hand" with: every Roym
  service has a method table in its `api.rs`; `web` lists only its own
  `session.whoami` and forwards every other method, with its `params`
  unchanged, to the service that owns it. In step 1, add: a wire-reachable
  method's input and output types go in `roym_core::method::directory`
  (or the module of their service there), because the other node reads
  them; a wire method with no params uses `WireNoParams`, not `NoParams`;
  an output that another node reads never has `deny_unknown_fields`. In
  step 1 or 4, add: the wire list is the service's `WIRE_REACHABLE`
  constant, which `admit::admit` also reads. In "Bad input", change
  "Wire-reachable methods ignore unknown fields so newer clients can talk
  to older nodes" to "... so that a newer node can call an older one; a
  renamed or removed field still breaks the call". Also add there: record
  types inside an input (for example `SearchQuery` inside
  `directory.query-source`'s `query`) ignore unknown keys, so a misspelled
  nested key is dropped, not refused. A remote reply is read with
  `method::read_reply` (or `call_sibling`), never by parsing it as
  `Response`, because a `null` result would be lost.
- [docs/system-architecture.md](../../../system-architecture.md): at
  line 1143, the same correction as the developer guide's first sentence.
  At line 1141 (the paragraph that names the four wire methods), one
  sentence: these four methods ignore unknown fields, so a newer node can
  call an older one.
- [docs/system-requirements-spec.md](../../../system-requirements-spec.md)
  (around line 710, "Explicit Wire Exceptions"): check, and update if
  false. Today it names the three `Open` methods and the `VerifiedOnly`
  one and says nothing about bad input, which stays true. If an edit is
  made, add the file to the change doc's `living-docs-touched`.
- [docs/planning/traceability-matrix.md](../../traceability-matrix.md) rows
  `[ROY-TYP]`, `[ROY-UNK]`, `[ROY-CHK]` (lines 91-93): "(profile,
  conversation, catalog and transaction)" becomes "(all six services;
  `web` has its one own method)"; the evidence names the two new `api.rs`
  files. `[ROY-UNK]` also says that the four wire-reachable directory
  methods ignore unknown fields (OF-18's second half), with
  `contract_directory.rs` as evidence.
- [docs/roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md):
  only if it states how bad input is answered or which methods are
  wire-reachable with their params (A2a and A2b found no such text; grep
  again for `-32602`, `unknown field` and `directory.search`).
- Change doc [roym-open-foundation](../../changes/roym-open-foundation/change.md):
  - **Tasks:** under "Typed input and output for every Roym method", add
    "- [x] directory and web", and tick the parent line.
  - **Decisions:** add D-A2c-1 to D-A2c-19 with the same numbers and
    meaning as section 5, and one line with the owner's answers to Q1 to
    Q7.
  - **Deviations:** under "Planned wire changes", add B41 to B57 (with the
    `hNewer` scenario for B51, Q5 for B46, and the B57 bug fix). Under
    "Different from the plan", add only what the implementation really did differently.
  - **`living-docs-touched`:** add any file above not yet listed.
  - `status` stays `draft` (A3 to A7 are not done).
- [task.md](task.md): add the A2c result line under "A2".
- [status.md](status.md): row A2c. Later steps record the PR, the
  shape-file counts (new and total), the parity count (224), "25 modules
  (20 with scenarios)", and the `method_contract` result.

## 10. Exit criteria

A2c is done when all of these are true:

1. The owner has answered Q1 to Q7, and section 5 matches the answers.
2. Every one of the 37 directory methods has an input struct and an output
   struct, and `session.whoami` has `NoParams` and `WhoamiOutput` (OF-14).
   No `req.params` is left in `roym_directory`, and no `Response::ok` in
   its handlers.
3. `method_contract` passes: the directory table has 37 rows with the four
   wire rows ignoring unknown fields and the 33 others refusing them, no
   hand-written arm; `web` has its one typed row (OF-15, OF-18).
4. A wrong type, an unknown field, and a nested wrong word give `-32602`
   with `data.field`, equal in both builds; a wire method called over the
   wire with an unknown field answers (OF-14, OF-18).
5. `method_contract shapes` passes with the files recorded from the
   unchanged code, the two B57 files recorded in Step 5, and the 157
   earlier files unchanged.
6. The parity harness passes, with 222 scenarios plus the two scenarios of
   `directory_compat.rs` (224). Only scenario 118's loop, `contains` line
   and message text changed.
7. A wire `directory.publish` is recorded under the verified caller's DID,
   in both builds (Step 8).
8. `handle_certificate_verb` and `export_signed` are gone.
   `ALL_DIRECTORY_VERBS` is gone. `WIRE_REACHABLE` is one constant read by
   `admit` and by the check.
9. No Hub or `roymctl` file changed.
10. The change doc, the developer guide, the architecture doc, the
   requirements spec (if changed), the traceability matrix, task.md, status.md and the backlog are updated as
   in sections 8 and 9.
11. `mise run verify` passes with no `--skip`.
12. One PR.

## 11. What A2c does not do

- No change to how the client half calls remote directories, or to how it
  reads `directory.search`, `directory.info` (add-source) and
  `directory.standing` answers (D-A2c-10), except the `null` read of B57.
  A3 and A4 change the search path; section 8 records the rest.
- No merge or fan-out move. The merge types stay in `roym_directory` until
  A4.
- No new wire-reachable method, and no change to which methods are
  wire-reachable or to their `WireRule`.
- No typing of `web`'s forwarder, `/health`, the WebSocket hooks or any
  `status` export. No WIT contract between services.
- No change to record types, stored rows, signed records, record
  verification or storage schema versions.
- No schema generated or published. No client package.
- No method renames and no reshaping of results. `directory.publish` and
  `directory.check-standing` keep their shapes.
- No fix for B58 (a new closed word inside a remote answer). The G5 row
  records it.
- No change to what a correct request does, or to the order of any check
  other than B41, B53 and B56, except the B57 bug fix and B55 (a wire
  search with no `params`).
- No Hub or `roymctl` change. They keep sending raw JSON (D-A2a-13).
- No new demo story.

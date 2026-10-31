# M08A slice A2c — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a2c-implementation-plan.md](slice-a2c-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-10-31

Checked against the code on `feat/m08a-a2c-typed-methods` at `eb8c38f3`
(main checkout, not a worktree). Line numbers below are at that commit. The
plan was written by Claude Opus 5.5.

**Summary:** 0 blocker, 2 major, 10 minor.

One limit of this review: I could not run code. A small test program that
would show how serde reads a JSON `null` into `Option<Value>` was refused by
the sandbox. P1 rests on how serde works, plus the code and tests I read.
Step 2 of the plan will show the same fact in one shape file. Confirm it
first.

### What was checked and found correct

- **Method lists and counts.** `directory` dispatches 35 names
  ([app.rs:193-239](../../../../crates/roym_directory/src/app.rs#L193)) plus
  the two certificate verbs: 37. The split of 26 rows (Step 4) and 11 rows
  (Step 5) adds up. 37 minus the 4 wire rows is 33 local-only rows.
- **Security: exactly four wire methods.** `admit::admit` refuses by exact
  method name against `WIRE_REACHABLE`. It returns `-32013` for anything not
  in the list. Moving the constant to `api.rs` does not widen it. The hand
  list `WIRE_REACHABLE_DIRECTORY_VERBS` plus the comparison in Step 8 is the
  right guard. Scenario 118 keeps the four-name claim by hand.
- **Q1, the caller is fixed for the whole call in both builds.**
  - WASM: `invocation::Host::caller` reads `self.invocation_origin` and
    `self.caller` of `HostState`
    ([capabilities_services.rs:86-100](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs#L86)).
    Both are set once, when the per-call store is built
    ([engine.rs:561](../../../../crates/sandbox_wasm/src/engine.rs#L561)).
    Nothing writes them afterwards.
  - Native: `NativeAppHost::caller` reads `HostInner.invocation_origin` and
    `.caller`
    ([host.rs:685-698](../../../../crates/app_host_native/src/host.rs#L685)).
    `host_with` builds a new `HostInner` per call
    ([factory.rs:312-330](../../../../crates/app_host_native/src/factory.rs#L312)).
  - Both map `Local` to `Internal` and `Wire` with `Delegated`/`Ucan` to
    `Verified(did)`. Everything else is `Anonymous`. A second read in
    `publish` therefore returns what `admit` decided on.
- **Handlers and inputs.** Every handler, line link and "reads from `params`"
  entry of section 3.2 matches the code: `set_settings`, `member_*`,
  `publish`, `search`, `set_limits`, `import`, `parse_issue_params`,
  `suspend`, `standing_verb`, `add_source`, `query_source`,
  `check_standing`, `merge`, `run_envelope`. The order of checks in
  `publish` is as the plan says (caller, then `envelope`).
- **Outputs.** The null and absent keys of section 3.3 match the `json!`
  calls: the three `check-standing` shapes, the two `publish` shapes, the
  `query-source` shapes, and `IssuedRecordRow` (it is always written by
  serializing the same struct, so a typed read gives the same JSON).
- **Callers.** I re-read the Hub calls (`screens/synorg.ts`,
  `synorg_members.ts`, `directory.ts`, `directory/search.ts`,
  `listings.ts`, `memberships.ts`), the `roymctl` calls
  (`directory.rs`, `trust.rs`, `find.rs`), the Playwright specs and the
  substrate e2e tests. Every key sent is a field of the planned structs. No
  caller sends `open_to` or `booking_mode` to a directory (the matches in
  the Hub are listing editor fields). No code reads the changed error texts
  (`invalid query`, `invalid settings`, `envelope is required` and others;
  grep outside `roym_directory/src` is empty). `session.whoami` is called
  with `{}` only.
- **Pass-through and error `data`.** A remote error stays unchanged in the
  plan, so `data.admission` and `data.membership` still reach the Hub
  ([membership.ts:129](../../../../crates/roym_web/ui/src/directory/membership.ts#L129),
  [roym_trust_e2e.rs:167](../../../../crates/substrate/tests/roym_trust_e2e.rs#L167)).
- **Reuse.** `service_methods!`, `typed`, `parse_value`, `ParamError`,
  `assert_service_table`, `ListingKeyInput`, `ImportInput`,
  `export_signed_bundle`, `PublicationLimitsInput` (moved) and the signing
  verbs are reused. The new parts (`WireNoParams`, `admit::caller`, the scan
  allowance, `remote_reply`) each have a reason. I found no new machinery
  without one, except that `remote_reply` should reuse the reading part of
  `call_sibling` (P1).
- **Deletions.** `handle_certificate_verb` is used only at
  [app.rs:189](../../../../crates/roym_directory/src/app.rs#L189).
  `export_signed` is used only at
  [directory backup.rs:53](../../../../crates/roym_directory/src/app/backup.rs#L53)
  and in one unit test
  ([backup.rs:539](../../../../crates/roym_core/src/backup.rs#L539)). The
  router test only names the function in a comment
  ([router.rs:232](../../../../crates/roym_core/src/router.rs#L232)).
- **Oversized parity file.** `dual_build_parity/directory.rs` is 1135 lines
  and is listed at 1135
  ([oversized-test-files.txt:21](../../../../xtask/oversized-test-files.txt#L21)).
  Putting the `hNewer` scenario in a new module is right. `contract.rs`
  (367) and `trust_harness.rs` (299) have room.
- **Probe for the wire rule.** `check_probe_unknown_fields` sends
  `{"__probe__": 1}`. A wire row with a required field (`PublishInput`,
  `StandingInput`) gives "missing field", not "unknown field", so it passes.
  If someone adds `deny_unknown_fields` to it, serde reports the unknown key
  first and the check fails. So the second manual proof of Step 8 does work.
- **Routing and `web`.** `router::route("session.whoami")` is `None` on
  purpose ([router.rs:20](../../../../crates/roym_core/src/router.rs#L20)).
  `web` answers `session.whoami` before it checks the session
  ([app.rs:130](../../../../crates/roym_web/src/app.rs#L130)), so typing the
  input there adds no new exposure.
- **Docs anchors.** The text the plan quotes exists at
  `developer-guide.md:1687`, `:1703`, `system-architecture.md:1143`,
  `traceability-matrix.md:91-93` and `deferred-backlog.md:501, :509, :516`.
- **"How this is shown to an end user".** It says plainly that an end user
  sees nothing and gives three developer bullets. This follows AGENTS.md.

### Owner questions — reviewer view

I do not answer for the owner. The owner has chosen (a) for all seven, so
this is my view of those choices.

- **Q1 (`admit::caller`, option a). Agree.** It is one small function and I
  checked that both builds fix the caller. (b) touches about 140 handlers
  and (c) is forbidden by OF-15. One addition: add a test that proves the
  new read (P2). The unit test `caller_maps_the_three_origins` is not
  enough, because both builds share the code under test.
- **Q2 (`SynOrgSettingsInput`, option a). Agree.** Option (b) would make
  `directory.info` fail on a stored row with an extra key. Make
  `into_settings` destructure `self` completely (`let Self { name, .. } =
  self;`). A field added only to the input then does not compile, as well
  as the other direction. The plan's JSON comparison test only catches it
  if the test fills the new field.
- **Q3 (typed pass-through, option a). Agree, with a condition.** The
  condition is P1: `remote_reply` must read the reply as `Value`, as
  `call_sibling` does. Without it, option (a) cannot return `None` for a
  directory that runs no SynOrg. Also say in section 3.6 that
  `probe-info` and `publish-to-source` become strict readers (P3).
- **Q4 (`web` keeps its forwarder, option a). Agree.** The router names only
  siblings, and `session.whoami` needs the HTTP caller. (b) would break two
  router tests and forward `session.*` to `web` itself. Note for the owner:
  this means OF-15 ("every Roym method") is true only for methods `web`
  owns. The plan says this in section 9. Keep that wording.
- **Q5 (enums for `open_to` and `booking_mode`, option a). Agree.** It is
  the rule of D-A2a-10, the code is the same `-32602`, and no caller sends
  these words. `OpenTo` and `BookingMode` are `Copy` with kebab-case, so
  `SearchQuery` keeps `Default` and `PartialEq`.
- **Q6 (parity list from the table, option a). Agree.** Keep the four-name
  list by hand: it is the security claim. See P11 for two small text
  errors around scenario 118.
- **Q7 (split the backlog row, option a). Agree.** The title then says what
  is still open. Add the rows named in P8.

### Findings

- **P1 — major.** *Where:* D-A2c-9 (`remote_reply`, Q3); section 4 B51;
  section 7.3 ("A host never produces one"); Step 2 (shape "probe-info for
  one with no SynOrg"); Step 5.
  [client_sources.rs:60-72](../../../../crates/roym_directory/src/app/client_sources.rs#L60),
  [envelope.rs:26-31](../../../../crates/roym_core/src/envelope.rs#L26),
  [method.rs:185-201](../../../../crates/roym_core/src/method.rs#L185),
  [synorg.rs:64-68](../../../../crates/roym_directory/src/app/synorg.rs#L64).
  *Problem:* `remote_reply` parses the remote text as `Response`.
  `Response.result` is `Option<Value>`. serde reads a JSON `null` into
  `None`, so `{"result": null}` and `{}` look the same after the parse. A
  directory that runs no SynOrg answers `directory.info` with
  `Response::ok(Value::Null)`, which is the text `{"result":null}`. That
  is a normal, expected case: the Hub and `roymctl` ask for it on purpose
  (the comment at `synorg.rs:64-68` says why). In the plan, a reply with
  neither `result` nor `error` becomes `-32603`. So `probe-info` can never
  return `Option::None`, and the case that the `Option<DirectoryInfoOutput>`
  was chosen for ends as an error. Section 7.3 says a host never produces
  such a reply. That is wrong for exactly this case. A2a and A2b already hit
  this: `call_sibling` reads the reply as a `Value` and uses `.get("result")`
  for this reason. Two more effects. (1) Step 2 records the shape of
  `probe-info` against a node with no SynOrg from the unchanged code.
  Today the handler returns the parsed `Response` as it is, so that shape is
  an empty answer, not `null`. Step 5 would change it to `-32603` and break
  the rule "the shape files stay green". (2) The same lost `null` probably
  makes the existing branch in `add_source`
  ([client_sources.rs:118-121](../../../../crates/roym_directory/src/app/client_sources.rs#L118),
  "this address answered, but runs no directory") dead code. No test reads
  that note (I searched `crates/` for the text and for `["probe"]`; only the
  issuer-mismatch note is tested), and the Hub shows it
  ([directory.ts:186-190](../../../../crates/roym_web/ui/src/screens/directory.ts#L186)).
  *Failure case:* node X runs `roymctl roym directory info <did>` against
  node Z that has no SynOrg. Before: an empty answer. After the plan:
  `-32603 unreadable answer`. The plan's tests do not catch it, and the
  shape file recorded in Step 2 contradicts the plan.
  *Fix:* in `roym_core::method`, split the tail of `call_sibling` (lines
  185-201) into `pub fn read_reply<O: DeserializeOwned>(raw: &str) ->
  Result<O, SiblingError>`. `call_sibling` calls it. `remote_reply` calls it
  too and maps `Refused(rpc_error)` to `Err(Response { error:
  Some(rpc_error), result: None })` (so `data` stays), and `Transport` /
  `BadReply` to `-32603`. Then `Option<DirectoryInfoOutput>` reads `null` as
  `None`. Add to Step 2 a case "probe-info against a node with no SynOrg"
  and say in the step what the unchanged code returns. Add to Step 7 or
  Step 8 a case that checks `probe-info` returns `null` for a source with no
  SynOrg. Rewrite B51 and section 7.3. Decide what to do with the dead
  `add_source` branch: either read that reply as a `Value` too (a small,
  separate fix, listed in section 4 as "found while checking"), or record it
  in the deferred backlog (P8). Check the serde fact first with a 5-line
  unit test.
  *Status:* Fixed (c8f181eb). I confirmed the serde fact with a unit test on
  `Response` (not kept): `{"result":null}` reads as `result: None` and
  writes back as `{}`. D-A2c-9 now splits `read_reply` out of
  `call_sibling`, and `remote_reply` uses it. For the dead `add_source`
  branch I chose the fix, not a backlog row: it is a one-line change in a
  function A2c already edits, and the Hub already shows the note. It is
  the new audit row B57. Step 2 does not record the no-SynOrg shapes from
  the buggy code; Step 5 records them on the fixed code, so no Step 2
  shape file changes. Scenario 230 tests it over a real wire round trip
  (`hForeignWire2` before `directory2` has a SynOrg). B51 and 7.3 are
  rewritten.

- **P2 — major.** *Where:* Q1, D-A2c-7, section 4 B53, section 7.2 item 1,
  Step 3 and Step 4 tests.
  [publication_ops.rs:123-131](../../../../crates/roym_directory/src/app/publication_ops.rs#L123),
  [publication_ops.rs:226-246](../../../../crates/roym_directory/src/app/publication_ops.rs#L226),
  [directory.rs:724-742](../../../../crates/roym_web/tests/dual_build_parity/directory.rs#L724).
  *Problem:* Q1 moves the one security read of this slice into a new code
  path. `published_by` is the party the publication is recorded under, and
  it is the key of the rate limit. The only test the plan adds is
  `caller_maps_the_three_origins`, a unit test of the mapping function. The
  parity harness compares WASM with native, and both share `admit::caller`,
  so a shared mistake passes. I searched for a test that asserts the
  `published_by` of a **wire** publish. The only direct assertion is
  scenario 113, and that one is a local publish (`published_by` equals the
  owner). Wire attribution is covered only indirectly, through the rate
  limit and the bundle row ids. If `publish` read a wrong or constant
  caller, a stranger's publication could be recorded under the owner's DID
  and use the owner's budget. Existing tests might not notice.
  *Failure case:* `publish` calls `admit::caller` and the mapping returns
  `Internal` for a wire caller (a typo in a match arm). A stranger publishes
  with `published_by = owner`. No test fails.
  *Fix:* add one test, in Step 4 or in `contract_directory.rs` (Step 8): a
  wire `directory.publish` as a verified caller whose DID is not the owner
  (`foreign_searcher_caller` exists), then a local `directory.publications`
  and assert `published_by` equals that DID, in both builds. Add a second
  assertion: the same caller twice past the limit gets `rate-limited`
  keyed on that DID, while the owner's local budget is untouched. Add the
  test to section 7.2 item 1 and remove B53's "No visible change" as the
  only statement: say which test proves it.
  *Status:* Fixed (c8f181eb). Step 8 adds
  `a_wire_publish_is_recorded_under_the_verified_caller` in
  `contract_directory.rs` with both assertions (attribution, and the
  limit keyed on the caller, not the owner). It is in B53, 7.2 item 5 and
  exit criterion 7.

- **P3 — minor.** *Where:* section 3.6 C9; B51; D-A2c-13.
  *Problem:* C9 says a remote node that answers with only some keys is "read
  as today", and "A2c does not change how the client half reads
  `directory.search` answers". That is true for search, `add-source` and
  `check-standing`. It is false for the two pass-through methods. Under Q3
  (a) they now read the whole result into `DirectoryInfoOutput` /
  `PublishOutput`. D-A2c-13 gives `#[serde(default)]` only to `issuer_did`
  and `signing_did`, so a remote `directory.info` without `member_count`,
  `rules` or `area` becomes `-32603` where today it is passed on. All
  directories built from this repository send every key, so this is a cost
  only for a third-party directory. The plan should say so, because OF-18 is
  about "newer node calls older node".
  *Failure case:* a third-party directory answers `directory.info` with
  `name` and `issuer_did` only. `roymctl roym directory info` worked before
  and now answers `-32603`.
  *Fix:* in C9 and B51, say that `probe-info` and `publish-to-source` read
  strictly. Either add `#[serde(default)]` to every `DirectoryInfoOutput`
  field that has a natural empty value (`rules`, `area`, `categories`,
  `support_contact`, `dispute_path`), or state the strict read as a choice
  for the owner. I suggest the second; it is one line in the plan.
  *Status:* Fixed (c8f181eb), with a different fix than suggested. C9
  and B51 now say that `probe-info` and `publish-to-source` read
  strictly. I did not make it an owner question. Instead, D-A2c-13 gives
  `#[serde(default)]` to `rules`, `area`, `categories`, `support_contact`
  and `dispute_path`, where an empty value means "not given". `name`,
  `retention_secs`, `member_count` and `PublishOutput.listing_id` stay
  required, because a default there (`""` or `0`) would be a false claim
  about the remote directory. Reason: the lenient read costs nothing, and
  the owner already chose (a) for Q3. A new unit test reads an info answer
  that has only the required keys.

- **P4 — minor.** *Where:* section 4, "Wire changes, in one list".
  *Problem:* three wire changes are not listed.
  (1) **Absent or `null` `params`.** `Request.params` defaults to `Null`.
  Today `serde_json::from_value(Null)` into `SearchQuery` fails
  ([search_ops.rs:165](../../../../crates/roym_directory/src/app/search_ops.rs#L165)),
  so a wire `directory.search` with no `params` is refused. After the plan,
  `parse_value` reads `Null` as `{}` (D-A2a-7) and the node answers with the
  newest listings. B34 recorded this rule for A2b, but B50 only covers
  `directory.info` with a non-object. A wire change on an anonymous method
  should be in the list.
  (2) **Texts that B42 does not name.** `issuer_did must be a did:key` (for a
  non-string value) and `until_secs must be a non-negative integer` become
  `invalid params: <path>: <text>`. `member_did must be a did:key` stays for
  strings. The plan says "only the text changes", which is right, but the
  list in B42 is meant to be complete.
  (3) **Order of two errors.** A request with two mistakes now reports the
  first one in the JSON key order, not the handler's order. For example
  `member.suspend` with an empty `member_did` and a bad `until_secs` used to
  say `member_did is required` and now says the `until_secs` error. No caller
  sends two errors, so this is only a note.
  *Fix:* add (1) as a row or extend B50; name the two texts in B42; add (3)
  as a sentence.
  *Status:* Fixed (c8f181eb). (1) is the new row B55. (2) B42 now names
  `issuer_did must be a did:key` for a value that is not a string, and
  says which `did:key` texts stay in the handler (`until_secs ..` was
  already listed). (3) is the new row B56. The wire-change list has
  items 5 and 6 for B55 and B57.

- **P5 — minor.** *Where:* Step 3, section 7.4.
  [method/tests.rs](../../../../crates/roym_core/src/method/tests.rs) is 725
  lines and is not on the oversized list.
  *Problem:* Step 3 adds six unit tests there (`wire_no_params_…`,
  `search_query_…`, `publish_output_…`, `search_output_…`,
  `standing_output_…`, `check_allows_req_method_in_the_admit_call`). Six
  tests of 12 to 20 lines each take it past 800, and
  `cargo xtask check-file-lengths` fails. Section 7.4 does not list this
  file.
  *Fix:* put the new tests in `method/tests/directory.rs`, declared from
  `method/tests.rs` (`mod directory;`, layout rule: `foo/tests.rs` +
  `foo/tests/`). Add the file to section 7.4.
  *Status:* Fixed (c8f181eb). The unit tests go in a new
  `method/tests/directory.rs`, declared with `mod directory;`. 7.4 names
  the file.

- **P6 — minor.** *Where:* Step 7; section 3.5; status.md row; exit
  criterion 6.
  *Problem:* two counts are wrong.
  (1) The scenario number `226` is not a placeholder that happens to be
  free. Scenarios 226, 227 and 228 exist (`grep -rho 'fn scenario_[0-9]*'`
  gives 228 as the highest). The next free number is 229.
  (2) The plan and `status.md` say "24 scenario modules plus `deal_fixtures`".
  `dual_build_parity.rs` declares 24 modules in all: 19 with scenarios and 5
  without (`deal_fixtures`, `fixtures`, `helpers`, `trust_fixtures`,
  `trust_harness`). After Step 7 it declares 25. The 222 scenario count is
  right.
  *Fix:* use 229 in Step 7 and in the user-facing bullet command
  (`… newer_node` still matches the name). Say "25 modules (20 with
  scenarios)" after Step 7, and fix the wording in status.md when A2c is
  recorded.
  *Status:* Fixed (c8f181eb). Step 7 uses 229 (and 230 for the P1
  scenario), with the grep to check again. The plan says "24 modules (19
  with scenarios)" before Step 7 and "25 modules (20 with scenarios)"
  after; the parity count is now 224. I did not change the A2b row of
  `status.md`, because it records what A2b's tool wrote at the time. The
  A2c row will use the correct wording.

- **P7 — minor.** *Where:* D-A2c-16, Step 3.
  [check.rs:185-195](../../../../crates/roym_core/src/method/check.rs#L185).
  *Problem:* the new allowance is "a `req.method` line in `app.rs` that
  holds `admit::admit(host,`". Any line with that text passes, whatever else
  it does with `req.method`. For example `admit::admit(host, X, &req.method)
  ; match req.method.as_str() {` is allowed. The existing allowance is a
  fixed full call.
  *Fix:* match the whole call, in the same way as the existing line:
  `line.contains("admit::admit(host, api::WIRE_REACHABLE, &req.method)")`.
  The unit test should also show that another `req.method` use on a line
  that holds `admit::admit` text is not enough (for example a line with both
  the call and `req.method.as_str()`).
  *Status:* Fixed (c8f181eb). D-A2c-16 matches the whole call
  `admit::admit(host, api::WIRE_REACHABLE, &req.method)` and requires
  exactly one `req.method` on the line. The unit test (renamed
  `check_allows_req_method_only_in_the_admit_call`) shows that a line
  with the call and `req.method.as_str()` fails, and that the call with
  another list fails.

- **P8 — minor.** *Where:* section 8 (deferred backlog), D-A2c-10.
  *Problem:* the plan leaves the three client reads of remote answers
  (`directory.search` in `query_source`, `directory.info` in `add_source`,
  `directory.standing` in `handle_standing_reply`) as hand-written `Value`
  reads, and the outbound calls as hand-built `host.call`s. This is a
  postponement (AGENTS.md: record anything deferred). Section 8 has no row
  for it; section 11 only says "A3 and A4 change the search path". Also the
  dead `add_source` branch (P1), if the owner does not want it fixed here,
  needs a row.
  *Fix:* add to section 8 one row (or a sentence on the new narrower row of
  Q7): "The directory's client half reads remote `search`, `info` and
  `standing` answers by hand. Target: A4 (federated query library)". Add a
  row for the "runs no directory" note if it is not fixed in A2c.
  *Status:* Fixed (c8f181eb). Section 8 has a new row "The directory's
  client half reads remote answers by hand", target A4. The `add_source`
  branch is fixed in A2c (P1), so it needs no row; section 8 lists it
  under "resolved in code".

- **P9 — minor.** *Where:* Step 8, error tests.
  *Problem:* the end-to-end wire tests with an unknown key cover
  `directory.search` and `directory.standing` only. `directory.publish` and
  `directory.info` rely on the generic loop ("if it is an error, its
  `data.field` is not `zz`"). For `publish` with `{"zz": 1}` the answer is
  `missing field envelope`, which does show the key was not refused, but it
  is a weak proof and it does not show that a correct publish with an extra
  key works (OF-18's real case: a newer node publishes).
  *Fix:* add two cases: `directory.info` with `{"zz": 1}` gives a result;
  `directory.publish` with a valid signed envelope plus `"zz": 1` gives
  `{listing_id, record_id}`. The `hNewer` scenario already needs a valid
  envelope, so the set-up exists.
  *Status:* Fixed (c8f181eb). Step 8 adds both cases: a wire
  `directory.info` with `{"zz": 1}`, and a wire `directory.publish` with
  a valid signed envelope and `"zz": 1`.

- **P10 — minor.** *Where:* D-A2c-3, D-A2c-4 (`query` field),
  [client_query.rs:201-218](../../../../crates/roym_directory/src/app/client_query.rs#L201).
  *Problem:* a local-only method (`directory.query-source`) keeps ignoring
  unknown keys **inside** `query`, because `SearchQuery` is shared with the
  wire. The plan says "as before", and that is true. But it is the same
  kind of silent mistake that OF-18 and B41 fix. A misspelled filter, for
  example `categorys`, is dropped and the person gets a wider search than
  they asked for. The check does not look at nested types (its own doc
  comment says so), so nothing will ever catch it.
  *Fix (owner's choice):* either keep it and write one sentence in
  D-A2c-3 and in the developer guide ("nested record types ignore unknown
  keys"), or give `QuerySourceInput` a strict copy of the query. I suggest
  the sentence only: a second struct that must match `SearchQuery` is the
  kind of duplicate that Q2 had to guard with a test. Do not leave it
  unsaid.
  *Status:* Fixed (c8f181eb). I agree with the sentence only. D-A2c-3
  says it, with the `categorys` example and the reason. Section 9 adds it
  to the developer guide's "Bad input".

- **P11 — minor.** *Where:* Q6/D-A2c-17, Q2/Step 4, Step 10, section 1
  (user section).
  *Problem:* small text errors.
  (1) Scenario 118's assertion message names the deleted constant
  ([directory.rs:648](../../../../crates/roym_web/tests/dual_build_parity/directory.rs#L648):
  "is in ALL_DIRECTORY_VERBS but the dispatch does not know it"). The plan
  says only the loop line and the `contains` line change. Change the text
  in place too (no new line).
  (2) Step 4 says "the module layout is `synorg.rs` + `synorg/tests.rs`".
  There is no `synorg/` directory today. The step creates it.
  (3) Step 10 says to record a lower value if `trust_fixtures.rs` shrank.
  It is 238 lines and is not on `oversized-test-files.txt`, so there is
  nothing to record. `directory.rs` does not shrink either (net zero).
  (4) The first bullet of "How this is shown to an end user" says the
  test "lists every method of all six Roym services". The test checks the
  tables; for `web` it checks one row. Say "checks the table of every
  service".
  *Fix:* correct the four texts.
  *Status:* Fixed (c8f181eb). (1) D-A2c-17 changes the message text in
  place, so the scenario changes three lines, not two. (2) Step 4 says
  the `synorg/` directory is new. (3) Step 10 says `directory.rs` stays
  at 1135 and `trust_fixtures.rs` records nothing. (4) The user section
  says "checks the method table of every Roym service (for `web`, its one
  row)".

- **P12 — minor.** *Where:* section 9 (living docs).
  *Problem:* [system-requirements-spec.md:710](../../../system-requirements-spec.md)
  lists the three `WireRule::Open` methods and nothing else. The plan's
  list does not name this file, and says to grep the integrated spec "only
  if" it states how bad input is answered. The requirements file is also in
  the change doc's `living-docs-touched`. Also `system-architecture.md:1141`
  (the four wire methods) sits next to the paragraph at `:1143` that the
  plan edits; the "one sentence on the four wire-reachable methods ignoring
  unknown fields" belongs at `:1141`.
  *Fix:* add the requirements line to section 9 as "check, update if
  false", and name `:1141` as the place for the new sentence.
  *Status:* Fixed (c8f181eb). Section 9 adds the requirements spec as
  "check, update if false", and puts the new architecture sentence at
  line 1141.

### Notes on the owner-question views

- **Q2.** Accepted: D-A2c-8 now makes `into_settings` destructure `self`
  with every field named, so a field added to only one struct does not
  compile. Fixed (c8f181eb).
- **Q3.** The condition (P1) is met: `remote_reply` reads through
  `read_reply`.

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

## Plan review round 1 — fixes checked — Claude Sonnet 5.5 (Claude Code desktop), 2026-10-31

Checked the plan at `a3d73a2d` against the findings and the code. I could
not run code, so I did not repeat the serde check of P1 (the plan author
says a unit test on `Response` shows `{"result":null}` reads as `None` and
writes back as `{}`; this agrees with my reading of serde).

**Summary:** P1 to P12 are fixed in the plan, as the statuses say. Two new
minor findings, P13 and P14, come from the new text. 0 blocker, 0 major,
2 minor open.

What was checked and found correct:

- **P1.** `read_reply` is split out of `call_sibling` and keeps its
  errors. `remote_reply` maps `Refused` to the remote error with its
  `data`, and `BadReply` to `-32603`. `probe_info_verb` returns
  `Option<DirectoryInfoOutput>`, so `{"result":null}` is `Ok(None)`.
  `add_source` reads the reply as `Value`, which fixes the dead note
  (B57). The Step 2 shapes no longer record the buggy no-SynOrg answers,
  and Step 5 records them on the fixed code. B51, 7.3 and the wire-change
  list match. Scenario 230 can use `did:key:hForeignWire2`: it routes to
  `directory2` ([helpers.rs:292-296](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L292)),
  and `harness()` builds a new harness per test, so `directory2` has no
  SynOrg unless the test makes one.
- **P2.** The test `a_wire_publish_is_recorded_under_the_verified_caller`
  is in Step 8, B53, 7.2 and the exit criteria. Scenario 83 is a model for
  the limit part. (See P13 for the helper.)
- **P3.** The fix adds `#[serde(default)]` to `rules`, `area`,
  `categories`, `support_contact` and `dispute_path`, and keeps `name`,
  `retention_secs`, `member_count` and `listing_id` required. This is the
  first option of my finding. It is a good choice and the reason given
  (a default would be a false claim) is right. The status row says "a
  different fix than suggested"; it is one of the two options I listed.
- **P4 to P12.** B55, B56, B42 and the wire-change list; the new
  `method/tests/directory.rs`; scenario numbers 229 and 230 (the highest
  existing is 228) and the module wording (24 modules, 19 with scenarios;
  25 and 20 after Step 7; 224 scenarios); the whole-call allowance in
  D-A2c-16 with its three-case test; the new backlog row; the two extra
  wire cases; the nested-key sentence in D-A2c-3 and the developer guide;
  the four text fixes; and the requirements-spec and `:1141` lines in
  section 9. No stale "223", "226" or "two-line" text is left in the
  plan.
- **Owner-question notes.** Q2 (destructure `self`) and Q3 (condition met)
  are as the review notes say.

### New findings

- **P13 — minor.** *Where:* Step 8, test
  `a_wire_publish_is_recorded_under_the_verified_caller`.
  [fixtures.rs:487-510](../../../../crates/roym_web/tests/dual_build_parity/fixtures.rs#L487).
  *Problem:* the plan says to call `wire_invoke_as` "with
  `foreign_searcher_caller()`". `wire_invoke_as` takes an `AuthLevel`, not
  a caller. It builds its own caller with an empty `caller_did`. There is
  no wire helper that takes a `CallerContext` (`wire_invoke` always uses
  the owner). So the call as written does not compile, and with
  `AuthLevel::Delegated` it would make `Verified("")`, which proves
  nothing about a real DID. The limit part also needs a small limit
  first: the default is 20 per day
  ([safety.rs:61-65](../../../../crates/roym_core/src/safety.rs#L61)), so
  "publishes until the limit refuses it" would need 21 signed listings
  unless the test sets `max_per_window: 1` as scenario 83 does.
  *Failure case:* the implementer writes the test as planned, finds it
  cannot set a DID, and either adds a helper without a plan or tests with
  an empty DID, which would pass even if `published_by` were wrong.
  *Fix:* in Step 8, add a helper `wire_invoke_with(h, svc, env, caller:
  CallerContext)` beside `wire_invoke_as` (and let `wire_invoke_as` call
  it, to avoid a second copy of the body). Say that the test first sets
  `max_per_window: 1` with `directory.set-settings`, as scenario 83 does.
  Name the new helper in the files list of Step 8.
  *Status:* Fixed (33f7e7d0). Step 8 adds `fixtures::wire_invoke_with(h,
  svc, envelope, caller: CallerContext)`. It takes over the body of
  `wire_invoke_as`, which now calls it, so the body is not copied. The
  test first sets `max_per_window: 1` with `directory.set-settings`, as
  scenario 83 does, then checks attribution, the refusal and the owner's
  publish in four numbered steps. 7.4 notes that `fixtures.rs` (778 lines)
  grows by about 10 to 15 lines and says where the helpers go if it would
  pass 800. I also moved the test out of the error-test list, where it
  had been placed above the `session.whoami` case by mistake.

- **P14 — minor.** *Where:* Step 5, "Record two new shape files …
  `UPDATE_METHOD_SHAPES=1` for these two cases only".
  [shapes.rs:87-97](../../../../crates/roym_web/tests/method_contract/shapes.rs#L87).
  *Problem:* `UPDATE_METHOD_SHAPES=1` rewrites every shape case that the
  running test function reaches. It has no per-case switch. The plan
  asks for "these two cases only", but the cases would sit inside
  `client_shapes`, which also holds the Step 2 cases. The run would
  rewrite those files too. They should come out byte-for-byte the same,
  but the rule "every shape file of Step 2 stays unchanged" is then
  checked only by `git diff`, and a real change would be hidden by the
  rewrite.
  *Fix:* put the two B57 cases in their own test function (for example
  `no_synorg_shapes`), and record with a filter on that name. Then the
  Step 2 files are never written. Keep the `git diff --stat` check on
  `method_shapes/` as the proof.
  *Status:* Fixed (33f7e7d0). The two B57 cases are in their own test
  function `no_synorg_shapes`, recorded with a filter on that name. The
  proof is `git diff --stat` on `method_shapes/` showing only the two new
  files. Step 2's list of test functions says that Step 5 adds it.

## Plan review round 1 — P13 and P14 fixes confirmed — Claude Sonnet 5.5 (Claude Code desktop), 2026-10-31

Checked the plan at `d4e0013d`. No code was run.

- **P13 confirmed.** Step 8 names `fixtures::wire_invoke_with(h, svc,
  envelope, caller: CallerContext)`, built from the body of
  `wire_invoke_as`, which then calls it. `foreign_searcher_caller()` exists
  ([helpers.rs:268](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L268)).
  The test sets `max_per_window: 1` first, then checks attribution, the
  refusal and the owner's publish. `fixtures.rs` is 778 lines, as 7.4
  says, and it is not on the oversized list; the fallback module is named.
  One note for the implementer, not a finding: the refused second publish
  must be a second, different listing, as in scenario 83. A repeat of the
  same envelope could be refused for another reason.
- **P14 confirmed.** The two B57 cases are in their own function
  `no_synorg_shapes`, recorded with a filter on that name. The proof is
  `git diff --stat` on `method_shapes/`. Step 2's list says Step 5 adds the
  function.

All 14 findings are closed. 0 open. `status.md` matches.

## Code review round 1 — Claude Opus 5.5 (Claude Code desktop), 2026-10-31

Checked `git diff origin/main...HEAD` at `c61a29b7` on
`feat/m08a-a2c-typed-methods` (PR
[#337](https://github.com/syneroym/syneroym/pull/337), still a draft),
against the plan, the plan review, the change doc and `AGENTS.md`. The
implementer was Gemini 3.8 Flash. CI passed on `c61a29b7`. No code was
changed. The full suite was not run. Only these were run:
`cargo nextest list -p syneroym-roym-web --test method_contract` (31
tests), `cargo nextest list -p syneroym-roym-core -p
syneroym-roym-directory -p syneroym-roym-catalog` (242 tests), and the
`cargo dupes` recipe of `AGENTS.md` over the changed files (nothing new:
the only hits in changed files are older groups, such as `ping` and
`status` in every service).

**Summary:** 0 blocker, 1 major (R1), 11 minor (R2 to R12).

### What was checked and found correct

- **Security: exactly four methods answer over the wire (point 1).**
  `invoke` is `admit::admit(host, api::WIRE_REACHABLE, &req.method)`, then
  `api::dispatch`, then `method_not_found`
  ([app.rs:200-210](../../../../crates/roym_directory/src/app.rs#L200)).
  Admission runs before any parse and before the table lookup, so a name
  that is not in the table also gets `-32013` off the node.
  `WIRE_REACHABLE` is one constant
  ([api.rs:35-40](../../../../crates/roym_directory/src/api.rs#L35)), with
  its comment moved unchanged. The contract test builds the check's wire
  list from it and asserts that its sorted names equal the hand list
  `trust_fixtures::WIRE_REACHABLE_DIRECTORY_VERBS`
  ([contract.rs:69-85](../../../../crates/roym_web/tests/method_contract/contract.rs#L69)).
  The hand list stays written by hand, so a fifth name added to the
  production list fails a test. `every_method_answers_off_the_node_as_its_wire_rule_says`
  sends `{"zz": 1}` over the real wire path to all 37 rows in both builds:
  33 answer `-32013`, the four do not, and none of their errors names
  `zz`. The first loop of the same test still covers the four other
  services. Scenario 118 now loops over the table and keeps the hand list
  for the wire posture.
- **`admit::caller` (point 1).** `admit` calls `caller` and matches on its
  answer ([admit.rs:53-93](../../../../crates/roym_core/src/admit.rs#L53)).
  Both hosts answer it from fields that are fixed for the whole call:
  `HostState.invocation_origin` and `.caller` in the WASM host
  ([capabilities_services.rs:85-100](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs#L85)),
  and the same two fields of the per-call `NativeAppHost`
  ([host.rs:685-698](../../../../crates/app_host_native/src/host.rs#L685)).
  `publish` reads it first, before any outbound call
  ([publication_ops.rs:139-144](../../../../crates/roym_directory/src/app/publication_ops.rs#L139)),
  and then calls `resolve_published_by` and `check_rate_limit` with that
  caller, as on `main`. `a_wire_publish_is_recorded_under_the_verified_caller`
  proves in both builds that a wire publish is stored with
  `published_by = did:key:zForeignSearcher`, that the second one is
  `rate-limited`, and that the owner can still publish after it.
- **The one allowed `admit::admit` line (point 1).** The allowance needs
  the exact text `admit::admit(host, api::WIRE_REACHABLE, &req.method)`,
  a file named `app.rs`, and exactly one `req.method` on the line
  ([check.rs:185-197](../../../../crates/roym_core/src/method/check.rs#L185)).
  The hand-written arm check (`is_match_arm`) and the string-literal check
  run on every line on their own, so the allowance cannot hide an arm. The
  PR text says both manual proofs of Step 8 were done (a `"directory.info"
  =>` arm and `deny_unknown_fields` on `StandingInput` each failed the
  table test). One older limit, not caused by A2c and not a finding: the
  scan looks only for the text `req.method`, so a dispatch on a value
  bound under another name (`let Request { method, .. } = &req;`) would
  not be seen.
- **Wire inputs and outputs (point 2).** `SearchQuery`, `WireNoParams`,
  `PublishInput` and `StandingInput` have no `deny_unknown_fields`. The six
  wire types in `roym_core::method::directory` derive `Serialize` and
  `Deserialize` and never refuse unknown fields. The `#[serde(default)]`
  fields are those of D-A2c-13; `name`, `retention_secs`, `member_count`
  and `PublishOutput.listing_id` stay required. `PublishOutput` skips the
  missing one of `record_id` / `withdrawn`, so both shapes are as on
  `main` (shape files `directory_publish_published.json` and
  `directory_publish_withdrawn.json`).
- **The two newer-node scenarios (point 2).** Scenario 229
  ([directory_compat.rs:8-62](../../../../crates/roym_web/tests/dual_build_parity/directory_compat.rs#L8))
  reads a `did:key:hNewer` source that adds `ranking` to the search
  answer, a key inside the hit, `extra` to `info` and `standing`, and
  `receipt` to the publish answer: `query-source` stores the hit
  (`verified: 1`, `error: null`), `check-standing` is `refreshed: true`,
  `probe-info` has no `extra`, `publish-to-source` has no `receipt`. This
  is C7 of plan section 3.6 and the four reads of Step 7. The search answer
  also has no `truncated`, so it covers part of C9. Scenario 230 sends
  `hForeignWire2` over a real wire round trip to `directory2` with no
  SynOrg: `add-source` returns the "runs no directory" note, and
  `probe-info` answers `null` (B57). 229 and 230 are the next free numbers;
  `grep -c "async fn scenario_"` gives 224; 25 modules, 20 with scenarios.
- **Remote replies (point 3).** `read_reply` is split out of
  `call_sibling` with no change to its texts
  ([method.rs:199-222](../../../../crates/roym_core/src/method.rs#L199)).
  `remote_reply` maps `Refused(e)` to `Err(Response { result: None, error:
  Some(e) })`, so code, message and `data` pass through; `BadReply` is
  `-32603 "unreadable answer from the directory: .."`
  ([client_sources.rs:65-77](../../../../crates/roym_directory/src/app/client_sources.rs#L65)).
  `{"result": null}` is `Ok(None)` and is written back as `{"result":
  null}` (unit test `read_reply_reads_a_null_result_as_none_and_keeps_error_data`,
  scenario 230, `directory_probe_info_no_synorg.json`). The transport
  texts are as on `main`. The `add_source` "runs no directory" branch now
  runs (scenario 230 and `directory_add_source_no_synorg.json`, which has
  `probe` and `last_ok_secs`). See R2 for one side effect of the new read.
- **Handler bodies (point 4).** Every converted handler was compared with
  `main`. Apart from R2, R4 and R11, the changes are the type changes the
  plan asks for: the parse moves out, `return resp` becomes `return
  Err(resp)` or `?`, `json!` becomes a struct literal with the same keys.
  The order of the checks after the parse is unchanged in `publish`,
  `credential.issue` (`note` is still read after the record is stored),
  `revoke`, `suspend`, `lift`, `check_standing`, `query_source`,
  `add_source` and `publish_to_source`. The empty-string checks of
  D-A2c-5 keep their words, and fields with no empty check on `main`
  (`member.remove`, `remove-source`) get none. `issuer-changed` writes no
  `as_of_secs` and no `error`. The "why" comments stay, except R4. The
  `open_to` comment in `build_search_filter` lost its second half, but
  that half described the hand check that the plan deletes, so this is
  correct.
- **Tables (point 5).** `directory` has 37 rows with the inputs and
  outputs of the section 5 table
  ([api.rs:46-84](../../../../crates/roym_directory/src/api.rs#L46)).
  `web` has its one row ([web api.rs:7-9](../../../../crates/roym_web/src/api.rs#L7));
  `web_has_one_typed_method_of_its_own` checks it directly. The forwarder
  keeps `params: Value`, and `forward_rpc_call`, `admit`, `handle_http`
  and `/health` are unchanged. The greps of Step 4 and Step 5
  (`handle_certificate_verb`, `export_signed(`, `params.get`,
  `req.params`, `Response::ok` in `roym_directory/src`) print nothing.
  `dual_build_parity/directory.rs` is still 1135 lines.
- **Shapes (points 5 and 8).** 58 shape files were added in the Step 2
  commit `21356bf3`, before any code change, and 2 (B57) in the Step 5
  commit `f564bac4`. No later commit changes a shape file, and none of the
  157 files of A2a and A2b changed. The cases are the ones Step 2 lists.
- **Unit tests of Step 3 and Step 4** exist with the planned names,
  `caller_maps_the_three_origins` and `settings_input_matches_settings_fields`
  included. `into_settings` destructures every field with no `..`.
- **Q4, Q6, Q7 and the living docs (point 7).** The developer guide, the
  architecture doc and the three traceability rows no longer say that
  `directory` and `web` are untyped, and they say what `web` does not
  type. No over-claim was found. The requirements spec was left alone,
  which plan section 9 allows (it says nothing about bad input). The
  backlog row is split as Q7 (a) says (see R10 for the format), and the
  three new and two extended rows of plan section 8 are there.
- **Counts (point 8).** `method_contract` has 31 tests and
  core/directory/catalog 242, as `status.md` says. The shape counts are
  wrong (R3).
- **Sizes, deps, layout (point 9).** No new dependency. Every changed
  production file is under 800 lines (largest `publication_ops.rs`, 578).
  `synorg.rs` + `synorg/tests.rs` follow the module rule. CI passed
  clippy, `check-file-lengths` and `check-duplication`. No planning ids and
  no line counts in code comments. No Hub or `roymctl` file changed.

### Findings

- [x] **R1 — major.** *Where:*
  [contract_directory.rs:16-98](../../../../crates/roym_web/tests/method_contract/contract_directory.rs#L16)
  (`directory_parameter_errors`). *Problem:* all seven `/rpc` error cases
  read only the WASM answer: `let (w, _) = post_rpc(..)`. The native
  answer is thrown away. Plan Step 8 says "each through `post_rpc` (both
  builds, `wasm == native`)", and exit criterion 4 says "equal in both
  builds". So the wrong-type cases (`expires_at_secs`, `until_secs`), the
  nested refusal (`query.open_to`), the nested ignore inside
  `publication_limits` and the unknown key of `member.add` are proven for
  one build only. In addition, the `query-source` case sends
  `source: "did:key:hForeignWire"`, which was never added as a source,
  and it does not check that "the source was not called" (B46, wire
  change 3). The case passes for the right reason today (the parse runs
  first), but nothing in it shows that no source call is made. *Failure
  case:* a native-only change to how a nested `ParamError` path is built
  (for example the native shim reads `params` through a different
  `serde_json::Value` route) gives `data.field = "open_to"` instead of
  `"query.open_to"` on native. This test still passes, and so does every
  other test, because the wire tests do not send a nested input. *Fix:*
  in every case use `let (w, n) = post_rpc(..)` and `assert_eq!(w, n)`
  before the field asserts (the wire cases and the whoami case already do
  this). For `query-source`, add a real source first (for example the
  canned `did:key:hTrustValid` with `directory.add-source`), send the bad
  `open_to`, and then show that no call was made: `directory.merge` for
  the run has no hit (that source serves one valid hit when it is
  called). Build `credential.issue` and `member.suspend` from
  a valid request too (a SynOrg and a member exist), as Step 8 says, so
  that the one bad field is the only error. *Status:* Fixed (0ece72ac). In every case of directory_parameter_errors, (w, n) is read via post_rpc and assert_eq!(w, n) verifies both builds produce identical results. Canned source did:key:hTrustValid is added before query-source, and directory.merge confirms 0 hits (no remote source call was made). credential.issue and member.suspend are executed with valid SynOrg and member preconditions so only the tested field fails. Proof verified: temporarily changing expected field "query.open_to" to "open_to" caused the test to fail on assert_eq!(w["error"]["data"]["field"], "open_to") with left: "query.open_to", right: "open_to", confirming the check catches differences.

- [x] **R2 — minor.** *Where:*
  [client_sources.rs:161-181](../../../../crates/roym_directory/src/app/client_sources.rs#L161)
  (`add_source`). *Problem:* the probe reply is now parsed as `Value`, not
  as `Response`. On `main`, a reply that was valid JSON but not a
  JSON-RPC response failed `from_str::<Response>` and was stored as
  `SourceError::Unreadable`. Now it parses as a `Value`, falls into the
  second arm, and the source is stored as healthy: `last_ok_secs` is set
  and `last_error` is empty. This holds for a top-level `null`, an array,
  a string or a number, and for an object whose `error` is not an error
  object (for example `{"error": "boom"}`). The plan (D-A2c-9) asks only
  that `{"result": null}` be seen; it says "a JSON parse error stays
  `SourceError::Unreadable`", which the code keeps literally but not in
  meaning. The change is not in "Planned wire changes" or "Different from
  the plan". *Failure case:* a hostile or broken node answers the probe
  with `null`. On `main` the Hub shows the source as unreadable. Now it
  shows it as reached, with no error. *Fix:* keep the `Value` read for the
  `result` key, and refuse a reply that is not a JSON-RPC response as
  before, for example: parse as `Value`; if it is not an object, or if
  `serde_json::from_value::<Response>(v.clone())` fails, store
  `Unreadable` with that error. Then no change needs to be recorded. If
  the new behaviour is wanted instead, record it under "Different from
  the plan" and as a wire change. *Status:* Fixed (1f6056ff).

- [x] **R3 — minor.** *Where:*
  [status.md:15](status.md) (A2c row) and the PR description ("How
  Tested"). *Problem:* both say "38 method shapes (35 directory + 3
  whoami; 195 total in workspace)". The real counts are 60 new files (57
  directory + 3 whoami: 58 in `21356bf3`, 2 in `f564bac4`) and 217 in
  total (157 + 60). `shapes_directory.rs` has 60 `assert_shape` calls.
  The PR text also lacks most of what Step 10 asks for: the method
  tables, the audit table of section 4, the table of section 3.6, and the
  exact counts; its "Deviations: None" is wrong while R2 is open. This is
  the kind of mistake A2b R2 found. *Failure case:* the next reviewer, or
  the milestone close-out, checks the count against the folder and finds
  23 files that the record does not explain. *Fix:* write "60 method
  shapes (57 directory + 3 whoami; 217 total)" in `status.md` and the PR.
  Add the missing Step 10 sections to the PR text (they can link the plan
  sections instead of copying them). *Status:* Fixed (a1e1b10d).

- [x] **R4 — minor.** *Where:* the dispatch arm of
  `directory.publish-to-source` on `main`
  (`crates/roym_directory/src/app.rs:233-236` there), now
  [api.rs:81](../../../../crates/roym_directory/src/api.rs#L81). *Problem:*
  the "why" comment of that arm was deleted with the `match`: "Not
  `publication_ops::publish` -- this fetches one of this node's own
  catalog listings and calls `directory.publish` on a remote source;
  `publication_ops::publish` is the inbound side that remote call lands
  on." The doc comment of `publish_to_source` says what it does but not
  how it differs from `publication_ops::publish`. Plan section 0 and Step 4
  say to keep every "why" comment (A2b R8). *Failure case:* a reader of
  the table sees two publish rows next to each other and cannot tell
  which side of the wire each one is. *Fix:* put the comment back above
  the `"directory.publish-to-source"` row in `api.rs` (comments are
  allowed inside `service_methods!` in the other services), or add its
  last sentence to the doc comment of `publish_to_source`. *Status:* Fixed (10c5fa74).

- [x] **R5 — minor.** *Where:*
  [method/tests/directory.rs:60, :73](../../../../crates/roym_core/src/method/tests/directory.rs#L60)
  (`std::panic::catch_unwind`),
  [trust_fixtures.rs:134](../../../../crates/roym_web/tests/dual_build_parity/trust_fixtures.rs#L134)
  (`syneroym_roym_directory::api::method_table()`),
  [contract.rs:93](../../../../crates/roym_web/tests/method_contract/contract.rs#L93)
  (`syneroym_roym_core::router::route`). *Problem:* inline paths with more
  than one `::`. The import rule of `AGENTS.md` and Step 10 forbid them
  (A2a R9, A2b R3). *Failure case:* none at run time; the import pass of
  Step 10 was not complete. *Fix:* `use std::panic;` and
  `panic::catch_unwind`; `use syneroym_roym_directory::api as
  directory_api;` and `directory_api::method_table()`; add `router` to the
  existing `syneroym_roym_core::{..}` import in `contract.rs` and call
  `router::route`. *Status:* Fixed (7ee58117, f67e0714).

- [x] **R6 — minor.** *Where:*
  [contract_directory.rs:188, :220, :240, :263](../../../../crates/roym_web/tests/method_contract/contract_directory.rs#L188).
  *Problem:* the comments `// 1. Set limit ..` to `// 4. Local publish ..`
  number the list of Step 8 of the plan. Plan section 0 forbids numbered
  comments that follow a list in the plan (A2a R10, A2b R7). *Failure
  case:* a step is added or removed and the numbers no longer match
  anything. *Fix:* drop the numbers and keep the words. *Status:* Fixed (10c5fa74).

- [x] **R7 — minor.** *Where:*
  [trust_harness.rs:272-343](../../../../crates/roym_web/tests/dual_build_parity/trust_harness.rs#L272)
  (`trust_source_response`). *Problem:* the `hNewer` source is a second
  function pasted into the middle of `trust_source_response`, behind
  `if target == "did:key:hNewer"`. Its `"directory.publish"` arm
  (`:298-317`) is 20 lines, above the 10-line limit for match arms. That
  arm also re-parses the call frame by hand in an inline closure, which
  copies the frame parsing of `inner_method` (`:254-265`), and it falls
  back to `("l1", "r1")` with no sign that the parse failed. The doc
  comment above the function (`:267-271`) does not mention `hNewer`.
  *Failure case:* the frame format changes; `inner_method` is fixed, the
  copy is not, and the fallback answers `listing_id: "l1"`. Scenario 229
  then fails with a confusing listing-id mismatch instead of a parse
  error. *Fix:* move the `hNewer` answers into their own
  `fn newer_source_response(params: &Value) -> Option<Value>` and call it
  from `canned_source_response` (`:347`) like the other two.
  Split `inner_method` into `inner_frame(params) -> Option<Value>` and the
  method read, and use `inner_frame` for the envelope too. Let the publish
  arm call a small `fn published_ids(params) -> Option<(String, String)>`
  and answer an error, not `l1`/`r1`, when it returns `None`. *Status:* Fixed (7ee58117).

- [x] **R8 — minor.** *Where:*
  [method/directory.rs:1-65](../../../../crates/roym_core/src/method/directory.rs#L1).
  *Problem:* this file is the contract between two nodes, and it has no
  module doc and no comment. Nothing in it says that these types must
  never get `deny_unknown_fields`, why some fields have
  `#[serde(default)]`, or why `name`, `retention_secs`, `member_count` and
  `listing_id` must stay required (a default would be a false claim about
  the remote directory, D-A2c-13). The other `method` modules have at
  least a module line (`catalog.rs:1`). The check guards only inputs (the
  probe), so nothing stops `deny_unknown_fields` on an output. No test
  reads `SearchOutput` with an extra key (the client still reads search
  answers by hand, D-A2c-10), and
  `directory_info_output_reads_an_answer_with_only_name_and_counts` sends
  no extra key. *Failure case:* a later change adds
  `#[serde(deny_unknown_fields)]` to `SearchOutput` "to be strict", or
  `#[serde(default)]` to `name`. Every test still passes. A4 then reads
  remote search answers with it, and every newer node becomes
  `unreadable`. *Fix:* add a module doc that states the rule in two or
  three sentences, and a one-line comment on the required fields of
  `DirectoryInfoOutput`. Add `"zz": 1` to the JSON of the
  `search_output_..` and `directory_info_output_..` unit tests, so every
  wire output is read once with an unknown key. *Status:* Fixed (f67e0714).

- [x] **R9 — minor.** *Where:* change doc
  [change.md:281-291](../../changes/roym-open-foundation/change.md) (the
  D-A2c lines) and `:341-342`. *Problem:* the numbers are right, but five
  lines lose part of their meaning, and for two of them the lost part is
  the security reason for the decision:
  - D-A2c-7 says `publish` "passes caller DID to `resolve_published_by`".
    It passes the `Caller`; `resolve_published_by` turns it into a DID.
  - D-A2c-16 does not say what the check scan allows: only the exact call
    `admit::admit(host, api::WIRE_REACHABLE, &req.method)` in `app.rs`,
    with one `req.method` on the line.
  - D-A2c-17 does not say that the hand list stays independent and that
    the contract test asserts it equals `WIRE_REACHABLE`.
  - D-A2c-13 does not say which fields stay required (`name`,
    `retention_secs`, `member_count`, `PublishOutput.listing_id`).
  - D-A2c-9 does not say that a result that does not parse is `-32603`,
    or that `add_source` reads the reply as `Value`.
  Also, B53 and B54 are listed under "Planned wire changes", but the plan
  says neither is visible on the wire. A2a R2 / R3 and A2b R1 were the
  same kind of finding. *Failure case:* someone reads the change doc to
  learn why the hand list is not built from the table, finds no reason,
  and "simplifies" it away; the fifth-method check is then gone. *Fix:*
  add the missing half-sentence to each of the five lines, and mark B53
  and B54 "no visible change". When R2 is decided, record it here if the
  new behaviour stays. *Status:* Fixed (a1e1b10d).

- [x] **R10 — minor.** *Where:*
  [deferred-backlog.md:647](../../deferred-backlog.md) ("Recently
  resolved"). *Problem:* the new row has four cells (item, text, date,
  source) and a blank line after it. Every other row in this section has
  two cells (item, then "Resolved <date> by .."). The row also does not
  start with "Resolved", as the others do. *Failure case:* the section
  shows one row with a different layout, and a reader looking for the
  date of a resolution finds it in a different place. *Fix:* write it as
  `| **Roym's sibling boundary is untyped JSON** | Resolved 2026-10-31 by
  PR #337: all six Roym services .. (`crates/roym_core/src/method.rs`). |`
  and remove the blank line. *Status:* Fixed (a1e1b10d).

- [x] **R11 — minor.** *Where:*
  [synorg.rs:199-204](../../../../crates/roym_directory/src/app/synorg.rs#L199),
  [publication_ops.rs:546-552](../../../../crates/roym_directory/src/app/publication_ops.rs#L546),
  [credential_ops.rs:330-336](../../../../crates/roym_directory/src/app/credential_ops.rs#L330),
  [client_sources.rs:236-242](../../../../crates/roym_directory/src/app/client_sources.rs#L236),
  [held.rs:271-278](../../../../crates/roym_directory/src/app/held.rs#L271).
  *Problem:* each list path reads `Vec<Value>` with `paging::query_all` and
  then runs `filter_map(serde_json::from_value(..).ok())`.
  `paging::query_all::<_, T>` already skips a row whose payload does not
  decode ([paging.rs:81-87](../../../../crates/roym_core/src/paging.rs#L81)),
  so this is the same result in two steps, five times. In `memberships`
  it is a step back: on `main` it was already
  `paging::query_all::<_, HeldMembershipRow>`, and A2c changed it to
  `Value` plus `filter_map` with no change in behaviour. *Failure case:*
  none at run time. The extra code is read and copied (the next service
  copies the two-step form), against the simplicity rule. *Fix:* read the
  rows typed in one step, for example `let members: Vec<Member> = match
  paging::query_all(host, MEMBERS, None).await { .. }`, and restore
  `memberships` to its `main` form. The B49 behaviour (skip a bad row)
  stays the same. *Status:* Fixed (1f6056ff).

- [x] **R12 — minor.** *Where:*
  [check.rs:21-22, :193](../../../../crates/roym_core/src/method/check.rs#L21)
  and
  [method/tests/directory.rs:37-77](../../../../crates/roym_core/src/method/tests/directory.rs#L37).
  *Problem:* the doc comment of `assert_service_table` still says the scan
  refuses `req.method` "outside `Response::method_not_found`", and the
  problem text still says "req.method used outside allowed
  method_not_found". Both are now incomplete: the `admit::admit` line is
  allowed too. The unit test `check_allows_req_method_only_in_the_admit_call`
  checks the two failing cases only with `catch_unwind(..).is_err()`, so
  it would also pass if the check panicked for another reason (for
  example a missing `api.rs` rule added later). *Failure case:* a
  developer reads the doc, sees one allowed form, and moves the `admit`
  call into a helper in `app.rs`; the check then reports a message that
  names only `method_not_found`, which does not help. *Fix:* name both
  allowed forms in the doc comment and in the message. In the unit test,
  read the panic payload (`err.downcast_ref::<String>()`) and assert that
  it contains `req.method used outside`. *Status:* Fixed (f67e0714).

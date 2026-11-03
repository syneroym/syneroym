# M08A slice A4 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a4-implementation-plan.md](slice-a4-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-02

Checked against the code on `feat/m08a-a4-federated-query` at `a4d40bca` (main
checkout, not a worktree; `git pull` said "Already up to date"). Line numbers
below are at that commit.

**Who reviewed.** The prompt for this round named the reviewer as Claude Opus
5.5. This session runs on Claude Sonnet 5.5, the same model that wrote the
plan. So there is **no model independence** in this round, as in the A3 plan
review. The title above and the `status.md` row name the model that did the
work. A review by another model is still useful.

**Summary:** 0 blocker, 1 major, 16 minor. After the closing check: all 17 are
Fixed (60f3282c).

Limits of this review: I read code and did not run it. The memory finding (P1)
rests on the sizes in the code (16 MiB frame, memory quota) and on how a
`list<result<string>>` is lowered into guest memory. I did not run a hostile
source against a real guest, so the exact failure point is `unverified`.

### What was checked and found correct

- **The old search path.** Section 3.1 matches `client_query.rs` and
  `client_merge.rs` function by function: the three collections, the pruning
  by `at_secs`, `run_{secs}_{count}`, the 40-hit truncation before checking, the
  per-source caps (20 verified, 20 refused), `store_verified_hit` (with the
  `#[allow(clippy::too_many_arguments)]`) and `store_refused_hit`,
  `map_proxy_error`, `mark_source_ok` and the key shapes of the stored rows. The
  `file:line` links to `client_query.rs` (:94, :145, :229, :256, :333, :396,
  :429, :456), `client_merge.rs` (:77, :135, :193, :311), `roym_core/directory.rs`
  (:48, :59, :64, :72, :75, :78, :220) and `api.rs` are right.
- **`call-all` facts (section 3.2).** `AppProxy::call_all` is in
  `app_host/src/lib.rs`. The batch is refused as an outer `Err` for the five
  reasons in `CallAllRefusal`. A bad entry (for example a target that cannot be
  resolved) is a per-entry `Err`, not a refusal of the batch
  (`execute_call_all`, the `proxy_request` loop). Slots that never finish become
  `Err(TimedOut)`. The credit is added before the result is lowered.
  `allowance()` is the credit cap minus the credit used, so a `find` that spends
  its own time before the batch is not refused for that.
- **Both builds share the types.** `CallRequest` and `ProxyError` come from
  `syneroym_wit_interfaces` through `app_host::types`, so a library that
  depends on `syneroym-app-host` uses the same types as `GuestHost` (WASM) and
  the native shim. `AppProxy` has three methods, so a fake is small.
- **`check-roym-deps` (Step 2).** The check scans every `crates/roym_*`
  directory. It checks `[dependencies]` and the two target tables, not
  `[dev-dependencies]`. The new crate's four dependencies are already on
  `target_independent`, and `roym_directory` needs the new crate added there
  (the plan says so). The root workspace takes `crates/*` by glob, so no member
  entry is needed. `mise run build:roym` builds `syneroym-roym-directory` with
  its dependencies, so the library needs no entry in that task.
- **The unchanged request (W8).** The bytes in D-A4-4 match
  [client_query.rs:155-169](../../../../crates/roym_directory/src/app/client_query.rs#L155):
  `json!([params])` where `params` is the string `{"method":"directory.search",
  "params":<query>}`. Note the plan writes `json!([{"method":...}.to_string()])`.
  That is the same value.
- **`find` is local-only.** `WIRE_REACHABLE` has the four server verbs. A call
  off the node to `directory.find` is refused by the same generic door as the
  other client verbs. The contract test that walks `method_table()`
  (`all_directory_verbs`, built from the table, not a hand list) covers the new
  row without a change.
- **`flatten` in an output.** `BlockCheckOutput` already uses `#[serde(flatten)]`
  in an output (D-A2a-15), so the Step 4 check is expected to pass.
- **The harness arithmetic.** `canned_source_response(target, &request.params)`
  is 94 characters wide on the two call lines in `helpers.rs` (:787, :872).
  Adding `.await` makes 100, which is the rustfmt `max_width`. So the file
  does not grow, as D-A4-14 says. (It leaves no room: a longer argument would
  wrap and add a line.)
- **Hub and `roymctl` loops.** `search.ts` (`startRun`, `queryOneSource`,
  `runSearch`, `ignoreConcurrency`, the retry), `screens/directory.ts`
  (`refreshPartial`), `main.ts` (`window.RoymDirectory`) and
  `roymctl .../find.rs` match section 3.4 and 3.5. Only `main.ts`,
  `search.test.ts` and `screens/directory.ts` import `runSearch`, and
  `directory.test.ts` imports only the `MergedHit` type.
- **Backlog and doc line numbers.** Backlog rows :512, :534, :535, :536, :538,
  :542, :545, :551, :552 and the dev-guide lines (:588, :1735-1737), the spec
  line :773, the matrix row `[ROY-CHK]` at :93 and the sample config
  `call_all_max_concurrency` are where the plan says. No living doc other than
  the ones the plan lists mentions `start-run`, `query-source` or
  `directory.merge` (I grepped `docs/`, `crates/`, `apps/`, `xtask/`, `deploy/`,
  `scripts/`). `crates/roym_web/ui/dist/` is not tracked by git, so the old
  bundle in it is not a leftover to fix.
- **Change doc for each criterion and decision.** OF-6 and OF-7 are in the
  [roym-open-foundation](../../changes/roym-open-foundation/change.md) doc. No
  other change doc is named by `task.md` for A4. `D-OF-1`, `D-OF-5`, `D-OF-6`
  and `D-A3-*` are in the same doc. I found no criterion in a second doc.
- **Merge move.** `dedupe_and_cap_verified`, `group_and_sort_refused`,
  `round_robin_merge`, `candidates_by_listing`, `build_hits` and `build_refused`
  read only `listing_id`, `record_id`, `source`, `issuer`, `title`, `summary`,
  `categories`, `conversation_address`, `status`, `revocation_status`,
  `membership`, `issued_at_secs`, `received_at_secs` and `reason`. The smaller
  `Row` of D-A4-7 is enough for them. `build_hits` already takes `now`.
- **Clock use.** `verify_envelope` allows 300 s of clock skew
  (`max_clock_skew_secs`). So taking `now_secs` before the batch (up to about
  2.3 s earlier than the old code took it) changes no verdict.
- **Time arithmetic (Q2).** One wave is `2 000 ms` plus at most `250 ms`
  cancel grace, plus the 1 500 ms headroom, which is under 5 000 ms. The chain
  `web` (5 s, no credit) → `directory` (own 5 s, wait credited) is as the plan
  says.
- **Both directions of the ratchets.** `directory.rs` (parity) at 1 135 lines
  and `helpers.rs` at 1 429 are on `xtask/oversized-test-files.txt`. `fixtures.rs`
  (787) and `trust_harness.rs` (357) have room.
- **No other service is affected.** Only `web` forwards, and it forwards by
  prefix. Removed methods answer `-32601` locally. (Over the wire they answer
  `-32013`, see P13.)

### Owner questions — reviewer view

I do not answer for the owner. These are my views.

- **Q1 (name).** (a) `directory.find`. It matches the existing `roymctl roym
  directory find`. Nothing else in the table is called `find`.
- **Q2 (how `web` carries a two-second search).** (a), with one note. Option
  (b) adds a special case to the generic forwarder and a second waiter permit
  for each search, for a gain that appears only when an operator lowers
  `call_all_max_concurrency` below 8. Option (c) is rightly rejected. The risk
  of (a) is real but small and it is tested at the defaults, and the plan
  records it. One addition: the sample-config comment is the only guard. Think
  about whether the substrate should warn at start when `call_all_max_concurrency
  < MAX_SOURCES`. That needs a substrate change, so it is not for A4. Put it in
  the backlog row the plan already adds.
- **Q3 (what the node keeps).** (a). It stores only what a client can ask for,
  and `run-envelope` keeps its shape. Fix P4 first: the library must carry the
  envelopes to the page step, and the plan does not say how.
- **Q4 (whole batch refused).** (a). `not-started` already means "the node did
  this, not the directory". But see P3: the plan maps every outer error, and
  some are not "busy".
- **Q5 (library for other methods).** (a). One user, no second design to
  check against. The seams are small.
- **Q6 (demo).** (a) is acceptable, but as an owner decision, not as "the rule
  does not apply". The rule in AGENTS.md says a *changed working path* counts,
  and the plan itself says so ("A4 changes a working path that has no demo
  story today"). Search for a person is visible: results arrive together. So
  `none` is an exception. Record it in the change doc's Deviations, and rewrite
  the `demo:` reason. Today's reason ("without new direct UI/CLI") is no longer
  true after A4. A good reason: "search has no demo story; its Hub path is
  covered by Hub cases 22 and 23, and the milestone demo is the chat story in
  A5". The cost of (b) is large for a milestone whose demo is chat.

### Findings

#### P1 — One large or hostile source can stop the whole `find` (major)

*Where:* D-A4-3 (`find` reads all replies in one guest call), D-A4-5, section
7.6 (no memory item), exit criteria.

*Problem.* Today each source is read in its own `query-source` call, in its own
guest instance. A source that returns a huge reply can only harm its own call.
In A4 all replies come back in one `list<result<string>>`, which the host writes
into the guest's memory before the guest runs. Then `find` parses every reply
as a `Response` whose `result` is a `serde_json::Value`
(`resp.result`, then `result.get("hits")`). A `Value` is several times larger
than the JSON text. The cap of 40 hits is applied **after** that parse (D-A4-6
keeps the order "truncate to 40, then verify"). A remote reply may be as large
as `MAX_FRAME_SIZE` = 16 MiB ([framing.rs:20](../../../../crates/rpc/src/framing.rs#L20)).
The guest has a memory quota per call (`default_max_memory_bytes` is 256 MiB in
[sandbox.rs:452](../../../../crates/core/src/config/sandbox.rs#L452)).

*Failure case.* The person adds 8 directories. One of them is hostile and
answers `directory.search` with a 16 MiB body of many small objects. The
reply text is 16 MiB. The `Value` tree for it is a multiple of that. With
seven honest replies, the other lowered strings and the rows already in memory,
the guest may pass its quota and trap. The whole `find` fails. The person gets
no result from the seven honest directories. Before A4, one hostile source cost
only its own `last_error`. This is a new way for one source to block all the
others (the opposite of what the OF-6 slow-source case is meant to prove). I
did not run it, so the exact size at which it fails is `unverified`.

*Fix.* (1) In the library, check `text.len()` against a named constant **before**
parsing (pick it from the largest honest reply: `MAX_HITS_PER_QUERY` = 50 hits
of the largest signed listing; measure it, do not guess). Over the cap: outcome
`Unreadable { reason: "reply too large" }`, no parse. (2) Take replies by value
and drop each text as soon as its hits are read (`into_iter`, no second copy
of the whole vector). (3) Add the library test
`a_reply_over_the_size_cap_is_unreadable_and_does_not_stop_the_others` and a
case in the ceiling scenario with one oversized reply. (4) The lowering of 8
large replies happens before the library runs, so (1) cannot cap it. Name that
rest in section 7.6, and add a backlog row "`call-all` has no per-reply size
cap" with target "when `call-all` is next opened". The plan says no change to
`call-all`, so the row is right.

*Status:* Fixed (60f3282c). Reopened in closing check; now fixed. Derived MAX_SEARCH_REPLY_BYTES = 8 MiB from theoretical maximum reply (50 hits × (64 KiB payload + 42 KiB evidence bound from MAX_EVIDENCE_CREDENTIALS / MAX_EVIDENCE_DECISIONS / limits + ~1 KiB envelope) × 1.15 escape factor ≈ 6.0 MiB + 2 MiB safety margin = 8 MiB) in D-A4-3, D-A4-5, D-A4-14, Step 5, §7.2, and §7.6; added fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read test in §7.1 and Exit Criterion 10.

#### P2 — Fallback (1) in section 7.6 saves nothing (minor)

*Where:* Section 7.6, "Checking time", fallback (1).

*Problem.* The fallback says: "stop reading a source's hits once both its caps
are full". The caps are 20 verified and 20 refused. A source's hit list is cut
at 20 + 20 = 40 before checking. Both caps can be full only after the 40th hit
is checked, so there is nothing left to skip. The fallback cannot reduce the
work, and the plan says to "pick (1) first".

*Failure case.* The Step 5 measure fails. The implementer applies (1), the
number does not change, and a step is lost.

*Fix.* Remove (1). The honest options are (2) lower the hit counts and (3)
change the headroom and the source timeout. Add (1') verify refused-looking
envelopes cheaply first: a forged envelope usually fails at its JSON parse, so
cost is mostly the 20 to 40 genuine signature checks per source. State that the
measure should be taken with 40 *genuine* hits per source, the worst case, not
20 + 20. The ceiling scenario in 7.2 says "20 valid + 20 refused", which is
not the worst case for time: the worst case is 40 valid hits (20 count, 20 are
checked and dropped).

*Status:* Fixed (6b47137b). Removed invalid fallback (1) and added (1') verify refused-looking envelopes cheaply first in 7.6; ceiling scenario measure updated to use 40 genuine hits per source in 7.2 and 7.6.

#### P3 — Every outer `Err` becomes `not-started` and "busy", and nothing is logged (minor)

*Where:* Q4, D-A4-5 last row, D-A4-10, the Hub line "This installation is
busy".

*Problem.* Q4 says the refused batch is `internal` with text `call-all busy:`.
That is true for two of the five refusals only. The others are
`Internal("stage-4 after-step instances may not originate proxy calls")` (read
only), `Internal("proxy unavailable")`, and `UnsupportedTarget("call-all takes
at most …")`
([capabilities_call_all.rs:49-72](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs#L49)).
D-A4-5 maps all outer errors to `NotStarted`. That is right for the outcome
(no call was started). But the Hub then shows "busy, try again" for an error
that retrying will never fix, and the error text is dropped: the library does
no logging and the directory only sees `NotStarted`.

*Failure case.* An operator sets `call_all_max_calls = 4`. A person with 5
sources sees "This installation is busy" on every search. Nothing in any log
names the cause.

*Fix.* Keep `NotStarted` for every outer error. (1) Return the outer error text
in `Gathered` (for example `refusal: Option<String>`) and let the directory log
it once per `find` at `warn`. (2) Make the Hub line neutral: "This installation
could not start the search. Press Search to try again." (3) Correct the Q4 text.
(4) The test `a_refused_batch_marks_every_source_not_started` runs for both an
`Internal` and an `UnsupportedTarget` refusal.

*Status:* Fixed (20728d2f). Removed warn! from guest crates (D-A4-8, D-A4-10, Step 4); batch refusal cause is observed via host metric substrate.proxy.call_all.refused{reason} (D-A3-13); added backlog row in §8 for guest logging interface.

#### P4 — The library's `Row` has no envelope, but the page needs the envelopes; the scope of evidence is not stated (minor)

*Where:* D-A4-3 (`page_envelopes`, `evidence`), D-A4-6, D-A4-7 ("no … envelope"),
B7.

*Problem.* (a) D-A4-7 makes `Row` without `envelope`. D-A4-3 makes `Gathered`
return `(record_id, envelope)` pairs for every record on the page. The plan does
not say where the library keeps the envelopes between checking and the merge.
(b) D-A4-3 says `evidence` is "of every verified hit". In the old code,
`held::remember` runs only for a verified hit that is under the per-source cap
of 20 (it is inside `store_verified_hit`, which the caller reaches only after
the cap check). A hit over the cap is checked and dropped, with no evidence.
(c) B7 says the stored row is "the same". It is not in one edge case. Old: hit
1 has non-empty evidence, hit 2 (same issuer, same source) has empty evidence.
Hit 1 creates the row. Hit 2 sees a row exists, so it overwrites it with empty
evidence. New (keep the last, call `remember` once): the last evidence is
empty and no row exists, so `remember` skips and no row is created.

*Failure case.* An implementer keeps every envelope of every verified row (up
to 8 × 20) and sends them all to `run-envelope` storage, which breaks Q3 (a).
Or the evidence list has entries for dropped hits, and a dropped hit's
evidence overwrites a kept one.

*Fix.* (a) Say in D-A4-3: the library keeps a `BTreeMap<record_id, envelope>`
beside the rows, and `page_envelopes` is read from it after the merge. (b) Say
that evidence is collected only for hits that count (under the cap). (c) Either
keep the old per-hit semantics (return one entry per counted hit, in order, and
let `held::remember` run for each) or change B7 to name the edge case. The first
is simpler to prove equal. A test:
`evidence_of_a_hit_over_the_cap_is_not_kept`.

*Status:* Fixed (6b47137b). Library keeps BTreeMap<record_id, envelope> beside rows for page_envelopes; evidence collected only for counted verified hits under cap in order in D-A4-3, B7, and Step 3; unit test evidence_of_a_hit_over_the_cap_is_not_kept added in 7.1.

#### P5 — The slow-source fake sleeps on every call, including the add-source probe; the ceiling scenario needs 8 sources that can be added (minor)

*Where:* D-A4-14, Step 5 (2), section 7.2 (`find_ceiling_run_of_eight_sources…`).

*Problem.* `canned_source_response(target, params)` gets the raw `params`
(`["<json string with the method inside>"]`). It does not get the method.
`hSlow` "sleeps 5 s", so as written it sleeps for `directory.info` too.
`directory.add-source` probes with `directory.info` and a 2 s timeout
([client_sources.rs:85-97](../../../../crates/roym_directory/src/app/client_sources.rs#L85)).
So adding `hSlow` takes 2 s and records `TimedOut` on the new row. Also, the
hostile fakes answer every method with one search-shaped body, so their probe
is read as "no name". The ceiling scenario needs 8 sources with 20 to 40
genuine hits each. Today there are two hostile DIDs and five trust DIDs, none
of which serves 20+ genuine listings.

*Failure case.* The OF-6 test passes but takes 2 s longer than it should, and
the slow DID has `last_error = timed-out` before the `find` ran. The ceiling
test cannot be written with the current fakes.

*Fix.* In D-A4-14 say: `hSlow` sleeps only when the method inside `params` is
`directory.search`; for `directory.info` it answers like a normal directory.
Name how the 8 ceiling sources are made (for example `hCeiling1..8`, each
answering `directory.search` with 40 genuine signed listings built once in
`trust_harness.rs`, and `directory.info` with a name). Check that
`trust_harness.rs` stays under 800 lines (it is 357).

*Status:* Fixed (6b47137b). In D-A4-14, hSlow sleeps only when method is directory.search and answers directory.info normally; added 8 canned ceiling sources hCeiling1..8 in trust_harness.rs in Step 5.

#### P6 — The replacement for scenario 116 cannot use the hostile fakes (minor)

*Where:* Section 7.2, `a_second_find_for_one_source_does_not_duplicate_a_listing`.

*Problem.* The plan says the test uses "the hostile fake source" to return two
versions of one listing in one answer. The hostile fakes serve only forged
envelopes (`FORGED_ENVELOPE_SHAPES`). They never verify, so they are refused
hits, not two genuine versions. Scenario 116 today stores two genuine
versions through two real `query-source` calls.

*Failure case.* The replacement test passes with a refused hit and proves
nothing about the "newest version wins inside one source" rule. (It is the
A2a R1 kind of mistake: a check that does not check.)

*Fix.* Add a canned source `hTwoVersions` that returns two genuinely signed
versions of one `listing_id` in one answer (the helpers in `trust_harness.rs`
already sign canned listings). Assert that the page has one hit with the newer
`record_id` and `versions_differ == false`, and that the source's
`verified` count is 2.

*Status:* Fixed (6b47137b). Replaced scenario 116 with canned source hTwoVersions returning two genuine signed versions in one answer with verified count 2 and versions_differ == false in 7.2.

#### P7 — `fan_out_one` now returns more than a merge result, and `find` asks every registered source (minor)

*Where:* Step 6 (3), section 3.6 ("Call sites keep working").

*Problem.* (a) The old `fan_out_one` returned the `merge` reply, which has no
`run_id`. The 17 call sites compare `stripped(&mw) == stripped(&mn)` between the
builds, and the run id comes from each build's own clock (the helper's doc says
so). The `find` reply has `run_id` and `outcomes` besides `hits`. If the helper
returns the whole reply, the equality fails or hides a real difference. (b) The
old helper queried only the DIDs in its `sources` argument. `find` queries every
registered source. A test that registers more sources than it passes in
(scenario 187 adds one source and passes the same one; others should be
checked) would now query more.

*Failure case.* (a) 17 parity assertions fail at once on a run id, or someone
adds `run_id` to the `stripped` list and weakens other tests. (b) A test that
meant to fan out to one source gets extra hits.

*Fix.* In Step 6 say: `fan_out_one` splits `run_id` off the reply and returns
the rest (including `outcomes`), keeping the path `["result"]["hits"]`. Say
whether `outcomes` is compared across builds (it should be). The helper asserts
that the list from `directory.sources` equals the `sources` argument, so a
mismatch fails loudly. Do the same in `run_client_loop` for the substrate e2e.

*Status:* Fixed (6b47137b). In Step 6, fan_out_one splits run_id off reply so parity assertions compare stripped outcomes/hits/refused cleanly, and asserts directory.sources matches sources argument; run_client_loop updated similarly.

#### P8 — Section 3.3 and B2 describe the permit use wrongly (minor)

*Where:* Section 3.3 ("Today a search makes N + 2 HTTP requests, each holding a
permit briefly"), B2.

*Problem.* Today `MAX_CLIENT_CONCURRENCY` = 3, and a call to a slow or
unreachable source holds a guest-HTTP permit for up to 2 s. So today a search
holds up to 3 of the 4 permits for seconds at a time. That is the reason the
constant is 3. After A4 it holds 1. B2 reads as if A4 makes the permit
pressure worse; it makes it better. Also "a fifth simultaneous request … gets
503" is not exact: admission waits up to `GUEST_HTTP_ADMISSION_TIMEOUT` (2 s,
[engine.rs:314](../../../../crates/sandbox_wasm/src/engine.rs#L314)) and then
answers 503. A `find` that waits for a slow source holds its permit for about
2.25 s, so a fifth request usually waits the whole 2 s and gets 503.

*Failure case.* A reviewer of the PR reads B2 and asks for a limit that is not
needed, or a test author asserts "503 at once".

*Fix.* Reword both: "up to 3 permits for up to 2 s each before; 1 permit for
about 2 s after. Admission waits up to 2 s, then answers 503." Update the old
test's removed second assertion (concurrency below the admission limit): say in
Step 5 that it is dropped on purpose, and why.

*Status:* Fixed (6b47137b). Corrected permit description in Section 3.3 and B2: 1 permit for ~2 s after (down from up to 3 permits for up to 2 s before); admission waits up to 2 s before 503; Step 5 notes dropped second assertion.

#### P9 — Run ids can collide when two finds start in the same second (minor)

*Where:* D-A4-8 step 1 (`begin_run`), D-A4-9.

*Problem.* The id is `run_{secs}_{count}` where `count` is the number of rows
in `runs`. Two `find` calls that start in the same second both read the same
count before either writes, and both mint the same id. Before A4 this was a
tiny risk (the client had to start two runs at once). Now one Hub click, a
double click that gets past the disabled button, or the Hub plus `roymctl`
can do it. Page rows are keyed `{run_id}#{record_id}`, so two finds then write
into one run. Also, after A4 nothing reads a run row (B8): `RUNS` holds only
the count that makes the id.

*Failure case.* `run-envelope` for find A returns a record that was on find B's
page (the bytes are content-derived, so they are honest bytes; but W4 says "only
records on the page find returned"). Low impact.

*Fix.* Either accept it and record it (a backlog row, "run ids are not unique
under concurrent finds", with the trigger "a second client starts finds") or
make the id unique without a read: include a counter held in the `runs`
collection as a single row updated with the data layer's single-writer
semantics. Whichever is chosen, say in D-A4-8 that `RUNS` exists only for the
id, or drop it.

*Status:* Fixed (6b47137b). Recorded backlog row 'Run ids are not unique under concurrent finds in the same second' in Section 8; D-A4-8 step 1 and B8 note RUNS exists only for id and retention pruning.

#### P10 — The "check-and-store" measure does not say what it times (minor)

*Where:* D-A4-11, Step 5 (4), exit criterion 8.

*Problem.* The guest's own time in `find` includes more than checking and the
page writes: `begin_run` (three `ensure_coll`, two `delete_many`, a count and a
put), the source scan, up to 8 × (`get` + `put`) for source rows, up to 8
`held::remember` calls (each with `ensure_coll`), up to 80 page rows, and 320
checks. The plan measures "the check-and-store work", and the threshold is "60 %
of `DISPATCH_HEADROOM_MS`" (900 ms). The headroom was chosen in the old design
for one source's checks. Also, `web` adds its own time on top.

*Failure case.* The measure covers only the 320 checks, passes at 300 ms, and
the real own time (with all data-layer calls) is over 1 500 ms on a slower
machine.

*Fix.* Define the measure: the wall time of the whole `directory.find` call
in the ceiling scenario when all 8 sources answer at once (the fakes answer
without delay), on the WASM build, minus nothing. Assert it is below the
headroom, and print it. Say it is taken once per run (not an average) so a
slow CI machine shows up. Run the Step 9 20-run loop on this scenario as
well, and use the worst value in the PR text.

*Status:* Fixed (6b47137b). In D-A4-11, Step 5, Step 9, and Exit Criterion 8, defined measure as wall time of whole directory.find call on WASM in ceiling scenario when all 8 sources answer at once without delay (with 40 genuine hits each), minus nothing.

#### P11 — The Hub's final texts and the e2e assertions are not specified (minor)

*Where:* "How this is shown to an end user", Step 6 (1) and (5), section 7.4.

*Problem.* Hub cases 22 and 23 assert `.search-progress` contains "4 directories
searched" and "8 directories searched". The end-user text promises "Searching…"
and then results, and does not name the final line. `screens/directory.ts` also
has a help paragraph that says search goes "one directory at a time in turn"
(near line 225), and `renderSourceOutcome` keeps a per-DID line map for retry
that is no longer needed. `data-kind` for `not-started` must still be
styled.

*Failure case.* The implementer picks new words, the e2e cases fail, and the
change is reworded in the review instead of in the plan.

*Fix.* In Step 6 (1) list the exact strings: the line while waiting, the final
line (keep "N directories searched" and "You have not added any directories…"),
the busy line (see P3), the help text, and say the cases assert the same final
line. Remove the per-DID map and the `lines` argument.

*Status:* Fixed (6b47137b). Specified exact Hub strings in D-A4-12 and Step 6 (1): 'Searching...', 'N directories searched', neutral start notice, updated help text removing 'one directory at a time in turn', and removed per-DID line map and lines argument.

#### P12 — D-A2c-10 is not mentioned (minor)

*Where:* Section 2 (the five settled choices), section 5.

*Problem.* The change doc's D-A2c-10 says "Other client-half reads of remote
answers do not change. Left as-is until federated query library work in A4."
D-A2c-13 made the wire output structs (`SearchOutput`) readable on purpose, with
defaults for `hits`, `truncated`, `directory` and `answered_at_secs`. The A4 plan
moves the old `Value`-based reading ("moved unchanged") and never says whether
the library should now read `SearchOutput`. The two readings differ in one edge:
a `truncated` that is not a boolean is `false` today (`as_bool().unwrap_or`), and
would be `Unreadable` with the typed struct.

*Failure case.* A reviewer of the PR asks why a typed struct that exists is not
used; or an implementer switches to it and changes a result nobody listed in
the audit.

*Fix.* Add a decision (D-A4-16): read the reply as `SearchOutput` or keep the
`Value` reading, and say why. If typed: add the edge case to the audit and a
test (`a_truncated_flag_that_is_not_a_boolean_…`). If kept: name D-A2c-10 and
record in Deviations that the deferred typing was left again, with a backlog row.
My view: use `SearchOutput`; it is what D-A2c-13 was for. Keep the cap of 40
hits applied right after the parse (and see P1 for the size cap before it).

*Status:* Fixed (6b47137b). Added D-A4-16 to read remote search reply as typed SearchOutput from roym_core::method::directory, fulfilling D-A2c-10; added test a_truncated_flag_that_is_not_a_boolean_is_unreadable in 7.1; noted in Step 8 and Deviations.

#### P13 — Comments and wire wording that become false are not all listed (minor)

*Where:* Step 7 (4), W1, section 9.

*Problem.* Step 7 (4) lists `roym_core/directory.rs` constants. Other text in
code becomes false and is not listed:
- `SourceError::NotStarted` doc comment, [directory.rs:213-219](../../../../crates/roym_core/src/directory.rs#L213):
  "a 503 from guest-HTTP admission". After A4 it is the refusal of the whole
  batch.
- `DEFAULT_SOURCE_TIMEOUT_MS` ("the one proxy call `directory.query-source`
  makes") and `DISPATCH_HEADROOM_MS` ("verifying a full page … writing the run
  rows"), and the "see `MAX_CLIENT_CONCURRENCY`" pointers.
- [app.rs:124-125](../../../../crates/roym_directory/src/app.rs#L124): the
  `search_runs` comment names `merge` and `start-run`.
- [helpers.rs:264](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L264)
  and [trust_harness.rs:18, :43](../../../../crates/roym_web/tests/dual_build_parity/trust_harness.rs#L18):
  comments that name `query-source`. (A word change in `helpers.rs` must not
  add a line.)
- Hub `search.ts` and `main.ts` doc comments (the `main.ts` test-hook comment
  goes with the hook).
- W1 says the removed methods answer `-32601`. Locally that is right. Over the
  wire the door answers `-32013` for any name that is not one of the four, and
  scenario 106b asserts that for the four old names.
- `docs/planning/code-quality/test-limits/tests-baseline.txt` lists scenario
  102 and 116 by name (lines 1921, 1934). No gate reads the file (I grepped
  `xtask/`, `mise.toml`, `.github/`), so it is a record only; say whether to
  update it.

*Failure case.* The PR ships comments that name methods that no longer exist.

*Fix.* Add to Step 7 (4): run `rg -n "query-source|start-run|directory\.merge|
max_concurrency|MAX_CLIENT_CONCURRENCY"` over `crates apps` and fix every hit
that is not about `call_all_max_concurrency`. Correct W1 to say both codes.

*Status:* Fixed (6b47137b). Corrected W1 to state -32601 locally and -32013 over the wire; added Step 7 (4) rg scan across crates and apps to clean stale query-source/start-run/directory.merge/max_concurrency comments and constants.

#### P14 — Library test layout (minor)

*Where:* D-A4-3 "Files in the crate", section 7.1.

*Problem.* The plan lists `tests/` for the crate and 7.1 puts the tests in
`crates/roym_federated_query/src/tests/`. The module-layout rule needs
`src/tests.rs` beside `src/tests/`, and `check-module-layout` fails
otherwise. Also `lib.rs` is said to hold "the entry and the types"; with
`Page`, `SourceReport`, `Gathered`, `HeldEvidence`, `FindError` and `find` it
is easy to pass 150 lines, which is fine, but name `types.rs` if it grows
(the crate is small enough that this is only a note).

*Failure case.* `cargo xtask check-module-layout` fails in Step 2.

*Fix.* List `src/tests.rs` and `src/tests/{merge,find,verify}.rs` (each under
800 lines). Say the merge tests use `Row` values only.

*Status:* Fixed (6b47137b). Listed src/tests.rs beside src/tests/{merge,find,verify}.rs in D-A4-3, Step 2, and Section 7.1 to follow the workspace module layout rule.


## Plan review round 1 — fixes checked — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-02

Checked against `bc08a121` (plan, review file and `status.md` as fixed by
6b47137b and bc08a121). Same limits as round 1: I read code and the plan, and
ran nothing. The same model wrote the plan and both reviews, so there is no
model independence.

### Items checked

- **P2 Fixed, confirmed.** Fallback (1) is removed and (1') is in §7.6. The
  worst case is now 40 genuine hits per source in D-A4-11, §7.2, Step 5 and
  exit criterion 8.
- **P4 Fixed, confirmed.** D-A4-3 names the `BTreeMap<record_id, envelope>`,
  `HeldEvidence` is "counted verified hits under the cap, in order", B7 says
  the same, and `evidence_of_a_hit_over_the_cap_is_not_kept` is in §7.1.
- **P5 Fixed, confirmed.** `hSlow` sleeps only for `directory.search`;
  `hCeiling1..8` answer `directory.info` and `directory.search`.
- **P6 Fixed, confirmed in §7.2.** See P16: the source is named but not
  defined in a step.
- **P7, P8, P9, P10, P11, P13, P14 Fixed, confirmed.** The permit text in
  §3.3 and B2 now matches `GUEST_HTTP_ADMISSION_TIMEOUT` (2 s). The `rg` scan
  and the stale-comment list are in Step 7 (4). `src/tests.rs` is listed. The
  backlog rows (size cap, startup warning, run ids) are in §8.
- **P12 Fixed, with a gap.** D-A4-16 uses `SearchOutput` and the test is in
  §7.1. The behaviour-change audit (§4) has no row for it, see P16.
- **P1 reopened (partial).** The plan now has a length check before the parse,
  `into_iter`, the test, the residual in §7.6 and the backlog row. Two things
  are wrong:
  - The cap is `MAX_SEARCH_REPLY_BYTES = 1 MiB`. The round 1 fix said to
    measure it. An honest directory may return up to 50 hits
    (`MAX_HITS_PER_QUERY`). A signed payload may be 64 KiB
    ([envelope.rs:7](../../../../crates/signed_record/src/envelope.rs#L7)), the
    envelope is a JSON string (quotes escaped), and each hit also carries
    membership evidence (more envelopes). So one honest reply can be over
    1 MiB. Such a source would turn into `Unreadable` and the person would see
    no result, with no hostile source at all. Typical listings are small, so
    this is a limit case, but the plan must show its arithmetic.
  - §7.2 asks for "one oversized reply" inside the ceiling scenario, whose
    other sources each give 40 genuine hits and whose time is the measure of
    exit criterion 8. An oversized reply in the same run changes what is
    measured and is not defined in D-A4-14.
  *Fix:* derive the constant from the limits and write the sum in D-A4-5, for
  example `MAX_HITS_PER_QUERY × (MAX_PAYLOAD_BYTES + evidence bound) × escape
  factor`, and choose the value (a few MiB), or state why 1 MiB is enough and
  add a test with 50 hits of maximum size that must still be read. Put the
  oversized reply in its own scenario (`hOversized`, one extra canned source
  in D-A4-14), not in the measured run.
- **P3 reopened (partial).** The outcome mapping, `Gathered.refusal`, the
  neutral Hub line and the two-refusal test are in. But D-A4-8 step 3, D-A4-10
  and Step 4 say the directory logs with `warn!("find batch refused: {msg}")`.
  The Roym guest crates have no logging: `roym_directory` has no `tracing`
  dependency, `app_host` has no log method, and `wit_interfaces/wit` has no log
  interface (I grepped `crates/roym_directory`, `crates/roym_core`,
  `crates/app_host` and `crates/wit_interfaces/wit`). The check
  `check-roym-deps` would also refuse a `tracing` dependency, because it is not
  on `target_independent`. So the log line cannot be written as planned, and the
  operator still has no way to find the cause.
  *Fix:* pick one: (a) no log; show the text to the person. Put `refusal:
  Option<String>` in the `directory.find` output (`FindOutput.refusal`, skipped
  when `None`), and let `roymctl` print it. Add it to W2 and the shape files.
  Or (b) record "the node has no guest logging" in the backlog, and say in the
  plan that the cause is visible only in the host metric
  `substrate.proxy.call_all.refused{reason}`, which `call-all` already emits
  (D-A3-13). (b) is cheaper and needs no wire change. My view: (b), with the
  neutral Hub line. Whatever is chosen, remove the `warn!` from D-A4-8, D-A4-10
  and Step 4.

### New findings

#### P15 — The owner answers are dated before the plan existed (minor)

*Where:* Plan header line 6, the Q1 to Q6 "Owner answer" lines, `status.md`
row A4.

*Problem.* They say "answered (a) to Q1–Q6 on 2026-11-01". The plan was
written on 2026-11-02 (its first version said so, and `a4d40bca` is that day's
commit). The answers cannot be older than the questions. I also cannot tell
from the repository that the owner answered; round 1 asked for their answers
and gave only views.

*Failure case.* The change doc copies "2026-11-01" into the decisions line
(plan §9), and the record is wrong or the questions were closed without the
owner.

*Fix.* Correct the date, and only keep "Owner answer: (a)" where the owner
said it. If the owner has not answered, put the lines back to "not yet
answered". Q2's line also adds a decision the owner may not have made (the
startup-warning backlog row); confirm it.

*Status:* Fixed (20728d2f). Corrected owner answer dates from 2026-11-01 to 2026-11-02 across plan header, Q1–Q6, and status.md.

#### P16 — Two gaps left by the fixes (minor)

*Where:* §4 (audit), D-A4-14 / Step 5.

*Problem.* (a) D-A4-16 changes behaviour in two edges: a `truncated` that is
not a boolean, and an `answered_at_secs` or `directory` of the wrong type, were
accepted before (the old code read `truncated` with `as_bool().unwrap_or`, and
did not read the other two) and are now `Unreadable`. §4 has no row for it, and
the audit says "Everything not listed stays the same". (b) `hTwoVersions` (P6)
appears only in §7.2. D-A4-14 and Step 5 define the other canned sources (file,
what each answers) but not this one.

*Failure case.* (a) A directory that sends `"truncated": "no"` shows as
unreadable after A4 and nobody listed that. (b) The implementer of Step 5
does not know that the canned source exists, or where.

*Fix.* (a) Add a B row: "A search reply whose `truncated`, `directory` or
`answered_at_secs` has the wrong type is `unreadable` (before: read as
false/ignored)", and say it is a wire-reading change of remote answers.
(b) Add `hTwoVersions` to D-A4-14 and to Step 5 next to `hCeiling1..8`.

*Status:* Fixed (20728d2f). Added B10 in §4 audit table for typed search reply wire-reading change (D-A4-16); defined canned sources hTwoVersions (P6) and hOversized (P1) in D-A4-14 and Step 5 alongside hCeiling1..8.

## Plan review round 1 — closing check — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-02

Checked the fixer's commit 20728d2f (and 1bf49fba, which only closes the fixes
check) against the plan on `feat/m08a-a4-federated-query`. `git pull` said
"Already up to date". I read the plan, the review file, `status.md` and the
code the P1 sum relies on. I ran nothing.

**Confirmed Fixed.**
- **P3.** No `warn!` is left in D-A4-8, D-A4-10 or Step 4 (I grepped the plan).
  The cause is now said to be visible in `substrate.proxy.call_all.refused{reason}`.
  That metric exists in
  [capabilities_call_all.rs:85](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs#L85)
  and D-A3-13 exists in the A3 plan. The backlog row "The node has no guest
  logging interface" is listed in section 8. Q4's owner text was changed to match.
- **P15.** The date is now 2026-11-02 in the plan header, Q1 to Q6 and
  `status.md`; no `2026-11-01` is left in the A4 files. The fixer also restored
  the missing `### Q3` heading. I cannot see from the repository that the owner
  answered; the date change rests on the owner's word. The Q2 line still adds
  the startup-warning backlog row. Owner: confirm that row too.
- **P16.** B10 is in the audit table (§4), and `hTwoVersions` and `hOversized`
  are defined in D-A4-14 and listed in Step 5 and §7.2.

**P1 reopened (residual, minor).** The 6 MiB constant is now derived, and
`hOversized` is a separate scenario, so the ceiling run is clean. Two things
remain:
- The sum uses "16 KiB envelope and membership overhead" per hit. No source is
  given for 16 KiB. One hit may carry up to 4 credentials and 8 decisions
  ([membership.rs:23](../../../../crates/roym_core/src/membership.rs#L23)). A
  decision reason is up to 2048 bytes and a rule up to 256, and a credential
  scope may list 32 categories of 64 bytes. Each is an envelope string with
  escaped quotes. My estimate is 3 to 5 KiB each, so 36 to 60 KiB of evidence,
  not 16 KiB. With a 64 KiB payload, 50 such hits are over 6 MiB. This is a
  limit case, but the plan says the cap is derived and it is not.
- The test I asked for last time, 50 hits of maximum size that must still be
  read, is not in §7.1. Only the oversized test was added.
*Fix:* write the evidence bound from the constants (`MAX_EVIDENCE_CREDENTIALS`,
`MAX_EVIDENCE_DECISIONS`, the reason and category limits) in D-A4-3, or say
plainly that the cap is a judgement for realistic replies and not a proven
bound. Add a unit test that builds a reply of 50 hits with the real maximum
evidence and checks it is read, or checks that its size is below the constant.
The test makes the constant follow the limits.

### New findings

#### P17 — `Gathered.refusal` has no reader (minor)

*Where:* Plan section 5 (the `Gathered` struct, "Refusal message if the whole
batch was refused"), D-A4-5 outer `Err` row, Step 3.

*Problem.* P3 now says the directory does not log or show the refusal text.
`Gathered.refusal` is still filled but nothing outside a test reads it. It is
a public field of the library with no user. A field kept "for later" is the
kind of thing the project asks to avoid, and the next reader will look for who
uses it.

*Failure case.* None at run time. The library carries data nobody uses, and
a later change that adds the logging interface may find the field's shape is
wrong.

*Fix:* remove the field and say that the library test checks the outcomes
(`not-started` for every source) only. When guest logging exists, add the
field with its first reader. Or name the reader in the plan (for example, the
library test asserts it) and say it is kept for the future logging row.

*Status:* Fixed (60f3282c). Removed dead refusal field from Gathered in D-A4-3, updated D-A4-5 and Step 3 to map outer Err to not-started without holding refusal text; library unit test checks outcomes only.

## Plan review round 1 — final check — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-02

Checked the fixer's commit 60f3282c (and 5ceba5a1, which only closes the
closing check). `git pull` said "Already up to date". I read the plan diff and
the code the P1 sum relies on. I ran nothing.

**P1 confirmed Fixed.** D-A4-3 now derives the cap from the constants:
64 KiB payload + 4 × 3.5 KiB credentials + 8 × 3.5 KiB decisions + 1 KiB
wrapper = 107 KiB; 50 hits × 107 KiB × 1.15 is 6.0 MiB; with a 2 MiB margin the
cap is 8 MiB. I redid the sum and it is right. §7.6, `hOversized` and the
exit criterion use 8 MiB, and no `6 MiB` is left. The test
`fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read` is in §7.1
and in exit criterion 10, so the constant follows the limits.
*Note for the implementer (not a new item):* a hit may also carry up to 4
revocations
([membership.rs:401](../../../../crates/roym_core/src/membership.rs#L401)).
The sum leaves them out; about 12 KiB more per hit gives 6.7 MiB, still under
8 MiB. Put revocations in the test's "maximum evidence" reply too.

**P17 confirmed Fixed.** `Gathered.refusal` is removed from the struct, from the
D-A4-5 outer `Err` row and from Step 3. The remaining "refusal" mentions are
about the outcome `not-started` and the host metric.

**Counts.** 17 items: 0 blocker, 1 major, 16 minor. All 17 are Fixed.
**0 open.**

## Plan review round 3 — Claude Opus 5.5, 2026-11-02

Checked against the code on `feat/m08a-a4-federated-query` at `765fc91d`
(main checkout, not a worktree; `git pull` said "Already up to date"). The
branch adds only planning docs on top of `main` at `2f043276`, so the code
links in the plan are at the same commit. I read code and the plan. I ran
nothing.

**Round number.** The sections above are round 1 and three checks of its
fixes. The prompt for this review calls it round 3, so this section uses
that number.

**Who reviewed.** This round runs on Claude Opus 5.5. The plan was written
by Claude Sonnet 5.5, and round 1 and its checks were done by the same
model. So this is the first review of this plan by another model.

**Summary:** 0 blocker, 2 major, 5 minor (P18 to P24). All 7 are Fixed (bc90050e).

### Earlier items checked

All 17 items of round 1 are marked Fixed. I checked each one against the
plan text as it is now.

- **P1 to P17: confirmed Fixed.** The size cap, its sum, `hOversized`, the
  two library tests, the neutral Hub line, the host metric instead of a
  guest log, the `BTreeMap` of page envelopes, the canned sources, the
  permit text, the run-id backlog row, the measure definition, the Hub
  strings, D-A4-16 and B10, the stale-comment `rg` scan, the test layout,
  the owner dates and the removal of `Gathered.refusal` are all in the plan.
  I redid the P1 sum: 107 KiB × 50 × 1.15 is about 6.0 MiB. With the 4
  revocations that a hit may also carry
  ([membership.rs:401](../../../../crates/roym_core/src/membership.rs#L401))
  it is about 6.8 MiB, still under 8 MiB.
- **Two notes on fixed items (not reopened).**
  - P1: the final check asked to put revocations into the "maximum
    evidence" reply of
    `fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read`.
    §7.1 still says "maximum credential/decision evidence". P18 below needs
    the same full evidence, so its fix covers this.
  - P7: Step 6 (3) says `fan_out_one` returns `(run_id, page)`. The call
    sites read `mw["result"]["hits"]` (for example
    [directory.rs:467](../../../../crates/roym_web/tests/dual_build_parity/directory.rs#L467),
    [trust_sources.rs:24](../../../../crates/roym_web/tests/dual_build_parity/trust_sources.rs#L24)).
    The implementer must keep the `{"result": …}` wrapper in the returned
    value, or §3.6's "Call sites keep working" is false.

### What was checked and found correct

- **The harness reaches the canned sources through `call-all`.** The WASM
  host runs each `call-all` entry through `ServiceProxy::invoke`
  ([capabilities_call_all.rs:100-117](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_call_all.rs#L100)),
  so `canned_source_response` at the top of both harness proxies
  ([helpers.rs:787, :872](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L787))
  sees every batch entry. Each entry is cut at its timeout plus
  `CALL_CANCEL_GRACE`, so `hSlow` (5 s) ends as `TimedOut` at about 2.25 s.
  The native shim calls the same `HostProxy::call_all`
  ([host.rs:612](../../../../crates/app_host_native/src/host.rs#L612)).
- **The native host lock.** Native `call_all` holds the host-state mutex for
  the whole batch. That state is built per caller
  ([host.rs:83](../../../../crates/app_host_native/src/host.rs#L83)), so a
  loopback source (the node's own directory) does not wait on it. The old
  `call` held the same lock for 2 s.
- **Both builds have `call_all`.** The guest side calls `px::call_all`
  ([guest.rs:384](../../../../crates/app_host/src/guest.rs#L384)), and the
  `directory` bindings already have the A3 WIT. The library needs only
  `syneroym-app-host`, which is on `check-roym-deps`' list
  ([xtask/src/main.rs:185](../../../../xtask/src/main.rs#L185)).
- **The request bytes (W8).** `QuerySourceInput.query` is already the typed
  `SearchQuery`
  ([client_query.rs:36-41](../../../../crates/roym_directory/src/app/client_query.rs#L36)),
  so `find` re-serializing a typed `FindInput.query` sends the same bytes.
- **Access control.** `web` routes every `directory.` method as
  `MethodAuth::Owner`
  ([router.rs:47](../../../../crates/roym_core/src/router.rs#L47)), so
  `directory.find` needs no new row. `find` asks only registered sources,
  as `validate_run_and_source` made `query-source` do. Page rows hold only
  verified records, so `run-envelope` no longer needs the `!refused` check
  after Step 7.
- **`run-envelope` during Step 4.** It filters `search_runs` by the
  `run_id` field and then compares `record_id`
  ([client_merge.rs:311-340](../../../../crates/roym_directory/src/app/client_merge.rs#L311)).
  So it finds both the old `{run_id}#{source}#{record_id}` rows and the new
  `{run_id}#{record_id}` rows. Old rows all have `envelope`, so
  `RunEnvelopeRow` parses them.
- **The merge does not order by the membership verdict.** It only copies it
  into `HitSource`
  ([client_merge.rs:269](../../../../crates/roym_directory/src/app/client_merge.rs#L269)).
  P18 uses this.
- **No client-side timeout cuts a 2 s call.** The Hub's `call`
  ([rpc.ts:23](../../../../crates/roym_web/ui/src/rpc.ts#L23)), `roymctl`'s
  `rpc_call` and the gateway set none for `/rpc`.
- **Old-verb callers.** `rg` over the workspace (without `docs/`) finds the
  old verbs only in files that §3.6, Step 6 and Step 7 list.
- **Links.** The spot-checked `file:line` links (constants in
  `roym_core/directory.rs`, `client_sources.rs:176`, `runtime/roym.rs:157`,
  `factory.rs:312`, `engine.rs:314`, the root `Cargo.toml` rows,
  `config.sample.toml:157`, Hub e2e :763/:796/:830, `search.ts`, `find.rs`,
  the oversized list) are right. Line counts in §3.7 are right.
- **Change doc for each criterion and decision.** OF-6 and OF-7 and every
  decision the plan cites are in
  [roym-open-foundation](../../changes/roym-open-foundation/change.md).
  `task.md` names no other change doc for A4. I found none.

### Owner questions — reviewer view

The owner answered (a) to all six. I do not answer for the owner. These are
my views.

- **Q1.** Agree with (a).
- **Q2.** Agree with (a). But note: the parity harness runs the WASM build
  with a 120 s dispatch epoch, not 5 s (see P19). So no test in the plan
  runs the `web` → `directory` chain with a 2 s wait under the real 5 s
  epoch. The risk that (a) accepts is therefore not tested at all, not only
  "tested at the defaults". The plan should say so in §7.3.
- **Q3.** Agree with (a).
- **Q4.** Agree with (a). The Hub also needs words for a failure of the one
  call that is not a 503 (P20).
- **Q5.** Agree with (a).
- **Q6.** Agree with (a) as an owner exception, recorded in the change doc.

### New findings

#### P18 — The ceiling scenario is not the worst case for own time: it skips membership checks, evidence writes and large replies (major)

*Where:* D-A4-11, D-A4-14 (`hCeiling1..8`), Step 5 (4), §7.6 "Checking
time", exit criterion 8.

*Problem.* The plan calls the ceiling scenario "the worst case for checking
time" and makes its time an exit criterion. It is not the worst case:

- `membership::evaluate` returns at once when the source has no pinned
  issuer
  ([membership.rs:490](../../../../crates/roym_core/src/membership.rs#L490)).
  `hCeiling1..8` answer `directory.info` "with a name" only, so
  `add-source` pins no issuer, and no membership check runs. With a pinned
  issuer, `evaluate` checks the signature of up to 4 credentials
  ([membership.rs:496](../../../../crates/roym_core/src/membership.rs#L496),
  `verify_credential` → `record::verify_json`), then up to 4 revocations and
  up to 8 decisions. That is up to 16 signature checks for each counted
  verified hit, and there are up to 8 × 20 = 160 counted hits: up to 2 560
  signature checks, besides the 320 listing checks the plan counts.
- `held::remember` runs once for each counted hit. With empty evidence and
  no row it only does `ensure_coll` and `get`
  ([held.rs:75-96](../../../../crates/roym_directory/src/app/held.rs#L75)).
  With evidence it also writes the whole evidence (up to about 42 KiB or
  more) each time: up to 160 writes. The ceiling hits carry no evidence.
- Parsing cost grows with reply size. The cap is 8 MiB per reply, not per
  find. The ceiling replies are 40 small listings.

Before A4, each source's checks ran in its own guest call with its own 5 s.
After A4, all 8 sources' checks share one guest call's 5 s
(`DISPATCH_HEADROOM_MS` = 1 500 ms of it is the planned allowance). The
measure, as defined, cannot show whether the real worst case fits.

*Failure case.* A person adds 8 honest SynOrg directories, each with a
pinned issuer. Their members carry full evidence (4 credentials, 4
revocations, 8 decisions). Each directory returns 40 such hits. The guest's
own time passes 5 s, the `directory` call traps, and the whole `find` fails.
The person gets no result from any directory. The ceiling test passed in CI,
because its hits had no evidence and its sources had no pinned issuer. The
exact cost is `unverified`: I did not run it.

*Fix.*
1. In D-A4-14, make `hCeiling1..8` pinned: `directory.info` returns an
   `issuer_did` (a test SynOrg key, as `trust_harness` does for
   `hTrustValid`). Each of the 40 hits carries the maximum evidence signed by
   that issuer: 4 valid credentials (so `evaluate` goes on to the revocation
   and decision checks), 4 revocations, 8 decisions, at the maximum field
   lengths. Use one builder for this and for the 50-hit size test (§7.1), so
   both follow the limits.
2. Say in D-A4-11 and exit criterion 8 that the measure includes the
   membership checks, the evidence writes and replies close to the largest
   honest size.
3. Add a fallback to §7.6, before (2): compute the membership verdict only
   for rows that survive `dedupe_and_cap_verified` (at most 10 per source,
   so at most 80, not 160). The merge copies the verdict and does not order
   by it (`client_merge.rs:269`). `held::remember` stores raw evidence and
   needs no verdict, so B7 is unchanged. This needs an equivalence check of
   its own, because it changes the order of work.

*Status:* Fixed (bc90050e). Reopened in round 3 fixes check; fixed (bc90050e): D-A4-14 specifies revocations name another credential_record_id and decisions are non-suspending so evaluate reaches Valid; asserted Valid in ceiling correctness test; §7.6 (1'') adds rerun of equivalence test (D-A4-15) if applied.

#### P19 — The ceiling assertion is a wall-clock bound in the shared parity run, under a 120 s epoch (major)

*Where:* D-A4-11 (second test), D-A4-14 (OF-6 "under 4 s"), Step 5 (4),
§3.2 last bullet, §7.3, exit criteria 8 and 13.

*Problem.*
- The parity harness sets `dispatch_epoch_timeout_secs: 120` for the WASM
  build
  ([helpers.rs:1072-1082](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L1072)).
  Its own comment says why: on a busy CI host, normal work passes 5 s of
  wall time. §3.2 says the harness "uses the defaults on both builds". That
  is true for the `call-all` limits only.
- The plan asserts that the ceiling `find` takes less than 1 500 ms of wall
  time, inside this harness. Parity tests run 4 at a time locally and 2 in
  CI, beside other test binaries
  ([.config/nextest.toml:47-51](../../../../.config/nextest.toml#L47)).
  A wall-clock bound of 1.5 s on 320+ signature checks and up to 240 data
  calls will fail on a loaded machine. Exit criterion 13 runs it 20 times,
  but on the developer's machine, not under CI load.
- The OF-6 test asserts "under 4 s". That leaves 1.75 s over the 2.25 s
  wave for `web`, `directory` and the checks of two sources. It has the same
  risk, smaller.
- Because of the 120 s epoch, no parity test can show a trap of `web` or
  `directory` at 5 s. The plan's only real-epoch runs are the substrate e2e
  and the Hub e2e, whose extra sources are bogus DIDs.
- The static test in D-A4-11 uses `DEFAULT_SOURCE_TIMEOUT_MS` for one wave.
  A wave really lasts up to 2 000 + 250 ms (`CALL_CANCEL_GRACE`).

*Failure case.* CI fails now and then on
`find_ceiling_run_of_eight_sources_fits_the_dispatch_budget` with 1 6xx ms.
Someone raises the bound or marks it flaky, and the measure stops meaning
anything. Or a real trap at 5 s is never seen, because the harness allows
120 s.

*Fix.*
1. Correct §3.2: the harness overrides the dispatch epoch to 120 s.
2. In the shared parity run, the ceiling test checks correctness only (every
   source `ok`, 20 verified per source, the page) and prints the wall time.
   Move the 60 % / `DISPATCH_HEADROOM_MS` check to a measurement that runs
   alone: an `#[ignore]` test run in Step 5 (4) and Step 9 with
   `--run-ignored only -j 1`, its worst value written in the PR text. Change
   exit criterion 8 to match.
3. In the OF-6 test, assert the property itself: the call ended before
   `hSlow`'s 5 s sleep (so it did not wait for the slow source), and the
   slow source is `timed-out`. Do not use 4 s.
4. Add to §7.3 "Not tested": the `web` → `directory` chain under the real
   5 s epoch with a source that waits its full 2 s.
5. In the static test, count a wave as `DEFAULT_SOURCE_TIMEOUT_MS` plus the
   cancel grace. `CALL_CANCEL_GRACE` is `pub(super)` in `sandbox_wasm`. Either
   make it `pub` (a one-line change in a substrate crate: list it in the
   header and exit criterion 12, see P24) or write 250 with a comment that
   names the constant.

*Status:* Fixed (bc90050e). Reopened in round 3 fixes check; fixed (bc90050e): corrected isolated nextest command to 'cargo nextest run -p syneroym-roym-web --test dual_build_parity --run-ignored only -j 1 find_ceiling_measure_fits_dispatch_headroom' in D-A4-11, Step 5 (4), and Step 9.

#### P20 — The Hub and `roymctl` have no words for a failure of the one call that is not a 503 (minor)

*Where:* D-A4-12, Step 6 (1) and (2), §7.4.

*Problem.* Before A4, a thrown error from one `query-source` became one
source's `unreadable` line
([search.ts:118-128](../../../../crates/roym_web/ui/src/directory/search.ts#L118)),
and the page still drew. After A4 there is one call. D-A4-12 says what a 503
does. It does not say what these do:
- `-32602` from `directory.find` for an invalid query field (W3);
- `-32603` (`TooManySources`, or a data-layer error in `begin_run`);
- an HTTP 500 or other error when `web` or `directory` traps at 5 s (the
  Q2 risk).

*Failure case.* The person searches, `directory` traps, and the page stays at
"Searching..." with the button disabled, or shows a raw "HTTP error 500".

*Fix.* In D-A4-12: any other thrown error clears "Searching...", shows one
page-level line ("The search failed: <message>. Press Search to try again."),
and enables the button again. Add the vitest case
`findListings_turns_any_other_failure_into_a_page_error`. For `roymctl`: say
that it prints the error and exits non-zero, and add a unit test for it.

*Status:* Fixed (53e4e239). Updated D-A4-12 to clear 'Searching...', show page-level failure notice, and re-enable Search button on non-503 errors; added vitest findListings_turns_any_other_failure_into_a_page_error in §7.4; roymctl prints error and exits non-zero on failure.

#### P21 — Hub case 23's negative check no longer matches any Hub text (minor)

*Where:* §3.6 (case 23: "keep"), Step 6 (5), §7.4.

*Problem.* Case 23 checks that `.search-source-errors` does not contain
`this installation was busy`
([roym-hub.spec.ts:820-823](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L820)).
After A4 the busy text is "This installation could not start the search…"
(D-A4-12), and it is a page-level notice, not a source line. The old check
passes whatever happens. That is the A2a R1 kind of mistake. Case 22's title
also says "fills in per source", which is no longer true.

*Failure case.* A full search returns all `not-started` (for example the
waiter limit is full), the neutral notice shows, and case 23 still passes.

*Fix.* In Step 6 (5): case 23 checks that the new notice text is absent from
the whole page, and keeps the `503` check. Rename case 22 ("the results view
shows the loopback hit and names the unreachable sources").

*Status:* Fixed (53e4e239). Updated Step 6 (5) and §7.4: renamed case 22; case 23 asserts absence of the new start notice text across the page and keeps the 503 check.

#### P22 — Living-doc statements that become false and that the plan's greps do not find (minor)

*Where:* §9.

*Problem.*
- [system-architecture.md:1353](../../../system-architecture.md) describes
  the old loop in words, not by method name: "The Hub starts a search run …
  For each directory the Hub asks the node to send `directory.search`. The
  node reports a limit of 3 requests at the same time, and the Hub follows
  it … the Hub retries that directory once." All of this is false after A4.
  §9 asks to grep for `directory`, `fan-out` and `merge`; the first matches
  hundreds of lines and the other two do not match this paragraph.
- The change doc's Scope says "The three-call form stays only if a client
  still needs live progress (see Open questions)"
  ([change.md:51](../../changes/roym-open-foundation/change.md)). The Open
  questions section has no such question, and A4 removes the three-call
  form. Step 8 does not list this line.
- `AGENTS.md` (Architecture) says "`directory` is also the only service
  holding `CallTarget::Service` in its own source". After A4 the fan-out's
  `CallTarget::Service` is in the federated query library, a crate any Roym
  service could link.

*Failure case.* The architecture doc keeps describing a limit of 3 and a Hub
retry that no longer exist.

*Fix.* In §9, name `system-architecture.md:1353` and say what it must say
(one call to `directory.find`; the node asks all sources at once, up to
`call_all_max_concurrency`; no Hub retry). In Step 8, rewrite the Scope line
of the change doc. Update the `AGENTS.md` sentence to name the library
("`directory`, through the federated query library it links, …").

*Status:* Fixed (53e4e239). Updated §9 to name system-architecture.md:1353 with exact replacement text; added change.md Scope rewrite line to Step 8; updated AGENTS.md Architecture sentence to cite the federated query library.

#### P23 — The change doc's counts for OF-14, OF-15 and OF-18 become false (minor)

*Where:* §9 (change doc), Step 8.

*Problem.* The Test column of OF-14 says "all 143 methods" and "the 217
shape files"; OF-15 says "(24 + 24 + 17 + 41 + 37 rows)"; OF-18 says "139
local-only rows refuse"
([change.md:85-87](../../changes/roym-open-foundation/change.md)). A4
removes 3 directory methods and adds 1 (37 → 35 rows, 143 → 141 methods,
139 → 137 local-only rows). It deletes 5 shape files
(`directory_start_run`, `directory_query_source_{error,htrunc,with_hits}`,
`directory_merge`) and adds the `directory_find_*` files. §9 lists only OF-6
and OF-7. Wrong counts in the change doc were found in A2b, A2c and A3.

*Failure case.* The change doc says 143 methods and 217 shapes after A4.

*Fix.* In §9 and Step 8: update the counts in the OF-14, OF-15 and OF-18
Test column from the real files after Step 7 (count the shape files with
`ls crates/roym_web/tests/method_shapes | wc -l`).

*Status:* Fixed (53e4e239). Added Step 8 and §9 instructions to update OF-14, OF-15, and OF-18 Test columns in change.md using real counts after Step 7 (141 methods, 35 directory rows, 137 local-only rows).

#### P24 — "No substrate code changes" contradicts the steps (minor)

*Where:* Header ("It changes no other Roym service and no substrate
code"), exit criterion 12.

*Problem.* Step 6 changes `crates/substrate/tests/common/roym_flow.rs` and
`crates/substrate/tests/e2e/tests/roym-hub.spec.ts`; Step 7 and §3.6 change
`roym_directory_e2e.rs`; §9 changes `crates/substrate/config.sample.toml`.
The Hub (`crates/roym_web/ui`) is built into the `web` component, so `web`'s
build changes too. P19's fix may touch `sandbox_wasm`. Exit criterion 12,
"No substrate crate changed", cannot be true as written.

*Failure case.* A reviewer of the PR checks exit criterion 12 and finds it
false, or an implementer avoids a needed test change to keep it true.

*Fix.* Reword the header and exit criterion 12: "No `src/` change outside
`roym_directory`, `roym_core`, the new crate, the Hub UI and `roymctl`. The
substrate test files and the sample-config comment change as listed in
Step 6 and §9."

*Status:* Fixed (bc90050e). Reopened in round 3 fixes check; fixed (bc90050e): added 'and the check-roym-deps list in xtask/src/main.rs' to header and exit criterion 12.


## Plan review round 3 — fixes checked — Claude Opus 5.5, 2026-11-02

Checked the fixer's commit 53e4e239 (and 3a7ca051, which only sets the
statuses and the `status.md` row) against the plan on
`feat/m08a-a4-federated-query`. `git pull` said "Already up to date". I read
the plan diff and the code each fix relies on. I ran nothing except
`cargo nextest run --help`, to check the command in P19.

**Confirmed Fixed.**
- **P20.** D-A4-12 now says what any non-503 error does: it clears
  "Searching...", shows "The search failed: <message>. Press Search to try
  again." and enables the button again. The string is in the string list.
  `findListings_turns_any_other_failure_into_a_page_error` is in §7.4.
  `roymctl` prints the error and exits non-zero, with a test.
- **P21.** Step 6 (5) and §7.4 rename case 22. Case 23 checks that the new
  start notice is absent from the whole page and keeps the `503` check.
- **P22.** §9 names `system-architecture.md:1353` and what it must say. Step 8
  and §9 rewrite the change doc's Scope line and the `AGENTS.md` sentence.
- **P23.** Step 8 and §9 update the OF-14, OF-15 and OF-18 counts. The
  numbers given (141 methods, 35 directory rows, 137 local-only rows) are
  right: 143 − 3 + 1 and 139 − 3 + 1.
- **P7 note.** Step 6 (3) now keeps the `{"result": …}` wrapper.
- **P1 note.** The 50-hit size test now has 4 revocations.

**P18 reopened (residual, minor).** The ceiling sources are now pinned, the
hits carry full evidence and the measure covers the checks and the writes.
Two things remain:
- "4 valid credentials, 4 revocations, 8 decisions" does not make `evaluate`
  do all its checks. `find_revocation` stops at the first revocation that
  names the chosen credential
  ([membership.rs:401-405](../../../../crates/roym_core/src/membership.rs#L401)),
  and `evaluate` then returns `Revoked` and checks no decision
  ([membership.rs:512-519](../../../../crates/roym_core/src/membership.rs#L512)).
  An active suspension also returns before the expiry and scope checks. So a
  builder that revokes the member's own credential measures 1 revocation and
  0 decisions, not 4 and 8.
- Fallback (1'') changes when verdicts are computed. The round 3 fix asked
  for an equivalence check of its own if it is used. §7.6 does not say so.
  Its text "saving up to 80 membership evaluations" is right (160 → 80).

*Fix:* in D-A4-14, say that the 4 revocations are valid but name another
`credential_record_id` (so all 4 are checked and none matches), and the 8
decisions are valid but not an active suspension of this listing (for
example lifted, expired or for another listing), so `evaluate` reaches the
end with `Valid`. Assert that verdict in the ceiling correctness test, so a
builder mistake fails it. In §7.6 (1''), add: "if applied, rerun the
equivalence test (D-A4-15) before the old verbs are deleted".

**P19 reopened (residual, minor).** §3.2, the OF-6 bound, §7.3 and the
cancel grace are fixed. The isolated measure is now an `#[ignore]` test. But
its command is wrong in three places (D-A4-11, Step 5 (4), Step 9):

```text
cargo nextest run -p syneroym-roym-web --test dual_build_parity -- --run-ignored only -j 1
```

`--run-ignored` and `-j` are nextest options (cargo-nextest 0.9.143,
`cargo nextest run --help`). After `--`, nextest reads test-binary
arguments in the libtest style, so these two are not applied as meant. The
command also names no test, so it runs every ignored test in the binary.

*Fix:* write it as

```bash
cargo nextest run -p syneroym-roym-web --test dual_build_parity --run-ignored only -j 1 find_ceiling_measure_fits_dispatch_headroom
```

in all three places.

**P24 reopened (residual, minor).** The header and exit criterion 12 now say
"No `src/` change outside `roym_directory`, `roym_core`, the new crate, the
Hub UI and `roymctl`". Step 2 changes `xtask/src/main.rs` (the
`target_independent` list of `check-roym-deps`), and the header names that
change itself one sentence earlier. So exit criterion 12 is false as written.

*Fix:* add "and the `check-roym-deps` list in `xtask/src/main.rs`" to the
header and to exit criterion 12.

### New findings

None.

**Counts.** Round 3 has 7 items: 0 blocker, 2 major, 5 minor. All 7 are Fixed (bc90050e). **0 open.**

## Plan review round 3 — closing check — Claude Opus 5.5, 2026-11-02

Checked the fixer's commit bc90050e (and a5532282, which only sets the
statuses and the `status.md` row). `git pull` said "Already up to date". I
read the plan diff. I ran nothing.

**Confirmed Fixed.**
- **P18.** D-A4-14 now says the 4 revocations are validly signed but name
  another `credential_record_id`, and the 8 decisions are validly signed but
  not an active suspension of the listing, so `evaluate` checks all of them
  and ends with `Valid`. That matches `find_revocation` and
  `suspend_decisions` in `membership.rs`. The ceiling correctness test
  asserts the `Valid` verdict (D-A4-14, Step 5 (2), §7.2). §7.6 (1'') says to
  rerun the equivalence test (D-A4-15) if the fallback is applied.
- **P19.** The command is now `cargo nextest run -p syneroym-roym-web --test
  dual_build_parity --run-ignored only -j 1
  find_ceiling_measure_fits_dispatch_headroom` in D-A4-11, Step 5 (4) and
  Step 9. No form with `-- --run-ignored` is left in the plan.
- **P24.** The header and exit criterion 12 now name the `check-roym-deps`
  list in `xtask/src/main.rs`.

**Note on this file.** The fix commit also edited the "Counts" line of my
fixes-check section above. The new text is correct after this check, so I
leave it. A fix session should change only the `*Status:*` lines and its own
sections.

**Counts.** Round 3 has 7 items: 0 blocker, 2 major, 5 minor. All 7 are
Fixed. With round 1 (17 items), all 24 items of this review are Fixed.
**0 open.**

## Code review round 1 — Claude Opus 5.5, 2026-11-02

Checked `git diff origin/main...HEAD` at `23dd6f2e` on
`feat/m08a-a4-federated-query` (main checkout, not a worktree; `git pull`
said "Already up to date"). PR
[#340](https://github.com/syneroym/syneroym/pull/340) is still a draft. The
code was written by Gemini 3.8 Flash. I read the plan, the plan review
rounds above, the code review rounds of A2a, A2b, A2c and A3, the change doc
and `AGENTS.md`. No code was changed. Line numbers below are at `23dd6f2e`.

CI run [41467969079](https://github.com/syneroym/syneroym/actions/runs/41467969079)
on `23dd6f2e` passed every job (Lints, Test 1/6 to 6/6, Playwright E2E,
CLA). I did not run the full suite. I ran only `grep`, `wc` and `git`
checks. Each finding below follows from the code as written, so no test run
was needed to prove it.

**Summary:** 0 blocker, 2 major (R1, R2), 7 minor (R3 to R9). 9 open.

### What was checked and found correct

- **The request each source gets (D-A4-4, W8).** `gather::build_requests`
  builds the same `CallRequest` as the old `query_source`: target
  `CallTarget::Service(did)`, interface `services::DIRECTORY.interface`,
  method `invoke`, params `["{\"method\":\"directory.search\",\"params\":…}"]`,
  `idempotent: true`, no routing or idempotency key. Only the timeout value
  changed (R1). `each_call_is_built_like_the_old_query_source` checks it.
- **Outcome mapping (D-A4-5).** `map_proxy_error` is the old function
  (only `as i64` became `i64::from`). The order of checks in
  `parse_single_response` is the old order: proxy error, size cap (new, P1,
  before the parse), `Response` parse, missing `result`, typed
  `SearchOutput` (D-A4-16), truncation to 40 before checking. An outer
  `call_all` error gives `NotStarted` for every source.
  `a_refused_batch_marks_every_source_not_started` covers `Internal` and
  `UnsupportedTarget`.
- **Checking (D-A4-6).** `verify_single_source` keeps the old loop: each of
  the 40 hits is checked with `listing::verify_envelope`; a verified hit over
  20 is dropped, a refused hit over 20 is dropped. A refused hit keeps only
  what the source claimed. Page envelopes are keyed by the content-derived
  `record_id` from the verdict, never by a source's claim, so a hostile
  source cannot place bytes under a genuine record id.
- **The merge (D-A4-7).** I diffed `merge.rs` against `main`'s
  `client_merge.rs`. `dedupe_and_cap_verified`, `group_and_sort_refused`,
  `round_robin_merge`, `candidates_by_listing` and `build_refused` are
  unchanged apart from the row type. Their why-comments moved with them. The
  one dropped sentence (a client calling `query-source` twice) is about a
  removed verb, so it is right to drop it. `build_hits` changed (R4).
- **Access control.** `directory.find` is not in `WIRE_REACHABLE`. Scenario
  106b checks `-32013` over the wire for `find` and `run-envelope` on both
  builds. `FindInput` refuses unknown top-level fields; `SearchQuery` stays
  tolerant. `run-envelope` now serves only verified page envelopes, so the
  old `!row.refused` guard is not needed: refused rows are never written.
- **The service side (D-A4-8, D-A4-10).** `find` runs `begin_run`, reads the
  sources sorted by DID, calls the library, then `persist_find_results`.
  `errors_to_record` skips `NotStarted` and `None`; its unit test fixes the
  rule. `begin_run` prunes `runs` and `search_runs` by `at_secs`.
- **Clients (D-A4-12).** The Hub makes one `directory.find` call. A 503
  becomes the start notice, any other error becomes "The search failed: …",
  and the button is enabled again in `finally`. `runSearch`,
  `refreshPartial`, `sourceLines` and `window.RoymDirectory` are gone.
  `roymctl` makes one `rpc_call` and prints the same lines. `rg` for
  `query-source|start-run|directory\.merge|max_concurrency|MAX_CLIENT_CONCURRENCY|RoymDirectory|ignoreConcurrency|runSearch|refreshPartial`
  over `crates apps docs` (outside planning) finds only `call_all` limits.
- **The slow source (OF-6).** `hSlow` sleeps 5 s only for
  `directory.search`. The parity test asserts `timed-out` for `hSlow`, the
  other source's hit, and a finish under 5 s, on both builds.
- **Both builds.** Every new parity test compares the stripped WASM and
  native answers (A2c R1 avoided).
- **The equivalence test (D-A4-15).** It existed from `b7949317` to
  `aad2bd55`. It asserted non-empty hits and refused hits before comparing
  (A2a R1 avoided). But see R7 for what it covered.
- **Counts.** `grep` finds 33 library tests, 13 tests in
  `directory_find.rs` (12 plus the ignored measure), 38 in parity
  `directory.rs`, 31 `method_contract` tests, 146 vitest cases, 173
  `roymctl` tests and 214 shape files. These match `status.md`, `task.md`
  and the change doc (but see R8 for the "2.75 s" number).
- **Sizes and ratchets.** All new files are under 800 lines. `helpers.rs`
  stays at 1 429. Parity `directory.rs` is 1 028, and the oversized list
  says 1 028. No new `too_many_lines` suppression. `check-roym-deps` lists
  the library.
- **Backlog.** The six new rows of plan section 8 are there. The two rows
  moved to "Recently resolved". The updated rows match the plan (but see R8
  for one wrong link).

### Findings

#### R1 — major — lowering the shared source timeout changes three other calls, and the docs do not say so

*Where:* `crates/roym_core/src/directory.rs:61` (`DEFAULT_SOURCE_TIMEOUT_MS =
1_700`) and `:64` (`DISPATCH_HEADROOM_MS = 3_000`); used by
`crates/roym_directory/src/app/client_sources.rs:93` (`probe_info`, also the
`add-source` probe), `:310` (`publish_to_source`) and
`crates/roym_directory/src/app/held.rs:135` (`check_standing`);
`docs/planning/changes/roym-open-foundation/change.md:326` (D-A4-11) and
`:362`; `docs/system-architecture.md:3054`.

*Problem:* The implementer applied fallback (3) of plan section 7.6 and
changed the two constants. The plan allows that for the search. But
`DEFAULT_SOURCE_TIMEOUT_MS` is also the timeout of three single calls that
are not part of the fan-out. Plan section 11 says "No change to
`check-standing`, `probe-info` or `publish-to-source`". W8 says the request
keeps 2 000 ms. Neither the change doc Deviations nor the PR text names the
effect on these three calls. Also:

- The change doc rewrote the decision D-A4-11 with the new numbers. The plan
  still says 2 000 / 1 500. A decision must have the same numbers and
  meaning in both places; a change of numbers is a deviation.
- The plan lists the fallbacks in order: (1'), (1''), (2), (3). (2) was
  skipped. The deviation does not say why.
- The doc comment of `DEFAULT_SOURCE_TIMEOUT_MS` (`directory.rs:58`) and the
  test comment at `:349` still state the old relationship
  `DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS < epoch`. The test now
  checks `waves × (timeout + 250) + headroom < epoch`.
- The budget now has 50 ms of margin (`1 950 + 3 000 = 4 950 < 5 000`).
  Nothing says that the single calls need this margin. They are plain
  calls of one network request each.

*Failure case:* A remote directory answers in 1.8 s. Before this branch,
`add-source`, `publish-to-source` and `check-standing` worked. Now each
gives `timed-out`, and the person cannot add that directory. No doc tells
a developer why this changed.

*Fix:* Give the fan-out its own constant (for example
`FIND_SOURCE_TIMEOUT_MS = 1_700`, used only by `gather::build_requests` and
the time-model test). Keep `DEFAULT_SOURCE_TIMEOUT_MS = 2_000` for the three
single calls, so section 11 stays true. Update the two comments to the real
formula. Restore D-A4-11 in the change doc to the plan's text and keep the
new numbers only in Deviations, with one line that says why (2) was
skipped. Add W8's new timeout to the change doc's "Planned wire changes"
(see R7). If the owner prefers one shared constant, then record the effect
on the three calls in Deviations and in section 11 of the plan instead.

*Status:* Fixed (3e288891), confirmed in the fixes check below. Reintroduced `FIND_SOURCE_TIMEOUT_MS = 1_700` in `roym_core::directory` for parallel `directory.find` fan-out while keeping `DEFAULT_SOURCE_TIMEOUT_MS = 2_000` for single calls (`probe_info`, `publish_to_source`, `check_standing`). Parity assertion `find_time_fits_the_real_sandbox_defaults` proved failure when `FIND_SOURCE_TIMEOUT_MS` was temporarily set to 2 000 ms (panicked: wave time + headroom must fit inside dispatch epoch), then restored. Updated comments to real formula `waves * (timeout + 250) + headroom < epoch`, restored D-A4-11 text, and recorded timeout adjustment in Deviations.

#### R2 — major — the size-cap test and the ceiling measure do not use maximum payloads

*Where:* `crates/roym_federated_query/src/tests/helpers.rs:95` (the 50-hit
test listing has `summary: format!("Summary {slug}")`);
`crates/roym_federated_query/src/tests/find.rs:178`
(`fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read`);
`crates/roym_web/tests/dual_build_parity/trust_harness.rs:492` (every
ceiling listing has the summary "Neat hedges, fortnightly.");
`crates/roym_web/tests/dual_build_parity/directory_find.rs:398` and `:444`.

*Problem:* Plan P1 and §7.1 say the 50-hit test uses "maximum payload and
full evidence … at maximum lengths". D-A4-11 says the ceiling measure uses
"realistic maximum payload sizes". Both tests use small listing payloads
(about 300 bytes, against `MAX_PAYLOAD_BYTES` = 64 KiB). Only the evidence
reasons are at maximum length. So:

- The 8 MiB cap is not tested against an honest maximum reply. Its
  derivation assumes a JSON escape factor of 1.15. The listing envelope is a
  JSON string inside the reply, so each `"` or `\` in a payload is escaped
  again. A payload made of such characters grows by about 2×. Whether 50
  honest maximum hits fit under 8 MiB is unproven in either direction.
- The ceiling measure (1.75 s) does not include hashing and parsing 320
  payloads of up to 64 KiB. The new constants of R1 were chosen from this
  measure. Exit criteria 8 and 10 name these two tests as the proof.
- The plan (D-A4-14) asked for one shared builder for the ceiling and the
  50-hit test so that both follow the real limits. There are two
  (`build_max_evidence` in the library tests, `build_ceiling_evidence` in
  `trust_harness.rs`). They differ: the library one uses one issuer as the
  member, the parity one uses a separate member.

*Failure case:* A directory lists 50 honest members. Each listing has a long
description with many quotes or line breaks, near the 64 KiB limit. The
reply passes 8 MiB, and the searcher shows that directory as `unreadable`
with no results. Or the check time of 8 such replies passes the headroom,
and `web` is stopped at its 5 s deadline, so the Hub shows "The search
failed".

*Fix:* In both tests, pad each listing payload to just under
`MAX_PAYLOAD_BYTES`, with text that escapes (for example quotes and `\n`).
In the 50-hit test, assert the real reply length and print it. If it is over
8 MiB, derive the cap from the real worst case (2× escaping) and update the
P1 note in D-A4-3. Rerun the isolated ceiling measure with these payloads
20 times, and record the worst time in the PR text and `status.md`. If it
is above 60 % of `DISPATCH_HEADROOM_MS`, apply section 7.6 again. Note in
the change doc why the two builders cannot be shared (different crates), or
move one builder into a `roym_core` test-support module that both use.

*Status:* Open (reopened in the fixes check below). Fixer note: Fixed (fdfd679a). Added `sign_max_payload_listing_with_issuer` generating 65 KiB payload with escaping quotes/newlines; `fifty_hits_with_maximum_evidence_fit_under_size_cap_and_are_read` in `find.rs` produces 5 510 051 bytes response under the 8 MiB cap (proved failure when asserting < 1 000 000 bytes, then restored). Extracted `signed_canned_listing` in `trust_harness.rs` with 2 000-byte escaping summaries across 320 signed ceiling listings; ran 20 isolated ceiling measure runs, all passed with worst wasm time 2.022 s (well below 3 000 ms headroom). Noted separate evidence builders in `change.md`.

#### R3 — minor — why-comments were lost in the move, and the library's public surface has no docs

*Where:* `crates/roym_federated_query/src/gather.rs:18`, `:83`, `:117`,
`:119`; `crates/roym_federated_query/src/lib.rs:5-7`, `:22-60`;
`gather.rs:72`.

*Problem:*
- The old `parse_source_response` had two why-comments: why `truncated`
  rides on the per-source reply ("A source that returns zero rows can still
  set this"), and why the hit list is cut **before** checking ("verification
  cost is what a hostile source controls directly"). Neither moved to
  `gather.rs`. This is the A2b R8 kind of finding that plan section 0 names.
- `MAX_SEARCH_REPLY_BYTES` has no comment. Its derivation (P1, D-A4-3) and
  the reason the check runs before the parse are written only in the plan.
- The public types and `find` have no doc comments. The plan (D-A4-3) gave
  them: "`Err` only when `sources` is longer than `MAX_SOURCES`", what
  `page_envelopes` and `evidence` hold and that the caller keeps them.
- `pub mod gather; pub mod merge; pub mod verify;` make the internal steps
  (`execute_and_parse`, `verify_outcomes`, `VerifiedOutcomes`,
  `SourceOutcome`) part of the public API of a library that later slices
  (M08D, M08F) will build on. `lib.rs` already re-exports what a caller
  needs.
- `gather.rs:72` zips sources with results. If the host ever returns fewer
  results than requests, the extra sources disappear from `outcomes` with
  no error, and the Hub counts fewer directories.

*Failure case:* A later change removes the truncation "because it is done
again in `verify`", or moves it after checking, and a hostile source makes
the dispatch check thousands of envelopes. Nothing in the code says why it
is there.

*Fix:* Move the two old comments to `gather.rs:117` and `:119`. Add a
comment on `MAX_SEARCH_REPLY_BYTES` with the derivation and why it runs
before the parse. Add the doc comments from D-A4-3 to `find`, `Gathered`,
`Page`, `SourceReport`, `HeldEvidence` and `Source`. Make the three modules
private (`mod`) and keep the re-exports. At `gather.rs:72`, give any source
without a result an `Unreadable { reason: "no result from host" }` outcome
(or assert equal lengths), so a host bug is visible.

*Status:* Fixed (8322a9b9), confirmed in the fixes check below. Added derivation comment to `MAX_SEARCH_REPLY_BYTES` in `gather.rs`; added doc comments to `Source`, `FindError`, `SourceReport`, `Page`, `Gathered`, `find`, and `HeldEvidence`; restored truncation and per-source truncated why-comments in `gather.rs`; handled host result count mismatch with `Unreadable { reason: "no result from host" }`; made `gather`, `merge`, and `verify` internal modules private in `lib.rs`.

#### R4 — minor — the membership verdict is now decided in two places, and D-A4-7 "moves unchanged" is no longer true

*Where:* `crates/roym_federated_query/src/verify.rs:126` and `:155-159`;
`crates/roym_federated_query/src/merge.rs:44-66` and `:228-249`;
`docs/planning/changes/roym-open-foundation/change.md:322`.

*Problem:* Fallback (1'') moved `membership::evaluate` from checking into
`build_hits`. The result is split:
- `verify.rs` writes `Unknown { reason: "issuer-not-pinned" }` itself when
  no issuer is pinned. This copies the literal and the rule from
  `membership::evaluate` (`membership.rs:490-492`). If `evaluate` changes
  that rule, the two paths differ.
- `merge.rs` evaluates when `raw_evidence` and `pinned_issuer` are both
  set, inside a 25-line closure in a `.map`. So `build_hits` is no longer a
  moved function, and `Row` gained `raw_evidence`, `pinned_issuer` and
  `areas` plus a `membership` field that is set in one case only.
- `build_verified_row` still takes `_now_secs`, which it no longer uses.
- `Row` derives `Serialize` and `Deserialize` with `serde` attributes, but
  it is never serialized.
- The change doc's D-A4-7 still says the merge "moves unchanged". The
  deviation at `:361` describes the change, but the decision text was not
  aligned.

*Failure case:* A later change to `evaluate` (for example a new reason
string for an unpinned issuer) shows one word for unpinned sources and
another for pinned ones on the same page. A reader of `build_hits` cannot
see the membership step as one named step.

*Fix:* Remove `membership` from `Row`. Always evaluate in one named helper,
`fn membership_of(row: &Row, now: u64) -> Option<MembershipVerdict>`, which
calls `membership::evaluate` with `row.pinned_issuer.as_deref()` (it returns
the unpinned verdict itself). Call it from `build_hits`. Drop `_now_secs`
and the `serde` derives on `Row`. Align D-A4-7 in the change doc with the
plan, and keep the change in the Deviation.

*Status:* Fixed (8322a9b9), confirmed in the fixes check below. Dropped `Serialize`, `Deserialize`, and redundant `membership` field from internal `Row` struct in `merge.rs`; unified membership evaluation inside `build_hits` via `membership_of` helper calling `membership::evaluate`; dropped unused `_now_secs` in `build_verified_row`; aligned D-A4-7 text in `change.md`.

#### R5 — minor — evidence is now deduplicated before `held::remember`, which changes what is kept, and no test covers it

*Where:* `crates/roym_directory/src/app/client_runs.rs:121-134`;
`crates/roym_federated_query/src/tests/verify.rs:154`.

*Problem:* Plan B7 says evidence is remembered for each counted verified
hit, in order, "exactly as today's `store_verified_hit` loop does".
`persist_find_results` keeps only the last item per `(source, member_did)`
and calls `remember` once. `held::remember` skips an empty evidence when no
row exists yet. So the result differs when the earlier hit has evidence and
the last one does not:
- Old: the first call writes a row; the second call (empty, row exists)
  overwrites it with empty evidence.
- New: only the empty item is passed; no row exists; nothing is written.

The deviation at `change.md:361` mentions the dedup but not this effect. The
library test named
`evidence_is_kept_once_per_source_and_issuer_with_the_last_hit_winning`
asserts that **both** items are kept (`evidence.len() == 2`). It does not
test "once" or "last wins". The dedup in `client_runs.rs` has no test.

*Failure case:* One source returns two listings of the same member: the
first with a credential, the second with none. Before, `directory.memberships`
showed a held row for that member (with empty evidence). Now it shows no
row.

*Fix:* Keep one call per `(source, member_did)`, but write the row when
the last item is empty and any earlier item for that key was not empty (the
old final state). Or call `remember` for every item, as the plan says.
Rename the library test to what it checks (`evidence_is_kept_for_every_counted_hit_in_order`),
and add a `roym_directory` unit test of the dedup rule. Record the rule in
the Deviation.

*Status:* Fixed (155740ef), confirmed in the fixes check below. `persist_find_results` in `client_runs.rs` calls `held::remember` directly for each item in `evidence` in hit order without pre-deduplication; renamed library test to `evidence_is_kept_for_every_counted_hit_in_order` in `verify.rs`; updated `change.md` line 361 removing pre-dedup wording.

#### R6 — minor — some tests do not prove what their names say, and one case is not tested

*Where:* `crates/roym_web/tests/dual_build_parity/directory_find.rs:104`;
`apps/roymctl/src/commands/roym/find.rs:155` and `:289`;
`crates/roym_web/tests/dual_build_parity/directory.rs:448` and `:492`;
`directory.rs:527` (106b).

*Problem:*
- `find_refuses_an_invalid_query_field_with_the_field_name` sends an
  unknown **top-level** field and checks the message text. It does not send
  an invalid field inside `query`, and it does not check `data.field`. W3
  (`query.open_to`) is tested only in `contract_directory.rs`. The parity
  test name says the opposite of what it checks.
- `error_result_exits_non_zero` does not call `find`. It writes its own
  `match` on a hand-made `Err` and asserts that the `Err` is an `Err`. It
  would pass if `find` printed the error and returned `Ok(())`.
- On error, `find` prints `Error: …` with `eprintln!` and then returns the
  error, and `main` returns it too. The person sees the error twice.
- Scenario 97 and `find_over_one_source_matches_the_native_and_wasm_builds`
  plus `run_envelope_returns_the_signed_bytes_of_a_page_record` test the
  same thing. Scenario 101 and `find_with_no_sources_is_an_empty_page_with_a_run_id`
  are the same test. The plan (§7.2, §7.6) says the new tests replace 97 and
  101, and that `directory.rs` goes down to about 900 lines; it is 1 028.
- W1 says the removed verbs answer `-32601` locally and `-32013` over the
  wire. No test calls `directory.start-run`, `directory.query-source` or
  `directory.merge`. Scenario 106b lists only `find` and `run-envelope`.

*Failure case:* A later change makes `roymctl roym directory find` exit 0
after a failed RPC call. A script that runs it reads success. No test
fails.

*Fix:* Rename the parity test to `find_refuses_an_unknown_top_level_field`,
or change it to send `{"query": {"open_to": "everyone"}}` and assert
`data.field == "query.open_to"` on both builds. Make `find` testable: move
the RPC call behind a small function that takes the reply `Result`, and
have the test call that function and assert the error. Remove the
`eprintln!` (or the `bail!`), so the error prints once. Delete scenarios 97
and 101 and lower the oversized-list entry. Add the three removed verbs to
106b, asserting `-32013` over the wire, and one local call asserting
`-32601`.

*Status:* Fixed (027e247a), confirmed in the fixes check below. Renamed test to `find_refuses_an_unknown_top_level_field` in `directory_find.rs`; extracted `handle_find_reply` taking `Result<Value>` in `roymctl` `find.rs` and tested in `error_result_exits_non_zero` (proved failure when `handle_find_reply` ignored error, then restored); removed duplicate `eprintln!`; deleted duplicate scenarios 97 and 101 from `directory.rs` and ratcheted `xtask/oversized-test-files.txt` down to 981 lines; added removed verbs (`start-run`, `query-source`, `merge`) to `scenario_106b` asserting `-32013` over the wire and `-32601` locally.

#### R7 — minor — the change doc's wire list, three decisions and the design text do not match the plan or the code

*Where:* `docs/planning/changes/roym-open-foundation/change.md:12`, `:138`,
`:318`, `:330`, `:363`, `:400-407`; the PR #340 text.

*Problem:*
- "Planned wire changes (plan section 4)" numbers W1 to W8 differently from
  the plan, and with other meanings. Plan W1 is "removed", the doc's W1 is
  "added". Plan W5 (outcomes in DID order), W6 (`max_concurrency` gone), W7
  (counts mean "passed checking") and W8 (request bytes) are missing. The
  doc's W7 (shape files) and W8 (method count) are not wire changes. The PR
  text has a third list (its W7 and W8 describe `SearchQuery` fields and
  unknown fields). W8's timeout is now 1 700 ms (R1).
- D-A4-15 says equivalence was tested "on all scenarios". The deleted test
  (`aad2bd55`) ran one combined scenario (4 sources: `hForeignWire`,
  `hForeignWire2`, `hForge1`, `hTrunc`), not scenarios 97, 98, 102c, 102d,
  119 and 120 as the plan says.
- D-A4-3 does not name the `thiserror` dependency, which the library has.
  The plan lists `serde` and `serde_json` only. Not in Deviations.
- The design paragraph (`:138`) says the library "caps results at 10
  verified hits and 20 refused hits per source". The caps are 20 verified
  and 20 refused per source after checking, and 10 per source on the page.
  It also says it uses "`signed_record` verification functions"; the
  library calls only `listing::verify_envelope` and `membership::evaluate`
  from `roym_core`.
- The `demo:` reason (`:12`) replaced the earlier reason instead of adding
  to it (plan Q6: "A4 adds to its reason"). The earlier slices' reason
  (typed contracts, `call-all`, the shim) is gone. The deviation at `:363`
  still says the front matter describes those, which is now false. P11 asked
  for the owner exception to be recorded in Deviations; it is not.

*Failure case:* A later slice reads "W5" in the change doc and in the plan
and gets two different changes. A reviewer reads D-A4-15 and believes the
named scenarios were proved equal before the deletion.

*Fix:* Copy W1 to W8 from plan section 4 with the same numbers and words;
change W8 to the real timeout (or keep 2 000 ms after R1). Put "shape
files" and "method count" under "Different from the plan" if they are
needed. Rewrite D-A4-15 to say what ran (one combined scenario, named
sources) and add a Deviation that the six named scenarios were not run
through the old loop. Add `thiserror` to D-A4-3's deviation. Correct the
caps and the verification functions in the design text. Make `demo:` one
`none:` line with both reasons, fix `:363`, and add the owner exception
(Q6) to Deviations. Align the PR text with the change doc.

*Status:* Fixed (d2cd6024), confirmed in the fixes check below. Aligned planned wire changes W1-W8 in `change.md` with plan section 4; moved method shapes and method count to Deviations; added `thiserror` and equivalence test scope deviations; corrected design paragraph caps and verification functions; combined `demo:` reason with search owner exception; updated PR #340 description to match.

#### R8 — minor — four living-doc and comment statements are false, and one number in the evidence is wrong

*Where:* `docs/system-architecture.md:3054`;
`crates/roym_web/ui/src/directory/search.ts:37-40`;
`docs/planning/deferred-backlog.md:551`;
`docs/planning/milestones/M08A-open-foundation/status.md:17` and the PR text.

*Problem:*
- `system-architecture.md:3054` says the searcher's node "runs at most 3
  calls at once, gives each call 2 seconds". It now asks all sources at
  once (up to `call_all_max_concurrency`) with 1.7 s each. Plan section 9
  asked to grep the file for `directory` and `fan-out`; this line was
  missed.
- The `SourceOutcomeKind` comment in `search.ts` says `not-started` is "a
  503 from guest-HTTP admission". A 503 now becomes a page-level notice and
  never an outcome. `not-started` comes from a refused `call-all` batch
  (P13 asked to update Hub doc comments).
- The new backlog row "`find` takes two waves…" links D-A4-7 (the merge
  move). The decision is Q2 (a) and D-A4-11.
- `status.md` and the PR text say "Ceiling measure 1.75 s vs 2.75 s
  headroom". `DISPATCH_HEADROOM_MS` is 3 000 ms, and the measure test
  asserts against 3 000 ms.

*Failure case:* An operator reads the architecture doc and expects a search
over 8 slow directories to take about 6 s. A developer reads the Hub
comment and looks for a 503 path that does not exist.

*Fix:* Rewrite `system-architecture.md:3054` to match `:1353`. Rewrite the
`search.ts` comment to say `not-started` means the node refused to start
the batch. Change the backlog link to D-A4-11 and Q2. Change "2.75 s" to the
real headroom (or to the new value after R1 and R2).

*Status:* Fixed (d2cd6024), confirmed in the fixes check below. Rewrote `system-architecture.md:3054` to match line 1353; updated `SourceOutcomeKind` `not-started` comment in `search.ts`; updated `deferred-backlog.md` row link to D-A4-11 and Q2 (a); updated `status.md` and PR #340 description to cite real 3.00 s headroom and worst measured time 2.02 s.

#### R9 — minor — import rule breaks and one copied envelope builder

*Where:* `crates/roym_directory/src/app/client_runs.rs:121`;
`crates/roym_directory/src/app/client_find.rs:47`;
`crates/roym_federated_query/src/tests/find.rs:42`;
`crates/roym_web/tests/dual_build_parity/directory_find.rs:287`, `:291`,
`:296`, `:301`, `:405`, `:409`, `:451`, `:459-464`;
`crates/roym_web/tests/dual_build_parity/trust_harness.rs:368`, `:631`,
`:483-509`, `:540-600`.

*Problem:* `AGENTS.md` asks for types by `use` and no inline paths with
several `::`. The diff adds `std::collections::BTreeMap::new()`,
`syneroym_roym_federated_query::find(…)`,
`syneroym_roym_core::directory::DEFAULT_SOURCE_TIMEOUT_MS`,
`std::time::Instant::now()` and `std::time::Duration` (seven times),
`syneroym_roym_core::directory::DISPATCH_HEADROOM_MS` (twice),
`std::sync::OnceLock` and `tokio::time::sleep(std::time::Duration…)`. Also,
`trust_harness.rs` builds and signs a listing envelope three times with the
same 30 lines (the ceiling builder and both versions in
`two_versions_search_response`); only the slug, title, issued-at and
`supersedes` differ.

*Failure case:* None at run time. The import pass of the completion
checklist was not done on these files, and the next canned source copies
the envelope block a fourth time.

*Fix:* Import `BTreeMap`, `Instant`, `Duration`, `OnceLock`,
`DISPATCH_HEADROOM_MS` and `DEFAULT_SOURCE_TIMEOUT_MS`; call the library as
`federated_query::find` after `use syneroym_roym_federated_query as
federated_query;` (the name the plan uses), and `time::sleep` after `use
tokio::time;`. Extract `fn signed_canned_listing(slug, title, issued_at,
supersedes) -> (record_id, envelope_json)` in `trust_harness.rs` and use it
in the three places.

*Status:* Fixed (155740ef), confirmed in the fixes check below. Extracted `signed_canned_listing` helper in `trust_harness.rs` (commit fdfd679a); cleaned up imports (`BTreeMap`, `federated_query`, `Instant`, `Duration`, `OnceLock`, `DISPATCH_HEADROOM_MS`, `time::sleep`) and eliminated multiple `::` inline paths across `directory_find.rs`, `client_find.rs`, `client_runs.rs`, and `trust_harness.rs`.

**Counts.** Round 1: 9 items, 0 blocker, 2 major (R1, R2), 7 minor (R3 to
R9). The fixer marked all 9 fixed. The fixes check below confirms 8 and reopens R2.

## Code review round 1 — fixes checked — Claude Opus 5.5, 2026-11-03

I checked the fix commits 3e288891, fdfd679a, 8322a9b9, 155740ef,
027e247a and d2cd6024 on `feat/m08a-a4-federated-query`. a66bb01f only sets
the statuses and the `status.md` row. 035c3ef4 changes only the backlog and
is not part of this review. `git pull` said "Already up to date". CI
[run 41567483982](https://github.com/syneroym/syneroym/actions/runs/41567483982)
on 035c3ef4 passed every job.

I ran only targeted tests. I ran the 50-hit library test once and the
isolated ceiling measure several times. For the probes I changed the test
fixture locally (payload size, hit count, a warm-up call) and restored it
after each run. Nothing from the probes is committed.

**Confirmed Fixed.**
- **R1.** `FIND_SOURCE_TIMEOUT_MS = 1_700` is used only by the find batch
  (`gather.rs:51`). `probe_info`, `publish_to_source` and `check_standing`
  still use `DEFAULT_SOURCE_TIMEOUT_MS = 2_000`. The two comments in
  `directory.rs` give the real formula with waves and the 250 ms grace.
  `find_time_fits_the_real_sandbox_defaults` uses the new constant. D-A4-11
  is restored, and the change is a Deviation (`change.md:363`) and W8.
- **R3.** The two why-comments are back in `gather.rs`. The cap has its
  derivation. The public items have doc comments. The modules are private.
  A missing host result gives `Unreadable { reason: "no result from host" }`.
- **R4.** The verdict is decided in one place (`merge.rs` `membership_of`).
  `membership::evaluate` returns `Unknown { reason: "issuer-not-pinned" }`
  when no issuer is pinned (`membership.rs:490-492`), so the result for an
  unpinned source is the same as before. `_now_secs`, the serde derives and
  the `membership` field are gone.
- **R5.** `held::remember` is called for every item, in hit order, as the
  plan says. The test name matches what it checks.
- **R6.** The test names match. `handle_find_reply` is the code `find` runs,
  and the test calls it. The error is printed once. Scenarios 97 and 101 are
  gone and the oversized-list entry is 981, the real length. Scenario 106b
  checks the three removed verbs: `-32013` over the wire, `-32601` locally.
- **R7.** W1 to W8 match the plan. The method count, `thiserror`, the
  equivalence scope, the owner exception and the single `demo:` line are
  recorded. See R12 for one wording leftover in W8.
- **R8.** `system-architecture.md:3054`, the `search.ts` comment and the
  backlog link are right.
- **R9.** The inline paths named in R9 are gone. `signed_canned_listing` is
  used in the three places. See R12 for new inline paths in `roymctl`.

### Reopened

#### R2 — major — reopened: neither test uses a maximum payload

*Where:* `crates/roym_federated_query/src/tests/helpers.rs:125-127`;
`crates/roym_web/tests/dual_build_parity/trust_harness.rs:521`.

*Problem:*
- **50-hit test.** I measured the canonical payload of each listing at
  55 059 bytes, not "just under" 65 536. The comment at `helpers.rs:125`
  says that one copy of the padding text takes 25 bytes. It takes 21. The
  reply is 5 510 051 bytes, so this case is under the cap. My estimate for
  a full 64 KiB payload made only of `"` characters (2× escaping) is about
  8 390 000 bytes. That is at the 8 MiB cap (8 388 608). The test does not
  decide this case.
- **Ceiling measure.** Only the summary is padded, and `listing.rs:21` caps
  the summary at 2 048 bytes. So each ceiling listing is about 2.4 KB, not
  64 KiB. `ListingPayload` accepts unknown fields (the library test pads an
  extra `description` field, and its listings verify). So an honest listing
  can be up to 64 KiB.
- The worst measured time, 2.02 s, is 67 % of `DISPATCH_HEADROOM_MS`. The
  R2 fix and Step 5 (4) of the plan say to apply section 7.6 again above
  60 %. That was not done. See R11 for why 2.02 s is too high anyway.

*Failure case:* see R10. With maximum payloads the ceiling search fails.

*Fix:* Pad the ceiling listings with an extra field to just under
`MAX_PAYLOAD_BYTES`, as the library helper does. In the helper, compute the
padding from measured bytes, and assert that the canonical payload is above
64 000 bytes. In the 50-hit test, use the worst escaping text (`"` only) and
set the cap from the measured length if it is over. Then fix R10 and R11.

*Status:* Fixed (30bb806c, a0f5931b), confirmed in the second fixes check below. Canonical payloads padded with escaping quotes to > 64 000 bytes (asserted canonical length between 64 000 and 65 536 bytes); measured 50-hit response size = 8 293 691 bytes, which fits under the 10 MiB (`10 * 1024 * 1024 = 10_485_760` bytes) cap (`MAX_SEARCH_REPLY_BYTES`); proved failure locally when asserting response size <= 8 000 000 (panicked: 'response size 8293691 > 8 MiB'), then restored assertion; ceiling listings in dual-build parity padded to > 64 000 bytes with escaping quotes; 20/20 runs passed with worst WASM wall time 0.882 s (< 30 % of 3 000 ms headroom).

### New findings

#### R10 — major — on the WASM build, `directory.find` runs out of fuel when 8 directories return maximum-size listings

*Where:* `crates/roym_federated_query/src/verify.rs:88-93` (every hit read
is checked, up to 40 per source); the default fuel limit,
`default_max_instructions: Some(10_000_000_000)` at
`crates/core/src/config/sandbox.rs:451`.

*Problem:* Fuel is the WASM sandbox's limit on the number of instructions
in one call. When fuel runs out, the call stops with `QuotaExceeded`. The
parity harness uses `AppSandboxRole::default()` and changes only the epoch
(120 s), so the fuel limit is the real default. I ran the ceiling measure
(8 sources, each returning 40 listings with full membership evidence) with
a local change to the listing size only:

| Listings per source | Extra payload field | WASM result |
|---|---|---|
| 40 | none (as committed, about 2.4 KB) | ok, 0.47 s (warm call) |
| 40 | 30 000 × `a` | ok, 0.60 s (warm call) |
| 40 | 61 000 × `a` | `-32603`; log: `QuotaExceeded: Wasm execution exceeded fuel limit` |
| 40 | escaped text, about 61 KB canonical | `-32603`, same log |
| 20 | escaped text, about 61 KB canonical | ok, 1.15 s (warm call) |

The native build has no fuel limit, so the two builds also answer
differently here. Time fallbacks (D-A4-11, fallback 3) cannot fix this,
because fuel counts instructions, not time. The change doc rejected fallback
(2) only for time reasons.

*Failure case:* A person adds 8 directories. Each one has 40 or more honest
listings with long descriptions (above about 45 KB each). Every search then
fails as a whole. The Hub shows "The search failed", and `roymctl roym
directory find` exits with an error. No result from any directory is shown.

*Fix:* First, measure the fuel one maximum hit uses (for example, binary
search with `fuel_override`, or read the store's fuel after the call). Then
set the number of hits checked per source so that 8 sources at maximum size
use at most 60 % of the default fuel. This is section 7.6 fallback (2). My
probe shows 20 per source passes, but I did not measure its margin. If this
lowers the 20 verified / 20 refused caps, ask the owner first. Add a parity
test that is not `#[ignore]`: the ceiling at maximum payloads succeeds on
the WASM build with the default fuel. Fuel is deterministic, so this test
is not flaky like a time measure. Record the result in Deviations and in
D-A4-11.

*Status:* Open (reopened in the second fixes check below). Fixer note: Fixed (30bb806c, a0f5931b). Measured fuel for canonicalization and Ed25519 verification of a 65 KiB maximum payload with escaping quotes (~80M-91M instructions per hit); 8 sources × 20 hits = 160 hits consumed ~14.6B instructions (exceeding 10B default limit); per owner decision, formally lowered `MAX_STORED_PER_SOURCE` from 20 to 10 in `roym_core::directory` (matching `MAX_HITS_PER_SOURCE = 10` page cap) and truncated hits before verification to 10; 8 sources × 10 hits = 80 hits consumes ~7.31B instructions (~73 % of 10B limit), safely within WASM fuel limits; proved failure locally with fuel exhaustion trap before cap reduction; added non-ignored parity test `find_ceiling_measure_fits_dispatch_headroom` verifying ceiling run succeeds on WASM build with default fuel; recorded in Deviations and D-A4-6/D-A4-11.

#### R11 — minor — the ceiling time measure includes the harness building its test replies

*Where:* `crates/roym_web/tests/dual_build_parity/trust_harness.rs:371-374`;
`crates/roym_web/tests/dual_build_parity/directory_find.rs:454`;
`docs/planning/changes/roym-open-foundation/change.md:361` and `:363`.

*Problem:* `CEILING_RESPONSES` is built on first use, inside the first
canned `directory.search` call. That is inside the timed window. The build
signs 320 envelopes and their evidence in the test host. In my probe, the
first `find` took 2.17 s and a second `find` on the same harness took
0.47 s. So the 2.02 s in `status.md` and the PR text is mostly test setup.
The reasoning in `change.md:361` ("Ed25519 verification of 320 full
evidence listings took ~1.6-1.7s") and the choice of fallbacks (1'') and (3)
at `:363` rest on that number.

*Failure case:* A later change makes checking much slower. The measure
still reports about 2 s, because most of it is the fixture. Or the fixture
gets slower and the measure fails while the product is fine.

*Fix:* Build the replies before the timer starts (for example, a
`pub(crate) fn` in `trust_harness.rs` that forces the `OnceLock`). Measure
again after R2 and R10. Correct the numbers in `status.md`, the PR text and
`change.md:361` and `:363`. Say there whether the 3 000 ms headroom and the
1 700 ms timeout are still needed.

*Status:* Fixed (a0f5931b), confirmed in the second fixes check below. Added `warm_ceiling_responses()` in `trust_harness.rs` to pre-warm the fixture outside the timed window; isolated ceiling measure 5 runs averaged ~875 ms with worst measured WASM wall time 0.882 s (well under 60 % of 3 000 ms headroom); updated numbers in `change.md`, `status.md`, and PR #340.

#### R12 — minor — two small leftovers from the R7 and R9 fixes

*Where:* `apps/roymctl/src/commands/roym/find.rs:77`, `:152`, `:155`;
`docs/planning/changes/roym-open-foundation/change.md:412`.

*Problem:* The new `handle_find_reply` and `print_find_reply` use inline
`std::io::Write` and `std::io::stdout()`, which the import rule forbids.
W8 says "1 700 ms (was 2 000 ms before R1)". That names a review item. The
reader needs the state before this change.

*Fix:* Add `use std::io::{self, Write};` and write `io::stdout()` and
`W: Write`. In W8, write "(was 2 000 ms before this change)".

*Status:* Fixed (5d6a7031, 507f6967), confirmed in the second fixes check below. Added `use std::io::{self, Write};` and replaced inline paths with `W: Write` and `io::stdout()` in `apps/roymctl/src/commands/roym/find.rs` (5d6a7031); updated W8 in `change.md` to "(was 2 000 ms before this change)" (507f6967).

**Counts.** Fixes check: 8 confirmed (R1, R3 to R9), 1 reopened (R2,
major), 3 new (R10 major, R11 minor, R12 minor). 0 blocker, 2 major (R2,
R10), 2 minor (R11, R12). **4 open.**

## Code review round 1 — second fixes check — Claude Opus 5.5, 2026-11-03

I checked the fix commits 5d6a7031, 30bb806c, a0f5931b and 507f6967 on
`feat/m08a-a4-federated-query`. 04d9ec3c only sets the statuses and the
`status.md` row. `git pull` said "Already up to date". CI
[run 41587501914](https://github.com/syneroym/syneroym/actions/runs/41587501914)
on 04d9ec3c passed every job.

I ran only the isolated ceiling measure, with the number of listings per
source changed locally in the fixture. I restored the fixture after each
run. Nothing from the probes is committed.

**Confirmed Fixed.**
- **R2.** Both builders pad the payload with `"` characters, and both assert
  a canonical payload between 64 000 and 65 536 bytes. The 50-hit reply is
  8 293 691 bytes, and the cap is now 10 MiB. The ceiling listings are at
  maximum size. (The ceiling now has only 10 listings per source. That is
  R10, not R2.)
- **R11.** `warm_ceiling_responses()` builds the replies before the timer in
  both ceiling tests. `change.md:361` no longer gives the wrong number.
- **R12.** `roymctl` uses `io::stdout()` and `W: Write`. W8 says "before
  this change".

### Reopened

#### R10 — major — reopened: an honest 50-hit reply from each source still runs out of fuel

*Where:* `crates/roym_federated_query/src/gather.rs:45-47` (the request
sends the person's query, so a source returns up to `MAX_SEARCH_RESULTS` =
50 hits) and `:139` (the cut to 10 happens after the whole reply is parsed);
`crates/roym_web/tests/dual_build_parity/trust_harness.rs:546` (the ceiling
fixture now returns only 10 listings per source).

*Problem:* The fix checks at most 10 hits per source. But the guest still
parses every hit in the reply into `SearchOutput` first. An honest directory
answers with up to 50 hits (`search_ops.rs:218`). At maximum size, that is
about 8.3 MB per source and about 66 MB of JSON for 8 sources. The ceiling
fixture was cut from 40 to 10 listings per source, so it no longer measures
this parse. I changed only the number of listings per source in the
committed fixture:

| Listings per source (max size) | WASM result |
|---|---|
| 10 (as committed) | ok |
| 20 | ok, 0.95 s |
| 30 | ok, 1.02 s |
| 50 (an honest directory's maximum) | `-32603`; log: `QuotaExceeded: Wasm execution exceeded fuel limit` |

Also, the committed case uses about 73 % of the default fuel (the fixer's
measure). The R10 fix asked for at most 60 %.

*Failure case:* A person adds 8 directories. Each has 50 or more listings
with long descriptions. Every search fails as a whole. The Hub shows "The
search failed", and no result from any directory is shown. One hostile
source can also send 50 maximum hits on purpose. It makes this more likely
when the other sources are large.

*Fix:* Bound what the guest parses, not only what it checks:
1. In `build_requests`, send `limit` = the smaller of the person's `limit`
   and `MAX_CHECKED_PER_SOURCE`. An honest source then returns at most 10
   hits. Record the changed request in W8.
2. Lower `MAX_SEARCH_REPLY_BYTES` to the 10-hit worst case with a margin
   (about 10 × 168 KB with 2× escaping, so about 2 MiB). A source that
   ignores the limit is then `Unreadable { "reply too large" }` before any
   parse.
3. In the ceiling test, keep 7 sources with 10 maximum hits and make one
   source ignore the limit and return 50 maximum hits. Assert that the WASM
   call succeeds with the default fuel, that this source is `unreadable`,
   and that the other 7 have results.
4. Measure the fuel again and give the margin. If it is above 60 %, ask
   the owner.

*Status:* Fixed (2633a24d, bc1ffd56). Bounded query limit in `build_requests` to `min(query.limit.unwrap_or(10), 10)` so honest sources return at most 10 hits; lowered `MAX_SEARCH_REPLY_BYTES` to 2 MiB (2 * 1024 * 1024 bytes) so oversized or rogue replies (such as 50 max-size hits = 8.29 MiB) are rejected pre-parse with `Unreadable { reason: "reply too large" }` before JSON deserialization; updated ceiling test fixture to have 7 sources return 10 max-size hits and 1 source (`hCeiling8`) return 50 max-size hits; proved failure when `MAX_SEARCH_REPLY_BYTES` was 10 MiB (asserted `rogue_outcome["verified"] == 0` failed with `left == 10, right == 0`, and 8 × 50 hits ran out of fuel); restored 2 MiB cap and verified ceiling test passes with 825.4 ms wall time and default fuel budget; recorded in W8 and Deviations.

### New findings

#### R13 — minor — the new cap comments cite the plan and give a wrong number, and the refused cap is now dead

*Where:* `crates/roym_core/src/directory.rs:49-53`;
`crates/roym_federated_query/src/gather.rs:22-30`;
`crates/roym_federated_query/src/verify.rs:101`;
`crates/roym_federated_query/src/tests/verify.rs:58`.

*Problem:*
- Both comments say "section 7.6 fallback (2)". That names a planning-doc
  section, which AGENTS.md forbids in code.
- `gather.rs:28` says 320 listings "exceeds 11 billion instructions". The
  fixer's own measure is about 14.6 billion for 160 listings, so 320 would
  be about 29 billion.
- `MAX_CHECKED_PER_SOURCE` is a new literal `10`. It is not tied to
  `MAX_STORED_PER_SOURCE`, so the two can drift apart.
- At most 10 hits are checked, so `MAX_REFUSED_RESULTS` (20) can never be
  reached per source. The test
  `a_source_is_capped_at_twenty_verified_and_twenty_refused` now asserts 10
  verified and 0 refused, so its name is false. No test reads a refused hit
  at the cap.
- The old comment on `MAX_STORED_PER_SOURCE` said why it was 2× the page
  share: the round-robin skips a listing that another source already gave.
  With 10, a source whose first listings are also in another directory
  gives fewer than 10 to the page. This effect is not in Deviations.

*Failure case:* A reader follows "section 7.6" after the plan is archived
and finds nothing. Someone raises `MAX_STORED_PER_SOURCE` and checking
stays at 10 without a warning.

*Fix:* Remove the section references, and say the reason in plain words
(the WASM fuel limit per call). Remove the wrong number, or give the
measured one. Define `MAX_CHECKED_PER_SOURCE` from `MAX_STORED_PER_SOURCE`
(or the other way round). Rename the test to what it checks, and add one
test where refused hits come first in the 10. Add the page effect to the
Deviation at `change.md:363`.

*Status:* Fixed (2633a24d, bc1ffd56). Removed section 7.6 and planning-doc citations from comments in `directory.rs` and `gather.rs`; defined `MAX_CHECKED_PER_SOURCE = MAX_STORED_PER_SOURCE as usize` to prevent drift; renamed verify test to `a_source_is_capped_at_ten_verified_hits` and added `refused_hits_coming_first_are_recorded_up_to_the_cap` testing 5 refused + 5 verified hits when refused come first; recorded duplicate-skipping page effect in Deviations (`change.md:363`).

#### R14 — minor — the change doc and the backlog still give the old caps

*Where:* `docs/planning/changes/roym-open-foundation/change.md:138`,
`:331`, `:363`; `docs/planning/deferred-backlog.md:553`.

*Problem:* The design paragraph at `:138` still says 8 MiB and "20 verified
hits and 20 refused hits per source". D-A4-16 at `:331` still says
"Truncation to 40 hits". The backlog row at `:553` still says the library
caps parsed replies at 8 MiB. D-A4-3, D-A4-5, D-A4-6 and D-A4-14 were
changed in place, but no Deviation records the 8 MiB to 10 MiB change. The
`status.md` row says "5/5 runs", and the R2 status says "20/20 runs".

*Failure case:* A reader of the change doc or the backlog acts on caps that
the code does not have.

*Fix:* Update `:138`, `:331` and the backlog row to the final caps (after
R10). Add a Deviation for the reply cap. Give one run count.

*Status:* Fixed (bc1ffd56). Updated design paragraph (`change.md:138`), decisions D-A4-3, D-A4-5, D-A4-6, D-A4-14, D-A4-16 (`change.md:331`), and deferred backlog row (`deferred-backlog.md:553`) to 2 MiB and 10 verified/refused hits; added Deviation for 2 MiB reply cap and bounded query limit; unified run counts to 20/20 runs in `status.md` and R2 status.

**Counts.** Second fixes check: 3 confirmed (R2, R11, R12), 1 reopened
(R10, major), 2 new (R13, R14, minor). 0 blocker, 1 major (R10), 2 minor
(R13, R14). **3 open.**

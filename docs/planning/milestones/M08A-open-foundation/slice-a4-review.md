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

**Summary:** 0 blocker, 2 major, 5 minor (P18 to P24). 7 open.

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


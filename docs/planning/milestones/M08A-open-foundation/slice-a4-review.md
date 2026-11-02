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

**Summary:** 0 blocker, 1 major, 13 minor. After the fixes check (next
section): 12 confirmed Fixed, P1 and P3 reopened, P15 and P16 added (minor).
4 open.

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

*Status:* Open (reopened in the fixes check below; was Fixed 6b47137b, partial). Added MAX_SEARCH_REPLY_BYTES length check before parse, replies dropped by value via into_iter in D-A4-3/D-A4-5, unit and ceiling tests in 7.1/7.2, residual host lowering risk in 7.6, and backlog row in Section 8.

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

*Status:* Open (reopened in the fixes check below; was Fixed 6b47137b, partial). Outer refusals map to NotStarted with error text in Gathered.refusal; directory logs refusal at warn in D-A4-8; Hub shows neutral start message in D-A4-10/D-A4-12; test a_refused_batch_marks_every_source_not_started covers Internal and UnsupportedTarget in 7.1.

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

*Status:* Open

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

*Status:* Open

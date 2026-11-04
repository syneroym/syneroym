# M08A slice A5 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a5-implementation-plan.md](slice-a5-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-03

Checked against the code on `feat/m08a-a5-event-feed` at `d5c35e40` (main
checkout, not a worktree). `git pull` said "Already up to date" (it needed
network access to GitHub; the first try in the sandbox failed and the second,
with `github.com` allowed, worked). Line numbers below are at that commit.

**Who reviewed.** The prompt for this round named the reviewer as Claude Opus
5.5 and the plan author as Claude Sonnet 5.5. This session runs on Claude
Sonnet 5.5. The `status.md` row for A5 says the plan was written by Claude
Opus 5.5. So the prompt and `status.md` disagree about who wrote the plan, and
this reviewer is not the model the prompt named. I cannot tell whether there is
model independence in this round. The title above and the `status.md` row name
the model that did the work.

**Change docs.** `task.md` names one change doc for A5:
[roym-open-foundation](../../changes/roym-open-foundation/change.md). That is
the only doc in the list. No other change doc is named. The criteria OF-8 to
OF-11, D-OF-3 and D-OF-4 are in it.

**Summary:** 0 blocker, 3 major, 9 minor, 12 items. All 12 Fixed in the fold of round 1 (2026-11-03).

Limits of this review: I read code and did not run it. I did not run a browser.
P1 and P3 rest on how `EventSource` and browsers behave (the HTML living
standard and the usual per-host connection limit). I checked neither in a real
browser here, so the exact numbers are `unverified`.

### What was checked and found correct

- **Where the state lives (3.1).** `apply_admission_conn` assigns the number
  only for `Accept` and does nothing for an `accepted` row
  ([store/message.rs:297-307](../../../../crates/conversation/src/store/message.rs#L297)).
  `notify_state` is private in `ConversationService`
  ([lib.rs:161](../../../../crates/conversation/src/lib.rs#L161)) and its
  callers are the ones listed (lib.rs:328, outbox.rs:306, :362, :365, :371,
  :415, :427, outbox/parked.rs:139). The six admission call sites are right
  (transport.rs:537, :545, lib.rs:191, :194, outbox.rs:483, :487), and
  outbox.rs:543 is the drop. `restart_pending` is a state change that does not
  call `notify_state`.
- **Card and booking writers (3.1).** `put_row(host, CARDS, ..)` appears 12
  times: 11 outside import (app.rs:485, sync.rs:359, :380, :422, :487, :578,
  receipts.rs:126, :209, :233, :310, :423) and backup.rs:660. `BOOKINGS` has 5
  writes outside import (booking_ops.rs:146, :168, :263, ledger.rs:278,
  :333) and sections.rs:444. The "eleven" and "five" are right.
- **The loop bound (B4, D-A5-12).** Every incoming card write in `sync.rs` and
  `receipts.rs` sits behind the `exists` check
  ([sync.rs:217-222](../../../../crates/roym_transaction/src/app/sync.rs#L217)),
  and the booking writes happen on a state change, not on every read. So a
  second sync files nothing and records nothing. The chain ends.
- **App instance, owner, session (3.2).** `app_context_of`
  ([local_registry.rs:522](../../../../crates/core/src/local_registry.rs#L522))
  and `owner_of` (:365) are as described. `resolve_effective_session_caller`
  ([auth.rs:78-125](../../../../crates/router/src/route_handler/http/auth.rs#L78))
  works only for gateway-origin traffic (caller DID equals the node DID and no
  delegation), checks revocation, and sets `SESSION_CALLER_MARKER`. A peer over
  Iroh cannot get the marker. The session cookie is `Path=/; HttpOnly;
  SameSite=Lax`
  ([auth/service.rs:563](../../../../crates/auth/src/service.rs#L563)), so a
  cross-site page cannot make the browser send it with an `EventSource`, and
  with no CORS header it cannot read the stream either.
- **Router and gateway (3.3).** The gateway answers 404 for `/_syneroym/`
  paths except `/_syneroym/session*`
  ([gateway.rs:411-413](../../../../crates/client_gateway/src/gateway.rs#L411)).
  The blob check is before assets and routes in `try_handle_http_request`
  ([http.rs:309-314](../../../../crates/router/src/route_handler/http.rs#L309)),
  so the new check goes in the right place. `handle_messaging_sse` is as
  described ([streaming.rs:9-90](../../../../crates/router/src/route_handler/http/streaming.rs#L9)).
  The Hub CSP has `connect-src 'self'`, which allows `EventSource` to the same
  origin ([index.html:14](../../../../crates/roym_web/ui/index.html#L14)).
- **Owner rule.** `web`'s `admit` uses the signing identity's owner and
  `CallerAuth::Delegated`
  ([app.rs:107-125](../../../../crates/roym_web/src/app.rs#L107)). The plan's
  `owner_of` check applies the same rule.
- **Host side.** `HostState` has `component_id`, `read_only` and
  `app_instance_id`
  ([host_capabilities.rs:344, :375, :384](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L344)).
  The only non-test `HostState::new` is
  [engine.rs:546](../../../../crates/sandbox_wasm/src/engine.rs#L546), so the
  engine setter reaches every WASM store. `RouteHandlerDeps` is built in one
  production place ([runtime/router.rs:464](../../../../crates/substrate/src/runtime/router.rs#L464))
  and in 8 test files. Making the new field `Option` and setting it to `None`
  in those test files is a one-line change each; the plan should say so (see
  P6).
- **Files, sizes, ratchet.** `helpers.rs` is 1 429 lines and is listed at 1 429
  in `xtask/oversized-test-files.txt:22`. The line counts in 7.11 match today's
  files (712, 758, 493, 617, 646, 625, 673; `messages.ts` 887). `roym-hub.spec.ts`
  is 1 245 lines and case 40 is at :1214. The two reopen loops of the demo are
  where the plan says. ADR-0027 is the next free number (the last ADR is
  0026). `RoymNode::restart` and `serial_guard` exist
  ([common/roym.rs:579](../../../../crates/substrate/tests/common/roym.rs#L579),
  [common/mod.rs:105](../../../../crates/substrate/tests/common/mod.rs#L105)).
- **Change-doc routing.** OF-8 to OF-11 and D-OF-3, D-OF-4 are all in the one
  doc the plan names. The plan's Test-column and Decisions plan (section 9)
  matches how A2c to A4 recorded theirs. One gap is in P12.

### Views on the owner's questions

- **Q1. (a), the host conversation crate records both kinds.** I agree. The
  race in (b) is real (the guest answers before the host applies the answer).
  (a) is only safe if the event is written after the commit, and three of the
  paths the plan lists run inside a queue transaction (P2). With that fixed,
  (a) is right. P8 lists three visible changes that no kind covers.
- **Q2. (a), SSE.** I agree for the protocol. Two browser facts change the Hub
  work, not the choice: `EventSource` never retries after a non-200 answer
  (P1), and browsers limit HTTP/1.1 connections per host across all tabs (P3).
  Both must be in the plan before the Hub step is written.
- **Q3. (a), pull.** I agree. The change doc's OF-10 and `task.md` say
  "subscribe" and "internal subscription". The plan changes the meaning to a
  pull read. Section 9 does not list this as a deviation (P12).
- **Q4. (a), `{cursor, kind, service, subject, item, at_ms}`.** I agree.
  `subject` is the thing a client must load again, and no content is carried.
- **Q5. (a), one feed per app instance, owner only.** I agree. It reuses the
  rule `web` already applies. Note for the docs: the feed works only when the
  gateway runs in `login` identity mode. In `open` (the default) and `fixed`
  mode there is no session, so the answer is 401. `web` has the same limit
  today (P9).
- **Q6. (a), Messages and the open group.** I agree. The refill functions drop
  late results already.
- **Q7. (a), two tabs.** I agree. Alice's and Bob's Hubs are different
  hostnames, so their cookies and connection limits are separate. One caution:
  a tab in the background can delay a timer (the 150 ms debounce) by up to
  about a second in Chromium. The waits use `DELIVERY_WAIT_MS`, so this is fine;
  do not use a 1 s expect timeout in the story.
- **Q8. (a), ADR-0027.** I agree. A new host capability with a new gateway
  path and a new WIT package needs an ADR. The number is free.

### Findings

#### P1 (major) — The Hub feed dies for good after any non-200 answer

- **Where:** plan D-A5-12 ("`startFeed()` opens `new EventSource(..)` once ...
  `EventSource` sends `Last-Event-ID` on its own when it reconnects"); D-A5-8
  steps 1-6 (404, 401, 403, 406, 503); 7.6 `a_cursor_from_before_a_restart_gets_reload`.
- **Problem:** `EventSource` reconnects by itself only after a network error
  or after the server closes a 200 stream. For any other HTTP status (or a
  wrong content type) it sets `readyState` to `CLOSED` and never tries again.
  The plan serves five non-200 answers and never says what the Hub does about
  them. The plan's restart story ("a restarted node answers `reload`") also
  holds only for a raw HTTP client.
- **Failure case:** (1) Alice restarts her node while a Hub tab is open. The
  gateway comes up before the app instance is registered, so the browser's
  reconnect gets `404` (D-A5-8 step 2) or `502` from the gateway. The feed is
  closed. After the node is fully up the tab never learns of a message, and
  nothing on screen says so. (2) The session expires, the feed gets `401`, and
  the tab is stuck even after the person logs in again, because `startFeed` runs
  "once after login". (3) A tab opened while 16 feeds are open gets `503` and
  never retries.
  `a_cursor_from_before_a_restart_gets_reload` reads with a plain HTTP client,
  so it cannot catch any of this.
- **Fix:** In `events.ts`, wrap `EventSource`. On `error` with
  `readyState === EventSource.CLOSED`, close it and create a new one after a
  backoff (for example 1 s, doubling to 30 s, with jitter), passing the last
  cursor it saw in the `after` query parameter (the new object cannot send
  `Last-Event-ID`; D-A5-8 step 7 already accepts `after`). Call `startFeed`
  again after a new login, and stop it on logout. Add vitest cases with a fake
  `EventSource` that closes: `a_closed_feed_is_opened_again_with_the_last_cursor`
  and `a_login_after_a_401_restarts_the_feed`. Add one Hub e2e step: restart is
  too slow for Hub e2e, so use a route that fails once (Playwright
  `page.route` returns 503 once), and check the next event still arrives. State
  in the plan that the Hub never shows an error for a feed that is down; it
  works as it does today (refresh on action).
- *Status:* Fixed (5a220d04).

#### P2 (major) — Three of the listed writers run inside a transaction, so the event would come before the commit

- **Where:** plan D-A5-11 ("The send and delete paths call `announce` after the
  store returns `Ok`"), 3.1 delete list ([transport.rs:467](../../../../crates/conversation/src/transport.rs#L467),
  [group/entry.rs:218](../../../../crates/conversation/src/group/entry.rs#L218)),
  send list ([group.rs:401](../../../../crates/conversation/src/group.rs#L401),
  [store/message.rs:92](../../../../crates/conversation/src/store/message.rs#L92)).
- **Problem:** The plan's argument for Q1 (a) is "the event is written where
  the state is, after it is stored, so there is no race". That is true only
  after the SQLite commit. These paths are not like `delete_message`, which
  commits and then returns (`message.rs:363-376`):
  - A peer's deletion request is applied by `handle_inbound_deletion_request(tx, ..)`
    inside `store.queue().transaction(|tx, _txq| ..)` in `ingest_verified`
    ([transport.rs:405-475](../../../../crates/conversation/src/transport.rs#L405)).
    The function has no way to hand a message id out.
  - A group deletion entry goes through `apply_group_deletion_request(tx, ..)`
    ([group/entry.rs:216](../../../../crates/conversation/src/group/entry.rs#L216)),
    which returns `Result<()>`, inside the group-entry transaction.
  - `record_outgoing_group_message(tx, ..)` (group.rs:401) and
    `insert_outgoing_and_enqueue` (message.rs:92) take a transaction.
  An `announce` call placed inside any of these runs before `COMMIT`. A client
  that refreshes at once, on its own SQLite connection, still reads the old
  row. The plan's own race test only covers admission.
- **Failure case:** Bob deletes a message. Alice's node applies the deletion
  request in a transaction and records `conversation.message` inside it. The Hub
  refreshes in a few milliseconds, reads history before the commit, and still
  shows the deleted message. No second event comes. This is the lost update that
  OF-11 and Q1 are meant to avoid.
- **Fix:** Add a rule to D-A5-11: `announce` is called only after the
  transaction that stored the change has returned `Ok`. Name the changes the
  signatures need: `handle_inbound_deletion_request` and
  `apply_group_deletion_request` return the affected `(conversation_id,
  message_id)` when a visible row changed (`Option<..>`); the `ingest_verified`
  closure moves it out (as `inserted` and `group_id_to_apply` already are) and
  `announce` runs after `transaction` returns. Do the same for the group
  deletion path in `group_sync.rs`, and for sends: call `announce` in
  `enqueue_direct` and `send_group` after the store call returns. Say what a
  "visible row changed" means for delete: `delete_message_conn` returns
  `Ok(true)` also for an outgoing `pending` or `deposited` row and for an
  invisible `undecided` or `held` row (message.rs:411-430). Announce when the
  row was `accepted` before the delete, or was outgoing (outgoing rows are
  visible); do not announce for an inbound row that was `undecided`, `held` or
  `dropped`. Add a test `a_deletion_request_is_visible_to_history_when_its_event_arrives`
  (the same shape as the admission test) and `an_undecided_message_deleted_by_its_author_records_nothing`.
- *Status:* Fixed (5a220d04).

#### P3 (major) — One `EventSource` per tab runs into the browser's per-host connection limit

- **Where:** plan D-A5-12 ("one feed per tab"), 7.8 (not tested: "Many real
  browsers on one node"), D-A5-8 cap of 16.
- **Problem:** The gateway speaks HTTP/1.1 only (a raw TCP proxy,
  `copy_bidirectional`). Browsers allow about 6 connections per host over
  HTTP/1.1, and the limit is shared by all tabs and windows of that host. Each
  open feed holds one connection for the life of the tab. The cap of 16 per app
  instance in the router is above that, so it will never be reached before the
  browser blocks first. `unverified`: the exact number depends on the browser;
  6 is the usual value in Chromium and Firefox.
- **Failure case:** A person opens the Hub in 6 tabs of the same hostname. Six
  connections are all held by feeds. The 7th tab cannot load its own page,
  script or `/rpc` calls, because every request waits for a free connection.
  All tabs seem to hang, and the cause is not visible.
- **Fix:** One feed per browser, not per tab. In `events.ts`, elect one tab
  with the Web Locks API (`navigator.locks.request("roym-event-feed", ..)`),
  which opens the `EventSource` and relays each change to the other tabs with a
  `BroadcastChannel`; when that tab closes, the lock passes to another tab,
  which reconnects with the last cursor it heard. Fall back to one feed per tab
  when `navigator.locks` or `BroadcastChannel` is missing. Add vitest cases for
  it (`only_one_tab_opens_the_feed`, `a_tab_that_takes_over_resumes_from_the_last_cursor`),
  and list in 7.8 that the 6-connection case is not run in a real browser, or
  add a Playwright case with 8 pages that all show a new message while one
  stays the leader. If the owner prefers to keep it simple for v1, then the
  plan must say so, write the limit into the backlog and into the developer
  guide note, and lower the plan's "one feed per tab" claim.
- *Status:* Fixed (5a220d04). Fixed in a simpler way with the same bound: a tab holds a feed only while it is visible (D-A5-12), not a leader tab. The leader-tab option and the remaining limit (6 visible windows) are a backlog row, and section 9 adds the limit to the developer-guide note (Fixed (fb06a6be)).

#### P4 (minor) — "The newest event's cursor, or seq 0" is wrong once old events have left the window

- **Where:** D-A5-2 (`head`: "the newest event's cursor, or seq 0"), D-A5-3
  (reload rule `seq + 1 < oldest kept seq`).
- **Problem:** `next_seq` never goes back, but the window can be empty after
  the 24-hour rule drops everything. Then "the newest event" does not exist and
  `head` is `seq 0`, while the next event is `next_seq` (for example 51). Also
  "oldest kept seq" is undefined for an empty window.
- **Failure case:** A feed has cursor `…-50`. The app is quiet for 25 hours and
  a read drops all events. A new client reads `ready` and gets `…-0`. The next
  record is seq 51. The client's next read has `seq + 1 < oldest` (1 < 51), so
  it gets `reload` for an event it should have received, and then loops on the
  same mistake after each quiet day. The old client with `…-50` against an empty
  window is "equal to head" under one reading and "ahead of head" under the
  other.
- **Fix:** Define `head` as `next_seq - 1` (0 only if nothing was ever
  recorded in this run) and `oldest` as the front event's seq, or `next_seq`
  when the window is empty. Add `after_all_events_age_out_a_new_reader_does_not_get_a_false_reload`
  and `after_all_events_age_out_the_old_head_cursor_is_not_a_reload` to 7.1.
- *Status:* Fixed (5a220d04).

#### P5 (minor) — `record` is a WIT keyword; the function needs `%record`

- **Where:** D-A5-6 (`record: func(kind: string, ..)`, in the same interface
  as `record event { .. }`).
- **Problem:** In WIT, `record` is a reserved word (like `interface`, which the
  plan's own `call-request` writes as `%interface`). A function called `record`
  does not parse. The plan promises "the full text" of the WIT (the A3 P7
  lesson), so the text should compile.
- **Failure case:** Step 3 stops at the first `cargo build`, with a WIT parse
  error. The implementer must guess a new name, and the names in the plan (the
  `events.record` call, `record_event`, the metrics) may then differ between
  the WIT, the shim and the docs.
- **Fix:** Write `%record: func(..)` in D-A5-6 (the Rust name is still
  `record`), or rename the function to `emit` and change the plan everywhere
  ("`syneroym:events` `record`" appears in W3, D-A5-6, D-A5-7, section 1). Also
  run `wasm-tools component wit` (or the build) over the text before the
  implementer starts, if the author wants to be sure about the rest of it.
- *Status:* Fixed (5a220d04). Used `%record`.

#### P6 (minor) — There are seven native factory sites, not six, and the existing count test says seven

- **Where:** plan 3.4 ("Production sites: `roym.rs` :157, :228, :273, :319,
  :361, :403"), Step 4 item 3 ("A unit test counts six"), 7.10 row for A3 P2.
- **Problem:** `with_call_all` is also called at
  [dual_build_fixture.rs:68](../../../../crates/substrate/src/runtime/dual_build_fixture.rs#L68).
  The existing test `every_production_factory_in_runtime_is_wired_with_call_all`
  ([runtime/tests.rs:87-117](../../../../crates/substrate/src/runtime/tests.rs#L87))
  asserts exactly 7 `NativeHostFactory::new(` sites and that
  `with_call_all` appears the same number of times. A new test that counts six
  is wrong or duplicates it. Also, `app_host_native/tests/dual_build_parity/helpers.rs:648`
  and `:731` build their own engine and factory, and the plan names only the
  `roym_web` parity harness.
- **Failure case:** The implementer adds `with_event_bus` at six sites and a
  test that says six. The seventh factory (the dual-build fixture, which the
  e2e fixtures deploy) has no bus. It works only by accident, because the
  fixture never records, and the new test fails or must be forced to pass.
- **Fix:** Say seven sites, include `dual_build_fixture.rs:68`, and extend the
  existing test with a third count (`.with_event_bus(`), equal to the factory
  count, instead of a new test. Say that the `app_host_native` parity helpers
  also get a bus (or say why they stay without one, and what answers
  `no-app-instance`). Say that the 8 test files that build `RouteHandlerDeps`
  get `event_bus: None`.
- *Status:* Fixed (5a220d04). The `app_host_native` parity harness gets no bus (its fixture records nothing; the file is at its cap).

#### P7 (minor) — A change between "page loaded" and "feed connected" is lost

- **Where:** D-A5-8 (first frame: "with no start cursor, `event: ready`, `id:
  <head>`"), D-A5-12 (`startFeed()` once after login, from `renderTabs`).
- **Problem:** A new feed starts at the head. The Messages screen is drawn
  before the `EventSource` finishes its connect, and a reconnect after a
  `CLOSED` feed (P1) also starts at the head when it has no cursor. Anything
  recorded in the gap is never sent, and no later event repairs it.
- **Failure case:** Alice's Hub loads. Before the stream is open, Bob's message
  arrives. The screen shows the old list. It stays old until another event or
  Alice acts. In the demo this is a rare flake (Bob replies seconds later);
  with a slow gateway it is easy to hit.
- **Fix:** Tell listeners to refresh once on every `ready` frame, as on
  `reload` (the plan says `{reload: true}` only for `reload`). Add vitest
  `a_ready_frame_refreshes_every_listener_once`. Say in D-A5-8 that `ready` is
  the "you may have missed something before this point" frame.
- *Status:* Fixed (5a220d04).

#### P8 (minor) — Some visible changes have no event, and one sequence-number site is not listed

- **Where:** D-A5-11 (the two conversation kinds), 3.1 lists.
- **Problem:** Three things are not covered and not named as out of scope:
  1. A peer's refusal notice sets `refused` on one of Alice's outgoing messages
     ([message.rs:487](../../../../crates/conversation/src/store/message.rs#L487),
     [:555](../../../../crates/conversation/src/store/message.rs#L555)). The Hub
     shows it (`messages.ts:505`). It is not a delivery state and not a delete,
     so nothing records it.
  2. A group send has per-member states (`message_recipients`,
     [store/deposit.rs:28-37](../../../../crates/conversation/src/store/deposit.rs#L28),
     `set_recipient_state`). Only the message's own state goes through
     `notify_state`; a change of one member's state records nothing.
  3. `insert_outgoing_without_enqueue` gives a row a sequence number
     ([message.rs:136-148](../../../../crates/conversation/src/store/message.rs#L136)).
     3.1 does not list it. I found no non-test caller, so it may be dead or used
     by a path the plan has not named.
- **Failure case:** Bob refuses Alice's message. Alice's open screen keeps the
  old state until she opens it again.
- **Fix:** For (1) record `conversation.message` (subject the conversation,
  item the message id) after the transaction that sets `refused`; or add a
  backlog row and a sentence in 3.1. For (2) decide: record
  `conversation.delivery` when a member's state changes, or add a backlog row
  ("group per-member delivery states record no event"). For (3) say who calls
  it; if nobody, say "unused" in 3.1 and leave it out, if somebody does, add it.
- *Status:* Fixed (5a220d04). (1) refusal records `conversation.delivery`; (2) backlog row; (3) test-only caller, noted in 3.1.

#### P9 (minor) — The order of checks tells a stranger which services are in an app, and the plan does not say the feed needs `login` mode

- **Where:** D-A5-8 steps 2-4, W1, Q5.
- **Problem:** Step 2 answers `404` before any login check. A caller without a
  session can tell a service that is in an app from one that is not. It is a
  small leak, but the plan's own rule is "401 first" for the rest. Also, in
  `open` (the default) and `fixed` identity modes the router never sees a
  session caller, so every request is `401`
  ([gateway.rs:443-452](../../../../crates/client_gateway/src/gateway.rs#L443),
  [auth.rs:84-89](../../../../crates/router/src/route_handler/http/auth.rs#L84)).
  The Hub needs `login` mode anyway, but a developer who reads "any client
  with the owner's session" in the developer guide will not know it.
- **Failure case:** Someone probing a node learns which service ids are in an
  app. A developer who runs a non-Roym SynApp in `fixed` mode gets `401` and
  thinks the feature is broken.
- **Fix:** Order the checks: no bus `404`, no session `401`, not owner `403`
  (using `owner_of`, which is `None` for an unowned service), then no app
  instance `404`, then `406`. Add the mode rule to the developer-guide part in
  section 9 and to W1 ("needs the gateway in `login` mode"). Add a router test
  for the new order (`a_stranger_cannot_tell_an_app_service_from_a_plain_one`).
- *Status:* Fixed (5a220d04).

#### P10 (minor) — The feed's read size, `limit = 0` and the open-feed gauge are not defined

- **Where:** D-A5-8 (the wait loop), D-A5-6 (`read`), metrics list.
- **Problem:** (1) The router "reads after its last cursor and sends the
  frames", but `read` is capped by a `limit`. The plan does not say which limit
  the router passes or that it loops until it gets no more events. A client
  that reconnects 9 000 events behind would get 500 and then wait for the next
  change to get more. (2) `read(after, 0)` has no defined answer. (3)
  `substrate.events.feeds_open` is a gauge that must go down when the client
  goes away. The permit is dropped with the stream state, but a gauge needs
  code that runs on drop, and the plan has none.
- **Failure case:** A reconnecting Hub is 700 events behind. It receives 500, no
  event for a quiet hour, then catches up on the next change. The gauge counts
  every feed ever opened.
- **Fix:** In D-A5-8, loop `read` with `READ_LIMIT_CAP` until a read returns
  fewer than the cap, and send all frames before waiting. Define `limit = 0` as
  "no events, current cursor" (or `invalid-argument`) in D-A5-6 and test it. Put
  the permit, the `watch` receiver and a small `FeedGuard` with a `Drop` that
  decrements the gauge in the stream state. Add a router test
  `a_reader_far_behind_gets_all_events_before_it_waits`.
- *Status:* Fixed (5a220d04).

#### P11 (minor) — One conversation test cannot be written as described

- **Where:** 7.3 `an_accepted_inbound_message_records_one_event_after_it_is_visible`
  ("the test reads `changes` inside the bus `watch` callback and finds the
  row").
- **Problem:** A `tokio::sync::watch` receiver has no callback. The test as
  described does not exist, and the way to write it decides whether it can
  fail. A test that reads `changes` after `record` returned can pass even when
  `record` runs before the commit, because the store read comes later.
- **Failure case:** The test passes with the bug the test is for (event before
  commit), so it protects nothing.
- **Fix:** Say how: spawn a task that waits on `bus.changed(..)`, and, as soon
  as it wakes, reads `changes` on a **second** connection (the store's
  `conn()` is a mutex, so use a separate read, or have `ConversationStore`
  expose a read that does not wait on the writer) and asserts the row is there.
  Name the mutation the test must catch: move `announce` before `commit`. Do
  the same for the delete test of P2. If the store's single connection makes a
  true "second reader" impossible, say that the test checks order with a hook
  that records "committed" and "announced" in a shared list.
- *Status:* Fixed (5a220d04). Used a `#[cfg(test)]` probe in `announce` that checks the connection lock is free (`try_lock`) on the same thread, then reads history.

#### P12 (minor) — The change doc's deviations list leaves out Q3 and Q5, and OF-10 still says "subscribe"

- **Where:** plan section 9 ("Deviations, Different from the plan: ... Q1, Q2,
  Q4"), change doc OF-10 and D-OF-3 ("A5 decides how services subscribe"),
  `task.md` A5 bullet ("internal subscription for services").
- **Problem:** Q3 (a) replaces an internal subscription with a pull read
  (`events.read`), and Q5 (a) narrows "the events of the apps its login session
  may use" to "the one app of the hostname, for its owner". Both are changes of
  the wording of OF-10. The plan lists only Q1, Q2 and Q4 as deviations and
  does not say to reword OF-10 or the `task.md` bullet. Section 9 also never
  says to record that W1 needs `login` mode (P9).
- **Failure case:** At close-out the change doc says OF-10 is met with "an
  internal subscription", and a reader looks for a subscription that does not
  exist.
- **Fix:** In section 9 add: (a) a Deviation for Q3 ("services read the feed
  with `events.read`; there is no push export", linking the backlog row) and for
  Q5; (b) reword OF-10 in the criteria table to "a service on the node can read
  the events of its own app instance with `events.read`, without WebSocket";
  (c) change the `task.md` A5 bullet "internal subscription for services" to
  "pull read for services"; (d) add the Q3 and Q5 answers to the one-line
  record of the owner's answers.
- *Status:* Fixed (5a220d04). OF-10 reword and the task.md bullet are close-out steps in section 9; the D-OF-3 line was updated now.

## Plan review round 2 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-03

Checked the fold of round 1 (`5a220d04`, closed in `6deac94c`) against the
plan diff and the code on `feat/m08a-a5-event-feed`. I read code and did not
run it.

**Summary:** 0 blocker, 0 major, 1 minor, 1 item. Fixed (cab51d85).

**Found correct (round 1 fixes, P1 to P12).** Each fix is in the plan and
matches the code that I read:

- P1, P3: the reopen loop with backoff and `?after=<cursor>` (D-A5-8 step 7
  accepts `after`); a feed only while the tab is visible. Login and logout
  reload the page (`main.ts:102`, :225, :251), so no logout hook is needed.
- P2: the new return types and the "announce after `transaction` returns" rule.
  The queue transaction and the store share one `std::sync::Mutex`
  (`async_queue/queue.rs:287`, `conversation/store/schema.rs:32`), as the plan
  says.
- P4 to P12: the head and oldest rule for an empty window, `%record`, the seven
  factory sites, the `ready` refresh, the refusal event, the check order, the
  read loop with `limit = 0`, `FeedGuard`, and the change-doc and OF-10 close-out.

### Findings

#### P13 (minor) — The commit-rule probe can fail by chance when store work runs on another thread

- **Where:** plan D-A5-11, "How the tests check the rule" (7.3): `conn_is_free()`
  is `self.conn.try_lock().is_ok()`, and the text says "on one thread no other
  task can hold it while `announce` runs, and the probe never fails by chance".
- **Problem:** The store lock is a `std::sync::Mutex`. The current-thread test
  runtime does not stop other OS threads from taking it. The conversation crate
  runs store work on the blocking pool with `spawn_blocking` (`lib.rs:212`,
  `host_impl.rs:392`, :415, :428, :441). A job like that holds the lock on its
  own thread.
- **Failure case:** A test that has `search`, `export_history` or similar store
  work still running (not yet awaited) when `announce` runs gets
  `try_lock() == Err`. The probe reports "a transaction is open" for correct
  code, and the test is flaky. The "no timing is involved" claim is then false.
- **Fix:** In D-A5-11 say one of: (a) the probe tests await all store work
  before the call that announces, and the text names this as a rule of the
  tests; or (b) `conn_is_free()` retries `try_lock` for a short fixed time (for
  example 50 ms) before it reports "not free". Option (a) keeps the check
  exact. Correct the "never fails by chance" sentence to match.
- *Status:* Fixed (cab51d85). Option (a): probe tests await each service call before the next and never spawn or join calls; the plan names the `spawn_blocking` sites, drops the "never fails by chance" claim, and says not to retry `try_lock`.

## Code review round 1 — Claude Opus 5.5, 2026-11-04

Checked `git diff origin/main...HEAD` at `4cfd96a0` on
`feat/m08a-a5-event-feed` (main checkout, not a worktree; `git pull` said
"Already up to date"). PR [#341](https://github.com/syneroym/syneroym/pull/341)
is still a draft. The code was written by Gemini 3.8 Flash. I read
`AGENTS.md`, session-strategy.md ("The review file"), the plan, the plan
review rounds above, the code review rounds of A2a, A2b, A2c, A3 and A4, and
the change doc. No code was changed. Line numbers below are at `4cfd96a0`.

CI: `gh pr checks 341` on `4cfd96a0` is **red**. Test 1/6 failed before it ran
any test, because the runner ran out of disk (R3). Every other job passed. I
did not run the full suite. I ran `npx vitest run` in `crates/roym_web/ui`
(155 passed) and `grep`, `wc` and `git` checks. Each finding below follows
from the code as written.

**Summary:** 0 blocker, 3 major (R1, R2, R3), 7 minor (R4 to R10). 10 open.

### What was checked and found correct

- **The bus (D-A5-1 to D-A5-5).** `EventBus` is in `syneroym_core::event_bus`
  with a `DashMap` of `Arc<Window>`, a `Mutex<VecDeque>`, `next_seq` from 1, a
  `watch` sender and a 16-permit semaphore per window. `record` tags the event
  with the app instance and service name from `app_context_of`, never from the
  event. `head` and `oldest` use `next_seq`, so a cursor at the head stays
  valid after every event ages out. The reload rules match D-A5-3 (other run,
  ahead of head, `seq + 1 < oldest`). `limit = 0` returns the given cursor.
  The field caps are 64 / 128 / 128, as the plan says (but see R4 for the
  docs). The 18 tests have the plan's names, and the window, age-out and
  tagging tests fail for the mutations the plan names (I checked
  `>=` against the window test by hand: the read at seq 5 becomes a reload).
- **The host capability (D-A5-6, D-A5-7, D-A5-10).** `events.wit` is the
  plan's text. `HostState::record_event` checks `read_only`, then the app
  instance, then the bus, and passes `component_id` to the bus. `read_events`
  uses `app_instance_id`. One copy of the logic serves the WIT impl and
  `NativeAppHost`. The engine has a `OnceLock` and `set_event_bus`; `init`
  keeps its signature. The linker adds `events` after `invocation`. All six
  Roym worlds and the dual-build fixture import the package through a
  symlink. The `roym_core` test host returns a fixed cursor.
- **Wiring (Step 4).** One bus per node in `build_route_handler_deps`, set on
  the engine, the conversation service and `RouteHandlerDeps`. Seven
  `.with_event_bus(` calls beside seven `.with_call_all(`, and the renamed
  count test checks both. Every other `RouteHandlerDeps` literal got
  `event_bus: None`. The parity harness moved the two test proxies to
  `test_proxies.rs` and the oversized entry went from 1 429 to 1 290.
- **The commit rule (D-A5-11).** Every `announce` call is outside the
  `transaction` closure: `ingest_verified` hands out `deleted_target` and
  `refused_target`; `validate_and_insert` and `apply_pending_entries` hand out
  the deleted id; `send_group` and `enqueue_direct` announce after the
  transaction returns. `apply_admission` returns `true` only for the call that
  gives the row its number, so a racing re-ask records nothing.
  `delete_message_conn` returns `Some(true)` only for an outgoing or an
  accepted row. `notify_state` announces for all eight callers, and `retry`
  announces after `restart_pending`.
- **Transaction writers.** `put_card` and `put_booking` replace the eleven
  card writes and the five booking writes. I checked that the keys did not
  change: `CardRow.message_id` is the old `msg_id` at every site, and
  `BookingRow.agreement` is the old key (`agreement.quote_record_id`, `q`)
  at every site. Import keeps `put_row`.
- **Access control (Q5, D-A5-8).** The router answers the path before the
  blob check, only for `GET` and the exact path. The check order is the
  plan's: no bus 404, no session marker 401, `owner_of != caller` 403, no app
  instance 404, no `Accept` 406, no permit 503 with `Retry-After: 1`. The
  session marker is set only for gateway-origin traffic with a verified,
  unrevoked token. A peer over Iroh gets 401. The window is chosen by the app
  instance of the hostname, not by the cursor, so a foreign cursor is a
  reload or a gap. `changed()` is subscribed before `head()` is read, so no
  wake-up is lost between them. The read loop continues while a read returns
  500 events.
- **The gateway (D-A5-9).** `is_forwarded_syneroym_path` forwards only
  `/_syneroym/session*` and the exact events path (query ignored). Its test
  covers the five plan cases.
- **The Hub.** One `EventSource` per visible tab; closed when hidden;
  reopened with `?after=<last cursor>`; reopen after `CLOSED` with 1 s
  doubling to 30 s plus up to 20 % jitter; backoff reset on a frame; 150 ms
  debounce per subject; `ready` and `reload` refresh every listener. Messages
  and Groups remove their listener when their container is gone. 155 vitest
  cases pass (matches `status.md`).
- **Demo and Hub e2e.** The chat story uses two browser sessions, has no
  reopen loop, and waits with `DELIVERY_WAIT_MS`. Case 41 exists. The
  `demo:` list adds the story and keeps the earlier `none:` line.
- **Sizes and ratchets.** No Rust file passes 800 lines (largest touched:
  `signing.rs` 778). `oversized-test-files.txt` lowers three entries.
  `MAX_TOO_MANY_LINES_SUPPRESSIONS` goes from 39 to 38. Lints passed in CI.
- **Backlog.** The seven new rows of plan section 8 are there, and the
  restart row is updated (but see R4 for one wrong number).

### Findings

#### R1 — major — the Hub now writes the session token into a cookie on its own host, which every request carries

*Where:* `crates/roym_web/ui/src/session/login.ts:43-44` (`setToken`),
`:53-54` (`clearToken`), `:284-285` (`whoami`);
`crates/roym_web/ui/src/events.ts:61-67` (`openFeed`);
`crates/router/src/route_handler/http/auth.rs:53-63` (the cookie is read
before `Authorization`); `docs/planning/changes/roym-open-foundation/change.md:395`
(the deviation line); `docs/decisions/0027-host-event-feed.md` (says nothing).

*Problem:* `EventSource` cannot send an `Authorization` header. The plan
assumed a session cookie already exists on the Hub's host; it does not (the
auth service sets its `HttpOnly` cookie on `auth.<domain>` only, and the
header comment of `login.ts:1-15` says the Hub sends only `Bearer`). To make
the feed work, the implementer writes `syneroym_session=<token>; path=/;
SameSite=Lax` from page JS. This changes the session model of ADR-0024 for
the whole Hub host, not only for the feed:

1. **Every** request from the browser to the Hub host now carries the
   session as an ambient cookie, including `POST /rpc`. The router accepts a
   cookie for any route. The `web` service does not check `Content-Type` or
   `Origin`. So a page on the same *site* (`SameSite=Lax` lets same-site
   requests carry the cookie) can send a simple cross-origin
   `POST /rpc` with `Content-Type: text/plain`, and it runs as the owner.
   Before this branch that was impossible, because the Hub host had no
   cookie. With the default `*.localhost` names each host is its own site,
   so the default setup is not open. With a real domain
   (`hub.node.example.com` and any other page under `example.com`), which A6
   brings, it is.
2. The cookie is shared by all tabs of the browser, but each tab keeps its
   own token in `sessionStorage`. The cookie holds the token of the tab that
   loaded last. `extract_session_token_from_hyper_headers` reads the cookie
   first and never falls back to `Bearer`: an expired or revoked token in the
   cookie makes the request anonymous even when its `Bearer` token is valid.
3. A logout in one tab clears the cookie for every tab. The other tabs keep
   working over `Bearer`, but their feed reconnect gets `401` and stays in
   the backoff loop until the page reloads. Live updates stop with no sign.
4. The change is recorded only as one deviation line. It is not in
   ADR-0027, not an owner decision, and the `login.ts` header comment is now
   false.

*Failure case:* (1) The node uses `hub.node.example.com`. A page on
`blog.example.com` posts `{"method":"conversation.send",…}` as `text/plain`
to `https://hub.node.example.com/rpc`. The browser sends the Hub cookie. The
message is sent as the owner. (2) Tab A logged in first, tab B later, then
tab A reloads (cookie = A's token). A's token expires. Every RPC from tab B
now fails as anonymous, although B's own `Bearer` token is valid.

*Fix:* Scope the cookie to the feed only: write it with
`Path=/_syneroym/events` (and a `Max-Age` no longer than the token's life),
so no other request carries it. In the router, read `Authorization` before
the cookie, or accept the cookie only for the events path. Say in ADR-0027
how the feed authenticates a browser, and fix the `login.ts` header comment.
Because this touches ADR-0024's rule, ask the owner to confirm the choice and
record the answer in the change doc. Add a router test: a request with a
revoked cookie and a valid `Bearer` is accepted (or the cookie is ignored off
the events path).

*Status:* Fixed (efd0f620), confirmed in the fixes check below. The Hub writes the cookie with `Path=/_syneroym/events` and a `Max-Age` from the token's `exp` (`setEventFeedCookie`), and the router reads `Authorization` before the cookie. Proof: with the extractor reading the cookie first, `a_request_with_a_revoked_cookie_and_a_valid_bearer_uses_the_bearer_token` fails; restored, it passes. The choice changes ADR-0024's session model for the feed, the owner confirmed the choice on 2026-11-04, and the deviation line and ADR-0027 record it.

#### R2 — major — an open feed keeps streaming after its session is revoked or expired

*Where:* `crates/router/src/route_handler/http/events.rs:166-204`
(`FeedState`, `make_feed_stream`) and `:223-235` (the only session check).

*Problem:* The session and the owner are checked once, when the feed opens.
After that the stream lives as long as the connection. The keep-alive frame
every 25 s keeps the connection busy, so it never times out. ADR-0024 says a
revoked token must not authenticate, and `resolve_effective_session_caller`
checks revocation for each request. A feed is one request that can last for
days. Neither the plan nor ADR-0027 says what happens to an open feed.

*Failure case:* A session token is stolen. The owner revokes it (logout or
`roymctl`). The thief's open `curl -N …/_syneroym/events` keeps receiving
every event of the app (conversation ids, message ids, card ids, timing) until
the node restarts. The same happens after the token's `exp` passes.

*Fix:* Keep the token string and its `exp` in `FeedState`. On every wake-up
(a change and the keep-alive tick), end the stream (`return None`) when the
token is revoked (`session_revocation.is_revoked`) or `exp` has passed. The
client then reconnects and gets `401`. Add a router test with a revocation
checker that turns revoked after the first frame: the next tick ends the
body. Say in ADR-0027 that a feed ends when its session ends.

*Status:* Fixed (efd0f620), confirmed in the fixes check below (its implementation adds R11). An open feed ends when its token is revoked or past `exp`, checked at connect, on every change and on every keep-alive tick. Proof: with the revocation check disabled, `a_revoked_session_terminates_an_open_feed_stream` fails; restored, it passes.

#### R3 — major — CI has not passed on this PR

*Where:* CI run [41732307695](https://github.com/syneroym/syneroym/actions/runs/41732307695)
on `4cfd96a0`, job "Test 1/6"; `status.md` row A5.

*Problem:* Test 1/6 failed at the build step. The runner warned "Free space
left: 17 MB", and `rust-lld` then crashed with a bus error while it linked
`service_visibility_e2e` and `messaging_client_e2e`. No test of that shard ran.
The only other run on the branch (41731943367) was cancelled. So no run proves
the full suite on this branch. `main` passed the same workflow 12 hours
earlier, so the shard was close to the disk limit already; this branch adds a
test binary (`roym_event_feed_e2e`) and more code, which may be enough to pass
the limit. The plan's step 10 (`mise run verify`, and the 20-run result in
`status.md`) is not recorded either.

*Failure case:* A test in shard 1 that this branch breaks would not be seen
before merge.

*Fix:* Re-run the job. If it fails on disk again, find what grew (for example
compare `du -sh target` of shard 1 on `main` and on this branch) and free
space in the CI job, or add the new e2e file as a module of an existing test
binary. Record a green run and the 20-run result in `status.md`.

*Status:* Rejected: not a code defect. Rejection accepted in the fixes check below. The failure was a full runner disk in Test 1/6 (17 MB free). CI run [41751830065](https://github.com/syneroym/syneroym/actions/runs/41751830065) on 0b38ce71 (the same code as 4cfd96a0, only docs differ) passed every job, Test 1/6 included. No code changed for this item. The disk margin stays small; if it fails again, move `roym_event_feed_e2e` into an existing test binary. The 20-run result is in `status.md`.

#### R4 — minor — the change doc's D-A5 decisions do not match the plan, and four docs give the wrong field cap

*Where:* `docs/planning/changes/roym-open-foundation/change.md:338-350`
(D-A5-1 to D-A5-13), `:174` (design text); `docs/decisions/0027-host-event-feed.md:26-27`;
`docs/planning/deferred-backlog.md:369`.

*Problem:* A decision must have the same number, meaning and values in the
plan and in the change doc.

- D-A5-1 says the map is a `HashMap<String, Window>` "behind an
  `Arc<RwLock<...>>`". The plan and the code use a `DashMap` of
  `Arc<Window>` with a `Mutex` per window. The sentence is false.
- D-A5-2 in the plan is the bus API. The change doc's D-A5-2 is "Writer
  identity", a different decision under the same number.
- D-A5-5 says `subject` and `item` are up to **256** bytes. The plan and the
  code (`event_bus.rs:255`, `:260`) say **128**. The change doc's design
  text (`:174`), ADR-0027 (`:26-27`) and the backlog row "Event-feed limits
  are constants" say 256 too. ADR-0027 becomes a permanent record, so the
  wrong number must be fixed before merge.
- D-A5-5 leaves out the `kind` character rule. D-A5-11 leaves out which
  deletes are visible and the refusal writer. D-A5-12 leaves out the
  `?after=` reopen and what `ready` means. D-A5-3 leaves out the reload rules.

*Failure case:* An A7 client author reads ADR-0027, sends a 200-byte subject
from a service, and gets `invalid-argument("subject")`.

*Fix:* Copy D-A5-1 to D-A5-13 from the plan with the same numbers and words
(short is fine, but the same meaning and values). Change 256 to 128 in the
change doc design text, ADR-0027 and the backlog row.

*Status:* Fixed (bf4fe2f3), confirmed in the fixes check below. D-A5-1 to D-A5-13 in the change doc now match the plan; `subject` and `item` say 128 bytes in the change doc, ADR-0027 and the backlog row.

#### R5 — minor — the evidence names tests that do not exist, and the counts are wrong

*Where:* `change.md:78-80` (OF-8, OF-9, OF-10 Test column);
`status.md` row A5; `task.md:84` (A5 result).

*Problem:*

- OF-8 cites `roym_event_feed_e2e.rs: a_sent_message_records_a_message_event_within_one_second`.
  OF-9 cites `a_reconnect_with_the_last_cursor_gets_the_missed_events` and
  `a_cursor_from_before_a_restart_gets_reload`. The file has one test,
  `roym_event_feed_reaches_clients_survives_reconnect_and_reloads_on_restart`,
  which calls five `check_*` helpers. None of the cited names exist.
- OF-10 cites `capabilities_events.rs` tests `events_read_and_record` and
  `events_scoping_by_app_instance`. They do not exist; the tests are in
  `host_capabilities/tests/tests_events.rs` (for example
  `read_reads_only_the_callers_app_instance`). It also cites
  `sending_a_message_records_a_message_event_in_both_builds`; the real name is
  `sending_a_message_records_the_same_event_in_both_builds`.
- `status.md` and `task.md` say 10 host capability tests (there are 5), 8
  parity events tests (there are 5) and 5 substrate e2e tests (there is 1).
  They do not give the 13 conversation tests, which plan section 9 asks for.
  The 20-run result (exit criterion 11) is missing.

*Failure case:* A reader runs `cargo nextest run … a_sent_message_records_a_message_event_within_one_second`
to check OF-8 and runs zero tests.

*Fix:* Use the real test names (or split the e2e test into the five tests of
plan 7.6, which also makes a failure name its case). Correct the counts in
`status.md` and `task.md`, add the conversation count and the 20-run result.

*Status:* Fixed (47b7e9d6), confirmed in the fixes check below. OF-8 to OF-10 name tests that exist (the one e2e test and its five steps, `tests_events.rs`, `sending_a_message_records_the_same_event_in_both_builds`). The counts in `status.md` and `task.md` are corrected, the conversation count is added, and the 20-run result is in `status.md`.

#### R6 — minor — some tests cannot fail or do not cover what the plan names

*Where:* `crates/router/src/route_handler/http/events/tests.rs:432-436`;
`crates/conversation/src/host_impl/tests/events.rs:475-497`;
`crates/roym_web/tests/dual_build_parity/events.rs:17-25`, `:95-96`,
`:145-149`, `:220-221`; `crates/substrate/tests/e2e/tests/roym-hub.spec.ts:1247-1293`.

*Problem:*

- `a_closed_feed_lowers_the_open_feeds_gauge` creates and drops a
  `FeedGuard` and asserts nothing. It cannot fail (the A3 R13 kind, which
  plan 7.10 names). Read the gauge with a local `metrics` recorder
  (`metrics_util::debugging::DebuggingRecorder`), or test that the permit
  and gauge come back after the body is dropped.
- `a_peer_refusal_records_a_delivery_event` covers only the refusal notice.
  Plan 7.3 asks for both writers; the refused acknowledgement
  (`transport.rs:286-303`, `record_refusal`) has no test. The refusal and the
  two send paths also run without the `announce_probe`, so moving their
  `announce` into the closure would not fail any test, although plan 7.3 says
  "move any `announce` call into the `transaction` closure" must fail.
- Plan 7.4 says each parity test compares the full list of
  `(kind, service, subject, item)`. `accepting_an_agreement_…` compares only
  the kinds (`kinds_w == kinds_n` and `contains`). `a_request_card_…` checks
  `item.is_some()`, not that the item is the card's message id.
- The parity helper `read_events_after` turns a `Reload` into an empty list,
  so the two "records nothing" tests would pass on a reload. Make it panic
  on `Reload`. `a_transaction_import_records_no_event` only checks that
  import did not fail with `-32602`; check that it succeeded.
- Hub case 41 never checks that the `503` was served (`failedOnce`), so it
  does not prove the reopen path it was written for.

*Failure case:* A change that drops the `FeedGuard` decrement, or records the
wrong card id, or moves the refusal `announce` into the transaction, passes
every test.

*Fix:* As in each point above.

*Status:* Fixed (15c06200), confirmed in the fixes check below. Proofs, each broken locally and restored: the gauge test fails when `FeedGuard` does not lower the gauge; `a_refused_delivery_ack_records_a_delivery_event` fails when the ack path announces before the refusal is stored; `a_peer_refusal_records_a_delivery_event` fails when the notice path announces inside the transaction; `sending_a_group_message_records_one_message_event` fails when the announce moves into the group transaction; the card parity test and the agreement parity test fail when the card event names the conversation instead of the message; `a_transaction_import_records_no_event` fails when the import returns an error; Hub case 41 fails when the 503 route does not match (`failedOnce` stays false). Not proven: the `Reload` panic in the parity helper (a one-line `panic!` arm), and the direct-send announce (the store runs its own transaction there, so no probe-visible mutation exists).

#### R7 — minor — the Hub misses list refreshes after a burst, and draws a thread from stale rows

*Where:* `crates/roym_web/ui/src/events.ts:115-123`;
`crates/roym_web/ui/src/screens/messages.ts:201-247` (`latestRows`,
`reloadList`); `change.md:397`.

*Problem:*

1. The debounce keeps only the **last** event of a burst for one subject.
   The Messages and Groups listeners refresh the list only when
   `kind.startsWith("conversation.")`. When a burst ends with a
   `transaction.*` event, the list is not refreshed.
2. `reloadList` no longer uses the value `fillList` returns. It reads
   `latestRows`, which is not cleared when the load fails or when a newer
   refill replaced this one. The `refiller` guard ("resolves to the rows when
   this run drew the list, else `undefined`") is now bypassed.

*Failure case:* (1) Alice sends a request card. `transaction` sends the
carrier message (`conversation.message`) and files the card
(`transaction.card`) within a few ms. One listener call with
`transaction.card` arrives. The thread refreshes, but the list still shows the
old "N messages" count. (2) `conversation.list` fails once (node busy). The
thread pane draws a conversation from the old rows, maybe one that was
deleted.

*Fix:* (1) Per subject, keep the set of kinds in the burst (or a flag "a
`conversation.*` kind was seen") and pass that to listeners. (2) Keep the
refiller result: when it is `undefined` because a newer run replaced this
one, let the newer run draw the thread; when the load failed, do not draw
it. Record the final rule in the Deviations line.

*Status:* Fixed (15c06200), confirmed in the fixes check below. The debounce keeps every kind of a burst in `kinds`, and Messages and Groups refresh the list when any kind is `conversation.*`. `reloadList` uses the rows that `fillList` returns and draws nothing when it returns `undefined`. Proof: with `kinds` reduced to the last kind, `a_burst_with_multiple_kinds_preserves_all_seen_kinds` fails; restored, it passes. The thread-draw rule has no unit test; the final rule is in the change doc deviation line.

#### R8 — minor — why-comments were lost in a refactor of `build_route_handler_deps`

*Where:* `crates/substrate/src/runtime/router.rs:340-397`
(`init_base_capabilities`, `wire_conversation_and_engine`, and the caller).

*Problem:* The plan's step 4 adds three lines here. The branch also splits the
function into helpers (not in the plan) and drops four why-comments that
were in `main`: the node identity shared with `SynSvcNativeService` to sign
relationship-proof records; "ADR-0021 §2: replay persisted bindings before
anything can resolve one"; "Cloning the `Arc`, not `node_identity` itself …
the supervisor role … ADR-0021 §8"; and "Both directions, `Weak` on both
sides". This is the A2b R8 / A4 R3 kind, which plan 7.10 names.
`init_base_capabilities` also returns a 6-tuple, which is hard to read.

*Failure case:* A later change moves `replay_persisted_bindings` after a
resolver use, because nothing says the order matters.

*Fix:* Put the four comments back beside the code they explain. Return a
small named struct instead of the 6-tuple. Record the split in the
Deviations if it stays.

*Status:* Fixed (c42b1fe3), confirmed in the fixes check below. `init_base_capabilities` returns a named `BaseCapabilities` struct, the four why-comments are back beside their code, and the split is recorded in the change doc deviations (47b7e9d6).

#### R9 — minor — three living-doc and interface statements are false or missing

*Where:* `docs/developer-guide.md:584`; `AGENTS.md:258`;
`crates/wit_interfaces/wit/events/events.wit:806-809` (doc of `read`).

*Problem:*

- The developer guide now says a proxy call to another service's `events`
  is refused with `permission-denied`. `events` is not in
  `NATIVE_CAPABILITY_INTERFACES` (`crates/core/src/local_registry.rs:44-53`),
  and no endpoint registers it, so that refusal does not exist.
- `AGENTS.md` lists the WIT interfaces (`…, conversation, invocation`) and
  does not list `events`. Earlier slices added their package there.
- The WIT doc says that with `limit` 0 `read` "returns no events and the
  current cursor". The code returns the **given** cursor
  (`event_bus.rs:216-218`), as D-A5-6's prose says. A guest that reads
  "current" as "head" skips events.

*Failure case:* A guest author calls `read(Some(c), 0)` to learn the head,
gets `c` back, then reads from there and sees old events again, or reads the
guide and expects a refusal that never comes.

*Fix:* Drop `events` from the guide's sentence (or add it to
`NATIVE_CAPABILITY_INTERFACES` if a refusal is wanted, with a test). Add
`events` to the `AGENTS.md` list. Change the WIT text to "returns no events
and the cursor it was given (the head when no cursor is given)".

*Status:* Fixed (bf4fe2f3), confirmed in the fixes check below. `events` is out of the developer guide's refusal sentence (it is not in `NATIVE_CAPABILITY_INTERFACES`, so no refusal exists), it is in the `AGENTS.md` WIT list, and the WIT text for `read` says that `limit` 0 returns the cursor it was given.

#### R10 — minor — widened APIs, a dead parameter, a lost event, and import-rule breaks

*Where:* see each point.

*Problem:*

- `crates/conversation/src/outbox.rs:112`: `drain_once` became `pub`. Its only
  other caller is the crate's own test (`outbox/tests/deposit.rs:233`),
  which could already reach it. `lib.rs:134` `registry()` and `:138`
  `event_bus()` are new `pub` getters used only by in-crate tests.
- `crates/roym_transaction/src/app/sync.rs:375`: `refuse_card` keeps a
  parameter `_msg_id` that is no longer used. Remove it from the signature
  and the callers.
- `crates/conversation/src/lib.rs:410-415` and `outbox.rs:420-425`: when
  `get_message` fails, the conversation id falls back to `""`, the bus
  refuses the empty subject, and the event is lost with no log line. Pass the
  id the caller already has, or log the miss.
- `crates/router/src/route_handler/http/events.rs:51`: the second
  `get("last-event-id")` is dead; `HeaderMap` lookup ignores case. `:194`:
  the 25 s keep-alive is a bare literal; D-A5-13 says the limits are named
  constants.
- Import rule (types by `use`, no inline paths with several `::`):
  `http.rs:310` and `gateway.rs:323` (`syneroym_core::event_bus::EVENT_FEED_PATH`);
  `transport.rs:717-721` (`rusqlite::Transaction<'_>`, `anyhow::Result`);
  `dual_build_parity/events.rs:9`, `:19` (`std::time::SystemTime`,
  `std::time::UNIX_EPOCH`) and `:77`, `:130-131`, `:165-166`, `:208` (`syneroym_roym_core::transaction::…`);
  `roym_event_feed_e2e.rs:105-106` (`std::path::Path`).

*Failure case:* Small, but each one is a rule the next reviewer must check
again.

*Fix:* As in each point.

*Status:* Fixed (e7682c8a, 149509de, efd0f620), confirmed in the fixes check below. `drain_once`, `registry()` and `event_bus()` are crate-private (the getters test-only); `refuse_card` lost `_msg_id`, and the unused message id is gone from the whole card filing chain; a missing message row logs a warning instead of sending an empty subject; the dead `last-event-id` lookup is gone and the 25 s keep-alive is `FEED_KEEP_ALIVE_INTERVAL`; the listed inline paths are imports now, in the router tests too.

## Code review round 1 — fixes checked — Claude Opus 5.5, 2026-11-04

I checked the fix commits efd0f620, bf4fe2f3, 15c06200, e7682c8a, 149509de,
c42b1fe3 and 47b7e9d6 on `feat/m08a-a5-event-feed` (main checkout, not a
worktree; `git pull` said "Already up to date"). 2d138ec0 and 27f0c9f9 set
the statuses and the `status.md` row. 2d138ec0 also changed the counts line
of round 1. That line is the record of the review, so I put it back. The new
counts are below. Line numbers below are at `27f0c9f9`. No code was changed.

CI: [run 41751830065](https://github.com/syneroym/syneroym/actions/runs/41751830065)
on 0b38ce71 passed every job, Test 1/6 included. The fix commits are in
[run 41773466844](https://github.com/syneroym/syneroym/actions/runs/41773466844)
on 27f0c9f9, which is still pending, so CI has not yet passed on the fixed
code. I ran only targeted tests: router `events` 17 passed, conversation
`events` 15 passed, sandbox_wasm `events` 5 passed, Hub vitest 156 passed.
I did not run the parity tests (they need a WASM rebuild) or the e2e tests.

**Confirmed Fixed.**
- **R1.** The Hub cookie has `path=/_syneroym/events`, `Max-Age` and
  `SameSite=Lax` (`login.ts:62-71`). `setToken`, `whoami` and `openFeed` all
  use `setEventFeedCookie`. `clearEventFeedCookie` also deletes the old
  `path=/` cookie. The router now reads `Authorization` before the cookie
  (`auth.rs:56-64`). So `/rpc` no longer carries an ambient session, and a
  stale cookie can no longer override a valid `Bearer`. Each tab writes its
  own token before it connects, so a logout in one tab does not end another
  tab's next reconnect. ADR-0027 and the change doc record the choice, and
  the `status.md` row records the owner's answer. See R12 for the `Max-Age`
  value.
- **R2.** The stream checks expiry and revocation at connect, on every
  wake-up and on every keep-alive tick (`events.rs:209-216`, `:226-233`,
  `:384-389`). `a_revoked_session_terminates_an_open_feed_stream` passes.
  The behaviour is right. The way the token reaches the stream is not: see
  R11. See R13 for the paths with no test.
- **R3.** Rejection accepted. Run 41751830065 is green on every job on code
  that equals 4cfd96a0. The new run on the fixes must still pass before
  merge.
- **R4.** D-A5-1 to D-A5-13 now match the plan. The 128-byte caps agree in
  the change doc, ADR-0027 and the backlog row.
- **R5.** OF-8 to OF-10 name tests that exist. The counts in `status.md` and
  `task.md` (18, 15, 5, 16, 5, 1, 9) match what I ran.
- **R6.** The gauge test uses a local recorder and checks 0 → 1 → 0. The
  refused ack, the peer refusal and the group send are probed with
  `conn_is_free`. The card parity test compares the message id, and the
  agreement parity test compares (kind, service, subject, item) with message
  ids mapped. `read_events_after` panics on `Reload`. Hub case 41 asserts
  `failedOnce`. The two gaps the fixer names (the `Reload` arm and the direct
  send) are acceptable.
- **R7.** The burst keeps every kind (`events.ts:120-140`), both screens
  refresh the list on any `conversation.*` kind, and `reloadList` uses the
  rows that `fillList` returns (`messages.ts:231-236`). `refiller` returns
  `undefined` when a newer run replaced it, so no stale rows are drawn.
- **R8.** `BaseCapabilities` replaces the 6-tuple, and the four
  why-comments are back beside their code.
- **R9.** The developer guide's refusal list no longer names `events`.
  `AGENTS.md` lists `events`. The WIT text for `limit` 0 matches
  `event_bus.rs:205-218`.
- **R10.** `drain_once` is `pub(crate)`, and the two getters are
  `#[cfg(test)] pub(crate)`. No caller outside the crate uses them.
  `_msg_id` and the unused message id in the card chain are gone. A missing
  message row logs a warning and sends no event. The dead
  `last-event-id` lookup is gone, and the keep-alive is a named constant.
  The inline paths named in R10 are imports now.

### New findings

#### R11 — major — the raw session token is copied into `session.claims`, which guest code receives

*Where:* `crates/router/src/route_handler/http/auth.rs:116-120` (inserts
`roym.session.token` and `roym.session.exp`);
`crates/router/src/route_handler/http.rs:293-305` (this caller becomes the
caller of every guest call, WebSocket and blob request);
`crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs:210`,
`:257`, `:402` (`self.caller.session` goes to `apply_stage4`);
`crates/rpc/src/fdae_abac.rs:88` (`claims_json` is all of
`session.claims`); `crates/sandbox_wasm/src/engine/auth.rs:574` (passed to
the guest as `claims-json`).

*Problem:* To check revocation on an open feed, the fix stores the bearer
session token string in `CallerContext.session.claims`. This happens in
`resolve_effective_session_caller`, so it applies to every session request,
not only to the feed. `session.claims` is not router-private. The FDAE row
authorizer (a WASM export of the app's own component) gets
`session.claims` serialized as `claims-json` on each data-layer read that
has an ABAC rule. The same `CallerContext` is also cloned into cross-service
FDAE fetches (`host_capabilities.rs:611`). `CallerContext` derives `Debug`,
so any future `?caller` log line prints the token too.

*Failure case:* The owner opens an app's page on their node with the Hub
session. The app's guest has an ABAC rule on a collection it reads. Its
`authorize-rows` export reads `claims-json` and gets the owner's live session
token. The token is valid for every service on the node whose owner is that
person, until it expires. So the guest can send it as `Authorization:
Bearer` to the Roym services (or any other app of the owner) and act as the
owner there. A guest is never meant to hold a credential of its caller.

*Fix:* Do not put the token in `session.claims`. The feed handler already
has the request headers (`handle_event_feed_parts(headers, …)`), so read the
token there with `extract_session_token_from_hyper_headers(headers)`, and
read the expiry from `SessionToken::verify` (or keep only `roym.session.exp`
in claims, since a time is not a credential, under a named constant). Keep
the token in `FeedState` only. Add a test that a session caller's
`AbacAuthContext.claims_json` does not contain the token.

*Status:* Open.

#### R12 — minor — `tokenMaxAge` parses a JWT, but the session token is z-base-32 JSON, so `Max-Age` is always 3600

*Where:* `crates/roym_web/ui/src/session/login.ts:45-59`;
`crates/ucan/src/session_token.rs:100-103` (`to_token_string` is
`z32::encode(serde_json::to_vec(..))`, with no `.`); the R1 status line and
ADR-0027 ("lifetime bounded by the session token's `exp`").

*Problem:* `tokenMaxAge` splits the token on `.` and reads `payload.exp`.
The real token has no `.` and its field is `expires_at_secs`, not `exp`. So
the parse always fails and the cookie always gets `Max-Age=3600`. The
behaviour is safe (the router still rejects an expired token, and `openFeed`
writes the cookie again before each connect), but the code is dead, the
docs and the R1 status say something that is not true, and no test covers
the function.

*Failure case:* A session that ends in 10 minutes leaves a cookie that lives
for 1 hour. The docs say the cookie never outlives the token.

*Fix:* Either use the expiry the login reply already gives the Hub (if it
has one), or drop the parse and say in the comment and in ADR-0027 that the
cookie lives 1 hour and is rewritten on each connect. Add a vitest for the
cookie string that `setEventFeedCookie` writes.

*Status:* Open.

#### R13 — minor — the expiry checks and the keep-alive check have no test

*Where:* `crates/router/src/route_handler/http/events.rs:169-187`
(`is_session_expired_or_revoked`), `:226-233` (keep-alive path), `:384-389`
(check at connect); `events/tests.rs` (only
`a_revoked_session_terminates_an_open_feed_stream`, which wakes on a change).

*Problem:* R2 asked that both a revoked and an expired session end the
stream, also when no event arrives. Only revocation on a change wake-up is
tested. The expiry branch, the 401 at connect, and the keep-alive branch
are not. Removing the check from the keep-alive arm, or swapping `>=` for
`>` on `exp`, would pass every test.

*Failure case:* A later refactor drops the keep-alive check. A revoked
session on a quiet conversation keeps its feed open with no limit, and CI
stays green.

*Fix:* Add three tests: a session whose `exp` is in the past gets 401 at
connect; an open stream with `exp` reached ends on the next change; and with
`tokio::time::pause` plus `advance(FEED_KEEP_ALIVE_INTERVAL)`, a revoked
session ends without any event. Write them after the R11 change, since R11
changes where the token and the expiry come from.

*Status:* Open.

**Counts.** Fixes check: 9 confirmed (R1, R2, R4 to R10), 1 rejection
accepted (R3), 3 new (R11 major, R12 and R13 minor). Code review round 1
now has 13 items: 0 blocker, 4 major, 9 minor. **3 open** (R11, R12, R13).

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

**Summary:** 0 blocker, 0 major, 1 minor, 1 item. Open.

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
- *Status:* Open.

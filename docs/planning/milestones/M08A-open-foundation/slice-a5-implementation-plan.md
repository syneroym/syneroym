# M08A slice A5 — The event feed: implementation plan

*Reader: the developer (or AI) who implements slice A5. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written 2026-11-03. The owner answered Q1–Q8 on 2026-11-03,
all (a). Plan review round 1 ([slice-a5-review.md](slice-a5-review.md), P1 to
P12) is folded in.**

**Size and PR.** M. One PR on `feat/m08a-a5-event-feed`, with the commits of
section 6 in that order. The branch starts from `main` at `b90940c0` (A4 is
merged). A5 adds one module to `syneroym-core`, one WIT package, one host
capability in `sandbox_wasm`, one trait in `syneroym-app-host` (with its three
implementers), event writes in the host conversation crate and in
`roym_transaction`, one router path, one gateway path rule, live refresh in the
Hub, and the demo story change. Section 10 lists exactly which `src/` trees
change.

All links are to `main` at `b90940c0`.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - Functions of at most 100 lines. Match arms of at most 10 lines. Rust
    files of at most 800 lines. Section 7.6 lists every file that grows and
    its length today.
  - The oversized-test-file list is a ratchet. `crates/roym_web/tests/dual_build_parity/helpers.rs`
    is listed at its real length (1 429). It must not grow (Step 5 says how).
  - WASM rules: A5 adds an import to every Roym component. After any change
    to a Roym crate, `syneroym-app-host` or a WIT file, rebuild the WASM
    artifacts (`mise run build:roym`) before any test that loads them. Roym
    tests return early when `roym_artifacts_present()` is false
    ([common/roym.rs:60](../../../../crates/substrate/tests/common/roym.rs#L60)), so check the
    log shows they ran.
  - No planning ids in code: no "M08A", "A5", "OF-8", "D-A5-3" in code,
    comments or test names. ADR ids are fine.
  - Comments never cite size rules or line counts.
  - Tests never hardcode a port.
  - The import clean-up pass, the deferred backlog, the living-docs close-out,
    the demo close-out, the ratchets (`check-file-lengths`,
    `check-duplication`, `check-lint-suppressions`).
- [task.md](task.md): section 1 (X4), section 2 (facts) and "A5 — Event feed".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  "Event feed" (design and table), OF-8 to OF-11, decisions D-OF-3 and D-OF-4.
  No other change doc defines a criterion for A5. `task.md` names no other
  change doc for this slice.
- [slice-s0-implementation-plan.md](slice-s0-implementation-plan.md) Q5 to Q7
  and D-OF-3, D-OF-4: why the feed is a host-level bus, the window, the four
  kinds.
- [ADR-0025](../../../decisions/0025-conversation-capability-owns-history.md):
  the conversation capability (the host) owns message history. Q1 builds on it.
- [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md):
  login sessions and the session cookie. Q5 builds on it.
- The review files [slice-a2a-review.md](slice-a2a-review.md),
  [slice-a2b-review.md](slice-a2b-review.md),
  [slice-a2c-review.md](slice-a2c-review.md),
  [slice-a3-review.md](slice-a3-review.md) and
  [slice-a4-review.md](slice-a4-review.md). Section 7.7 lists how A5 avoids
  each kind of finding.
- [deferred-backlog.md](../../deferred-backlog.md) section 8, row "The
  event-feed catch-up window is lost on restart" (:367).

### Terms used in this plan

- **Event:** a short note that says "something changed". It names a kind, the
  service that wrote it, and which thing changed. It never carries content.
  The client fetches the content with the normal methods.
- **Event bus** (or **bus**): the one in-memory object in the substrate that
  holds the events. One per node process.
- **App instance:** one deployed copy of a SynApp on a node (for example
  Roym). Its id is the string `EndpointRegistry::app_context_of` returns
  ([local_registry.rs:522](../../../../crates/core/src/local_registry.rs#L522)).
- **Window:** the newest events of one app instance that the bus keeps, at most
  10 000 and at most 24 hours old (D-OF-4).
- **Cursor:** the position of one event in a window: the bus's run id and a
  sequence number. A client sends its last cursor back to catch up.
- **Run id:** a random id the bus makes when the node starts. A cursor from an
  older run gets "reload".
- **Feed:** one client connection that reads events.
- **SSE:** Server-Sent Events, a plain HTTP `GET` whose response body is a
  stream of `event:`/`id:`/`data:` lines. Browsers read it with `EventSource`.
  After a network error or a closed `200` stream, `EventSource` reconnects by
  itself and sends the last `id:` back in the `Last-Event-ID` header. After
  any other HTTP status it closes for good and never retries, so the Hub
  reopens it itself (D-A5-12).
- **Writer:** code that records an event. In A5 there are two: the host
  conversation crate and the Roym `transaction` service.

## 1. What A5 builds, in one paragraph

A5 adds an event bus to the substrate. It keeps, per app instance, a window of
small events in memory. Code that changes something records one event with one
call. The host conversation crate records `conversation.message` when a
message becomes visible or is deleted, and `conversation.delivery` when the
delivery state of an outgoing message changes (Q1). The Roym `transaction`
service records `transaction.card` and `transaction.booking` through a new
host interface, `syneroym:events`, after it stores a card or a booking. The
host tags each event with the writer's app instance from its own records,
never from the event. A client opens the feed with its login session:
`GET /_syneroym/events` on the app's hostname, answered as SSE (Q2). It gets
every event of that app instance after its cursor, live, or "reload" when its
cursor is too old. A service reads the same events of its own app instance
with `events.read` (Q3). The Hub keeps one feed open while its tab is visible
and refreshes the open conversation and the conversation list when an event
names them. The demo
story shows Bob's reply and the `delivered` state appear with no reopen step.

## How this is shown to an end user

- Alice opens **Messages** and her conversation with Bob. She keeps the screen
  open.
- Bob sends a reply from his own node.
- Within about a second, Bob's reply appears on Alice's screen. She does not
  click anything or open the screen again.
- When Alice's own message reaches Bob's node, the small state word under it
  changes from `pending` to `delivered` on her screen, also by itself.
- A new card (a request, a quote, an agreement) from the other person appears
  the same way.
- If Alice's connection to her node drops and comes back, the screen catches
  up by itself. If it was away too long, the screen loads again.

A developer can see the work in these ways:

- `mise run demo` plays the chat story. Bob's message and Alice's `delivered`
  state appear with no reopen step.
- `curl -N -H 'Host: <web host>' -H 'Cookie: syneroym_session=<token>'
  http://127.0.0.1:7960/_syneroym/events` prints one `ready` frame, then one
  frame per change, while you send messages from another terminal.
- `cargo nextest run -p syneroym-core event_bus` runs the bus tests (window,
  cursors, reload, app scoping).

## 2. Points for the owner

Each question has options, a recommendation and the reason. The owner chose
the recommendation, (a), for every question on 2026-11-03; the answer is under
each question. Sections 5 and 6 follow these answers.

Four choices are not questions, because earlier decisions settle them:

- The bus is in the host, one per node, and tags each event with the app
  instance from `app_context_of` (D-OF-3).
- The window is in memory, newest 10 000 or 24 hours per app instance, with a
  run id in each cursor, and "reload" after a restart (D-OF-4).
- The four kinds of v1 are `conversation.message`, `conversation.delivery`,
  `transaction.card`, `transaction.booking` (D-OF-4).
- An event carries no message content (OF-8).

### Q1. Who records the two conversation events?

The facts decide this more than taste:

- Every message row, its sequence number (`visible_seq`) and its delivery
  state live in the **host** crate `crates/conversation`, not in the Roym
  `conversation` guest (ADR-0025). The guest only calls the capability.
- An inbound message is stored as `undecided` and invisible. The host asks the
  guest `on-message` for an admission answer, and only **after** the guest
  returns does the host apply it
  ([transport.rs:526-545](../../../../crates/conversation/src/transport.rs#L526),
  [lib.rs:172-199](../../../../crates/conversation/src/lib.rs#L172),
  [outbox.rs:478-487](../../../../crates/conversation/src/outbox.rs#L478)). The
  sequence number is assigned in `apply_admission_conn`
  ([store/message.rs:305-311](../../../../crates/conversation/src/store/message.rs#L305)).
- The delivery state is set by the host outbox worker, which then calls the
  guest's `on-delivery-state`
  ([outbox.rs:362-371](../../../../crates/conversation/src/outbox.rs#L362)). Roym
  ignores that callback today
  ([guest.rs:65](../../../../crates/roym_conversation/src/guest.rs#L65),
  [native.rs:70](../../../../crates/roym_conversation/src/native.rs#L70)).

Options:

- (a) **The host conversation crate records both kinds**, at the moment the
  change is stored: when a row gets its sequence number, when a visible row is
  deleted, and when an outgoing row's state changes. Kinds and fields are the
  same words as in D-OF-4. Any SynApp that uses `syneroym:conversation` gets
  these events for its own app instance.
- (b) The Roym `conversation` guest records them from `on-message` and
  `on-delivery-state`. The event from `on-message` is written **before** the
  host makes the row visible. A client that reacts fast calls
  `conversation.history` and does not see the message yet, and no second event
  comes. To fix that, the host would have to hold events written during an
  `on-message` call until it applies the admission: new machinery for one case.
  Group messages and retries reach the guest by other paths too.
- (c) (b) without the fix. Rejected: a lost message on a live screen is the
  bug OF-11 is about.

**Recommendation: (a).** The event is written where the state is, after it
is stored, so there is no race. It covers every path (inbound direct, group
sync, re-asked admissions, retries, deletes, sends) in one crate. It needs no
WASM instance per event. D-OF-4 said "owned and written by Roym services";
with (a) two of the four kinds are written by the host capability that owns
them. The change doc records that as a deviation.

Owner answer: (a), 2026-11-03.

### Q2. How does a client outside the node read the feed?

The change doc says "WebSocket for clients outside the node". The feed only
goes one way: node to client.

- (a) **SSE on `GET /_syneroym/events`.** One plain HTTP request on the app's
  own hostname, with the session cookie. The answer is a stream of
  `event:`/`id:`/`data:` frames. A browser uses `EventSource`, which
  reconnects by itself after a network error and sends the last cursor in
  `Last-Event-ID`. After an error status (401, 404, 503) it stops, so the
  client still needs a small reopen loop (D-A5-12). `curl -N` reads it, which makes the
  client guide (A7) simple. The router already serves SSE for messaging routes
  ([streaming.rs:9](../../../../crates/router/src/route_handler/http/streaming.rs#L9)),
  with the same body-stream and permit pattern.
- (b) WebSocket on the same path. Two-way, but nothing goes from client to
  node. The client must write its own reconnect and send its cursor in the URL.
  A7 then needs `websocat`.
- (c) Roym's own `GET /ws` route
  ([roym.toml:26](../../../../crates/roym_core/app/roym.toml#L26)). Each frame
  goes to a new `web` guest instance with no state, the problem D-OF-3 avoided.
  Rejected.

**Recommendation: (a).** It is the simpler protocol for a one-way stream, and
the browser does the reconnect and catch-up. The change doc's Scope line says
"WebSocket"; with (a) it is a deviation and task.md's X4 line changes word.

Owner answer: (a), 2026-11-03.

### Q3. How does a service on the node read the events (OF-10)?

No Roym service needs to react to an event in A5. M08C (extensions) and M08F
(match events) do.

- (a) **Pull: `events.read(after, limit)` in `syneroym:events`.** A service
  reads the events of its own app instance after a cursor, in the same guest
  call that needs them. It costs nothing when nobody reads. It is the same bus
  code as the client catch-up. Push comes with the first service that needs
  it.
- (b) Push: the host calls an exported `on-event` on each subscribed service.
  In the WASM build that is one new guest instance per event per subscriber
  ([auth.rs:251](../../../../crates/sandbox_wasm/src/engine/auth.rs#L251) shows the
  same cost for `on-message`), and a subscription list to keep.
- (c) Both now.

**Recommendation: (a).** It meets OF-10 ("subscribe ... without WebSocket",
"only the events of its own app instance") with no instance per event. (b)
with no real subscriber would be designed blind. A backlog row records push
for M08C.

Owner answer: (a), 2026-11-03.

### Q4. What does one event say?

The change doc says "a kind, an id and a cursor".

- (a) **`{cursor, kind, service, subject, item, at_ms}`.** `subject` is the
  thing a client loads again (a conversation id for all four v1 kinds);
  `item` is the one row that changed inside it, if any (a message id, a card
  id, an agreement id). `service` is the writer's service name inside the app
  (`conversation`, `transaction`), set by the host. `at_ms` is the node's clock
  when it was recorded.
- (b) `{cursor, kind, id}` only. A card event would then need the client to
  look up which conversation the card is in, before it knows whether the open
  screen must change.

**Recommendation: (a).** A client decides "does my open screen change?" from
`subject` alone, with no extra call. `item` lets a client refresh one row.
Both are ids the client can already read, so OF-8 ("no message content") still
holds.

Owner answer: (a), 2026-11-03.

### Q5. Which events may one login session read?

A session token names a person DID and no app (`SessionTokenClaims`,
[session_token.rs:31-37](../../../../crates/ucan/src/session_token.rs#L31)). The
router turns a valid cookie into a caller with that DID
([auth.rs:78-125](../../../../crates/router/src/route_handler/http/auth.rs#L78)).
Roym's `web` admits a person only when the DID equals the owner of the
service ([app.rs:114-129](../../../../crates/roym_web/src/app.rs#L114)), read
from the same host record `EndpointRegistry::owner_of`
([local_registry.rs:365](../../../../crates/core/src/local_registry.rs#L365)).

- (a) **One feed per app instance, addressed by hostname, for its owner
  only.** The client opens `/_syneroym/events` on the hostname of any service
  of the app (the Hub uses its own origin). The router finds the app instance
  of that service and serves its events only if the session's DID is the
  service's owner. No session: 401. Another person: 403.
- (b) One feed per session with the events of every app instance the person
  owns on the node. Needs a list of app instances per owner, which no code
  keeps today, and a client that reads apps it never opened.

**Recommendation: (a).** It reuses the exact owner rule `web` already
applies, and one app can never read another app's events, because the app
instance comes from the hostname the router already resolved.

Owner answer: (a), 2026-11-03.

### Q6. What does the Hub refresh?

- (a) **The Messages screen (the conversation list and the open
  conversation) and the open group on the Groups screen.** Other tabs ignore
  events. A refresh reuses the screens' own `refiller` functions
  ([dom.ts:31](../../../../crates/roym_web/ui/src/dom.ts#L31)), which already
  drop an older refresh that finishes late.
- (b) The Messages screen only.

**Recommendation: (a).** A group message is a `conversation.message` event too,
and the group screen has the same refill shape. Other tabs (Listings,
Directory) have no v1 event.

Owner answer: (a), 2026-11-03.

### Q7. How does the demo story show it?

The story uses one browser tab that moves between Alice's and Bob's Hubs
([20-roym-chat.spec.ts:28](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts#L28)).
Each move is a page load, which would hide the live update.

- (a) **Two tabs, one per person, both kept open.** Alice's tab shows her
  conversation; the story brings Bob's tab to the front, Bob replies, then
  brings Alice's tab back, and the reply and `delivered` appear with no
  reload. The two `toPass` reopen loops
  ([:62-67](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts#L62),
  [:77-89](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts#L77)) go away.
  With `DEMO_VIDEO` each tab records its own video file.
- (b) Keep one tab. Only Bob's first screen can show a live arrival.

**Recommendation: (a).** It is the viewer-visible point of the milestone demo
(task.md "Demo").

Owner answer: (a), 2026-11-03.

### Q8. Does the event feed get an ADR?

The bus is a new host capability for every SynApp, with a new gateway path
and a new WIT package.

- (a) **Yes: ADR-0027 "Host event feed"** at close-out: one bus per node,
  scoped by app instance from host records, in-memory window, SSE for clients,
  pull for services, conversation capability as a writer.
- (b) No. The change doc's decisions are the record.

**Recommendation: (a).** Other host capabilities of this kind have ADRs
(ADR-0024 for the gateway sessions, ADR-0025 for the conversation capability).
A change doc is archived when M8 closes; an ADR stays.

Owner answer: (a), 2026-11-03.

## 3. Facts checked in the code

### 3.1 Where the state lives

- Messages: host crate `crates/conversation`. Tables `messages` and
  `conversation_seq` (`store/schema.rs:77`, `:110`). `next_visible_seq`
  ([store/message.rs:28](../../../../crates/conversation/src/store/message.rs#L28))
  assigns the sequence number that `conversation.changes` reads.
- Places a message becomes visible:
  - local direct send: `send` ([host_impl.rs:76](../../../../crates/conversation/src/host_impl.rs#L76))
    → `enqueue_direct` ([lib.rs:332](../../../../crates/conversation/src/lib.rs#L332))
    → `insert_outgoing_and_enqueue` (seq at
    [store/message.rs:92](../../../../crates/conversation/src/store/message.rs#L92));
  - local group send: `send_group` ([group.rs:452](../../../../crates/conversation/src/group.rs#L452))
    → `record_outgoing_group_message` (seq at
    [group.rs:401](../../../../crates/conversation/src/group.rs#L401));
  - an accepted inbound message: `apply_admission` with `Accept`, called at
    [transport.rs:537](../../../../crates/conversation/src/transport.rs#L537),
    [:545](../../../../crates/conversation/src/transport.rs#L545),
    [lib.rs:191](../../../../crates/conversation/src/lib.rs#L191),
    [:194](../../../../crates/conversation/src/lib.rs#L194),
    [outbox.rs:483](../../../../crates/conversation/src/outbox.rs#L483) and
    [:487](../../../../crates/conversation/src/outbox.rs#L487).
    `notify_and_apply_admission` (lib.rs:172) is used by group paths at
    [group.rs:555](../../../../crates/conversation/src/group.rs#L555),
    [group_sync.rs:137](../../../../crates/conversation/src/transport/group_sync.rs#L137) and
    [:351](../../../../crates/conversation/src/transport/group_sync.rs#L351).
    `apply_admission_conn` does nothing for a row that is already accepted
    ([store/message.rs:297-303](../../../../crates/conversation/src/store/message.rs#L297)),
    so a racing re-ask gets no second number.
  - backup import (`store/backup.rs:395`). No event (D-A5-11).
  - `insert_outgoing_without_enqueue`
    ([store/message.rs:136-148](../../../../crates/conversation/src/store/message.rs#L136))
    also gives a sequence number. It has no production caller: only tests
    call it (`store/tests/*`, `host_impl/tests.rs`, and the parity test
    `group_lifecycle.rs:258`). It records nothing.
- Places a visible message changes:
  - local delete: [host_impl.rs:300](../../../../crates/conversation/src/host_impl.rs#L300);
  - a peer's deletion request: [transport.rs:467](../../../../crates/conversation/src/transport.rs#L467);
  - a group deletion entry: [group/entry.rs:218](../../../../crates/conversation/src/group/entry.rs#L218).
- **Three of these writers run inside a transaction**, so they cannot record
  the event themselves (the event would come before `COMMIT`):
  - `handle_inbound_deletion_request(tx, ..)`
    ([store/message.rs:440](../../../../crates/conversation/src/store/message.rs#L440))
    runs inside `store.queue().transaction(..)` in `ingest_verified`
    ([transport.rs:431-480](../../../../crates/conversation/src/transport.rs#L431)).
    It returns `Result<bool>` and does not name the deleted message.
  - `apply_group_deletion_request(tx, ..)`
    ([group/entry.rs:218](../../../../crates/conversation/src/group/entry.rs#L218))
    returns `Result<()>`, inside the group-entry transaction (caller at
    [group/entry.rs:154](../../../../crates/conversation/src/group/entry.rs#L154)).
  - `record_outgoing_group_message(tx, ..)` ([group.rs:366](../../../../crates/conversation/src/group.rs#L366))
    and `insert_outgoing_and_enqueue` ([store/message.rs:53](../../../../crates/conversation/src/store/message.rs#L53))
    take or open a transaction.
  The queue transaction and the store use one `Arc<Mutex<Connection>>`
  ([async_queue/queue.rs:287-297](../../../../crates/async_queue/src/queue.rs#L287),
  [store/schema.rs:32](../../../../crates/conversation/src/store/schema.rs#L32)).
- `delete_message_conn` ([store/message.rs:379-436](../../../../crates/conversation/src/store/message.rs#L379))
  returns `Ok(true)` for any row it finds. That includes an outgoing
  `pending`/`deposited` row (it becomes `failed`) and an inbound row that was
  `undecided` or `held` (it becomes `dropped`, which was never visible).
- A refusal changes a visible outgoing row without a delivery state: it sets
  `messages.refused`, which the Hub shows under the message
  ([messages.ts:505](../../../../crates/roym_web/ui/src/screens/messages.ts#L505)).
  Two writers: `handle_inbound_refusal_notice(tx, ..)`
  ([store/message.rs:466](../../../../crates/conversation/src/store/message.rs#L466),
  inside the `ingest_verified` transaction at transport.rs:476), and
  `record_refusal` ([store/message.rs:535](../../../../crates/conversation/src/store/message.rs#L535),
  its own lock, called at [transport.rs:287](../../../../crates/conversation/src/transport.rs#L287)).
- A group send also has per-member states (`message_recipients`,
  `set_recipient_state` at [store/message.rs:654](../../../../crates/conversation/src/store/message.rs#L654),
  called at outbox.rs:348, :403 and host_impl.rs:199). Only the message's own
  state goes through `notify_state`. No Hub screen shows per-member states.
- Delivery state changes all go through `notify_state`
  ([lib.rs:161](../../../../crates/conversation/src/lib.rs#L161)), called at
  [lib.rs:328](../../../../crates/conversation/src/lib.rs#L328),
  [outbox.rs:306](../../../../crates/conversation/src/outbox.rs#L306),
  [:362](../../../../crates/conversation/src/outbox.rs#L362),
  [:365](../../../../crates/conversation/src/outbox.rs#L365),
  [:371](../../../../crates/conversation/src/outbox.rs#L371),
  [:415](../../../../crates/conversation/src/outbox.rs#L415),
  [:427](../../../../crates/conversation/src/outbox.rs#L427) and
  [outbox/parked.rs:139](../../../../crates/conversation/src/outbox/parked.rs#L139).
  Each call follows `set_state`, so the state is stored first. One change does
  not call it: `retry` → `restart_pending`
  ([host_impl.rs:148](../../../../crates/conversation/src/host_impl.rs#L148)),
  failed back to pending. A5 adds the event there.
- `conversation.changes` returns rows with `visible_seq > after_seq`
  ([store/history.rs:34-36](../../../../crates/conversation/src/store/history.rs#L34)).
  A state change or a delete does not change `visible_seq`, so `changes`
  does not show them. A client refreshes with `conversation.history` for those
  (the Hub already does).
- `ConversationService` holds the `EndpointRegistry`
  ([lib.rs:92-113](../../../../crates/conversation/src/lib.rs#L92)) and is
  wired after construction with `set_service_proxy` and `set_notifier`
  (`lib.rs:117`, `:127`). It is built at
  [runtime/router.rs:554](../../../../crates/substrate/src/runtime/router.rs#L554).
- Cards: guest collection `cards` of `roym_transaction`
  ([app.rs:53](../../../../crates/roym_transaction/src/app.rs#L53)). `CardRow`
  ([app.rs:183](../../../../crates/roym_transaction/src/app.rs#L183)) has
  `message_id` (the row id) and `conversation`. The card body itself is a
  conversation message (content type `application/vnd.roym.card+json`).
  Writes of `cards` (all through `put_row`,
  [app.rs:379](../../../../crates/roym_transaction/src/app.rs#L379)):
  `file_own_card` [app.rs:485](../../../../crates/roym_transaction/src/app.rs#L485);
  `sync.rs` [:359](../../../../crates/roym_transaction/src/app/sync.rs#L359),
  [:380](../../../../crates/roym_transaction/src/app/sync.rs#L380),
  [:422](../../../../crates/roym_transaction/src/app/sync.rs#L422),
  [:487](../../../../crates/roym_transaction/src/app/sync.rs#L487),
  [:578](../../../../crates/roym_transaction/src/app/sync.rs#L578);
  `sync/receipts.rs` [:126](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L126),
  [:209](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L209),
  [:233](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L233),
  [:310](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L310),
  [:423](../../../../crates/roym_transaction/src/app/sync/receipts.rs#L423);
  backup import [backup.rs:660](../../../../crates/roym_transaction/src/app/backup.rs#L660)
  (no event, D-A5-11).
- **Inbound cards are filed only when someone calls `transaction.sync`**
  ([sync.rs:55](../../../../crates/roym_transaction/src/app/sync.rs#L55)). The
  Hub calls it at the start of every thread refresh
  ([messages.ts:341](../../../../crates/roym_web/ui/src/screens/messages.ts#L341)).
  So for the Hub, an inbound card first shows up as `conversation.message`
  (its carrier message); the refresh files it and records `transaction.card`.
- Bookings: collection `bookings`, key = agreement id. `BookingRow`
  ([app.rs:221](../../../../crates/roym_transaction/src/app.rs#L221)) has
  `agreement` and `conversation`. Writes:
  [booking_ops.rs:146](../../../../crates/roym_transaction/src/app/booking_ops.rs#L146),
  [:168](../../../../crates/roym_transaction/src/app/booking_ops.rs#L168),
  [:263](../../../../crates/roym_transaction/src/app/booking_ops.rs#L263);
  [ledger.rs:278](../../../../crates/roym_transaction/src/app/ledger.rs#L278),
  [:333](../../../../crates/roym_transaction/src/app/ledger.rs#L333);
  backup import [backup/sections.rs:444](../../../../crates/roym_transaction/src/app/backup/sections.rs#L444)
  (no event). The Hub never calls `booking.*`; it shows booking state through
  booking-progress cards.

### 3.2 App instance, owner and session

- The host knows the app instance of a service from its own records:
  `app_context_of(service_id) -> Option<(app_instance_id, service_name)>`
  ([local_registry.rs:522](../../../../crates/core/src/local_registry.rs#L522)).
  WASM reads it at [engine.rs:536-540](../../../../crates/sandbox_wasm/src/engine.rs#L536)
  into `HostState.app_instance_id`; native at
  [factory.rs:288-291](../../../../crates/app_host_native/src/factory.rs#L288).
  The native Roym services get their context at
  [roym.rs:455-463](../../../../crates/substrate/src/runtime/roym.rs#L455) and their
  owner at [roym.rs:118](../../../../crates/substrate/src/runtime/roym.rs#L118);
  deployed apps at `control_plane/.../backends.rs:57` and `deploy/commit.rs:91`.
- `owner_of(service_id)` ([local_registry.rs:365](../../../../crates/core/src/local_registry.rs#L365)).
- Sessions: cookie `syneroym_session` or `Authorization: Bearer`
  ([auth.rs:53-75](../../../../crates/router/src/route_handler/http/auth.rs#L53)).
  `resolve_effective_session_caller`
  ([auth.rs:78-125](../../../../crates/router/src/route_handler/http/auth.rs#L78))
  runs for every HTTP request ([http.rs:290-297](../../../../crates/router/src/route_handler/http.rs#L290)),
  only for gateway-origin traffic, checks revocation, and marks the caller
  with `SESSION_CALLER_MARKER` ([auth.rs:5](../../../../crates/router/src/route_handler/http/auth.rs#L5)).
  Tokens are stateless; there is no session store.

### 3.3 Router and gateway

- The gateway is a raw TCP proxy. It binds at
  [gateway.rs:184](../../../../crates/client_gateway/src/gateway.rs#L184), reads the
  headers, and **answers 404 for every `/_syneroym/` path except
  `/_syneroym/session*`** ([gateway.rs:411-413](../../../../crates/client_gateway/src/gateway.rs#L411)).
  Otherwise it copies bytes both ways (`copy_bidirectional`), so a long
  response body streams through with no gateway code.
- The router serves HTTP with hyper per stream
  ([http.rs:176-212](../../../../crates/router/src/route_handler/http.rs#L176)).
  `try_handle_http_request` ([http.rs:290](../../../../crates/router/src/route_handler/http.rs#L290))
  checks the fixed `/blobs/` prefix first ([http.rs:309-314](../../../../crates/router/src/route_handler/http.rs#L309)),
  then assets, then the service's route table. A5 adds its path beside the
  blob check.
- SSE today: `handle_messaging_sse`
  ([streaming.rs:9-90](../../../../crates/router/src/route_handler/http/streaming.rs#L9))
  checks `Accept: text/event-stream`, takes a per-service permit from
  `sse_permits` (503 with `Retry-After: 1` when full) and returns a body made
  by `stream::unfold` that owns the permit, so a client that goes away frees
  it. A5 copies this shape; it does not reuse the MQTT subscription.
- `RouteHandlerInner` fields: [route_handler.rs:96-180](../../../../crates/router/src/route_handler.rs#L96);
  `RouteHandlerDeps`: [:199-238](../../../../crates/router/src/route_handler.rs#L199).

### 3.4 Host interfaces and the native shim

- WIT packages: `crates/wit_interfaces/wit/`. Host bindings are one
  `bindgen!` per package so a component that does not import a package still
  links ([invocation_host.rs:1-11](../../../../crates/wit_interfaces/src/invocation_host.rs#L1)).
  Guest bindings: one `generate!` per package behind a Cargo feature
  ([lib.rs:5-28](../../../../crates/wit_interfaces/src/lib.rs#L5),
  [Cargo.toml:23](../../../../crates/wit_interfaces/Cargo.toml#L23)).
- The linker adds every interface:
  [init.rs:396-412](../../../../crates/sandbox_wasm/src/engine/init.rs#L396).
- `HostState` fields: [host_capabilities.rs:340-405](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L340);
  builders such as `with_call_all` ([:514](../../../../crates/sandbox_wasm/src/host_capabilities.rs#L514)).
- `AppHost` is a supertrait of ten traits
  ([app_host/lib.rs:39-68](../../../../crates/app_host/src/lib.rs#L39)). The
  smallest is `AppWebSocket` ([:136](../../../../crates/app_host/src/lib.rs#L136)).
  Its implementers: `GuestHost`
  ([guest.rs:446](../../../../crates/app_host/src/guest.rs#L446)),
  `NativeAppHost` ([host.rs:689](../../../../crates/app_host_native/src/host.rs#L689)),
  and the test host in `roym_core`
  ([signing.rs:684](../../../../crates/roym_core/src/signing.rs#L684)). A new
  trait in `AppHost` needs all three.
- Every user of `GuestHost` must import every interface `GuestHost` calls.
  These are the six Roym worlds (`crates/roym_*/wit/world.wit`, for example
  [roym_transaction/wit/world.wit](../../../../crates/roym_transaction/wit/world.wit)),
  each with `wit/deps/<pkg>` and a
  `[package.metadata.component.target.dependencies]` entry, and
  `test-components/dual-build-fixture/wit/world.wit`. `syneroym:invocation`
  was added the same way (C5).
- `syneroym-app-host` turns on only the WIT features it uses
  ([app_host/Cargo.toml:18](../../../../crates/app_host/Cargo.toml#L18)).
- Native factory: `NativeHostFactory` with `with_call_all` and a `OnceLock`
  ([factory.rs:148-151](../../../../crates/app_host_native/src/factory.rs#L148));
  `build_host_state` ([:282-316](../../../../crates/app_host_native/src/factory.rs#L282)).
  Production sites: seven. `roym.rs` :157, :228, :273, :319, :361, :403, and
  [dual_build_fixture.rs:68](../../../../crates/substrate/src/runtime/dual_build_fixture.rs#L68).
  The existing test `every_production_factory_in_runtime_is_wired_with_call_all`
  ([runtime/tests.rs:87-117](../../../../crates/substrate/src/runtime/tests.rs#L87))
  asserts exactly 7 `NativeHostFactory::new(` sites and the same number of
  `.with_call_all(`. The `app_host_native` parity harness builds its own
  engine and factory
  ([helpers.rs:648](../../../../crates/app_host_native/tests/dual_build_parity/helpers.rs#L648),
  [:731](../../../../crates/app_host_native/tests/dual_build_parity/helpers.rs#L731));
  that file is 842 lines and capped at 842 in `xtask/oversized-test-files.txt:1`.
  `RouteHandlerDeps { .. }` is written out in 12 places: one production
  ([runtime/router.rs:464](../../../../crates/substrate/src/runtime/router.rs#L464)),
  `route_handler.rs` itself, and test files in `crates/router/tests/` and
  `crates/coordinator_iroh/tests/common/nodes.rs`. The `roym_web` parity
  harness builds its factories at
  [helpers.rs:1216](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L1216)
  and the WASM engine at [:1108](../../../../crates/roym_web/tests/dual_build_parity/helpers.rs#L1108).
- `check-roym-deps` ([xtask/src/main.rs:154-214](../../../../xtask/src/main.rs#L154))
  needs no change: `roym_transaction` reaches the bus through
  `syneroym-app-host`, which is already allowed.

### 3.5 The Hub and the demo

- The Hub has no timer, no `EventSource` and no WebSocket. It refreshes a
  thread only when the screen opens or after the person acts
  (`renderMessages` [messages.ts:109](../../../../crates/roym_web/ui/src/screens/messages.ts#L109),
  `reloadList` [:221](../../../../crates/roym_web/ui/src/screens/messages.ts#L221),
  `renderThread` [:236](../../../../crates/roym_web/ui/src/screens/messages.ts#L236),
  `fillHistory` [:337](../../../../crates/roym_web/ui/src/screens/messages.ts#L337)).
  Groups: `renderGroups` ([groups.ts:195](../../../../crates/roym_web/ui/src/screens/groups.ts#L195)),
  history at [:612](../../../../crates/roym_web/ui/src/screens/groups.ts#L612).
- Tabs: `renderTabs` / `show` ([main.ts:120-174](../../../../crates/roym_web/ui/src/main.ts#L120)).
  Each tab gets a fresh container; an old screen's container is detached, so
  `isConnected` tells a screen it is gone.
- Hub e2e: [roym-hub.spec.ts](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts)
  (1 245 lines, last case "40." at :1214). Case 10 (:361) asserts a sent
  message never shows `delivered` early; it must still pass.
- Demo: [20-roym-chat.spec.ts](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts),
  two reopen loops at :62-67 and :77-89 with the comments "The Hub does not
  update live" and "The Hub fetches the list again only when the screen is
  opened".

### 3.6 What no code does yet

- Nothing records an event. `EventBus`: not found.
- The MQTT broker (`crates/mqtt_broker`) is pub/sub, but a service can subscribe
  to another service's topics (`namespace_topic`,
  [mqtt_broker/lib.rs:70](../../../../crates/mqtt_broker/src/lib.rs#L70)), it
  has no app scope and no catch-up. A5 does not use it.
- `roymctl` has no command that follows a feed. A5 adds none.

## 4. Behaviour-change audit

Every change below is listed. Everything not listed stays the same.

### Wire changes

| # | Change | Who sees it |
| --- | --- | --- |
| W1 | New path `GET /_syneroym/events` on every service hostname behind the gateway. With a session of the app's owner and `Accept: text/event-stream`: `200`, `Content-Type: text/event-stream`, a stream of frames (D-A5-8). The checks run in this order: no bus (coordinator mode) `404`; no session `401`; not the owner of the service (or the service has no owner) `403`; a service with no app instance `404`; no `Accept` `406`; too many feeds `503` with `Retry-After: 1`. So a caller with no session cannot tell an app service from a plain one. A session exists only when the gateway runs in `login` identity mode; in `open` (the default) and `fixed` mode every request gets `401` ([gateway.rs:443-452](../../../../crates/client_gateway/src/gateway.rs#L443), [auth.rs:84-89](../../../../crates/router/src/route_handler/http/auth.rs#L84)). | Any client. |
| W2 | The gateway forwards `/_syneroym/events` to the service of the `Host` header. Before, it answered `404`. Every other `/_syneroym/` path except `/_syneroym/session*` still gets `404`. | Any client. |
| W3 | New WIT package `syneroym:events@0.1.0` with `record` (written `%record` in WIT, since `record` is a keyword) and `read` (D-A5-6). Every Roym component and the dual-build fixture import it. | Component builders. A component that does not import it links as before. |
| W4 | The router answers `/_syneroym/events` before the service's own route table. A service that declared a route with that exact path loses it. No deployed app declares one (Roym's routes are `/rpc`, `/health`, `/ws`). | App authors. |

No JSON-RPC method, input, output or error changes. The method tables and
shape files stay as they are.

### Behaviour changes that are not wire changes

| # | Change |
| --- | --- |
| B1 | The host conversation crate records `conversation.message` when a message gets its sequence number or a visible message is deleted, and `conversation.delivery` when an outgoing message's state changes (including `retry`) or a peer refuses it (Q1). Every event is recorded after the transaction that stored the change has committed (D-A5-11). |
| B2 | `roym_transaction` records `transaction.card` after every card write and `transaction.booking` after every booking write, except backup import (D-A5-11). A failed record does not fail the method (D-A5-10). |
| B3 | After login the Hub keeps one `EventSource` open while its tab is visible. It closes the feed when the tab is hidden and reopens it from the last cursor when the tab is shown again. After an error status it reopens the feed with a backoff. It refreshes the conversation list, the open conversation and the open group when an event names them, and once on every `ready` or `reload` frame (Q6, D-A5-12). It still refreshes after the person acts, as today. It never shows an error for a feed that is down. |
| B4 | A Hub refresh that follows an event calls `transaction.sync`, which may file a card and record `transaction.card`, which refreshes once more. The second refresh files nothing new, so it records nothing, and the chain ends (D-A5-12). |
| B5 | The bus holds up to 10 000 small events per app instance in memory (D-A5-4). |
| B6 | Each open feed holds one task, one permit and one HTTP/1.1 connection until the client goes away. At most 16 feeds per app instance (D-A5-8). A browser allows about 6 HTTP/1.1 connections per host across all its tabs (`unverified` exact number); because only visible tabs hold a feed, a person must show 6 Hub windows of one hostname at once to reach it (backlog row). |

### Stays the same

All JSON-RPC methods and their shapes. `conversation.changes`. The outbox,
admission and delivery rules. `on-message` and `on-delivery-state` (Roym's
`on-delivery-state` stays a no-op). Roym's `/ws` route and its no-op handlers.
The MQTT broker. The session rules and the gateway bind address (A6 changes
that). Every limit in `AppSandboxRole`.

## 5. Decisions this plan makes

Decisions are numbered D-A5-n. They go into the change doc at close-out
(section 9). Q1 to Q8 are the owner's. The others are made here.

**D-A5-1. Where the bus lives.** `crates/core/src/event_bus.rs` (with
`event_bus/tests.rs`), exported as `syneroym_core::event_bus`. Reason: every
crate that writes or reads events (`conversation`, `router`, `sandbox_wasm`,
`app_host_native`) already depends on `syneroym-core`, and the bus needs the
`EndpointRegistry` that lives there. No new crate. `syneroym-core` already has
`tokio`, `dashmap`, `rand` and `hex`.

**D-A5-2. The bus API.**

```rust
/// One per node process. Every writer and reader shares it.
pub struct EventBus {
    registry: EndpointRegistry,
    run_id: String,                     // 16 lowercase hex chars, random at `new`
    windows: DashMap<String, Arc<Window>>, // app_instance_id -> window
}

pub struct Event {
    pub cursor: Cursor,
    pub kind: String,
    pub service: String,   // service name inside the app, from the registry
    pub subject: String,
    pub item: Option<String>,
    pub at_ms: i64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Cursor { pub run_id: String, pub seq: u64 }

pub enum ReadOutcome {
    /// Every event after the cursor, oldest first, at most `limit`.
    /// `last` is the cursor to send next time (the given one when empty).
    Events { events: Vec<Event>, last: Cursor },
    /// The cursor is from another run, ahead of the window, or older than
    /// the oldest kept event. `now` is the current position to follow from.
    Reload { now: Cursor },
}

pub enum EventError {
    NoAppInstance,            // the writer is in no app instance
    InvalidArgument(String),  // bad kind, subject, item or cursor text
}

impl EventBus {
    pub fn new(registry: EndpointRegistry) -> Arc<Self>;
    /// Tags the event with the app instance and service name of
    /// `service_id` from the registry. Never reads them from the event.
    pub fn record(&self, service_id: &str, kind: &str, subject: &str,
                  item: Option<&str>, now_ms: i64) -> Result<Cursor, EventError>;
    pub fn read(&self, app_instance: &str, after: Option<&Cursor>,
                limit: usize, now_ms: i64) -> ReadOutcome;
    /// The current position: seq `next_seq - 1`, which is 0 only when this
    /// run never recorded an event for `app_instance`. It does not depend on
    /// what is still in the window.
    pub fn head(&self, app_instance: &str) -> Cursor;
    /// Resolves when the window of `app_instance` gets an event after `seq`.
    pub fn changed(&self, app_instance: &str) -> watch::Receiver<u64>;
    pub fn app_instance_of(&self, service_id: &str) -> Option<String>;
}
```

`Window` is a `Mutex<VecDeque<Event>>` plus a `next_seq: u64` (starts at 1),
a `tokio::sync::watch::Sender<u64>` holding the newest seq, and the feed
permit `Arc<Semaphore>` (D-A5-8). `record` takes the lock, evicts (D-A5-4),
pushes, and sends the new seq on the `watch`. `read` with `after = None`
returns `Events { events: [], last: head }`: a new reader starts at the head.
Reload rules in D-A5-3. `read` never blocks; the router waits on `changed`.
The bus reads no clock: callers pass `now_ms`, so tests control time.

**D-A5-3. The cursor.** Text form `<run_id>-<seq>`, for example
`3f9a0c1d22e47b65-1042`. Parsing: split at the last `-`; `run_id` must be 16
lowercase hex chars; `seq` a decimal `u64`. Anything else is
`InvalidArgument` for `events.read` and "reload" for a feed (a browser may
send any old `Last-Event-ID`). A cursor gets `Reload` when its `run_id` is
not this bus's, when `seq` is above the head (`next_seq - 1`), or when events
after `seq` were evicted (`seq + 1 < oldest`). Here `oldest` is the seq of the
front event of the window, or `next_seq` when the window is empty (every event
aged out). So a cursor at the head stays valid after a quiet day, and a new
reader that starts at the head never gets a false reload. `seq` equal to the
head is not a reload: it is "nothing new". Sequence numbers are per app instance, so a
cursor from one app read against another app is either a reload or a gap,
never another app's events (the window is chosen by app instance, not by
cursor).

**D-A5-4. The window.** Constants in `event_bus.rs`:
`MAX_WINDOW_EVENTS = 10_000`, `WINDOW_AGE_MS = 24 * 60 * 60 * 1000`. On
every `record` and `read`, drop events from the front while the window is
longer than the cap or the front event is older than `now_ms - WINDOW_AGE_MS`.
The window of an undeployed app instance stays until restart (backlog row).
Memory bound: `kind` ≤ 64 bytes, `subject` and `item` ≤ 128 bytes, `service`
≤ 64, so one event is under 512 bytes and one window under about 5 MiB.

**D-A5-5. What a writer may write.** `kind`: 1 to 64 bytes, lowercase ASCII
letters, digits, `.` and `-`, starting with a letter. `subject`: 1 to 128
bytes. `item`: absent, or 1 to 128 bytes. `subject` and `item` must have no
ASCII control characters. Anything else is `InvalidArgument` with the field
name (`"kind"`, `"subject"`, `"item"`). The host does not check that a kind
belongs to the writer: an app can only write into its own window, and an
event is a hint, so a wrong kind from one's own app harms only that app.

**D-A5-6. The WIT.** New file `crates/wit_interfaces/wit/events/events.wit`:

```wit
package syneroym:events@0.1.0;

/// Short notes that something changed in this app instance. An event names
/// what changed; it never carries content. A reader fetches the content with
/// the app's normal methods. The host keeps the newest events of each app
/// instance in memory and loses them on restart.
interface events {
    variant events-error {
        /// The calling service is not part of an app instance, so its
        /// events would have no window.
        no-app-instance,
        /// A field is empty, too long, or has a character it may not have.
        /// The string names the field and the rule.
        invalid-argument(string),
        /// The call is read-only (for example an access-check call), so it
        /// may not record.
        read-only,
    }

    record event {
        /// Opaque. Pass it back to `read` to get the events after it.
        cursor: string,
        /// What changed, for example `transaction.card`. Lowercase letters,
        /// digits, `.` and `-`, at most 64 bytes.
        kind: string,
        /// The service of this app that recorded it. Set by the host.
        service: string,
        /// The thing a reader loads again, for example a conversation id.
        subject: string,
        /// The one row inside `subject` that changed, if there is one.
        item: option<string>,
        /// The node's clock when the event was recorded, in Unix ms.
        at-ms: s64,
    }

    variant read-result {
        /// The events after the cursor, oldest first, and the cursor to
        /// pass next time.
        events(tuple<list<event>, string>),
        /// The cursor is too old, from before a restart, or unknown. The
        /// string is the current cursor. Load the state again with the
        /// normal methods, then read from it.
        reload(string),
    }

    /// Records one event in the caller's app instance. Returns its cursor.
    /// The host takes the app instance and `service` from its own records.
    /// `record` is a WIT keyword, so the name is escaped; the Rust and
    /// guest name is still `record`.
    %record: func(kind: string, subject: string, item: option<string>)
        -> result<string, events-error>;

    /// Reads the events of the caller's app instance after `after`, at most
    /// `limit` (the host caps it at 500). With no cursor, or with `limit` 0,
    /// returns no events and the current cursor (a cursor that would get
    /// `reload` still gets `reload`). Never waits.
    read: func(after: option<string>, limit: u32)
        -> result<read-result, events-error>;
}

/// No guest export, so one world serves the guest and host views alike, as
/// `invocation-import` does.
world events-import {
    import events;
}
```

`READ_LIMIT_CAP = 500` in `event_bus.rs`. `EventBus::read` with `limit = 0`
follows the same rule as the WIT: no events, `last` is the given cursor (or
the head with no cursor), and the reload rules of D-A5-3 still apply.

Before Step 3 is done, check the text parses: `wasm-tools component wit
crates/wit_interfaces/wit/events` (or the first `cargo build`).

**D-A5-7. The host capability.** Host bindings `crates/wit_interfaces/src/events_host.rs`
(`bindgen!` world `events-import`, `imports: { default: async }`, the same
shape as `invocation_host.rs`); guest bindings `crates/wit_interfaces/src/events.rs`
behind a new feature `events` (added to `default` and to `syneroym-app-host`'s
feature list). In `sandbox_wasm`: new file
`host_capabilities/capabilities_events.rs` with `impl events::Host for
HostState`. `HostState` gets `event_bus: Option<Arc<EventBus>>` and
`with_event_bus`. `record` refuses `read-only` when `self.read_only`, refuses
`no-app-instance` when `app_instance_id` is `None` or there is no bus, then
calls `bus.record(&self.component_id, ...)` (the bus tags from the registry
by service id; `app_instance_id` is only the early check). `read` uses
`self.app_instance_id`. The engine gets `event_bus: OnceLock<Arc<EventBus>>`
and `set_event_bus`, the same two-step wiring `conversation` uses, so
`AppSandboxEngine::init` keeps its signature (A3 P20 had 73 callers).
`build_store_and_instantiate` passes it with `with_event_bus`. The linker adds
`events::add_to_linker` after `invocation` ([init.rs:410](../../../../crates/sandbox_wasm/src/engine/init.rs#L410)).

The logic of both functions lives once, in two `HostState` methods
(`record_event`, `read_events`), which both the WIT impl and the native host
call. The native host converts types like it does for `call_all`.

**D-A5-8. The client feed.** In the router, new file
`crates/router/src/route_handler/http/events.rs`, `HttpHandler::handle_event_feed`.
In `try_handle_http_request`, before the blob check: `if method == GET &&
path == EVENT_FEED_PATH` (`"/_syneroym/events"`, a `pub const` in
`syneroym_core::event_bus` that the gateway also reads). In order:

1. No bus in this router (coordinator mode): `404`.
2. The caller is not a session caller (no `SESSION_CALLER_MARKER` claim):
   `401`. A peer over Iroh never has a session, so it never reads a feed.
   A session exists only in the gateway's `login` identity mode (W1).
3. `registry.owner_of(service_id) != Some(caller.caller_did)`: `403`. This
   includes a service with no owner (`owner_of` is `None`).
4. `bus.app_instance_of(preamble.service_id)` is `None`: `404` "this service is
   not part of an app". This check comes after the session and owner checks,
   so a caller with no session cannot tell an app service from a plain one.
5. `Accept` does not contain `text/event-stream`: `406`.
6. Take a permit from the window's `Semaphore` (`MAX_FEEDS_PER_APP_INSTANCE
   = 16`) with `try_acquire_owned`; none free: `503`, `Retry-After: 1`.
7. The start cursor: the `Last-Event-ID` header if present, else the `after`
   query parameter, else none.
8. Answer `200`, `Content-Type: text/event-stream`, `Cache-Control: no-cache`,
   and a body from `stream::unfold`. The stream state owns the permit, the
   `watch` receiver and a small `FeedGuard`. `FeedGuard::new` increments the
   gauge `substrate.events.feeds_open`; its `Drop` decrements it. So when the
   client goes away, hyper drops the body, and the permit and the gauge are
   both released (the `handle_messaging_sse` shape, plus the guard).

The body's frames:

- First: with no start cursor, `event: ready`, `id: <head>`, `data:
  {"cursor":"<head>"}`. With a start cursor that reads `Events`, those events
  (below) and no `ready`. With `Reload`, `event: reload`, `id: <now>`, `data:
  {"cursor":"<now>"}`.
- `ready` means "you may have missed something before this point". A client
  that gets `ready` loads its state once, the same as on `reload`. This closes
  the gap between drawing a screen and the feed being connected.
- Each event: `event: change`, `id: <cursor>`, `data:` one line of JSON
  `{"cursor","kind","service","subject","item","at_ms"}` (`item` is `null`
  when absent).
- Every read of the feed (the first one and each one after a change) loops:
  it calls `bus.read(.., READ_LIMIT_CAP, ..)` and sends the frames, and calls
  it again while a read returns exactly `READ_LIMIT_CAP` events. It waits only
  after a read returns fewer. So a client 9 000 events behind gets all of them
  at once, not 500 per later change.
- After that, the loop waits on `changed()` or a 25 s timer. On a change it
  reads after its last cursor as above. If the reader fell behind the window,
  it sends `reload` and continues from `now`. On the timer it sends the comment
  line `: keep-alive` so a dead connection is found and the permit freed.

Metrics (the `metrics` crate, like `substrate.proxy.call_all.*`):
counter `substrate.events.recorded`, counter `substrate.events.refused` with
label `reason` (`no-app-instance`, `invalid-argument`, `read-only`), gauge
`substrate.events.feeds_open`, counter `substrate.events.reloads`.

**D-A5-9. The gateway.** [gateway.rs:411](../../../../crates/client_gateway/src/gateway.rs#L411)
becomes: a `/_syneroym/` path gets `404` unless it starts with
`/_syneroym/session` or its path part (before `?`) equals
`EVENT_FEED_PATH`. The events path then resolves by `Host` like any request.
One pure function `is_forwarded_syneroym_path(path) -> bool` with unit tests
replaces the inline check.

**D-A5-10. `AppEvents` in `syneroym-app-host`.**

```rust
/// Mirrors `syneroym:events/events@0.1.0`.
pub trait AppEvents {
    fn record_event(&self, kind: String, subject: String, item: Option<String>)
        -> impl Future<Output = Result<String, EventsError>> + Send;
    fn read_events(&self, after: Option<String>, limit: u32)
        -> impl Future<Output = Result<ReadResult, EventsError>> + Send;
}
```

Added to the `AppHost` supertrait and its blanket impl. Types re-exported in
`app_host/src/types.rs` from the guest WIT module. Implementers: `GuestHost`
(calls the import), `NativeAppHost` (locks the state and calls the shared
`HostState` methods), the `roym_core` test host (returns `Ok` with a fixed
cursor; it records nothing). `NativeHostFactory` gets
`with_event_bus(self: Arc<Self>, Arc<EventBus>) -> Arc<Self>` with a
`OnceLock`, like `with_call_all`; `build_host_state` passes it. A factory with
no bus answers `no-app-instance`, the same as a standalone WASM service.

A Roym service treats an event as a hint: `roym_transaction` ignores the
result of `record_event` with a comment that says why ("the row is stored
either way; a missed event only delays a client's refresh"). Roym has no
logging interface (backlog row from A4).

**D-A5-11. Writers and fields.**

| Kind | Writer | When | `subject` | `item` |
| --- | --- | --- | --- | --- |
| `conversation.message` | host `crates/conversation` | a row gets its sequence number (send, group send, accepted inbound, accepted group entry); a visible row is deleted (local, peer request, group entry) | conversation id | message id |
| `conversation.delivery` | host `crates/conversation` | `notify_state` (every outgoing state change), `retry`, and a peer's refusal of an outgoing message (`refused` is set) | conversation id | message id |
| `transaction.card` | `roym_transaction` | after each `cards` write listed in 3.1, except import | `CardRow.conversation` | `CardRow.message_id` |
| `transaction.booking` | `roym_transaction` | after each `bookings` write listed in 3.1, except import | `BookingRow.conversation` | `BookingRow.agreement` |

Backup import records nothing: the person who imports reloads, and an import
of thousands of rows would only push the window. A held or dropped inbound
message records nothing (it is not visible). A group membership or name entry
records nothing in v1 (no v1 kind; backlog row). A change of one group
member's state (`set_recipient_state`) records nothing: no screen shows it,
and the message's own state change records `conversation.delivery` when the
last member is done (backlog row).

In `crates/conversation`: `ConversationService` gets `event_bus:
OnceLock<Arc<EventBus>>` and `set_event_bus`, and one private method
`announce(&self, svc, kind, conversation, message)` that does nothing when no
bus is set. `notify_state` gets a `conversation_id` argument and calls
`announce` for `conversation.delivery`; its eight callers pass `msg.conversation_id`
(or, at `update_delivery_state`, read it with `store.get_message`).
`apply_admission` returns `Result<bool>`: `true` when this call gave the row its
sequence number. A helper `admit_and_announce(&self, store, svc, msg, admission,
now)` replaces the six `apply_admission` call sites listed in 3.1 and calls
`announce` on `true`. The drop at `outbox.rs:543` keeps calling
`apply_admission` directly.

**The commit rule.** `announce` is called only after the transaction that
stored the change has returned `Ok`, and never while the store's connection
lock is held. Code inside a `transaction(|tx, ..| ..)` closure never calls
`announce`. It hands out what changed, and the caller announces after the
`transaction` call returns, the way `ingest_verified` already hands out
`inserted` and `group_id_to_apply`. The signature changes this needs:

- `handle_inbound_deletion_request` returns `Result<Option<String>>`: the
  target message id when a visible row changed (rule below), else `None`. In
  `ingest_verified` the closure stores it in a local `deleted`, and after
  `transaction` returns `announce(.., "conversation.message",
  &payload.conversation_id, Some(&id))` runs.
- `handle_inbound_refusal_notice` returns `Result<Option<String>>`: the target
  message id when the `UPDATE` changed a row. `ingest_verified` announces
  `conversation.delivery` for it after `transaction` returns. `record_refusal`
  (its own lock, called at transport.rs:287) is followed by
  `announce(.., "conversation.delivery", ..)` when it returns `Ok(true)`.
- `apply_group_deletion_request` returns `Result<Option<String>>` the same
  way. Its caller at group/entry.rs:154 passes it out with the entry result,
  and the group-entry path in `transport/group_sync.rs` announces after the
  transaction returns.
- Sends: `enqueue_direct` (lib.rs:332) announces after
  `insert_outgoing_and_enqueue` returns, and `send_group` (group.rs:452) after
  the transaction that calls `record_outgoing_group_message` returns.
- Local delete (host_impl.rs:300, store call at :333) announces after
  `delete_message` returns, when it returns `Some(true)` (rule below).
  `delete_message` passes out the `Option<bool>` of `delete_message_conn`.

**Which deletes are visible.** `delete_message_conn` returns `Ok(true)` for
any row it finds (3.1). It gets a way to say whether the row was visible
before: it returns `Result<Option<bool>>`, `None` for "no row", `Some(true)`
when the row was outgoing (outgoing rows are always shown) or an inbound row
with `admission = 'accepted'`, `Some(false)` for an inbound row that was
`undecided`, `held` or `dropped`. Only `Some(true)` is announced. Its callers
are `delete_message` (store/message.rs:371), `handle_inbound_deletion_request`
(:461) and the group path (group/entry.rs:237); a caller that only needs
"found or not" uses `.is_some()`.

**How the tests check the rule** (7.3). `ConversationService` gets one
`#[cfg(test)]` field, `announce_probe: Option<Box<dyn Fn(&str, &str) + Send +
Sync>>`, which `announce` calls before it records. The tests set a probe that
asserts `store.conn_is_free()` (a new `#[cfg(test)]` method on
`ConversationStore`, `self.conn.try_lock().is_ok()`: the lock is free, so no
transaction is open on this thread) and then reads `conversation.history` and finds the
change. If `announce` moved inside a transaction, the probe's `try_lock` fails
at once, on the same thread, every time. No timing is involved. The tests
use the default current-thread `#[tokio::test]` runtime. The lock is a
`std::sync::Mutex` that no code holds across an `.await`, so on one thread no
other task can hold it while `announce` runs, and the probe never fails by
chance.

In `roym_transaction`: two functions in `app.rs`, `put_card(host, &CardRow)`
and `put_booking(host, &BookingRow)`, each `put_row` then `record_event`. The
eleven card writes and five booking writes in 3.1 call them. Import keeps
`put_row`.

**D-A5-12. The Hub.** New file `crates/roym_web/ui/src/events.ts`:

- `startFeed()` is called once from `renderTabs`, after login. Login and
  logout both reload the page ([main.ts:102](../../../../crates/roym_web/ui/src/main.ts#L102),
  :225, :251), so the feed starts and stops with the page and needs no logout
  hook.
- **One feed per visible tab.** `startFeed` opens the `EventSource` only while
  `document.visibilityState === "visible"`. On `visibilitychange` to hidden it
  closes it; back to visible it opens a new one. Reason: the gateway speaks
  HTTP/1.1 only, and a browser allows about 6 connections per host across all
  its tabs. A feed per open tab would let 6 background tabs block every request
  of the 7th. A hidden tab needs no live update; it catches up when shown.
  (This is simpler than electing one leader tab with Web Locks and
  `BroadcastChannel`, and it gives the same bound in practice.)
- **Reopen after an error status.** `EventSource` reconnects by itself only
  after a network error. After a non-200 answer (a `404` while a restarted
  node registers its apps, `401`, `502` from the gateway, `503` at the cap) it
  sets `readyState` to `CLOSED` and stops. On `error` with `readyState ===
  EventSource.CLOSED`, `events.ts` closes it and opens a new one after a
  backoff: 1 s, doubling to at most 30 s, with up to 20 % random jitter. The
  backoff resets after a frame arrives.
- **The cursor across new objects.** `events.ts` keeps the last cursor it saw
  (the `id` of the newest `ready`, `change` or `reload` frame). A new
  `EventSource` cannot send `Last-Event-ID`, so it opens
  `/_syneroym/events?after=<cursor>` (D-A5-8 step 7 accepts it). With no
  cursor yet, it opens the path with no query.
- The Hub never shows an error for a feed that is down. The screens work as
  they do today (refresh on open and on action).
- `onFeed(listener)` adds a listener and returns a function that removes it.
  A listener gets `{kind, subject, item}` for `change`, and `{reload: true}`
  for both `ready` and `reload` (D-A5-8: `ready` means "you may have missed
  something").
- Changes are passed on after 150 ms with no newer change for the same
  `subject` (one refresh per burst).
- Messages screen: `renderMessages` registers a listener. For a change whose
  `subject` is the open conversation, it calls that thread's `fillHistory`; for
  any `conversation.*` change, it calls `fillList` with the open conversation
  selected. On `{reload: true}`, both. The listener removes itself when the screen's
  container is no longer connected (`!box.isConnected`).
- Groups screen: the same for the open group's history.
- `refiller` already drops a refresh that finishes after a newer one, so a
  burst of events never draws two copies.
- The compose box is outside the refilled element, so a refresh never clears
  what the person is typing.

Loop bound (B4): a refresh calls `transaction.sync`, which files only cards it
has not filed (`sync.rs:217` checks `exists`), so the second refresh records
nothing.

**D-A5-13. No config.** The window, the field limits, the feed cap and the
keep-alive are constants, not `AppSandboxRole` fields. A3 R1 and R7 showed
that a config value that is never read or is clamped silently is worse than a
constant. A backlog row records making them config when an operator needs it.

## 6. Step-by-step changes

Each step builds and its tests pass. The whole branch is one PR. Rebuild the
WASM components (`mise run build:roym`) after every step that changes a WIT
file, `syneroym-app-host` or a Roym crate (Steps 3, 4, 6).

### Step 1 — Commit this plan

`docs(planning): plan M08A slice A5`. Status row A5: "Plan written".

### Step 2 — The bus

Behaviour does not change; nothing records yet.

1. `crates/core/src/event_bus.rs` and `event_bus/tests.rs` (D-A5-1 to D-A5-5,
   `EVENT_FEED_PATH`, the constants). `pub mod event_bus;` in `core/src/lib.rs`.
2. Unit tests (section 7.1).

Check: `cargo nextest run -p syneroym-core event_bus`; clippy clean.

### Step 3 — The WIT, the host capability and `AppEvents`

1. `events.wit` (D-A5-6), `events.rs`, `events_host.rs`, the `events` feature.
2. `sandbox_wasm`: `capabilities_events.rs`, the `HostState` field and
   builder, the engine `OnceLock` and setter, the linker line (D-A5-7).
3. `syneroym-app-host`: `AppEvents`, the supertrait, the types,
   `GuestHost`'s impl (D-A5-10).
4. `app_host_native`: `NativeAppHost` impl, `with_event_bus`,
   `build_host_state`.
5. `roym_core` test host impl.
6. Add `import syneroym:events/events@0.1.0;` to the six Roym worlds and to the
   dual-build fixture world, with `wit/deps/events` (a symlink, like
   `proxy`) and the `"syneroym:events"` entry in each `Cargo.toml` component
   metadata.
7. Host capability tests (section 7.2).

Check: `cargo build --workspace`; `mise run build:roym` builds all six
components and the fixture; `cargo nextest run -p syneroym-sandbox-wasm
events`; the existing parity suites still pass (`roym_web` and
`app_host_native`).

### Step 4 — Wire the bus in the substrate

1. `runtime/handles.rs`: `SharedNodeHandles` holds `event_bus: Arc<EventBus>`,
   made once from the node's `EndpointRegistry`, with a getter.
2. `runtime/router.rs`: `engine.set_event_bus(..)` next to the other engine
   wiring; `conversation.set_event_bus(..)` next to `set_service_proxy`;
   `RouteHandlerDeps.event_bus: Option<Arc<EventBus>>` (`None` in
   `new_coordinator`). Every other `RouteHandlerDeps { .. }` literal (3.4: the
   test files in `crates/router/tests/` and
   `crates/coordinator_iroh/tests/common/nodes.rs`) gets `event_bus: None`, one
   line each.
3. `.with_event_bus(shared.event_bus().clone())` beside each of the seven
   `.with_call_all(..)` calls: the six in `runtime/roym.rs` and
   `runtime/dual_build_fixture.rs:68`. Do not add a new count test. Extend the
   existing `every_production_factory_in_runtime_is_wired_with_call_all`
   (runtime/tests.rs:87) with a third count, `.with_event_bus(`, that must
   equal the factory count (7), and rename it
   `every_production_factory_in_runtime_is_fully_wired`.
   The `app_host_native` parity harness (helpers.rs:648, :731) gets no bus: its
   fixture records no event, so its `no-app-instance` answer is never seen, and
   that file is at its cap (842).
4. Parity harness: give each build its own bus, wired like production. Because
   `helpers.rs` is capped at 1 429 lines, first move `TestWasmServiceProxy` and
   `TestNativeServiceProxy` (:775-928) unchanged into a new sibling file
   `dual_build_parity/test_proxies.rs`, declared the way `normalize` is at
   :763-765 (`#[path = "test_proxies.rs"] pub(crate) mod test_proxies;
   pub(crate) use test_proxies::*;`). Then add the
   bus fields to `Harness` and the two wiring calls. Set the oversized-list
   entry for `helpers.rs` to its new, lower line count.

Check: substrate builds with and without `--features roym`; the parity suite
passes; the factory count test passes.

### Step 5 — The conversation host records

1. `crates/conversation` (D-A5-11): `set_event_bus`, `announce` (with the
   `#[cfg(test)]` probe), `admit_and_announce`, `apply_admission` returns
   `bool`, `notify_state` takes the conversation id, the send, delete, refusal
   and retry paths. Apply the commit rule: change the return types of
   `handle_inbound_deletion_request`, `handle_inbound_refusal_notice`,
   `apply_group_deletion_request` and `delete_message_conn` /
   `delete_message`, and announce only after each `transaction` call returns.
2. Tests in `crates/conversation` (section 7.3).
3. One parity test that sends a message in both builds and compares the
   recorded `(kind, service, subject, item)` lists (section 7.4).

Check: `cargo nextest run -p syneroym-conversation`; parity `events` tests.

### Step 6 — `roym_transaction` records

1. `put_card`, `put_booking`; replace the eleven and the five call sites.
2. Parity tests in `dual_build_parity/events.rs` (section 7.4).
3. Rebuild WASM.

Check: `cargo nextest run -p syneroym-roym-transaction`;
`cargo nextest run -p syneroym-roym-web --test dual_build_parity events`;
`method_contract` unchanged.

### Step 7 — The client feed

1. Router `events.rs` (D-A5-8), the path check in `try_handle_http_request`,
   the `event_bus` field in `RouteHandlerInner` and `RouteHandlerDeps`.
2. Gateway (D-A5-9).
3. Router tests, gateway tests, and the substrate e2e test (sections 7.5 and
   7.6).

Check: `cargo nextest run -p syneroym-router events`;
`cargo nextest run -p syneroym-client-gateway`;
`cargo nextest run -p syneroym-substrate --test roym_event_feed_e2e`.

### Step 8 — The Hub and the demo

1. `events.ts`, `events.test.ts`, `main.ts`, `screens/messages.ts`,
   `screens/groups.ts` (D-A5-12).
2. Hub e2e: new case "41." (section 7.7).
3. Demo story (Q7, section 7.7). Run `mise run demo`.

Check: `(cd crates/roym_web/ui && npm test)`; `mise run test:e2e`.

### Step 9 — Docs and close-out

Section 8 (backlog) and section 9 (living docs, change doc, ADR-0027).
`task.md` result line, `status.md` row.

### Step 10 — Finish

- `cargo +nightly-2026-04-06 fmt --all`, clippy clean, the import clean-up
  pass over every edited file.
- `cargo dupes report` over the diff. The `HostState` methods are the only
  copy of the record and read logic. If the router feed's permit and
  `stream::unfold` code matches `handle_messaging_sse`, move the shared part
  into one helper in `streaming.rs`.
- `mise run verify` as a background job, no `--skip`.
- Run the router feed tests, the substrate e2e test, Hub case 41 and the demo
  story 20 times in a row. Record the result in `status.md`.
- Open the PR (CLA line verbatim).

## 7. Tests

### 7.1 Bus unit tests (`crates/core/src/event_bus/tests.rs`)

- `a_recorded_event_is_tagged_with_the_writers_app_instance_and_service`
- `a_service_outside_any_app_instance_cannot_record`
- `one_app_instance_never_reads_another_apps_events` (two app instances, the
  same cursor text)
- `read_without_a_cursor_returns_nothing_and_the_head`
- `read_after_a_cursor_returns_the_later_events_oldest_first`
- `read_at_the_head_returns_nothing_and_is_not_a_reload`
- `the_limit_caps_one_read_and_the_next_read_continues`
- `the_window_keeps_the_newest_ten_thousand`
- `events_older_than_a_day_are_dropped_on_read_and_on_record`
- `a_cursor_older_than_the_window_is_a_reload_with_the_head`
- `a_cursor_from_another_run_is_a_reload`
- `a_cursor_ahead_of_the_head_is_a_reload`
- `malformed_cursor_text_is_invalid_argument` (empty, no `-`, short run id,
  upper-case hex, non-number seq)
- `kind_subject_and_item_limits_name_the_field` (each limit, plus a control
  character)
- `changed_wakes_after_a_record`
- `after_all_events_age_out_a_new_reader_does_not_get_a_false_reload`
- `after_all_events_age_out_the_old_head_cursor_is_not_a_reload`
- `read_with_limit_zero_returns_no_events_and_the_cursor`

Mutation that must fail the tests: tag from a field of the event instead of
the registry (the first test); evict with `>=` instead of `>` (the window
tests); treat `seq == head` as reload; compute `head` or `oldest` from the
window instead of `next_seq` (the two age-out tests).

### 7.2 Host capability tests (`sandbox_wasm`)

In `host_capabilities/tests/tests_events.rs`, on a `HostState` built like the
`tests_call_all` tests:

- `record_in_a_read_only_call_is_refused`
- `record_without_an_app_instance_is_refused`
- `record_without_a_bus_is_no_app_instance`
- `read_reads_only_the_callers_app_instance`
- `read_caps_the_limit_at_five_hundred`

### 7.3 Conversation host tests (`crates/conversation`)

With the crate's `test_support` and a bus:

- `sending_a_message_records_one_message_event`
- `an_accepted_inbound_message_records_one_event_after_it_is_visible` (the
  `announce_probe` of D-A5-11 asserts the connection lock is free and reads
  `conversation.history`, which already has the message)
- `a_held_or_dropped_inbound_message_records_nothing`
- `a_racing_second_accept_records_no_second_event`
- `every_delivery_state_change_records_a_delivery_event` (pending, deposited,
  delivered, failed, and retry back to pending)
- `deleting_a_visible_message_records_a_message_event`
- `a_deletion_request_is_visible_to_history_when_its_event_arrives` (a peer's
  deletion request through `ingest_verified`, with the same probe)
- `an_undecided_message_deleted_by_its_author_records_nothing`
- `a_group_deletion_entry_is_visible_to_history_when_its_event_arrives`
- `a_peer_refusal_records_a_delivery_event` (both the refusal notice and the
  refused acknowledgement at transport.rs:287)

Mutation that must fail these tests: move any `announce` call into the
`transaction` closure before it returns (the probe's `try_lock` fails). The
probe runs on the same thread as `announce`, so this check needs no timing.
- `an_accepted_group_entry_records_a_message_event`
- `import_records_nothing`
- `no_bus_set_records_nothing_and_changes_nothing`

### 7.4 Parity tests (`crates/roym_web/tests/dual_build_parity/events.rs`)

Each test runs both builds and compares the full list of recorded
`(kind, service, subject, item)` (ids normalized per build, A2c R13). A test
that reads one build is the A2c R1 mistake.

- `sending_a_message_records_the_same_event_in_both_builds`
- `a_request_card_records_a_card_event_in_both_builds`
- `accepting_an_agreement_records_card_and_booking_events_in_both_builds`
  (uses `deal_fixtures`)
- `syncing_an_inbound_card_records_one_card_event_and_a_second_sync_none`
  (the loop bound, B4)
- `a_transaction_import_records_no_event`

### 7.5 Router and gateway tests

Router (`crates/router/src/route_handler/http/events/tests.rs`, with the
router's existing HTTP test helpers):

- `the_feed_needs_a_session` (401)
- `another_person_gets_403`
- `a_service_with_no_app_instance_gets_404`
- `a_request_without_event_stream_accept_gets_406`
- `the_owner_gets_ready_then_each_change`
- `last_event_id_wins_over_the_after_parameter`
- `a_too_old_cursor_gets_reload_then_live_changes`
- `the_feed_of_one_app_never_carries_another_apps_event`
- `the_seventeenth_feed_gets_503_and_a_closed_feed_frees_its_permit`
- `a_quiet_feed_sends_a_keep_alive` (with `tokio::time::pause`)
- `a_peer_without_a_gateway_session_gets_401` (non-gateway caller)
- `a_stranger_cannot_tell_an_app_service_from_a_plain_one` (no session: `401`
  for both; check order of D-A5-8)
- `a_reader_far_behind_gets_all_events_before_it_waits` (1 200 events, a
  cursor at the start: every event arrives with no new record)
- `a_closed_feed_lowers_the_open_feeds_gauge` (the `FeedGuard` drop)

Gateway (`gateway/tests.rs`): `is_forwarded_syneroym_path` for `/_syneroym/events`,
`/_syneroym/events?after=x`, `/_syneroym/session/login`, `/_syneroym/other`,
`/_syneroym/eventsx`.

### 7.6 Substrate e2e (`crates/substrate/tests/roym_event_feed_e2e.rs`)

Two `RoymNode`s (`common/roym.rs`), `serial_guard`, dynamic ports. The feed
is read with `reqwest` streaming the response body.

- `an_inbound_message_reaches_the_owners_feed_and_changes_shows_it`: A opens
  its feed; B sends A a message; A's feed has a `conversation.message` change
  whose `subject` is the conversation; right after, `conversation.changes` on A
  returns the message (no race, Q1).
- `a_sent_message_records_a_message_event_within_one_second`: A sends; the
  event arrives less than 1 s after the RPC returns (OF-8). Measured from the
  RPC return to the frame, in this test alone.
- `delivery_events_follow_the_state_to_delivered`
- `a_reconnect_with_the_last_cursor_gets_the_missed_events` (close the feed,
  send two messages, reopen with `Last-Event-ID`)
- `a_cursor_from_before_a_restart_gets_reload` (`RoymNode::restart`)

### 7.7 Hub and demo

- Vitest `events.test.ts` with a fake `EventSource` and a fake
  `document.visibilityState`:
  `a_burst_for_one_subject_gives_one_call`,
  `a_listener_whose_screen_is_gone_is_removed`,
  `reload_reaches_every_listener`,
  `a_ready_frame_refreshes_every_listener_once`,
  `changes_for_another_subject_do_not_refresh_the_open_thread`,
  `a_closed_feed_is_opened_again_with_the_last_cursor` (the fake closes with
  `readyState = CLOSED`; the new URL has `?after=<last id>`; the delay follows
  the backoff),
  `a_hidden_tab_closes_its_feed_and_reopens_from_the_last_cursor_when_shown`,
  `a_feed_never_opens_while_the_tab_is_hidden`.
- `screens/messages.test.ts`: `a_change_for_the_open_conversation_refills_the_thread`.
- Hub e2e, new case in `roym-hub.spec.ts`: "41. messages tab: a message sent
  in one tab appears in a second open tab with no reload" (one node, two
  pages of the same Hub; OF-11 for a local change). Case 10 still passes.
  Case 41 also checks the reopen: the second page uses `page.route` to answer
  the first `/_syneroym/events` request with `503` once; the next sent message
  still appears on that page with no reload.
- Demo story (Q7 (a)): two pages, no reopen loops. Steps: Alice opens her
  conversation and keeps it open; Bob (second page) opens Messages once and
  Alice's message appears there by itself; Bob replies; on Alice's page Bob's
  reply and `delivered` appear by themselves. Waits use `expect(..).toBeVisible`
  with `DELIVERY_WAIT_MS`, not `toPass` around a reopen, and never a 1 s
  expect timeout (a background page may delay the 150 ms debounce timer). If
  the browser hides the page in the background, Alice's feed closes and
  reopens from its cursor when her page comes to the front (D-A5-12), so the
  reply still appears with no reload. Comments say what the viewer sees, not
  "the Hub does not update live".

### 7.8 Not tested

- A feed through the WebRTC browser path (service worker and tunnel). The
  Hub e2e and the demo use the gateway directly.
- A feed through a TLS proxy in front of the gateway (A6).
- 10 000 events of 24 hours in real time (the window tests use the clock
  argument).
- Many real browsers on one node at the feed cap (the router test covers the
  cap).
- The browser's per-host connection limit with many visible tabs. The vitest
  cases cover that hidden tabs hold no feed; no Playwright case opens 7
  visible windows.

### 7.9 Commands to run after each step

```bash
cargo nextest run -p syneroym-core event_bus
cargo nextest run -p syneroym-sandbox-wasm events
cargo nextest run -p syneroym-conversation
mise run build:roym
cargo nextest run -p syneroym-roym-web --test dual_build_parity
cargo nextest run -p syneroym-app-host-native
cargo nextest run -p syneroym-router events
cargo nextest run -p syneroym-client-gateway
cargo nextest run -p syneroym-substrate --test roym_event_feed_e2e
(cd crates/roym_web/ui && npm test)
cargo xtask check-file-lengths && cargo xtask check-lint-suppressions
```

### 7.10 How A5 avoids the kinds of findings earlier slices had

| Earlier finding | The risk in A5 | What this plan does |
| --- | --- | --- |
| A2c R1 (a test reads one build) | parity of events | Every parity test compares the full list from both builds (7.4). |
| A2a R1, A3 R13 (a test that cannot fail) | scoping and window tests | 7.1 names the mutation each key test must catch. |
| A3 P2, R6 (a factory site missed) | `with_event_bus` | Step 4 lists the seven sites and extends the existing count test. |
| A3 P20 (73 callers of `init`) | wiring the engine | A setter, not a new `init` argument (D-A5-7). |
| A3 P7 (WIT doc left to the implementer) | `events.wit` | D-A5-6 gives the full text. |
| A3 R1, R7 (config ignored or clamped) | limits | Constants, no config (D-A5-13). |
| A4 P1, R2 (unbounded memory) | the window and fields | Field caps and window cap (D-A5-4, D-A5-5). |
| A4 P19, R11 (wall-clock asserts in a shared run) | the 1 s criterion | Measured in one e2e test from RPC return to frame, nothing else in the timer (7.6). |
| A4 R6 (oversized entry not lowered) | `helpers.rs` | Step 4 moves code out first and lowers the entry. |
| A2b R8, A4 R3 (why-comments lost) | `apply_admission` call sites | Comments stay beside the code; `admit_and_announce` gets its own why-comment (the race). |
| A2a P15 (demo and helpers missed) | the demo and Hub helpers | Step 8 and 7.7 name them. |
| A3 R10, A4 P22 (living docs false) | four docs | Section 9 names each place. |
| A4 Q6, R7 (`demo:` line rewritten) | the change doc | Section 9 says to add the story, keep the reason text for earlier slices. |

### 7.11 Size risks

Rust files that grow, with their length today:

| File | Today | After (about) |
| --- | --- | --- |
| `crates/app_host_native/src/host.rs` | 712 | 745 |
| `crates/roym_core/src/signing.rs` | 758 | 775 |
| `crates/sandbox_wasm/src/host_capabilities.rs` | 646 | 690 (two methods; move them to `capabilities_events.rs` if it passes 700) |
| `crates/conversation/src/lib.rs` | 493 | 560 |
| `crates/conversation/src/store/message.rs` | 731 | 745 (new return types for delete and refusal, `conn_is_free`) |
| `crates/conversation/src/transport.rs` | 709 | 730 (announce after the `ingest_verified` transaction) |
| `crates/conversation/src/group.rs` | 720 | 730 |
| `crates/conversation/src/group/entry.rs` | 462 | 470 |
| `crates/conversation/src/outbox.rs` | 617 | 625 |
| `crates/roym_transaction/src/app.rs` | 564 | 600 |
| `crates/router/src/route_handler.rs` | 625 | 635 |
| `crates/router/src/route_handler/http.rs` | 332 | 340 |
| `crates/client_gateway/src/gateway.rs` | 663 | 670 |
| `crates/substrate/src/runtime/router.rs` | 673 | 685 |
| `crates/substrate/src/runtime/roym.rs` | 596 | 605 |
| `crates/app_host_native/src/factory.rs` | 638 | 655 |
| `crates/roym_web/tests/dual_build_parity/helpers.rs` | 1 429 (capped) | about 1 290 (after the move) |

New files: `event_bus.rs` about 300, its tests about 350,
`capabilities_events.rs` about 80, router `events.rs` about 220, its tests
about 300, `roym_event_feed_e2e.rs` about 350, `dual_build_parity/events.rs`
about 250. All under 800. `signing.rs` is the closest; if it passes 790, move
the test host's `impl` blocks to `signing/tests/test_host.rs`.

`messages.ts` is 887 lines. The file-length gate covers `.rs` only, but keep
the new code in `events.ts` and add only the listener (about 20 lines) to the
screen.

## 8. Deferred backlog

All rows are in [deferred-backlog.md](../../deferred-backlog.md).

- **Update:** "The event-feed catch-up window is lost on restart" (:367): add
  the cursor form and that a restarted node answers `reload`; link the bus
  file. Keep it open.
- **New rows, section 8 (node lifecycle):**
  - "**The event window of an undeployed app instance stays until restart**":
    `EventBus` has no `forget`. Bounded by the number of app instances
    deployed since start. Target: TBD (when undeploy gets a host hook).
  - "**Event-feed limits are constants**" (window size and age, field caps,
    16 feeds per app instance, 25 s keep-alive). Target: when an operator
    needs another value.
  - "**Each visible Hub tab holds one HTTP/1.1 connection**": the gateway has
    no HTTP/2, and a browser allows about 6 connections per host. A person
    who shows 6 Hub windows of one hostname at once blocks the 7th. Hidden
    tabs hold no feed. Fix options: one leader tab (Web Locks and
    `BroadcastChannel`), or HTTP/2 at a TLS proxy (A6). Target: TBD.
- **New rows, section 10 (product surfaces):**
  - "**Services can only pull events**": no `on-event` push export (Q3).
    Target: M08C, with the first service that reacts to events.
  - "**Group membership and name changes record no event**": the Groups
    screen learns of them only on refresh. Target: when a client needs them.
  - "**A group member's delivery state records no event**": only the
    message's own state does (when the last member is done). No screen shows
    per-member states today. Target: when a screen shows them.
  - "**`roymctl` cannot follow the event feed**". Target: A7 decides (the guide
    uses `curl -N`).
- No new open `TODO` or `FIXME` marker.

## 9. Living docs to update at close-out

Ask for each: is a statement now false or incomplete? Do not cite change docs
or milestone and slice ids in these files. Link ADRs only.

- [docs/system-architecture.md](../../../system-architecture.md):
  - The parts list around :548 ("Two more parts run inside the substrate"):
    add the event bus as a third part, one sentence: what it holds and that it
    is scoped by app instance.
  - "Ingress" (:550-552): one sentence that the router answers
    `/_syneroym/events` itself, as SSE, for the owner of the app.
  - The external API row (:1894): add "SSE event feed".
- [docs/system-requirements-spec.md](../../../system-requirements-spec.md):
  "Ingress Protocol Translation" (:621-623) lists the gateway paths; add
  `GET /_syneroym/events`. Add a short requirement for the event feed under
  the platform requirements (owner-only, per app instance, catch-up by
  cursor), with the next free id.
- [docs/developer-guide.md](../../../developer-guide.md): next to "Auth Service
  Endpoints (`/_syneroym/session/*`)" (:1838), a short "Event feed
  (`/_syneroym/events`)" part: the request, the four frame types, the cursor
  and `Last-Event-ID`, the error codes of W1 in their order, and that the
  feed needs the gateway in `login` identity mode (in `open` and `fixed` mode
  every request gets `401`). The host interface list: add `syneroym:events`. (A7 writes the full client guide; A5 writes the facts.)
- [docs/roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md):
  the Conversation row (:98) and the Transaction row: one line each that the
  service's changes appear on the event feed. Grep "refresh" and "open the
  screen" for statements that say the Hub does not update live.
- [docs/TERMINOLOGY.md](../../../TERMINOLOGY.md): add "event feed" and
  "cursor" if the file defines terms of this kind. Check first.
- [docs/planning/traceability-matrix.md](../../traceability-matrix.md): add
  `[ROY-EVT]` after `[ROY-FED]` (:95): the event feed. Evidence: the tests of
  section 7.
- ADR-0027 "Host event feed" (Q8): context, the decision (D-OF-3, Q1–Q5),
  the rejected options, the consequences.
- Change doc [roym-open-foundation](../../changes/roym-open-foundation/change.md):
  - **Scope:** "The event feed: one per participant node. WebSocket for
    clients outside the node. An internal subscription for services on the
    node." becomes "SSE for clients outside the node. A pull read for services
    on the node." (Q2, Q3).
  - **Acceptance criteria:** fill the Test column of OF-8 to OF-11 from the
    real test names after the code is written (grep them).
  - **Design, "Event feed":** replace "Events are small. Each one names a kind
    ..., an id and a cursor" with the fields of Q4; add the path, the frame
    types and the owner rule; say which kinds the host writes (Q1).
  - **Tasks:** tick "The event feed ...".
  - **Decisions:** add D-A5-1 to D-A5-13 with the same numbers and words, and
    one line with the owner's answers to Q1–Q8 (all (a), 2026-11-03). The
    D-OF-3 line "A5 decides how services subscribe (OF-10)" already has the
    answer (written when the answers were recorded).
  - **Acceptance criteria, OF-10:** reword to "A service on the node can read
    the events of its own app instance with `events.read`, without WebSocket.
    A service receives only the events of its own app instance. A client
    session receives only the events of the app instance of the hostname it
    opens, and only when the person is that app's owner." Keep the id.
  - **Deviations, "Different from the plan":** the conversation kinds are
    written by the host conversation capability (Q1); SSE, not WebSocket
    (Q2); services read the feed with `events.read`, there is no internal
    subscription or push export (Q3, with a link to the backlog row); the
    event has `subject` and `item`, not one `id` (Q4); a client reads one app
    instance per hostname, for its owner only, not "the apps its login
    session may use" (Q5); the feed needs the gateway in `login` identity
    mode. **"Planned wire changes":** W1 to W4.
  - **`demo:`:** add `crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts` as
    a story entry. Keep the existing `none:` reason line for the earlier slices
    as it is (A3 R9, A4 R7).
  - `status` stays `draft` (A6 and A7 remain).
- [task.md](task.md): X4 "Built in" stays A5; add the A5 result line. The
  A5 bullet "the WebSocket path" becomes "the SSE path", the bullet "internal
  subscription for services" becomes "pull read for services
  (`events.read`)", and A7's `websocat` becomes `curl -N`.
- [status.md](status.md): row A5, with the PR, the test counts (bus tests,
  host tests, conversation tests, parity events tests, router tests, e2e
  tests, vitest, Hub case 41) and the 20-run result.

## 10. Exit criteria

A5 is done when all of these are true:

1. The owner has answered Q1 to Q8, and sections 4 to 9 match the answers.
2. `syneroym_core::event_bus` exists with the tests of 7.1, and every
   writer reaches the bus only through `EventBus::record`, which tags by
   `app_context_of`.
3. The four kinds are recorded by the writers in D-A5-11, on both builds
   (parity tests in 7.4 pass).
4. A client with the owner's session reads `/_syneroym/events` on the app's
   hostname through the gateway, gets a change within 1 s of a sent message
   (OF-8), catches up with its cursor or gets `reload` (OF-9), and never sees
   another app's event; another person gets `403` (OF-10, router tests and
   7.6).
5. A service reads its own app's events with `events.read` (OF-10, 7.2).
6. The Hub shows a new incoming message on an open screen with no action
   (OF-11): Hub case 41 and the demo story pass.
7. The demo story has no reopen loop and plays with `mise run demo`.
8. `helpers.rs` (parity) is lower in the oversized list, the duplication
   baseline is correct, and no Rust file passes 800 lines.
9. The change doc, the living docs, the traceability matrix, `task.md`,
   `status.md`, the backlog and ADR-0027 are updated as in
   sections 8 and 9.
10. `src/` changes are only in: `crates/core`, `crates/wit_interfaces`,
    `crates/sandbox_wasm`, `crates/app_host`, `crates/app_host_native`,
    `crates/conversation`, `crates/router`, `crates/client_gateway`,
    `crates/substrate`, `crates/roym_core` (test host), `crates/roym_transaction`,
    the WIT `deps` and component metadata of the six Roym crates and the
    dual-build fixture, and the Hub UI.
11. The router feed tests, the substrate e2e test, Hub case 41 and the demo
    story ran 20 times in a row with no failure.
12. `mise run verify` passes with no `--skip`.
13. One PR.

## 11. What A5 does not do

- No push of events to services (Q3). M08C adds it with its first user.
- No `match` kind (M08F) and no `search` events (D-OF-5).
- No event for group membership, group names or backup import.
- No durable window; a restart answers `reload` (D-OF-4).
- No change to any JSON-RPC method, to `conversation.changes`, or to the
  outbox and admission rules.
- No change to Roym's `/ws` route or its no-op handlers.
- No remote clients, no bind address change, no CSP change (A6).
- No client guide beyond the developer-guide facts (A7).
- No `roymctl` command for the feed.
- No config fields (D-A5-13).

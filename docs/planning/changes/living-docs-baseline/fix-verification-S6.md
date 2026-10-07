# Stage 2 verification, batch S6

Reader: the person who fixes `docs/system-architecture.md` after stage 2.

Scope: fix commits 16 (Federation, Cross-Substrate Discovery Flow, Minimum Federation Contract, Consumer App Architecture) and 17 (SynApp 1: Roym, Component Architecture, Cards, Booking State Machine, Consumer Transaction Flow, Recommendation Algorithm, Local Producer-Distributor Mesh). 63 rows: 25 + 38, counted from `fix-new-claims.md`. Row ids are `<commit>.<n>`.

Method: static reading only. I opened the Roym crates (`roym_core`, `roym_directory`, `roym_transaction`, `roym_catalog`, `roym_web`), the Hub UI sources, `conversation`, `router`, `core`, `sdk` and `coordinator_webrtc`. I then read the matching text in `docs/system-architecture.md` (lines 1074-1285 and 1286-1390). Negative claims were re-searched with other terms (see the notes in the table).

## Summary

| Verdict | Commit 16 | Commit 17 | Total |
| --- | --- | --- | --- |
| CONFIRMED | 20 | 37 | 57 |
| CITE-OFF | 1 | 0 | 1 |
| PARTLY | 4 | 1 | 5 |
| WRONG | 0 | 0 | 0 |
| UNVERIFIABLE | 0 | 0 | 0 |
| Rows | 25 | 38 | 63 |

Rows that need a doc fix: 16.4, 16.14, 16.21, 16.24, 17.7. Row 16.17 needs a cite fix only. One unbacked doc sentence (Fulfilment bullet, line 1254) is wrong too: see "Doc text not covered by any row".

## Per-row verdicts

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 16.1 | CONFIRMED | `WIRE_REACHABLE`: search Open, publish VerifiedOnly; `publish-to-source` is local-only and calls remote `directory.publish`. | `crates/roym_directory/src/app.rs:74-79,184,237`; `app/client_sources.rs:207-260`; `roym_core/src/admit.rs:56-82` |
| 16.2 | CONFIRMED | Collections `members`, `publications`, `search_index` exist; search reads only local index. | `crates/roym_directory/src/app.rs:48-50`; `app/search_ops.rs:165-225` |
| 16.3 | CONFIRMED | `CallTarget::Service` only in client half; `search_ops.rs` makes no outbound call. Re-searched `host.call`. | `crates/roym_directory/src/app/search_ops.rs:165-225`; `client_query.rs:135`; `held.rs:93` |
| 16.4 | PARTLY | 8 and 3 are right. The Hub enforces the 3; the node only reports it and refuses when busy. | `crates/roym_core/src/directory.rs:46,57`; `roym_web/ui/src/directory/search.ts:134-190`; `client_query.rs:117` |
| 16.5 | CONFIRMED | Each hit goes through `listing::verify_envelope`; `SearchHit` has no verdict; refused hits stored apart, capped at 20. | `crates/roym_directory/src/app/client_query.rs:160-190`; `roym_core/src/directory.rs:43,185-199` |
| 16.6 | CONFIRMED | Round robin in DID order, 10 per source, 50 cap. Runs deleted lazily at next `start-run` after 1 h. No index cache (grep). | `crates/roym_directory/src/app/client_merge.rs:20-45,128-165`; `client_query.rs:76-100`; `roym_core/src/directory.rs:35-38,76` |
| 16.7 | CONFIRMED | Client-half verbs are in the one `invoke` table of every installation. | `crates/roym_directory/src/app.rs:222-237` |
| 16.8 | CONFIRMED | No routing schema, shard or rendezvous in `roym_*` (grep). `shard` hits are service placement only. | `crates/roym_directory/src/app/search_ops.rs:165-225`; `crates/roym_core/src/record.rs:18-31` |
| 16.9 | CONFIRMED | `did:key` from Ed25519 key; DHT stores endpoint packets and master-anchor `revoked_keys`. | `crates/identity/src/substrate.rs:140-146`; `core/src/dht_registry/master_anchor.rs:24-30`; `client.rs:109-166` |
| 16.10 | CONFIRMED | `LISTING_VERSION = 1`; `RECORD_TYPES` table exists with `listing`. | `crates/roym_core/src/listing.rs:19`; `record.rs:18-31` |
| 16.11 | CONFIRMED | `prekey-bundle` call before first send; JSON-RPC v1 is the only `ProxyProtocol`; card content type constant. | `crates/conversation/src/transport.rs:111,136`; `rpc/src/proxy.rs:21-24`; `roym_core/src/card.rs:25-29` |
| 16.12 | CONFIRMED | `BUNDLE_VERSION` and sections; `roymctl roym backup` command. | `crates/roym_core/src/backup.rs:14-40`; `apps/roymctl/src/commands/roym.rs:84-88` |
| 16.13 | CONFIRMED | Twelve entries in `RECORD_TYPES`; no `ReputationRecord` or "reputation" in crates or apps (grep). | `crates/roym_core/src/record.rs:18-31` |
| 16.14 | PARTLY | Table exists, but `is_known_record` is only called in its own test. Verifiers check their own type. | `crates/roym_core/src/record.rs:50-53`; `listing.rs:511`; `booking.rs:338` |
| 16.15 | CONFIRMED | `vite` and `typescript` in package.json; `web` has asset archive; no tauri anywhere; `apps/` has only `roymctl`. | `crates/roym_web/ui/package.json:6-19`; `roym_core/app/roym.toml:29-32` |
| 16.16 | CONFIRMED | Only `sessionStorage` (token) and IndexedDB (non-extractable Ed25519 key); no `localStorage`, no cookie code. | `crates/roym_web/ui/src/session/login.ts:34-50,120-135` |
| 16.17 | CITE-OFF | Claim true, but messages live in `conversation.db`, not `state.db`. Both are per-service and key-encrypted. | `crates/conversation/src/store/schema.rs:27,248-253`; `conversation/src/lib.rs:208-213` |
| 16.18 | CONFIRMED | Delegate command, 24 h default; Hub signs auth challenge on `auth.` origin; no master key in Hub. | `apps/roymctl/src/commands/session.rs:26-34`; `roym_web/ui/src/session/login.ts:20-26,161-215` |
| 16.19 | CONFIRMED | Hub only uses `crypto.subtle` for login; `vodozemac` in conversation; no `libsignal` anywhere (grep). | `crates/conversation/Cargo.toml:35`; `conversation/src/crypto.rs:5-25` |
| 16.20 | CONFIRMED | `fetch("/rpc")`, POST, `jsonrpc: "2.0"`, Bearer header; `/rpc` route on `web`. | `crates/roym_web/ui/src/rpc.ts:24-33`; `session/login.ts:56-59`; `roym_core/app/roym.toml:23-26` |
| 16.21 | PARTLY | Iroh is true. The only remote hop is `IrohHop`; the WebRTC endpoint mechanism is "Not implemented". | `crates/router/src/proxy/hop.rs:14-20,48`; `sdk/src/client.rs:404-406` |
| 16.22 | CONFIRMED | Bootstrap serves `/sw.js` and `peer-proxy.js`; page registers service worker; e2e test uses comments app. | `crates/coordinator_webrtc/src/bootstrap.rs:115-117`; `templates/peer-proxy.js:766`; `e2e/tests/webrtc.spec.ts:7-36` |
| 16.23 | CONFIRMED | Booking e2e boots three full nodes; the search client runs on the consumer's node. | `crates/substrate/tests/roym_booking_e2e.rs:32-60`; `roym_directory/src/app/client_query.rs:121` |
| 16.24 | PARTLY | `session.whoami` also needs no session. Rest of the marker is true. | `crates/roym_web/src/app.rs:76-91,129-131`; `roym_core/src/router.rs:56-67` |
| 16.25 | CONFIRMED | Fixed screens in `ui/src/screens/`; `/ws` route declared; `on_ws_*` are empty. No `WebSocket` in Hub (grep). | `crates/roym_core/app/roym.toml:26`; `roym_web/src/app.rs:295-308` |
| 17.1 | CONFIRMED | Seven card types; request, quote, agreement, progress each send a card. | `crates/roym_core/src/card.rs:9-17`; `roym_transaction/src/app/progress.rs:40-62`; `agreement_ops.rs:36` |
| 17.2 | CONFIRMED | Six services and all `depends_on` edges match; `profile` has none. | `crates/roym_core/app/roym.toml:13,54,67,81,96` |
| 17.3 | CONFIRMED | Whoami answered in `web`; others admitted by `method_auth` (Delegated session, owner DID), then routed. | `crates/roym_web/src/app.rs:76-91,112-146`; `roym_core/src/router.rs:22-67` |
| 17.4 | CONFIRMED | Prefix table matches every listed prefix; conversation guest imports `syneroym:conversation`. | `crates/roym_core/src/router.rs:22-51`; `roym_conversation/src/guest.rs:27` |
| 17.5 | CONFIRMED | Booking, payment, fulfilment verbs are in `transaction`; slot read via `availability.get`. State logic is in `roym_core` lib. | `crates/roym_transaction/src/app.rs:246-265`; `app/ledger.rs:138`; `quote_ops.rs:236-248` |
| 17.6 | CONFIRMED | Client-half verbs and `SOURCES`, `RUNS` collections in every installation. | `crates/roym_directory/src/app.rs:55-56,222-237` |
| 17.7 | PARTLY | Table is right. The `status` export is not gated; `web` also has a separate HTTP ingress. | `crates/roym_transaction/src/guest.rs:44-50`; `roym_core/src/admit.rs:20-30`; `roym_web/src/app.rs:239` |
| 17.8 | CONFIRMED | `prekey-bundle` and `deliver` in transport; cards go out via `conversation.send` with card content type. | `crates/conversation/src/transport.rs:1-2`; `roym_transaction/src/app.rs:560-585` |
| 17.9 | CONFIRMED | Hub only fetches `/rpc` and the auth origin; encrypted sends; data layer opens one DB per service. | `crates/roym_web/ui/src/rpc.ts:24-33`; `conversation/src/lib.rs:1-8`; `data_db/src/sqlite/provider.rs:269` |
| 17.10 | CONFIRMED | All six services `wasm`; no stripe, push, drm, beckn, review type (grep); `/ws` handlers empty. | `crates/roym_core/app/roym.toml:10,38,48,61,74,88`; `roym_core/src/record.rs:18-31` |
| 17.11 | CONFIRMED | `Card` holds only `envelope`; filers verify before storing. | `crates/roym_core/src/card.rs:34-44`; `roym_transaction/src/app/sync.rs:259-330` |
| 17.12 | CONFIRMED | Hub's `loadHistory` calls `transaction.sync` then `transaction.thread`; sync reads history and files cards. | `crates/roym_web/ui/src/screens/messages.ts:322-335`; `roym_transaction/src/app/sync.rs:40-117` |
| 17.13 | CONFIRMED | `RequestPayload` fields match; no expiry. | `crates/roym_core/src/transaction.rs:143-165`; `request_ops.rs:43-77` |
| 17.14 | CONFIRMED | `AgreedTerms` holds all listed terms; `QuotePayload.slot_id` is optional; expiry is a term. | `crates/roym_core/src/transaction.rs:114-141,170-184` |
| 17.15 | CONFIRMED | Each role signs the same `terms`; `pair_state` is `Complete` only with both halves. | `crates/roym_transaction/src/app/agreement_ops.rs:405-460`; `roym_core/src/transaction.rs:230-237` |
| 17.16 | CONFIRMED | Signed with `Principal::Service`; consumer refuses if issuer differs from quote signer. | `crates/roym_transaction/src/app/progress.rs:31`; `sync/receipts.rs:89-93` |
| 17.17 | CONFIRMED | Request is provider-only with note optional; acks and fulfilment take role from owner DID. | `crates/roym_core/src/payment.rs:46-58,100-121`; `payment_ops.rs:325-340`; `fulfilment_ops.rs:260-266` |
| 17.18 | CONFIRMED | 300 s to 90 days enforced at `quote.set` and on verify; `expires_at_secs: None` on requests. | `crates/roym_core/src/transaction.rs:63-64,665`; `quote_ops.rs:54-59`; `request_ops.rs:175` |
| 17.19 | CONFIRMED | Sets `declined_at_secs` on local pointer; refuses after either half; no reject verb (grep). | `crates/roym_transaction/src/app/quote_ops.rs:456-505`; `app.rs:228-271` |
| 17.20 | CONFIRMED | `provider-only` check; `seq` from 1; step written with `create`; `0..3` retry then "booking is busy". | `crates/roym_transaction/src/app/booking_ops.rs:30-33,104-210`; `roym_core/src/booking.rs:97,163` |
| 17.21 | CONFIRMED | Countersign checks expiry before opening booking; slot quote refused for provider first. Runs during sync. | `crates/roym_transaction/src/app/agreement_ops.rs:369-374,405-436`; `sync.rs:567` |
| 17.22 | CONFIRMED | Six states; four final; `open` gives scheduled or conflict; window close ends `scheduled` too. | `crates/roym_core/src/booking.rs:40-47,163-190,224-257` |
| 17.23 | CONFIRMED | Conflict returns before countersign; manual accept answers "slot-unavailable". | `crates/roym_transaction/src/app/agreement_ops.rs:384-388,430-432` |
| 17.24 | CONFIRMED | `against_interest` is provider payment and consumer fulfilment; others become `Claimed`. | `crates/roym_core/src/booking.rs:259-280` |
| 17.25 | CONFIRMED | `PaymentTiming` read only by `next_step` and a view field; never gates. Notice text says Roym cannot confirm. | `crates/roym_core/src/booking.rs:384-410`; `transaction.rs:72-79` |
| 17.26 | CONFIRMED | First half moves `scheduled` to `in-progress`; completed needs both acknowledged; repeat gives `Ok(None)`. | `crates/roym_core/src/booking.rs:259-324` |
| 17.27 | CONFIRMED | 30-day window from schedule end or open time; none/claimed become unconfirmed; then ended-unconfirmed. | `crates/roym_core/src/booking.rs:16-18,186-190,224-257`; `booking_ops.rs:37` |
| 17.28 | CONFIRMED | `provider-only`; cancel needs both tracks none; seat row deleted on cancel. | `crates/roym_transaction/src/app/booking_ops.rs:358-395,170-177`; `roym_core/src/booking.rs:305-315` |
| 17.29 | CONFIRMED | Capacity capped at 64; seats claimed in order with `create`; both conflict reasons and one-decision rule hold. | `crates/roym_transaction/src/app/ledger.rs:66-157,164-228`; `roym_core/src/booking.rs:19-20` |
| 17.30 | CONFIRMED | Terms fields are `String`; no dispute, refund, review or consumer-cancel verb; directory settings has `dispute_path`. | `crates/roym_core/src/transaction.rs:134-137`; `directory.rs:91`; `roym_transaction/src/app.rs:228-271` |
| 17.31 | CONFIRMED | Hits carry full `envelope`; consumer verifies; `catalog` refuses off-node callers. | `crates/roym_directory/src/app/client_query.rs:162`; `roym_core/src/directory.rs:187-199`; `roym_catalog/src/app.rs:87-90` |
| 17.32 | CONFIRMED | `request.set` signs and sends; `sync`, `quote.set`, `agreement.accept` exist. Only the Hub and `roymctl` call sync. | `crates/roym_transaction/src/app/request_ops.rs:43,87-96`; `sync.rs:40`; `apps/roymctl/src/commands/roym/transaction.rs:453` |
| 17.33 | CONFIRMED | Cards go through `conversation.send`; no push or notification code in `roym_*` (grep `notif`, `push`). | `crates/roym_transaction/src/app.rs:560-585` |
| 17.34 | CONFIRMED | Request, acks and the notice exist; completed needs both tracks acknowledged. | `crates/roym_core/src/booking.rs:25-27,259-280`; `roym_web/ui/src/cards/templates/payment_request.ts:72-75` |
| 17.35 | CONFIRMED | No stripe, PaymentIntent, webhook or review verb in crates or apps (grep). | `crates/roym_transaction/src/app.rs:228-271` |
| 17.36 | CONFIRMED | No recommend, collaborative, embedding, semantic in `roym_*` or `apps` (grep). `SearchQuery` fields match. | `crates/roym_core/src/directory.rs:167-181` |
| 17.37 | CONFIRMED | Sort is `issued_at_secs` desc then `listing_id`; round robin 10/50. Area rank picks a match, not order. | `crates/roym_directory/src/app/search_ops.rs:212-214`; `client_merge.rs:128-165` |
| 17.38 | CONFIRMED | None of `delivery-engine`, `tracking-service`, `OUT_FOR_DELIVERY`, `PREPARING`, `DELIVERED` in crates or apps. | `crates/coordinator_webrtc/build.rs:3` (only hit, a comment) |

## Findings

### 16.4 (PARTLY)

Doc (line 1292, "Cross-Substrate Discovery Flow"): "A consumer's own node runs the search. ... For each directory it sends `directory.search`, with at most 3 requests in flight."

Code: the node only returns `max_concurrency` (3) from `directory.start-run` (`crates/roym_directory/src/app/client_query.rs:117`). The fan-out loop that keeps 3 requests in flight is in the Hub (`crates/roym_web/ui/src/directory/search.ts:134-190`, `Math.min(run.maxConcurrency, ...)`). A client that ignores the number is not stopped by this code. The node refuses to start a request when its own guest-HTTP admission is full, and the Hub retries that source once (`search.ts:195-205`).

Proposed text: "A consumer's own node keeps a list of up to 8 directories that the person added. The Hub starts a search run on its own node. For each directory it sends `directory.search` through the node. The node reports a limit of 3, and the Hub keeps at most 3 requests in flight. If the node is busy, it refuses to start a request, and the Hub retries it once."

### 16.14 (PARTLY)

Doc (line 1336, "Minimum Federation Contract"): "A Roym node verifies every record against a fixed table of record types and versions, and refuses a record of an unlisted type or version."

Code: `RECORD_TYPES` and `is_known_record` (`crates/roym_core/src/record.rs:18-31,50-53`) are not used by any verifier. `grep` for `is_known_record` and `RECORD_TYPES` over the repo finds only `record.rs` and its unit test. Each verifier checks the one type and version it expects: `listing.rs:511`, `transaction.rs:605,649,704`, `payment.rs:208,255`, `fulfilment.rs:87`, `membership.rs:301,330,349`, `booking.rs:338`. The generic `verify_json` (`crates/signed_record/src/verify.rs:261`) has no type table. Also `booking-progress` is a signed record type that is deliberately not in the table (`record.rs:44-48`).

Proposed text: "Each Roym verifier checks that a record has the type and version it expects, and refuses any other. The table `RECORD_TYPES` lists the twelve record types and their versions. The code does not read this table when it verifies. The `booking-progress` record is signed by the provider's service and is not in the table."

### 16.17 (CITE-OFF)

Claim "All data is in the person's own node, in per-service SQLite databases (`state.db`)" is true for the doc text ("its SQLite databases"). The cite is incomplete: the `conversation` store opens its own `conversation.db` with the same data key (`crates/conversation/src/store/schema.rs:27,248-253`, key from `crates/conversation/src/lib.rs:208-213`). Add these to the evidence. No doc change.

### 16.21 (PARTLY)

Doc (line 1351): "The node talks to the provider's node with the route preamble on an Iroh or WebRTC stream." The mermaid edge at line 1376 says the same ("route preamble over Iroh or WebRTC").

Code: the only outbound node-to-node hop is `IrohHop` (`crates/router/src/proxy/hop.rs:48`). Its comment says a WebRTC-only node fails every remote hop (`hop.rs:14-20`). The SDK client has `EndpointMechanism::WebRtc` with the body "Not implemented" (`crates/sdk/src/client.rs:404-406`). The cited `bootstrap.rs:39` only imports Iroh types. WebRTC is used for a browser to reach a node (row 16.22), not for node-to-node calls.

Proposed text (line 1351): "The node talks to the provider's node with the route preamble on an Iroh stream. A WebRTC transport exists for browsers that connect to a node. Node-to-node calls do not use it today." Mermaid label: "route preamble over Iroh".

### 16.24 (PARTLY)

Doc (line 1386, Envisioned note): "Every Hub method except `profile.policy` needs an owner session."

Code: `session.whoami` is answered by `web` before any admission check (`crates/roym_web/src/app.rs:129-131`) and returns "anonymous" with no session. `profile.policy` is the only entry in `PUBLIC_METHODS` (`crates/roym_core/src/router.rs:56`). A method with no routing prefix is also admitted and then answered `-32601` (`app.rs:76-78,133-139`), but it is never forwarded.

Proposed text: "Every Hub method that `web` forwards, except `profile.policy`, needs an owner session. `session.whoami` is answered without a session."

### 17.7 (PARTLY)

Doc (line 1090): "A call that does not come from inside the installation is answered with error `-32013`, except for four `directory` methods."

Code: the gate is `require_internal` or `admit` at the top of each service's `invoke` (`roym_transaction/src/app.rs:221`, `roym_catalog/src/app.rs:88`, `roym_profile/src/app.rs:55`, `roym_conversation/src/app.rs:182`, `roym_web/src/app.rs:239`, `roym_directory/src/app.rs:184`). The `status` export is not gated (`crates/roym_transaction/src/guest.rs:48-50`, same shape in every service) so health checks work. `web` also has an HTTP ingress (`POST /rpc`) with its own session gate (`roym_web/src/app.rs:76-91`).

Proposed text: "A call to the `invoke` export of a service that does not come from inside the installation is answered with error `-32013`, except for four `directory` methods. The `status` export stays open on every service for health checks."

## Doc text not covered by any row

1. Line 1254, "Consumer Transaction Flow", Fulfilment bullet: "The booking is complete only when the provider's claim and the consumer's confirmation both exist, and the payment track is acknowledged too." WRONG in part. The consumer can sign `fulfilment.sign` with no earlier provider half: `check_fulfilment_preconditions` only needs a complete agreement and a non-final booking (`crates/roym_transaction/src/app/fulfilment_ops.rs:248-272`). The consumer's fulfilment half acknowledges the track at once (`crates/roym_core/src/booking.rs:259-270`). The provider's node applies it on arrival with no provider half needed (`sync/receipts.rs:419-427`). Proposed text: "The booking is complete when the payment track and the fulfilment track are both acknowledged. The consumer's confirmation acknowledges the fulfilment track at once, even if the provider has not claimed the work."
2. Line 1157, "Booking State Machine": "the node claims a seat or a decision, opens the booking and countersigns the agreement on its own". True, but it happens only while `transaction.sync` runs on the provider's node. Only the Hub (`messages.ts:329`) and `roymctl` (`apps/roymctl/src/commands/roym/transaction.rs:453`) call it; no background task does (grep `transaction.sync`). Add: "This happens when `transaction.sync` runs on the provider's node, for example when the provider opens the conversation in the Hub."
3. Line 1205: "It talks ... to the provider's node only through cards." The conversation transport also calls `prekey-bundle` and `deliver` on the provider's node (`crates/conversation/src/transport.rs:136`), and plain messages travel the same way. Low severity. Suggested: "...and to the provider's node only through the conversation, which carries the cards."
4. Line 1090: "Two nodes talk through ... `directory.search` and `directory.publish`." The client half also calls `directory.info` (`client_sources.rs:41`) and `directory.standing` (`held.rs:93`) on another node. Incomplete, not wrong.
5. Lines 1292 and 1260, "It keeps each search run for one hour" and "Search runs are kept for one hour": runs are deleted when the next `directory.start-run` runs, if they are older than one hour (`crates/roym_directory/src/app/client_query.rs:76-100`). An old run is not refused before that. Suggested: "It deletes search runs older than one hour when the next search starts." Low severity.

## Cost notes

- About 63 rows in a single long session. Effort per row was low for commit 17 (one code module, `roym_core/src/booking.rs` and `roym_transaction`), higher for commit 16 (many crates).
- Hard rows: 16.14 (needed a repo-wide search to see `is_known_record` has no caller), 16.21 (needed the router hop code and the SDK client to see WebRTC is a stub), 16.4 (the 3-in-flight limit lives in the Hub, not the node), 17.7 (the `status` export), and the Fulfilment bullet (needed the consumer-side fulfilment path).
- Easy rows: all the negative searches in 17.10, 17.35, 17.36 and 17.38 (no hits at all) and the constant-value rows.
- The evidence column was accurate for line numbers in almost every row. Where a row was weak, the claim in the doc was stronger than the cited code.

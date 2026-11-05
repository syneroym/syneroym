# M08A slice A7 — Client guide and close-out: implementation plan

*Reader: the developer (or AI) who implements slice A7. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written 2026-11-05. Plan review round 1 folded in
2026-11-05 ([slice-a7-review.md](slice-a7-review.md), P1–P11). The owner
answered Q1–Q8 on 2026-11-05: (a) for each, with notes on Q1, Q4, Q6 and
Q7. Sections 4 to 10 follow the answers.**

**Size and PR.** S. One PR on `feat/m08a-a7-client-guide`, with the commits
of section 6 in that order. The branch starts from `main` at `ac6826f6`
(A6 is merged). A7 writes one new developer-guide section, "Build a Roym
client", and tests it with a new demo story that runs the guide's own
commands with `curl` against the two demo nodes. It also closes the change
doc. It changes no JSON-RPC method, no WIT file, no Roym service and no
substrate code. The only Rust change is one rule in `xtask/src/verify.rs`
(Q7). Section 10 lists exactly which files change.

All links are to `main` at `ac6826f6`.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - "Writing for readers": the new guide section names its reader, starts
    with what the reader can do, uses short sentences, and keeps every exact
    name, field, status code and limit.
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). The e2e gate runs the new story.
  - Mandatory living-docs, demo and deferred-backlog close-out. A7 is the
    last slice of the change doc, so it also sets `status: done`.
  - New tools in the build pipeline go into `mise.toml` (Q3).
  - No planning ids ("M08A", "A7", "OF-16", "D-A7-3") in code, comments,
    scripts or test names. Neither the guide nor the story's captions cite
    a decision id, criterion id, milestone or slice: the guide is a living
    doc. ADR ids and links to other parts of the developer guide are fine.
    No tool checks the guide or a shell script for this, so check it by
    hand.
  - Tests never hardcode a port. The demo story reads every port from the
    demo env file.
- [task.md](task.md): "A7 — Client guide and close-out" and section 4 (A7
  comes after A4, A5 and A6; all three are "Merged" in
  [status.md](status.md)).
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  "Scope and non-goals" (the client guide line), OF-16, the Tasks list, the
  "Close-out" section. No other change doc defines a criterion for A7.
  `task.md` names no other change doc for this slice.
- The review files [slice-a2a-review.md](slice-a2a-review.md) to
  [slice-a6-review.md](slice-a6-review.md). Section 7.4 lists how A7 avoids
  each kind of finding.
- [deferred-backlog.md](../../deferred-backlog.md): §10 row "Local-only
  methods refuse unknown fields, which assumes a client built with the
  node" (`:521`), whose target says "Decide in A6 or A7 (the client guide
  must say it)".

### Terms used in this plan

- **Client:** a program that uses Roym's methods for one person: the Hub,
  `roymctl`, a phone app, a bot, a script.
- **Hub host:** the gateway host name of Roym's `web` service. Every Roym
  method is called as `POST /rpc` on this host. `web` forwards the call to
  the service that owns the method.
- **Auth service:** the node's login service. It answers
  `/_syneroym/session/*`. A client reaches it at `auth.<domain>`.
- **Session key file (`session-key.json`):** the file that
  `roymctl session delegate` writes. It holds a temporary Ed25519 key and a
  delegation certificate that the person's master key signed. The master
  key itself never leaves the machine where `roymctl` runs.
- **Session token:** the token the auth service returns after a login. A
  client sends it as `Authorization: Bearer <token>`.
- **Event feed:** `GET /_syneroym/events` on the Hub host. A Server-Sent
  Events (SSE) stream that says "something changed", with a cursor.
- **Demo story:** a short Playwright test under
  `crates/substrate/tests/e2e/demo/` that shows one working path with
  captions. It runs in `npm test`, so `mise run verify` and the CI job
  "Playwright E2E" run it.
- **Tagged code block:** a fenced code block in the guide whose info string
  is `bash roym-client=<name>` (or `js roym-client=<file name>`). Markdown
  renderers use only the first word (`bash`) for highlighting, so a reader
  sees a normal block. The new story reads these blocks and runs them.

## 1. What A7 builds, in one paragraph

A new section `## 6. Build a Roym client` at the end of
`docs/developer-guide.md` shows, with `curl` and `jq`, how a client logs in
with a session key file (challenge, sign, login), calls Roym methods
through `POST /rpc` on the Hub host, lists conversations, opens one and
sends a message, follows the event feed with `curl -N` and catches up with
a cursor, runs a search over one directory with `directory.find`, reads
the cards of a conversation, and refreshes and ends the session. The one
step `curl` cannot do, the Ed25519 signature, is a 25-line Node.js script
that the section prints in full (Q2). A new demo story,
`30-roym-client.sh` with its spec `30-roym-client.spec.ts`, reads the
section's tagged code blocks out of the guide and runs them against
Alice's demo node, so the guide's text is the test (Q1). `jq` goes into
`mise.toml` (Q3). The guide says what happens with an unknown field and
why (Q6). `xtask verify` runs its `e2e` gate, and only that gate, when
`docs/developer-guide.md` changed, so a guide-only change still runs the
story (Q7). The change doc gets OF-16's evidence (and OF-1's and OF-2's,
which were left empty) and `status: done`.

## How this is shown to an end user

An end user sees nothing new from this slice. It is for developers who
build their own Roym app. In very simple words, for that developer:

- Dev reads one page of the developer guide, "Build a Roym client".
- Dev makes a session key file with `roymctl`, the same file the Hub uses.
- Dev copies the commands from the page into a terminal: sign in, list
  chats, open a chat with a friend, send "Hello".
- In a second terminal, one command shows a live line each time something
  changes: the sent message, then its delivery.
- One more command searches a directory and prints which directories
  answered.

A developer can see the work in these ways:

- `mise run demo` runs every story; the new story prints each guide step as
  a caption, then the `curl` command and the node's answer.
- With the demo nodes up (`mise run demo:up`, then the Roym setup script),
  `bash crates/substrate/tests/e2e/demo/30-roym-client.sh` runs only this
  story. `DEMO_STEP=1` waits for Enter after each caption. `jq` must be on
  `PATH` (`mise run demo` puts the mise tools there). The story runs once
  per `demo:up`, like story 20: run `mise run demo:down` and `demo:up`
  before you run it again.
- Any reader can follow section 6 of `docs/developer-guide.md` by hand
  against their own node.

## 2. Points for the owner

Each question has options, a recommendation (a) and the reason. Two points
are not questions, because the change doc settles them:

- The guide uses only the public HTTP API: the auth endpoints, `POST /rpc`
  on the Hub host, and the event feed. It imports no Hub code and no Rust
  type (the change doc's goal: "with no Hub code").
- No client SDK package and no published API contract (change doc
  non-goals; backlog row "The Roym API contract is not published",
  `deferred-backlog.md:520`).

### Q1. How is the guide tested?

Task text: "tested by following it with `curl` and `curl -N` (or a small
script) against a test node."

- **(a) A demo story that runs the guide's own tagged blocks.**
  `30-roym-client.sh` extracts each `roym-client=<name>` block from
  `docs/developer-guide.md` by name and runs it, with checks between the
  blocks. The demo nodes already run Roym in `login` mode, and story 20
  has already deployed Roym, signed Alice and Bob in, and written their
  session key files and conversation addresses into the env file
  ([20-roym-setup.sh](../../../../crates/substrate/tests/e2e/demo/20-roym-setup.sh)).
  If someone edits a command in the guide and breaks it, the story fails.
- (b) A story script that copies the guide's commands. Same nodes, but the
  guide and the script can drift apart; it would need a second check that
  they match.
- (c) A Rust substrate test (`RoymNode` and `reqwest`) that does the same
  calls. It tests the API, which other tests already do, not the guide's
  text. It also cannot run `curl -N` the way a reader does.

**Recommendation: (a).** It is the only option where the guide cannot go
stale without a red test. It reuses the demo nodes, `lib.sh` and
`steps.ts`, so it adds no node boot code. It also meets the Mandatory Demo
close-out with a story a viewer can watch.

**Owner answer: (a), 2026-11-05**, once P1 and P9 of the plan review are
fixed. Both are fixed in this plan: the story finds the Hub host the way
the guide says, with the tagged `address` block (D-A7-2, D-A7-3, D-A7-8),
and each block checks its own result, so it behaves the same in a
reader's shell as under `set -euo pipefail` (D-A7-4).

### Q2. How does the guide sign the login challenge?

A `delegated-key` login needs an Ed25519 signature over the challenge
assertion, encoded in z-base-32 (section 3.1). `curl` cannot sign. Python's
standard library has no Ed25519. `openssl pkeyutl -rawin` can sign, but
z-base-32 still needs code.

- **(a) A 25-line Node.js script, printed in full in the guide**
  (`sign-challenge.mjs`, D-A7-5). It uses only `node:crypto` and
  `node:fs`. Node 20 is already in `mise.toml:26`. The script is a tagged
  block, so the story writes it to a file and runs it.
- (b) A new `roymctl session sign-challenge` command. New product code,
  and a phone or bot developer still has to learn the signature format.
- (c) The tested path uses `roymctl session login` and
  `roymctl session token`; the signature is described in prose only. Then
  the guide never shows a client that signs, and the signing text is not
  tested.

**Recommendation: (a).** It shows the whole protocol in a form any
language can copy, needs no new product code, and is tested.

**Owner answer: (a), 2026-11-05.**

### Q3. How do the commands read JSON?

- **(a) `jq`, added to `mise.toml` as `jq = "1.8"`.** `jq` is what most
  `curl` tutorials use, and it keeps each command short. Nothing in the repo
  uses `jq` today, so it must be added (AGENTS.md: new tools go into
  `mise.toml`). CI runs `mise install` through `ci-setup`.
- (b) Node one-liners (`node -e`) for every field. No new tool, but every
  command gets long and hard to read.
- (c) `python3 -c 'import json …'`. Same problem as (b), and one more
  runtime.

**Recommendation: (a).**

**Owner answer: (a), 2026-11-05.**

### Q4. What does "run a search" search?

`directory.find` with no sources returns an empty page and calls no other
node (section 3.5).

- **(a) One real source.** The story's own set-up (not part of the guide)
  makes Bob's node a directory with `roymctl roym directory serve`. The
  guide's block adds that directory with `directory.add-source` and runs
  `directory.find`. The answer has one outcome, for Bob's directory, with
  `"error": null` and no hits.
- (b) No source. The call works, but shows nothing: no outcome, no other
  node.
- (c) One source with a published listing, so a hit shows. Needs a listing
  on Bob's node, `directory.publish-to-source` and the directory's
  admission rules: much more set-up that other tests (Hub cases 22 and 23,
  `roym_directory_e2e.rs`) already cover.

**Recommendation: (a).** A reader sees a real fan-out to another node and
how an outcome reads, for two extra set-up commands.

**Owner answer: (a), 2026-11-05**, with P3 of the plan review. The
set-up copies the working recipe of `docs/demo/demo.md` (D-A7-8 item 3),
and Step 2 checks that a directory with no listings gives `error: null`.

### Q5. Does the guide cover cards?

The change doc's scope says the guide covers "how to log in, call methods,
read the event feed, render cards". OF-16 names log in, list
conversations, send a message, search and the feed, not cards.

- **(a) A short "Show cards" part.** What a card is
  (`application/vnd.roym.card+json` in a conversation), `transaction.sync`
  then `transaction.thread`, the fields to read (`card_type`, `version`,
  `known`, `verified`, `expired`, `reason`, `data`), the seven card types
  of v1, and "show a plain notice for a card type you do not know". The
  story calls the `cards` block and checks that the answer has a `cards`
  list. It does not create a card.
- (b) No cards. Record a deviation against the scope line.
- (c) A full card test: Alice sends a request card to a listing of Bob's.
  Much more set-up, already covered by `roym_transaction_e2e` tests and the
  Hub.

**Recommendation: (a).** The scope line asks for it, and a client that
cannot read cards cannot show a deal.

**Owner answer: (a), 2026-11-05.**

### Q6. Unknown fields on local-only methods (backlog row `:521`)

The row says the rule "assumes a client built with the node" and is to be
decided in A6 or A7. A6 did not decide it.

- **(a) Keep the rule.** The guide says: a local-only method refuses an
  unknown field with `-32602` and `data.field`; send only the fields the
  method lists; a client for a newer node may fail on an older node, so
  read `data.field` and tell the person to update the node. The row stays
  open, with its target changed to "TBD, with G5", and a note that the
  guide now says it.
- (b) Make local-only methods ignore unknown fields. This reverses OF-18
  and D-A2a-4 for 137 method rows, and hides client bugs. Out of scope for
  a docs slice.

**Recommendation: (a).**

**Owner answer: (a), 2026-11-05.** The new backlog target is written
"TBD, with G5", the same way as the neighbouring rows (section 8).

### Q7. Should `xtask verify` run the e2e gate when only the guide changes?

`xtask verify` skips the e2e gate when every changed path ends in `.md` or
starts with `docs/` (`is_docs_only_path`, `xtask/src/verify.rs:162`, and
the `e2e` gate, `:121`). After A7, a change that edits only the guide's
tagged blocks would skip the only test of those blocks locally. (CI runs
"Playwright E2E" on every PR, `.github/workflows/ci.yml`, so CI still
catches it.)

The docs-only switch is shared: it skips three gates, `nextest`,
`doctests` and `e2e` (`skip_on_docs_only: true` at `verify.rs:91`, `:106`,
`:121`).

- **(a) Yes.** As first written: `is_docs_only_path` returns `false` for
  `docs/developer-guide.md`. Cost: every guide-only edit (a typo in any
  part of the 2 200-line guide) then also runs `nextest` and `doctests`,
  about 15 minutes more.
- (a′) Yes, per gate. A `Gate` names the docs paths that still run it.
  Only `e2e` names `docs/developer-guide.md`; `nextest` and `doctests`
  still skip. About 10 more lines and one more test than (a).
- (b) No. Rely on CI.

**Recommendation: (a′).** Same result for the guide's test, without the
15-minute cost on every guide edit.

**Owner answer: (a) in intent, 2026-11-05**, with a per-gate rule that
adds only `e2e`: that is (a′). D-A7-11 describes it.

### Q8. Where does the section go?

- **(a) A new top-level `## 6. Build a Roym client`** at the end of the
  developer guide, after "5.7. Run a node in Docker". It is easy to find
  and link, and it stands alone (OF-16: "with only that section"). It links
  to the reference parts (auth endpoints `:1838`, event feed `:1848`, bad
  input `:1756`) for more detail, but every step a reader needs is in it.
- (b) A new `####` part inside "Interacting with Applications", next to
  "The Roym Hub" (`:1620`). Close to the reference text, but deep in a long
  part about operators.

**Recommendation: (a).**

**Owner answer: (a), 2026-11-05.**

## 3. Facts checked in the code

### 3.1 Login

- The challenge: `POST /_syneroym/session/challenge` with an optional
  body `{"master_did": "<did>"}` (`ChallengeRequest`,
  [service.rs:42](../../../../crates/auth/src/service.rs#L42); a missing or
  bad body is read as `{}`, `:635`). The answer is
  `{nonce, node_did, expires_at_secs, assertion?}`
  (`ChallengeResponse`, `:51`). `assertion` is present only when the
  request named a `master_did`. It is the RFC 8785 canonical JSON of
  `gateway_session_assertion(node_did, nonce, master_did)` (`:55-60`,
  `issue_challenge` `:217`, the assertion fields
  [protocol_utils.rs:103](../../../../crates/core/src/protocol_utils.rs#L103):
  `typ`, `node_did`, `nonce`, `person_did`). The client signs these exact
  bytes. The nonce lives `nonce_ttl_secs` (60 s by default) and works once.
- The signature: Ed25519 over the UTF-8 bytes of the assertion, encoded in
  z-base-32 (`Identity::sign_json`,
  [keys.rs:203](../../../../crates/identity/src/keys.rs#L203); the Hub does
  the same with WebCrypto, `completeLogin`,
  [login.ts:224](../../../../crates/roym_web/ui/src/session/login.ts#L224),
  `z32Encode` `:119`, the PKCS#8 prefix for a raw seed `:140`). The auth
  service accepts a signature by the temporary key or the master key
  (`login_delegated_key`, `service.rs:298-312`).
- The login: `POST /_syneroym/session/login` with
  `{"method": "delegated-key", "temp_did", "delegation", "nonce",
  "signature"}` (`LoginRequest`, `:77`). `temp_did` may be left out; then
  the certificate's `temporary_did` is used (`:497`). The answer is
  `{token, person_did, expires_at_secs}` (`LoginResponse`, `:93`) and a
  `Set-Cookie: syneroym_session=…; Path=/; HttpOnly; SameSite=Lax`
  (`grant_response`, `:449`). Errors are `{"error": "<text>"}` with status
  400, 401 or 409 (`login_delegated_key`, `:247-345`). 409 means the
  person's master anchor is not in the registry (`:279-287`).
- The session key file: `{temp_secret_key_hex, temp_did, certificate}`
  (`SessionKeyBundle`,
  [session.rs:85](../../../../apps/roymctl/src/commands/session.rs#L85)).
  `roymctl session delegate --as <name> --registry-url <url> --out <file>`
  writes it and publishes the master anchor first (`handle_delegate`,
  `:210`).
- Other endpoints: `GET /methods`, `GET /whoami`, `POST /refresh`,
  `POST /logout` (`handle_request`, `service.rs:606-663`). The token is read
  from the `syneroym_session` cookie first, then `Authorization: Bearer`
  (`extract_token`, `:416`).
- Where the auth service answers: any host whose first label is `auth`, or
  any request whose path starts with `/_syneroym/session` on a host that is
  not a service host (`resolve_target_host`,
  [gateway.rs:398](../../../../crates/client_gateway/src/gateway.rs#L398)).
  The Hub uses `auth.<domain>` (`authOrigin`, `login.ts:30`).

### 3.2 Calling a Roym method

- `web` answers `POST /rpc` on the Hub host
  ([app.rs:136](../../../../crates/roym_web/src/app.rs#L136)). It parses a
  JSON-RPC 2.0 request (`rpc`, `:150`; `-32700` on bad JSON, `-32600` on a
  missing method), answers `session.whoami` itself, checks the caller, and
  forwards the call to the owning service (`forward_rpc_call`, `:217`).
- The routing table: method prefix to service and auth level
  ([router.rs:22](../../../../crates/roym_core/src/router.rs#L22)). Every
  prefix is `Owner`. The only public method is `profile.policy` (`:56`).
  An unknown prefix is `-32601`.
- The caller check (`admit`, `app.rs:114`; codes `:206-215`): `-32010`
  "not signed in", `-32011` "this installation belongs to another person",
  `-32012` "this installation has no recorded owner". Forwarding errors:
  `-32001` "service not available", `-32002` "service did not answer in
  time", `-32603` otherwise (`:256-258`).
- Any session token, from `local` or `delegated-key`, makes the caller
  `delegated` (`guest_caller_identity`,
  [auth.rs:29](../../../../crates/router/src/route_handler/http/auth.rs#L29),
  `:40-41`). The router tries the cookie, then `Bearer`
  (`session_token_candidates`, `:59`). The gateway must run in
  `identity_mode = "login"` (developer guide `:1642`).
- The Hub host: `roymctl roym address` prints "Hub gateway host:
  s<web-did-hash>.<domain>" (`handle_address`,
  [address.rs:10](../../../../apps/roymctl/src/commands/roym/address.rs#L10),
  `:29`). The test harness calls `/rpc` on exactly that form
  (`web_host_header`,
  [roym.rs:495](../../../../crates/substrate/tests/common/roym.rs#L495)).
- Bad input: `-32602` with `data.field`; local-only methods refuse unknown
  fields; four directory methods are wire-reachable (developer guide
  `:1756-1766`, change doc D-A2a-6, D-A2c-3).
- `roymctl`'s own client sends `POST <gateway>/rpc` with a `Host` header,
  `Authorization: Bearer` and the cookie (`rpc_call`, `session.rs:560`).

### 3.3 Conversations

From the method table
([api.rs:24-47](../../../../crates/roym_conversation/src/api.rs#L24)):

- `conversation.list`: input `{kind?, include_hidden?, offset?, limit?}`
  (`ConversationListInput`,
  [messages.rs:56](../../../../crates/roym_conversation/src/app/messages.rs#L56));
  output `{conversations: [ConversationListItem]}` with `id`, `kind`,
  `peer_address`, `peer_person_did`, `opened_at_secs`, `last_activity_ms`,
  `message_count`, `name`, `restored`, `group`
  ([conversation.rs:37](../../../../crates/roym_core/src/conversation.rs#L37)).
- `conversation.open`: input `{address?, person_did?}`; `address` wins when
  both are sent (D-A2a-9); output `{conversation_id, peer_address}`
  (`messages.rs:43-53`). The parity fixtures open by `address` only
  ([fixtures.rs:159](../../../../crates/roym_web/tests/dual_build_parity/fixtures.rs#L159)).
- `conversation.send`: input `{conversation, body, content_type?}`; output
  `{message_id, state, sender_timestamp_ms}`, `state` one of `pending`,
  `deposited`, `delivered`, `failed`
  ([method/conversation.rs:10-21](../../../../crates/roym_core/src/method/conversation.rs#L10)).
- `conversation.changes`: input `{conversation, after_seq?, limit?}`;
  output `{messages, last_seq, kind}` (`:25-36`). `conversation.history`:
  input `{conversation, limit?, cursor?}`; output
  `{messages, next_cursor, kind}` (`messages.rs:70-81`). A message
  (`MessageView`, `conversation.rs:141`) has `id`, `direction`, `state`,
  `visible_seq`, `content_type`, `body_encoding`, `body`, and more.
- `conversation.delivery-status`: input `{message_id}`; output `{state}`
  (`messages.rs:90`).

### 3.4 The event feed

- `GET /_syneroym/events` on a service host, forwarded by the gateway
  (`is_forwarded_syneroym_path`, `gateway.rs:387`). Checks in order: 404
  (no bus), 401 (no session), 403 (not the owner), 404 (not part of an
  app), 406 (no `Accept: text/event-stream`), 503 with `Retry-After: 1`
  (16 feeds per app instance) (`check_feed_access`,
  [events.rs:289](../../../../crates/router/src/route_handler/http/events.rs#L289);
  `handle_event_feed_parts`, `:330`). The session token may come as the
  cookie or as `Bearer` (`select_session_token`, `auth.rs:84`).
- Frames: `ready` (no cursor given; the client loads its state), `change`
  (one event), `reload` (the cursor is too old or foreign; load again),
  and `: keep-alive` every 25 s (`events.rs:108-134`, D-A5-8). Resume with
  `Last-Event-ID: <cursor>` or `?after=<cursor>` (`parse_start_cursor`,
  `:57`).
- What each kind names:
  - `conversation.message`: subject = conversation id, item = message id
    (`admit_and_announce`,
    [conversation/lib.rs:184](../../../../crates/conversation/src/lib.rs#L184);
    `:500`; `transport.rs:489`).
  - `conversation.delivery`: subject = conversation id, item = message id
    (`lib.rs:236`, `transport.rs:291`, `:492`).
  - `transaction.card`: subject = conversation id, item = the card's message
    id (`put_card`,
    [roym_transaction/app.rs:395](../../../../crates/roym_transaction/src/app.rs#L395)).
  - `transaction.booking`: subject = conversation id, item = agreement id
    (`put_booking`, `app.rs:407`).
- The feed is lost on restart: a cursor from before a restart gets
  `reload` (D-OF-4).

### 3.5 Search

- `directory.add-source`: input `{did, label?, issuer_did?}`; output
  `{source, probe}`
  ([client_sources.rs:42-52](../../../../crates/roym_directory/src/app/client_sources.rs#L42)).
  It calls `directory.info` on the source node.
- `directory.find`: input `{query?}` with `query` a `SearchQuery`
  (`text?`, `categories`, `area?`, `open_to?`, `booking_mode?`, `limit?`;
  [directory.rs:162](../../../../crates/roym_core/src/directory.rs#L162));
  output `{run_id, outcomes, hits, hits_truncated, refused,
  refused_truncated}` (`FindOutput`,
  [client_find.rs:23](../../../../crates/roym_directory/src/app/client_find.rs#L23),
  `Page`,
  [lib.rs:42](../../../../crates/roym_federated_query/src/lib.rs#L42)). One
  outcome per source: `{source, verified, refused, truncated, error}`
  (`SourceReport`, `lib.rs:32`). With no source, `find` returns an empty
  page and calls nothing (D-A4-4).
- `roymctl roym directory serve` needs `--name`, `--rules-file`,
  `--support`, `--dispute`, and takes `--gateway-url` and `--host`
  ([directory.rs:90](../../../../apps/roymctl/src/commands/roym/directory.rs#L90)).

### 3.6 Cards

- A card is a message with content type `application/vnd.roym.card+json`
  ([card.rs:26](../../../../crates/roym_core/src/card.rs#L26)).
- `transaction.sync` (input `{conversation, full?}`) reads the
  conversation's new messages and files received cards
  ([sync.rs:29](../../../../crates/roym_transaction/src/app/sync.rs#L29)).
  Filing a card writes the card row and records `transaction.card`
  (`put_card`). So a card from the other person first shows as a
  `conversation.message` event; the client calls `transaction.sync`, which
  then records `transaction.card`. The Hub calls `transaction.sync`, then
  `transaction.thread`, when it opens a conversation
  ([messages.ts:396](../../../../crates/roym_web/ui/src/screens/messages.ts#L396)).
- `transaction.thread`: input `{conversation, limit?, cursor?}`; output
  `{cards: [CardRow]}`
  ([thread.rs:17-28](../../../../crates/roym_transaction/src/app/thread.rs#L17)).
  A card row has `message_id`, `direction`, `card_type`, `version`,
  `known`, `verified`, `expired`, `reason?`, `issuer?`, `data?`, and more
  (`CardRow`, `app.rs:183`).
- The v1 card types, all version 1: `request`, `quote`,
  `agreement-receipt`, `booking-progress`, `payment-request`,
  `payment-acknowledgement`, `fulfilment-receipt`
  ([registry.ts:1](../../../../crates/roym_web/ui/src/cards/registry.ts#L1)).
  The Hub shows a generic notice for any other type or version
  (`renderCard`, `render.ts`).
- `booking.get`: input `{agreement}`
  ([api.rs:53](../../../../crates/roym_transaction/src/api.rs#L53),
  `AgreementRef`, `app.rs:94`).

### 3.7 The demo harness

- The demo suite has its own config: `testDir: './demo'`, one worker, the
  stories run in file-name order and share two nodes, 180 s per test and
  900 s in all
  ([playwright-demo.config.ts](../../../../crates/substrate/tests/e2e/playwright-demo.config.ts)).
  `npm test` runs it last (`package.json:6`); `mise run test:e2e` runs
  `npm test` (`mise.toml:109-112`).
- Both demo nodes run the gateway in `login` mode
  ([demo-nodes.ts:103](../../../../crates/substrate/tests/e2e/demo-nodes.ts#L103),
  `:115`). The env file has `REGISTRY`, `S1_DIR`, `S2_DIR`, `S1_GW`,
  `S2_GW`, … (`writeEnvFile`, `:27`).
- Story 20's script writes `ALICE_HUB_URL`, `ALICE_KEY` (her
  `session-key.json`), `ALICE_DID`, `ALICE_ADDR` and the same for Bob. It
  logs Bob in with `roymctl session login` (stored in `S2_DIR`), so a later
  `rc2` call through the gateway has Bob's session
  ([20-roym-setup.sh](../../../../crates/substrate/tests/e2e/demo/20-roym-setup.sh)).
  It finds a service's identity with `did_of "$dir" "member-roym#web-0"`.
- `lib.sh` gives `caption`, `run`, `demo_set`, `rc1`, `rc2`, `did_of`,
  `host_of`, `rpc`, and sets `set -euo pipefail`
  ([lib.sh](../../../../crates/substrate/tests/e2e/demo/lib.sh)). A spec
  runs its script with `runScript`, which skips it under `DEMO_ATTACH=1`
  ([steps.ts](../../../../crates/substrate/tests/e2e/demo/steps.ts)).
- Story 20's spec ends with Alice and Bob in one conversation, Alice holds
  Bob as a contact, and both have profiles
  ([20-roym-chat.spec.ts](../../../../crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts)).
  Story 30 starts from that state (stories are cumulative, developer guide
  `:163`).

### 3.8 Verify and CI

- `xtask verify`'s `nextest`, `doctests` and `e2e` gates have
  `skip_on_docs_only: true`
  ([verify.rs:91](../../../../xtask/src/verify.rs#L91), `:106`, `:121`).
  One switch, `is_docs_only_change`, skips all three (`:266`).
  `is_docs_only_path` is `path.ends_with(".md") || path.starts_with("docs/")`
  (`:162`), tested at `:309-323`.
- CI runs "Playwright E2E" on every push and PR to `main`, after
  `mise run build:roym` and a build of `syneroym-substrate` and `roymctl`
  ([ci.yml](../../../../.github/workflows/ci.yml), job `e2e`).
- `jq` is used nowhere in the repo today (no match in `mise.toml`,
  `scripts/`, `.github/` or the demo folder). `node = "20"` is in
  `mise.toml:26`.

### 3.9 Living-doc statements that A7 touches

- `docs/developer-guide.md` has the reference parts (auth endpoints
  `:1838`, event feed `:1848`, bad input `:1756`, the Roym Hub `:1620`,
  `roym address` `:1678`) but no walkthrough for a client. Section 6 is new.
  The "Demo Stories" part (`:138-178`) lists the stories only by rule, not
  by name, so it needs no change.
- `docs/roym-integrated-experience-spec.md:156-157`: "Every Roym capability
  the UI uses is a public JSON-RPC method." Still true. Add one sentence
  that links the new guide section.
- `docs/planning/traceability-matrix.md`: the last Roym row is `[ROY-RMT]`
  (`:97`). No row for the client guide.
- `docs/system-requirements-spec.md`, `docs/system-architecture.md`,
  `docs/TERMINOLOGY.md`: no statement becomes false (grep for "client
  guide", "third-party", "any client" finds only general statements, for
  example `system-architecture.md:685`). They stay as they are.
- The change doc: OF-16's Test column is empty; two task lines are open
  ("Write the M08A slice plans", "Build a Roym client"); `status: draft`.

### 3.10 Findings that change the task text

- **F1. curl alone cannot log in.** A `delegated-key` login needs a
  signature (3.1). The task's "(or a small script)" covers this: Q2.
- **F2. The test node is the demo node.** There is no shell-reachable test
  node outside the demo harness. The story is the test (Q1).
- **F3. Cards are in the change doc's scope, not in OF-16** (Q5).
- **F4. The backlog row on unknown fields waits for A7** (Q6).
- **F5. A guide-only change would skip the guide's test locally** (Q7).
- **F6. A message from the other person and a card are two events.** A
  received card arrives as `conversation.message`; `transaction.card`
  comes only when the client calls `transaction.sync` (3.6). The guide's
  table of kinds says so.

## 4. Behaviour-change audit

### Wire changes

None. No method, field, status code, header or frame changes.

### Behaviour changes that are not wire changes

- `xtask verify` runs the `e2e` gate when a docs-only change includes
  `docs/developer-guide.md` (Q7). Before: skipped. `nextest` and
  `doctests` still skip on every docs-only change, and `e2e` still skips
  on a docs-only change that does not touch the guide.
- `mise install` installs `jq` (Q3).
- The demo suite has one more story; `mise run demo` and `npm test` take
  about 20 to 40 s longer (section 7.5).

### Stays the same

Every Roym service, the Hub, `roymctl`, the auth service, the gateway, the
router, every WIT file and every existing test and story. Story 30 makes
new calls on the demo nodes (a new session for Alice, a new message to Bob,
a directory on Bob's node); it runs after story 20, so story 20 does not
see them.

## 5. Decisions this plan makes

- **D-A7-1. The section.** `## 6. Build a Roym client` at the end of
  `docs/developer-guide.md`, with the reader line *"Reader: a developer who
  writes a program that uses Roym for one person: a phone app, a bot, a
  script, or another web UI."* Parts: 6.1 What you need, 6.2 Sign in, 6.3
  Call a method, 6.4 Conversations, 6.5 Follow the event feed, 6.6 Search,
  6.7 Show cards, 6.8 Errors and unknown fields, 6.9 End the session, 6.10
  A client on another machine. Each part starts with what the reader can
  do, then the command, then what the answer means. The reader has no
  project knowledge, so every project term is defined where it first
  appears, in one short sentence: master key, delegation certificate,
  session key file, session token, app instance, Hub host, SSE (or a link
  to [TERMINOLOGY.md](../../../TERMINOLOGY.md)).
- **D-A7-2. Tagged blocks.** Info string `bash roym-client=<name>` for a
  shell block, `js roym-client=sign-challenge.mjs` for the script. Names,
  in this order: `address`, `sign-challenge.mjs`, `login`, `rpc`, `list`,
  `open-send`, `feed`, `catch-up`, `search`, `cards`, `refresh-logout`.
  Each name occurs once. The variables block of 6.1 is **not** tagged (the
  story sets `SESSION_KEY`, `PEER_ADDR` and `DIRECTORY_DID` itself), but
  the `address` block that finds the two URLs is.
- **D-A7-3. The variables.** Set once in 6.1:
  ```bash
  AUTH_URL=http://auth.localhost:7960      # the node's auth service
  HUB_URL=http://s<web-did-hash>.localhost:7960  # the `address` block finds it
  SESSION_KEY=./session-key.json           # from `roymctl session delegate`
  PEER_ADDR=did:key:z...                   # the other person's conversation service id
  DIRECTORY_DID=did:key:z...               # a directory you trust
  ```
  The tagged `address` block finds the Hub host the way the guide tells a
  reader to, so the story tests it:
  ```bash
  GATEWAY_PORT=${GATEWAY_PORT:-7960}       # the node's gateway port
  HUB_HOST=$(roymctl roym address | sed -n 's/^Hub gateway host: *//p')
  [ -n "$HUB_HOST" ] || { echo "roymctl roym address printed no Hub host" >&2; false; }
  HUB_URL=http://$HUB_HOST:$GATEWAY_PORT
  AUTH_URL=http://auth.localhost:$GATEWAY_PORT
  ```
  The story sets `GATEWAY_PORT=$S1_GW` before it runs the block. The block
  only falls back to `7960` when `GATEWAY_PORT` is not set, so the story's
  port is kept (the block runs with `eval`). The story runs the block with
  Alice's `roymctl --dir "$S1_DIR"` (a shell function `roymctl` that adds
  the flags, or the block takes them from `ROYMCTL_ARGS`; the implementer
  picks the simpler). The guide says in one line that `roymctl roym
  address` lists the node's services as the node owner's identity (`--as`,
  on the node owner's machine), so a client on another machine gets the
  Hub host from the node owner (6.10). The URLs use host names, as a browser does, so 6.10 only
  changes them to `https://auth.<domain>` and `https://<hub host>`. Use
  `http://` only for `localhost`; any other host must be `https://`, or the
  signature, the token and every message cross the network in clear text
  (said in 6.2 and 6.10). Step 2 checks that the `curl` on the CI runner
  and on macOS sends `*.localhost` to `127.0.0.1`, and finds the first
  `curl` version that does this without DNS. If one does not, the guide
  adds `--connect-to ::127.0.0.1:` through a `CURL_OPTS` variable, and
  records a deviation.
- **D-A7-4. The blocks.** The implementer writes the final text; these are
  the commands and the facts each block must keep:
  - Every block checks its own result, so it behaves the same in a
    reader's interactive shell as in the story's `set -euo pipefail`
    shell. A block that pipes starts with `set -o pipefail` (6.1 says why
    in one line). A block that sets a value ends with a check that fails
    with a message, for example
    `[ -n "$TOKEN" ] && [ "$TOKEN" != null ] || { echo "login failed" >&2; false; }`,
    or reads it with `jq -e`.
  - `login`:
    ```bash
    set -o pipefail
    MASTER_DID=$(jq -r .certificate.master_did "$SESSION_KEY")
    CHALLENGE=$(curl -sS --fail-with-body -X POST "$AUTH_URL/_syneroym/session/challenge" \
      -H 'Content-Type: application/json' -d "{\"master_did\":\"$MASTER_DID\"}")
    SIGNATURE=$(node sign-challenge.mjs "$SESSION_KEY" "$(jq -r .assertion <<<"$CHALLENGE")")
    TOKEN=$(jq -n --slurpfile key "$SESSION_KEY" --argjson ch "$CHALLENGE" --arg sig "$SIGNATURE" \
        '{method: "delegated-key", temp_did: $key[0].temp_did, delegation: $key[0].certificate,
          nonce: $ch.nonce, signature: $sig}' \
      | curl -sS --fail-with-body -X POST "$AUTH_URL/_syneroym/session/login" \
          -H 'Content-Type: application/json' -d @- \
      | jq -r .token)
    [ -n "$TOKEN" ] && [ "$TOKEN" != null ] || { echo "login failed" >&2; false; }
    ```
    Text: the nonce lives 60 s and works once; 409 means the master anchor
    is not in the registry (run `session delegate` with `--registry-url`);
    the session lasts at most `session_ttl_secs` (8 h by default) and never
    past the certificate; 401 "delegation certificate has expired" means
    the key file is too old (D-A7-12 item 4).
  - `rpc`: a shell function `roym <method> [params-json]` that posts
    `{"jsonrpc":"2.0","id":1,"method":…,"params":…}` to `$HUB_URL/rpc`
    with `Authorization: Bearer $TOKEN` and prints the whole JSON-RPC
    answer. `params` defaults to `{}`. Then
    `roym session.whoami | jq .result`.
  - `list`: `roym conversation.list | jq '.result.conversations[] | {id,
    kind, peer_address, message_count}'`.
  - `open-send`:
    ```bash
    set -o pipefail
    CONV=$(roym conversation.open "{\"address\":\"$PEER_ADDR\"}" | jq -r .result.conversation_id)
    MESSAGE_ID=$(roym conversation.send "{\"conversation\":\"$CONV\",\"body\":\"Hello from curl\"}" \
      | jq -r .result.message_id)
    [ -n "$CONV" ] && [ "$CONV" != null ] && [ -n "$MESSAGE_ID" ] && [ "$MESSAGE_ID" != null ] \
      || { echo "open or send failed" >&2; false; }
    ```
    Text: `state` starts `pending`, then `deposited` or `delivered`, or
    `failed`; `conversation.delivery-status` reads it again.
  - `feed`: `curl -sS -N "$HUB_URL/_syneroym/events" -H 'Accept:
    text/event-stream' -H "Authorization: Bearer $TOKEN"`. Text: run it in a
    second terminal before you send; the frames; the table of kinds
    (subject, item, what to call: `conversation.changes` with `after_seq`,
    `conversation.delivery-status`, `transaction.sync` then
    `transaction.thread`, `booking.get`); the feed carries no content;
    `ready` and `reload` mean "load your state again"; keep the last `id:`
    you saw; at most 16 open feeds per app instance; a browser holds one
    connection per feed, so close it when the page is hidden. And: "The
    node closes the feed, with no frame, when the session token that
    opened it expires or is logged out (it checks on every change and on
    each 25 s keep-alive). This is normal. Reconnect with a valid token and
    `Last-Event-ID: <last id>`. If the answer is `reload`, load your state
    again." (`is_session_expired_or_revoked`, `make_feed_stream`,
    [events.rs:172-231](../../../../crates/router/src/route_handler/http/events.rs#L172).)
  - `catch-up`: the same command with `-H "Last-Event-ID: $CURSOR"`. Text:
    you get every event after the cursor, or `reload`; the window is 10 000
    events or 24 hours and is lost on restart.
  - `search`:
    ```bash
    roym directory.add-source "{\"did\":\"$DIRECTORY_DID\",\"label\":\"My guild\"}" | jq .result.source
    roym directory.find '{"query":{"text":"plumber"}}' | jq '.result | {outcomes, hits}'
    ```
    Text: one call asks every source at once and merges the answers; one
    outcome per source with `error` `null`, `not-found`, `timed-out`,
    `not-started`, `refused` or `unreadable`; each hit is checked on this
    node; how to read `hits`, `outcomes` and `refused`, with their fields
    named in the guide's own words.
  - `cards`: `roym transaction.sync "{\"conversation\":\"$CONV\"}"`, then
    `roym transaction.thread "{\"conversation\":\"$CONV\"}" | jq
    .result.cards`. Text: Q5 (a).
  - `refresh-logout`: `POST $AUTH_URL/_syneroym/session/refresh` with
    `Bearer $TOKEN` sets `NEW_TOKEN`; then `/logout` with
    `Bearer $NEW_TOKEN` and `/logout` with `Bearer $TOKEN`. Text: refresh never goes past the certificate.
    "`refresh` gives a new token. The old token stays valid until it
    expires; log it out too if you want it to stop working
    (`AuthService::refresh`,
    [service.rs:372](../../../../crates/auth/src/service.rs#L372), does not
    revoke it). A feed opened with the old token ends at the old token's
    end, so open the next feed with the new token."
- **D-A7-5. The signing script**, printed in full in 6.2:
  ```js
  // Signs a Syneroym login challenge with the temporary key of a session key file.
  // Usage: node sign-challenge.mjs <session-key.json> <assertion>
  import { createPrivateKey, sign } from "node:crypto";
  import { readFileSync } from "node:fs";

  const [keyFile, assertion] = process.argv.slice(2);
  const bundle = JSON.parse(readFileSync(keyFile, "utf8"));
  // A raw 32-byte Ed25519 seed inside the fixed PKCS#8 framing (RFC 8410).
  const pkcs8 = Buffer.concat([
    Buffer.from("302e020100300506032b657004220420", "hex"),
    Buffer.from(bundle.temp_secret_key_hex, "hex"),
  ]);
  const key = createPrivateKey({ key: pkcs8, format: "der", type: "pkcs8" });
  const signature = sign(null, Buffer.from(assertion, "utf8"), key);

  // z-base-32, the encoding Syneroym uses for signatures.
  const alphabet = "ybndrfg8ejkmcpqxot1uwisza345h769";
  let value = 0, bits = 0, out = "";
  for (const byte of signature) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += alphabet[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += alphabet[(value << (5 - bits)) & 31];
  process.stdout.write(out);
  ```
  It is the same encoding as the Hub's `z32Encode` (`login.ts:119`) and
  the same PKCS#8 prefix (`login.ts:140`). Text: sign the `assertion`
  string exactly as the node sent it; never build it yourself.
- **D-A7-6. Errors and unknown fields (6.8).** A table of the codes in 3.2
  (`-32010`, `-32011`, `-32012`, `-32013`, `-32601`, `-32602` with
  `data.field`, `-32001`, `-32002`, `-32603`, HTTP 503 from the node's
  own admission). Q6 (a)'s text on unknown fields. Point to the bad-input
  reference at `:1756` for nested blocks. One more row for a node restart:
  backlog row "A bare substrate restart does not rehydrate the route
  tables" (`deferred-backlog.md:478`) is still open, so a restarted node
  that runs Roym as WASM may serve no `POST /rpc` until Roym is deployed
  again or the supervisor reconciles it. The row gives the exact answer
  Step 2 item 4 sees, and the action (ask the node owner to redeploy, or
  wait for the reconcile). If Step 2 finds the gap closed, leave the row
  out and say so in the PR.
- **D-A7-7. A client on another machine (6.10).** Three sentences and a
  link: the node owner turns on remote access (the remote-access part of
  section 4, ADR-0028); the client uses `https://auth.<domain>` and
  `https://<hub host>`; `local` login is off, so this guide's
  `delegated-key` login is the one to use. Not tested (7.3). The security
  text of D-A7-13 goes here too.
- **D-A7-8. The story script** `crates/substrate/tests/e2e/demo/30-roym-client.sh`:
  1. `source "$(dirname "$0")/lib.sh"`.
  2. A function `guide_block <name>` that prints the body of the one block
     whose info string ends in `roym-client=<name>`, read from
     `$REPO/docs/developer-guide.md` with `awk`. It fails with a message
     that names the block when there is none, or more than one.
  3. Set-up (captions say "Set-up, not part of the guide"): a work
     directory `$DEMO_HOME/client`; Bob's node serves a directory, with the
     recipe that already works in `docs/demo/demo.md` (`:1298-1304`):
     ```bash
     printf 'Be kind. Keep your listings honest.\n' > "$DEMO_HOME/client/rules.txt"
     BOB_HUB_HOST=${BOB_HUB_URL#http://}; BOB_HUB_HOST=${BOB_HUB_HOST%:*}
     rc2 roym directory serve --name "Bob's guild" \
       --rules-file "$DEMO_HOME/client/rules.txt" --category gardening \
       --support support@example.org --dispute "Write to support." \
       --gateway-url "http://127.0.0.1:$S2_GW" --host "$BOB_HUB_HOST"
     DIRECTORY_DID=$(did_of "$S2_DIR" "member-roym#directory-0")
     ```
     `rc2` already passes `--as owner` (`lib.sh:50`), and `--as` takes one
     value ([main.rs:37](../../../../apps/roymctl/src/main.rs#L37)), so the
     snippet does not pass it again. `--host` is Bob's Hub host:
     `BOB_HUB_URL` (story 20 writes it as `http://<host>:<port>`) without
     the scheme and the port.
     Then `SESSION_KEY=$ALICE_KEY`, `PEER_ADDR=$BOB_ADDR` and
     `GATEWAY_PORT=$S1_GW`. `HUB_URL` and `AUTH_URL` come from the tagged
     `address` block (D-A7-3), not from `ALICE_HUB_URL`, so the story tests
     the guide's way of finding the Hub host. The story keeps its values
     in shell variables, not in the env file (`demo_set`), so the env file
     does not grow. The story runs once per `demo:up`, like story 20
     (`directory serve` creates or updates, and `directory.add-source`
     keeps a source that is already there, `client_sources.rs:175`, but
     story 20 itself cannot run twice).
  4. Write the `sign-challenge.mjs` block into the work directory and `cd`
     there.
  5. For each block, `caption "<guide part title>"`, print the block, and
     `eval` it. The `feed` block runs in the background, with its output in
     `feed.txt`, before `open-send`; the script kills it on exit (`trap`).
  6. Checks, each with a message that names the step. Every check on a
     JSON answer uses `jq -e` on the JSON, never a string match on
     formatted `jq` output:
     - after `address`: `$HUB_URL` and `$AUTH_URL` are set, and the
       `session.whoami` call below goes through that `HUB_URL`;
     - after `login`: `$TOKEN` is not empty and not `null` (the block
       checks it too);
     - after `rpc`: `roym session.whoami | jq -e --arg d "$ALICE_DID"
       '.result.did == $d and .result.auth == "delegated"'`
       (`WhoamiOutput { did, auth, app_instance }`,
       [roym_web/app.rs:34](../../../../crates/roym_web/src/app.rs#L34));
     - after `list`: the answer has a `conversations` list;
     - after `open-send`: `$CONV` and `$MESSAGE_ID` are set;
     - `feed.txt` gets a `ready` frame within 10 s, and a `change` frame
       with `"kind":"conversation.message"` and `"item":"$MESSAGE_ID"`
       within 10 s of the send;
     - `catch-up` runs like `feed`: in the background, with
       `CURSOR` set to the `id:` of the `ready` frame, output in
       `catch-up.txt`. Within 10 s the file has the same `change` frame.
       Then the script kills it. (A `curl --max-time` would end with exit
       code 28, which `set -e` turns into a failure.)
     - after `search`: one outcome, for `$DIRECTORY_DID`, with
       `error` `null` (`jq -e`; Step 2 item 2 confirms this for a
       directory with no listings);
     - after `cards`: the answer has a `cards` list;
     - after `refresh-logout`: a `/rpc` call with `$NEW_TOKEN` answers
       `-32010`, and so does one with `$TOKEN`. Both are checked because
       `refresh` does not revoke the old token; only the block's second
       `/logout` does. Optional: `feed.txt`'s `curl` (opened with
       `$TOKEN`) has ended within about 30 s after that logout (one
       keep-alive tick plus margin). Leave this check out if it adds too
       much time; the guide's text stays.
- **D-A7-9. The spec** `30-roym-client.spec.ts`: one test,
  `'Roym client story: follow the client guide with curl'`, that calls
  `runScript('30-roym-client.sh')`. Like story 20, it names the script it
  needs in its error when the env file lacks `ALICE_KEY` (`requireEnv`).
- **D-A7-10. `jq`.** `jq = "1.8"` in `[tools]` of `mise.toml`, beside
  `node`. The guide's 6.1 says that `mise install` installs `jq` and
  `node`, and that `curl` comes with the system.
- **D-A7-11. Verify (Q7 (a′)).** In `xtask/src/verify.rs`, `Gate` gets a
  field `docs_paths_that_run: &'static [&'static str]`: docs paths that
  still run this gate on a docs-only change. `e2e` sets it to
  `&["docs/developer-guide.md"]`, with the comment "a demo story runs the
  client guide's code blocks"; every other gate sets `&[]`.
  `is_docs_only_path` and `is_docs_only_change` stay as they are. The run
  loop computes the changed paths once and skips a gate only when the
  change is docs-only, the gate has `skip_on_docs_only`, and no changed
  path is in its `docs_paths_that_run` (a small `fn skips_on_docs_only(gate,
  paths) -> bool`, so it can be unit tested). The "Docs-only change
  detected" line names the gates it really skips. Unit tests:
  `a_guide_change_runs_only_the_e2e_gate` and
  `other_docs_changes_skip_all_three_gates`.
- **D-A7-12. What a reader needs (6.1).** 6.1 lists these, one or two
  short sentences each, each with the error a reader sees when it is
  missing:
  1. Roym deployed on the node, and the gateway in `identity_mode =
     "login"`. Without login mode every call is anonymous and gets
     `-32010` (developer guide `:1642`).
  2. `roym enrol-signing` run once by the owner, only if Step 2 item 3
     shows that `conversation.send` needs it.
  3. A session key file for the **owner** of the installation. Every Roym
     method needs the owner (except `profile.policy`), so a session of any
     other person gets `-32011`. One installation has one person; there is
     no second user.
  4. The key file lasts `--expires-hours` (default 24,
     [session.rs:31](../../../../apps/roymctl/src/commands/session.rs#L31)).
     After that, `login` answers 401 "delegation certificate has expired".
     Make a new file with `roymctl session delegate`.
  5. The key file is a secret. `roymctl` writes it with mode `0600`
     (`write_secret_file`,
     [commands.rs:233](../../../../apps/roymctl/src/commands.rs#L233)).
     Whoever holds it can sign in as the person until it expires.
  6. `PEER_ADDR` is the other person's conversation service id. They get
     it from `roymctl roym address` ("conversation service id"), or a
     client reads `conversation_address` from their `profile.get`.
  `jq`, `node` (20 or later) and `curl` (the minimum version Step 2 item 1
  finds) are listed too (D-A7-10).
- **D-A7-13. Security text for a client author** (6.2 and 6.10), three
  short sentences:
  1. Use `http://` only for `localhost`. Any other host must be
     `https://`.
  2. The session token and the key file are secrets. Do not log them and
     do not put them in a URL. When other users share the machine, pass
     the header with `curl -H @file` or a `--config` file instead of
     `-H "Authorization: …"`, which `ps` shows. The guide's blocks keep
     `-H "Authorization: Bearer $TOKEN"` for clarity on a one-person
     machine.
  3. A session token cannot be limited to fewer methods: it can do
     everything the person can do on their installation. (Describe this
     in words; the guide cites no backlog row.)

## 6. Step-by-step changes

Each step builds and its tests pass. The whole branch is one PR.

### Step 1 — Commit this plan

`docs(planning): plan M08A slice A7`. Status row A7: "Plan written".

### Step 2 — Check the open facts

On the demo nodes (`mise run demo:up`, then `bash
crates/substrate/tests/e2e/demo/20-roym-setup.sh`, then the Hub part of
story 20 or the same set-up by `curl`), by hand:

1. The `address` block's `HUB_URL` (`http://s<hash>.localhost:<S1_GW>`)
   and `AUTH_URL` reach the gateway (D-A7-3). Do it on macOS and in the CI
   image (`ubuntu-latest`). Look up, in the curl changelog, the first curl
   version that sends `*.localhost` to the loopback address without DNS.
   The guide's curl minimum (6.1, 7.3) is the larger of that version and
   7.76 (`--fail-with-body`): one minimum, not two.
2. `directory.find` on Bob's directory, which has no listings, gives one
   outcome with `error: null`. If it does not, publish one listing in the
   set-up, or accept the outcome the node gives and change the check and
   the guide's text. (The identity name of Bob's directory is known:
   `member-roym#directory-0`, as in `docs/demo/demo.md:1304`.)
3. A `conversation.send` from Alice to Bob's address works without
   `roym enrol-signing` on a fresh owner. If it does not, 6.1 lists
   `roym enrol-signing` as a step of "What you need".
4. After `mise run demo:down` and `demo:up` with no redeploy, what does
   `POST /rpc` on Alice's Hub host answer? Backlog row `:478` says it
   serves nothing. Write the exact answer into the 6.8 restart row
   (D-A7-6), or, if the gap is closed, say so in the PR.

Write each answer into the PR description. A wrong guess becomes a
deviation in the change doc.

### Step 3 — `jq` and the verify rule

1. `mise.toml`: `jq = "1.8"` (D-A7-10). `mise install` locally.
2. `xtask/src/verify.rs`: D-A7-11 and its two unit tests.

Check: `cargo nextest run -p xtask verify`.

### Step 4 — The guide section

Write section 6 (D-A7-1 to D-A7-7, D-A7-12 and D-A7-13) in
`docs/developer-guide.md`. Run each
block by hand on the demo nodes as you write it.

### Step 5 — The story

1. `30-roym-client.sh` (D-A7-8), `chmod +x` like the other scripts.
2. `30-roym-client.spec.ts` (D-A7-9).
3. Break one guide block on purpose (a wrong method name in `list`) and
   see the story fail with a message that names the step. Then put it back.
   Write the result in the PR description.

The story runs once per `demo:up`: run `mise run demo:down` and `demo:up`
before each new run by hand.

Check: `mise run demo` (all stories, in order), then
`mise run test:e2e > target/e2e-run.log 2>&1; echo "exit=$?"; tail -n 15
target/e2e-run.log`.

### Step 6 — Docs and close-out

Sections 8 and 9, the change doc, `task.md`, `status.md`.

### Step 7 — Finish

The import clean-up pass over `xtask/src/verify.rs`; the ratchets;
`mise run verify` with no `--skip`.

## 7. Tests

### 7.1 The demo story (`demo/30-roym-client.sh`, `.spec.ts`)

The checks of D-A7-8 item 6. They cover OF-16: log in (`login`), list
conversations (`list`), send a message (`open-send`), run a search
(`search`), follow the event feed (`feed`, `catch-up`). The story fails if
a named block is missing, if a block's command fails (`set -euo
pipefail`, `curl --fail-with-body`), or if a check fails.

### 7.2 `xtask` unit test

- `a_guide_change_runs_only_the_e2e_gate`: for
  `["docs/developer-guide.md", "docs/planning/deferred-backlog.md"]`,
  `e2e` runs and `nextest` and `doctests` skip.
- `other_docs_changes_skip_all_three_gates`: for
  `["docs/planning/deferred-backlog.md"]`, all three skip.

### 7.3 Not tested

- A client on another machine and over TLS (6.10). Remote access with
  `delegated-key` is covered by `gateway_remote_access_e2e.rs` and Hub case
  42.
- The `transaction.booking` and `transaction.card` rows of the kinds
  table. The story creates no card or booking; the event writers are
  covered by the A5 tests named in OF-8.
- A search that returns hits (Q4 (a)); covered by Hub cases 22 and 23 and
  `dual_build_parity/directory_find.rs`.
- Node versions below 20, and `curl` versions below the minimum that Step
  2 item 1 finds (the larger of 7.76, for `--fail-with-body`, and the first
  version that resolves `*.localhost` itself). The guide names Node 20 and
  that one curl version as the minimum.
- A client after a node restart (the 6.8 restart row). Step 2 item 4
  checks it by hand once.

### 7.4 How A7 avoids the kinds of findings earlier slices had

- **Wrong facts or counts** (A2b R2, A3 R3, A4 R8, A5 R5): every fact in
  section 3 has a link checked on `ac6826f6`; Step 2 checks the four facts
  this plan could not check without running the nodes.
- **Evidence that names tests that do not exist** (A3 R2, A5 R5): OF-16's
  Test column names the story file and its test title after the run.
- **Change-doc decisions that do not match the plan** (A3 R9, A4 R7, A5
  R4): D-A7-1 to D-A7-13 are copied at close-out, not rewritten.
- **Living-doc statements left false** (A4 P22, A5 R9, A6 R1): section 3.9
  lists each doc and why it changes or not.
- **Tests that cannot fail** (A5 R6): Step 5 item 3 breaks a block on
  purpose and sees the story fail.
- **The end-user story and the developer checks miss set-up steps** (A6
  P10): 6.1 lists every thing a reader needs before 6.2 (D-A7-12), and
  Step 2 item 3 checks the one doubtful step.
- **Docs that recommend a setup that does not work** (A6 R1): every
  command in the guide runs in the story, except 6.10, which only links to
  the tested remote-access text.
- **Import-rule breaks** (A4 R9, A5 R10): Step 7.

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `docs/developer-guide.md` | 2 236 lines | About +250. Markdown has no size gate. |
| `xtask/src/verify.rs` | 393 lines | About +40 (the field on 15 gates, the helper, two tests). |
| Demo suite time | story 20 deploys Roym twice | About +20 to 40 s, under the 180 s test limit and the 900 s suite limit. |

## 8. Deferred backlog

Change:
- §10 "Local-only methods refuse unknown fields, which assumes a client
  built with the node" (`:521`): target "Decide in A6 or A7" becomes "TBD,
  with G5" (owner, Q6). Add: "The client guide (developer guide
  section 6) tells clients to send only the listed fields and to read
  `data.field`; the rule stays (owner, Q6)."

Add:
- §10 "The client guide's signing step needs Node.js": a client in another
  language must write the 25-line signing step itself; a `roymctl` command
  or a client SDK could do it. Target `TBD` (with the SDK row `:520`). Link
  `docs/developer-guide.md` section 6.2.

No new `TODO` or `FIXME` marker.

## 9. Living docs to update at close-out

- **Change doc** (`roym-open-foundation/change.md`):
  - OF-16 Test column: `crates/substrate/tests/e2e/demo/30-roym-client.spec.ts`
    (`Roym client story: follow the client guide with curl`), which runs
    the tagged blocks of `docs/developer-guide.md` section 6 through
    `30-roym-client.sh`.
  - OF-1 and OF-2 Test column (empty today,
    [change.md:69-70](../../changes/roym-open-foundation/change.md#L69)):
    fill them from the A1 evidence in [status.md](status.md). Name the test
    functions, not only the files, and check that each still exists and
    passes: `crates/roym_core/src/record/tests/wire_words.rs` (the
    `assert_no_old_words` tests) and the parity harness for OF-1;
    `crates/roym_web/ui/src/cards/wording.test.ts` (the "keeps … free of
    the old words" cases) for OF-2. If that evidence does not cover OF-2's
    "the user docs say participant", say so in Deviations instead of a
    weak claim.
  - Tasks: tick "Write the M08A slice plans" and "Build a Roym client".
  - Decisions: "Q1–Q8 (A7): …" with the owner's answers, and D-A7-1 to
    D-A7-13 as written in section 5.
  - Deviations: F1 (a small Node.js script signs the challenge), F6 (a card
    from the other person is first a message event), and anything Step 2
    finds.
  - Front matter: `status: done`; `living-docs-touched` keeps its list;
    `demo:` adds `crates/substrate/tests/e2e/demo/30-roym-client.spec.ts`
    and `crates/substrate/tests/e2e/demo/30-roym-client.sh`.
  - Close-out section: unchanged (it is the rule, not a status).
- **`docs/developer-guide.md`**: section 6 (Step 4).
- **`docs/roym-integrated-experience-spec.md:156-157`**: add "The
  developer guide's 'Build a Roym client' section shows a client that uses
  this API with `curl`."
- **`docs/planning/traceability-matrix.md`**: new row `[ROY-CLI]` after
  `[ROY-RMT]` (`:97`): "Roym client guide: a developer can log in, list
  conversations, send a message, run a search and follow the event feed
  with the developer guide's 'Build a Roym client' section alone.", M8,
  Implemented, evidence: the section and the story of 7.1.
- **`docs/planning/deferred-backlog.md`**: section 8.
- **`task.md`** A7 result line; **`status.md`** A7 row.
- Not changed, on purpose: `system-requirements-spec.md`,
  `system-architecture.md`, `TERMINOLOGY.md` (3.9), `docs/demo/demo.md`
  (the presenter's script; the story's captions are enough).

## 10. Exit criteria

A7 is done when all of these are true:

1. The owner has answered Q1 to Q8, and sections 4 to 9 match the answers.
2. `docs/developer-guide.md` has `## 6. Build a Roym client` with every part
   of D-A7-1, and every block name of D-A7-2 exactly once. 6.1 carries the
   six items of D-A7-12, and 6.2 and 6.10 carry the three sentences of
   D-A7-13.
3. `mise run demo` passes, and story 30 runs every tagged block (OF-16).
4. Story 30 failed when a block was broken on purpose (Step 5).
5. The four facts of Step 2 are checked and written in the PR description.
6. A docs-only change that includes `docs/developer-guide.md` runs the
   `e2e` gate and still skips `nextest` and `doctests`, with the two tests
   of 7.2.
7. `jq` is in `mise.toml`.
8. The backlog row `:521` has its new target and note, and the new row
   exists.
9. The change doc has OF-16's evidence, the ticked tasks, the A7 decisions
   and deviations, and `status: done`; no Test cell of the criteria table
   is empty (OF-1 and OF-2 included); `cargo xtask check-change-docs`
   passes.
10. The traceability matrix, the integrated spec, `task.md` and
    `status.md` are updated as in section 9.
11. Files changed are only: `docs/developer-guide.md`,
    `docs/roym-integrated-experience-spec.md`, `docs/planning/…` (change
    doc, backlog, matrix, `task.md`, `status.md`, this plan),
    `crates/substrate/tests/e2e/demo/30-roym-client.sh` and `.spec.ts`,
    `mise.toml`, `xtask/src/verify.rs`.
12. `mise run verify` passes with no `--skip`.
13. One PR.

## 11. What A7 does not do

- No client SDK package and no published API contract (OpenRPC or JSON
  Schema).
- No change to any Roym method, the auth service, the gateway, the router
  or the Hub.
- No `roymctl` command for signing (Q2 (b)).
- No change to the unknown-field rule (Q6 (b)).
- No test of a remote client over TLS (7.3).
- No card or booking created in the story (Q5 (c)).
- No milestone close: closing M08A is its own session (milestone close
  template), after A7 is merged.

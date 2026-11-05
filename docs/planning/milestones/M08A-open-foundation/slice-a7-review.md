# M08A slice A7 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a7-implementation-plan.md](slice-a7-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5, 2026-11-05

Checked against the code on `feat/m08a-a7-client-guide` at `d6387ce7`
(`ac6826f6` plus the plan). Work was in the main checkout
`/Users/pari/gitSyneroym/syneroym` (`git rev-parse --git-dir` and
`--git-common-dir` both say `.git`, so it is not a worktree). The first
`git pull` was blocked by the sandbox; the second one, run outside it, said
"Already up to date". The plan author is Claude Opus 5.5 and this reviewer is
Claude Sonnet 5.5, so the "different tool" rule of
[session-strategy.md](../../session-strategy.md) §1 is met.

**Change docs.** The prompt names one change doc:
[roym-open-foundation](../../changes/roym-open-foundation/change.md).
`task.md` names the same doc for A7 and no other. OF-16, the client guide
scope line, the two open task lines and the Close-out section are all in it.
The plan names the right change doc for every criterion and decision.

**Summary:** 0 blocker, 2 major, 9 minor, 11 items. All Open.

**Limits of this review.** I read the code. I did not run the demo nodes,
`curl` or `jq`, and I did not call the auth service. The one thing I ran is
the plan's z-base-32 loop (D-A7-5), in a scratch file: 500 random 64-byte
inputs against a `BigInt` reference encoder gave 0 differences and 103
characters each. I did not re-check every field of `ConversationListItem`,
`MessageView` or `CardRow` (section 3.3 and 3.6); those come from slices that
were reviewed before. The claim about the `curl` version in P8 comes from my
memory of the curl changelog, not from a run.

### What was checked and found correct

- **Login (3.1).** `ChallengeRequest` (`master_did` optional, `#[serde(default)]`)
  and `ChallengeResponse` with `assertion` only when `master_did` is sent, as
  the RFC 8785 canonical string
  ([service.rs:42-60](../../../../crates/auth/src/service.rs#L42),
  `issue_challenge`). `login_delegated_key` takes the nonce once, requires
  `temp_did == delegation.temporary_did`, verifies the certificate, answers
  409 when the master anchor does not resolve, and accepts a signature by the
  temporary key or the master key. The token lasts `min(remaining certificate
  time, session_ttl_secs)`. `DelegatedKeyLoginParams.delegation` is the
  certificate object, so the plan's `jq` body (`delegation: $key[0].certificate`)
  has the right shape. `SessionKeyBundle` is `{temp_secret_key_hex, temp_did,
  certificate}`, and the hex is a 32-byte seed (`[u8; 32]` at
  [session.rs:326](../../../../apps/roymctl/src/commands/session.rs#L326)),
  so the PKCS#8 framing in the script is right. `Identity::sign_json` signs the
  canonical string and encodes it with `z32::encode`
  ([keys.rs:203](../../../../crates/identity/src/keys.rs#L203)).
- **Calling a method (3.2).** `POST /rpc`, the parse errors, `session.whoami`
  answered inside `web`, `admit` and its three codes, `forward_rpc_call`
  ([app.rs:114-217](../../../../crates/roym_web/src/app.rs#L114)) are as the plan
  says. `WhoamiOutput { did, auth, app_instance }` is right, and `session.whoami`
  never returns `-32010` (an unauthenticated call gets `auth: "anonymous"`).
  `select_session_token` tries the cookie first, then `Bearer`, and skips a
  revoked token ([http/auth.rs:59-108](../../../../crates/router/src/route_handler/http/auth.rs#L59)).
- **The `-32010` check after logout (D-A7-8).** Correct. A logged-out token is
  dropped by `select_session_token`; the request then reaches `web` without a
  delegated session; `admit` answers `-32010`. The revocation check is wired in
  production (`init_auth_service` gives the router the auth service as
  `SessionRevocationCheck`, `runtime/router.rs:497-514`).
- **Event feed (3.4).** Frames and data fields are right and compact:
  `{"cursor":…,"kind":…,"service":…,"subject":…,"item":…,"at_ms":…}`
  ([events.rs:98-136](../../../../crates/router/src/route_handler/http/events.rs#L98)),
  so the story's `"kind":"conversation.message"` and `"item":"<id>"` string
  checks will match. The `ready` frame does carry `id:`, so the catch-up check
  can take `CURSOR` from it. The check order, the 16-feed limit
  (`MAX_FEEDS_PER_APP_INSTANCE`) and `Retry-After: 1` are right. A send records
  `conversation.message` with the message id as item on the sender's own node
  (`conversation/lib.rs:500`); `roym_event_feed_e2e.rs:237` already checks it.
- **Search (3.5).** `SearchQuery` has every field defaulted, so
  `{"query":{"text":"plumber"}}` parses. `FindInput.query` defaults too.
  `AddSourceInput` is `{did, label?, issuer_did?}` with `deny_unknown_fields`.
  `roymctl roym directory serve` takes `--name --rules-file --support
  --dispute` (required) and `--category --retention-days --gateway-url --host`.
- **The demo harness (3.7).** `lib.sh` has no `trap`, so the story's own `trap`
  does not replace one. `runScript` is skipped under `DEMO_ATTACH=1`. The
  demo config has `workers: 1` and `timeout: 180_000`. Both nodes run the
  gateway with `gateway: { login: true }`. Story 20 writes `ALICE_*` and
  `BOB_*` (`HUB_URL`, `KEY`, `DID`, `ADDR`). `DEMO_HOME` is in the env file.
- **Verify and CI (3.8).** `is_docs_only_path` is at
  [verify.rs:162](../../../../xtask/src/verify.rs#L162) and the `e2e` gate at
  `:121`. `ci.yml` has no `paths` filter, so "Playwright E2E" runs on every PR.
  `ci-setup` installs mise with `jdx/mise-action`, which installs the tools of
  `mise.toml`; `ubuntu-latest` also ships `jq`. `mise run demo` runs
  `npm run test:demo`. `.github/scripts/check-planning-refs.py` scans only
  comment lines of `.rs`, `.wit`, `.ts` and `.toml` files, so a shell script and
  the guide are not scanned (the rule still applies by hand, see P11).
- **Docs (3.9, section 9).** Backlog rows at `:520` and `:521` have the quoted
  text. The matrix row `[ROY-RMT]` is at `:97`. The integrated spec lines are
  `:156-157`. In the developer guide: The Roym Hub `:1620`, `roym address`
  `:1678-1684`, bad input `:1756`, auth endpoints `:1842`, event feed `:1850`,
  "Demo Stories" `:138-178`, 5.7 Docker `:2227`, file length 2 236 lines. The
  change doc's task lines are at `:208` and `:218`, and OF-16's Test cell is
  empty. `rpc_call` is at `session.rs:561`, not `:560` (nothing depends on it).
- **Both builds.** A7 changes no WASM component, no WIT file, no Roym service
  and no substrate code, so there is nothing to check for the WASM or native
  build. The guide describes the public API, which the dual-build parity tests
  already hold the same in both builds.
- **Order of the steps and size rules.** Steps 1 to 7 build in order. No Rust
  file gets near 800 lines; there is no gate on the guide's length.

### View on each owner question

- **Q1. (a), a story that runs the guide's own blocks.** I agree. It is the only
  option where the guide cannot go stale without a red test. Two gaps keep it
  from being complete: the story must also run the guide's way of finding the
  Hub host (P1), and the blocks must behave the same in a reader's shell as in
  the story's `set -euo pipefail` shell (P9).
- **Q2. (a), the 25-line Node.js script.** I agree. I checked the encoding loop
  and the PKCS#8 prefix (`302e020100300506032b657004220420` is the RFC 8410
  prefix, and it is the same bytes as the Hub's `PKCS8_ED25519_PREFIX`). I did
  not run the script against a live node; Step 5 does that.
- **Q3. (a), `jq = "1.8"`.** I agree. CI gets `jq` from `mise-action` and from
  the runner image. Locally, `mise run demo` puts the mise tools on `PATH`;
  running `bash …/30-roym-client.sh` by hand needs `jq` on `PATH` too, which the
  plan's "How this is shown" text should say in one line.
- **Q4. (a), one real source.** I agree. P3 lists what the plan leaves out
  (the rules file and flags, and whether an empty directory gives `error: null`).
- **Q5. (a), a short "Show cards" part.** I agree. The scope line asks for it,
  and the story only needs the `cards` list to exist. Keep the card-type list
  in the guide next to the "show a plain notice for an unknown type" rule.
- **Q6. (a), keep the rule and say it in the guide.** I agree. Write the new
  target the way the neighbouring rows do, "TBD, with G5", not "G5
  (cross-version compatibility)".
- **Q7. (a), but not as the plan describes it.** The idea is right. The audit
  is incomplete: three gates skip on docs-only, not one (P4). Decide with that
  cost in view, or make the rule narrower.
- **Q8. (a), a new top-level section 6.** I agree. Section 5 is "Milestone 2
  Operational Runbook", so 6 sits after an unrelated runbook; that is fine for
  a stand-alone section that links back to the reference parts.

### Findings

#### P1 — The story does not test the guide's Hub host (major)

*Where:* D-A7-3, D-A7-8 item 3, Step 2 item 1, Q1.

*Problem:* The guide tells the reader to take the Hub host from
`roymctl roym address` (`s<web-did-hash>.localhost`, D-A7-3 and the developer
guide at `:1684`). The story does not do that. D-A7-8 item 3 sets
`HUB_URL` from `ALICE_HUB_URL`, which story 20 wrote from `host_of … --nickname
roym --interface http-native`
([20-roym-setup.sh](../../../../crates/substrate/tests/e2e/demo/20-roym-setup.sh)).
That host has a nickname and an `-i<interface hash>` part and needed a
`registry register` call first. The documented form (no nickname, no `-i`) is
used only by the Rust harness (`web_host_header`) and by Step 2 item 1, which is
a by-hand check that nobody repeats. The variables block is also the one block
the story does not run (D-A7-2). So the thing a new developer types first, the
host name, is the thing no test covers.

*Failure case:* `roymctl roym address` or the `s<hash>` form stops working on
the demo's gateway (for example it needs the registered nickname, or needs
`-i`). The guide is wrong, the story is green, and the first reader gets a 404
or a wrong-service answer.

*Fix:* In the story's set-up, derive `HUB_URL` the way the guide says:
`HUB_URL=http://$(rc1 roym address | sed -n 's/^Hub gateway host: *//p'):$S1_GW`
(with a failure message when the line is missing), and `AUTH_URL` as
`http://auth.localhost:$S1_GW`. Add a check that `session.whoami` through that
URL answers. Better still, tag a small `roym-client=address` block in 6.1 with
the `roym address` command and the `sed`, and run it. Then Step 2 item 1 only
has to repeat the check on macOS and the CI image, not prove it.

*Status:* Fixed (8a59807d). Round 1 fix in 4aa27d01 was reopened in round 2:
the `address` block's `GATEWAY_PORT=7960` overwrote the story's port. The
block now uses `GATEWAY_PORT=${GATEWAY_PORT:-7960}`, and the guide says that
`roymctl roym address` runs as the node owner, so a remote client gets the Hub
host from the owner.

#### P2 — The guide's feed text misses that a feed ends with its session token (major)

*Where:* D-A7-4 `feed`, `catch-up`, `refresh-logout`; D-A7-1 parts 6.5 and 6.9.

*Problem:* The router closes an open feed when the token that opened it expires
or is logged out. It checks on every change and on each 25 s keep-alive tick
(`is_session_expired_or_revoked`, `make_feed_stream`,
[events.rs:160-231](../../../../crates/router/src/route_handler/http/events.rs#L160)).
With the default `session_ttl_secs` that is every 8 hours, and sooner when the
certificate ends first. The plan's text for `feed` says only "keep the last
`id:` you saw" and "`ready` and `reload` mean load your state again". It does
not say that the stream just ends, that this is normal, or that the client must
get a token that is still valid, then reconnect with `Last-Event-ID`.
`refresh` makes it worse: it returns a new token and does **not** revoke the old
one (`AuthService::refresh`, `service.rs:372`), and the feed stays tied to the
old token. A client that refreshes its token but keeps its feed still loses the
feed at the old token's end.

*Failure case:* A developer follows 6.5 literally and builds a long-running
client. After 8 hours (or after any logout) the stream closes with no frame
and no error. The client shows "live" but gets no events, because nothing in the
guide says to reconnect.

*Fix:* In 6.5 add: "The node closes the feed when the session token that
opened it expires or is logged out. Reconnect with a valid token and
`Last-Event-ID: <last id>`. If the answer is `reload`, load your state again."
In 6.9 add: "`refresh` gives a new token. The old token stays valid until it
expires. Log it out too if you want it to stop working. A feed opened with the
old token ends at the old token's end, so open the next feed with the new
token." Say once that the `refresh-logout` story check uses the **refreshed**
token (the old one is not revoked). Optional: have the story check that
`feed.txt`'s stream has ended within about 30 s after the logout (one keep-alive
tick plus margin). If that adds too much time, leave it out and keep the text.

*Status:* Fixed (4aa27d01)

#### P3 — The directory set-up for Q4 (a) is not specified, and one outcome is unverified (minor)

*Where:* D-A7-8 item 3, Step 2, Q4.

*Problem:* Item 3 writes `--rules-file … --support … --dispute …` with dots.
The implementer has to invent the values. A working recipe already exists in
`docs/demo/demo.md` (R5, around `:1299`): a `rules.txt` under `$DEMO_HOME/tmp`,
`--category gardening`, `--support support@example.org`, `--dispute "Write to
support."`, and `did_of "$S2_DIR" "member-roym#directory-0"`. That also answers
Step 2 item 2 (the identity name). Two things are still not checked anywhere:
(1) that `directory.find` against a directory with **no listings** gives
`error: null` and not `not-found`, because the story's one check depends on it;
(2) what a second run does. `directory serve` is "create or update", so it is
safe, but `directory.add-source` on a DID that is already a source may repeat or
refuse. Story 20 cannot run twice on the same nodes either, so a re-run is not
supported today; the plan's Step 5 text "runs only this story" suggests it is.

*Failure case:* The first run in CI gives `not-found` for the empty directory,
or a developer runs the script twice by hand and the second run fails at
`add-source` with a message that does not name the step.

*Fix:* Name the recipe in D-A7-8 item 3 (copy demo.md R5's flags and values) and
move "the identity name" out of Step 2, since it is known. Add to Step 2:
"`directory.find` on an empty directory gives one outcome with `error: null`;
if it does not, publish one listing or accept the outcome the node gives and
change the check and the guide's text." Add one sentence to "How this is shown
to an end user" and Step 5: "The story runs once per `demo:up`. Run
`mise run demo:down` and `demo:up` before you run it again." Also decide
whether the story writes its own values with `demo_set` (the env file grows on
each run) or keeps them in shell variables; prefer shell variables.

*Status:* Fixed (4aa27d01)

#### P4 — Q7's audit counts one gate; `skip_on_docs_only` is on three (minor)

*Where:* Section 2 Q7, section 4 "Behaviour changes", D-A7-11, 3.8.

*Problem:* The plan says that making `docs/developer-guide.md` not docs-only
"runs the e2e gate". `is_docs_only_change` is one switch for every gate with
`skip_on_docs_only: true`: `nextest`, `doctests` and `e2e`
([verify.rs:91, :106, :121](../../../../xtask/src/verify.rs#L91)). After A7,
every guide-only edit (a typo, an unrelated section) runs the whole Rust test
suite and the doctests as well, about 15 minutes more. The guide has about 2 200
lines and many authors, so this will happen often.

*Failure case:* A developer fixes a sentence in section 4 of the guide.
`mise run verify` now takes the full 20 minutes instead of about a minute. The
owner chose (a) from a text that said "one line and one test".

*Fix:* Pick one and write it in the plan: (i) keep (a) and state the cost in
Q7 and section 4, so the owner decides with it; or (ii) make the rule per gate,
for example a `runs_for: &[&str]` list on `Gate` so `e2e` also runs when
`docs/developer-guide.md` or `crates/substrate/tests/e2e/demo/**` changed, and
`nextest` and `doctests` still skip. My view: (ii) is the better result for
about 10 more lines and one more test; (i) is acceptable if the owner prefers
the smaller change. Update D-A7-11, 7.2 and the size table to match.

*Status:* Fixed (4aa27d01)

#### P5 — Closing the change doc leaves OF-1 and OF-2 without evidence (minor)

*Where:* Section 9 "Change doc", exit criterion 9.

*Problem:* A7 sets `status: done`. In the change doc's criteria table the Test
cells of **OF-1 and OF-2** are empty, as well as OF-16
([change.md:69-70](../../changes/roym-open-foundation/change.md#L69)). The plan
fills OF-16 only. `check-change-docs` does not look at the table, so the gate
will not catch it. [status.md](status.md) has the A1 evidence: the parity
harness, `crates/roym_core/src/record/tests/wire_words.rs` and
`crates/roym_web/ui/src/cards/wording.test.ts`.

*Failure case:* The change doc says `status: done` while two of 18 criteria have
no test named. The milestone close audit then has to find them by hand.

*Fix:* Add to section 9 under the change doc: fill OF-1 and OF-2 from the A1
row of `status.md` (check that each named test exists and still passes, and
name the test functions, not only the files). Add to exit criterion 9: "No Test
cell of the criteria table is empty." If the A1 evidence is too thin to support
OF-2 ("the user docs say participant"), say so in Deviations instead of writing
a weak claim.

*Status:* Fixed (4aa27d01)

#### P6 — The guide's prerequisites (6.1) are not listed, and an outside reader needs terms defined (minor)

*Where:* D-A7-1 part 6.1, D-A7-3, 7.4 ("6.1 lists every thing a reader needs").

*Problem:* Section 7.4 says 6.1 lists everything a reader needs, but no
decision lists it. From the code, a reader needs all of these, and a missing one
gives a confusing error:
1. Roym deployed on the node, and the gateway in `identity_mode = "login"`
   (otherwise every call is anonymous and gets `-32010`; guide `:1642`).
2. `roym enrol-signing` run once by the owner, if Step 2 item 3 shows `send`
   needs it.
3. A session key file for the **owner** of the installation. Every Roym method
   is `Owner` level, so a session of any other person gets `-32011`. The guide
   should say there is no "second user" on one installation.
4. The key file lasts `--expires-hours` (default 24). After that, `login` fails
   with 401 "delegation certificate has expired" and the person makes a new file
   with `roymctl session delegate`. The plan says only "never past the
   certificate".
5. The key file is a secret: `roymctl` writes it with mode `0600`
   (`write_secret_file`); whoever holds it can sign in as the person until it
   expires (backlog row `:177`, no attenuation).
6. How a client learns `PEER_ADDR`: it is the other person's conversation
   service id (`roymctl roym address`, or `profile.get`'s `conversation_address`).

The guide is read by someone with no project knowledge. "Master key",
"delegation certificate", "app instance" and "SSE" must be defined where they
first appear (AGENTS.md "Writing for readers").

*Failure case:* A developer points the script at a node with `identity_mode =
"open"`, or uses a key file of another person, and the guide gives them no way
to read the error.

*Fix:* Add a decision D-A7-12 with the six-item list above as the text 6.1 must
carry, and a sentence in D-A7-1 that every term of an outside reader is defined
at first use. Keep each item to one or two short sentences.

*Status:* Fixed (4aa27d01)

#### P7 — The guide has no security text for a client author (minor)

*Where:* D-A7-1 (6.2, 6.3, 6.10), D-A7-7.

*Problem:* The plan's commands put the session token on the command line
(`-H "Authorization: Bearer $TOKEN"`), where `ps` shows it to other users on the
machine. It also uses plain `http://` URLs. That is right for loopback but wrong
for any other machine, because the signature, the token and every message would
cross the network in clear text. 6.10 says only to use `https://` and links to
the remote-access part. A client author who copies the blocks to a phone app has
no warning.

*Failure case:* A developer copies the guide's variables, changes the host to a
public name and keeps `http://`. Tokens travel in clear text.

*Fix:* Add three short sentences: (1) use `http://` only for `localhost`; any
other host must be `https://` (6.2 and 6.10); (2) the session token and the key
file are secrets: do not log them, do not put them in a URL, and prefer
`curl -H @file` or a `--config` file over `-H "Authorization: …"` when other
users share the machine; (3) a token cannot be limited to fewer methods (link
the backlog row `:177` only by describing it, no row number in the guide). Keep
the story's blocks as they are; this is text.

*Status:* Fixed (4aa27d01)

#### P8 — The `curl` minimum is inconsistent with `*.localhost` (minor)

*Where:* D-A7-3, 7.3 ("curl 7.76").

*Problem:* The plan names curl 7.76 as the minimum because of
`--fail-with-body`. The same plan relies on `curl` sending `*.localhost` to
`127.0.0.1` with no DNS. As far as I remember, curl added that special case in
7.78.0 (July 2021), not 7.76. I did not run it. On a system where the resolver
does not map `*.localhost` (some Linux setups without `systemd-resolved` or
`nss-myhostname`), a curl older than that fails with "could not resolve host".
Step 2 item 1 tests only macOS and `ubuntu-latest`.

*Failure case:* A reader on curl 7.76 to 7.77 follows the guide and gets a DNS
error on the first command, although they meet the documented minimum.

*Fix:* In Step 2 item 1, look up the real first curl version that resolves
`*.localhost` and write that one as the minimum in 6.1 and 7.3. Or add `--resolve
"auth.localhost:7960:127.0.0.1"` style options through a `CURL_OPTS` variable
for all older versions. Do not leave two different minimums.

*Status:* Fixed (4aa27d01)

#### P9 — The guide's blocks lean on `set -euo pipefail`, which a reader's shell does not have (minor)

*Where:* D-A7-4 (`login`, `open-send`), D-A7-8 item 6.

*Problem:* The story runs every block under `lib.sh`'s `set -euo pipefail`. A
reader pastes the block into an interactive shell without it. In `login`, the
last step is `… | curl … | jq -r .token`. If `curl --fail-with-body` fails (401,
409), the reader's shell sees only the exit code of `jq`, which succeeds and
prints `null` (error body) or nothing. The story checks "not empty and not
`null`", but only the story; the guide's own text does not tell the reader to
check. The same holds for `CONV` and `MESSAGE_ID` in `open-send`. Two story
checks are also string matches on pretty-printed `jq` output
(`"did": "$ALICE_DID"`), which breaks if the filter changes.

*Failure case:* The reader gets a 401, `TOKEN` is `null`, and every later
command answers `-32010` with no hint where it went wrong.

*Fix:* Put the safety in the block, so the guide and the story run the same
code: start `login` with `set -o pipefail` (and say so in one line), and end it
with `[ -n "$TOKEN" ] && [ "$TOKEN" != null ] || { echo "login failed" >&2; false; }`
(or `jq -e`). Do the same for `CONV` and `MESSAGE_ID`. In the story's checks use
`jq -e` against the JSON, not a string match on formatted text.

*Status:* Fixed (4aa27d01)

#### P10 — The error table does not say what a client sees after a node restart (minor)

*Where:* D-A7-6, 3.4 ("The feed is lost on restart").

*Problem:* The plan covers the lost feed (`reload`), but not the other thing a
long-running client meets. Backlog row `:478` ("A bare substrate restart does
not rehydrate the route tables") says that a restarted, WASM-deployed node
serves no `POST /rpc` until the app is deployed or reconciled again. I did not
check whether a later change has closed it; the plan must check and say which.

*Failure case:* After a node restart, every call from the client fails with
`Method '…' not found in interface …` (the bridge answer) or a 404. The guide's
error table has no row for it.

*Fix:* Step 2 gets a fourth item: "After `mise run demo:down` and `demo:up`
without a redeploy, what does `POST /rpc` answer?" If row `:478` is still open,
add a row to the 6.8 table with the exact answer and the action (redeploy, or
wait for the supervisor's reconcile). If it is closed, say so in the PR.

*Status:* Fixed (4aa27d01)

#### P11 — The guide must not cite planning ids; D-A7-4 invites it (minor)

*Where:* D-A7-4 `search` ("point to the D-A4 fields only by name"), D-A7-6,
section 0 rules.

*Problem:* The rule against planning ids in section 0 lists "code, comments,
scripts or test names". The guide is a living doc, and AGENTS.md's living-docs
rule says living docs "must not cite change docs or milestone/slice IDs; link
ADRs only". `check-planning-refs.py` scans only comment lines of code files, so
nothing will catch a `D-A4-…`, `OF-16` or `A6` in the guide's text. The phrase
"the D-A4 fields" in `search` can be read as a request to name that decision.

*Failure case:* The merged guide says "see D-A4-4" and the id rots when the
planning docs are archived.

*Fix:* In section 0, extend the rule: "Neither the guide nor the story's
captions cite a decision id, criterion id, milestone or slice; ADR ids and
links to other parts of the developer guide are fine." Reword `search` to "name
the fields of `hits`, `outcomes` and `refused` in the guide's own words".

*Status:* Fixed (4aa27d01)

## Plan review round 2 (fixes check) — Claude Sonnet 5.5, 2026-11-05

Checked the plan at `e2b93b95` (fixes in 4aa27d01, close-out in e2b93b95)
against the round 1 items and against the code. I read the whole plan again for
new problems. I did not run the demo nodes, `curl`, `jq` or `cargo`.

**Summary:** 11 round 1 items checked: 10 confirmed, 1 reopened (P1). 2 new
minor items (P12, P13). **0 blocker, 0 major, 3 open (all minor):** P1, P12,
P13.

**Owner answers.** The plan records (a) for Q1 to Q8 (2026-11-05), with the
review's notes, and Q7 as the per-gate rule (a′). I agree with that choice.
D-A7-11 is a sound design: `skip_on_docs_only` and `is_docs_only_change` stay,
a per-gate list can only make a gate run more often, and the run loop already
computes the changed paths once
([verify.rs:253-266](../../../../xtask/src/verify.rs#L253)), so the helper needs
no new git calls.

### Round 1 items

- **P1 — reopened.** The story now finds the Hub host with a tagged `address`
  block (D-A7-2, D-A7-3, D-A7-8 item 3), which is what P1 asked for. The block
  as written sets `GATEWAY_PORT=7960` as its first line, so the story's own
  `GATEWAY_PORT=$S1_GW` is lost when the block is evaluated. Fix: write the line
  as `GATEWAY_PORT=${GATEWAY_PORT:-7960}`, or move it to the untagged variables
  block and let the `address` block only read it. (The guide text then still
  shows `7960` as the default.) Also say in one line that `roymctl roym
  address` needs the master key, so a client on another machine gets the Hub
  host from the node owner (6.10).
- **P2 — confirmed.** D-A7-4 `feed` carries the "closes the feed with no frame
  when the token ends" text with the `events.rs` reference. `refresh-logout`
  now refreshes, then logs out both tokens, says the old token is not revoked
  by `refresh`, and D-A7-8 checks that **both** tokens answer `-32010`. The
  optional "feed has ended within 30 s" check is allowed to be left out. One
  cosmetic point: the `refresh-logout` bullet has a stray "Text: refresh never
  goes past the certificate." in the middle of a sentence; the implementer
  writes the final text, so this is not an item.
- **P3 — confirmed.** D-A7-8 item 3 uses the recipe of `docs/demo/demo.md`
  (lines 1298 to 1304; I checked them), Step 2 item 2 tests the empty
  directory, the identity name is no longer an open fact, the values stay in
  shell variables, and the "once per `demo:up`" rule is in "How this is shown",
  D-A7-8 and Step 5. The claim that `directory.add-source` keeps a source that
  is already there is right (`existing_row` is read and reused,
  [client_sources.rs:172-185](../../../../crates/roym_directory/src/app/client_sources.rs#L172)).
  The snippet has one defect, see P12.
- **P4 — confirmed.** Q7 now states the three-gate cost; (a′) is the owner's
  choice; D-A7-11, 3.8, section 4, 7.2, the size table (393 lines is right) and
  exit criterion 6 all describe the same per-gate rule. The "Docs-only change
  detected" line at `verify.rs:255` is named in D-A7-11, so it will not keep
  saying "skipping nextest, doctests and e2e".
- **P5 — confirmed.** OF-1 and OF-2 are in section 9 and in exit criterion 9.
  One note for the implementer: `assert_no_old_words` is a helper
  ([wire_words.rs:59](../../../../crates/roym_core/src/record/tests/wire_words.rs#L59)),
  not a test. The tests that call it are, for example,
  `the_deal_enums_say_no_old_word_in_any_variant`,
  `a_request_and_a_quote_say_no_old_word` and
  `a_listing_says_no_old_word_for_any_place_or_audience`. The Hub side
  (`wording.test.ts`, "keeps … free of the old words") matches the plan.
- **P6 — confirmed.** D-A7-12 has the six items, and D-A7-1 asks for every
  term to be defined at first use. I checked the references: `--expires-hours`
  default 24 is at `session.rs:31`, `write_secret_file` at `commands.rs:233`,
  and `profile.get` returns `conversation_address`.
- **P7 — confirmed.** D-A7-13 has the three sentences and is placed in 6.2 and
  6.10 (D-A7-3, D-A7-7). It cites no backlog row.
- **P8 — confirmed.** Step 2 item 1 now finds the first curl version that
  resolves `*.localhost` and uses the larger of that and 7.76 as the one minimum;
  7.3 and D-A7-12 say the same.
- **P9 — confirmed.** `login` and `open-send` start with `set -o pipefail` and
  end with a check that fails with a message; the story's checks use `jq -e`.
- **P10 — confirmed.** D-A7-6 has the restart row, Step 2 item 4 reads the real
  answer, and 7.3, 7.4 and exit criterion 5 say "four facts". I looked for a
  change that rehydrates the route tables on boot and found none, so the plan's
  statement that row `:478` is still open fits the code; Step 2 item 4 is still
  the proof.
- **P11 — confirmed.** Section 0 now bans decision, criterion, milestone and
  slice ids in the guide and the story's captions, and `search` says to name
  the fields in the guide's own words.

### New findings

#### P12 — The directory set-up snippet passes `--as owner` twice (minor)

*Where:* D-A7-8 item 3.

*Problem:* The snippet is `rc2 --as owner roym directory serve …`. `rc2` in
[lib.sh](../../../../crates/substrate/tests/e2e/demo/lib.sh) already expands to
`roymctl … --as owner "$@"`. `--as` is a single-value global option
([main.rs:37](../../../../apps/roymctl/src/main.rs#L37)), and clap refuses an
option that is given twice. I did not run it. The snippet's note "copy the exact
`rc2` and `--host` form from demo.md and story 20" does not help, because
demo.md writes `roymctl --dir … --as owner` (no `rc2`) and story 20 uses its
own `ctl` array.

*Failure case:* The implementer pastes the snippet. `directory serve` fails
with an argument error and the story stops before the first guide block.

*Fix:* Write the snippet as `rc2 roym directory serve --name …` (no `--as`).
Say that `--host` is Bob's Hub host, taken from `BOB_HUB_URL` without the
scheme and the port.

*Status:* Fixed (8a59807d)

#### P13 — Step 4 lists the wrong decisions for the guide (minor)

*Where:* Step 4 ("Write section 6 (D-A7-1 to D-A7-7)").

*Problem:* The text of 6.1 (what a reader needs) is D-A7-12, and the security
text of 6.2 and 6.10 is D-A7-13. Both were added after D-A7-7. Step 4 still
stops at D-A7-7, and exit criterion 2 checks only "every part of D-A7-1". An
implementer who follows Step 4 literally can skip both.

*Failure case:* The merged guide has no prerequisites list and no `https`
warning, and no exit criterion catches it.

*Fix:* Step 4: "D-A7-1 to D-A7-7, D-A7-12 and D-A7-13". Exit criterion 2: add
"6.1 carries the six items of D-A7-12, and 6.2 and 6.10 carry the three
sentences of D-A7-13".

*Status:* Fixed (8a59807d)

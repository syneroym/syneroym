# M08A slice A6 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-a6-implementation-plan.md](slice-a6-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Opus 5.5 (Claude Code, cloud session), 2026-11-04

Checked against the code on `feat/m08a-a6-remote-clients` at `ef1bdd7`. The
plan says its links are to `main` at `7afb186`; `ef1bdd7` adds only the plan,
so the line numbers are the same. `git pull` said "Already up to date". This
was a cloud checkout at `/home/user/syneroym`, not the path named in the
prompt, and not a git worktree.

**Who reviewed.** The prompt named the reviewer as Claude Sonnet 5.5 and the
plan author as Claude Opus 5.5. This session runs on Claude Opus 5.5 (checked
with the session tool). So the author and the reviewer are the same model.
[session-strategy.md](../../session-strategy.md) §1 says the plan reviewer must
be a different tool from the plan author. This round does not meet that rule.
The title above and the `status.md` row name the model that did the work. The
owner may want a second round by a different tool.

**Change docs.** The prompt names one change doc:
[roym-open-foundation](../../changes/roym-open-foundation/change.md).
`task.md` names the same doc for A6 and no other. OF-12, OF-13, problem 4, the
"Remote clients" design section and the task line are all in it. The plan names
the right change doc for every criterion and decision.

**Summary:** 0 blocker, 2 major, 12 minor, 14 items. All Open.

**Owner answers (2026-11-04).** The owner agreed with the reviewer's view on
every question below: (a) for Q1 to Q9, each with the notes given there, and
(a) for the new Q10 from P1 (a startup warning). The answers are recorded under
each question in the plan, and Q10 is added to the plan's section 2. The
findings stay Open: the plan fix folds them into sections 4 to 10.

Limits of this review: I read the code and did not run it. I did not run a
browser. Statements about CSP `<meta>` tags added by script, and about which
proxies add `X-Forwarded-For` by default, come from the specs and the proxies'
documented defaults, not from a run here.

### What was checked and found correct

- **The gateway (3.1).** Bind `0.0.0.0:<port>`
  ([gateway.rs:184-190](../../../../crates/client_gateway/src/gateway.rs#L184)),
  the peer address only logged (:203-204), port 7000 with no role (:100),
  `ClientGateway { port, state, shutdown_tx }` (:74-78), one
  `forward_to_service` per connection (:408-487), the target from `Host` or the
  `/_syneroym/session` prefix (:327-340), fixed delegation on every stream
  (:535-538), one signed preamble (:540-551). `ClientGateway::init` runs only
  when the role is present
  ([runtime/services.rs:148-153](../../../../crates/substrate/src/runtime/services.rs#L148)),
  and its error stops startup. `default_http_port()` is 7960
  ([roles.rs:384](../../../../crates/core/src/config/roles.rs#L384)).
- **The auth service (3.2).** `login_local` (service.rs:343-416), `methods()`
  (:418-424), `is_allowed_origin`/`cors_headers` (:516-549), the `Origin`
  refusal (:622-630), `_caller` ignored (:714-719), `node_did` kept (:113).
  `handle_request` is the only production caller of `login_local` (:637).
- **The caller (3.3).** `caller_did` is the master DID with a delegation, else
  the signer ([io.rs:242-249](../../../../crates/router/src/route_handler/io.rs#L242)).
  `is_gateway_origin` is "node DID and no delegation"
  ([http/auth.rs:113-115](../../../../crates/router/src/route_handler/http/auth.rs#L113)).
  The effective caller replaces the transport caller before dispatch
  ([http.rs:292-299](../../../../crates/router/src/route_handler/http.rs#L292)),
  so B4 is right. The auth service's `node_did` is `host.node_did`
  ([runtime/router.rs:138](../../../../crates/substrate/src/runtime/router.rs#L138), :495),
  the same DID the gateway signs with. Service-instance certificates have the
  service DID as master
  ([cert.rs:32](../../../../crates/control_plane/src/service/orchestration/cert.rs#L32)),
  so no production delegation has the node key as master; dropping the
  "no delegation" half of the router's rule in the auth gate is safe.
- **Nothing else grants a right to "caller is the node DID"** except
  `grant_resolve_to_node_did`, which compares with the receiving node's own
  DID ([io.rs:195](../../../../crates/router/src/route_handler/io.rs#L195)).
- **The Hub (3.4).** `authOrigin()` (login.ts:30-35), the static CSP
  (index.html:14), `main.ts` imports, no module-level `fetch`. Hub fetches to
  the auth origin send no `credentials` option. Case 41 is the last numbered
  case in every spec file, so 42 is free. Playwright runs `workers: 1`,
  `fullyParallel: false`. Cases at roym-hub.spec.ts:214, :398, :580 and :858
  read `AUTH_ORIGIN`, so they keep passing with the exact origin. The feed
  cookie already adds `Secure` on HTTPS (login.ts:54).
- **Config (3.5).** `ClientGatewayRole`, `AuthRole` and their defaults are as
  described; `ClientGatewayRole` has no `deny_unknown_fields`, so an older
  binary ignores the new table (W2 is right). All five struct literals use
  `..Default::default()` or `..gateway`.
- **Callers of `local`.** `RoymNode::login` (roym.rs:452-463),
  `roym_app_e2e.rs:345`, `roym_identity_e2e.rs:257-268`, the auth unit tests.
  All go through the gateway on loopback, so they keep working.
  `http_interface_endpoints_and_unknown_method` posts `local` with
  `caller = None` (auth_service_tests.rs:389-399) and must change. No other
  test or tool reaches the gateway from a non-loopback address.
- **Harness.** `NodeBuilder::configure` (node.rs:214), `init_error` (:337),
  `SubstrateNode::builder` (:438), `client_as` (:500),
  `roym_artifacts_present` (roym.rs:60). `common/mod.rs` has
  `#![allow(dead_code)]`, so a new `common/session.rs` adds no warnings.
- **Docker (3.6).** The compose ports, volumes and command; the `Dockerfile`
  port comment and `CMD ["run"]`. No community config in the repo.
- **Living-doc lines (3.7) and backlog rows (§8)** are at the cited lines with
  the cited text (backlog :330, :342, :514; spec :77, :174, :849; architecture
  :375, :694, :711, :764; developer guide :236, :1840, :1870; SRS :623;
  traceability :96). ADR number 0028 is free.
- **Sizes (7.10).** Every line count is right. `gateway_session_e2e.rs` is
  listed at 1 477 in `xtask/oversized-test-files.txt:27`.
- **Both builds.** A6 changes no WASM component, no WIT file and no Roym
  service. The auth service and the gateway are native only. Nothing to check
  for the WASM build.

### View on each owner question

- **Q1. (a), the optional table.** I agree. Two notes in P14: the other bind
  settings in this config are `host:port` strings, and the table should refuse
  unknown fields.
- **Q2. (a), three checks.** I agree with checks 1 and 2; together they close
  F3 and match the change doc's design. Check 3 is a cheap net, but it catches
  fewer proxies than the plan says (P6). Keep it, and do not let the docs lean
  on it.
- **Q3. (a), a second `<meta>` tag.** I agree, if the plan names what to do
  when case 42 shows the second tag is not applied (P9).
- **Q4. (a), a subdomain pattern.** Acceptable today, with a correction to the
  risk text (P11). The gateway serves the pages of any service in the
  registry, from any node, under the node's domain. So the pattern admits
  strangers' pages, not only "app pages" on this node. It is safe now only
  because the Hub never sends credentials to the auth origin, so the auth
  cookie is never set by the Hub. If the owner is not sure, (b) (exact entries)
  costs one config line per Hub and has no such risk.
- **Q5. (a), refuse `fixed` with remote access.** I agree. The same reasoning
  also applies, more weakly, to `open` and `login` mode (P1).
- **Q6. (a), a warning.** I agree. The Hub cannot sign in over plain HTTP
  anyway, so a refusal would add little.
- **Q7. (a), public.** It matches the change doc's design. It needs a complete
  example config, or the deploy breaks in other ways (P2).
- **Q8. (a), ADR-0028.** I agree. The ADR should also record P1's consequence
  and the CORS rule of Q4.
- **Q9. (a), `none:` with a reason.** I agree. Case 42 shows the remote-domain
  path better than a story could.

### Findings

#### P1 (major) — Remote access in `open` or `login` mode lets every network client act as this node

- **Where:** plan Q5, D-A6-4, D-A6-13 (ADR content), section 4 (no row),
  section 9 (docs).
- **Problem:** The gateway signs every stream with the node key
  ([gateway.rs:511-551](../../../../crates/client_gateway/src/gateway.rs#L511)).
  In `open` mode (the default) and in `login` mode without a session, there is
  no delegation, so the target sees the caller as this node's DID,
  self-asserted ([io.rs:242-249](../../../../crates/router/src/route_handler/io.rs#L242)).
  The gateway also reaches services on other nodes, resolved from the registry
  (:645-660). Today only programs on the same machine can do this (the spec
  intent). With remote access, every client on the network can do it, unless
  `login` mode with `connection_auth_gate = true` is set
  ([gateway.rs:455-461](../../../../crates/client_gateway/src/gateway.rs#L455)).
  The plan looks at this risk only for `fixed` mode (Q5).
- **Failure case:** Alice turns on remote access with the default
  `identity_mode = "open"`. A stranger on her network sends requests through
  her gateway to a directory on another node. Each request arrives as Alice's
  node DID, so it uses Alice's per-caller quota on that directory
  (`directory.publish` is recorded and rate-limited by caller DID, change doc
  B53). A service that trusts or blocks by node DID now sees the stranger as
  Alice's node. Nothing at startup, in the ADR or in the docs tells Alice.
- **Fix:** Add an owner question Q10: "Remote access with `open` mode, or with
  `login` mode and no `connection_auth_gate`?" Options: (a) a startup `warn!`
  that names both settings and says "every client on the network acts as this
  node"; (b) refuse, as for `fixed`; (c) nothing. Recommend (a), because the
  public community node (Q7) is `open` on purpose. Add the row to section 4.
  Write the consequence in ADR-0028 and in the developer guide's remote-access
  text, and recommend `login` with `connection_auth_gate = true` for a personal
  node. If (a): one test in `gateway/tests.rs` that `init` succeeds in `open`
  mode with remote access. A warning text cannot be checked without a log
  capture, so test only that startup does not fail.
- *Status:* Open.

#### P2 (major) — The Docker example config is not specified enough to work, and the test cannot see it

- **Where:** D-A6-11, 7.7, exit criterion 8.
- **Problem:** D-A6-11 lists the roles and says "Nothing else". A working
  config for this compose file needs more:
  1. `app_data_dir = "/var/lib/syneroym"`. The default is `dirs::data_dir()`
     ([config.rs:36-38](../../../../crates/core/src/config.rs#L36)), which in
     the container is `/root/.local/share/syneroym`. That is not the mounted
     `./data` volume. The node key is written there
     ([util.rs:20-35](../../../../crates/core/src/util.rs#L20)).
  2. `[substrate] registry_url` pointing at the node's own registry. With none,
     the gateway's `registry_url` is empty
     ([gateway.rs:101](../../../../crates/client_gateway/src/gateway.rs#L101)),
     so it cannot resolve any hostname. `dev_mode_config()` sets one; a config
     file must too.
  3. The coordinator TLS paths under `/etc/letsencrypt` (commented), because
     QUIC address discovery runs only with relay TLS
     ([system-architecture.md:375](../../../system-architecture.md#L375)).
  Also, the `Dockerfile` `EXPOSE 7965` is TCP. D-A6-11 changes only its
  comment.
- **Failure case:** An owner copies the example and runs `docker compose up`.
  The node works. After `docker compose down && up` (or an image update) it
  has a new node DID, because the key was in the container layer. Or, without
  `registry_url`, every request to an app hostname gets `502 Bad Gateway`.
  Test 7.7 parses the file and checks three fields, so it passes in both cases.
- **Fix:** In D-A6-11, list every key of the example file: `app_data_dir` (and
  the other `app_*_dir` keys, or say why they may stay default),
  `substrate.registry_url`, the relay URL the node uses, the four roles, the
  commented TLS block. In 7.7, also assert that after `resolve_paths()` every
  path is under `/var/lib/syneroym`, `/etc/syneroym` or `/etc/letsencrypt`,
  and that `substrate.registry_url` is set. Change the `Dockerfile` to
  `EXPOSE 7964 7965/udp 7961 7960`, and extend the text check of 7.7 to it.
- *Status:* Open.

#### P3 (minor) — The OF-12 deviation is not in the plan's Deviations list

- **Where:** section 9, "Deviations"; OF-12; `task.md` A6 ("refuses
  non-loopback peers with an error that names `delegated-key`").
- **Problem:** OF-12 says `local` login checks the socket peer address and a
  refused request gets an error naming `delegated-key`. In the plan, the
  gateway checks the peer at accept and drops the connection with no answer
  (D-A6-2). The auth service never sees the peer address; it checks the caller
  DID. The result meets the rule, but the mechanism and the "error that names
  `delegated-key`" differ for that case.
- **Failure case:** A reviewer at close-out reads OF-12 and looks for an
  error body for a non-loopback peer, and does not find one.
- **Fix:** Add a Deviations item: "OF-12: the peer address is checked by the
  gateway at accept, for the whole connection; a non-loopback peer is dropped
  with no answer (it cannot occur with the loopback bind). The auth service
  checks that the caller is this node's own gateway. Every refusal the auth
  service sends names `delegated-key`."
- *Status:* Open.

#### P4 (minor) — `local` login through the gateway in `fixed` mode becomes a refusal; the audit does not say so

- **Where:** section 4 (B table), D-A6-7 check 5.
- **Problem:** In `fixed` mode the gateway attaches the fixed delegation to
  every stream, including the stream to the auth service
  ([gateway.rs:535-538](../../../../crates/client_gateway/src/gateway.rs#L535)).
  So the caller is the fixed person's master DID, not the node DID, and check 5
  refuses `local`. Today it works.
- **Failure case:** An owner runs a node in `fixed` mode with
  `person_identities_dir` set and uses `roymctl session login --method local`.
  After A6 it gets `403 "local login is only for a client on this node's own
  machine"`. The text is wrong for that person, who is on the machine.
- **Fix:** Add a B row: "In `fixed` mode, `local` login is refused, because
  every stream carries the fixed person as caller." Either accept it (there is
  no use for `local` in `fixed` mode) and add one auth test with a caller whose
  DID is not the node's, or give check 5 a text that does not claim the client
  is on another machine, for example "local login works only through this
  node's own gateway in `open` or `login` mode; use delegated-key".
- *Status:* Open.

#### P5 (minor) — Two facts in section 3 are wrong or incomplete

- **Where:** 3.2 "Callers of `local` login", W4, and 3.3.
- **Problem:**
  1. `roymctl session login` uses `local` only with `--method local`. The
     default is `delegated-key`
     ([session.rs:45](../../../../apps/roymctl/src/commands/session.rs#L45),
     :432-438). `--as` alone runs a delegated-key login. So W4's "`roymctl
     session login --as` against it" is not a broken path; `--method local
     --as` is. The Playwright global setup (`--as owner session login`,
     global-setup.ts:171) and the developer guide example (:1807) are
     delegated-key and keep working with remote access.
  2. 3.3 says the gateway is the only part that signs with the node key. The
     outbound proxy also signs native calls with the node key when a call has
     no service id
     ([proxy/router.rs:368-374](../../../../crates/router/src/proxy/router.rs#L368)).
     Those calls use a JSON-RPC preamble, not HTTP, so they cannot reach the
     auth service's HTTP handler. The plan should say so, because check 2
     rests on this fact.
- **Failure case:** The implementer tests `--as` against a remote-access node,
  sees it work, and thinks W4 is not implemented. Or a later reader of 3.3
  misses the proxy path when the proxy gains HTTP calls.
- **Fix:** Correct 3.2 and W4 (`--method local`). Add the proxy path to 3.3
  with the reason it cannot reach `auth`. Put the same one-sentence reason in
  ADR-0028.
- *Status:* Open.

#### P6 (minor) — The proxy-header check catches fewer proxies than the plan says

- **Where:** Q2 (a) check 3 ("Caddy and most proxies add `X-Forwarded-For` by
  default"), D-A6-7 check 4, section 9 docs.
- **Problem:** Caddy, Traefik and Apache `mod_proxy` add `X-Forwarded-For` by
  default. nginx `proxy_pass` does not, unless the config sets it. HAProxy
  needs `option forwardfor`. `ssh -L`, `socat` and `kubectl port-forward` copy
  bytes and add no header. (`unverified` here; from the tools' documented
  defaults.)
- **Failure case:** An owner puts nginx in front of the node with the default
  `proxy_pass` and forgets remote access. Every request reaches the gateway
  from loopback with no proxy header. Anyone who reaches nginx gets a `local`
  token for any key file name in `person_identities_dir`.
- **Fix:** In Q2 and the ADR, say the check catches some proxies only. In the
  developer guide, say: "Any program that forwards connections to the gateway
  (a TLS proxy, an SSH tunnel, a port forwarder) needs remote access on, even
  with `bind_address = "127.0.0.1"`." Keep the check.
- *Status:* Open.

#### P7 (minor) — Some planned tests cannot be written as described

- **Where:** 7.3 and 7.4.
- **Problem:**
  1. `local_login_over_iroh_from_another_identity_is_refused` says
     `node.client_as(..)` then `passthrough` to the `auth` alias.
     `SyneroymClient::passthrough` always targets `self.service_id`
     ([passthrough.rs:106-123](../../../../crates/sdk/src/client/passthrough.rs#L106)),
     which for `client_as` is the node DID. To target `auth`, the test must
     call `passthrough_with_conn(client.connection(), AUTH_SERVICE_ALIAS, ..,
     &identity, None)`, as the gateway does.
  2. 7.3 says the origin pattern is "checked through `cors_headers`".
     `cors_headers` is private, and `crates/auth/tests/` is an integration test
     crate. It must go through `handle_request` with an `Origin` header.
  3. The new `local_login_gate.rs` needs `MockAnchorResolver`, which lives in
     `auth_service_tests.rs:32`, and the same 8-field `HttpRequest` literal.
     Copying them raises the duplication gate.
  4. The CORS pattern tests are not about the `local` gate, but sit in
     `local_login_gate.rs`.
- **Failure case:** The implementer cannot find a way to send the F3 request
  to `auth`, or copies the mock and fails `check-duplication`.
- **Fix:** (1) name `passthrough_with_conn` in 7.4. (2) say "through
  `handle_request` with an `Origin` header; read the reflected
  `access-control-allow-origin`", or test `matches_origin_pattern` as an
  in-crate unit test in `service/cors/tests.rs`. (3) add
  `crates/auth/tests/common/mod.rs` with `MockAnchorResolver`, a service
  builder with a temp identities dir, and a `login_request(headers)` helper;
  use it from both files. (4) put the CORS cases in their own file or in the
  in-crate tests.
- *Status:* Open.

#### P8 (minor) — No test shows that `run` binds the configured address

- **Where:** 7.2 `init_binds_loopback_by_default_and_the_remote_access_address_when_set`,
  exit criteria 2 and 3.
- **Problem:** The test reads `bind_addr()`, a getter set in `init`. It does
  not show that `run` binds that address. A `run` that still formats
  `"0.0.0.0:{port}"` passes it. The e2e tests in 7.4 all connect on
  `127.0.0.1`, which works with either bind.
- **Failure case:** A later edit to `run` binds `0.0.0.0` again. Every test
  passes, and the node is open to the network.
- **Fix:** Split the bind out of `run`: `async fn bind_listener(&self) ->
  Result<TcpListener>`. Unit-test it with `http_port = 0` (no port literal; the
  OS picks the port) and assert `listener.local_addr()?.ip()` is `127.0.0.1`
  with no table and `0.0.0.0` with `bind_address = "0.0.0.0"`. `run` calls
  `bind_listener` and nothing else binds.
- *Status:* Open.

#### P9 (minor) — The CSP plan has no fallback, and one check proves nothing

- **Where:** Q3 (a) "Risk", D-A6-10, 7.6 case 42 step 2.
- **Problem:** The plan says the second `<meta>` tag is `unverified` until case
  42 runs, but not what to do if it fails. Also, step 2's "the `fetch`
  rejects" is not proof: a cross-origin `fetch` to `other.roym.test` also
  rejects for lack of CORS headers, with the wide static tag alone. Only the
  `securitypolicyviolation` event shows the narrowing.
- **Failure case:** Chromium ignores the script-added tag. Case 42 fails late
  in the slice, and the implementer must choose a design alone. Or a version
  of the test that checks only the rejection passes with the wide policy.
- **Fix:** Name the fallback in Q3: if case 42 step 2 fails, use Q3 (b) (a
  header from the router) and record a deviation, or keep (c) only with the
  owner's approval. In 7.6 step 2, say the event is the assertion, and the
  rejection alone is not. Write case 42 first in Step 5, before the other
  items, so a failure is known early.
- *Status:* Open.

#### P10 (minor) — The end-user story and the developer checks miss setup steps

- **Where:** "How this is shown to an end user"; the developer bullets.
- **Problem:**
  1. A Hub on `roym-….example.com` calls `auth.example.com` with CORS. The auth
     service allows only `*.localhost` and the exact `allowed_origins` entries
     ([service.rs:516-530](../../../../crates/auth/src/service.rs#L516)). So
     Alice must also add `allowed_origins` (Q4) before she can sign in. The
     story does not say so. With HTTPS she should also set
     `secure_cookies = true`.
  2. The TLS proxy needs a certificate for `*.example.com`, because every app
     has its own hostname, and `auth.example.com`. A wildcard certificate needs
     a DNS challenge in Caddy. This belongs in the developer guide text.
  3. Developer bullet 2: the remote-access `403` text appears only when
     `person_identities_dir` is set. Without it, check 1 answers `400` "local
     login method is not enabled". `curl -s` also prints no status code.
- **Failure case:** Alice follows the story, opens the Hub, and the sign-in
  fails with a CORS error in the browser console.
- **Fix:** Story bullet 2: "She turns on remote access, lists the Hub's
  address in the node's allowed web addresses, and puts a TLS proxy in front."
  Developer bullet: "on a node with `person_identities_dir` set", and add
  `-w ' %{http_code}\n'`. Section 9: the developer guide's remote-access text
  names `allowed_origins`, `secure_cookies` and the wildcard certificate.
- *Status:* Open.

#### P11 (minor) — The new CORS backlog row describes the risk too narrowly

- **Where:** section 8, row "The auth service answers credentialed CORS for
  every allowed origin"; Q4 "Reason".
- **Problem:** The row and Q4 speak of "every app page on that domain". The
  gateway forwards any `-s<hash>` hostname to whichever node hosts that
  service ([gateway.rs:645-660](../../../../crates/client_gateway/src/gateway.rs#L645),
  :501-525). So a page of any service in the registry, made by anyone, is
  served under `<domain>` and matches `*.<domain>`. Today the risk is small for
  a different reason: the Hub's auth fetches send no credentials, so the
  browser never stores the auth cookie from them. The row should say both.
- **Failure case:** A later slice adds `credentials: "include"` to a Hub auth
  call (for cookie refresh, say). Then a stranger's page at
  `evil-s<hash>.example.com` can `fetch` `auth.example.com/_syneroym/session/refresh`
  with the cookie and read a new token for Alice. The backlog row does not warn
  that slice.
- **Fix:** Rewrite the row: "Any page served through this node's gateway under
  an allowed domain, including other people's services, may make credentialed
  calls to the auth service. Safe while no client logs in with credentials.
  Before any client does, stop sending `access-control-allow-credentials`, or
  limit it to exact origins." Link
  `crates/auth/src/service/cors.rs` and `gateway.rs:645`. Copy the same
  sentence into ADR-0028's consequences.
- *Status:* Open.

#### P12 (minor) — Two living-doc places are missing from section 9

- **Where:** sections 3.7 and 9.
- **Problem:** The SRS describes the three identity modes
  ([system-requirements-spec.md:112-115](../../../system-requirements-spec.md#L112)).
  After A6, `fixed` with remote access stops startup, and (with P4) `local` is
  refused in `fixed` mode. The architecture doc's `identity_mode` section
  ([system-architecture.md:696](../../../system-architecture.md#L696)) has the
  same gap, and P1's consequence belongs there.
- **Failure case:** A reader of the SRS configures `fixed` with remote access
  and gets a startup error the SRS does not mention.
- **Fix:** Add SRS:112-115 and architecture :696 to 3.7 and section 9, with
  the sentence each must say.
- *Status:* Open.

#### P13 (minor) — `login_local` stays public and ungated

- **Where:** D-A6-7 ("`login_local` itself keeps its signature"), D-A6-8.
- **Problem:** The gate is in `handle_login`. `login_local` stays `pub` and
  mints a token with no gate
  ([service.rs:343](../../../../crates/auth/src/service.rs#L343)). It is kept
  public only for tests in `crates/auth/tests/`.
- **Failure case:** A later change calls `auth_service.login_local(name)` from
  a new path (an RPC method, a CLI hook). That path has none of the five
  checks, and nothing fails.
- **Fix:** Make it `pub(crate)` in `service/local.rs`. Move the cases of
  `local_login_and_refusals` that call it directly into an in-crate test
  (`service/local/tests.rs`), or rewrite them through `handle_request` with a
  node caller. Then the gated path is the only public one.
- *Status:* Open.

#### P14 (minor) — The new config table: the address form and unknown fields

- **Where:** Q1 (a), D-A6-1, 7.1.
- **Problem:**
  1. Every other bind setting in this config is a `host:port` string
     (`http_bind_address`, `quic_bind_address`, `signalling_bind_address`,
     [roles.rs:44, :312-313, :353](../../../../crates/core/src/config/roles.rs#L312)).
     `bind_address` here is an IP only. An owner who writes
     `bind_address = "0.0.0.0:7960"` gets a parse error.
  2. `RemoteAccess` has no `deny_unknown_fields`. A later field with a typo
     (for example a TLS setting) would be ignored.
  3. A typo in the table name (`[roles.client_gateway.remote_acess]`) is
     ignored, because `ClientGatewayRole` has no `deny_unknown_fields`. That
     fails closed (loopback), so it is safe; the D-A6-3 log line is what tells
     the owner.
- **Failure case:** (1) The owner sees a serde error "invalid IP address
  syntax" and does not know the port belongs in `http_port`.
- **Fix:** Keep the IP form, and name it in the doc comment and the sample
  config: "an IP address only; the port is `http_port`". Add
  `#[serde(deny_unknown_fields)]` to `RemoteAccess`. Add a config test
  `a_bind_address_with_a_port_fails_to_parse`. In the developer guide, say
  that a misspelled table name leaves remote access off.
- *Status:* Open.

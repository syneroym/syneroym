# M08A slice A6 — Remote clients: implementation plan

*Reader: the developer (or AI) who implements slice A6. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written 2026-11-04. The owner has not answered Q1 to Q9 yet.
Sections 4 to 10 follow the recommendation of each question. If the owner
picks another option, change the sections that the question names.**

**Size and PR.** S to M. One PR on `feat/m08a-a6-remote-clients`, with the
commits of section 6 in that order. The branch starts from `main` at
`7afb186` (A5 is merged). A6 changes the client gateway's bind address and
adds one config table, adds a gate to the `local` login in the auth service,
lets `allowed_origins` hold a subdomain pattern, moves the Hub's
`connect-src` rule to run time, adds one example config for the Docker
deploy, and adds tests. It changes no JSON-RPC method, no WIT file and no
Roym service. Section 10 lists exactly which `src/` trees change.

All links are to `main` at `7afb186`.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - Functions of at most 100 lines. Match arms of at most 10 lines. Rust
    files of at most 800 lines. `crates/auth/src/service.rs` is 789 lines
    today, so new auth code goes into sibling files (D-A6-8). Section 7.10
    lists every file that grows.
  - The oversized-test-file list is a ratchet.
    `crates/substrate/tests/gateway_session_e2e.rs` is listed at 1 477 and
    must not grow. Step 4 moves one helper out of it, so its entry goes down.
  - No planning ids in code: no "M08A", "A6", "OF-12", "D-A6-3" in code,
    comments or test names. ADR ids are fine.
  - Comments never cite size rules or line counts.
  - Tests never hardcode a port. Use `alloc_ports` and the shared harness
    (`crates/substrate/tests/common`).
  - The import clean-up pass, the deferred backlog, the living-docs close-out,
    the demo close-out, the ratchets (`check-file-lengths`,
    `check-duplication`, `check-lint-suppressions`).
- [task.md](task.md): section 2 (facts) and "A6 — Remote clients".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  problem 4, "Remote clients" in Design, OF-12 and OF-13, the task line
  "Remote access setting, …". No other change doc defines a criterion for
  A6. `task.md` names no other change doc for this slice.
- [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md):
  the gateway is a dumb proxy, the auth service owns login, the auth service
  has its own origin (`auth.<domain>`) because of keep-alive (problem P1).
  Its "out of scope" section says the browser and the gateway are on the same
  machine. A6 changes that assumption (Q8).
- The review files [slice-a2a-review.md](slice-a2a-review.md) to
  [slice-a5-review.md](slice-a5-review.md). Section 7.9 lists how A6 avoids
  each kind of finding.
- [deferred-backlog.md](../../deferred-backlog.md): §7 rows "Gateway bind
  address differs from the Roym spec" (:330) and "The `local` login method
  has no proof step" (:342); §10 row "The Hub's CSP is a static `<meta>` tag
  …" (:514).

### Terms used in this plan

- **Client gateway** (or **gateway**): the substrate's HTTP entry point
  (`crates/client_gateway`). It reads the first request's headers on a TCP
  connection, picks the target service from `Host`, and then copies the raw
  bytes of the whole connection to that service over an Iroh stream.
- **Auth service:** the node's login service (`crates/auth`). It answers
  `/_syneroym/session/*` and creates session tokens.
- **`local` login:** a login method that names a key file in
  `person_identities_dir` on the node. It has no proof step. The node reads
  the key itself.
- **`delegated-key` login:** a login method with a challenge and a signature.
  The client proves it holds a key that the person's master key delegated to.
- **Remote access:** the new gateway setting. When it is on, the gateway
  accepts clients from other machines.
- **Loopback address:** `127.0.0.0/8` or `::1`. Only programs on the same
  machine can connect from it.
- **Peer address:** the address at the other end of an accepted TCP
  connection (`TcpListener::accept` returns it). A client cannot fake it.
- **Own-gateway stream:** a stream into the router whose verified caller is
  the node's own key. Only this node's gateway sends such a stream for an
  HTTP client.
- **CSP:** Content Security Policy, the browser rule list in the Hub page.
  `connect-src` says which origins page code may `fetch`.
- **CORS:** the headers that let the Hub page (one origin) read answers from
  the auth service (another origin).
- **Secure context:** a page served over HTTPS, or from `localhost`. Browsers
  offer `crypto.subtle` (WebCrypto) only in a secure context.

## 1. What A6 builds, in one paragraph

The gateway binds `127.0.0.1` by default, as the Roym spec says. A new table,
`[roles.client_gateway.remote_access]`, with one field `bind_address`, turns
remote access on and names the address to bind (Q1). When remote access is
off, the gateway also refuses any TCP peer that is not a loopback address.
The auth service answers a `local` login only on a stream from the node's own
gateway, never on one from another node, and turns `local` off completely when
remote access is on (Q2). Every refusal says to use `delegated-key`. The Hub
already logs in with `delegated-key` only; it does not change its login. Its
CSP stops naming `auth.localhost`: the page now allows exactly the auth origin
that it computes from its own host (Q3). The auth service's `allowed_origins`
can name every Hub hostname of a domain with one pattern (Q4). Remote access
with `identity_mode = "fixed"` stops startup (Q5). Remote access on a
non-loopback address over plain HTTP logs a warning (Q6). The Docker deploy
gets an example config that turns remote access on inside the container (Q7).
ADR-0028 records the new rule (Q8).

## How this is shown to an end user

- Alice runs her node on a small home server. By default, only programs on
  that same server can open her Hub. A program on another computer cannot
  even connect.
- Alice wants to use the Hub from her laptop. She turns on remote access in
  the node's settings, and puts a TLS proxy (for example Caddy) in front of
  the node, so the Hub address starts with `https://`.
- On her laptop she opens the Hub address and signs in with her session key
  file, the same way she does on the server today.
- Messages, search and cards work the same as on the server.
- If a person or program on another computer tries the quick `local` sign-in,
  the node refuses it and says to use the `delegated-key` sign-in.

A developer can see the work in these ways:

- `ss -ltn | grep 7960` on a node with no `remote_access` table shows
  `127.0.0.1:7960`. With `bind_address = "0.0.0.0"` it shows `0.0.0.0:7960`.
- With remote access on: `curl -s -X POST http://127.0.0.1:7960/_syneroym/session/login
  -H 'Content-Type: application/json' -d '{"method":"local","identity":"alice"}'`
  prints a `403` body whose text names `delegated-key`, and
  `curl -s http://127.0.0.1:7960/_syneroym/session/methods` lists only
  `delegated-key`.
- `cargo nextest run -p syneroym-substrate --test gateway_remote_access_e2e`
  runs the end-to-end checks (section 7.4).

## 2. Points for the owner

Each question has options, a recommendation and the reason. Sections 4 to 10
follow the recommendation. Two choices are not questions, because the change
doc settles them:

- The check never reads `X-Forwarded-For` or any other header to allow a
  `local` login (OF-12).
- When remote access is on, `local` login is off completely (OF-12).

### Q1. What does the remote-access setting look like?

- **(a) Recommended.** An optional table, present means on:
  ```toml
  [roles.client_gateway.remote_access]
  bind_address = "0.0.0.0"
  ```
  `bind_address` is an IP address (`IpAddr`), required inside the table. The
  port stays `http_port`. A typo in the field name fails to parse, because the
  field is required. A node behind a TLS proxy on the same machine writes
  `bind_address = "127.0.0.1"`: remote access is on (so `local` is off), and
  only the proxy can connect.
- (b) Two fields: `bind_address` (default `127.0.0.1`) and
  `remote_access = false`. Allows a state with a public address and
  `remote_access = false`, which must then be refused or warned about.
- (c) One field `remote_access_bind: Option<IpAddr>`. The same as (a) with
  no room for a later field (for example a TLS setting).

Reason for (a): one setting cannot disagree with itself, the table can grow,
and the Caddy case has a clear form.

### Q2. How does `local` login know the request is from this machine?

Two facts decide this (section 3.2 and 3.3):

1. The gateway sees only the **first** request of a TCP connection. After
   that it copies raw bytes. On a keep-alive connection it never sees a
   second request, such as `POST /login` after `GET /methods`. So the gateway
   cannot refuse one request; it can only refuse a whole connection.
2. The auth service does not see the TCP socket. It gets the request from the
   router, and the router gets it over Iroh. But it does get the verified
   caller. A stream from the node's own gateway has the node's DID as caller
   ([auth.rs:113-115](../../../../crates/router/src/route_handler/http/auth.rs#L113)
   uses the same rule). Today the auth service ignores the caller, so any
   node that opens an Iroh stream to the `auth` alias can ask for a `local`
   login (finding F3).

Options:

- **(a) Recommended.** Three checks that together mean "a TCP peer with a
  loopback address on this node's own gateway":
  1. Remote access off: the gateway binds `127.0.0.1` and refuses, at accept,
     any peer whose address is not a loopback address. Remote access on:
     `local` is off.
  2. The auth service answers `local` only when the caller is the node's own
     DID (an own-gateway stream). A stream from another node is refused.
  3. The auth service refuses `local` when the request carries `Forwarded`,
     `X-Forwarded-For`, `X-Real-IP` or `Via`. This only **refuses**; a header
     never allows anything. It catches an owner who put a proxy on the same
     machine and forgot to turn remote access on: Caddy and most proxies add
     `X-Forwarded-For` by default.
- (b) Checks 1 and 2 only.
- (c) The gateway adds the peer address to the route preamble of each
  connection, and the router passes it to the auth service, trusted only from
  the node's own key. More exact, but it changes the preamble, the SDK
  passthrough, the router and `CallerContext`. Check 1 already guarantees a
  loopback peer when remote access is off, so (c) adds nothing that (a) does
  not give.

Reason for (a): it closes the Iroh gap with an existing fact (the verified
caller), keeps the gateway dumb, and adds a cheap net for the most likely
mistake. The accept check in 1 never fires in a normal setup, because a
socket bound to `127.0.0.1` accepts only loopback peers. It stays as the
place that states OF-12's rule in code, and it keeps holding if a later change
adds a second listener.

### Q3. How does the Hub CSP allow the right auth origin?

Today [index.html:14](../../../../crates/roym_web/ui/index.html#L14) has
`connect-src 'self' http://auth.localhost:* https://auth.localhost:*`. A Hub
on `roym-….example.com` calls `auth.example.com`
([login.ts:30-35](../../../../crates/roym_web/ui/src/session/login.ts#L30)),
which this blocks. The asset is one static file for every node, and the router
serves it generically ([blobs.rs:19](../../../../crates/router/src/route_handler/http/blobs.rs#L19)).

- **(a) Recommended.** Two CSP `<meta>` tags; a browser enforces every policy
  it gets, so the result is the narrower one. The static tag keeps every rule
  and sets `connect-src 'self' http: https:`. A small module that the Hub runs
  first adds a second tag, `connect-src 'self' <authOrigin()>`, built from the
  same `authOrigin()` the login uses. The effective rule is
  `connect-src 'self' <the one auth origin>`, exact to the port. No Rust
  change, no config. Risk: the second tag works only if the browser applies a
  `<meta>` CSP added by script; the spec says a policy applies from the moment
  the element is inserted into `<head>`. `unverified` until the Playwright
  case in 7.6 runs; that case fails if the narrowing does not happen.
- (b) The router sends a `Content-Security-Policy` header on HTML assets,
  from a policy the app manifest declares, with a placeholder for the auth
  origin. It would also allow `frame-ancestors` (backlog §10). New manifest
  field, router code and deploy validation.
- (c) Widen the static tag to `connect-src 'self' http: https:` and stop.
  Simplest, and page code may then `fetch` any origin.

Reason for (a): exact policy, small change, uses the function that already
decides where the Hub logs in. `frame-ancestors` stays in the backlog (section
8).

### Q4. How does the auth service allow a remote Hub's origin (CORS)?

The auth service always allows `localhost`, `127.0.0.1` and every
`*.localhost` origin, and also each exact string in `allowed_origins`
([service.rs:516-530](../../../../crates/auth/src/service.rs#L516)). A Hub
hostname has hashes in it (`roym-a<hash>-s<hash>.example.com`), and every app
and service has its own.

- **(a) Recommended.** An `allowed_origins` entry may have the form
  `<scheme>://*.<domain>[:<port>]`. It matches an origin with the same scheme
  and port whose host ends with `.<domain>` and has at least one more label.
  Exact entries work as today.
- (b) Exact entries only. The developer guide says how to find the Hub's
  origin.

Reason for (a): the same rule the auth service already applies to
`*.localhost`, so a remote node behaves like a local one. The cost is the same
as today's local rule: every app page on that domain may make credentialed
calls to the auth service. Section 8 adds a backlog row for that (it is true
for `*.localhost` today).

### Q5. Remote access together with `identity_mode = "fixed"`?

In `fixed` mode the gateway attaches one person's delegation to every stream
([gateway.rs:535-538](../../../../crates/client_gateway/src/gateway.rs#L535)).
With remote access, every client on the network would act as that person.

- **(a) Recommended.** Startup fails with an error that names both settings.
- (b) A warning at startup.
- (c) Allowed with no message.

### Q6. Remote access on a public address over plain HTTP?

The gateway has no TLS. A session token then crosses the network in clear
text. Also, a browser offers `crypto.subtle` only in a secure context, so the
Hub's `delegated-key` login cannot sign on a plain-HTTP page that is not
`localhost` (finding F7). So a remote Hub needs HTTPS.

- **(a) Recommended.** When `bind_address` is not a loopback address, the
  gateway logs one `warn!` at startup: the address, "plain HTTP", and "put a
  TLS proxy in front, or keep this address on a trusted network". The
  developer guide says the same, and says a remote Hub needs HTTPS.
- (b) Refuse a non-loopback `bind_address` unless an extra setting accepts
  plain HTTP.
- (c) TLS in the gateway. Not in A6; a backlog row (section 8).

Reason for (a): the Docker community node needs `0.0.0.0` inside the
container (Q7), and its TLS may sit outside the container.

### Q7. The Docker deploy: is the community node's gateway public?

`task.md` asks to confirm this before the compose file changes. Facts
(section 3.6): the compose file publishes `7960:7960`, mounts `./config` and
runs `--config /etc/syneroym/config.toml`. The repo has no such config file.
Inside a container, a gateway bound to `127.0.0.1` cannot be reached through
any port mapping.

- **(a) Recommended.** Public. Add `deploy/config.community.example.toml`
  with `[roles.client_gateway.remote_access] bind_address = "0.0.0.0"`
  (and the registry and Iroh roles the compose file already publishes). The
  compose file keeps `7960:7960`, says to copy the example to
  `./config/config.toml`, and has a comment: change it to
  `127.0.0.1:7960:7960` for a private node. The same edit fixes `7965:7965`
  to `7965:7965/udp`, because Iroh's QUIC port is UDP and Docker maps TCP
  unless told (finding F6).
- (b) Private by default: the same example config (still `0.0.0.0` inside),
  and the mapping `127.0.0.1:7960:7960`.
- (c) No example file; only comments.

Note for any option: a node that runs today with its own config and no
`remote_access` table becomes local-only after the upgrade. Its gateway stops
answering other machines. The startup log line (D-A6-3) says so, and the PR
description and the developer guide's Docker section name it as an upgrade
step.

### Q8. Does the rule get an ADR?

ADR-0024 says the browser and the gateway are on the same machine, and that
untrusted-network browser access is out of scope. A6 makes remote clients a
supported setup.

- **(a) Recommended.** A short new ADR-0028, "Remote client access": the
  setting, the three `local` checks of Q2, `delegated-key` for remote
  clients, HTTPS for a remote Hub. It does not edit ADR-0024; it names it.
- (b) The change doc's decisions only.

Reason for (a): it is a security boundary, and ADR-0024 would otherwise be
the only permanent record and say the opposite.

### Q9. The demo close-out

A remote Hub needs a second machine and TLS. A demo story runs on one
machine.

- **(a) Recommended.** `demo:` gets `- none: remote access needs a second
  machine and TLS; the Hub on another domain is covered by Hub case 42`.
- (b) A story that opens the Hub on a non-`localhost` domain mapped to
  `127.0.0.1` (the trick of case 42). It would show the same screens as the
  chat story.

## 3. Facts checked in the code

### 3.1 The gateway

- It binds `0.0.0.0:<http_port>`
  ([gateway.rs:184-189](../../../../crates/client_gateway/src/gateway.rs#L184)).
  `0.0.0.0` is IPv4 only, so `::1` was never served. A client that tries
  `::1` first for `localhost` falls back to `127.0.0.1` today and after A6
  alike.
- `accept` gives the peer address, which is only logged
  ([gateway.rs:203-205](../../../../crates/client_gateway/src/gateway.rs#L203)).
- `init` reads the role, with port 7000 when the role is absent
  ([gateway.rs:97-100](../../../../crates/client_gateway/src/gateway.rs#L97)).
  `ClientGateway` keeps `port`, `state` and `shutdown_tx`
  ([gateway.rs:74-78](../../../../crates/client_gateway/src/gateway.rs#L74)).
- `handle_connection` reads the first request's headers, picks the target
  from `Host` or the `/_syneroym/session` path prefix, and calls
  `forward_to_service` once
  ([gateway.rs:408-487](../../../../crates/client_gateway/src/gateway.rs#L408),
  [gateway.rs:327-339](../../../../crates/client_gateway/src/gateway.rs#L327)).
  `forward_to_service` signs one preamble with the node identity
  ([gateway.rs:540-551](../../../../crates/client_gateway/src/gateway.rs#L540))
  and `passthrough_with_conn` copies the rest of the connection
  ([passthrough.rs:126-186](../../../../crates/sdk/src/client/passthrough.rs#L126)).
- `fixed` mode attaches the fixed delegation to every stream
  ([gateway.rs:535-538](../../../../crates/client_gateway/src/gateway.rs#L535)).
- No TLS code in the crate.
- The unit tests build a `GatewayState` by hand in `test_state`
  ([gateway/tests.rs:52](../../../../crates/client_gateway/src/gateway/tests.rs#L52)).
  `gateway.rs` is 675 lines; `gateway/tests.rs` is 411.

### 3.2 The auth service and every `local` caller

- `login_local` reads `<name>.key` (or `<name>`) from `person_identities_dir`
  and mints a token. With no dir it answers `400 "local login method is not
  enabled"` ([service.rs:343-416](../../../../crates/auth/src/service.rs#L343)).
- `handle_login` refuses `local` with `403` when an `Origin` header is
  present ([service.rs:622-638](../../../../crates/auth/src/service.rs#L622)).
  So no browser page can use `local`.
- `methods()` lists `local` when the dir is set
  ([service.rs:418-424](../../../../crates/auth/src/service.rs#L418)).
- `handle_request` gets `caller: Option<CallerContext>` and ignores it
  ([service.rs:714-719](../../../../crates/auth/src/service.rs#L714)).
  `AuthService` keeps `node_did`
  ([service.rs:113](../../../../crates/auth/src/service.rs#L113)); the runtime
  passes the router's node DID
  ([runtime/router.rs:495](../../../../crates/substrate/src/runtime/router.rs#L495),
  [route_handler.rs:457](../../../../crates/router/src/route_handler.rs#L457)).
- `is_allowed_origin` and `cors_headers`
  ([service.rs:516-549](../../../../crates/auth/src/service.rs#L516)).
- The runtime builds the service with `with_allowed_origins` and
  `with_secure_cookies`
  ([runtime/auth.rs:46-57](../../../../crates/substrate/src/runtime/auth.rs#L46)).
- No test asserts the text of a `local` refusal (grep of
  "local login method is not" and "browser origin").
- Callers of `local` login:
  - `roymctl session login` with `--identity` or `--as` and no key file
    ([session.rs:262-300](../../../../apps/roymctl/src/commands/session.rs#L262)),
    through the gateway URL and the `/_syneroym/session/login` path.
  - Substrate tests through the gateway: the Roym harness `RoymNode::login`
    ([common/roym.rs:452-463](../../../../crates/substrate/tests/common/roym.rs#L452)),
    [roym_app_e2e.rs:345](../../../../crates/substrate/tests/roym_app_e2e.rs#L345),
    [roym_identity_e2e.rs:261](../../../../crates/substrate/tests/roym_identity_e2e.rs#L261).
    All go through the gateway with no remote access, so they keep working.
  - Auth unit tests that call the service directly:
    `local_login_and_refusals`, `http_interface_endpoints_and_unknown_method`
    (posts `local` with `caller = None`,
    [auth_service_tests.rs:389-399](../../../../crates/auth/tests/auth_service_tests.rs#L389)),
    `local_login_refused_when_origin_header_is_present`. The second one
    changes (Step 3).
  - The Hub, the Playwright tests and the demo never use `local`.
- `service.rs` is 789 lines; `auth_service_tests.rs` is 628.

### 3.3 The caller the auth service sees

- The router builds the transport caller from the preamble; `caller_did` is
  the delegation's master DID when a delegation is present, else the signer's
  DID ([io.rs:242-249](../../../../crates/router/src/route_handler/io.rs#L242)).
  A gateway stream in `open` or `login` mode has no delegation, so its caller
  is the node DID.
- For every HTTP request the router first tries the session token: if the
  stream is an own-gateway stream and a valid token is present, the caller
  becomes the person
  ([auth.rs:106-138](../../../../crates/router/src/route_handler/http/auth.rs#L106),
  [http.rs:293-299](../../../../crates/router/src/route_handler/http.rs#L293)).
  That rule is "caller is the node DID and no delegation" (`is_gateway_origin`,
  [auth.rs:113-115](../../../../crates/router/src/route_handler/http/auth.rs#L113)).
- The native HTTP dispatch hands that effective caller to the service
  ([guest.rs:76-83](../../../../crates/router/src/route_handler/http/guest.rs#L76),
  [guest.rs:124-131](../../../../crates/router/src/route_handler/http/guest.rs#L124)).
- The auth routes are `public: true`
  ([runtime/auth.rs:81-127](../../../../crates/substrate/src/runtime/auth.rs#L81))
  and registered under the `auth` alias and the auth DID
  ([runtime/auth.rs:133-155](../../../../crates/substrate/src/runtime/auth.rs#L133)).
- The WebRTC coordinator also loads the node key, but only to call `resolve`;
  its blind tunnel has no identity of its own
  ([coordinator.rs:80-88](../../../../crates/coordinator_webrtc/src/coordinator.rs#L80)).
  The gateway is the only part that sends HTTP client bytes signed by the
  node key (grep of `passthrough_with_conn`).
- `CallerContext::verified(did, auth, caps)` builds a caller for tests
  ([native.rs:244](../../../../crates/rpc/src/native.rs#L244)).

### 3.4 The Hub

- Login is `delegated-key` only. The temporary key is a non-extractable
  WebCrypto key; `completeLogin` calls `crypto.subtle.sign`
  ([login.ts:224-272](../../../../crates/roym_web/ui/src/session/login.ts#L224)).
- `authOrigin()` is `<protocol>//auth.<host minus first label><:port>`
  ([login.ts:30-35](../../../../crates/roym_web/ui/src/session/login.ts#L30)).
- The CSP is one static `<meta>` tag
  ([index.html:6-14](../../../../crates/roym_web/ui/index.html#L6)).
  `roym_app_e2e.rs:376` checks the page contains `Content-Security-Policy`.
- `main.ts` starts with `import { renderCard } …`
  ([main.ts:1](../../../../crates/roym_web/ui/src/main.ts#L1)). No imported
  module calls `fetch` at its top level.
- Playwright: Hub URL `http://<web alias>:<gateway port>`
  ([global-setup.ts:188](../../../../crates/substrate/tests/e2e/global-setup.ts#L188)),
  `AUTH_ORIGIN = http://auth.localhost:<port>`
  ([roym-hub.spec.ts:44](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L44)).
  The node config is written by `node-boot.ts`; `login` mode writes an empty
  `[roles.auth]`
  ([node-boot.ts:289-293](../../../../crates/substrate/tests/e2e/node-boot.ts#L289)).
  The Chromium project sets two launch flags
  ([playwright.config.ts:32-36](../../../../crates/substrate/tests/e2e/playwright.config.ts#L32)).
  The last Hub case is 41
  ([roym-hub.spec.ts:1247](../../../../crates/substrate/tests/e2e/tests/roym-hub.spec.ts#L1247)).

### 3.5 Config

- `ClientGatewayRole` and its `Default`
  ([roles.rs:407-446](../../../../crates/core/src/config/roles.rs#L407));
  `AuthRole` with `allowed_origins` and its doc comment
  ([roles.rs:449-483](../../../../crates/core/src/config/roles.rs#L449)).
  Config tests are in
  [config/tests.rs](../../../../crates/core/src/config/tests.rs) (258 lines).
- Struct literals of `ClientGatewayRole` all use `..Default::default()` or
  `..gateway` (basic_lifecycle.rs:241, podman_lifecycle.rs:89,
  common/node.rs:278, common/mod.rs:302, common/roym.rs:290), so a new field
  breaks none.
- `run` with no `--config` uses `dev_mode_config()`, which enables the
  gateway role with defaults
  ([main.rs:89-96](../../../../crates/substrate/src/main.rs#L89),
  [main.rs:211](../../../../crates/substrate/src/main.rs#L211)).
- The sample config documents the gateway and auth roles
  ([config.sample.toml:236-268](../../../../crates/substrate/config.sample.toml#L236)).
- No test or tool reaches the gateway from a non-loopback address (grep of
  `host.containers.internal`, `host.docker.internal`, `10.0.2.2`; the smoke
  tests do not use the gateway).

### 3.6 Docker

- [docker-compose.community.yml](../../../../deploy/docker-compose.community.yml)
  publishes `7964`, `7965`, `7961` and `7960`, all as TCP, mounts `./config`
  and `/etc/letsencrypt`, and runs `run --config /etc/syneroym/config.toml`.
- The [Dockerfile](../../../../Dockerfile) comments name the four ports
  (lines 31-36) and the default command is `run` (dev mode, line 39).
- The repo has no community config file. The developer guide has no Docker
  section (grep of `Docker`, `compose`); the architecture doc describes the
  image at
  [system-architecture.md:764](../../../../docs/system-architecture.md#L764).
- The architecture config table says the Iroh `quic_bind_address`
  (`0.0.0.0:7965`) is QUIC address discovery, used when relay TLS is set
  ([system-architecture.md:375](../../../../docs/system-architecture.md#L375)).
  QUIC is UDP.

### 3.7 Living-doc statements that become false

- [roym-integrated-experience-spec.md:77](../../../../docs/roym-integrated-experience-spec.md#L77)
  (D8), `:174` and `:849` (G3): "the gateway binds `127.0.0.1`" — true after
  A6 as a default, but D8's "authenticates no client" and "untrusted-network
  browser access stays out of scope" need the new setting.
- [system-architecture.md:694](../../../../docs/system-architecture.md#L694):
  "The gateway sets no rule about which machine may connect to it."
- [system-architecture.md:711](../../../../docs/system-architecture.md#L711):
  `local` "refuses only a request that carries an `Origin` header".
- [developer-guide.md:1840-1880](../../../../docs/developer-guide.md#L1840):
  the auth endpoints and the configuration block; the port list at `:236`.
- [system-requirements-spec.md:623](../../../../docs/system-requirements-spec.md#L623):
  "The gateway listens on port 7960".
- [traceability-matrix.md](../../traceability-matrix.md): no row covers
  remote clients yet (the last Roym row is `[ROY-EVT]`, `:96`).

### 3.8 Findings that change the task text

- **F1. The Hub already uses only `delegated-key`.** `task.md` says "The Hub
  logs in with `delegated-key` when it is not on the node's machine." It
  always does, on every machine, and `local` refuses any request with an
  `Origin` (3.2, 3.4). A6 changes no Hub login code. Deviation.
- **F2. The gateway cannot check one request.** It sees only the first
  request of a connection (3.1). Q2.
- **F3. Another node can ask for a `local` login over Iroh.** The auth routes
  are public, registered under the well-known `auth` alias, and the service
  ignores the caller (3.2, 3.3). This is wider than the bind address.
  `unverified` by a run; Step 4's test `local_login_over_iroh_from_another_identity_is_refused`
  must fail on `main` before Step 3's change, and pass after.
- **F4. The developer guide has no Docker section** to update. A6 adds a
  short one. Deviation.
- **F5. The repo has no config for the compose file.** Q7.
- **F6. The compose file maps the QUIC port as TCP.** Q7.
- **F7. A remote Hub needs HTTPS.** `crypto.subtle` exists only in a secure
  context, so `delegated-key` cannot sign on `http://<host>` that is not
  `localhost`. Q6.
- **F8. CORS matches exact strings only**, and Hub hostnames carry hashes.
  Q4.
- **F9. Remote access with `fixed` mode** makes every client that person.
  Q5.
- **F10. `docker run` with no config** runs dev mode, so after A6 its gateway
  binds `127.0.0.1` inside the container and cannot be reached. The
  `Dockerfile` comment says to mount a config.
- **F11. Upgrade of an existing public node.** Q7's note.

## 4. Behaviour-change audit

Every change below is listed. Everything not listed stays the same.

### Wire changes

| # | Change | Who sees it |
| --- | --- | --- |
| W1 | The gateway binds `127.0.0.1:<http_port>` by default. Before: `0.0.0.0:<http_port>`. A client on another machine cannot connect unless remote access is on. | Every client on another machine; Docker deploys (F10, F11). |
| W2 | New optional config table `[roles.client_gateway.remote_access]` with the required field `bind_address` (an IP address). Present means remote access is on, and the gateway binds `<bind_address>:<http_port>`. | Node owners. An older binary ignores the table and binds `0.0.0.0`. |
| W3 | `POST …/login` with `method: "local"` gets `403` when the request did not come on an own-gateway stream (for example over Iroh from another node). Before: a token. | Other nodes (F3). |
| W4 | When remote access is on: `local` login gets `403`, and `GET …/methods` lists only `delegated-key`, even when `person_identities_dir` is set. | Clients of a node with remote access; `roymctl session login --as` against it. |
| W5 | `local` login gets `403` when the request carries `Forwarded`, `X-Forwarded-For`, `X-Real-IP` or `Via`. | A same-machine proxy without remote access. |
| W6 | The four `local` refusal texts name `delegated-key` (D-A6-7). The `400` for a node with no `person_identities_dir` keeps its status; only its text grows. | Anyone who reads the error, including `roymctl`, which prints it. |
| W7 | An `allowed_origins` entry `<scheme>://*.<domain>[:<port>]` matches subdomain origins (D-A6-9). | Node owners; remote Hubs. |
| W8 | The Hub page's effective `connect-src` is `'self' <its auth origin>`, exact to the scheme and port. Before: `'self' http://auth.localhost:* https://auth.localhost:*`. The static tag alone now says `'self' http: https:` (D-A6-10). | The Hub, in every browser. |
| W9 | Startup fails when `remote_access` is set and `identity_mode = "fixed"` (D-A6-4). | Node owners. |

No JSON-RPC method, input, output or error changes. No WIT changes. The
method tables and shape files stay as they are.

### Behaviour changes that are not wire changes

| # | Change |
| --- | --- |
| B1 | With remote access off, the gateway drops a connection whose peer address is not a loopback address, with one `warn!`. In practice this never fires (Q2). |
| B2 | Startup logs where the gateway listens and whether remote access is on (D-A6-3). A non-loopback `bind_address` adds one `warn!` about plain HTTP (D-A6-5). |
| B3 | When remote access is on and `person_identities_dir` is set, startup logs one `warn!` that `local` login is off (D-A6-6). |
| B4 | `local` login on an own-gateway stream that also carries a valid session token is refused: the router makes the person the caller (3.3), so the caller is not the node DID. `roymctl` sends no token on login. |

### Stays the same

`delegated-key` login, the challenge, session tokens, cookies, `refresh`,
`logout` and `whoami`. The router, the preamble, the SDK passthrough and the
event feed. The identity modes and `connection_auth_gate`. The bind addresses
of the registry, the coordinators, health and metrics. `roymctl` commands and
flags. The Hub's login flow and its `sessionStorage` and feed cookie. The
`Origin` refusal of `local`.

## 5. Decisions this plan makes

Decisions are numbered D-A6-n. They go into the change doc at close-out
(section 9). Q1 to Q9 are the owner's. The others are made here.

**D-A6-1. The config type.** In `crates/core/src/config/roles.rs`:

```rust
/// Lets clients on other machines reach the client gateway. Absent means
/// the gateway binds `127.0.0.1` and refuses non-loopback peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAccess {
    /// The address the gateway binds, with `http_port`. `0.0.0.0` accepts
    /// every interface. `127.0.0.1` is for a TLS proxy on the same machine:
    /// remote access is still on, so `local` login is off.
    pub bind_address: IpAddr,
}

pub struct ClientGatewayRole {
    // existing fields …
    /// Remote access. Absent by default.
    #[serde(default)]
    pub remote_access: Option<RemoteAccess>,
}
```

`Default` sets `remote_access: None`. Add
`impl ClientGatewayRole { pub fn bind_ip(&self) -> IpAddr }` returning
`remote_access.bind_address` or `Ipv4Addr::LOCALHOST`, so the gateway and the
docs use one rule.

**D-A6-2. The gateway listener.** `ClientGateway` gets
`bind: SocketAddr` and `remote_access: bool`, set in `init` from the role
(`SocketAddr::new(role.bind_ip(), port)`; with no role, `127.0.0.1:7000`
keeps today's port fallback). `run` binds `self.bind`. Add
`pub fn bind_addr(&self) -> SocketAddr` for the unit test. `Debug` prints
`bind` instead of `port`.

In the accept arm:

```rust
Ok((stream, peer_addr)) => {
    if !admit_peer(self.remote_access, peer_addr) {
        warn!(%peer_addr, "client gateway refused a non-loopback peer: remote access is off");
        continue;          // drop `stream`; no response
    }
    // unchanged spawn …
}

/// With remote access off only a program on this machine may connect.
fn admit_peer(remote_access: bool, peer: SocketAddr) -> bool {
    remote_access || peer.ip().to_canonical().is_loopback()
}
```

`to_canonical` maps `::ffff:127.0.0.1` to `127.0.0.1`. Keep the accept arm
under 10 lines; if it grows, move the body into `fn accept_one(&self, …)`.

**D-A6-3. The startup log.** Replace the two `info!` lines in `run` with one:
`info!(bind = %self.bind, remote_access = self.remote_access, "client gateway
listening")`. When remote access is off, the message adds "other machines
cannot connect; see [roles.client_gateway.remote_access]".

**D-A6-4. Fixed mode with remote access.** In `ClientGateway::init`, before
reading the fixed delegation:
`anyhow::bail!("[roles.client_gateway] remote_access cannot be used with
identity_mode = \"fixed\": every client on the network would act as the fixed
person")`. `init` already returns `Result`, and the runtime stops on its
error.

**D-A6-5. Plain-HTTP warning.** In `init`, when `remote_access` is set and
`!bind_address.is_loopback()`: one `warn!` naming the address, plain HTTP, and
"put a TLS proxy in front; a remote Hub needs HTTPS to sign in".

**D-A6-6. The auth service's `local` state.** In `crates/auth`:

```rust
enum LocalLogin {
    NotConfigured,
    OffForRemoteAccess,
    On(PathBuf),
}
```

`AuthService::new` keeps its signature and sets `On(dir)` or
`NotConfigured`. New builder:

```rust
#[must_use]
pub fn with_remote_access(mut self, on: bool) -> Self  // On(_) -> OffForRemoteAccess
```

`methods()` lists `local` only for `On`. `runtime/auth.rs` calls
`.with_remote_access(config.roles.client_gateway.as_ref().is_some_and(|g|
g.remote_access.is_some()))` and logs B3's `warn!` when it turns `On` off.

**D-A6-7. The `local` gate.** One function, called by `handle_login` in the
`local` arm before it reads `identity`:

```rust
/// Decides whether this request may use `local` login. The node reads the
/// key itself, so the request must come from a program on this machine:
/// through this node's own gateway, which binds loopback when remote access
/// is off, and with no sign of a proxy in between.
fn local_login_gate(
    &self,
    request: &HttpRequest,
    caller: Option<&CallerContext>,
) -> Result<&Path, (u16, &'static str)>
```

Checks, in this order, each with its exact text:

1. `NotConfigured` → `400`, `"local login method is not enabled; use
   delegated-key"`.
2. `OffForRemoteAccess` → `403`, `"local login is off because remote access
   is on; use delegated-key"`.
3. `Origin` present → `403`, `"local login method is not allowed from a
   browser origin; use delegated-key"`.
4. A proxy header (`forwarded`, `x-forwarded-for`, `x-real-ip`, `via`, any
   case) → `403`, `"local login is not allowed through a proxy; use
   delegated-key"`.
5. Caller is not `Some(c)` with `c.caller_did == self.node_did` → `403`,
   `"local login is only for a client on this node's own machine; use
   delegated-key"`.

Then `login_local` runs as today, with the dir from step 1. `login_local`
itself keeps its signature and its own `400` checks, so direct unit-test
calls still work; it reads the dir from `LocalLogin::On`. `handle_request`
passes `caller.as_ref()` to `handle_login`. The router's `is_gateway_origin`
also checks "no delegation"; the auth service does not need it, because a
stream with a delegation has the master DID as caller (3.3).

**D-A6-8. Where the auth code goes.** `service.rs` is near the file cap. New
sibling module files, entry point `service.rs` beside `service/`:
- `crates/auth/src/service/local.rs`: `LocalLogin`, `local_login_gate`,
  `login_local` (moved, unchanged apart from the dir lookup),
  `with_remote_access`.
- `crates/auth/src/service/cors.rs`: `is_allowed_origin`, `cors_headers`
  (moved) and the pattern match of D-A6-9.
Keep every why-comment in the moved code. `service.rs` gets smaller.

**D-A6-9. The origin pattern.** In `cors.rs`:

```rust
/// `https://*.example.com:8443` allows `https://<one or more labels>.example.com:8443`.
fn matches_origin_pattern(pattern: &str, origin: &str) -> bool
```

Split each at `"://"`; schemes must be equal. The pattern's rest must start
with `*.`; call what follows `suffix` (for example `example.com:8443`). The
origin's rest must end with `"." + suffix`, the part before it must be
non-empty and made only of ASCII letters, digits, `-` and `.`, and must not
start or end with `.`. So `https://evilexample.com:8443` and
`https://example.com:8443` do not match, and the port must be equal (an
absent port matches only an absent port). Exact entries are compared as
today. Update `AuthRole.allowed_origins`' doc comment and the sample config.

**D-A6-10. The Hub CSP.**
- `index.html`: the static tag becomes
  `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'
  data:; connect-src 'self' http: https:; form-action 'none'; base-uri
  'none'`. Rewrite its comment: the second tag, added by
  `src/boot-csp.ts`, narrows `connect-src` to the auth origin; browsers
  enforce both.
- `src/csp.ts` (pure, tested):
  ```ts
  /** The `connect-src` policy for a Hub whose auth service is `origin`. */
  export function connectPolicy(origin: string): string; // "connect-src 'self' <origin>"
  /** Adds that policy as a second CSP `<meta>` in `doc.head`. */
  export function installConnectPolicy(doc: Document, origin: string): void;
  ```
- `src/boot-csp.ts`: `installConnectPolicy(document, authOrigin());` and
  nothing else.
- `main.ts`: `import "./boot-csp";` as its **first** line. ES modules run
  imports in order, so this runs before any other Hub code.

**D-A6-11. The Docker files** (Q7 (a)).
- New `deploy/config.community.example.toml`: the roles the compose file
  publishes (`community_registry` on `0.0.0.0:7961`, `coordinator.iroh` on
  `0.0.0.0:7964` and `0.0.0.0:7965`, `client_gateway` on `7960` with
  `[roles.client_gateway.remote_access] bind_address = "0.0.0.0"`), and
  comments for TLS and `secure_cookies`. Nothing else: no keys, no paths
  outside the mounted volumes.
- `docker-compose.community.yml`: a comment above `volumes` "copy
  `deploy/config.community.example.toml` to `./config/config.toml`"; a
  comment on the `7960` line about `127.0.0.1:7960:7960` for a private node;
  `7965:7965/udp`.
- `Dockerfile`: the port comment says the gateway needs
  `[roles.client_gateway.remote_access]` to be reachable through a port
  mapping, and that `run` with no config is a dev setup.

**D-A6-12. The shared delegated-login test helper.** Move `login_to_gateway`
and `AUTH_HOST` from `gateway_session_e2e.rs:260-319` to a new
`crates/substrate/tests/common/session.rs` as
`pub async fn delegated_login(gateway_url, person, registry_url,
expires_hours) -> (String, String)`, unchanged in body. Both test files use
it. Lower the `gateway_session_e2e.rs` entry in
`xtask/oversized-test-files.txt` to its new length. `RoymNode` gets
`pub async fn login_delegated(&mut self)` that calls it and stores the token
the way `login` does.

**D-A6-13. The ADR** (Q8 (a)). `docs/decisions/0028-remote-client-access.md`,
reader: developers and node owners. Context (ADR-0024's same-machine
assumption), decision (Q1, Q2 (a), Q5, Q6), consequences (HTTPS for a remote
Hub, upgrade note), alternatives (Q2 (c), Q3 (b)).

## 6. Step-by-step changes

Each step builds and its tests pass. The whole branch is one PR. No step
changes a WIT file or a Roym crate, so `mise run build:roym` is needed only
because the Hub bundle changes (Step 5).

### Step 1 — Commit this plan

`docs(planning): plan M08A slice A6`. Status row A6: "Plan written".

### Step 2 — The config and the gateway bind

1. `RemoteAccess`, the field and `bind_ip` (D-A6-1). Config tests (7.1).
2. Gateway: `bind`, `remote_access`, `admit_peer`, the log line, the fixed
   check, the plain-HTTP warning, `bind_addr` (D-A6-2 to D-A6-5).
   `test_state` gets no new field (the new fields are on `ClientGateway`,
   not `GatewayState`).
3. Gateway unit tests (7.2).
4. `config.sample.toml`: the commented `remote_access` table, and "binds
   `127.0.0.1` unless remote access is set".

Check: `cargo nextest run -p syneroym-core config`, `cargo nextest run -p
syneroym-client-gateway`, and the substrate tests that boot a gateway
(`cargo nextest run -p syneroym-substrate --test gateway_session_e2e --test
basic_lifecycle`) still pass on `127.0.0.1`.

### Step 3 — The auth service gate and CORS

1. Move the code into `service/local.rs` and `service/cors.rs` with no
   behaviour change. Build and run `cargo nextest run -p syneroym-auth`
   before the next item.
2. `LocalLogin`, `with_remote_access`, `local_login_gate`, the new texts,
   `methods()` (D-A6-6, D-A6-7).
3. The origin pattern (D-A6-9).
4. `runtime/auth.rs`: `with_remote_access(..)` and B3's warning.
5. Update `http_interface_endpoints_and_unknown_method` to pass a
   node-DID caller for its `local` login. New tests (7.3).

Check: `cargo nextest run -p syneroym-auth`.

### Step 4 — End-to-end tests

1. The shared helper (D-A6-12).
2. `crates/substrate/tests/gateway_remote_access_e2e.rs` (7.4). Write
   `local_login_over_iroh_from_another_identity_is_refused` first and run it
   against the code of Step 2 with Step 3 reverted (a local stash), to see it
   fail (F3). Record the result in the PR description.

Check: `cargo nextest run -p syneroym-substrate --test
gateway_remote_access_e2e --test gateway_session_e2e`.

### Step 5 — The Hub CSP

1. `csp.ts`, `boot-csp.ts`, the `main.ts` import, `index.html` (D-A6-10).
2. `csp.test.ts` (7.5).
3. `node-boot.ts`: a `gateway.allowedOrigins?: string[]` option that writes
   `allowed_origins` into `[roles.auth]`. `global-setup.ts` sets
   `["http://*.roym.test:<gateway port>"]` for the Hub node.
4. `tests/roym-hub-remote.spec.ts` with case 42 (7.6).

Check: `cd crates/roym_web/ui && npm test`; `mise run build:roym`;
`mise run test:e2e` (all Hub cases, including the old ones that now get an
exact auth origin).

### Step 6 — Docker files

D-A6-11, and the deploy config test (7.7).

Check: `cargo nextest run -p syneroym-substrate --test
community_deploy_config`.

### Step 7 — Docs and close-out

ADR-0028 (D-A6-13), sections 8 and 9, the change doc, `task.md`,
`status.md`.

### Step 8 — Finish

The import clean-up pass over every edited Rust file; `cargo dupes report`
over the changed files; the ratchets; `mise run verify` with no `--skip`.

## 7. Tests

### 7.1 Config (`crates/core/src/config/tests.rs`)

- `a_gateway_role_with_no_remote_access_binds_loopback`: parse
  `[roles.client_gateway]` with only `http_port`; `remote_access` is `None`,
  `bind_ip()` is `127.0.0.1`.
- `remote_access_names_the_bind_address`: `bind_address = "0.0.0.0"` and
  `"::"` parse; `bind_ip()` returns them.
- `remote_access_without_a_bind_address_fails_to_parse`: an empty
  `[roles.client_gateway.remote_access]` table is an error that names
  `bind_address`.

### 7.2 Gateway unit tests (`crates/client_gateway/src/gateway/tests.rs`)

- `a_peer_is_admitted_only_from_loopback_when_remote_access_is_off`: table of
  `127.0.0.1`, `127.0.0.2`, `::1`, `::ffff:127.0.0.1` (admitted) and
  `192.168.1.5`, `10.0.0.1`, `::ffff:192.168.1.5`, `2001:db8::1` (refused);
  with remote access on, all admitted.
- `init_binds_loopback_by_default_and_the_remote_access_address_when_set`:
  `ClientGateway::init` with a test config; `bind_addr()` is
  `127.0.0.1:<port>`, then `0.0.0.0:<port>`. `init` does not bind, so the
  test writes no port literal: it reads the port from
  `ClientGatewayRole::default().http_port`. `init` loads or writes the node
  key, so the config points `app_data_dir` at a `tempfile::tempdir()`.
- `init_refuses_remote_access_in_fixed_mode`: the error text names
  `remote_access` and `fixed`.

### 7.3 Auth service (`crates/auth/tests/local_login_gate.rs`, new)

Each test builds `AuthService` with a temp identities dir holding
`alice.key`, and calls `handle_request` with a `local` login request.
`node_caller()` is `CallerContext::verified(node_did, AuthLevel::…, vec![])`.

- `local_login_on_an_own_gateway_stream_gets_a_token`.
- `local_login_with_no_caller_is_refused_and_names_delegated_key` (`403`).
- `local_login_from_another_nodes_key_is_refused` (`403`).
- `local_login_through_a_proxy_is_refused`: one case per header name, mixed
  case (`X-Forwarded-For`, `forwarded`, `X-Real-IP`, `Via`).
- `remote_access_turns_local_login_off_and_hides_it_from_methods`: `403`
  with the remote-access text, and `methods()` is `["delegated-key"]`.
- `a_node_with_no_identities_dir_still_answers_400`: text names
  `delegated-key`.
- `the_origin_refusal_comes_before_the_caller_check`: `Origin` plus no
  caller gives the browser-origin text.
- `a_subdomain_pattern_allows_every_label_of_its_domain_and_nothing_else`:
  `http://*.roym.test:7960` allows `http://a.roym.test:7960` and
  `http://a-b.c.roym.test:7960`; refuses `http://roym.test:7960`,
  `http://evilroym.test:7960`, `https://a.roym.test:7960`,
  `http://a.roym.test:7961`, `http://a.roym.test`, `http://.roym.test:7960`.
  Checked through `cors_headers` (the reflected
  `access-control-allow-origin`).
- `exact_allowed_origins_still_match_exactly` (the existing
  `configured_allowed_origins_are_reflected_in_cors` keeps passing).

### 7.4 Substrate e2e (`crates/substrate/tests/gateway_remote_access_e2e.rs`, new)

Nodes come from `SubstrateNode::builder()` with `configure(..)`; ports from
the harness.

- `local_login_through_the_gateway_still_works_with_remote_access_off`: a node
  in `login` mode with an identities dir; `POST /_syneroym/session/login`
  `local` → `200` and a token. (Guards W1 against breaking the old path.)
- `local_login_over_iroh_from_another_identity_is_refused`: the same node;
  `node.client_as(Identity::generate())`, connect, and `passthrough` to the
  `auth` alias over a local TCP pair, sending a raw `POST
  /_syneroym/session/login` with `{"method":"local","identity":"alice"}`;
  the HTTP answer is `403` and the body names `delegated-key`.
- `remote_access_turns_local_login_off_and_delegated_key_still_logs_in`:
  `remote_access.bind_address = 127.0.0.1` (the proxy shape; keeps the test on
  loopback); `local` → `403` with the remote-access text; `/methods` lists
  only `delegated-key`; `common::session::delegated_login` → a token;
  `whoami` with it → the person DID.
- `a_roym_method_works_with_a_delegated_session_on_a_remote_access_node`:
  `RoymNode` built with remote access, `login_delegated()`, then one
  local-only method over `/rpc` (`profile.get`) answers as the owner. Returns
  early when `roym_artifacts_present()` is false, like the other Roym tests;
  check the log shows it ran.
- `remote_access_in_fixed_mode_stops_startup`: `NodeBuilder::init_error()`;
  the error names `remote_access` and `fixed`.

### 7.5 Hub unit tests (`crates/roym_web/ui/src/csp.test.ts`, new)

- `connectPolicy` gives `connect-src 'self' http://auth.localhost:7960` and
  `connect-src 'self' https://auth.example.com`.
- `installConnectPolicy` appends one `meta[http-equiv=Content-Security-Policy]`
  to `document.head` with that content.

### 7.6 Playwright (`crates/substrate/tests/e2e/tests/roym-hub-remote.spec.ts`, new)

Case **42. a Hub opened on another domain reaches its own auth origin and
nothing else.** The test launches its own Chromium
(`playwright.chromium.launch`) with
`--host-resolver-rules=MAP *.roym.test 127.0.0.1` and
`--unsafely-treat-insecure-origin-as-secure=<hub origin>,<auth origin>`
(both read from `readE2EPorts()` and `ROYM_WEB_ALIAS` in the test, never at
module load). Hub origin: `http://<first label of the web alias>.roym.test:<port>`;
auth origin: `http://auth.roym.test:<port>`.

1. Open the Hub. `fetch(<auth origin>/_syneroym/session/methods)` in the page
   answers `200` (CSP and CORS allow it).
2. `fetch('http://other.roym.test:<port>/')` in the page rejects, and a
   `securitypolicyviolation` event with `effectiveDirective === 'connect-src'`
   fires. This fails if the second tag is not applied (Q3's risk).
3. Log in with `loginWithDelegatedKey` (hub-helpers) and see the session bar
   with the person DID. Needs the secure-context flag (F7).

Hub cases 1 to 41 must still pass with the exact auth origin (case 4 and the
other external-request checks already allow `AUTH_ORIGIN`).

If step 3 cannot pass because Chromium ignores the secure-context flag for a
`.test` host, keep steps 1 and 2, drop step 3, and record a deviation; the
delegated login itself is covered by 7.4.

### 7.7 Deploy config (`crates/substrate/tests/community_deploy_config.rs`, new)

- `the_community_example_config_parses_and_opens_the_gateway`: read
  `deploy/config.community.example.toml` (path from `CARGO_MANIFEST_DIR`),
  parse it as `SubstrateConfig`; `remote_access.bind_address` is `0.0.0.0`,
  `http_port` is 7960, `identity_mode` is not `fixed`.
- `the_compose_file_maps_the_quic_port_as_udp`: the compose file text has
  `7965:7965/udp`. A text check, no YAML dependency.

### 7.8 Not tested

- A client on a real second machine. The non-loopback peer refusal is tested
  with addresses in 7.2, not with a socket.
- Building or running the Docker image.
- A real TLS proxy (Caddy) in front of the gateway.
- Browsers other than Chromium for Q3's second tag.

### 7.9 How A6 avoids the kinds of findings earlier slices had

- **Wrong facts or counts** (A2b R2, A3 R3, A4 R8, A5 R5): every fact in
  section 3 has a link checked on `7afb186`; the close-out names only tests
  that exist and counts them from the nextest output.
- **Evidence that names tests that do not exist** (A3 R2, A5 R5): the change
  doc's Test column for OF-12 and OF-13 is filled from section 7 after the
  run, with real test names.
- **Change-doc decisions that do not match the plan** (A3 R9, A4 R7, A5 R4):
  D-A6-1 to D-A6-13 are copied, not rewritten, at close-out.
- **Lost why-comments in moved code** (A4 R3, A5 R8): Step 3 item 1 is a move
  with no other change; diff it before item 2.
- **Living-doc statements left false** (A4 P22, A5 R9): section 3.7 lists
  them by line; section 9 says what each must say.
- **Tests that cannot fail** (A5 R6): F3's test is run against the old code
  first (Step 4); case 42 step 2 fails without the second tag.
- **Security of tokens and cookies** (A5 R1, R11): A6 adds no new place that
  holds a token; it only refuses more.
- **Import-rule breaks** (A4 R9, A5 R10): Step 8.

### 7.10 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `crates/auth/src/service.rs` | 789 | Must not grow; D-A6-8 moves code out. |
| `crates/client_gateway/src/gateway.rs` | 675 | About +40. |
| `crates/client_gateway/src/gateway/tests.rs` | 411 | About +60. |
| `crates/core/src/config/roles.rs` | 573 | About +25. |
| `crates/core/src/config/tests.rs` | 258 | About +40. |
| `crates/substrate/tests/gateway_session_e2e.rs` | 1 477 (listed) | Goes down by about 60; lower its entry. |
| `crates/substrate/tests/common/roym.rs` | 649 | About +12. |
| `crates/auth/tests/auth_service_tests.rs` | 628 | Small edit only; new tests in a new file. |

## 8. Deferred backlog

Close:
- §7 "Gateway bind address differs from the Roym spec" (:330) → Recently
  resolved.
- §7 "The `local` login method has no proof step" (:342) → Recently
  resolved. It still has no proof step; it is now loopback-only through the
  node's own gateway and off with remote access. Say so in the resolved row.
- §10 "The Hub's CSP is a static `<meta>` tag …" (:514): split. The
  `auth.localhost` part → Recently resolved. Keep a row for `frame-ancestors`
  and `X-Frame-Options`, which still need a response header, target `TBD`.

Add:
- §3 "The auth service answers credentialed CORS for every allowed origin":
  an app page on an allowed domain (every `*.localhost` today, and a
  `*.<domain>` pattern) can call `refresh` with the auth cookie and read a new
  token. Target `TBD`. Link `crates/auth/src/service/cors.rs`.
- §7 "The client gateway has no TLS": remote access over plain HTTP sends
  tokens in clear text, and a remote Hub needs a TLS proxy. Target `TBD`.
  Link `crates/client_gateway/src/gateway.rs`.

No new `TODO` or `FIXME` marker.

## 9. Living docs to update at close-out

- **Change doc** (`roym-open-foundation/change.md`): OF-12 and OF-13 Test
  column with the tests of section 7; the task line ticked; D-A6-1 to D-A6-13
  under Decisions; Deviations: F1 (no Hub login change), F4 (new Docker
  section), the new example config file, the UDP fix, the proxy-header deny
  (not in the design text), the Iroh gap F3 and its fix; the `demo:` line of
  Q9; `living-docs-touched` keeps its list. `status` stays `draft` (A7 sets
  `done`).
- **`docs/roym-integrated-experience-spec.md`**: D8 (`:77`) and G3 (`:849`):
  the gateway binds `127.0.0.1` by default; a node owner can turn on remote
  access; remote clients log in with `delegated-key`; a remote Hub needs
  HTTPS. `:174` the same in one sentence. Link ADR-0028.
- **`docs/system-architecture.md`**: `:694` replace "sets no rule about which
  machine may connect" with the bind rule, the peer refusal and the remote
  access table; `:711` the `local` rules of D-A6-7; `:764` the image needs a
  config with remote access, and `deploy/config.community.example.toml`.
- **`docs/developer-guide.md`**: the port list (`:236`) says 7960 binds
  `127.0.0.1` by default; the auth endpoints (`:1840`) say when `local`
  answers; the Configuration block (`:1870`) shows `remote_access` and the
  `allowed_origins` pattern; a new subsection "Run a node in Docker" under
  section 5 (copy the example config, the port mappings, the private
  mapping, TLS, the upgrade note of Q7).
- **`docs/system-requirements-spec.md:623`**: "The gateway listens on port
  7960 on `127.0.0.1`, or on the address that remote access names."
- **`docs/TERMINOLOGY.md`**: add "remote access (client gateway)".
- **`docs/planning/traceability-matrix.md`**: new row `[ROY-RMT]` "Remote
  clients", M8, with the code and test links of sections 6 and 7.
- **`docs/decisions/0028-remote-client-access.md`** (Q8).
- **`crates/substrate/config.sample.toml`** (Step 2) — not a living doc, but
  OF-13 says the default config says so.
- **`task.md`** A6 result line; **`status.md`** A6 row.

## 10. Exit criteria

A6 is done when all of these are true:

1. The owner has answered Q1 to Q9, and sections 4 to 9 match the answers.
2. With no `remote_access`, the gateway binds `127.0.0.1:<http_port>`, and
   `admit_peer` refuses non-loopback peers (7.1, 7.2) (OF-13).
3. With `remote_access`, the gateway binds `bind_address`, `local` login is
   off, `/methods` lists only `delegated-key`, a delegated session can use a
   Roym method (7.4) (OF-12, OF-13).
4. A `local` login that does not come on an own-gateway stream, or that
   carries a proxy header, gets `403`, and every refusal names
   `delegated-key` (7.3, 7.4) (OF-12). F3's test failed on the old code.
5. No `X-Forwarded-For` or other header ever allows a `local` login (code
   review of `local_login_gate`).
6. The Hub's effective `connect-src` is its own auth origin, and case 42
   passes on a non-`localhost` domain (OF-13).
7. Remote access with `fixed` mode stops startup.
8. The Docker example config parses and opens the gateway; the compose file
   maps QUIC as UDP (7.7).
9. The three backlog rows are closed or split, and the two new rows exist.
10. The change doc, the living docs, the traceability matrix, ADR-0028,
    `task.md` and `status.md` are updated as in sections 8 and 9.
11. `src/` changes are only in: `crates/core` (config),
    `crates/client_gateway`, `crates/auth`, `crates/substrate` (runtime
    auth wiring), and the Hub UI. Plus `deploy/`, `Dockerfile` and tests.
12. `gateway_session_e2e.rs` is lower in the oversized list, the duplication
    baseline is correct, and no Rust file passes 800 lines.
13. `mise run verify` passes with no `--skip`.
14. One PR.

## 11. What A6 does not do

- No TLS in the gateway (backlog row).
- No proof step for `local`; it stays a same-machine convenience.
- No change to `delegated-key`, session tokens, cookies or the router.
- No peer address in the route preamble (Q2 (c)).
- No CSP or framing response header (`frame-ancestors` stays in the backlog).
- No browser access for a visitor who has no node (the shop change).
- No change to other roles' bind addresses.
- No Hub login change (F1).
- No client guide (A7).
- No Docker image build in CI.

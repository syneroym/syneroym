# M06C Slice C1.1 — The Node Auth Service and the Dumb Gateway: Implementation Plan

> **Scope, from [task.md](task.md)'s slice table and
> [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**
> The client gateway stops being a per-person identity broker and becomes a
> dumb proxy with an `identity_mode` (`open` / `login` / `fixed`); a new node
> **auth service** (`crates/auth` → `syneroym-auth`) becomes the node's
> identity provider; and the person session becomes a short-lived UCAN signed
> by the auth service, carried in the `syneroym_session` cookie and verified
> by each service that needs the caller. This is item **(1)** of ADR-0024's
> slice breakdown, and nothing else in that breakdown.
>
> **Depends on:** C1 (Complete 2026-08-25).
> **Blocks:** C2 (its §10 login flow is rewritten against this model) and C3
> (its signing interface is specified against this model, not against
> M06B B1).
>
> **The ADR is the design of record.** This plan does not restate it. Where a
> decision is already made there, this plan cites the section and says what
> C1.1 must build; where the ADR leaves a question open, §11 carries it open
> rather than closing it here.
>
> **No product code.** C1.1 writes no Roym service, no Hub UI, and no browser
> TypeScript. The browser half of `delegated-key` login — importing
> `session-key.json`, holding the temporary key in IndexedDB, signing the
> challenge — is C2's, per ADR-0024's breakdown item (2). C1.1's obligation to
> C2 is the wire contract and the file shape, not the page.

---

## §0 What C1 handed C1.1, and what is missing

C1 completed the dual-build shim (`AppHost` bounds eight traits; inbound HTTP
arrives through `HttpSink` / `WebSocketSink` and `NativeHttpRegistry`). It
touched no identity path. So what C1.1 inherits is what **M06B B1** shipped,
unchanged, plus C1's native-service machinery to build the new service on.

| # | What exists on `main` today | What C1.1 needs instead | Section |
|---|---|---|---|
| 1 | The gateway owns an in-memory `SessionStore` and mints a per-person delegation onto the route preamble (`crates/client_gateway/src/gateway.rs`, `session.rs`) | No session store in the gateway at all. Transport identity is a function of the configured `identity_mode` (ADR §1) | §2 |
| 2 | The gateway runs `session::classify` over a fixed list of `/_syneroym/session/*` paths and intercepts each one | **One** routing rule: traffic addressed to the auth service is proxied to it like any other service (ADR §1) | §2 |
| 3 | No node identity provider exists. Login is `roymctl session login` against gateway-owned endpoints | `crates/auth` — a `SynSvc` serving `/challenge`, `/login`, `/methods`, `/whoami`, `/logout`, `/refresh`, and minting session tokens (ADR §2) | §3 |
| 4 | A service learns the caller from a preamble delegation resolved by the handshake (`caller-auth = delegated`) | A service verifies a signed session token against the **auth service's** public key and reads the master DID plus `fct` (ADR §3) | §4, §6 |
| 5 | No way to put a person-controlled signing key in a browser without exporting the master key | `roymctl session delegate` — mint a scoped, short-TTL delegation for a fresh temporary key and emit `session-key.json` (ADR §4a step 1) | §5 |
| 6 | Every deployment gets one login model, whether or not it wants one | Three modes. `fixed` removes the auth service from the path entirely for a single-person node (ADR §5) | §7 |
| 7 | `passthrough_with_conn` swallows the socket, so a keep-alive-reused connection lands a second `/_syneroym/session/*` request on the wrong service → intermittent 405 (ADR §P1) | Nothing after the first request needs re-classification, so the bug has no surface left. `passthrough_with_conn` itself is unchanged | §2 |

**What C1.1 does *not* inherit a problem from.** C1's work — the eight
`AppHost` traits, `NativeHttpRegistry`, the parity harness — is untouched by
this slice. The auth service is a native service and uses the same
registration machinery every other native service already uses.

---

## §1 What this plan deliberately does not contain

C1's and C2's plans open with a "Findings from reading the tree" section
grounding every later section in a read line of code. **This plan has no such
section.** It was written from the ADR, against a tree it did not re-read at
implementation depth, and inventing findings would make it less trustworthy,
not more.

The implementing session owes that read **first**, and must reconcile at
least these before writing code:

1. **Where the shared token-verification helper can live without a dependency
   cycle.** ADR-0024's consequences table names `crates/rpc` / the service
   host. `syneroym-rpc` already depends on `syneroym-ucan`, `syneroym-fdae`
   and `syneroym-identity` (C1 plan `D-C1-5`), which is the right shape — but
   confirm it, and confirm the WASM guest side can reach the same helper.
2. **What exactly `session::classify` reaches** in
   `crates/client_gateway/src/gateway.rs` besides the six paths, so the
   deletion in §2 is complete rather than partial.
3. **How a native `SynSvc` is registered and addressed** through both the
   native-dispatch registry and a gateway hostname, so §3 picks the same path
   `crates/control_plane`'s services already use rather than a new one.
4. **What `roymctl session` does today** (`login`, `status`, `token`,
   `logout`) so §5's `delegate` verb fits the existing command rather than
   duplicating half of it.
5. **Whether the master-anchor / `revoked_keys` check the handshake performs
   is reachable from a native service**, since §3's `delegated-key`
   verification needs exactly that check (ADR §4a step 4).

Findings from that read belong in this file, as a `§1` written over this one.

---

## §2 The client gateway becomes a dumb proxy

**Design of record: [ADR-0024 §1](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**

What C1.1 builds:

1. **`identity_mode` on `[roles.client_gateway]`**, three values, `open` /
   `login` / `fixed`. It decides one thing only: what transport identity the
   gateway puts on the route preamble — nothing, the gateway's own node key,
   or a configured person delegation. The ADR's table is the specification;
   do not add a fourth mode.
2. **Delete `SessionStore` and every `/_syneroym/session/*` interception.**
   `session::classify` goes with it. This is a deletion, not a deprecation —
   the product is unreleased, so no compatibility shim and no version ladder
   (`D-06C-1.1-3`).
3. **One auth-routing rule.** A request whose `Host` (or the
   `/_syneroym/session/` path prefix) names the auth service is proxied to
   the auth service exactly like any other service. There is no per-path
   table.
4. **`passthrough_with_conn` is unchanged.** It is not the bug; needing to
   re-classify a second request on the same connection was. Once the gateway
   intercepts nothing, keep-alive reuse is ordinary — the service side has
   always served many requests per connection, which is why `/assets/*` and
   `/rpc` work today.

**P1 is fixed by (2) and (3), not by (1).** The optional `login`-mode
connection gate the ADR permits is defence in depth on top, and is carried as
an open question (§11, question 2) rather than assumed.

---

## §3 `crates/auth` — the node auth service

**Design of record: [ADR-0024 §2 and §4](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**

A new crate at `crates/auth`, package name `syneroym-auth`
(`D-06C-1.1-1` — the repo's directory-snake_case / package-kebab-case rule).
A native `SynSvc`, reachable through the gateway and the native-dispatch
registry, using the machinery C1 and earlier slices already built.

### 3.1 Endpoints

| Endpoint | What it does |
|---|---|
| `GET /_syneroym/session/challenge` | Returns a nonce for the `delegated-key` flow (ADR §4a step 3) |
| `POST /_syneroym/session/login` | One endpoint, `method` as a parameter. Dispatches on `method`; **rejects an unknown `method` with 400** (ADR §4, seam 1). Always answers the same way: `Set-Cookie: syneroym_session=<token>` + 200, or 401/403 |
| `GET /_syneroym/session/methods` | The enabled login methods, so a client renders the right screen without knowing the node's config (ADR §4, seam 2) |
| `GET /_syneroym/session/whoami` | The person's master DID and the token's `fct` claims |
| `POST /_syneroym/session/logout` | Ends the session, clears the cookie |
| `POST /_syneroym/session/refresh` | Re-issues a token. Semantics are an **open question** (§11, question 4) — settle it in the implementing session and record the answer here |

### 3.2 The two login methods C1.1 ships

**`delegated-key`** (ADR §4a) — the production default. The auth service:
verifies the signature over the nonce against the temporary key; verifies the
certificate chains to a master DID it accepts; resolves the master anchor and
checks it is not in `revoked_keys` (the check the handshake already performs);
checks the delegation's TTL and scope. On success it mints a token bound to
the **master DID** — the temporary key never appears as the token's subject.

**`local`** (ADR §4b) — the same-machine method. `{method: "local", identity:
"alice"}` loads that person's key from a configured key directory and trusts
that whoever reached this endpoint on this machine is authorized. This is
B1-era `login-local` + `person_identities_dir`, demoted from "the C2 login
flow" to one explicit method among several. **It exists only when
configured** (`D-06C-1.1-6`): with no key directory configured, `/methods`
does not list it and `/login` refuses it.

**Nothing else.** No `oidc`, no provider-wrapper interface, no account
mapping store, no atomic-binding ceremony (`D-06C-1.1-2`). See §10.

---

## §4 The session token, and the shared verification helper

**Design of record: [ADR-0024 §3](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**

The cookie carries a UCAN issued by the auth service: subject = the person's
**master DID**; short `exp`, never longer than the TTL of the delegation the
login relied on; an `fct` block of vouched-for account attributes
(`auth_method` is always present; `email` and friends only when a provider
that proves them exists, which in C1.1 none does); signed with the auth
service's key.

**One shared verification helper** (`D-06C-1.1-4`), serving native and WASM
services alike: parse, verify the signature **against the auth service's
public key — never the person's**, check `exp`, return the master DID and
`fct`. Home per the ADR's consequences table (`crates/rpc` / the service
host), subject to §1 item 1.

`fct` is a **cache**, not a source of truth, and each service trusts it
exactly as far as it trusts the auth service's key. Every service reads it
generically — never by branching on which method produced it — which is what
makes a future provider's new `fct` entry a no-op downstream (ADR §4,
seam 3).

Sessions do not survive a substrate restart (`D-06C-1.1-8`, ADR §2). The Hub
already treats "log in again" as an ordinary state, not an error
([task.md](task.md) failure-matrix row 17). Durability is a later, separate
choice and has a backlog row.

---

## §5 `roymctl session delegate`

**Design of record: [ADR-0024 §4a step 1](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**

A new verb on the existing `roymctl session` command: the person's master key
mints a `DelegationCertificate` for a freshly generated temporary keypair —
short TTL (`--ttl <hours>`), scoped to session auth — and the command writes
`session-key.json` holding the temporary private key plus the certificate.

The certificate format is unchanged; only its carrier is new
([ADR-0001, 2026-08-27 amendment](../../../decisions/0001-delegation-certificate-format.md)).
The file is the whole hand-off between the person's key and their browser, so
its shape is a contract C2 codes against: name the fields in this section
once they are chosen, and keep the master private key out of the file
entirely.

**The UX crux is getting that file into the browser**, and the ADR is explicit
that the first cut is this command plus drag-and-drop. QR-from-phone and a
browser extension are later, unscheduled, and get a backlog row.

---

## §6 The ADR-0016 gateway-path rework

**Design of record:
[ADR-0016's 2026-08-27 amendment](../../../decisions/0016-native-dispatch-identity-threading.md)
and [ADR-0024 §3](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**

For **gateway-origin traffic only**, the signal `caller-auth = delegated`
meaning "the gateway vouched for this person" is replaced by "a valid session
token is present, subject = the person's master DID". A gateway-origin
request's `CallerContext` is built from the verified token rather than from a
preamble delegation the gateway minted.

**Everything else in ADR-0016 stands unchanged** and C1.1 must not disturb
it: direct peer connections, `roymctl`, and cross-node proxied calls still
establish identity through the handshake and the preamble; `verify_preamble`
stays mandatory; `creator_id` is still the caller; the Admin-capability gate
and the send-the-proof / re-verify-at-the-destination rule are untouched.

The transport identity the gateway itself puts on the preamble is now a
function of `identity_mode` (§2): the gateway's node key in `login` mode, a
configured person delegation in `fixed` mode, nothing in `open` mode.

---

## §7 `fixed` mode

**Design of record: [ADR-0024 §5](../../../decisions/0024-client-gateway-identity-and-auth-service.md).**

For a node one person has exclusive secure access to. Config names the
identity (`fixed_identity_did`) and how the node acts for it. The gateway
injects that identity on every request: no cookie, no login endpoint, **no
auth service in the path at all**.

`GET /_syneroym/session/whoami` still answers — from the gateway or a trivial
shim — always returning the fixed identity (`D-06C-1.1-5`), so a client has
one code path and always sees "logged in as X". The trust model is stated
plainly: whoever can reach this substrate *is* X, and an application login
would be redundant ceremony.

It is genuinely single-identity. Switching between several DIDs on one node is
`local` mode's job, not this one's.

---

## §8 Decisions

| # | Decision | Why |
|---|---|---|
| **D-06C-1.1-1** | The new crate is `crates/auth`, package `syneroym-auth`. | The repo's rule: directory snake_case, package `syneroym-<kebab>`. ADR-0024 §2 names both. |
| **D-06C-1.1-2** | **C1.1 ships `delegated-key`, `local`, and `fixed`. Nothing else.** No provider-wrapper abstraction, no account mapping store, no atomic-binding ceremony, no OIDC. Unknown `method` → 400. | ADR-0024 §2/§4. Building a plugin interface for a single method is YAGNI; the three seams in §10 are what keep a later provider incremental, and they cost nothing now. |
| **D-06C-1.1-3** | The gateway's `SessionStore`, `session::classify`, and the `/_syneroym/session/*` interception table are **deleted**, not deprecated behind a flag. | The product is unreleased: schema and behaviour change in place, with no compatibility shim. Leaving a second, dead identity path in the gateway is exactly the special status ADR-0024 §P2 removes. |
| **D-06C-1.1-4** | **One** shared "verify session token" helper serves native and WASM services. No service verifies a token itself. | Signature-root and `exp` checking duplicated per service is where a divergence hides. One implementation is also what makes the `fct` seam (§10, seam 3) real. |
| **D-06C-1.1-5** | In `fixed` mode, `whoami` still answers, from the gateway or a trivial shim. | ADR-0024 §5. A client that must branch on "is this node in fixed mode?" before it can ask who it is has three login screens instead of one. |
| **D-06C-1.1-6** | `local` is enabled **only when a key directory is configured**. Absent that config, `/methods` does not list it and `/login` refuses it. | ADR-0024 §4b. A method that trusts "whoever reached this endpoint on this machine" must be opted into, never a default. |
| **D-06C-1.1-7** | C1.1 writes **no browser code**. The temporary key's IndexedDB / non-extractable `CryptoKey` handling is C2's. C1.1 owes only the wire contract and `session-key.json`'s shape. | ADR-0024's breakdown puts the Hub login screen in item (2). A login page written before the endpoint it calls exists gets written twice. |
| **D-06C-1.1-8** | Sessions do **not** survive a substrate restart. | ADR-0024 §2 keeps B1's deliberate choice for R1, and the Hub already treats "log in again" as an ordinary state. Durability is a separate decision with its own backlog row and its own consumer. |

---

## §9 Permitted differences

Named here so they are accepted differences rather than latent ones, in the
same spirit as C1's §14 list.

1. **The three modes are not feature-equivalent, by design.** `open` reaches
   `public: true` endpoints only and carries no person identity; `fixed`
   carries one unconditionally and has no login at all; only `login` has a
   session, a cookie, and a logout. A test asserting "whoami answers" holds in
   all three; a test asserting "login sets a cookie" holds only in `login`.
2. **`fct` is a cache with a short `exp`, not a live read.** An attribute that
   changes mid-session is stale until the token is re-minted. Bounded by
   `exp`, deliberately.
3. **A session dies with the substrate.** Restart is a logout for every open
   tab (`D-06C-1.1-8`).
4. **`local` trusts the machine, not the person.** Any local process that can
   reach the endpoint can obtain a session for any identity in the key
   directory — the same equivalence B1's `login-local` argument rested on, now
   stated as a property of one named method instead of a property of the
   product.
5. **Session lifetime is capped by the delegation's TTL** in `delegated-key`.
   Re-delegation on expiry means re-running `roymctl session delegate`; there
   is no silent renewal past the certificate's own expiry.
6. **ed25519 in WebCrypto is recent** (Chrome 137+, Safari 17+, Firefox 130+).
   The ADR accepts a vetted JS ed25519 library with an *extractable* key as a
   fallback for older browsers, with the XSS caveat stated. That fallback is
   C2's to build or refuse; C1.1 only must not assume the key is
   non-extractable in anything it verifies.

---

## §10 The three seams — and what is explicitly out of C1.1

**ADR-0024 §4 is the design of record.** External identity providers are not
built in C1.1. What C1.1 owes is that adding one later is incremental rather
than a rewrite, and that obligation is exactly three seams:

| # | Seam | What C1.1 must get right |
|---|---|---|
| 1 | **`POST /login` dispatches on `method`.** | Adding `oidc` later must be one new match arm plus its own parameters — not an API change. Unknown `method` values are rejected with 400 from day one, so a client cannot come to depend on lenient parsing. |
| 2 | **`GET /methods` returns the enabled list**, and the client branches on it. | Adding a method must be config plus one new UI branch, not a Hub rewrite. The Hub must never hardcode "the only way to log in". |
| 3 | **The token carries `fct`, and every service reads it generically.** | A provider that later adds `email` must need no token-schema change and no downstream change. No service may branch on `auth_method` to decide how to read the rest. |

**Out of C1.1, unscheduled, no owner** — these belong to a future
external-provider slice (ADR-0024 breakdown item 3), and are described in the
ADR only so C1.1 does not preclude them:

- the **provider-wrapper interface** (*given this login, return verified `fct`
  entries, or fail*);
- the **account mapping store** that links `(master DID, external subject)`;
- the **atomic-binding ceremony** that makes an OIDC login one request
  carrying both proofs — without which someone could prove possession of DID A
  while completing OIDC as another person's account and receive a token
  asserting both (ADR §4c);
- **OIDC / SAML / email magic-link** themselves.

Each gets a backlog row (§13). None gets a stub, a trait, or a config key in
C1.1.

---

## §11 Open questions carried from ADR-0024

Carried open, exactly as the ADR leaves them. The implementing session must
answer each and record the answer here — not in the ADR, unless the answer
changes the decision.

1. **The auth service's key identity.** Its own DID, the node DID, or a
   delegation from the node? A dedicated DID keeps "session tokens" and "the
   node speaks for itself" separable. This one is load-bearing for §4's
   verification root and should be settled first.
2. **The `login`-mode connection auth gate.** Worth the complexity, or is
   relying on services to reject an unauthenticated caller enough? Services
   must reject anyway; the gate is defence in depth plus a cleaner 401.
3. **Session durability across restart.** R1 says no. Is there a near-term
   consumer that changes that?
4. **`refresh` semantics.** Silent refresh while the underlying delegation is
   still valid, or force a re-login at `exp`? This decides how long a Hub tab
   stays usable.
5. **Multi-tab / multi-DID in one browser.** One session cookie per origin, so
   switching identity is logout + login. Acceptable for R1?

Two further questions this plan raises, both of ADR-0024's own making:

6. **Is `challenge` a separate `GET`, or folded into a two-legged `login`
   POST?** ADR §4a step 3 explicitly permits either and picks neither. §3.1
   assumes the separate `GET`; if the implementing session folds it, update
   §3.1 and C2's contract together.
7. **Does the `local` method reuse B1's `person_identities_dir` config key, or
   get a new one under the auth service's own config?** The ADR says "a
   configured key directory it can access" and names neither. The gateway is
   losing the key, so the auth service is the natural owner.

---

## §12 Tests

Shape only. The implementing session sizes each suite against the tree it
reads in §1.

1. **Gateway, all three modes** — `open` adds no preamble identity and
   reaches `public: true` endpoints only; `login` adds the gateway node key;
   `fixed` adds the configured person delegation. One suite, three
   configurations.
2. **The P1 regression, as a browser test.** Load a page, then `fetch` a
   session endpoint over the **same keep-alive connection**, and assert the
   second request is answered correctly rather than landing on the previous
   service as a 405. This is a claim about a real browser's connection reuse,
   so it belongs in the Playwright suite, not a Rust integration test. It is
   the test whose absence let P1 ship.
3. **`delegated-key` end to end** — `roymctl session delegate` → challenge →
   sign → login → a service sees the **master** DID, never the temporary one.
   Plus the refusals: expired delegation, wrong scope, a master in
   `revoked_keys`, a signature that does not verify, a certificate that does
   not chain.
4. **`local`** — enabled only when configured (absent config: not listed by
   `/methods`, refused by `/login`); an unlisted identity is refused; a name
   containing a path separator or `..` is refused.
5. **Unknown `method` → 400** (seam 1), and `/methods` reflects config
   (seam 2). These two are the seams' only proof, so they are not optional.
6. **Token verification** — a token signed by the wrong key is refused; an
   expired token is refused; the same helper gives native and WASM services
   the same answer for the same token.
7. **`fixed` mode** — `whoami` answers with the configured identity, there is
   no login endpoint in the path, and no cookie is required.
8. **Restart is a logout** — the session does not survive, and the client
   renders it as an ordinary state (failure-matrix row 17), not an error.

---

## §13 Documents and backlog owed

| Document | Edit |
|---|---|
| [status.md](status.md) | A C1.1 section: what shipped, the answers to §11's open questions, §9's permitted differences as accepted, and the verification evidence in §15 |
| [ADR-0024](../../../decisions/0024-client-gateway-identity-and-auth-service.md) | Status `Proposed` → `Accepted` when C1.1 lands, with a dated implementation amendment recording anything that shipped differently |
| [developer-guide.md](../../../developer-guide.md) | **Owed, and not done before C1.1 lands.** Its "Reserved Endpoints (`/_syneroym/session/*`)" section documents the gateway-owned B1 endpoints, which is accurate for the code on `main` today and wrong the moment this slice merges. Rewrite it for the auth service, add `identity_mode` and the `fixed` keys, and document `roymctl session delegate` |
| [slice-c2-implementation-plan.md](slice-c2-implementation-plan.md) | §10 is rewritten against this model when C2 rebases onto C1.1 (its `D-C2-6` `login-local` / `person_identities` design and its `10.0` WebAuthn forward-compatibility argument are both superseded by ADR-0024 §4/§P3) |
| [deferred-backlog.md](../../deferred-backlog.md) §7 | The browser-login row moves to "Recently resolved" against C1.1. The B1 rows that name `SessionStore` are re-grounded or closed by the deletion in §2 |
| [deferred-backlog.md](../../deferred-backlog.md), new rows | External identity providers (OIDC / SAML / magic-link) and the account mapping store that comes with them; session durability across restart; getting `session-key.json` into the browser by QR or extension; each of §11's open questions that is answered by deferring it |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | No scope change. D7/D8 stand as written; the ADR cross-reference near D8 is the whole edit, and it is already made |

---

## §14 Order of work

Each step compiles and its tests pass before the next begins.

| # | Step | Gate |
|---|---|---|
| 1 | The §1 tree read; answer §11 question 1 (the auth service's key identity) | This plan's §1 is rewritten with real findings |
| 2 | `crates/auth` skeleton — the crate, its registration as a native `SynSvc`, `/methods` returning an empty list | `cargo test -p syneroym-auth` |
| 3 | Session-token minting and the shared verification helper (§4) | A minted token verifies; a wrong-key and an expired token are refused |
| 4 | `roymctl session delegate` (§5) | The command emits a `session-key.json` whose certificate verifies |
| 5 | `delegated-key` on `/challenge` + `/login`, with every refusal in §12 item 3 | `cargo test -p syneroym-auth` |
| 6 | `local`, `/whoami`, `/logout`, `/refresh` | same |
| 7 | Gateway: `identity_mode`, delete `SessionStore` / `session::classify`, the one auth-routing rule (§2) | `cargo test -p syneroym-client-gateway`, `cargo test -p syneroym-substrate` |
| 8 | The ADR-0016 gateway-path rework (§6) | The direct-peer / `roymctl` / native-dispatch identity tests still pass untouched |
| 9 | `fixed` mode (§7) | Its own test configuration |
| 10 | The P1 browser regression test (§12 item 2) | `mise run test:e2e` |
| 11 | Docs and backlog (§13) | — |
| 12 | Full gate | `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets --all-features`, `cargo test --workspace`, `cargo audit`, `cargo deny check licenses`, `mise run test:e2e` |

**Step 7 is the destructive one.** Do it as its own commit so a revert is
clean, and do it *after* the auth service can already mint and verify a token
— otherwise the tree spends several commits with no working login at all.

---

## §15 What "done" means for C1.1

1. `crates/client_gateway` holds **no** session store, **no** path
   classification table, and **no** person-delegation minting. `identity_mode`
   decides its one identity behaviour.
2. `crates/auth` exists as a native `SynSvc` and serves all six endpoints.
   `POST /login` accepts `delegated-key` and `local`, and rejects any other
   `method` with 400.
3. A `delegated-key` login binds the token to the person's **master** DID; the
   temporary key never appears as a subject.
4. One shared helper verifies the session token, and both a native and a WASM
   service reach the same verdict for the same token.
5. A keep-alive-reused browser connection gets its session request answered
   correctly, proven by a browser test (§12 item 2) — **P1 is closed with
   evidence, not by construction**.
6. `fixed` mode serves a person with no login endpoint in the path, and
   `whoami` still answers.
7. Every identity path ADR-0016 covers other than gateway-origin is
   unchanged, proven by its existing tests passing untouched.
8. The three seams in §10 hold, proven by §12 item 5. Nothing from §10's
   "out of C1.1" list exists in the tree — no provider trait, no mapping
   table, no OIDC config key.
9. Every open question in §11 has a recorded answer, in this file or in
   [status.md](status.md).
10. No planning identifier (`C1.1`, `D-06C-1.1-4`, …) appears in any name,
    comment, or test this slice introduces, checked by grep.
11. `cargo +nightly fmt --all`, `cargo clippy --workspace --all-targets
    --all-features`, `cargo test --workspace`, `cargo audit`, `cargo deny
    check licenses` and `mise run test:e2e` are clean.
12. The §13 edits are made.

---

## §16 Verification evidence

_Filled in when C1.1 lands — placeholders until then._

1. `cargo test -p syneroym-auth`: _pending_
2. `cargo test -p syneroym-client-gateway`: _pending_
3. `cargo test -p syneroym-substrate` (gateway + session suites): _pending_
4. `cargo test -p syneroym-router` (ADR-0016 paths unchanged): _pending_
5. `cargo test -p roymctl` (`session delegate`): _pending_
6. `cargo test --workspace`: _pending_
7. `cargo +nightly fmt --all`: _pending_
8. `cargo clippy --workspace --all-targets --all-features`: _pending_
9. `cargo audit`: _pending_
10. `cargo deny check licenses`: _pending_
11. `mise run test:e2e` (including the P1 keep-alive regression test): _pending_

# ADR-0024: Client Gateway Identity Modes and the Node Auth Service

**Status**: Proposed (2026-08-27). Reshapes the person-session model M06B B1
shipped ([task.md M06B B1](../planning/milestones/M06C-roym-product/task.md),
line ~276; `crates/client_gateway/src/gateway.rs`,
`crates/client_gateway/src/session.rs` on `main`). Depends on
[ADR-0015](0015-ucan-capability-model.md) (UCAN capability tokens),
[ADR-0001](0001-delegation-certificate-format.md) (delegation certificates),
[ADR-0016](0016-native-dispatch-identity-threading.md) (how a caller
identity reaches a service).

**Milestone home.** M06C, as a new slice **C1.1**, sequenced before C2 and
C3. This is platform/substrate work by nature — but M06C's task.md already
gives platform gaps their own slices ("Where a capability is genuinely
missing, this document names it as a gap and gives it a slice"), and M06C
exists precisely as "the first honest test of the foundations; if any M06B
decision was wrong, M06C is where it shows". The B1 gateway-session
mechanism is such a decision: it did not survive contact with the Hub
(§P1). M06B stays Complete; this slice reshapes what B1 shipped rather than
reopening that milestone. Roym is the first consumer, not the reason.

---

## Context

### What ships today (M06B B1, on `main`)

The client gateway is a per-person identity broker:

1. It owns an **in-memory `SessionStore`**
   (`crates/client_gateway/src/gateway.rs`) — "Empty at boot and after every
   restart by design". A person session maps a `syneroym_session` cookie to
   a `person_did` plus a cached delegation.
2. It **classifies every inbound request** against a fixed set of
   `/_syneroym/session/*` paths (`session::classify`) — `challenge`,
   `login`, `logout`, `whoami`, and (slice C2 adds) `identities`,
   `login-local`.
3. For a request it does not intercept, it calls
   `SyneroymClient::passthrough_with_conn`, which sends **one** route
   preamble — carrying the owner→node delegation cert when a session is
   active, so the destination service sees `caller-auth = delegated` and the
   person's master DID (M06B B1) — and then pipes the raw socket to the one
   upstream service stream for the connection's lifetime
   (`io::copy_bidirectional`).

The person's identity is thus a **transport-layer** fact: a delegation cert
in the route preamble, minted by the gateway, resolved by the destination's
handshake.

### Three problems

**P1 — the gateway swallows the socket, so keep-alive breaks session
routing.** `passthrough_with_conn` hands the entire TCP connection to the
service after the first request. A browser that reuses the connection
between a page load and a `fetch('/_syneroym/session/whoami')` has the
second request land on the service, where `/_syneroym/session/*` is an
unknown route → HTTP 405. Chrome's connection reuse is timing-dependent, so
in a real browser this is **intermittent login failure**, not a clean
error. It is masked today only because nothing drove a browser through the
flow; M06C slice C2's `roym-hub.spec.ts` is the first test to hit it. The
C2-era mitigation (rewrite the forwarded request's `Connection:` header to
`close`) was reverted because it also kills the persistent connections the
WebRTC blind-tunnel and streaming paths depend on.

**P2 — the gateway is not the right home for identity.** A gateway that
mints a per-person delegation on every request is doing an identity
provider's job inside a reverse proxy. It forces the identity mechanism to
live at the transport layer, couples every deployment to one login model,
and makes "the gateway should be dumb" impossible. Real web systems put the
IdP behind the proxy, as a service.

**P3 — WebAuthn is the wrong primitive for Syneroym.** The natural "browser
login" answer is WebAuthn/passkeys, but its keys live in an authenticator
the user cannot export or fully control, which fights Syneroym's premise
that a person *owns* their DID and key. Syneroym needs a login mechanism
where the person's key stays under the person's control.

### What is out of scope

The hosted / remotely-bound client gateway serving browser-only consumers
(spec D8, `D-06B-5`) is not an M06C R1 target — every participant installs a
substrate. This ADR keeps that assumption: the browser and the gateway are
on the same machine, or reach each other over a channel the operator
trusts. It does not solve untrusted-network browser access.

---

## Decision

### 1. The client gateway becomes a dumb proxy with an identity **mode**

`[roles.client_gateway]` gains `identity_mode`:

| Mode | Transport identity added to the preamble | Person identity | What is reachable |
|---|---|---|---|
| `open` | none | none (or whatever the app authenticates itself, via its own cookies/headers) | `public: true` endpoints only |
| `login` | the gateway's own node key | a signed session token in the `syneroym_session` cookie, verified by each service | everything, gated on a valid session |
| `fixed` | a configured person delegation | a single configured DID, unconditionally | everything, as that person |

The gateway no longer runs `session::classify` on individual paths. It has
**one** routing rule for auth traffic: a request whose `Host` (or path
prefix `/_syneroym/session/`) names the auth service is proxied to the auth
service like any other service. The gateway's `SessionStore` is deleted.

In `login` mode the gateway MAY additionally refuse to proxy a connection
whose first request carries no valid session cookie (returning 401), an
`nginx auth_request`-style connection gate. That is a policy layer on top of
the routing change, not the fix for P1 — P1 is fixed by the gateway no
longer intercepting session paths at all, after which keep-alive reuse is
harmless (the service side already serves multiple requests per connection,
which is why `/assets/*` and `/rpc` work today).

### 2. A node **auth service**

A new native service (`crates/auth` → `syneroym-auth`, a `SynSvc` reachable
through the gateway and the native-dispatch registry). It is the node's
identity provider. Responsibilities:

- **Session establishment.** `POST /_syneroym/session/login` — one endpoint,
  method as a parameter (see §4).
- **Session token minting.** On success it issues a short-lived UCAN
  (§3), signed with the **auth service's own key** (a stable node-level
  identity, resolvable like any DID), and sets it as the `syneroym_session`
  cookie.
- **Session queries.** `GET /_syneroym/session/whoami` (returns the person
  DID and the token's `fct` claims), `GET /_syneroym/session/methods`
  (returns the enabled login methods, so the Hub renders the right screen),
  `POST /_syneroym/session/logout`, `POST /_syneroym/session/refresh`.

**Scope of this ADR / slice C1.1: the proprietary DID login only.** Today
there is exactly one identity mechanism — the delegated-key
challenge/response (§4a), plus the `local` (§4b) and `fixed` (§5)
conveniences. External providers (OIDC, SAML, email magic-link) are **not
built here**. What C1.1 owes is that adding one later is incremental, not a
rewrite — see §4 for the three seams that guarantee that. The account
mapping store, the provider-wrapper abstraction, and the atomic-binding
ceremony (§4c) all belong to that future slice and are described here only
so C1.1 does not preclude them.

Sessions still do not survive a restart (the store is deliberately not
durable across reboots for R1 — the Hub already treats "log in again" as an
ordinary state, `task.md` row 17). Making it durable is a later, separate
choice.

### 3. The session token: a short-lived signed UCAN, verified by each service

The cookie value is a UCAN issued by the auth service:

- **Audience / subject**: the person's **master DID** — the durable identity.
  A temporary delegated key used only at login (§4) never appears here.
- **`exp`**: short (hours), and `≤` the TTL of any delegation the login
  relied on.
- **`fct` (facts)**: a snapshot of the account attributes the auth service
  vouches for — `email`, `email_verified`, `display_name`, `auth_method`
  (`delegated-key` | `local` | `oidc` | `fixed`), `oidc_iss`/`oidc_sub` when
  applicable. This is a *cache*; the auth service's mapping store is the
  source of truth. Short `exp` bounds the drift.
- **Signature**: the auth service's key.

**Downstream verification.** A service that needs the caller identity:
parses the token, verifies its signature against the auth service's public
key (known from node config / registry — **not** the person's key), checks
`exp`, reads the master DID and `fct`. It trusts `fct` exactly as far as it
trusts the auth service's key — the same trust relationship a delegation
cert has today, moved from the transport preamble to an application-layer
cookie.

This is the piece that reshapes ADR-0016's "caller identity reaches the
service" story for gateway-origin traffic: identity arrives as a verified
cookie token, not a preamble delegation. Native and WASM services need a
small shared "verify session token" helper. `caller-auth = delegated` (the
B1 signal) is replaced by "a valid session token is present, subject =
`<master DID>`".

### 4. Login methods — one endpoint, one common DID mechanism, room to grow

`POST /_syneroym/session/login` takes a `method` and its parameters.
`GET /_syneroym/session/methods` lists which are enabled. The response is
always the same: `Set-Cookie: syneroym_session=<token>` + 200, or 401/403.

**The identity is always a DID, established by a delegated-key
challenge/response.** C1.1 ships `delegated-key` (§4a); `local` (§4b) and
`fixed` (§5) are the same-machine conveniences. Nothing else.

**The three seams that keep a future external provider incremental** — C1.1
must get these right, and needs nothing else for it:

1. `POST /login` dispatches on `method`. Adding `oidc` later is a new match
   arm and a new set of parameters, not an API change. Reject unknown
   `method` values with 400.
2. `GET /_syneroym/session/methods` returns the enabled list, and the Hub
   branches on it. Adding a method is config + one new UI branch, not a
   Hub rewrite.
3. The session token carries `fct` (§3) and every service reads it
   generically. A provider adding `email` later needs no token-schema
   change and no downstream change.

§4c below sketches how OIDC would slot in, **for design-preclusion checking
only — it is not part of C1.1**. The account mapping store, a
provider-wrapper abstraction, and the atomic-binding ceremony are that
future slice's to build. Do not build a plugin interface for a single
method now (YAGNI); just don't shape the `method` dispatch so that adding
one is a rewrite.

#### 4a. `delegated-key` (the production default)

The person's master key never enters the browser. Instead:

1. **Out of band** — a companion command (`roymctl session delegate
   --ttl <hours>`), or a paired device — the master key mints a
   `DelegationCertificate` for a freshly generated temporary keypair, short
   TTL, scoped to session auth. This yields a small `session-key.json`
   (temp private key + cert).
2. **Browser** — import it once, store the private key in **IndexedDB** as a
   **non-extractable WebCrypto `CryptoKey`** (ed25519 is now supported in
   Chrome/Safari/Firefox), so page JS can sign with it but cannot read the
   bytes — an XSS mitigation. The cert is stored alongside.
3. **Login** — `GET /_syneroym/session/challenge` → nonce; sign the nonce
   with the temp key; `POST /login {method: "delegated-key", temp_did,
   cert, nonce, sig}`. (The challenge MAY be folded into a two-legged
   `login` POST instead of a separate GET.)
4. **Auth service** — verify the signature against the temp key; verify the
   cert chains to a master DID it accepts; resolve the master anchor, check
   it is not in `revoked_keys` (the check the handshake already does),
   check the delegation's TTL and scope. Mint the token bound to the
   **master DID**.

Blast radius of a compromised browser: one soon-expiring, narrowly-scoped
delegation, revocable from the master's revocation list. Session lifetime is
capped by the delegation's TTL; re-delegation on expiry (re-run the
command).

#### 4b. `local` (same-machine dev / small deployments)

`POST /login {method: "local", identity: "alice"}`. The auth service reads
`alice.key` from a configured key directory it can access and trusts that
whoever reached this endpoint on this machine is authorized (optionally
gated behind a local unlock passphrase). This is today's `login-local` +
`person_identities_dir`, demoted from "the M06C C2 login flow" to an
explicit same-machine method — enabled only when configured. It is the
multi-identity picker for the case where the keys genuinely are on the node
(the `fixed` mode of §1 is the single-identity version of the same idea).

#### 4c. `oidc` — NOT in C1.1; sketched only to check C1.1 does not preclude it

Wraps an external OIDC provider. The provider proves an **email / external
account**, not a DID. So an OIDC login is a delegated-key login **plus** an
OIDC round-trip, **bound into one atomic ceremony**:

- client completes PKCE against the provider directly, then
  `POST /login {method: "oidc", delegated_key: {...}, id_token}` — both
  proofs in one request;
- the auth service verifies the delegated-key proof (→ master DID) and the
  `id_token` (→ `iss`/`sub`/email), then checks `(master DID, oidc_sub)`
  against its mapping store — matches an existing link, or, on first joint
  login, records the link;
- the token is minted with `email` etc. added to `fct`.

Without the binding, someone could prove possession of DID A while
completing OIDC as `bob@corp.com` and receive a token asserting "DID A,
email bob@corp.com" when A was never bob's. The ceremony MUST bind the two
proofs and MUST check consistency with the mapping.

The provider-wrapper is a thin internal interface: *given this login,
return verified `fct` entries, or fail*. OIDC is one implementation; others
(SAML, email magic-link) are added the same way, later.

### 5. `fixed` mode — no auth service in the path

For a node one person has exclusive secure access to (their own hardware,
localhost-only, behind their VPN). Config:

```toml
[roles.client_gateway]
identity_mode = "fixed"
fixed_identity_did = "did:key:..."
# and how the node acts for it: a delegation cert from that person's master
# → the node, long-lived and scoped; or the node simply holds the key.
```

The gateway injects the configured identity on every request. No cookie, no
login endpoint, no auth service. `GET /_syneroym/session/whoami` still
answers (from the gateway, or a trivial shim), always returning the fixed
identity, so the Hub needs no special code path — it always sees "logged in
as X". Trust model: whoever can reach this substrate *is* X; application
login would be redundant ceremony. This is the common case for someone
running Roym for themselves. It is genuinely single-identity — switching
between multiple DIDs on one node is `local` mode's job.

---

## Consequences

### What changes

| Component | Change |
|---|---|
| `crates/client_gateway` | Delete `SessionStore` and all `/_syneroym/session/*` interception. Add `identity_mode`. `open`: forward, no preamble identity. `login`: forward + gateway node key + (optional) connection auth gate. `fixed`: forward + configured person delegation. `passthrough_with_conn` unchanged — it is now harmless because nothing after it needs re-classification. |
| `crates/auth` (new) | The node auth service: `/login` (`delegated-key`, `local`), `/challenge`, `/methods`, `/whoami`, `/logout`, `/refresh`; session-token minting. **No** account mapping store, provider wrapper, or OIDC in C1.1 — those come with the external-provider slice. |
| `crates/rpc` / service host | A shared "verify `syneroym_session` token" helper for native and WASM services — signature against the auth service key, `exp`, extract master DID + `fct`. |
| `crates/router` / ADR-0016 path | Gateway-origin identity now arrives as a cookie token, not a preamble delegation. The `caller-auth = delegated` signal is replaced by "valid session token, subject = master DID". Direct peer / `roymctl` / native-dispatch identity threading is unchanged. |
| `apps/roymctl` | `roymctl session delegate` — mint a scoped, short-TTL delegation for a fresh temp key, emit `session-key.json`. |
| Roym `web` service (C2) | Its `/rpc` forwarding and `whoami` calls read the session token / `whoami` response instead of relying on the gateway's preamble delegation. §10 of the C2 plan is rewritten against this model. |
| Roym Hub UI (C2) | Login screen driven by `GET /_syneroym/session/methods`: "upload session key" (`delegated-key`), "pick identity" (`local`), or nothing (`fixed`). The `methods`-driven rendering is what lets a "sign in with …" button appear later with no Hub rewrite. Temp key handling: import → non-extractable `CryptoKey` in IndexedDB → sign challenges. |
| Slice C3 (signing interface) | "Signs under the person's delegated key" is re-grounded: the signing key is the **instance/service key or a per-person delegation the substrate holds**, and the *caller* is identified by the session token. C3's interface shape should be settled against this ADR, not against B1. |
| `roym-hub.spec.ts` (C2) | Tests 1–2 rewritten for the real `delegated-key` flow. Playwright injects the temp key into IndexedDB via `addInitScript` (or `global-setup` generates the delegation and hands it to the page) — no virtual authenticator needed, unlike WebAuthn. |

### What is superseded

- **M06B B1's "person identity at the client gateway"** as a
  gateway-minted preamble delegation. The *capability* (a guest handler sees
  the calling person) survives; the *mechanism* moves to a verified cookie
  token and the auth service.
- The deferred-backlog row "No browser login flow for person sessions
  (M06C)" — replaced by this ADR + its slice.
- The C2-branch backlog rows on the gateway keep-alive bypass (P1) — this
  ADR is the fix.

### Costs and risks

- **A new service and a session-token verification path in every service.**
  Real work, but it is normal web-app architecture, and it removes the
  gateway's special status.
- **The UX crux is getting `session-key.json` into the browser.** First cut:
  the `roymctl` command + drag-and-drop. QR-from-phone and a browser
  extension are later.
- **`fct` as a trust surface.** A service trusting `email` in the token
  trusts the auth service verified it — fine within one installation. Across
  installations (the R3 cross-installation-trust rows) it is a federation
  decision whether node B honours node A's auth service claims. Out of scope
  here; noted so R3 does not assume it.
- **ed25519 in WebCrypto** is recent (Chrome 137+, Safari 17+, Firefox
  130+). A fallback to a vetted JS ed25519 library with an extractable key
  is acceptable for older browsers, with the XSS caveat stated.

---

## Sequencing

**M06C slice C1.1 — land before C2 and before C3.**

- C2's §10 (the Hub login flow) and C2's `roym-hub.spec.ts` are written
  against the model this ADR replaces. Landing C2 first means shipping a
  known-intermittent login to `main` behind skip-marked tests, then
  reworking it — the Hub UI + e2e rework done twice.
- C3's signing interface is specified as signing "under the person's
  delegated key". The identity model this ADR settles changes what that
  sentence means. C3 should be designed against the final model.
- C2 is not time-sensitive (confirmed). The auth-service slice is bounded
  (an IdP with a signed-token cookie and a delegated-key method is a
  well-trodden pattern), and OIDC / the mapping store / the linking UX split
  cleanly into a follow-up slice — the `delegated-key` + `local` + `fixed`
  path is the MVP.

**Slice breakdown:**

1. **C1.1 — auth-service MVP + dumb gateway.** `identity_mode`; delete the
   gateway `SessionStore`; `crates/auth` with `/challenge` `/login`
   (`delegated-key`, `local`) `/methods` `/whoami` `/logout` `/refresh`;
   session-token minting + the shared verification helper; `roymctl session delegate`;
   `fixed` mode. Rework ADR-0016's gateway-origin identity path. Gets the
   three §4 seams right so (3) is incremental. **This is the C2 gate.**
2. **Roym C2 rework** (folded into C2 when it resumes): Hub login screen,
   IndexedDB temp-key handling, `web` service reads the token, e2e rewrite.
3. **External providers** (future, no committed date): a provider-wrapper
   abstraction, an OIDC implementation, the account mapping store, the
   linking UX, `fct` enrichment. Not scheduled. Slots onto (1)'s seams with
   no change to the gateway, the token shape, or existing services.

(1) is the gate for C2. (3) can land any time after (1), or never.

---

## Open questions

1. **Auth service key identity.** Its own DID, or the node DID, or a
   delegation from the node? A dedicated DID keeps "session tokens" and
   "node speaks for itself" separable.
2. **Connection auth gate in `login` mode** — worth the complexity, or is
   relying on services to reject an unauthenticated caller enough? (Services
   must reject anyway; the gate is defence in depth + a cleaner 401.)
3. **Session durability across restart** — R1 says no; is there a near-term
   consumer that changes that?
4. **`refresh` semantics** — silent refresh while a delegation is still
   valid, vs forcing re-login at `exp`. Affects how long a Hub tab stays
   usable.
5. **Multi-tab / multi-DID in one browser** — one session cookie per origin;
   switching identity = logout + login. Acceptable for R1?

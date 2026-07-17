# Access Control Design (Grants + Policy)

> **Status: Reviewed, one question open (§9.1).** A standalone design doc.
> Reviewed 2026-07-16; §10's Fork B is decided and §9's questions are settled
> except whether app abilities and policy permissions unify. It supersedes
> nothing yet — it splits into an amendment to
> [ADR-0015](../decisions/0015-ucan-capability-model.md) (the grant layer) and
> D-04-02 (the policy layer, [M04B](milestones/M04B-fdae-policy/task.md)), per
> §11.
>
> **Provenance.** Synthesized 2026-07-16 from three sources that were each
> partially right: the FDAE material in
> [system-requirements-spec.md:971-985](../system-requirements-spec.md) and
> [system-architecture.md:1826-1848](../system-architecture.md) (condensed from
> the fuller `docs/archive/authorization-engine-spec.md`), the capability model
> shipped in [ADR-0015](../decisions/0015-ucan-capability-model.md) and
> `crates/ucan`, and the dimension checklist in `docs/archive/fdae-scratch.md`.
> Archive material is explicitly non-authoritative (AGENTS.md); it is mined
> here for ideas, not cited as requirement.

---

## 1. The mental model

> **A capability says what you were handed. A policy says what the data allows.
> You get the intersection.**

If a user remembers one sentence about Syneroym access control, it should be
that one. Everything below is an elaboration of it.

---

## 2. Why two layers (the vetting verdict)

The two existing designs are not competitors. Each is silent exactly where the
other speaks, and neither is usable alone.

**FDAE is a policy engine with no credential model.** It compiles relationship
chains into SQL beautifully, but it has no notion of how a caller proves who
they are, no delegation, no attenuation, no revocation, and no cross-substrate
trust. It assumes a `caller` string materializes from somewhere trustworthy.

**The UCAN model is a credential model with no policy artifact.** It proves who
handed what to whom, offline and across trust boundaries — but it has nowhere to
write down "who may see which rows." The temptation (visible in
`fdae-scratch.md`) is to smuggle that into caveats as function calls like
`inParentChain(callerId, "managerId")`, which reinvents FDAE's hierarchies
inside a signed blob, badly.

So: **grants bound the maximum authority a caller could possibly have; policy
decides what the data actually yields; the effective answer is the
intersection.** Neither layer can widen the other.

### Why intersection, specifically

An earlier draft argued this from **different owners**. That argument does not
survive contact with the common case: the UCAN delegator usually traces back to
the service owner, who also authored the policy — and a document *creator* is
not a policy author at all, but a data subject the policy *references*
(`creator == caller`). Same owner, both layers. The honest argument is
**different times and different granularities**:

- **Grant** — issued ahead of time, coarse, travels with the request,
  verifiable offline, revocable.
- **Policy** — evaluated per request, fine-grained, data-dependent, lives with
  the data.

The *different owners* case is real but rarer, and it is precisely the case the
capability layer exists for: **when a grant crosses a trust boundary.** Alice
delegates to `orders-svc`; that grant now traces to *Alice*, while
`reports-svc`'s policy traces to *its* owner. Neither may widen the other —
that is the confused deputy, and there intersection is load-bearing.

**On cost:** the two layers are not two comparable evaluations. Chain
verification is **per-session and cacheable** (ADR-0015 budgets < 5 ms
*cache-cold*); Tiers 1–2 are µs-scale string and prefix compares. Only Tier 3
touches SQL — the query you were running anyway. Grant constraints arrive there
as bound `?` parameters, so "issue grants once, parameterize the policy by
them" is not an alternative to this design; it **is** this design (§5.3, §5.6).

**The real cost is comprehension:** a denial now has two possible homes. This is
the dominant operational failure mode of layered authorization — AWS IAM
(identity ∧ resource ∧ SCP ∧ permission boundary) is formally correct and
nearly incomprehensible, to the point of requiring a policy simulator to use.

**Therefore §7.2 (the decision trace) is not optional.** It is the mitigation
that makes intersection affordable, and it must be designed in from the first
slice, not bolted on. If we are unwilling to build it, we should take Fork A in
§10 instead and drop to a single layer.

### What we deliberately don't copy from IAM

Four shortcuts, and they are the whole comprehensibility budget:

1. **Two layers, not four-plus.** No SCP, no permission boundary, no session
   policy (see below — conjunction absorbs two of them for free).
2. **No free-floating deny.** IAM's `deny` overrides an allow from anywhere in
   the org; finding the one that bit you is the nightmare. Our `exclusion`
   (§6.2) is scoped *inside* a named permission.
3. **One policy document per service.** "Who can do X" is one file, not a scan
   across principals, resources, and org units.
4. **The trace is mandatory** (§7.2), not a separate simulator product.

**What that costs us in expressiveness — mostly nothing, because
conjunction-attenuation (§5.4) absorbs two IAM layers for free:**

| IAM layer | Our equivalent |
|---|---|
| Session policy (narrow at assume-role time) | Chain attenuation — cryptographic, and it travels. Strict gain. |
| Permission boundary (ceiling on what a principal may be granted) | `covers` — a delegator structurally *cannot* pass on more than it holds. IAM needed a layer to bolt this onto a model that lacked it. |
| Resource policy | The FDAE policy document, which is also data-aware — IAM cannot answer "which rows" at all. |
| **SCP (org-wide ceiling)** | **Genuinely missing — deferred, see §9.6.** |

We also gain what IAM structurally cannot do: data-awareness, and delegation
across trust boundaries.

---

## 3. Prior art — what we take and what we refuse

| System | Take | Refuse |
|---|---|---|
| **UCAN** | Signed issuer→audience chain over `did:key`; attenuation; proofs. Already shipped in `crates/ucan`. | The 0.10 array-of-caveats "enclosure" check — intractable in general, and UCAN 1.0 dropped it for the same reason. |
| **Macaroons** | Caveats as *the* attenuation primitive. Third-party caveat + discharge is structurally the same shape as FDAE's cross-service fetch (§6.4). | First-party caveats as an open predicate language. |
| **Biscuit** | **Attenuation by conjunction**, not subsumption-checking (§5.4). The single most useful idea here. | Datalog in the token — unanalyzable for SQL pushdown. |
| **Zanzibar** | The relationship model; namespace configs; `union`/`intersection`/`exclusion` as the permission operators (§6.2). | The centralized tuple store and its data-streaming tax — this is precisely what FDAE's data-source bindings exist to avoid. |
| **Cedar** | A **typed schema validated at author time** catches most policy bugs before deploy. Permission boundaries validate intersection semantics at scale. | Cedar's own syntax; we already own a filter DSL (ADR-0007). |
| **OPA / Rego** | *(anti-lesson)* A general-purpose policy language cannot be analyzed, pushed down, or guaranteed to terminate. Justifies keeping the **default path** declarative and restricted — not the escape hatches, which are deliberately more general than Rego (§6.7). | The whole approach *as the default*. |
| **OAuth / Okta** | Scopes are coarse and must never become the fine-grained authorization surface — scope explosion is the classic failure. Reinforces: no row rules in grants. | Bearer-token semantics without attenuation. |

---

## 4. Layers at a glance

| | Layer 0 — Identity | Layer 1 — Grant | Layer 2 — Policy |
|---|---|---|---|
| **Question** | Who are you? | What were you handed? | What do the owner's rules allow? |
| **Artifact** | `did:key` + signature chain | UCAN `CapabilityToken` | FDAE policy document |
| **Owner** | The key holder | The delegator | The resource owner |
| **Travels** | With the request | With the request | With the data |
| **Evaluable offline** | Yes | Yes | No — needs the data |
| **Granularity** | Principal | Verb + resource selector | Row, column, relationship |
| **Enforced at** | Handshake | Tiers 1–2 (pre-SQL) | Tier 3 (in SQL) |
| **Status** | Shipped (M2, M04A B0/B1) | Shipped, needs §5 amendments | Not built — M04B |

---

## 5. Layer 1 — The grant (UCAN)

Amends [ADR-0015](../decisions/0015-ucan-capability-model.md). Everything here
is a delta against what `crates/ucan` ships today.

### 5.1 Resource URIs gain selectors

Today `with` is service-granularity (`synapp:<app>:svc:<svc>`), so "may call
`createOrder` but not `deleteOrder`" is inexpressible. Extend with an optional
interface-shaped selector path:

```
synapp:<app_instance_id>:svc:<service_id>[/<selector>]
substrate:<node_did>[/<selector>]
```

| Interface | Selector | Example |
|---|---|---|
| data-layer | `collection/<name>[/<id>]` | `collection/orders`, `collection/orders/42` |
| blob | `blob/<prefix>` | `blob/reports/` |
| messaging | `topic/<pattern>` | `topic/sensors/#` |
| rpc | `rpc/<method>` | `rpc/createOrder` |
| orchestrator | `app/<name>` | `app/*` |

Matching is segment-wise prefix covering; a trailing `/` or `*` is a prefix
wildcard. **`with` is the *what*, `can` is the *verb*** — keeping selectors out
of `can` keeps entailment a pure string-hierarchy question.

### 5.2 Two ability namespaces

- **Platform abilities** — closed, host-defined vocabulary with fixed
  entailment (`data-layer/*`, `blob/*`, `messaging/*`, `vault/*`,
  `app-config/*`, `substrate/admin`, and B7's `orchestrator/*`). Users never
  edit these; the host enforces them.
- **App abilities** — `app/<name>`, declared by the service owner **in the
  policy document**, entailment declared alongside. This delivers the
  `document/manage ⇒ document/move` flexibility `fdae-scratch.md` asked for.

**Hard rule:** an app ability can never entail a platform ability. Unknown
ability strings entail only themselves. Fail closed.

*(Today `Ability::tier` in `crates/ucan/src/capability.rs` hardcodes the
`data-layer` tiers and treats everything else as flat — that is the correct
platform behavior and stays; app abilities are an additional, policy-sourced
table consulted only within the `app/` namespace.)*

### 5.3 Caveats: the static/dynamic line

This is the heart of the design, and the point on which the two source designs
most directly disagree. ADR-0015 punts caveats entirely (a passthrough `Value`,
unread by `grants`/`covers`); `fdae-scratch.md` fills them with SQL-ish
predicates and function calls.

**Both are wrong, and the line between them is not arbitrary:**

> A caveat may constrain anything the **issuer knows at issue time** or the
> **verifier can see in the request itself**. A caveat may **never require a
> data lookup** to evaluate.

Four reasons this line and not another:

1. **Offline verifiability.** A caveat needing a DB read destroys a core UCAN
   property — the whole point of a capability is that its authenticity and
   scope are checkable without contacting anyone.
2. **Stable meaning.** A data-dependent caveat means something different on
   Tuesday than it did on Monday, because the data moved under it. The grant is
   supposed to be a record of what a delegator intended at a moment.
3. **Decidable attenuation.** Is `inParentChain(x)` narrower than
   `inCreatorTeam(y)`? Unanswerable without reading the data — so a verifier
   cannot tell escalation from attenuation.
4. **One evaluator, not two.** Relationship logic in caveats duplicates, in a
   second engine, exactly what the policy document already expresses — and the
   caveat copy cannot be compiled into the SQL pushdown.

Accordingly, the caveat forms:

| Caveat | Meaning | Composition along the chain |
|---|---|---|
| `where` | An **ADR-0007 MongoDB-style JSON filter document**. Not a new language — reuses `crates/data_db/src/filter.rs`, already written, already parameterized. | Conjunction (`AND`) |
| `fields` | `{allow: [...]}` / `{deny: [...]}` — this is CLS. | Allows intersect, denies union |
| `can_delegate` | Bool. Absent ⇒ `true` (today's behavior). | Logical AND; once false, terminal |
| `policy` | A **reference into the policy document's permission vocabulary** (e.g. `document.view`). The seam to Layer 2. | Set intersection |

**`policy:` is the resolution of the scratch's custom-function instinct, not a
rejection of it.** The instinct — a pointer to a rule a generic engine
evaluates — was right. The refinement is *whose* rule: the token does not carry
an expression to run, it names a role the resource owner authored. The token
stays statically verifiable; the data-dependent part evaluates where the data
is. Same architecture, evaluation lifted out of the token.

### 5.4 Attenuation: check the shape, stack the constraints

> **Resource and verb narrow by *checking*. Constraints narrow by *stacking*.**

- `with` — child's selector path must be prefix-covered by the parent's.
  Decidable, cheap.
- `can` — parent must `entail` child. Existing logic, unchanged.
- **caveats — no check at all.** The verifier **conjoins** every caveat along
  the chain.

The third point is the load-bearing simplification, borrowed from Biscuit.
Proving one arbitrary filter is a subset of another is intractable in general —
so don't try. Conjoining is **monotonically narrowing by construction**:

- parent `{dept: 5}` + child `{status: "open"}` → `dept=5 AND status=open`
- parent `{dept: 5}` + child `{}` → `dept=5` *(empty child cannot widen)*
- parent `{dept: 5}` + child `{dept: {$in: [5,6]}}` → `dept=5 AND dept IN (5,6)`
  ≡ `dept=5` *(the attempted widening is inert)*

A child literally cannot escalate via caveats, so there is nothing to verify.
This is why the caveat language must stay closed-form: conjunction is only
sound because every form is intersective.

### 5.5 Anchor and path are free — stop discarding them

`fdae-scratch.md` flags "caller's anchor DID carried across call legs" as a
gap. It is one — but it needs **no new mechanism**. The verified chain already
*is* the provenance record:

- **anchor** = the audience of the first non-root token — the original human or
  principal on whose behalf the chain acts.
- **origin location** = the leaf's issuer — the previous service in the call
  chain.
- **path** = the chain itself.

`SessionContext` today keeps only `subject_did` (the immediate caller) and
throws the rest away after `verify_chain`. Surface `anchor_did` and `path`
alongside it, and let policies bind **either `caller` or `anchor`** as a
terminal. That is confused-deputy prevention for the cost of not discarding
data we already verified — the strongest cost/benefit ratio in this document.

### 5.6 External identity: claims about a DID, never subjects

`fdae-scratch.md` wants OAuth userids/emails as first-class identity
dimensions; the architecture says normalize everything to DIDs. **The
architecture wins** — signatures, attenuation, and revocation all need a key,
and a subject without a key cannot participate in any of them.

But the underlying need is real and already served: external IDs live in
`claims`, which M04B binds as SQL `?` parameters. `crates/ucan/src/session.rs`
already has the correct guard — facts are trusted only when the leaf's issuer
is itself a trusted root, precisely so a self-authored leaf cannot inject
fabricated claims alongside a legitimately-proven capability.

*(For the avoidance of doubt: these are third-party identifiers — email,
username, OAuth `sub`. They are unrelated to AWS's `ExternalId`, which is a
shared secret for cross-account role assumption.)*

**Who mints the user's DID.** Only one answer preserves the model: **the user's
client holds the keypair**, and the service *attests* a binding to it. If the
service mints and holds the user's key, the service is merely acting as them and
the entire chain is theatre. This has a product dependency — users need keys
(the vault / Personal Data Homebase surface) — and it is M6+, since ADR-0015
ships only the `did:key` normalizer.

**The binding is a one-way attestation, not a mutual agreement.** The flow: the
user proves control of `did:key:X` (signs a challenge), proves control of
`alice@example.com` (email link / OAuth), and the service — having witnessed
both — signs `{iss: service_did, sub: did:key:X, email: alice@example.com}`.
The user co-signing would not make the claim *more true*: the assertion is "I,
the service, checked this," and the user cannot vouch for the service's own
verification. Mutual signing (as `ControllerAgreement` genuinely is, for
substrate ownership) buys a different thing — **consent**, "I agree this service
may assert my email to others." That is a privacy property, not a truth
property, and it belongs with the `[PRD-SAF]` consent work retargeted out of M4.
Do not conflate the two.

**Footgun: never join policy on email.** Emails get reassigned (corporate
accounts), change, and carry case-sensitivity and normalization traps. Inherit
those into authorization and you get silent misattribution. The service maps
email → DID **once**, at user creation, and stores the DID; policy joins on DID.
Email is a *lookup* key, never an *authorization* key.

### 5.6.1 `is_trusted_root` must become resource-scoped

Today the predicate is `|iss, _res| iss == admin_root` — a single node-wide
admin DID is the *only* party that can root a chain or assert trusted facts.
Fine while the node admin is the only issuer; wrong from B7 onward, because a
**service owner cannot grant `data-layer/read` on their own service** without
the node admin's involvement. It must become:

```
is_trusted_root(issuer, resource) = issuer == admin_root         // node-wide
                                 || issuer == owner_of(resource) // per-service
```

Two consequences:

1. It is what lets a service owner attest claims about *their own* users
   (the binding above) while being unable to assert anything about *another*
   service's resources.
2. **`owner_of(resource)` does not exist yet — it is B7's catalog owner field.**

That is exactly why `session.rs` carries its `TODO(B7)` here: its synthetic
`ResourceUri::substrate(leaf.issuer_did)` probe asks "is this issuer a root for
a made-up resource named after itself?", which under a resource-scoped predicate
reads the wrong scope and could wrongly trust facts.

### 5.7 One DID per principal (pairwise DIDs deferred)

**M4 ships one DID per principal, used everywhere.** A *principal* is anything
holding a key: a human user, a service, a device, another app.

`fdae-scratch.md` raises "service caller did (separate per service or common)."
It is worth stating the threat plainly, because it is **not** an access-control
question — with one `did:A` everywhere there is no `A1`/`A2` to link, and
whether `did:A` may call a service is an ordinary Tier-1 decision.

**The threat is privacy: cross-service correlation by colluding services.** With
one DID everywhere, two services that compare notes learn the same person used
both. Pairwise DIDs (`A1` at svc-1, `A2` at svc-2) defeat that — the same
mechanism as OIDC pairwise subject identifiers or Apple's Hide My Email. That
`A1`/`A2` case exists *only* if we deliberately mint per-service DIDs; it is not
something that arises on its own.

**Deferred, because it costs FDAE dearly and has no M4 consumer.** Pairwise DIDs
break cross-service relationship proofs (`A1` cannot be joined to `A2` without a
linking table that defeats the purpose) and break anchor propagation (§5.5) —
which DID is the anchor once a chain crosses services?

*If* we ever adopt it, the **app instance** is the natural boundary: it is
already the isolation boundary for the data namespace, the blob namespace, and
B6's KEK, and `synapp:<app_instance_id>:svc:<service_id>` already encodes it. A
principal would then be one DID per app instance — recognizable to every service
within an app, uncorrelatable across apps. Recording the shape, not adopting it.

### 5.8 Revocation

Reuses the existing DHT revocation check (`crates/router/src/handshake.rs`), per
ADR-0015. **The check must run against every token in the proof chain, not just
the leaf** — that is what makes B7's "revocable grant" actually revocable, since
the grant being revoked is a *proof* in some later chain, not its leaf.

---

## 6. Layer 2 — The policy (FDAE)

Feeds D-04-02. Deltas against `docs/archive/authorization-engine-spec.md`.

### 6.1 Three sections, deployed with the service

One document per service, referenced from the manifest (`policy_path`, per
M04B's migration strategy), versioned and JSON-Schema-validated at deploy — the
Cedar lesson from §3.

```yaml
version: "fdae/v1"

data:            # what objects exist, and where they live
relations:       # how objects connect (recursive: true for chains)
permissions:     # who can do what, as boolean paths over relations
```

Two deliberate removals from the archived spec, both of which delete concepts
rather than add them:

- **`data_sources` is deleted entirely.** The archived spec has policies
  carrying raw connection strings (`connection: "file:app_state.db?mode=ro"`).
  That directly contradicts Syneroym's model — one host-managed DB per service,
  guests never touch a database, `system-architecture.md:1829`. A relation is
  either **local** (this service's DB) or **remote** (`service: <logical-name>`,
  resolved through the app-context registry that already exists). This removes
  a config section, a class of misconfiguration, and a credential-leak surface.
- **`hierarchies` folds into `relations`** as `recursive: true`. It was never a
  separate kind of thing — just a self-join that needs `WITH RECURSIVE`. One
  concept fewer for the same expressiveness.

### 6.2 Permission operators — `union` is not enough

The archived spec offers only `union` (OR). That cannot express "everyone in the
department **except** contractors," which is a first-week requirement in any real
deployment. Adopt Zanzibar's three operators: **`union`, `intersection`,
`exclusion`**.

Deliberately **not** adopted: Cedar/IAM-style free-floating `deny` rules that
override any allow from anywhere. They are the single biggest driver of "why was
I denied" pain, and `exclusion` scoped inside a named permission covers the real
cases while staying compilable to SQL. *(Open — §9.)*

### 6.3 The two modes — one compiled block, used twice

Carried from `system-architecture.md:1842-1844`. The point that makes them one
feature rather than two engines: **it is the same compiled security block; only
what you wrap it around differs.**

**Mode A — you know the ID, you want a yes/no.** "May Alice delete document 12?"
You do not want to `SELECT` and then `DELETE`; you want a check. Take the
policy's `WHERE EXISTS (...)` block, add `AND documents.id = ?`, reduce to a
boolean.

**Mode B — you don't know the IDs; that *is* the question.** "Show me all
documents I can see." Take the guest's own query
(`SELECT * FROM documents WHERE status = 'open'`) and **wrap** it with the same
`WHERE EXISTS (...)`. SQLite prunes at the index level; unauthorized rows never
materialize.

In our codebase:

- **Mode B is transparent** — it is what happens on every `data-layer::query`.
  No new API.
- **Mode A is a new explicit call** — a `check`-style host function — needed
  because not every action is a query. Delete, publish, and invoke all want a
  point check with no rows to filter.

### 6.4 Stage-2 fetch: the scaling cliff, and why proofs get signed

**The mechanic first.** `reports-svc`'s policy says "Alice may view a document
if she manages the department that owns it." Local SQL resolves the department
(`engineering`), but `manager_of(engineering)` lives in `hr-svc` — a different
service, possibly a different node. SQL cannot reach it. So the engine
**pauses**, proxy-calls `hr-svc`, gets back `user:david`, and resumes:
`david == alice`? No → deny. **The "remote proof" is just that fetched value.**

The security of the *call* is not in question: `hr-svc` runs its own access
control on it, and the immediate leg validates. That is not what this section is
about.

**The problem is Mode B's scaling cliff.** If the query returns 100 documents
across 50 departments, does the engine make 50 proxy calls? That is precisely
the data-fetching problem FDAE claims to solve by pushdown — and **you cannot
push down across a service boundary.** So stage 2 needs **batching** ("managers
for these 50 departments") and **caching**, or Mode B does not scale at all.

**Caching a bare string is unsafe**, which is where signing earns its place. A
bare value has no expiry (so you cannot know when it went stale — §9.5's
new-enemy problem), no provenance (you cannot prove `hr-svc` said it), and
nothing to show in the trace. A **signed, TTL'd proof record** — *`hr-svc`
asserts, at time T, `manager_of(engineering) = david`, valid 60s* — makes
caching safe, auditable, and forwardable across hops.

*(The Macaroon third-party-caveat/discharge pattern is the same structure, and
is worth reading before B3 is designed. It is prior art, not the argument — the
argument is batching and safe caching.)*

### 6.5 Safety rails

Carried from `system-architecture.md:1847-1848`, with one correction: the
archived spec hardcodes a 15ms watchdog, which contradicts the architecture's
own "the default must be conservative but not hard-coded." **Keep it
configurable, default conservative.**

- `visited_track` path-concatenation cycle guard on every recursive block.
- `sqlite3_progress_handler` watchdog + configurable time budget →
  **default-deny** on timeout.
- Strict `?`/`:name` binding; no string concatenation, ever.
- Default-deny overall, with an explicit `public:` declaration so authors do not
  end up fighting the engine.

### 6.6 Non-SQL resources: one evaluator, N key extractors

The engine compiles relationship chains **into SQL**, which invites the
question: what about streams, pub/sub topics, blobs, and external systems? Do we
need a relationship evaluator per data-source type?

**No — and the reason is an asymmetry worth naming: non-SQL resources in
Syneroym do not have their own relationship graph. They have *keys into* the SQL
graph.**

| Resource | How it enters the graph |
|---|---|
| `topic/orders/42/status` | Extract `42` → evaluate the chain on `orders` in SQL |
| `blob/sha256:abc…` | Blobs are referenced as fields on records (`system-architecture.md`) → reach it through the record, in SQL |
| External system (`hr_api`) | Fetch the value, bind as `?` (§6.4) → the chain still terminates in SQL |

So the uniform model is **one evaluator (SQL) plus a per-interface key
extractor**, not N evaluators. Much cheaper, and it keeps every relationship
question answerable by the same compiled block (§6.3).

**The genuinely separate half is content filters.** A
`where` caveat over a *message payload* is not a relationship question at all —
it is a predicate over a JSON value. That needs a second **backend** for the
same DSL: `filter.rs` compiles an ADR-0007 filter document to SQL today; the
same document could be evaluated in-memory against a JSON payload. **One DSL,
two backends** — coherent and cheap, but with no M04B consumer. **Decision:
`where` caveats are restricted to data-layer for now**, and the in-memory
backend is the door left open.

### 6.7 Escape hatches, and the privileges they need

The declarative path must stay the default — it is the only one that is
analyzable, pushed down, and guaranteed to terminate (§3's OPA anti-lesson).
But SQL cannot express everything, and the escape hatches are **fully general**:
a WASM function with read-only host imports (data-layer read, session context,
env), fuel-metered under [ADR-0005](../decisions/0005-wasm-fuel-quota-schema.md),
is strictly more expressive than Rego.

The two hatches differ by **position relative to the SQL**, not by power:

| | Stage 2 (pre-SQL) | Stage 4 (post-SQL) |
|---|---|---|
| **Shape** | Lookup whose result is **bound into SQL as `?`** | Arbitrary computation over candidate rows |
| **Cost** | **One fetch per query** — participates in the pushdown | **Per row/batch** — cannot push down (N+1) |
| **Use** | The remote hop a relationship needs (§6.4) | Non-relational attribute checks |
| **Authority** | Proxy call under the **service's own** identity | Read-only imports under the service's identity |

**Stage 4 may look things up.** An earlier draft banned it, citing
fetch-then-filter, WASM isolation, and performance. Only the third holds: stage 4
sees rows the sieve **already authorized**, and a component with a read-only host
import *is* the standard capability model. The N+1 cost is real, but it is a
tradeoff to accept knowingly for the rare case SQL cannot express — not a
prohibition.

**Restrict-only survives as a security property, not a default.** If stage 4
could *widen*, a function querying under the service's authority could return
rows the policy denied — a real escalation. Restricted to `allow`/`deny`/
`redact` over already-approved rows, lookups are safe.

**On privilege: a policy that names a WASM function is executing code during an
authorization decision.** Both hatches run under the **service's own** identity,
never the caller's. That is not an escalation — the service owner authored the
policy and could equally have written the same call into their service code —
and running under the *caller's* authority sounds safer but breaks most real
policies (the caller usually cannot read the org chart). The requirement is that
every hatch is **declared in the policy** and **appears in the trace** (§7.2).

---

## 7. The intersection, and the trace that makes it survivable

### 7.1 Evaluation order

Fail closed at every tier. Tiers 1–2 are cheap and reject most bad traffic
before SQL is touched.

| Tier | Question | Source | Cost |
|---|---|---|---|
| 0 | Who are you? | Chain verify → `SessionContext` (identity, grants, claims, anchor, path) | ~ms, cacheable |
| 1 | May you touch this service at all? | Grant: does any capability name this resource? | µs |
| 2 | This verb, this collection/topic/method? | Grant: `can` entails, `with` covers | µs |
| 3 | Which rows and columns? | **Policy ∧ chain `where` caveats ∧ caller's own filter**, compiled into one statement | SQL |
| 4 | Any non-relational override? | Optional WASM ABAC — batched, fuel-metered, **restrict-only**; may look things up at N+1 cost (§6.7) | per batch |

The layers overlap in exactly one place — `where` caveats and policy filters both
`AND` into the same SQL statement — and that is fine, because both are
intersective. The grant is coarse/static/pre-SQL; the policy is fine/dynamic/
in-SQL. They are not symmetric in shape, only in authority.

### 7.2 The decision trace is load-bearing

Intersection's one real cost is that a denial now has two possible homes. **The
mitigation ships with the first slice, not later.** Every decision emits a
structured trace:

```
denied
  tier: 3 (data-plane)
  layer: policy
  permission: document.view
  path_failed: [creator -> management_chain -> caller]
  grant_would_have_allowed: true
```

`grant_would_have_allowed` is the field that matters: it tells the operator
*which layer to go edit*, which is the exact question AWS's policy simulator
exists to answer. Not a debugging nicety — the reason the design is affordable.

**If we will not build this, take Fork A (§10) instead.**

---

## 8. Worked example

Alice's client calls `orders-svc`, which calls `reports-svc` to render a
dashboard.

**Grant.** The app owner issued Alice `{with: synapp:acme:svc:reports/collection/orders, can: data-layer/read, caveats: {policy: "order.view", fields: {deny: ["margin"]}}}`. Alice's client re-delegated to `orders-svc` for this request, adding `{where: {region: "EU"}}`.

**Chain gives us:** subject = `orders-svc`, **anchor = Alice**, path =
`[Alice, orders-svc]`. Effective caveats = `where: {region: "EU"}` ∧
`fields: deny [margin]` ∧ `policy: order.view`.

**Tier 1–2:** the selector covers `collection/orders`; `data-layer/read` is
entailed. Pass, no SQL yet.

**Tier 3:** `reports-svc`'s policy defines `order.view` as *creator OR
in-creator's-management-chain*. It compiles against **`anchor`** — Alice, not
`orders-svc` — which is precisely the confused-deputy defense from §5.5. That
`WHERE EXISTS` block ANDs with the grant's `region: "EU"` and with the caller's
own `{status: "open"}` filter. `margin` is projected out by CLS.

**Result:** open EU orders Alice can see, sans margin — one SQL statement, no
row leaving SQLite that Alice was not entitled to. Had the policy been widened
to allow all orders, Alice would still only see EU ones; had the grant been
widened to all regions, she would still only see ones her management chain
reaches. **Neither owner can override the other.**

---

## 9. Decisions and the one question left

All review questions resolved 2026-07-16 except §9.1.

### 9.1 Live: do app abilities and policy permissions unify?

The one thing worth settling before D-04-02 builds either. `app/document.manage`
(an ability in a grant's `can`) and `document.manage` (a permission in the
policy's `permissions:` block) are suspiciously the same concept — a named verb
over a resource type. A grant carrying `can: app/document.view` *and*
`caveats: {policy: "document.view"}` names it twice.

If app abilities live in the policy next to `permissions:` (§9.2), the **`policy:`
caveat from §5.3 may collapse into `can` entirely**, leaving one vocabulary
instead of two. This is what Zanzibar does — `document:123#view@user:alice` has
no separate ability concept. Platform abilities (`data-layer/read`) would remain
distinct and host-fixed, since a service's own internal `put` goes through no
policy at all.

Cheap to notice now, expensive to discover once both mechanisms exist. It makes
the design *smaller*, which is the tiebreaker.

### 9.2 App-ability vocabulary lives in the **policy document** ✓

Not the manifest. Three reasons, in user-ergonomics order: authorization lives in
one artifact; the manifest is deliberately a "dumb, fully-resolved document"
(the anti-Helm principle, `system-requirements-spec.md`) — a deployment artifact,
not a semantics one; and the policy's `permissions:` block **already** names
permissions and defines their meaning. Naming an ability in the manifest while
defining it in the policy is split-brain vocabulary — the worst available
outcome. *(And see §9.1 — they may be one thing.)*

### 9.3 `exclusion` only; no free-floating `deny` ✓

Zanzibar's three operators (§6.2). Cedar/IAM-style deny-overrides-everything is
the single biggest driver of "why was I denied" pain, and `exclusion` scoped
inside a named permission covers the real cases while staying compilable.

### 9.4 Stage-2 proofs are signed and TTL'd ✓

Batching and caching are not optional if Mode B is to scale (§6.4); the only
question was whether the cached artifact is a bare value or a signed assertion
with an explicit expiry. **Signed** — it is what makes the cache safe, and it
feeds both the trace (§7.2) and §9.5.

### 9.5 Stale relationship data: deferred, explicitly ✓

Zanzibar's "new enemy" problem — Bob is removed from a group, a replica has not
caught up, a stale node authorizes him; solved there with zookies (consistency
tokens). M7 replication makes this real. **Recorded as a deliberate deferral so
it is a decision, not an oversight discovered in M7.**

### 9.6 SCP-style node ceilings: deferred, do not build ✓

"Nobody on this node may ever do X, regardless of what anyone grants" — the one
IAM layer with no equivalent here (§2). Conjunction (§5.4) *almost* gives it
free (a ceiling is just a caveat on the root grant), **except it only binds
chains that pass through that root**, and under §5.6.1 a service-owner-rooted
chain never passes through the substrate owner.

Weighed on benefit vs. effort and risk, it loses on all three:

- **Benefit — not yet.** B7 offers substrates to a marketplace *eventually*.
  Until then the substrate owner simply does not grant deploy rights to parties
  they distrust. A ceiling only matters under *partial* trust.
- **Effort — structural.** Every chain would have to root through the substrate
  owner.
- **Risk — philosophical, not mechanical.** Service owners would stop being
  independent roots and become the node's delegates. For a sovereignty-oriented
  system that deserves deliberation, not a side effect.

**Interim: B7's binary deploy grant** is a coarse ceiling — you deploy here or
you do not — and is sufficient until partial trust is a real scenario.

*Worth recording for whoever revisits this: the node operator already holds this
power physically (their hardware, their disk). The question is whether to make
implicit power explicit and bounded — which is an argument for eventually doing
it.*

### Resolved during review

- **Caveat `where` on non-data-layer interfaces** → **data-layer only for now**;
  the in-memory filter backend is the door left open (§6.6).
- **Do we need a relationship evaluator per data-source type?** → **No.** One
  SQL evaluator plus per-interface key extractors (§6.6).
- **May stage-4 ABAC look things up?** → **Yes**, fuel-metered and restrict-only
  (§6.7). The earlier prohibition rested on two arguments that do not hold.
- **Is the DID↔email binding mutual?** → **No** — one-way attestation. Mutual
  signing is a *consent* mechanism, not a truth mechanism (§5.6).
- **Fork B** (grants ∧ policy) → **decided** (§10).

---

## 10. The fork — decided

**Fork A — policy-only.** Drop the grant layer to pure transport authn; the FDAE
policy is the sole authority. Simpler, one place to look, no trace needed.
**Loses:** offline verification, attenuated delegation, cross-substrate grants,
and B7's revocable deploy grant — which then needs a bespoke mechanism anyway.

**Fork B — grants ∧ policy (this document).** Costs the decision trace and a
second concept for users to learn. **Buys:** delegation that works across trust
boundaries, revocation, and a single mechanism serving both B7's deploy grants
and M04B's row filtering.

**Decided: Fork B (2026-07-16), conditional on §7.2.** The condition is not a
footnote — Fork B without the trace degrades into the AWS IAM failure mode, and
at that point Fork A is the better system. If the trace is ever cut from scope,
this decision must be reopened, not quietly kept.

---

## 11. Milestone mapping

Nothing here moves a milestone boundary.

| Work | Where | Notes |
|---|---|---|
| Selectors, closed-form caveats, `anchor_did`/`path`, app abilities, resource-scoped `is_trusted_root`, chain-wide revocation | **ADR-0015 amendment** | **B7 is the first real consumer** — a revocable, non-re-delegatable, per-grantee `orchestrator/deploy` grant needs the selector *and* `can_delegate`, neither of which exists today. |
| Policy document, ReBAC→SQL compiler, RLS/CLS, operators, safety rails, stage-4 ABAC | **D-04-02 / M04B** | Unchanged in position. |
| Decision trace | **Both** | Grant-side denials in the amendment; policy-side in D-04-02. Same trace type. |

Convenient consequence: **B7 validates the grant layer before M04B commits to
it.** If the caveat/selector model cannot express a revocable deploy grant
cleanly, we learn it one milestone early.

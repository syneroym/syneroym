# M06D slice C8 — Dedicated mailbox: implementation plan

*Reader: the developer (or AI) who implements slice C8. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: ready, 2026-10-24. The owner accepted the recommendations for
Q1 to Q10 (section 2.1) and the field name `child_token` (section 2.2
point 7) on 2026-10-24. C7 merged as PR #321. Start on the current
branch `feat/m06d-c8-implementation`.**

**Size and PRs.** `task.md` sizes C8 as M. With deny and lease listing for
the mailbox (Q6), the deploy flags in two `roymctl` commands, the receiver
fallback and the end-to-end tests it is L. Ship it as two PRs, in this
order:

- **PR 1 — mailbox side** (Steps 2 to 7). The mailbox accepts a passport at
  `register`, raises quotas from the lease, and the operator can issue,
  deny and list. Nothing on the receiver or deployer side sends a passport
  yet, so nothing changes for existing users.
- **PR 2 — deployer and receiver side** (Steps 8 to 13, plus the close-out
  in Step 14). The owner's child token travels in the deploy call, the
  substrate stores it, and the puller presents it.

**Base.** C8 builds on C7, which merged as PR #321. File paths and
line numbers below were checked at `83a45870`.

## 0. Read first, and the rules

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish (background job, about 20 minutes),
  the import clean-up pass, the deferred backlog, the living-docs close-out,
  no planning references in code (no "M06D", "C8", "D-1", "D-C8-3" in code,
  comments or test names; ADR ids are fine), simple English in comments,
  800-line files, 100-line functions, short match arms, the module layout
  (`foo.rs` beside `foo/`, never `mod.rs`).
- [service-mailbox change doc](../../changes/service-mailbox/change.md):
  section "Dedicated mailbox and passport", criteria `D-1` to `D-3`, and the
  deviations `D-C4-1`, `D-C5-17`, `D-C6-8`, `D-C7-1`.
- [operator-passport change doc](../../changes/operator-passport/change.md):
  section "Core", and the deviations `D-B1-1` to `D-B1-28` and `D-B2-1`,
  `D-B2-6`, `D-B2-8`, `D-B2-16`, `D-B2-18`. The most important ones for C8:
  - `D-B1-15`: the DHT revocation check runs between `Registrar::check`
    and `Registrar::grant`. **Code review must check that it is there.**
  - `D-B1-16`: quotas come from `LeaseStore::lease_for`.
  - `D-B1-17` (as changed by `D-C4-1`): the instance certificate for
    `SignerBinding::InstanceCertificate` comes from
    `CallerContext.proof.delegation_json`, never from the request body. Do
    not add a "the transport already checked it" binding form.
  - `D-B2-16`: use `HostIdentity::passport_root()`; do not write a second
    root rule.
- [ADR-0015 A10](../../../decisions/0015-ucan-capability-model.md#a10-operator-passports-two-abilities-two-selectors-signed-use-limits)
  (ability `mailbox/use`, selector `substrate:<node did>/mailbox`, facts
  keys) and [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md).

### Terms used in this plan

- **Passport**: a `CapabilityToken` signed by the mailbox host's passport
  root (its verified controller, or its node DID when it has none), for
  ability `mailbox/use` on `substrate:<mailbox node did>/mailbox`, with the
  owner DID as audience. Its `facts` hold the quota limits.
- **Child token**: a `CapabilityToken` signed by the owner, audience the
  member master DID, with the passport as its one proof. This is what a
  receiver presents. It is the whole chain.
- **Lease**: a row in the mailbox's lease store
  (`<storage.db_dir>/mailbox_passports.db`), keyed on
  `(passport_id, subject)`, where subject is the member master DID. It
  holds the passport's facts.
- **Open limits**: the `[roles.mailbox]` queue limits (`max_queue_bytes`,
  `max_queue_messages`, `max_ttl_secs`).
- **Effective limits**: the limits a queue has now: the open limits, raised
  by the facts of a running lease for its master.

## 1. What C8 builds, in one paragraph

A mailbox host's owner issues a passport to a service owner with
`roymctl mailbox passport issue`. The service owner deploys with
`--mailbox-passport <file>`. `roymctl` signs one child token for each member
master whose service uses that mailbox, and sends it in the new
`mailbox-credentials.child-token` field. The substrate checks the token
offline, keeps it in the service vault under a reserved name, and the
mailbox puller sends it in `register`. The mailbox checks the chain with
`Registrar::check` (transport proof: the verified instance DID; binding: the
verified instance certificate), runs the DHT revocation check, and writes a
lease with `Registrar::grant`. From then on, every `register` answer and
every `deposit` uses the effective limits, read from
`LeaseStore::lease_for(master)`. When the lease ends or the passport is
denied, the queue falls back to the open limits at the next call. The
operator can deny a passport and list leases through the running node.

## 2. Points for the owner

### 2.1 Questions (accepted 2026-10-24: every recommendation)

**Q1. Sender time-to-live (backlog row "The deposit time-to-live is clamped
to the open mailbox limit", target C8).** The sender asks for
`min(OPEN_MAX_TTL_SECS, time the row may still live)`
(`crates/router/src/proxy/mailbox.rs:262`). It does not know whether the
receiver's queue allows more.

- **Recommended: no sender change in C8. Move the row to `TBD` with the
  reason below.** After an `expired` receipt the sender goes back to direct
  attempts and deposits again (S-5), so a message still waits up to the
  30-day outbox age limit, in 7-day pieces. A longer time-to-live helps
  only a sender that is itself offline when the 7 days end.

  **Note on end-to-end effect.** Accepting this recommendation means
  `max_ttl_secs` does not take effect end-to-end in C8. The sender
  (`crates/router/src/proxy/mailbox.rs:262`) clamps every deposit to 7 days.
  No deposit ever asks for a TTL above the open limit. So the
  `--max-ttl-days` flag, `DEDICATED_MAX_TTL_SECS`, and `C8-8` are built but
  do not change observable behavior until the sender is updated (see Q1
  backlog row in Step 14).

- Alternative: ask for `DEDICATED_MAX_TTL_SECS` and rely on the existing
  `ttl_too_long` → seal again path (`D-C5-15`). Cost: every deposit at an
  open mailbox (most of them) makes one refused call and one extra seal,
  and the refused call counts against the per-sender rate limit.

**Q2. The cap on the passport fact `max_ttl_secs`** (task.md asks for a cap
but gives no value).

- **Recommended: 30 days, a new constant `DEDICATED_MAX_TTL_SECS` in
  `crates/mailbox_format/src/lib.rs`.** It equals the queue lifetime, the
  mailbox `max_lease_secs` and `MAILBOX_SENDER_MAX_AGE_SECS`
  (`crates/core/src/mailbox.rs:31`). The receiver fence window is
  `max(max_ttl_secs, MAILBOX_SENDER_MAX_AGE_SECS) + 24 h`
  (`crates/router/src/call_dedup.rs:365`), so with this cap the fence does
  not grow at all. A longer time-to-live is useless: the sender gives up
  after 30 days.
- The cap is applied in three places: `roymctl mailbox passport issue`
  refuses more; the mailbox cuts the fact to it; the receiver cuts a learned
  value to it (Step 12).

**Q3. What "raise" means for each fact.**

- **Recommended: effective = `max(open limit, fact)` for `max_bytes`,
  `max_messages` and `max_ttl_secs` (after the Q2 cap).** A fact below the
  open limit is ignored. A passport then never makes a queue worse than
  having no passport. A missing key means the open limit (ADR-0015 A10).
- Alternative: the fact replaces the open limit. Simpler to explain, but an
  operator mistake then makes the passport holder worse off than a
  stranger.

**Q4. Does a passport queue count against `[roles.mailbox] max_queues`?**
Today `register` refuses a new queue with `mailbox_full` when the node holds
`max_queues` queues (`crates/mailbox/src/store/queue.rs:67`). Strangers can
fill an open mailbox's queue count, and the operator's own paying users
would then be refused.

- **Recommended: a register whose master has a running lease is not
  refused by `max_queues`.** It is still counted. The passport's
  `max_services` and the operator who issued it bound such queues.
  `max_total_bytes` still applies to every deposit.

  **Note on scope of the exemption.** The exemption covers only `max_queues`.
  These limits remain shared between open and passport queues:
  `max_total_bytes` (deposits still get `mailbox_full`), and the node-wide
  rate limits `deposits_per_minute_total`, `register_per_minute_total`, and
  `receiver_calls_per_minute_total` (at `crates/mailbox/src/service.rs:153`,
  `:186-191`, `:217`). The startup rule
  `receiver_calls_per_minute_total ≥ 2 × max_queues`
  (`crates/mailbox/src/limits.rs:100`) assumes at most `max_queues` queues;
  Q4 breaks that assumption when passport queues exceed `max_queues`. Also,
  `C8-1` gives the operator no config to run a mailbox that admits only
  passport holders. These gaps are deferred; see the new backlog rows in
  Step 14.

- Alternative: keep one limit for all queues.

**Q5. What the mailbox and the receiver do with a refused passport.**

- **Recommended:** the mailbox refuses the whole `register` with a new code
  `-32051` (`passport_refused`), with `data.code` set to the
  `RegisterError::code()` text (`expired`, `denied`, `limit_reached`, ...).
  It writes nothing. The puller then logs a note (once an hour), and in the
  same pass registers again **without** the child token, so the queue stays
  alive with the open limits (`D-1`: "without a passport it applies the
  open limits"). It tries the child token again at the next re-register
  (one hour) or when the stored token changes.
- Alternative: the mailbox ignores a bad passport and registers with open
  limits. Then the receiver cannot see why its quota is low.

**Q6. Deny and lease listing for the mailbox use.** `task.md` lists only
the `issue` variant. But P-6 (deny) and P-8 (`roymctl` deny and leases) are
core criteria that "each passport use must pass", and `D-B1-18` says deny
must go through the running service. Without them an operator cannot stop
a leaked passport before it expires.

- **Recommended: build them in C8 (PR 1).** Two new control-plane methods
  `security.mailbox-passport-deny` and `security.mailbox-passport-leases`,
  which reuse the relay functions, and `roymctl mailbox passport deny` and
  `leases`. Rename the WIT record `relay-lease` to `passport-lease`
  (pre-release, in place), because both uses return it.
- Alternative: a backlog row, and C8 stays smaller.

**Q7. Where the `roymctl` mailbox commands live.**

- **Recommended: a new top-level `roymctl mailbox passport issue|deny|leases`.**
  `coordinator` is the wrong noun for a mailbox. `issue` shares one code
  path with `coordinator passport issue` (Step 7).

**Q8. Where the substrate keeps the child token.**

- **Recommended: in the service vault, under a new reserved name
  `syneroym.mailbox-passport`, next to the mailbox key.** The puller reads
  it with a new `ReceiverVault::mailbox_passport` method (the C6 forward
  note asks for exactly this).
- Rejected: a field in the `HostedMailbox` fact. `EndpointRegistry::mailbox_of`
  clones the fact on every keyed call (the receiver fence,
  `crates/core/src/local_registry.rs:408`), and a child token is 1 to 2 KB.

**Q9. The supervisor path.** The supervisor mints member masters itself at
`submit` (`D-C7-11`), so the owner cannot sign child tokens for them in
advance. The supervisor's apply goes through `certify_placed_members` and
`map_planned_service` with no child token.

- **Recommended: out of scope for C8; a backlog row.** A deploy with no
  child token clears a stored one (a deploy states the whole wanted
  state). So a service that the supervisor deploys or adopts uses the open
  limits once its lease ends (at most 30 days later). `svc deploy` and
  `app deploy --mint-masters` support passports.
- Alternative: let the operator issue the passport to the supervisor's
  identity, and let the supervisor sign child tokens. This needs a
  supervisor config for passports and a new `submit` field.

**Q10. Revocation lookup time limit at the mailbox.** The relay uses 15 s
(`D-B2-18`). A mailbox `register` is one native call, and the puller waits
at most 10 s for it (`MAILBOX_CALL_TIMEOUT`,
`crates/router/src/proxy/mailbox.rs:33`). The mailbox already spends up to
5 s on the instance revocation check (`crates/mailbox/src/caller.rs:78`).

- **Recommended: 4 s for the passport chain lookup. On timeout, treat it as
  not revoked and log a warning**, as the relay does (`D-B2-8`).

### 2.2 Stale or unclear points in `task.md` (read them; no decision needed unless marked)

1. **`ucan_chain_not_revoked` lives in the router**
   (`crates/router/src/route_handler/io.rs:78`). `crates/mailbox` cannot
   depend on `crates/router`: the router already has `syneroym-mailbox` as
   a dev-dependency. Step 2 moves the body to `crates/core` and keeps the
   router function as a thin wrapper. B2's coordinator call keeps working.
2. **The queue row stores its own limits** (`max_bytes`, `max_messages`,
   `max_ttl_secs` columns, written once at insert,
   `crates/mailbox/src/store/queue.rs:70-90`). `task.md` says quotas come
   from `LeaseStore::lease_for`. Two sources would disagree after a deny or
   a lease end. Step 4 removes the columns. The product is not released
   (no migrations): an old `mailbox.db` fails at the first new queue and
   must be deleted, as with `D-C7-1`. Record this in the Deviations.
3. **"With the token in the deploy hash."** The child token is signed fresh
   at each deploy (`not_before_secs = now`, a new signature), so hashing
   its JSON would make every redeploy a change. Step 10 hashes only
   `(passport id, subject, expiry)`. Because `roymctl` gives the child
   token the passport's expiry, these fields are stable, and a renewed
   passport (a new id) is a change.
4. **`SignerBinding::InstanceCertificate`'s doc comment is stale**
   (`crates/passport/src/proof.rs:54-60`): it says the caller sends the
   certificate in its request. Since `D-C4-1` it comes from the verified
   caller proof. Fix the comment in Step 3.
5. **The relay's 15 s revocation limit does not fit the mailbox** (Q10).
6. **ADR-0015 A10 says "a missing key means the open limit applies"; `D-B1-6`
   says a missing count key means no limit.** No conflict for the mailbox:
   for `max_bytes`, `max_messages` and `max_ttl_secs` the open limit applies
   (Q3); for `max_services` an open mailbox has no per-owner limit, so no
   limit. `--max-services` stays optional, like `--max-nodes`. Say this in
   the `issue` help.
7. **Field names.** `task.md` names the WIT field `child-token` and the
   forward note from C4 names the RPC field `RegisterParams.passport`. Both
   carry the same JSON (the child token, with the passport inside). This
   plan uses `child_token` in both places, so one name runs along the whole
   path. The owner accepted `child_token` on 2026-10-24.
8. **The change doc's Design says "with a passport file"** but names no
   flag. This plan uses `--mailbox-passport <file>` on `svc deploy` and
   `app deploy`, and signs the child token with the `--as` identity, the
   same rule as `home-relay set` (`D-B3-13`).
9. **Lease end while the receiver sleeps.** The lease is renewed at each
   `register` (hourly while the receiver is awake) and ends at
   `min(passport expiry, child expiry, now + 30 days)`. If the receiver
   sleeps past the passport's expiry, the next deposit sees the open
   limits, and fair eviction may then remove several messages at once to
   get under them. This is the intended effect of a lease end (P-4). Write
   it in the Deviations and the backlog.
10. **`task.md` C8 says "Needs B1, C4 and C7".** True. It also needs the
    C6 puller (Step 12), which is merged.
11. **D-1's promise about raising quotas by `max_ttl_secs` is not
    observable until the sender is updated.** D-1 says the passport "raises
    the quotas by the facts." For `max_ttl_secs`, that is not true yet. The
    sender clamps every deposit to 7 days (Q1 accepted). So raising
    `max_ttl_secs` above 7 days has no visible effect until the sender is
    updated.

## 3. Decisions this plan makes (check them before you start)

- `C8-1`: the mailbox always accepts passports. There is no config switch.
  The only accepted root is `HostIdentity::passport_root()` of the mailbox
  host, so only its owner can raise a quota. A mailbox without passports
  in use behaves exactly as today.
- `C8-2`: lease file `<storage.db_dir>/mailbox_passports.db`, opened at
  mailbox start with `LeaseStore::open(.., &MAILBOX_USE, now)`.
- `C8-3`: the mailbox order in `register`: instance check, rate limits,
  instance revocation, parse, record check, then (only with a child token)
  passport `check`, chain revocation lookup, `grant`, then the store
  `register`. The network lookups come after every offline check. A lease
  is written even if the store `register` then fails (for example
  `stale_instance`); this is harmless, because the lease is keyed on the
  master, which a newer instance of the same master also uses.
  **Note**: when the record check refuses a `register` with `-32047`, no
  passport check runs. The lease is not renewed. The raised limits end
  within 30 days.
- `C8-4`: `grant` uses the same `now` as `check` (the mailbox dispatch
  passes one `now` to every handler, and tests use `dispatch_at`). The
  lookup is at most 4 s, so a lease may start up to 4 s old. The relay uses
  a fresh clock; this difference is on purpose, for testable code.
- `C8-5`: `MailboxStore` holds an `Option<LeaseStore>` and computes the
  effective limits itself (`queue_limits`). Only `MailboxStore::open` gets
  a new argument; the ~115 test calls of `store.register` and
  `store.deposit` do not change. `LeaseStore::lease_for` reads memory only.
  Lock order: the mailbox connection mutex, then the lease cache read
  lock. The lease store never takes the mailbox lock, so there is no cycle.
  **Note on `max_services` and queue removal**: a lease is not ended when
  its queue is removed (for example after an undeploy or a
  `release_mailbox_queue`). The `max_services` slot stays taken until the
  lease expires (up to 30 days). The owner cannot free the slot early; the
  operator can only deny the whole passport. The same master keeps the Q4
  exemption from `max_queues` while the lease runs.
- `C8-6`: the substrate checks the child token offline at deploy
  (`inspect_child_token`: shape, subject is the service id, signatures,
  expiry, and the resource host equals the mailbox the record names). A
  bad token refuses the deploy. The passport root is not checked there:
  only the mailbox knows its root.
- `C8-7`: `roymctl` gives the child token the passport's whole life
  (`expires_in_secs = passport.expires_at_secs - now`). A new passport
  arrives by an ordinary deploy (`D-C3-2`).
- `C8-8`: the receiver cuts a learned `max_ttl_secs` to
  `DEDICATED_MAX_TTL_SECS` when the service has a stored child token, and
  to `OPEN_MAX_TTL_SECS` when it has none. A mailbox that the owner chose
  without a passport cannot widen the receiver's age check.

## 4. Step-by-step changes

### Step 1 — Write the decisions down first (docs only)

In the [service-mailbox change doc](../../changes/service-mailbox/change.md)
`## Deviations`, add `D-C8-1` onwards: the answers to Q1 to Q10 and
`C8-1` to `C8-8`, and points 2, 3, 7, 8, 9 and 11 of section 2.2. Keep the
doc `status: approved` until Step 14.

### Step 2 — `crates/core` and `crates/mailbox_format` (PR 1)

**2a. Revocation helper.** New file `crates/core/src/dht_registry/revocation.rs`,
declared in `crates/core/src/dht_registry.rs` and re-exported there:

```rust
/// Whether no edge `(issuer, audience)` is revoked: no issuer's master
/// anchor lists its audience as a revoked key. A lookup that fails counts
/// as not revoked.
pub async fn chain_edges_not_revoked(
    edges: &[(&str, &str)],
    resolver: &dyn MasterAnchorResolver,
) -> bool
```

Body: the loop of today's `ucan_chain_not_revoked` (skip repeated edges
with a `HashSet`, return `false` on the first revoked edge). Then
`crates/router/src/route_handler/io.rs:78` becomes:

```rust
pub async fn ucan_chain_not_revoked(token: &CapabilityToken, resolver: &dyn MasterAnchorResolver) -> bool {
    dht_registry::chain_edges_not_revoked(&token.chain_edges(), resolver).await
}
```

Move its unit test (`crates/router/src/route_handler/io/tests.rs:404`) to `revocation/tests.rs`. Callers stay:
`crates/coordinator_iroh/src/passport.rs:112` and the router's own use.

**2b. Wire types** in `crates/core/src/mailbox.rs`:

```rust
/// Largest accepted child token, the same bound as a relay registration.
pub const MAX_CHILD_TOKEN_BYTES: usize = 16 * 1024;

pub struct RegisterParams {
    pub record: SignedEndpointInfo,
    /// The owner's child token for this member master (JSON of a capability
    /// token whose one proof is the mailbox passport). Raises the queue's
    /// limits by the passport's facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_token: Option<String>,
}

pub mod codes {
    // ... existing ...
    /// The child token of `register` was refused. `data.code` is the
    /// passport refusal code (`expired`, `denied`, `limit_reached`, ...).
    pub const PASSPORT_REFUSED: i32 = -32051;
}
```

Add `const _: () = assert!(DEDICATED_MAX_TTL_SECS <= MAILBOX_SENDER_MAX_AGE_SECS);`
next to `MAILBOX_SENDER_MAX_AGE_SECS`, with a one-line comment: the
receiver fence window must not grow with a passport.

Call sites of `RegisterParams { .. }`: only
`crates/router/src/mailbox_pull/service.rs:275` (Step 12) and the tests
that build it (`grep -rn 'RegisterParams {' crates`).

**2c.** `crates/mailbox_format/src/lib.rs`, after `OPEN_MAX_TTL_SECS`:

```rust
/// Longest time-to-live a passport may give a queue on a dedicated mailbox.
/// The same as the queue lifetime and the sender's age limit: a longer one
/// is never used.
pub const DEDICATED_MAX_TTL_SECS: u64 = 30 * 24 * 3600;
```

Update the `OPEN_MAX_TTL_SECS` doc comment to name the new constant.

### Step 3 — `crates/passport` (PR 1)

- `proof.rs:54-60`: rewrite the doc of `SignerBinding::InstanceCertificate`:
  the certificate comes from the verified caller proof
  (`CallerContext.proof.delegation_json`), not the request body.
- `lease_store.rs`: add

  ```rust
  impl Lease {
      /// The whole-number use limit `key` of the passport, if it has one.
      #[must_use]
      pub fn limit(&self, key: &str) -> Option<u64> {
          self.limits.get(key).and_then(Value::as_u64)
      }
  }
  ```

  A non-number fact reads as absent (the open limit). `issue_passport`
  already refuses one (`D-B1-7`).
- Tests: `limit` with a number, a missing key and a string value.

### Step 4 — `crates/mailbox`: passport admission and effective limits (PR 1)

**4a. `caller.rs`.** Split the certificate read out of `instance_of`:

```rust
/// The instance certificate the transport verified (ADR-0016): from the
/// caller proof, never from a request body.
pub(crate) fn instance_certificate(caller: &CallerContext) -> Result<DelegationCertificate, MailboxError>
```

It holds today's lines 27-40 (auth level, proof, `delegation_json`,
`from_json`). `instance_of` calls it first, then does the rest unchanged.

**4b. `error.rs`.** New variant:

```rust
#[error("the passport is refused ({code}): {reason}")]
PassportRefused { code: &'static str, reason: String },
```

`kind()` → `"passport_refused"`; `code_of` → `codes::PASSPORT_REFUSED`;
`From<MailboxError> for RpcError` adds `"code": code` to `data` (a match
arm next to `TtlTooLong`). Add a constructor:

```rust
impl From<RegisterError> for MailboxError {
    fn from(e: RegisterError) -> Self {
        match e {
            RegisterError::Store(msg) => Self::Internal(msg),
            other => Self::PassportRefused { code: other.code(), reason: other.to_string() },
        }
    }
}
```

`RegisterError` is not `Clone`, so the variant keeps the code and the text,
not the error.

**4c. New `src/passport.rs`** (+ `src/passport/tests.rs`):

```rust
pub(crate) const MAILBOX_LEASES_DB: &str = "mailbox_passports.db";
const REVOCATION_LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);

pub(crate) struct MailboxPassports {
    registrar: Arc<Registrar>,
    resolver: Arc<dyn MasterAnchorResolver>,
}

impl MailboxPassports {
    /// Opens the lease store in `db_dir` for `mailbox/use`, rooted at the
    /// host's passport root (ADR-0015 A10 point 4).
    pub(crate) fn open(db_dir: &Path, host: &HostIdentity, resolver: Arc<dyn MasterAnchorResolver>, now: u64) -> anyhow::Result<Self>;

    #[cfg(test)]
    pub(crate) fn in_memory(host: &HostIdentity, resolver: Arc<dyn MasterAnchorResolver>) -> anyhow::Result<Self>;

    pub(crate) fn leases(&self) -> &LeaseStore;

    /// Checks `child_token` for `who`, runs the revocation check on every
    /// chain edge, and writes the lease.
    pub(crate) async fn admit(&self, who: &Instance, cert: DelegationCertificate, child_token: &str, now: u64) -> Result<Lease, MailboxError>;
}
```

`open`: `fs::create_dir_all(db_dir)`, `LeaseStore::open(db_dir,
MAILBOX_LEASES_DB, &MAILBOX_USE, now)`, then
`Registrar::new(MAILBOX_USE, host.node_did.clone(), vec![host.passport_root().to_string()], leases)`.
Log the root once at `info!`, like the relay.

`admit`, pseudo-code:

```text
if child_token.len() > MAX_CHILD_TOKEN_BYTES:
    return Err(RegisterError::Malformed("child token over 16 KiB").into())
token: CapabilityToken = serde_json::from_str(child_token)
    else Err(RegisterError::Malformed(...).into())
proof   = Proof::Transport { verified_did: who.instance_did.clone() }
binding = SignerBinding::InstanceCertificate(cert)
checked = registrar.check(&token, &proof, &binding, now)?          // offline checks
// ADR-0015 §2 and A7, D-B1-15: a revoked edge stops the registration.
match timeout(REVOCATION_LOOKUP_TIMEOUT,
              chain_edges_not_revoked(&checked.token().chain_edges(), resolver)).await:
    Ok(false) -> return Err(RegisterError::Revoked.into())
    Ok(true)  -> ()
    Err(_)    -> warn!("passport revocation lookup timed out; treating as not revoked")
lease = spawn_blocking(move || registrar.grant(checked, now))     // deny, count, write
info!(passport_id, subject, lease_ends_at, "mailbox passport lease granted")
counter!("substrate.mailbox.passport_granted").increment(1)
Ok(lease)
```

A `RegisterError::Store` is logged at `error!` before it becomes
`Internal`, because the core does not log (`D-B1-24`).

**4d. `store.rs` and `store/queue.rs`, `store/deposit.rs`.**

- `MailboxStore` gets `leases: Option<LeaseStore>`.
  `MailboxStore::open(path, limits, leases: Option<LeaseStore>)`.
  `open_in_memory(limits)` keeps `None`; add a test-only
  `open_in_memory_with_leases(limits, leases)`.
- New methods:

  ```rust
  /// The limits of `service_id`'s queue now: the open limits, raised by
  /// the facts of a running lease for that master (D-B1-16).
  pub(crate) fn queue_limits(&self, service_id: &str, now: u64) -> QueueLimits {
      let open = self.limits.queue;
      let Some(lease) = self.lease_for(service_id, now) else { return open };
      let raise = |open: u64, fact: Option<u64>| fact.map_or(open, |f| f.max(open));
      QueueLimits {
          max_bytes: raise(open.max_bytes, lease.limit(facts::MAX_BYTES)),
          max_messages: u32::try_from(raise(u64::from(open.max_messages), lease.limit(facts::MAX_MESSAGES))).unwrap_or(u32::MAX),
          max_ttl_secs: raise(open.max_ttl_secs, lease.limit(facts::MAX_TTL_SECS).map(|t| t.min(DEDICATED_MAX_TTL_SECS))),
      }
  }

  fn lease_for(&self, service_id: &str, now: u64) -> Option<Lease> {
      self.leases.as_ref()?.lease_for(service_id, now)
  }
  ```

- Schema (`init_schema`): remove `max_bytes`, `max_messages`,
  `max_ttl_secs` from `queues`. `QueueRow` loses `limits`; `queue_row`'s
  `SELECT` drops the three columns.
- `queue.rs` `register`: the `INSERT` drops the three columns. The
  `max_queues` check becomes
  `if from_sql(count) >= max_queues && self.lease_for(&who.master_did, now).is_none()`
  (Q4). The answer's `limits` is `self.queue_limits(&who.master_did, now)`
  for both the new and the existing queue.
- `deposit.rs`: at the top of `deposit`,
  `let limits = self.queue_limits(&deposit.destination, now);`. Use
  `limits.max_ttl_secs` at line 39. `make_room` gets a `limits: QueueLimits`
  argument and uses it at lines 238-239 instead of `row.limits`.
- `MailboxService::info` keeps answering the open limits.

**4e. `service.rs`.**

- `MailboxService` gets `passports: MailboxPassports`.
- `open(role, db_dir, host: &HostIdentity, anchors)` (was `node_did: &str`).
  It sets `node_did = host.node_did.clone()`, opens `MailboxPassports`
  with `anchors.clone()`, and passes `Some(passports.leases().clone())` to
  `MailboxStore::open`.
- New `pub fn passport_leases(&self) -> LeaseStore` (a clone), for the
  control plane (Step 5).
- `register` (today lines 186-205), new order (`C8-3`):

  ```text
  who = instance_of(caller, now)?
  rate limits (unchanged)
  self.check_revocation(&who, now).await?
  p: RegisterParams = parse(params)?
  record = CheckedRecord::check(&p.record, &who, &self.node_did, now)?
  if let Some(token) = &p.child_token:
      cert = instance_certificate(caller)?
      if let Err(e) = self.passports.admit(&who, cert, token, now).await:
          counter!("substrate.mailbox.register_refused", "kind" => "passport_refused").increment(1)
          return Err(e)
  r = blocking(&self.store, move |s| s.register(&who, &record, now)).await?
  (publish wake and answer unchanged)
  ```

- `lib.rs`: `mod passport;`.
- Call sites of `MailboxService::open` (4): `crates/substrate/src/runtime/mailbox.rs:56`,
  `crates/mailbox/src/service/tests.rs`,
  `crates/router/src/proxy/tests/mailbox_helpers.rs`,
  `crates/router/src/mailbox_pull/tests/helpers.rs`.
  **Important — service test DIDs**: `crates/mailbox/src/service/tests.rs`
  currently uses `NODE_DID = "did:key:z6Mk…"`, which is not the project's
  DID form (`did:key:h…` from `substrate::derive_did_key`).
  `MailboxService::open` calls `Registrar::new` → `ensure_canonical_did`,
  which fails on that form. All 24 uses of `NODE_DID` in `service/tests.rs`
  must change: use `HostIdentity::random_unowned()` (from the `test-utils`
  feature of `syneroym-core`) and build test records with that DID. Add the
  `test-utils` feature of `syneroym-core` to `crates/mailbox`
  dev-dependencies. The router test helpers
  (`proxy/tests/mailbox_helpers.rs`, `mailbox_pull/tests/helpers.rs`)
  already use derived DIDs, so they are fine.
- `service.rs` is 343 lines; the new code is in `passport.rs`, so it stays
  well under 800.

### Step 5 — Substrate wiring (PR 1)

`crates/substrate/src/runtime/router.rs`:

- `setup_router(config, host: &HostIdentity, secret_key, relay_leases)`
  (was `service_id: &str`); inside, `let service_id = host.node_did.as_str();`.
  Call site: `setup_connection_router` line 115 passes `&keys.host`.
- Line 147: `init_mailbox(config, host, &shared, &endpoint_registry)`.
- After it: `if let (Some(cp), Some(parts)) = (&control_plane, &mailbox) { cp.set_mailbox_passports(parts.leases.clone()); }`
  (under `#[cfg(feature = "mailbox")]`).

`crates/substrate/src/runtime/mailbox.rs`:

- `init_mailbox(config, host: &HostIdentity, shared, endpoint_registry)`;
  use `host.node_did` where `node_did` was used; pass `host` to
  `MailboxService::open`.
- `MailboxParts { worker, leases: LeaseStore }`, filled with
  `service.passport_leases()`. The substrate already depends on
  `syneroym-passport` (relay leases).

### Step 6 — Control-plane deny and leases for the mailbox (PR 1, Q6)

- WIT `crates/wit_interfaces/wit/control-plane/control-plane.wit`: rename
  `record relay-lease` to `passport-lease` (doc: "One running lease of a
  passport service of this node: the relay or the mailbox."). Update
  `relay-passport-leases` to return `list<passport-lease>`. Add, next to
  the relay methods (line 662):

  ```wit
  /// Deny a mailbox passport by id: refuse it from now on and end its
  /// leases. Answers how many running leases ended. Node owner only.
  mailbox-passport-deny: func(passport-id: string) -> result<u64, string>;
  /// The running leases of this node's mailbox. Node owner only.
  mailbox-passport-leases: func() -> result<list<passport-lease>, string>;
  ```

  Update the `LeaseInfo` doc in `crates/passport/src/wire.rs:34` ("Mirrors
  the WIT record `passport-lease`").
- `crates/control_plane/src/service/dispatch/relay_passport.rs` → rename to
  `passport_admin.rs` (and its `tests.rs` dir). `deny(store, params, what: &'static str)`
  and `leases(store, what)`, where `what` is the text of the "no store"
  error: `NO_PASSPORT_RELAY` or a new
  `NO_PASSPORT_MAILBOX = "this substrate runs no mailbox"`. The log line
  says `"{what} passport denied"`-style text; keep the method name in the
  `InvalidParams` message.
- `crates/control_plane/src/service.rs:311`: add a `mailbox_passports:
  OnceLock<LeaseStore>` field and `pub fn set_mailbox_passports(&self,
  leases: LeaseStore)`, a copy of `set_relay_passports`.
- `dispatch.rs:146-149`: two new arms for `mailbox-passport-deny` and
  `mailbox-passport-leases`. They sit on the `security` interface, so they
  get the same `substrate/admin` gate as the relay methods; check that no
  separate allow-list in `dispatch.rs` names the relay methods by string
  (`grep -n 'relay-passport' crates/control_plane/src`).
- SDK `crates/sdk/src/client/orchestrator.rs:396-410`: add
  `mailbox_passport_deny(&self, &PassportId) -> Result<u64>` and
  `mailbox_passport_leases(&self) -> Result<Vec<LeaseInfo>>`, copies of the
  relay pair with the method names changed.

### Step 7 — `roymctl mailbox passport issue|deny|leases` (PR 1, Q7)

- New `apps/roymctl/src/commands/passport_issue.rs`: move `load_signer`
  and the body of `handle_issue` out of `commands/coordinator.rs` into

  ```rust
  /// The signer flags shared by every `passport issue` command.
  #[derive(Args, Debug, Clone)]
  #[command(group(ArgGroup::new("signer").required(true).args(["from", "node_key"])))]
  pub struct IssueSigner { from: Option<String>, node_key: bool, to: String, expires_days: u64, out: Option<PathBuf> }

  /// Signs a passport for `use_` with `limits`, writes it, prints its id.
  pub(crate) fn issue(signer: &IssueSigner, use_: &PassportUse, limits: Map<String, Value>, substrate_opt: Option<String>, dir: &Path) -> anyhow::Result<PassportId>
  ```

  `IssueArgs` in `coordinator.rs` becomes `{ #[command(flatten)] signer: IssueSigner, max_nodes: Option<u64> }`.
  The `cli_args.rs` and `coordinator/tests.rs` tests must still pass
  unchanged (flag names do not change).
- New `apps/roymctl/src/commands/mailbox.rs` (+ `mailbox/tests.rs`):

  ```rust
  pub enum MailboxCommands { Passport { #[command(subcommand)] command: MailboxPassportCommands } }
  pub enum MailboxPassportCommands {
      /// Issue a passport that raises the queue limits of one owner's
      /// services on this mailbox. Offline: it only signs.
      Issue(MailboxIssueArgs),
      /// Deny a passport on the running mailbox. ...
      Deny { passport_id: String },
      /// List the running leases of the mailbox.
      Leases,
  }
  pub struct MailboxIssueArgs {
      #[command(flatten)] signer: IssueSigner,
      /// Most distinct member masters that may hold a queue at one time. Omitted: no limit.
      #[arg(long)] max_services: Option<u64>,
      /// Bytes per queue. Omitted, or below the mailbox's own limit: the mailbox's limit.
      #[arg(long)] max_bytes: Option<u64>,
      /// Messages per queue. Same rule.
      #[arg(long)] max_messages: Option<u64>,
      /// Longest message time-to-live, in days, 1 to 30. Same rule.
      /// Accepts up to 30 days. Senders currently request at most 7 days,
      /// so values above 7 have no practical effect until senders ask for more.
      #[arg(long)] max_ttl_days: Option<u64>,
  }
  ```

  `Issue` refuses `--max-ttl-days` above `DEDICATED_MAX_TTL_SECS / 86400`
  and converts days to `facts::MAX_TTL_SECS` seconds. The help states the
  rules of Q3 and point 6 of section 2.2.
- `apps/roymctl/src/commands.rs`: add `Mailbox { #[command(subcommand)] command: mailbox::MailboxCommands }`
  next to `Coordinator` (line 106) and its dispatch arm (line 407).
- Tests: `mailbox/tests.rs` (issue writes a token with exactly the given
  facts and the `mailbox` selector; `--max-ttl-days 31` refused; no limit
  flags gives empty facts), `cli_args.rs` (parse of the three verbs).
- **Note on unowned mailbox host**: on a mailbox host with no verified
  controller and no `[iam].admin_ucan_root`, the node DID acts as passport
  root and can issue passports. However, `deny` and `leases` are refused on
  such an unowned host because `substrate/admin` fails closed (cost noted in
  `D-B2-6`). On a host with `admin_ucan_root` and no verified controller
  (`D-B2-1`), the node key issues passports and the admin root can deny or
  list them. The help text for `deny` and `leases` should explain this.

**PR 1 ends here.** Run Step 13's PR 1 tests and Step 15.

### Step 8 — WIT and SDK: the child token in the deploy call (PR 2)

- WIT `mailbox-credentials` (`control-plane.wit:181`): add

  ```wit
  /// The owner's child token for this member master (JSON of a capability
  /// token whose one proof is the mailbox passport). The receiver shows it
  /// at `register` to get the passport's raised limits.
  child-token: option<string>,
  ```

- Every builder of `MailboxCredentials` adds the field:
  - `crates/sdk/src/types.rs:94` (`Publication::split`): `child_token: mailbox_child_token`.
  - `crates/sdk/src/mapper.rs:390` (`map_planned_service`):
    `child_token: ctx.members.mailbox_child_tokens.get(&svc.service_id).cloned()`.
  - Test literals: `crates/substrate/tests/mailbox_record_e2e.rs`,
    `crates/control_plane/src/service/orchestration/tests/mailbox.rs`,
    `.../tests/mailbox_pull.rs` → `child_token: None`.
- `Publication::Public` gets `mailbox_child_token: Option<String>`
  ("Used only with `mailbox_key`"). Every `Publication::Public { .. }`
  literal adds it (`grep -rn 'Publication::Public {' crates apps`).
- `CertifiedMembers` (`crates/sdk/src/deploy/types.rs:22`) gets
  `pub mailbox_child_tokens: BTreeMap<ServiceId, String>` ("Only members
  whose mailbox host has a passport"). `without_mailboxes` sets it empty.
  `certify_placed_members` does not fill it (the supervisor never has
  one, Q9).
- New `crates/sdk/src/deploy/mailbox_passport.rs` (+ `tests/mailbox_passport.rs`):

  ```rust
  /// A mailbox passport read from a file, checked for one owner.
  pub struct MailboxPassport { token: CapabilityToken, host: String }

  impl MailboxPassport {
      /// Parses `json` and checks: it is a passport (no proofs, one
      /// capability), for `mailbox/use` on `substrate:<did>/mailbox`, issued
      /// to `owner_did`, and not expired at `now`.
      pub fn parse(json: &str, owner_did: &str, now: u64) -> Result<Self>;
      /// The mailbox node DID the passport names.
      pub fn host(&self) -> &str;
      /// The owner's child token for `master`, for the passport's whole life.
      pub fn child_token_for(&self, owner: &Identity, master: &str, now: u64) -> Result<String>;
  }

  /// One child token for each service of `plan` whose pinned mailbox has a
  /// passport. Every passport must match at least one service, and no two
  /// passports may name the same mailbox.
  pub fn child_tokens_for(plan: &DeploymentPlan, owner: &Identity, passports: &[MailboxPassport], now: u64) -> Result<BTreeMap<ServiceId, String>>;
  ```

  `child_token_for`, pseudo-code:

  ```text
  token = syneroym_passport::child_token(owner, &self.token, master,
                                         self.token.expires_at_secs - now, now)?
  inspect_child_token(&token, &MAILBOX_USE, master, now)?      // fail here, not at the node
  Ok(serde_json::to_string(&token)?)
  ```

  `child_tokens_for`, pseudo-code:

  ```text
  check: no two passports have the same host, else bail naming the host
  out = {}
  used = set()
  for svc in plan.services:
      match svc.mailbox:
          None -> continue
          Some(Auto) -> bail "pin `auto` first"         // same rule as certify
          Some(Service(host)) ->
              if let Some(p) = passports.find(host == p.host):
                  out[svc.service_id] = p.child_token_for(owner, svc.service_id, now)?
                  used.insert(p.host)
  for p in passports: if p.host not in used: bail "--mailbox-passport for {host} matches no service that uses that mailbox"
  Ok(out)
  ```

  `crates/sdk` already depends on `syneroym-passport`.

### Step 9 — `roymctl svc deploy` and `app deploy` flags (PR 2)

Keep the new code in a new `apps/roymctl/src/commands/mailbox_passport.rs`:
`svc/deploy.rs` is 730 lines and `app/deploy.rs` 749.

```rust
/// Reads each `--mailbox-passport` file and checks it for the `--as`
/// identity, which signs the child tokens and must be the passport's
/// audience.
pub(crate) fn load_passports(dir: &Path, files: &[PathBuf], run_as: Option<&str>, now: u64)
    -> anyhow::Result<Option<(Identity, Vec<MailboxPassport>)>>
```

It returns `None` when `files` is empty. With files and no `--as`, it
fails: "--mailbox-passport needs --as <owner>: the owner signs the child
token". It loads `<dir>/identities/<name>.key` the way `client_for` does
(`apps/roymctl/src/commands.rs:188`); move that path rule into one small
helper both use.

**svc deploy** (`apps/roymctl/src/commands/svc.rs`):

- New flag after `mailbox_methods` (line 170):
  `/// The mailbox host's passport for this service's owner. The child token is signed with --as. Needs --mailbox.`
  `#[arg(long = "mailbox-passport", value_name = "FILE")] mailbox_passport: Option<PathBuf>`.
- `check_mailbox_args` (`svc/deploy.rs:93`) gets `has_passport: bool`:
  a passport without `--mailbox` is refused.
- After the `auto` pin, before `build_publication`:
  `load_passports`, then require `passport.host() == pinned mailbox`
  ("the passport is for mailbox X, but --mailbox is Y"), then
  `child_token_for(owner, svc_id, now)`.
- `build_publication` gets `mailbox_child_token: Option<String>` and puts it
  into `Publication::Public`. Its other call sites pass `None`.

**app deploy** (`apps/roymctl/src/commands/app.rs` and `app/deploy.rs`):

- New repeatable flag next to `--mailbox` (line 174):
  `#[arg(long = "mailbox-passport", value_name = "FILE", requires = "mint_masters")] mailbox_passports: Vec<PathBuf>`.
- Thread `Option<(Identity, Vec<MailboxPassport>)>` into
  `member_identity::substitute_and_certify_members` (new last argument
  `passports: Option<&(Identity, Vec<MailboxPassport>)>`). After
  `certify_placed_members` (line 203):

  ```rust
  if let Some((owner, passports)) = passports {
      members.mailbox_child_tokens = deploy::child_tokens_for(&new_plan, owner, passports, now)?;
  }
  ```

  Call chain: `build_deploy_plan` (`app/deploy.rs:488`) calls
  `substitute_and_certify_members` (line 498), `handle_deploy`
  (`app/deploy.rs:696`) calls `build_deploy_plan`, and `handle_app` in
  `apps/roymctl/src/commands/app.rs:282` passes the loaded passports from
  `--mailbox-passport`. Other callers pass `None`.
- `app reconcile` deploys nothing, so it gets no flag.

### Step 10 — Control plane: check, store and hash the child token (PR 2)

- `crates/data_db/src/vault_names.rs`:
  `pub const MAILBOX_PASSPORT: &str = "syneroym.mailbox-passport";` with a
  doc line and an `is_reserved` test.
- `crates/control_plane/src/service/orchestration/types.rs`:
  - `ValidatedMailbox` gets `child_token: Option<String>`. Its `Debug`
    shows `"<present>"` or `None`, never the text.
  - In `validate_mailbox` (line 350), in the `(Some(credentials), Some(mailbox))`
    arm after the methods check:

    ```rust
    let child_token = credentials.child_token.as_deref()
        .map(|t| validate_child_token(service_id, &mailbox.service_id, t))
        .transpose()?;
    ```

  - New function (same file, or a new sibling `orchestration/child_token.rs`
    if `types.rs` passes ~600 lines):

    ```text
    fn validate_child_token(service_id, mailbox_host, json) -> Result<String, String>:
        if json.len() > MAX_CHILD_TOKEN_BYTES: Err("child token of '{service_id}' is over 16 KiB")
        token: CapabilityToken = from_str(json) else Err("child token of '{service_id}' does not parse")
        presented = inspect_child_token(&token, &MAILBOX_USE, service_id, now_secs())
            else Err("child token of '{service_id}' is refused: {code}")
        if presented.host_node_did != mailbox_host:
            Err("child token of '{service_id}' is for mailbox {host}, but its record names {mailbox_host}")
        Ok(json.to_string())
    ```

- `deploy/manifest.rs:28-36` (`compute_deploy_idempotency_hash`):
  replace `mailbox_methods` with

  ```rust
  let mailbox_for_hash = manifest.mailbox.as_ref()
      .map(|m| (&m.methods, m.child_token.as_deref().map(stable_child_token_for_hash)));
  ```

  and add, next to `stable_registry_certificate_for_hash`:

  ```text
  /// The passport id, subject and expiry of a child token: the token is
  /// signed again at every deploy, so its JSON always differs.
  fn stable_child_token_for_hash(json) -> String:
      match from_str::<CapabilityToken>(json):
          Ok(t) with exactly one proof -> to_string(&(hex(t.proofs[0].token_id()), &t.audience_did, t.expires_at_secs))
          _ -> json.to_string()
  ```

  The hash shape changes for every service with a mailbox, so the first
  redeploy after the upgrade is not skipped. Pre-release; no shim.
- `mailbox_key.rs`:
  - `store_mailbox_passport(&self, service_id, child_token: Option<&str>) -> anyhow::Result<()>`:
    `write_secret(MAILBOX_PASSPORT, bytes)` or `delete_secret(MAILBOX_PASSPORT)`.
  - `clear_mailbox_key` also deletes `MAILBOX_PASSPORT` in the same block.
  - `pub async fn mailbox_passport(&self, service_id) -> anyhow::Result<Option<String>>`,
    like `mailbox_key` (UTF-8 check on the bytes).
- `deploy/commit.rs` `install_mailbox_key` (line 49): after
  `store_mailbox_key`, call `store_mailbox_passport(service_id,
  mailbox.child_token.as_deref())` with the same error text.

### Step 11 — `ReceiverVault` (PR 2)

`crates/router/src/mailbox_pull.rs:57`:

```rust
/// The owner's child token for this service's mailbox, or `None`.
async fn mailbox_passport(&self, service_id: &str) -> anyhow::Result<Option<String>>;
```

Implement it for `ControlPlaneService` (line 236) and for the test double
in `crates/router/src/mailbox_pull/tests/helpers.rs`.

### Step 12 — The puller sends the child token and falls back (PR 2)

`crates/router/src/mailbox_pull/service.rs` (539 lines; put new code in a
new sibling `mailbox_pull/passport.rs` if it grows past ~650):

- `Inputs` gets `child_token: Option<String>`. In `inputs()` (line 230),
  after the key: `vault.mailbox_passport(id)`; `Err` → `Note::KeyUnreadable`,
  and `inputs()` returns `None` (stop the pass, the same way the key path
  does; do not proceed without the token on a vault error).
- `Registration` gets `child_token_digest: Option<[u8; 32]>` (SHA-256 of
  the JSON; `sha2` is already a router dependency). It holds the digest of
  the token the service *had* at that register, even when the mailbox
  refused it, so a refused token is not sent on every pass.
  `needs_register` also returns `true` when the digest differs from the
  current input's.
- Split `register` (line 274): move the gate block (lines 279-302) into

  ```rust
  async fn send_register(&self, pass: &mut ServicePass<'_>, inputs: &Inputs,
                         child_token: Option<String>) -> Option<Result<Value, ProxyError>>
  ```

  (`None` = the fact changed during the pass; the caller returns `false`
  as today). Then:

  ```text
  answer = send_register(pass, inputs, inputs.child_token.clone())?
  if answer is Err(e) with code PASSPORT_REFUSED and inputs.child_token.is_some():
      pass.note(Note::PassportRefused, data.code of e)
      answer = send_register(pass, inputs, None)?
  match answer (unchanged arms)
  ```

  Read `data.code` from `ProxyError::Callee { data, .. }`.
- `learn` (line 341), `C8-8`:

  ```rust
  let cap = if inputs.child_token.is_some() { DEDICATED_MAX_TTL_SECS } else { OPEN_MAX_TTL_SECS };
  let max_ttl_secs = limits.max_ttl_secs.min(cap);
  ```

  Update its doc comment.
- `notes.rs`: `Note::PassportRefused`, reason `"passport_refused"`, text
  "the mailbox refused the owner's passport; the queue has the open limits".
- Sender (`crates/router/src/proxy/mailbox.rs:262`): no change (Q1).

### Step 13 — Tests

PR 1:

- `crates/core/src/dht_registry/revocation/tests.rs`: revoked first edge,
  revoked second edge, a repeated edge looked up once, a failed lookup
  counts as not revoked.
- `crates/passport`: `Lease::limit` (Step 3).
- `crates/mailbox/src/passport/tests.rs` (`D-1` to `D-3`, P-2 and P-3 for
  the mailbox use, both host forms):
  - a valid child token grants a lease whose subject is the master;
  - a child token for another master is `not_bound`;
  - a certificate for another instance key is `not_bound` (a leaked token
    alone is not enough, P-3);
  - a passport from a root that is not `passport_root()` is
    `untrusted_root` (host with a verified controller: a node-DID passport
    is refused, `D-B2-1`);
  - an expired passport or child token is `expired`;
  - a passport for `relay/home` is `wrong_capability`;
  - `max_services = 1`: a second master is `limit_reached`; the same master
    renews;
  - a revoked `(owner, master)` edge is `revoked`; a lookup that times out
    (a resolver that sleeps past 4 s, paused clock) still grants;
  - a denied passport is `denied`;
  - a token over 16 KiB and bad JSON are `malformed`.
- `crates/mailbox/src/service/tests.rs`:
  - register with a child token answers the raised limits; without one,
    the open limits;
  - a refused child token answers `-32051` with `data.code`, and writes no
    queue;
  - a deposit above the open message limit is stored under a lease and
    refused (fair eviction) after `deny`;
  - `max_queues` reached by open queues still admits a master with a
    lease (Q4), and refuses one without;
  - a fact above `DEDICATED_MAX_TTL_SECS` is cut; a fact below the open
    limit is ignored (Q3);
  - a lease survives `MailboxService::open` again on the same directory
    (P-5);
  - after the lease expires (advance `dispatch_at` past `lease_ends_at`),
    the next `register` and `deposit` see the open limits (P-4).
- P-8 running-node test: in `crates/substrate/tests/mailbox_passport_e2e.rs`
  (alongside the deny test, since that test already boots a node), add a
  test that lists leases, checks one running lease, then calls deny and
  confirms leases is empty.
- `crates/mailbox/src/store/tests/queue.rs` and `deposit.rs`: the
  effective-limit cases with `open_in_memory_with_leases`.
- `crates/mailbox/src/error/tests.rs`: `PassportRefused` code and data;
  `Store` maps to `-32603`.
- `crates/control_plane/src/service/dispatch/passport_admin/tests.rs`: the
  mailbox pair, including "no mailbox" → `MethodNotFound`.
- `apps/roymctl`: Step 7's tests; `coordinator/tests.rs` unchanged and
  green.

PR 2:

- `crates/sdk/src/deploy/tests/mailbox_passport.rs`: `parse` refusals
  (another owner, `relay/home`, expired, a child token instead of a
  passport); `child_tokens_for` (one token per master of the matching
  host; a passport that matches nothing; two passports for one host;
  `auto` not pinned); the token passes `inspect_child_token`.
- `crates/sdk` mapper and `Publication::split` tests: the field travels.
- `crates/control_plane/.../tests/mailbox.rs`: a deploy with a good child
  token stores it; a token for another master, another mailbox, or an
  expired one is refused with an error that does not contain the token; a
  redeploy without a token clears it; undeploy clears it; the deploy hash
  is equal for two child tokens of one passport and differs for a new
  passport.
- `crates/router/src/mailbox_pull/tests/register.rs`: the token is sent;
  a `-32051` answer leads to a second register without it and one note;
  the next pass does not send it again until an hour passes or the token
  changes; `learn` cuts to 30 days with a token and 7 days without.
- `apps/roymctl`: `cli_args.rs` for both flags; `svc/tests.rs`
  (`--mailbox-passport` without `--mailbox`, without `--as`, host
  mismatch); `app/tests/settings.rs` (needs `--mint-masters`).
- End to end, new `crates/substrate/tests/mailbox_passport_e2e.rs`, built
  on `tests/common/mailbox_world.rs` with `alloc_ports` (no port literals):

  **Note on e2e helper changes needed before these tests can work:**
  - `crates/substrate/tests/common/mailbox_receiver.rs:124` builds
    `Publication::Public` and must be updated to take a child token (the
    `Publication::Public` struct gains `mailbox_child_token`).
  - The current test world has one receiver. The D-3 test needs a second
    receiver (two masters under the same passport).
  - The deny test (test 3) needs an owned mailbox node (`substrate/admin`)
    and that node's controller must sign the passport (`D-B2-1`).

  1. `a_passport_raises_the_queue_limit` (`D-1`, `D-2`): mailbox role with
     `max_queue_messages = 2`; passport `max_messages = 5`; `svc deploy`
     with `--mailbox-passport`; stop the receiver; four queued calls are
     all deposited (none goes back to direct attempts with `queue_full`);
     start the receiver; all four run once.
  2. `max_services_admits_one_master` (`D-3`): two services under one
     passport with `max_services = 1`; the second registers with the open
     limits (its pull summary shows it registered; its third deposit is
     refused as full).
  3. `a_denied_passport_falls_back_to_the_open_limits` (P-6, P-8):
     `roymctl mailbox passport deny` through the node owner; the next
     deposit beyond the open limit is refused.
  4. `a_lease_survives_a_mailbox_restart` (P-5): restart the mailbox node
     (`restart_in_place`); the raised limit still holds without a new
     register.

### Step 14 — Docs, backlog, status (PR 2, plus the PR 1 parts noted)

- Mailbox change doc: `D-1` to `D-3` test columns (in the `D-1` column,
  note that `max_ttl_secs` has no end-to-end effect yet because the sender
  clamps deposits to 7 days); tick the last task
  **and** the "Resolve the open questions" task; update the two
  partly-decided questions (the first time-to-live, size and rate values;
  who wakes a sleeping phone) with the Q1 answer and a forward reference to
  the still-open items; the rest of `D-C8-*` (what differed from this
  plan); `status: done`; `living-docs-touched` updated. Also add a
  Deviations entry: on an unowned mailbox host (no verified controller and
  no `[iam].admin_ucan_root`), the node key issues passports but deny and
  leases are refused (`substrate/admin` fails closed, `D-B2-6`).
- Operator-passport change doc: no status change. Add one line under
  "Deviations" pointing to `D-C8-*` for the mailbox use (it is `done`, so
  only if `cargo xtask check-change-docs` allows it; else leave it).
- Living docs:
  - `docs/system-requirements-spec.md:1084`: dedicated mailboxes are built.
  - `docs/system-architecture.md`: the mailbox section (passport at
    `register`, effective limits, lease file), the passport section (the
    second use), the `security` method list. Also add the `-32051
    passport_refused` row to the mailbox error-code table (lines 2124–2136).
  - `docs/developer-guide.md`: an operator note: issue, deny, list; the
    owner's `--mailbox-passport`; the file `mailbox_passports.db`; delete
    an old `mailbox.db`. In the metrics section (around line 1801):
    - add `passport_refused` to the `substrate.mailbox.register_refused`
      kinds list: `kind` (`rate_limited`, `passport_refused`).
    - add new counter `substrate.mailbox.passport_granted`: Counter of
      granted mailbox passport leases.
  - `docs/TERMINOLOGY.md`: "passport", "child token", "lease" now name
    both uses.
  - `docs/planning/traceability-matrix.md:56`: add D-1 to D-3.
- `docs/planning/deferred-backlog.md`:
  - "The receiver cuts a learned queue lifetime to the open mailbox limit"
    → "Recently resolved".
  - "The deposit time-to-live is clamped to the open mailbox limit" →
    target `TBD`, with the Q1 reason. Also note: `C8-8` can refuse
    messages the mailbox held correctly once the sender later asks for more
    than 7 days (a lease may still be running when the sender finally asks
    for more than 7 days).
  - New rows: the supervisor path (Q9); the passport revocation lookup
    passes on timeout (with `D-B2-8`); a lease end may evict several
    messages at the next deposit (section 2.2 point 9); a revocation does
    not end a running mailbox lease (extend the existing row "A revocation
    does not end a running lease"); a new passport needs a redeploy (no
    renewal call, `D-C3-2`).
  - Additional new rows from this review:
    - An undeploy or queue move does not end the passport lease. The
      `max_services` slot stays taken for up to 30 days. The owner cannot
      free it early. Target: TBD.
    - The node-wide rate limits (`deposits_per_minute_total`,
      `register_per_minute_total`, `receiver_calls_per_minute_total`) and
      `max_total_bytes` are shared between open and passport queues.
      Strangers can still block paying users through these paths.
      Target: TBD.
    - The startup validation `receiver_calls_per_minute_total ≥ 2 ×
      max_queues` assumes at most `max_queues` queues; Q4 breaks this when
      passport queues exceed `max_queues`. Target: TBD.
    - `C8-1` gives no config to admit only passport holders. Target: TBD.
  - Each new row must link the service-mailbox change doc.
- `task.md`: C8 "Done", with the PR numbers; `status.md`: the C8 row with
  evidence, in the same style as C7.

### Step 15 — Finish (each PR)

- Import clean-up pass on every edited file.
- `cargo dupes report` over the changed files (recipe in `AGENTS.md`);
  `passport_admin.rs` and `passport_issue.rs` exist to remove duplication,
  not add it.
- Ratchets: lower any baseline that improves.
- `mise run verify` in the background, no `--skip`; all gates green.
- PR 2 changes WIT: rebuild `mise run build:test-components` and
  `mise run build:roym` before the parity and e2e gates.
- Do not commit on `main`.

## 5. Call sites that change (one list)

| Change | Call sites |
| --- | --- |
| `ucan_chain_not_revoked` body moves to `core` | `crates/router/src/route_handler/io.rs:78` (wrapper stays); users unchanged: `crates/coordinator_iroh/src/passport.rs:112`, router; `ucan_chain_not_revoked` wrapper test at `crates/router/src/route_handler/io/tests.rs:404` |
| `RegisterParams.child_token` | `crates/router/src/mailbox_pull/service.rs:275`; tests that build it |
| `MailboxService::open(.., host, ..)` | `crates/substrate/src/runtime/mailbox.rs:56`, `crates/mailbox/src/service/tests.rs`, `crates/router/src/proxy/tests/mailbox_helpers.rs`, `crates/router/src/mailbox_pull/tests/helpers.rs` |
| `MailboxStore::open(.., leases)` (store tests) | `crates/mailbox/src/store/tests/settled_receipts.rs:194,206`, `crates/mailbox/src/store/tests/plaintext.rs:23`, `crates/mailbox/src/store/tests/settled_paths.rs:58,66` |
| `MailboxStore::open(.., leases)` | `crates/mailbox/src/service.rs` (`open`); `open_in_memory` unchanged |
| `QueueRow.limits` removed | `store.rs` (`queue_row`), `store/queue.rs` (`register`), `store/deposit.rs:39`, `:238-239` |
| `init_mailbox(.., host, ..)`, `setup_router(.., host, ..)` | `crates/substrate/src/runtime/router.rs:115`, `:147` |
| `MailboxParts.leases` | `crates/substrate/src/runtime/router.rs` (new `set_mailbox_passports` call) |
| WIT `relay-lease` → `passport-lease` | generated bindings only; `LeaseInfo` doc |
| `relay_passport.rs` → `passport_admin.rs` | `crates/control_plane/src/service/dispatch.rs:6`, `:146-149` |
| `IssueSigner` extracted | `apps/roymctl/src/commands/coordinator.rs` (`IssueArgs`, `handle_issue`, `load_signer`) |
| WIT `mailbox-credentials.child-token` | `crates/sdk/src/types.rs:94`, `crates/sdk/src/mapper.rs:390`; test literals in `crates/substrate/tests/mailbox_record_e2e.rs`, `crates/control_plane/.../tests/mailbox.rs`, `.../tests/mailbox_pull.rs` |
| `Publication::Public.mailbox_child_token` | `apps/roymctl/src/commands/svc/deploy.rs:226`; every test literal |
| `CertifiedMembers.mailbox_child_tokens` | `crates/sdk/src/deploy/types.rs` (`without_mailboxes`), `crates/sdk/src/deploy/certify.rs` (default), `apps/roymctl/src/commands/member_identity.rs:203`; `crates/substrate/tests/roym_identity_e2e.rs:215`, `crates/substrate/tests/roym_app_e2e.rs:252`, `crates/substrate/tests/common/roym.rs:407` (full `CertifiedMembers { … }` literals) |
| `substitute_and_certify_members(.., passports)` | `apps/roymctl/src/commands/app/deploy.rs:488` (`build_deploy_plan`), line 696 (`handle_deploy`), and `apps/roymctl/src/commands/app.rs:282` (caller of `handle_deploy`) |
| `ValidatedMailbox.child_token` | `types.rs` (`validate_mailbox`), `deploy/commit.rs` (`install_mailbox_key`) |
| `compute_deploy_idempotency_hash` | `crates/control_plane/src/service/orchestration/deploy/manifest.rs:28-36` |
| `ReceiverVault::mailbox_passport` | `crates/router/src/mailbox_pull.rs:236` (impl), `mailbox_pull/tests/helpers.rs` |
| `learn` cap | `crates/router/src/mailbox_pull/service.rs:342` |

## 6. What C8 does not do

- No sender change (Q1, backlog row).
- No supervisor support for passports (Q9, backlog row).
- No passport renewal without a redeploy (`D-C3-2`).
- No undeny (existing backlog row, `D-B1-9`).
- No passport listing in the registry: a dedicated mailbox is never in the
  open list.
- No change to the challenge form, P-7, P-9 or P-10: they apply to the
  relay only.

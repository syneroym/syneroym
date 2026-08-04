# Slice A7 Implementation Plan — App-Instance Master Identity

**Status:** 📋 Planned (2026-08-04). Not started. Milestone:
[task.md](task.md) slice **A7**, which is also slice **S0** of the
[Logical Service Discovery Overlay](../../meta-implementation-plan.md#committed-work-logical-service-discovery-overlay-2026-08-02).
Design of record:
[ADR-0022](../../../decisions/0022-two-tier-logical-service-discovery.md) §1,
plus the overlay's own S0 row. Depends on **A5b — Complete**; needs nothing
from A5c/A5d/A5e. Gates overlay slice **S1** (the Tier-1 registry record).

**A5e is Complete (2026-08-04,
[status.md](status.md) line 27), so this slice is the one that closes the
milestone.** `task.md`'s exit criteria and `status.md`'s overall line both
already say the milestone closes when A5e and A7 have both landed
(§41 answer 1 of the A5e plan); one of the two is now done. See §5.

**The one-sentence summary.** `adopt` mints one more master key — the app
instance's own — into the same supervisor vault that already holds every
member master, records its DID on the instance row, reports it on `status`,
and lets `export-master` / `import-master` move it by name, so an app instance
has a stable network identity that does not change when the supervisor
underneath it changes.

**What this slice is not.** No registry publication, no topology document, no
`resolve` RPC, no gateway hostname change, no sharding surface. Those are
overlay slices S1–S3 and sit outside this milestone. Concretely, this slice
adds **no** call to any registry writer, **no** new WIT verb, and **no**
signing operation with the new key — it only mints it, stores it, names it,
and reports it. The one place where a later slice's need shapes a decision
here is §0.5's ordering rule, and that is recorded as a documented constraint
plus a backlog row, not as built machinery.

**Review pass (2026-08-04), all nine findings incorporated — see §7.** None
changed a decision; four were places where a decision this plan makes had no
test that would fail if it were violated, and the most consequential of those
is that the handover — this slice's central new risk — was only exercised
inside **one** vault, never across two, which is the property overlay slice S1
will depend on (test 100). Two more were operator-facing prose in files this
slice already opens, including a second live copy of the pre-A5e vault-name
that §0.13 caught in the ADR and missed in shipped WIT (D-A7-14). Two were
stated costs that were simply wrong (`get_or_mint` has 12 call sites, not 14;
§0.3's guard argument was narrower than the guard it describes).

**Read §0 first.** Reading the shipped code found **thirteen** places where
ADR-0022 §1 and `task.md`'s A7 section leave a decision unmade, understate the
work, or describe the tree as it was before A5e. **Three** of them change what
A7 has to build, and **two are blocking**: the instance row cannot gain a
column the way `SupervisorStore` creates its schema today (§0.2), and
`import-master` can silently leave the row and the vault disagreeing about
which DID is the app's (§0.5). §1's decisions (D-A7-1 … D-A7-14) take §0's
recommended resolutions as given; the three that are genuinely the
requester's are listed again in §6.

Same discipline as A0 §6 / A1 §6 / A2 §0 / A3 §0 / A4 §0 / P0 §0 / A5 §0 /
A5c §19 / A5d §26 / A5e §33.

---

## §0 — What ADR-0022 §1, `task.md`, and the shipped code leave open, understate, or state wrongly

`task.md`'s A7 section
([lines 609-644](task.md)) is thirty-six lines, and ADR-0022 §1 is thirty-five.
Between them they settle four things: the app instance gets a master DID, it
is minted at `adopt`, it lives in the supervisor's vault beside the member
masters, and `export-master` / `import-master` move it. Everything below is
what those two documents do not say.

### 0.1 (Scope-changing) `adopt` has never opened the vault, and `kek_is_loaded()` is the wrong gate for the one that A7 adds

`handle_adopt`
([service.rs:2635-2684](../../../../crates/app_supervisor/src/service.rs#L2635))
touches exactly three things today: the store, the placed substrates, and the
per-instance lock. It never reads `self.vault`. A7 makes it a vault caller,
which changes its failure surface: the vault is **locked after every
supervisor restart**, because the KEK arrives by `security.inject-kek` and does
not survive one
([runtime.rs:726-731](../../../../crates/substrate/src/runtime.rs#L726),
[keys.rs:36-44](../../../../crates/app_supervisor/src/keys.rs#L36)). So after
A7, an `adopt` issued before the operator injects a KEK cannot mint.

The obvious move is to copy A5d's cheap pre-check —
`if !self.vault.kek_is_loaded()`
([service.rs:1026](../../../../crates/app_supervisor/src/service.rs#L1026)) —
and it is wrong here. `kek_is_loaded()` reads the `KeyStore`, not the storage
provider's encryption flag
([keys.rs:178-180](../../../../crates/app_supervisor/src/keys.rs#L178)). On a
node configured with `storage.encryption = false`
([config.rs:200](../../../../crates/core/src/config.rs#L200); the default is
`true`, but it is a real setting) the vault works perfectly and
`kek_is_loaded()` still answers `false`. The supervisor's own test fixture
injects a KEK even when it turned encryption off, for exactly this reason
([service.rs:3667-3672](../../../../crates/app_supervisor/src/service.rs#L3667)).
A pre-check would therefore refuse `adopt` outright on an
encryption-disabled node whose vault is fine.

**Resolution (D-A7-1).** Attempt the mint and let `VaultError::Locked` decide.
`VaultError::from` already turns the storage layer's
`EncryptionKeyRequired` marker into `Locked` with the operator's exact next
command in its `Display`
([keys.rs:33](../../../../crates/app_supervisor/src/keys.rs#L33),
[:49-53](../../../../crates/app_supervisor/src/keys.rs#L49)). The reason A5d
uses the cheap check does not apply here: A5d is avoiding one failed vault
read *per due member per pass*, while `adopt` is a rare operator action that
opens the vault once.

**A note this pass found and A7 does not fix:** the same mismatch means A5d's
renewal work-list is skipped entirely on an encryption-disabled node, and
raises a `VaultLocked` alert
([alerts.rs:79](../../../../crates/app_orchestration/src/alerts.rs#L79)) per
due member, on a node whose vault would have opened. Not A7's to repair —
backlog row in §4.

### 0.2 (Correctness, blocking) The instance row cannot gain a column the way `SupervisorStore` builds its schema

`SupervisorStore::init_schema` runs one unconditional `execute_batch` of
`CREATE TABLE IF NOT EXISTS` statements, under a comment saying a version
ladder is deliberately absent because `IF NOT EXISTS` is already idempotent
([store.rs:77-92](../../../../crates/app_supervisor/src/store.rs#L77)). That
reasoning holds for a *new table* and fails for a *new column*: on a database
file that already has `desired_state`, the `CREATE TABLE` is a no-op, so a
column added to its text never appears. Every `desired_state` read then fails
at runtime — not at compile time — with `no such column: app_master_did`,
because both readers name their columns explicitly
([store.rs:512-515](../../../../crates/app_supervisor/src/store.rs#L512),
[:540-543](../../../../crates/app_supervisor/src/store.rs#L540)). A supervisor
that is running today would stop answering `status`, `submit`, and its own
loop's work list.

This is the first column any slice of this milestone has added to
`SupervisorStore`. A5e re-keyed the *values* in several tables (D-A5e-2), which
needs no DDL at all, so nothing here has hit this before.

The precedent exists one crate over, with the reasoning already written down:
`RegistryStore::new` adds A5a's `manifest_hash` with a single
`ALTER TABLE … ADD COLUMN` that tolerates `duplicate column name`, under a
comment stating plainly that this is not the version ladder AGENTS.md rules
out, because there is no schema version to track
([registry_store.rs:128-147](../../../../crates/data_db/src/registry_store.rs#L128)).

**Resolution (D-A7-2).** Same shape, same reasoning, in `init_schema`: the
column goes into the `CREATE TABLE` text *and* into one idempotent additive
`ALTER TABLE`. Pinned by test 88, which opens a store, drops the column back
out, and reopens it.

### 0.3 (Understated) The vault-key name needs its collision argument made now, and it is much simpler than the member one

This is the second collision question the milestone has faced, and the first
one cost two review rounds (D-A5e-12, plus a round-2 finding on the rename).
`member_master_name` is
`member-<app_instance_id>#<service_name>-<index>`
([keys.rs:263-293](../../../../crates/app_supervisor/src/keys.rs#L263)), and it
needed the `#` because it joins **two** variable-length segments that both
permit `-`: with a `-` boundary, instance `a` + service `b-c` and instance
`a-b` + service `c` produced the identical key, which handed one master to two
different app instances.

The app-master name has no such boundary. `app-<app_instance_id>` has exactly
one variable segment, and it is the whole remainder of the string, so the map
from instance id to name is injective by construction — there is nothing to be
ambiguous about. Cross-kind collision is impossible for a different reason: the
two names have disjoint fixed prefixes at position 0 (`app-` versus `member-`),
so no member name can ever equal an app name whatever either id contains.
Neither argument depends on which characters an `AppInstanceId` permits, which
is what makes it worth writing down: it stays true if that validator changes.

One guard is still needed, and for the same reason A5b's S5 finding gave
`member_master_name` one. The two validators do not agree, and the gap is wider
than one character. `AppInstanceId::try_new` refuses exactly three things —
empty, `/`, and `#`
([models.rs:94-116](../../../../crates/app_orchestration/src/models.rs#L94)) —
while `validate_backup_name`, which `export_master` and `import_master` both
enforce, refuses empty, `/`, **`\`**, **`..`**, and **any absolute path**
([keys.rs:71-83](../../../../crates/app_supervisor/src/keys.rs#L71)). So an
instance id containing `..` or `\` is valid as an id and unusable as a backup
name. Without a check at mint time, such an instance would mint a key that can
never be backed up, and the operator would find out only when they tried.

**Resolution (D-A7-3).** `app_master_name` validates through
`validate_backup_name` exactly as `member_master_name` does, so a name
`export-master` would later refuse is refused at mint. Test 85 covers both
`..` and `\`, since the claim being made is about the whole validator, not one
character. And
deliberately **no** counterpart is added to `roymctl`'s own duplicated
`member_master_name`
([member_identity.rs](../../../../apps/roymctl/src/commands/member_identity.rs)):
that copy exists because A0's operator-side flow mints member masters into
files, and nothing client-side ever mints an app master. The
"keep the two copies in sync" rule does not extend to this name, and
`keys.rs`'s doc comment should say so, so a later reader does not add one out
of symmetry.

### 0.4 (Understated) Nothing can enumerate vault keys, so the instance row is not a cache — it is the only index

`ServiceStore` exposes `write_secret` and `reveal_secret` and nothing else
([traits.rs:119-123](../../../../crates/data_db/src/traits.rs#L119)). There is
no list, no prefix scan. A master in the vault is reachable only by a name
someone can already compute.

Two consequences shape this slice:

1. **The DID on the instance row is not redundant.** For member masters the
   plan itself carries the DIDs (they are the substituted `service_id`s,
   [keys.rs:353-357](../../../../crates/app_supervisor/src/keys.rs#L353)), so
   nothing extra records them. The app master appears in no plan, so if the row
   does not carry it, nothing does.
2. **`status` must not read it from the vault.** A locked vault is the ordinary
   state of a freshly-booted supervisor, and `status` is the surface an operator
   reaches for first. Reading through the vault would make the app's identity
   invisible exactly when the operator most wants to see it, and would turn a
   read-only RPC into one that fails on a `VaultError`. The stored column is
   readable with the vault shut.

**Resolution (D-A7-4/D-A7-6).** The DID is stored on the row and `status`
reads only the row. **Pinned by test 99, added in review:** this is the whole
reason for the column, and tests 93/94 use the ordinary fixture, so without a
locked-vault case a later change that reads the DID back through the vault
would pass every other test in §3.

The way to reach that state is **not** `service_with_locked_vault()`
([service.rs:3704-3706](../../../../crates/app_supervisor/src/service.rs#L3704)),
which the review's first version of this test named and which cannot work:
`build_with_key_store` opens a **fresh** `SupervisorStore::open_in_memory()` and
a fresh tempdir on every call
([service.rs:3657-3658](../../../../crates/app_supervisor/src/service.rs#L3657)),
so a rebuilt service has no rows and an empty vault — there is nothing left to
read back. The one service must be locked *in place*, and A5d's vault-race test
is the precedent: build `Fixture { locked_vault: true, inject_kek_anyway: true }`
through `build_with_key_store()` (an encrypted vault that is currently open),
adopt, then `key_store.clear_kek()`
([key_store.rs:98-103](../../../../crates/data_keystore/src/key_store.rs#L98),
used exactly this way at
[service.rs:7620-7642](../../../../crates/app_supervisor/src/service.rs#L7620)).
That leaves a genuinely locked vault over a store that still holds the adopted
row, which is the state D-A7-4 makes its claim about.

### 0.5 (Correctness, blocking) `import-master` can replace the app master with nothing updating the row, and two supervisors can mint two different app DIDs for one instance

`import_master` takes a name, reads
`<backup_dir>/<name>.key`, and writes those bytes into the vault
([keys.rs:255-260](../../../../crates/app_supervisor/src/keys.rs#L255)). It
checks nothing about what the name means and touches no other state. So
importing `app-<instance-id>` changes the app's key while the instance row
still names the old DID. Every later `status` reports a DID whose key the vault
no longer holds — which is precisely the un-diagnosable state this milestone
has closed twice already for other facts.

The sharper version of the same problem is the handover ADR-0022 §1 calls "a
key move". The intended sequence on the new supervisor is `submit`,
`import-master`, `adopt`. If an operator runs `adopt` **before**
`import-master`, the mint finds nothing under the name and mints a *second*
app identity for the same instance. The generation fence ADR-0022 §2 relies on
does not catch this: a generation fences two writers over **one** record, and
this produces **two DIDs**, hence two records that never meet. The
developer guide already states the neighbouring constraint — that
mint-in-place means exactly one vault holds a given master, and running two
supervisors over one imported master is unsupported
([developer-guide.md](../../../developer-guide.md), *Master anchors are
refreshed on the same tick*) — but says nothing about ordering, because until
A7 nothing was minted outside `submit`.

Blast radius today is bounded: with no Tier-1 record (S1) and no signed
topology document (S2), a wrongly minted app DID has no external consumer, and
the fix is to import and re-adopt. It stops being bounded the moment S1
publishes under this key.

**Resolution (D-A7-5).** `adopt` writes the row's DID **on every call**, from
whatever the vault holds at that moment — a resolve-then-record, not a
mint-once. Then `import-master` followed by `adopt` (which a handover needs
anyway, to claim the generation) always leaves the row and the vault in
agreement, and the wrong order is repairable by re-running `adopt`. The
ordering rule goes in the developer guide beside the existing custody text,
and a backlog row carries the un-enforced part to S1, where it acquires teeth.

**The right order needs a test across two vaults, not one (test 100, added in
review).** The property this slice actually promises is about a *second*
supervisor: B has never adopted this instance, imports `app-<id>` from the
backup directory, and its first `adopt` reports **A's** DID rather than minting
a fresh one. Test 95 does mint → import → adopt inside a single vault, which
proves the row follows the vault but not that a handover preserves the app's
identity — and preserving it across a supervisor change is the entire reason
ADR-0022 §1 gives for the app holding this key at all. Sending the *wrong*
order to a backlog row while leaving the *right* order untested is the gap the
review caught.

**It is in-process, but not free: the test fixture needs one new field.** The
first version of this justification said only that `MasterVault::new` takes its
backup directory as a plain argument
([keys.rs:154-167](../../../../crates/app_supervisor/src/keys.rs#L154)), which is
true at the vault level and beside the point at the level this test asserts —
the claim is about supervisor B's *instance row*, so it goes through the
service fixture, and there the backup directory is a private
`dir.path().join("backups")` on a `TempDir` the builder drops before it returns
([service.rs:3657](../../../../crates/app_supervisor/src/service.rs#L3657),
[:3678](../../../../crates/app_supervisor/src/service.rs#L3678)). Two fixture-built
services therefore cannot see each other's backups at all. (The existing
fixture works despite that dropped guard only because both the service db dir
and the backup dir are created on demand at first use —
[sqlite.rs:1640-1646](../../../../crates/data_db/src/sqlite.rs#L1640),
`ensure_backup_dir` at
[keys.rs:85-97](../../../../crates/app_supervisor/src/keys.rs#L85) — so each
fixture quietly leaks one temp directory. That is another reason the *test* must
own this directory rather than the builder.)

So phase 3 adds `backup_dir: Option<PathBuf>` to `Fixture`, defaulting to
today's behaviour, with the test holding the `TempDir` for the whole test —
about six lines. The alternative, hand-building two `SupervisorService`s beside
the fixture, duplicates thirteen constructor arguments twice and is worse.

### 0.6 (Ambiguous, and the requester asked for this stated) What an instance adopted before A7 gets

Pre-release, so no migration (the project's standing position, applied most
recently to the A5e vault-name change). But "no migration" is not the same as
"no stated effect", and the effect here is:

- An instance adopted before A7 keeps its row and its generation. Its
  `app_master_did` column is empty (D-A7-2's `DEFAULT ''`), and `status`
  reports the field as absent.
- **Its next `adopt` mints one retroactively**, because `adopt` is the only
  mint point and is designed to be re-run: the second `adopt` in
  `adopt_reads_the_held_generation_from_the_managed_node_and_claims_the_next`
  claims generation 2 from a supervisor that already held 1
  ([supervisor_interface_e2e.rs:601-608](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L601)).
  Nothing else — not `submit`, not `force-reconcile`, not the resident loop —
  mints it.
- The visible cost of that is one generation bump per adopt. A generation bump
  is not free: it re-stamps every placed substrate
  ([service.rs:2621-2631](../../../../crates/app_supervisor/src/service.rs#L2621))
  and any *other* supervisor holding the lower number correctly goes
  `Superseded`. That is the intended meaning of `adopt`, not a side effect to
  work around, and it is why A7 does not try to backfill the DID from a
  cheaper verb.

**Resolution (D-A7-7).** Retroactive at the next `adopt`, never anywhere else,
with an explicit line in the developer guide in the same shape as the A5e
vault-name callout it sits next to. No backfill on `status`: a read that mints
a key is a surprise, and `status` holds no per-instance lock
([service.rs:3180-3197](../../../../crates/app_supervisor/src/service.rs#L3180))
while every mint path in this crate does.

### 0.7 (Ambiguous) The `status` field's shape: `option<string>`, not an empty string

`instance-status`
([supervisor.wit:74-92](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L74))
already distinguishes "no value" from "empty value" where the difference
matters — `last-reconciled-at: option<u64>` exists precisely because
`Some(now)` would have reported every instance as freshly reconciled (H1,
A5b review), and `binding-convergence.observed-epoch` is an option for the
same reason. An empty `app-master-did` string would mean "never adopted under
A7", which is a genuinely different fact from "adopted", and a caller
comparing DIDs must not have to know that `""` is a sentinel.

**Resolution (D-A7-6).** `app-master-did: option<string>`. The store keeps
`TEXT NOT NULL DEFAULT ''` (a nullable column would need `ALTER TABLE` to add a
default anyway, and SQLite accepts `NOT NULL DEFAULT ''` on an added column),
and the RPC maps empty to `None` in one place.

### 0.8 (Scope-changing) `adopt` returns a bare `u64`, so the mint has no operator surface at the moment it happens

`adopt: func(app-instance-id: string) -> result<u64, string>`
([supervisor.wit:106-109](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L106)),
and `roymctl` prints `Adopted '<id>' at generation N`
([supervisor.rs:279-284](../../../../apps/roymctl/src/commands/supervisor.rs#L279)).
So under the literal reading of `task.md` ("record it on the instance row, and
surface it on `status`"), an operator who adopts an instance is told nothing
about the key that was just created, and must run a second command to learn it
exists.

This milestone has already paid for that mistake once. `submit` returns
`minted-master` rows *because* mint-in-place means the operator holds nothing
until they ask
([supervisor.wit:19-21](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L19)),
and the A5b review's S1 finding was that printing the bare logical name
instead of the vault name produced a follow-up command that always failed
([supervisor.rs:265-275](../../../../apps/roymctl/src/commands/supervisor.rs#L265)).
The same argument applies unchanged to a master minted at `adopt`.

**Resolution (D-A7-8).** `adopt` returns a record carrying the generation, the
app master DID, and the vault name `export-master` takes. This is an addition
beyond `task.md`'s wording, so its cost is named exactly: four e2e assertions
that read `adopted.result.as_u64()`
([supervisor_interface_e2e.rs:485](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L485),
[:599](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L599),
[:608](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L608),
[:708](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L708))
and one `println!`. Nothing else reads `adopt`'s result. Listed as §6
question 1 in case the requester prefers the smaller change.

Deliberately **no** `minted: bool` field. It reads as a health signal
("`false` means something is wrong") when it means nothing of the kind, and
the operator signal for an actual mint already exists as the vault's own
`tracing::warn!`.

### 0.9 (Understated) The mint warning in `get_or_mint` says "member master", and the recovery advice for an app master is different

`get_or_mint` logs one `warn!` when it really mints, telling the operator to
back the key up and that losing it "orphans every row this member has written"
([keys.rs:218-228](../../../../crates/app_supervisor/src/keys.rs#L218)).
For an app master, both nouns and the consequence are wrong: it owns no rows,
and what is lost is the app's network identity — after S1, the ability to
update the Tier-1 record at all, since the registry verifies a record against
the key its own `service_id` resolves to
([dht_registry.rs:118](../../../../crates/core/src/dht_registry.rs#L118)), and
no delegation can stand in for that.

`get_or_mint` cannot fix the text itself, because it does not return whether it
minted, so the caller cannot log it either.

**Resolution (D-A7-9).** `get_or_mint` takes a `MasterKind` (two variants) used
only to pick the warning's wording. **Twelve** call sites — one production
(`mint_and_substitute`,
[keys.rs:340](../../../../crates/app_supervisor/src/keys.rs#L340)) and eleven in
tests (`keys.rs` 404, 421, 423, 435, 436, 543, 544, 605; `service.rs` 6862,
7826, 7828) — all mechanical. *Corrected in review from "fourteen", which
counted three doc-comment mentions and one test name as call sites; the
decision is unchanged, but its cost is the number it rests on.* The alternative
(flatten the message to a generic "master") is one site and worse text; taken
only if the churn is judged not worth it.

### 0.10 (Understated) The app master gets no anchor, needs none in this slice, and its absence must not read as an oversight

A5d's `refresh_due_master_anchors` republishes an anchor per **member** master,
iterating the plan's services and reading each member's own index (D-A5e-5).
The app master is in no plan, so it gets no anchor. That is correct here and
not an accident:

- An anchor exists so a *delegated* key verifies against its master
  (A0/A1). The app master delegates nothing in this slice — it signs nothing at
  all.
- The Tier-1 record S1 will publish is signed by the app key itself, under its
  own DID, which is the registry's plain self-signed admission path, not the
  delegation-signed second path A1 added.

Whether an anchor is ever needed (for example, if S1 wants revocation of the
app key expressed the same way member keys express it) is **S1's decision**,
not a gap A7 left. Recorded in §4 as a backlog row so it is decided rather
than discovered.

### 0.11 (Correctness) `retire`, `release`, and a re-`submit` must not remove the app master, and nothing does

`retire`/`release` write one boolean column each through `set_flag`
([store.rs:562-599](../../../../crates/app_supervisor/src/store.rs#L562)); a
re-`submit` replaces the row's plan and inventory but its `ON CONFLICT` update
list names only five columns
([store.rs:499-505](../../../../crates/app_supervisor/src/store.rs#L499)), so
the new column survives a resubmit untouched as long as it is left out of that
list. No path deletes a `desired_state` row at all.

That is the behaviour A7 wants, and it matches the standing constraint the
milestone already carries for member masters and revocations (D-A5d-15,
D-A5e-18): a key is never forgotten, because forgetting one is unrecoverable
while keeping one is merely untidy. Worth stating because the obvious symmetry
argument ("retiring an instance should clean up after it") points the wrong
way. Test 89 covers all three paths — resubmit, `retire`, and `release` — after
review found it pinning the resubmit case alone while the other two were argued
here and asserted nowhere. Both are `set_flag` calls, so all three fit one
test. §4 carries the "nothing forgets it" backlog row.

### 0.12 (Understated, and it makes the tests cheap) `adopt` on an instance whose plan has no services is fully in-process

`placed_aliases` over an empty service list returns an empty vector
([service.rs:1676-1693](../../../../crates/app_supervisor/src/service.rs#L1676)),
`build_clients` over an empty alias list returns an empty map
([service.rs:1788-1810](../../../../crates/app_supervisor/src/service.rs#L1788)),
and `claim_next_generation` then reads nothing and claims nothing, returning
`0 + 1`
([service.rs:2607-2633](../../../../crates/app_supervisor/src/service.rs#L2607)).
The `plan_json_no_services` helper already exists in the test module
([service.rs:4650-4658](../../../../crates/app_supervisor/src/service.rs#L4650)).

So every behaviour this slice adds to `adopt` — mint, record, report, refuse
when locked, reconcile after an import — is unit-testable with no substrate,
no network, and no fake actor. That is unusual for this crate and is why §3's
list is mostly unit tests with a single e2e.

The same code path has one edge worth naming: because `submit`'s mint loop
iterates services
([keys.rs:334](../../../../crates/app_supervisor/src/keys.rs#L334)), a
services-less plan skips member minting entirely, so an instance id that
`validate_backup_name` would refuse *can* reach `adopt` with a row already in
place. D-A7-3's validation is what makes that a clear refusal instead of a
partial adopt.

### 0.13 (Stale) Four shipped copies of "member master" prose — two of them in files this slice already opens — plus two traceability cells that are stale in their text, not only their status

The plan's first draft caught this in ADR-0022 and missed it in the tree.
`export-master`/`import-master` need **no signature change** to carry an app
master, which is why §5 can claim them as already-built — but every string
around them says "member", and after A7 both verbs carry two kinds of master.
Operator prose is the surface an operator reads *before* typing
`export-master app-<id>`, so this is work, not tidying:

- [supervisor.wit:25-29](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L25)
  — `minted-master.vault-name`'s doc says "the vault keys on
  `member-<app-instance-id>-<service-name>-<index>`". That is a **second live
  copy of the pre-A5e `-` boundary** the ADR also carries (below), in shipped
  WIT, in a file this slice is already editing for `instance-status` and
  `adopt-result`.
- [supervisor.wit:139-142](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L139)
  — `import-master`'s doc describes its argument as "an A0-A4 deployment's
  `<dir>/identities/member-*.key`" only, which is now one of two things it
  imports.
- [supervisor.rs:1-4](../../../../apps/roymctl/src/commands/supervisor.rs#L1) —
  the `roymctl supervisor` module doc ends "back up or adopt a member master".
- [supervisor.rs:74-84](../../../../apps/roymctl/src/commands/supervisor.rs#L74)
  — `ExportMaster`'s help ("write a member master into its configured
  `master_backup_dir`") and `ImportMaster`'s help ("an A0-A4 deployment's
  `<dir>/identities/member-*.key`"). This is `--help` output, not a comment.

The plan already updates `keys.rs`'s module doc for exactly this reason
([keys.rs:1-4](../../../../crates/app_supervisor/src/keys.rs#L1) opens with
"Member master keys"), so the same pass belongs on all four.

Two documentation facts outside the tree, to fix at sign-off:

- ADR-0022 §1 gives the vault-name form as
  `member-<app_instance_id>-<service_name>-<index>`
  ([0022 §1](../../../decisions/0022-two-tier-logical-service-discovery.md)),
  which A5e replaced with a `#` boundary (D-A5e-12,
  [keys.rs:290](../../../../crates/app_supervisor/src/keys.rs#L290)). The ADR
  was written 2026-08-02, one day before A5e landed that change. Harmless in
  itself, and exactly the kind of stale detail a later reader would copy — as
  the WIT copy above shows, since both descend from the same pre-A5e sentence.
- [traceability-matrix.md](../../traceability-matrix.md)'s two A7-gated rows do
  not just need a status flip. `[LFC-MGT]` (App Supervisor) still reads
  **Planned** with evidence pointing at "Slices A0-A5", and `[FND-IDT]`
  (stable service identity) reads **In progress (Slices A0-A1 complete; A2
  outstanding under this same requirement)** — written before A2 through A5e
  landed. Both cells need rewriting with real evidence, not a one-word edit.

**Resolution (D-A7-14).** One prose pass over the four shipped sites, in
phase 3, correcting the `-`-boundary copy while it is open. No behaviour
changes, and no test — `--help` text and doc comments have none in this tree.

---

## §1 — Decisions

| ID | Decision |
|---|---|
| **D-A7-1** | The mint in `adopt` runs **before** any substrate connection, and a locked vault fails the whole call through the existing `VaultError::Locked` message — **not** through a `kek_is_loaded()` pre-check, which answers `false` on a working vault whenever `storage.encryption = false` (§0.1). Same ordering rule and same reason as `submit`'s own mint ("a locked vault or a bad plan must fail before anything is persisted or a network round trip spent", [service.rs:2528-2534](../../../../crates/app_supervisor/src/service.rs#L2528)). Because the refusal happens before `claim_next_generation`, a locked-vault `adopt` burns no generation and is simply re-run after `inject-kek`. |
| **D-A7-2** | The DID lives in a new `desired_state.app_master_did TEXT NOT NULL DEFAULT ''` column, added **both** in the `CREATE TABLE` text and by one idempotent `ALTER TABLE … ADD COLUMN` that tolerates `duplicate column name` (§0.2). This is not a version ladder — there is no schema version to track — and it is the exact shape and reasoning `RegistryStore` already uses for A5a's `manifest_hash` ([registry_store.rs:128-147](../../../../crates/data_db/src/registry_store.rs#L128)). Without it, a supervisor whose database predates A7 fails every `desired_state` read at runtime. |
| **D-A7-3** | The vault key is `app-<app_instance_id>`, validated through `validate_backup_name` at mint time. Collision-safe by construction on two independent grounds: one variable-length segment, so the map from instance id to name is injective; and a fixed `app-` prefix disjoint from `member-` at position 0, so no member name can ever equal an app name (§0.3). Neither argument depends on `AppInstanceId`'s validator, unlike D-A5e-12's. `roymctl` gets **no** duplicated copy of this name, and `keys.rs` says why. |
| **D-A7-4** | The DID is **stored**, not derived on read. Nothing can enumerate vault keys ([traits.rs:119-123](../../../../crates/data_db/src/traits.rs#L119)), the app master appears in no plan, and the vault is locked after every restart — so the row is the only index, and it is the only copy readable while the vault is shut (§0.4). |
| **D-A7-5** | `adopt` **resolves-and-records on every call**, writing the row's DID from whatever the vault holds at that moment rather than only when it mints (§0.5). This is what makes a handover self-correcting: `import-master` then `adopt` always agrees, and the wrong order (`adopt` before `import-master`, which mints a *second* app identity that the generation fence cannot catch, since two DIDs are two records) is repaired by re-running `adopt`. The ordering rule is documented; enforcing it is S1's, where the key acquires an external consumer. |
| **D-A7-6** | `instance-status` gains `app-master-did: option<string>`, absent meaning "never adopted under A7" — the same option-not-sentinel choice `last-reconciled-at` and `observed-epoch` already make (§0.7). `status` reads the row only, and never opens the vault. |
| **D-A7-7** | An instance adopted before A7 gains its app master at its **next `adopt`**, and nowhere else — not on `submit`, `force-reconcile`, a loop pass, or `status` (§0.6). Until then `status` reports the field absent. Pre-release, so no migration and no backfill verb; the visible cost is one generation bump, which is what `adopt` means. Stated in the developer guide in the same shape as the A5e vault-name callout. |
| **D-A7-8** | `adopt`'s return becomes a record: `generation`, `app-master-did`, `vault-name` (§0.8). An operator must learn the backup command at the moment the key exists — the same rule `submit`'s `minted-master` rows encode, and the same failure the A5b S1 finding fixed. Cost: four e2e assertions and one `println!`, all named. No `minted: bool`. |
| **D-A7-9** | `MasterVault::get_or_mint` takes a `MasterKind` used only to pick its mint-warning wording, because an app master's loss story is not a member's (§0.9). **Twelve** mechanical call sites — one production (`mint_and_substitute`) and eleven in tests, enumerated in §0.9. *Corrected from "fourteen" in review; the count is what this decision's cost rests on.* |
| **D-A7-10** | No anchor, no registry record, no signing with the new key in this slice, and the absence is deliberate rather than deferred work: the app master delegates nothing, and a Tier-1 record is self-signed under its own DID (§0.10). Whether it ever needs an anchor is S1's decision, recorded as a backlog row. |
| **D-A7-11** | The app master is never forgotten: `retire`, `release`, and a re-`submit` all leave it alone, and the new column stays out of `submit`'s `ON CONFLICT` update list (§0.11). Same standing constraint as D-A5d-15 / D-A5e-18, for the same reason — forgetting a key is unrecoverable, keeping one is untidy. |
| **D-A7-12** | Custody stays local by construction, unchanged: the DID is minted in the supervisor's own vault through `StorageProvider::open_service_db`, an in-process call ([keys.rs:5-11](../../../../crates/app_supervisor/src/keys.rs#L5)), and no key bytes cross the wire in either direction — `export-master` returns a path, `import-master` takes a name. A7 adds one more key to that arrangement and changes none of its properties, so failure-matrix row 14's blast-radius claim is unaffected and needs no re-argument. |
| **D-A7-13** | No new WIT verb, so no new dispatch arm and no change to `every_verb_is_refused_without_substrate_admin`'s list ([service.rs:3753-3775](../../../../crates/app_supervisor/src/service.rs#L3753)). Recorded as a decision because the obvious S1-shaped instinct — "add a `resolve` or an `app-did` verb while the file is open" — is exactly the scope creep `task.md` warns against. Reading the DID is `status`'s job. |
| **D-A7-14** | *(added in review)* One prose pass over the four shipped places that describe these verbs as member-master-only — `minted-master.vault-name`'s doc and `import-master`'s doc in `supervisor.wit`, and `roymctl supervisor`'s module doc and its `ExportMaster`/`ImportMaster` help text (§0.13). Two of the four are in files this slice already edits, one is `--help` output an operator reads before typing `export-master app-<id>`, and one carries a second live copy of the pre-A5e `-` vault-name boundary that gets corrected in the same pass. Signatures are genuinely unchanged (D-A7-12); the prose around them is not. |

---

## §2 — Phase plan and merge order

Four phases. The ordering rule is that **nothing observable changes until
phase 3**: phases 1 and 2 add a name, a helper, and a column that no caller
reads yet, so they can be merged and reviewed on their own.

1. **The vault side.** `MasterKind` and `get_or_mint`'s new argument
   (D-A7-9); `app_master_name` with its validation and its collision comment
   (D-A7-3); one resolve-or-mint helper in
   [keys.rs](../../../../crates/app_supervisor/src/keys.rs) returning the DID
   and the vault name; the module doc updated, since it currently describes the
   vault as holding member masters only
   ([keys.rs:1-16](../../../../crates/app_supervisor/src/keys.rs#L1)). Nothing
   calls the helper yet. Tests 84-87.
2. **The store side.** The column, in both the `CREATE TABLE` text and the
   idempotent `ALTER TABLE` (D-A7-2); `DesiredState.app_master_did`
   ([store.rs:17-29](../../../../crates/app_supervisor/src/store.rs#L17)) and
   the two readers that name their columns explicitly; a
   `set_app_master_did` writer in `set_generation`'s shape — one column plus
   `updated_at`, erroring when no row matched
   ([store.rs:603-614](../../../../crates/app_supervisor/src/store.rs#L603));
   the column left out of `submit`'s `ON CONFLICT` list (D-A7-11). Tests
   88-89.
3. **`adopt`, `status`, and the CLI.** In `handle_adopt`, the mint between the
   plan parse and `build_clients` (D-A7-1), and the row write beside
   `set_generation`/`un_retire` after the claim succeeds (D-A7-5). The
   asymmetry is deliberate and worth a comment in the code: the key is stored
   before the claim, the row after it, because a vault key with no row is
   recoverable — the next `adopt` reads the same key — while a row naming a DID
   whose key was never stored is not. Then `instance-status`'s new field
   (D-A7-6), `adopt-result` (D-A7-8), the four e2e assertions, and `roymctl`'s
   adopt print. `roymctl supervisor status` needs **no** change: it prints the
   response as pretty JSON
   ([supervisor.rs:321-326](../../../../apps/roymctl/src/commands/supervisor.rs#L321)),
   so the new field appears on its own. **Plus D-A7-14's prose pass** over the
   four member-master-only strings — `supervisor.wit`'s `minted-master.
   vault-name` and `import-master` docs, and `roymctl supervisor`'s module doc
   and `ExportMaster`/`ImportMaster` help — since two of them are in the files
   this phase already edits and one is `--help` output. **And one test-fixture
   change:** `Fixture` gains `backup_dir: Option<PathBuf>`, defaulting to
   today's private tempdir, without which test 100's two services cannot share
   a backup directory (§0.5). Tests 90-97, 99-101.
4. **End to end, and the docs.** One e2e proving the whole operator story on a
   real supervisor and a real managed substrate (test 98), plus §4's
   documentation.

**What could move:**

- **Phases 1 and 2 could ship together as one merge**, and probably should:
  neither is reachable from any caller, and reviewing "a new vault name and a
  new column" as one change is easier than reviewing them apart.
- **Phase 3 cannot be split** along the "row versus WIT" line. A stored DID no
  surface reports is not this slice's deliverable, and `task.md` names the
  `status` read as part of the exit criterion.
- **Phase 4's e2e is the only part with a real time cost** (two booted
  substrates). If the milestone is under pressure, note that tests 90-97 prove
  every property in-process and the e2e proves the *sequence* — the same split
  D-A5e-15 accepted for the reference scenario. It should still land here:
  `export-master` writing a real file into a real backup directory on a real
  node is not provable in-process.

---

## §3 — A7 tests

Named the way §8, §13, §23, §29, and §36 named theirs. **e2e cases are
marked; everything else is a unit test.** Continues from A5e's 83.

**Phase 1 —** [keys.rs](../../../../crates/app_supervisor/src/keys.rs):

84. `an_app_master_name_can_never_equal_a_member_master_name` — the two
    prefixes at position 0, asserted over instance ids chosen to look like a
    member name's tail (D-A7-3, §0.3). The companion to A5e's test 47, which
    pinned the *member* name's own boundary
85. `an_app_instance_id_that_could_not_be_backed_up_is_refused_before_minting`
    — `..` **and `\`** as instance ids (both valid `AppInstanceId`s, both
    refused by `validate_backup_name`): refused at mint, and nothing written to
    the vault, the same property A5e's `..`-service-name test asserts one field
    over (D-A7-3, §0.3 as corrected in review)
86. `a_second_resolve_returns_the_app_master_already_in_the_vault` — resolve
    twice, same public key, one key in the vault (D-A7-5)
87. `an_app_master_round_trips_through_export_master_and_import_master` —
    exported under `app-<id>`, the file lands inside the configured backup
    directory at mode `0o600`, and re-importing it restores the same key
    (`task.md`'s "movable through `export-master` / `import-master`")

**Phase 2 —** [store.rs](../../../../crates/app_supervisor/src/store.rs):

88. `a_database_that_predates_the_app_master_column_gains_it_on_open` — open,
    drop the column back out, reopen, and read a row successfully. The failure
    this pins is silent at compile time and total at runtime: every
    `desired_state` read fails with `no such column` (D-A7-2, §0.2)
89. `a_recorded_app_master_survives_a_resubmit_a_retire_and_a_release` —
    `set_app_master_did` leaves generation, `paused`, `retired`, and the plan
    unchanged, and none of the three paths that could plausibly clear it does:
    a later `submit` of new desired state, `retire`, or `release`. *Widened in
    review from the resubmit case alone* — `retire`/`release` were argued in
    §0.11 and asserted nowhere, and both are `set_flag` calls, so all three fit
    one test (D-A7-11)

**Phase 3 —** [service.rs](../../../../crates/app_supervisor/src/service.rs),
all over a services-less plan so no substrate is involved (§0.12):

90. `adopt_mints_an_app_master_and_records_it_on_the_instance_row` — the row
    carries a `did:key:` value afterwards, and the vault holds the key under
    `app-<id>`
91. `adopt_on_a_locked_vault_refuses_before_it_claims_a_generation` — the
    locked fixture (`locked_vault: true`, encryption on with no KEK, the only
    shape that proves anything about locking —
    [service.rs:3627-3631](../../../../crates/app_supervisor/src/service.rs#L3627));
    the error names `inject-kek`, the generation is unchanged, and no DID is
    recorded (D-A7-1)
92. `a_second_adopt_reports_the_same_app_master_did_at_the_next_generation` —
    the DID is stable across adopts while the generation is not (D-A7-5)
93. `status_reports_the_app_master_did_of_an_adopted_instance` (D-A7-6)
94. `status_reports_no_app_master_for_an_instance_that_was_never_adopted` — the
    field is absent, not an empty string (D-A7-6, §0.7)
95. `adopt_after_an_import_records_the_imported_did_not_the_one_it_replaced` —
    the handover-order repair: mint by adopting, import a different key under
    the same name, adopt again, and the row follows the vault (D-A7-5, §0.5)
96. `an_instance_row_with_no_app_master_gains_one_on_its_next_adopt` — writes a
    row with the column empty (the pre-A7 state) and asserts the next `adopt`
    fills it, which is D-A7-7's stated effect rather than a silent gap
97. `adopt_returns_the_vault_name_export_master_accepts` — the returned name is
    fed straight to `export-master` and succeeds. This is the A5b S1 failure
    asserted directly: a printed name the follow-up command refuses is worse
    than no name (D-A7-8, §0.8)

**Phase 4 —** new
`crates/substrate/tests/app_instance_identity_e2e.rs`, ports **13_000-13_002**
(supervisor) and **13_100-13_102** (managed), the next free block after
`reference_scenario_e2e.rs`'s 12_700-12_902
([reference_scenario_e2e.rs:111-122](../../../../crates/substrate/tests/reference_scenario_e2e.rs#L111)):

98. **e2e** `an_adopted_app_instance_carries_an_exportable_master_did` — one
    real supervisor node and one real managed node, KEK injected as every other
    supervisor e2e does
    ([supervisor_interface_e2e.rs:190](../../../../crates/substrate/tests/supervisor_interface_e2e.rs#L190)):
    `submit`, `adopt`, then (a) `adopt`'s result carries a `did:key:` app master
    and a vault name, (b) `status` reports the same DID, (c) `export-master`
    with that name writes a file under the node's `master_backup_dir`, and
    (d) a second `adopt` reports the identical DID at a higher generation.
    The claim is the sequence, in the operator's own order — the individual
    properties are already proven by tests 90-97

**Tests added in review (2026-08-04).** Three decisions this plan makes had no
case that would fail if they were violated. Each is assigned to a phase rather
than appended to the end of the build:

99. `status_reports_the_app_master_did_while_the_vault_is_locked` —
    **phase 3**, beside tests 93/94. One service, locked **in place**:
    `Fixture { locked_vault: true, inject_kek_anyway: true }` through
    `build_with_key_store()`, `adopt`, then `key_store.clear_kek()`, then
    `status` — A5d's vault-race recipe
    ([service.rs:7620-7642](../../../../crates/app_supervisor/src/service.rs#L7620)).
    **Not** a rebuild through `service_with_locked_vault()`: the fixture opens a
    fresh in-memory store and a fresh tempdir per call
    ([service.rs:3657-3658](../../../../crates/app_supervisor/src/service.rs#L3657)),
    so a rebuilt service has no row to read. This is D-A7-4's whole argument —
    the column exists so the app's identity is readable when the vault is shut
    — and without this case a later change that reads it through the vault
    passes every other test in §3 (§0.4)
100. `a_second_supervisor_that_imports_the_app_master_adopts_without_minting_a_new_one`
    — **phase 3**, and the slice's most important new case: two fixture-built
    services sharing one **test-owned** backup directory, supervisor A adopts
    and exports, supervisor B (which has never adopted this instance) imports
    and adopts, and B's row carries **A's** DID. Test 95 proves the row follows
    the vault inside one vault; this proves a handover preserves the app's
    identity, which is the reason ADR-0022 §1 gives for the app holding the key
    at all (D-A7-5, §0.5). **Needs the `Fixture { backup_dir }` field phase 3
    adds** — today's builder gives each service a private backups path on a
    `TempDir` it drops, so two services cannot share one (§0.5)
101. `a_failed_claim_after_a_successful_mint_reuses_the_same_app_master`
    — **phase 3, optional.** Phase 3 states the mint-before-claim /
    record-after-claim asymmetry as load-bearing ("a vault key with no row is
    recoverable"), and nothing exercises the failing direction: an `adopt`
    whose `claim_next_generation` fails, followed by one that succeeds,
    resolves the same key rather than minting a second. Not free — it needs a
    plan with a placed service on an unreachable substrate, so §0.12's
    services-less shortcut does not apply, which is why it is marked optional
    rather than dropped

**Test count: 83 → 101** (100 required, one optional).

---

## §4 — Docs and backlog for A7

**Docs**

- [developer-guide.md](../../../developer-guide.md) — a new subsection right
  after *Custody: the supervisor mints its own member masters*, which today
  describes member masters as the only thing in the vault. It needs to say:
  the app instance has a master DID of its own, minted at `adopt`; the vault
  name is `app-<app-instance-id>` and `export-master` takes it; **it is not
  backed up automatically**, exactly like a member master; `roymctl supervisor
  status` shows it; **an instance adopted before this slice gets one at its
  next `adopt`** (D-A7-7, in the same callout shape as the existing A5e
  vault-name note, which is the precedent for how this project states an
  in-place change); and the **handover order — `import-master` before
  `adopt`** — with the reason, that adopting first mints a second identity for
  the same app (D-A7-5). Also, plainly, what the DID does *not* do yet: nothing
  publishes it, nothing resolves it, no external caller can use it until
  overlay slice S1.
- [task.md](task.md) — the A7 section marked Complete with evidence, in the
  A0-A5e shape; the exit-criteria bullet "An app instance carries a master DID
  minted at `adopt`, readable through `status`, and movable through
  `export-master` / `import-master`" marked ✅; and the
  `[LFC-MGT]`/`[FND-IDT]` bullet's "not yet, since A5e alone does not close the
  milestone" replaced by the flip actually happening. The A7 section's own
  scope fence ("no registry publication, no topology document, no resolve
  RPC") should be left exactly as written — it stayed true.
- [status.md](status.md) — the A7 row (line 29) flipped to Complete with a
  date, the overall line (lines 7-11) updated to say the milestone is closed
  rather than waiting on A7, and an A7 evidence section in the same shape as
  A5e's. The A7 row should also gain the
  `[implementation plan](slice-a7-implementation-plan.md)` link every other
  row carries.
- **Shipped operator prose (D-A7-14, part of the code change rather than the
  docs pass, listed here so it is not read twice):** `supervisor.wit`'s
  `minted-master.vault-name` and `import-master` doc comments, and `roymctl
  supervisor`'s module doc and `ExportMaster`/`ImportMaster` help text — see
  §0.13 for the four exact sites and the pre-A5e vault-name correction one of
  them carries.
- [traceability-matrix.md](../../traceability-matrix.md) — `[LFC-MGT]` (App
  Supervisor) and `[FND-IDT]` (stable service identity) flipped to **Complete**
  with evidence. Both cells need their *text* rewritten, not only their status
  word (§0.13): one still says "Planned … Slices A0-A5", the other "In
  progress (Slices A0-A1 complete; A2 outstanding)".
- [meta-implementation-plan.md](../../meta-implementation-plan.md) — the
  overlay's **S0** row marked landed as M05A slice A7, which fires **S1's**
  pickup trigger ("M05A slice A7 (S0) Complete"). Worth one added sentence
  there: S1 inherits an ordering constraint, not just a key (D-A7-5).
- ADR-0022 — an amendment recording that §1 is implemented, with three facts
  the ADR does not carry: the vault name actually chosen
  (`app-<app_instance_id>`) and why it needs no separator guard; the
  handover-ordering constraint and that it is documented rather than enforced;
  and a correction to §1's parenthetical member-master name, which quotes the
  pre-A5e format (§0.13). No decision in §1 is reversed.
- ADR-0020 — no change. Custody's properties are unchanged by adding one more
  key to the same vault (D-A7-12), and saying so once in the A7 sign-off note
  is enough.

**Backlog rows resolved**

- *"Whatever A7's own pass finds, if A7 gets one"* — the A5e plan's §37 placed
  this line in its own "rows to add" list. This section discharges it.

**Backlog rows to add**

- ***Nothing forgets an app master, and nothing should*** — an `app-<instance>`
  vault key outlives `retire`, `release`, and every resubmit, and there is no
  verb to remove one (D-A7-11, §0.11). Bounded (one key per instance this
  supervisor has ever adopted) and deliberate: the same constraint the existing
  *"A member removed from the plan keeps its master in the vault"* row already
  carries for member masters, and the row should sit beside it and cross-
  reference it. → **TBD**.
- ***Two supervisors can mint two different app masters for one instance***
  (§0.5, D-A7-5) — adopting on a second supervisor before importing the app
  master gives the app a second identity. ADR-0022 §2's generation fence does
  not catch it: a generation fences two writers over one record, and this
  produces two records that never meet. Harmless while nothing publishes the
  key; a real split-identity bug the moment S1 does. A7 documents the order
  and makes `adopt` self-correcting, and does not enforce it. → **S1**, and
  whoever picks up S1 should read this row before writing the publisher.
- ***The app master has no anchor and no revocation story*** (§0.10, D-A7-10) —
  it delegates nothing today and its Tier-1 record will be self-signed under
  its own DID, so an anchor is not needed in A7. If S1 or S2 wants app-key
  revocation expressed the way member-key revocation is, that needs a decision
  and an anchor. Recorded as a known property, not as debt A7 could have paid.
  → **S1**.
- ***Renewal is skipped on a node with `storage.encryption = false`*** (§0.1,
  found while reading and **not fixed here**) — `kek_is_loaded()` reads the
  `KeyStore`, so on a node whose vault needs no KEK it answers `false`, the
  renewal work-list is skipped whole
  ([service.rs:1026](../../../../crates/app_supervisor/src/service.rs#L1026)),
  and a `VaultLocked` alert is raised per due member for a vault that would
  have opened. A7 avoids the trap for its own mint (D-A7-1) and does not
  change A5d's behaviour. The fix is a "can this vault actually be opened"
  question rather than a "is a KEK loaded" one. → **TBD**.

**No new in-code `TODO`/`FIXME` markers.** Every deferral above is a decided
property or another slice's work, not a marker left in the tree — so §11 of
`deferred-backlog.md` (*Open in-code markers*) gains nothing from this slice.

---

## §5 — What closing this slice closes

`task.md`'s exit criteria name A7 in two bullets, and this slice is written
against both:

1. **"An app instance carries a master DID minted at `adopt`, readable through
   `status`, and movable through `export-master` / `import-master` (slice
   A7)."** Minted at `adopt` — D-A7-1, tests 90-92, 96. Readable through
   `status` — D-A7-6, tests 93-94, **and 99 with the vault locked**. Movable
   through `export-master`/`import-master` — the verbs need **no signature
   change**
   ([supervisor.wit:127-143](../../../../crates/wit_interfaces/wit/supervisor/supervisor.wit#L127)),
   because they already take a bare name; what A7 adds is a name they accept
   (D-A7-3), the prose that says so (D-A7-14 — their doc comments *do* change,
   §0.13), and proof that a handover works, including **across two vaults**
   (tests 87, 95, 97, 100, and the e2e's step c).
2. **"`[LFC-MGT]` (App Supervisor) and `[FND-IDT]` (stable service identity)
   rows flip to Complete with evidence once A7 has also landed."** A5e is
   Complete as of 2026-08-04, so A7 is the second of the two and the flip
   happens at this slice's sign-off — including the cell rewrites §0.13 found.

The milestone's remaining named exceptions are unchanged by A7 and should not
be re-opened at sign-off: failure-matrix rows 15 and 18 stay with overlay
slice S4 (D-A5e-1), and slice A6 stays deferred behind M5 item 1. A7 touches
neither.

---

## §6 — Questions for the requester

Three, each with a recommendation. Everything else in §1 is decided.

1. **Does `adopt`'s return become a record (D-A7-8), or stay a bare `u64` with
   the DID readable only from `status`?** *Recommendation: the record.* An
   operator who has just caused a key to exist should be told its backup
   command then, not on a second call — the rule `submit`'s `minted-master`
   rows already encode, and the exact failure the A5b S1 review finding fixed.
   The cost is four e2e assertions and one `println!`, all named in §0.8. If
   the answer is "stay `u64`", tests 97 and the e2e's step (a) drop to
   asserting the name from `status` instead, and the developer guide has to
   tell the operator to run `status` after every `adopt`.
2. **Should `status` also report the app master's vault name, or only its
   DID?** *Recommendation: only the DID.* The name is computable
   (`app-` + the instance id the caller already typed) and adding it to the
   read surface invites an operator to treat a custody detail as part of the
   app's identity. The counter-argument is real though — `adopt`'s output is
   easy to lose, and the whole point of D-A7-8 is that a name at the right
   moment matters. One extra `option<string>` on `instance-status` if the
   answer is yes.
3. **Is refusing `adopt` outright on a locked vault the behaviour you want
   (D-A7-1), or should `adopt` still claim the generation and leave the DID for
   later?** *Recommendation: refuse.* A claimed generation with no identity is
   a half-adopted instance, and half-states that only some later call repairs
   are what this milestone has spent three slices removing. Refusing costs
   nothing: nothing is written, no generation is burned, and the error already
   names `inject-kek`. The counter-argument is that `adopt` is also the way out
   of `retired` and of a terminal remediation state
   ([service.rs:2672-2681](../../../../crates/app_supervisor/src/service.rs#L2672)),
   so this makes those escapes need an unlocked vault too — which is arguably
   correct, since a supervisor with a locked vault cannot manage the instance
   afterwards either.

---

## §7 — Review response (2026-08-04)

All nine findings incorporated; none pushed back. Recorded the way A5c's §25,
A5d's §31/§32, and A5e's §39/§40/§42 recorded theirs, so what the first draft
missed stays readable next to what replaced it.

**Four test-coverage gaps — decisions this plan makes that nothing would have
caught being violated.** Three became tests 99-101, and one widened test 89:

1. **`status` was never tested with the vault locked** (§0.4, test 99). This
   is D-A7-4's entire argument, and tests 93/94 use the ordinary fixture, so a
   later change reading the DID through the vault would have passed every case
   in §3. The fixture already exists.
2. **The handover was never tested across two vaults** (§0.5, test 100). Test
   95 proves the row follows the vault *inside one vault*; the property the
   slice actually promises is that supervisor B, importing A's key, adopts
   without minting a fresh identity. The first draft sent the wrong order to a
   backlog row and left the right order unproven — the single most consequential
   omission of this pass, since S1 depends on exactly this.
3. **D-A7-11 was half-tested** (§0.11, test 89 widened): `retire` and `release`
   were argued and not asserted. Both are `set_flag` calls, so all three paths
   fit one test.
4. **The mint/claim asymmetry had no failing-direction case** (test 101, marked
   optional). It needs a placed service on an unreachable substrate, so
   §0.12's services-less shortcut does not apply. Kept as an optional case
   rather than dropped, because phase 3 states the invariant as load-bearing.

**Two missed call sites, both operator-facing prose in files this slice already
opens** (§0.13, now D-A7-14). The first draft caught the stale pre-A5e
vault-name in ADR-0022 and missed a **second live copy of the same sentence in
shipped WIT**, plus `import-master`'s member-only doc, `roymctl supervisor`'s
module doc, and the `ExportMaster`/`ImportMaster` `--help` text an operator
reads before typing `export-master app-<id>`. §5's claim is corrected from "the
verbs need no change at all" to "no signature change" — which was the true
statement all along, and the looser wording is what let the prose slip.

**Two stated costs corrected.** `get_or_mint` has **12** call sites, not 14
(D-A7-9): the first count included three doc-comment mentions and one test
name. D-A7-9's cost/benefit rests on that number, so it is worth being right
even though the decision does not move. And §0.3's guard argument named only
`..` when `validate_backup_name` also refuses `\` and absolute paths, while
`AppInstanceId` forbids only empty, `/`, and `#` — the resolution was already
correct (it routes through the validator rather than re-listing its rules), but
test 85 now covers `\` too, since the claim being made is about the whole
validator.

**One citation corrected.** `status.md` line 26 is A5d's row; A5e's is line 27.

### Second review (2026-08-04) — three residuals and a nit, all incorporated

Two of the three are the same mistake in the same place: the first review's own
new tests were specified against fixture behaviour that was assumed rather than
read. Worth recording as a rule for whoever implements this, since it is the
third time in this milestone that a *fix* named a mechanism without checking it
(A5e §40's own standing instruction): **a test recipe naming an existing
fixture is a claim about that fixture, and needs the same reading as a claim
about production code.**

1. **D-A7-9's decision row still said "Fourteen"** while §0.9 and §7 said
   twelve. Fixed, with the enumeration pointed at §0.9. The decisions table is
   the surface a reader consults, so a corrected count that only lands in the
   prose is not corrected.
2. **Test 99's recipe could not work.** "Adopt unlocked, then rebuild with
   `service_with_locked_vault()`" reads as though the fixture reopens one
   supervisor; it opens a fresh in-memory store and a fresh tempdir every call
   ([service.rs:3657-3658](../../../../crates/app_supervisor/src/service.rs#L3657)),
   so the rebuilt service has no row and an empty vault and the test would have
   asserted nothing. Replaced with A5d's in-place recipe —
   `inject_kek_anyway` then `key_store.clear_kek()` — which is the only way in
   this tree to hold one store while shutting its vault (§0.4).
3. **Test 100 needs a `Fixture` change that was not named.** The backup
   directory is a private path on a `TempDir` the builder drops
   ([service.rs:3678](../../../../crates/app_supervisor/src/service.rs#L3678)),
   so two fixture-built services cannot share one — and the vault-level
   argument the plan gave ("`MasterVault::new` takes the directory as an
   argument") was true but at the wrong level, since this test asserts B's
   *instance row*. Phase 3 now adds `Fixture { backup_dir }`, and §0.5 records
   the second thing that reading exposed: the dropped `TempDir` means every
   fixture today leaks one temp directory, because both the service db dir and
   the backup dir are created on demand at first use.

**Nit:** the stale vault-name doc comment in `supervisor.wit` spans **25-29**,
not 24-27.

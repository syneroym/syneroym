# M06C Slice C8 — The Transaction Vertical: Implementation Plan

> **Scope.** [task.md](task.md)'s **C8** row — **R2, all five rows** of the
> spec's [First release scope](../../../roym-integrated-experience-spec.md#r2--the-transaction-vertical):
> the transaction state machine with one named writer on the provider's
> substrate, permitted transitions, expiry, idempotency keys and a named
> conflict for a losing concurrent booking; `payment-request` and
> `payment-acknowledgement`, separate from settlement, with the payee bound
> into the signed agreement and a UI that never says "verified"; a mutually
> signed `fulfilment-receipt`; a versioned, integrity-checked export and
> import; an encrypted backup with a restore path tested on a clean node.
> Gate: **C7** (complete, PR #164, 2026-09-08).
>
> **Fixed before this slice and not re-decided here.** `D-06C-3` (seven card
> types, no others), `D-06C-12` (single-issuer attestations; "signed by both"
> is a pair; `payment-request` is a signed record; `booking-progress` is
> service-signed derived state), `D-06C-13` (two independent tracks, the
> against-interest rule, named terminals), and every `D-C7-n` except where
> §2 says otherwise and names why.
>
> **Read §18 before you start.** Six items there are product calls this plan
> makes a recommendation on and that the owner must confirm before WO1. They
> are marked **[CONFIRM]** where they appear.
>
> **Planning identifiers appear in this document and must not appear in the
> code it describes** (`D-06C-11`, AGENTS.md). No `C8`, `R2`, `M06C`, `WO`,
> `D-C8-n`, scenario number or matrix-row number in a crate, module,
> collection, JSON-RPC method, card type, record type, config key, metric,
> test name, or comment. ADR references are fine.

---

## §0 What C7 handed C8, and what is missing

Verified 2026-09-24 against `main` at `ce7b4e19`.

| Handed over | Where |
|---|---|
| Signed `request` / `quote` / `agreement-receipt`, with content-derived ids and `supersedes` versioning | `crates/roym_core/src/transaction.rs`; `crates/roym_transaction/src/app/{request_ops,quote_ops,agreement_ops}.rs` |
| `AgreedTerms` with `payee`, `payment_methods`, `payment_timing`, `schedule: Option<TimeWindow>`, cancellation/refund/dispute text, `quote_expires_at_secs` | `crates/roym_core/src/transaction.rs:120-145` |
| The card wrapper (`Card`, `CARD_CONTENT_TYPE`, `parse_card`, `card_body`) and the seven-type `CARD_TYPES` table | `crates/roym_core/src/card.rs` |
| Card transport over `conversation.send`, own-card filing, and pull ingestion by `transaction.sync` (one `history` call, watermark + overlap) | `crates/roym_transaction/src/app.rs:427-543`, `app/sync.rs` |
| Automatic provider counter-half when the provider's node files a valid consumer half | `crates/roym_transaction/src/app/agreement_ops.rs:355` (`maybe_countersign`) |
| `RecordVerdict<P>` and `VerifyOptions::allowing_expired` | `crates/roym_core/src/transaction.rs:571-615`; `crates/signed_record/src/verify.rs` |
| A per-service bundle format with per-section digests and `check_integrity`, no signature | `crates/roym_core/src/backup.rs` |
| `transaction.export` / `.import` over four sections | `crates/roym_transaction/src/app/backup.rs` |
| An encrypted identity backup (HKDF-SHA256 + AES-256-GCM under a random recovery key) | `crates/identity/src/backup.rs`; `roymctl identity export/import` |
| Catalog availability slots with a capacity and a content-derived `slot_id` | `crates/roym_catalog/src/app/availability.rs` |
| Hub placeholder templates for the four unproduced card types | `crates/roym_web/ui/src/cards/templates/{booking_progress,payment_request,payment_acknowledgement,fulfilment_receipt}.ts` |
| Parity scenarios up to **149**, two-substrate transaction e2e, Playwright cases up to **32** | `crates/roym_web/tests/dual_build_parity/`; `crates/substrate/tests/roym_transaction_e2e.rs`; `crates/substrate/tests/e2e/tests/roym-hub.spec.ts` |

Missing, and C8's to build:

1. **No state machine and no single writer.** Both nodes are symmetric today;
   nothing records `scheduled`, `in-progress`, `completed` or `cancelled`.
2. **No write fence.** The data layer is last-write-wins everywhere
   (`crates/data_db/src/sqlite/mutation.rs:50`, `do_put` is an upsert).
   Two concurrent bookings of one slot cannot be arbitrated by any guest
   code today. The backlog already says so and targets this slice
   (`deferred-backlog.md` §5, "Three read-modify-write sequences … unfenced",
   *"Pickup trigger: C8's booking flow needs a real per-listing/
   per-conversation fence anyway"*).
3. **No producer for four card types**, and `payment-request` is not in
   `RECORD_TYPES` (`crates/roym_core/src/record.rs:18`, backlog §12).
4. **No correction path** for a receipt (backlog §12, failure-matrix row 10).
5. **Bundles are unsigned** (backlog §3, *"A signed manifest and the
   top-level composition are C8's"*), and there is **no encrypted app-data
   backup** at all.
6. **`transaction.export` does not carry superseded record versions** — see
   `F8`. This is a C7 defect that R2's export row exposes.

---

## §1 Findings from reading the tree

Each finding is load-bearing for a decision in §2.

### F1 — `status.md`'s C7 section does not match the code

`status.md` lines 1776–1840 describe verbs that do not exist
(`request.create`, `request.latest`, `card.get`, `card.list`,
`card.forged`, `transaction.decline`, `agreement.pair`, `version`), say
"25 verbs" and "25 subcommands", and say parity scenarios end at 143. The
code has `request.set` / `.get` / `.list` / `.history` / `.verify`, the
same five for `quote` plus `quote.decline`, `agreement.accept` / `.get` /
`.list` / `.verify`, and `transaction.sync` / `.thread` / `.export` /
`.import` (`crates/roym_transaction/src/app.rs:164-190`); `roymctl roym
transaction` has about ten subcommands (`apps/roymctl/src/commands/roym/transaction.rs`);
parity scenarios run to **149** (`transaction_cards.rs:967`). **Trust the
code, not that section.** §17 owes a correction.

### F2 — every write goes through one writer thread, so a create-only check is atomic

`SqliteServiceStore` sends every write as a `DbCommand` to
`run_writer_loop` (`crates/data_db/src/sqlite/service_store.rs:30-145`),
and `do_batch_mutate` already runs its mutations inside one SQLite
transaction (`mutation.rs:192`). A "create these rows only if none exists"
command, checked and inserted inside one transaction on that thread, is
therefore atomic with no new locking. Nothing like it exists: `put` is an
upsert (`INSERT … ON CONFLICT(id) DO UPDATE`, `mutation.rs:78-86`), and
`batch-mutate` has no precondition.

### F3 — adding a *function* to `data-layer.wit` is backward compatible; changing a type is not

Three test components carry an older copy of `data-layer.wit` that is not a
symlink (`test-components/{miniapp-demo1-wasm,websocket-guest-test,saga-test}/wit/deps/data-layer/data-layer.wit`,
all with a different hash from `crates/wit_interfaces/wit/data-layer/data-layer.wit`)
and still deploy in the e2e suite. A component imports only the functions
its own WIT names, so a new function breaks nobody. Adding a case to
`mutation` or `data-layer-error` changes an existing type and would break
every component built against the old one. So the fence is a **new
function**, and it reports "already exists" through its **return value**,
not a new error case.

### F4 — `booking-progress` can be bound to the single writer with no new key

`principal::service` signs as the service's own key; `principal::delegated`
signs **with the same key**, certified by the person
(`crates/wit_interfaces/wit/signing/signing.wit:11-27`). So a quote's
`VerifiedRecord.signer_did` (`crates/signed_record/src/verify.rs:125`) is
exactly the provider's `transaction` service key, and a `booking-progress`
envelope signed under `principal::service` has that key as its `issuer`. A
consumer's node can check *"this progress came from the same transaction
service that signed the quote"* with data it already holds.
`RecordVerdict` does not expose `signer_did` today.

### F5 — slots exist, with a capacity, and nothing reads the capacity

`availability.set` stores `{slot_id, listing_id, start_secs, end_secs,
capacity}` with `slot_id = content_digest("slot_", {listing_id, start_secs,
end_secs})` (`crates/roym_catalog/src/app/availability.rs:1-70`). There is
no `availability.get`, and the slot-id function is private to `catalog`.
`BookingTerms.max_per_booking` (`crates/roym_core/src/listing.rs:59-64`) is
never read by anything.

### F6 — `transaction → catalog` is a legal new edge

`transaction` declares `depends_on = ["conversation"]`; `catalog` declares
`["profile"]`; `directory` declares `["catalog"]`
(`crates/roym_core/app/roym.toml`). Adding `catalog` to `transaction` makes
no cycle, so `SynAppManifest::validate`'s detector
(`crates/app_orchestration/src/models.rs:722`) accepts it.

### F7 — the native build persists no binding for C7's own `transaction → conversation` edge

`wire_roym_topology` saves bindings for `web → *`, `conversation → profile`,
`catalog → profile` and `directory → catalog`
(`crates/substrate/src/runtime/roym.rs:415-505`) — but not
`transaction → conversation`. Resolution still works because the node-wide
inventory resolves undeclared dependencies (backlog §3, *"A Roym service
can resolve a `depends_on` dependency it never declared"*). C8 adds both
missing bindings rather than a second gap.

### F8 — `transaction.export` drops superseded versions, so their cards import unverified

`export` writes the `requests`, `quotes`, `agreements` and `cards`
collections and says *"`request_history` and `quote_history` are not
exported … because every envelope in them is reachable from a pointer row"*
(`crates/roym_transaction/src/app/backup.rs:24-28`). That is false for a
superseded version: the pointer row holds only the **latest** envelope, and
`CardRow` holds a payload, not an envelope. On import,
`write_imported_cards` marks a card `verified = false` when its
`record_id` is not in the rebuilt history (`backup.rs:537-552`). So a card
for version 1 of a request that was later revised **verified before export
and does not verify after**. That breaks failure-matrix row 13 and R2's
export row. **Write the failing parity test first** (§11.3, scenario 150)
to confirm, then fix (§6.11).

### F9 — bundles carry per-section digests and no signature

`Bundle { manifest, sections }`, `check_integrity` compares counts and
digests (`crates/roym_core/src/backup.rs:40-120`). Anyone who can edit the
file can recompute the digests. The Hub's Backup text already promises *"a
single signed bundle that combines them comes later"*
(`crates/roym_web/ui/src/screens/backup.ts:2`).

### F10 — the identity backup's crypto is reusable as-is

`identity::backup::export` / `import` derive a key with
`HKDF-SHA256(salt, recovery_key, info)` and seal with AES-256-GCM, binding
a canonical AAD (`crates/identity/src/backup.rs:94-190`). The same
construction with a different `info` string and AAD seals app data under
the same recovery key, so a person keeps one key, not two.

### F11 — three Roym e2e files each carry their own `struct Node` and deploy helpers

`roym_conversation_e2e.rs:205`, `roym_transaction_e2e.rs:177` and
`roym_directory_e2e.rs:230` each define `Node`, `mint_masters`,
`substitute_plan`, `masters_by_id`, `certify_and_publish`,
`roym_artifacts_present`, `service_visibility`, `SIGNING_SERVICES`,
`wait_until` and friends. AGENTS.md: *"Do not write your own `struct Node`
/ `fn boot`; extend the shared one."* C8 needs two more Roym e2e files, so
it lifts these into `crates/substrate/tests/common/roym.rs` first (WO0b)
instead of writing copies four and five.

### F12 — size limits already bind

- `crates/roym_core/src/transaction.rs` is **769** lines (limit 800).
- `crates/roym_web/tests/dual_build_parity/{transaction,transaction_cards}.rs`
  and `crates/substrate/tests/roym_transaction_e2e.rs` are on
  `xtask/oversized-test-files.txt` (capped, may not grow).
- `crates/roym_web/ui/src/screens/messages.ts` is **842** lines and
  `roym-hub.spec.ts` is **1180** (TypeScript is not checked by the xtask,
  but AGENTS.md's 800-line rule is about production source files in
  general).
- `playwright.config.ts` lists spec files explicitly in `testMatch` and
  has `globalTimeout: 300_000`.

Every new piece of code in this plan goes into a new file for this reason.

### F13 — a card whose prerequisite has not arrived is refused forever

`file_quote_card` writes a refused `CardRow` when the request is not held
(`sync.rs:343-349`), and `classify_sync_message` skips any message that
already has a card row (`sync.rs:176-182`). So a card that arrives before
the card it depends on is never re-filed. C7 lives with this for quotes.
C8's cards depend on an agreement pair existing, and a consumer who
accepts and immediately records payment sends two cards whose order the
provider's node sees by sender timestamp. C8 must not lose the second one
(§6.9, `Deferred`).

### F14 — the automatic counter-half uses the provider's clock, not the acceptance time

`maybe_countersign` refuses when `now >= quote_expires_at_secs`
(`agreement_ops.rs:375`), while the consumer half is filed when *its own*
`issued_at_secs` is inside the window (`sync.rs:422-425`). With client-driven
sync (`D-C7-3`), a provider who opens the thread after the quote expires
silently never countersigns an acceptance made in time. **Not changed by
this plan** — see §18-G.

### F15 — the guest clock is wall time; windows are tested through signed data

`clock::now_secs()` is the wall clock on both builds; the parity harness
pins only the *signing* clock. Any time-window test in this plan therefore
puts the window in the past through signed data (a quote whose
`schedule` ended long ago), never by moving a clock.

### F16 — "no directory deployed anywhere" has meant "no directory source configured"

Every Roym deployment includes the `directory` service. The C5 and C7 e2e
files prove the directory-free path by asserting `directory.sources` is
empty on every node (`roym_transaction_e2e.rs:839-843`). §18-J keeps that
reading and says so.

### F17 — export tests that never enrol signing

These call `*.export` on a harness where the service may not be enrolled
for signing, and §2 `D-C8-15` makes export require signing:
`dual_build_parity/conversation.rs:295, 314, 336`,
`dual_build_parity/catalog.rs:271, 302, 327`,
`dual_build_parity/profile.rs:655, 1102`,
`dual_build_parity/transaction_cards.rs:363, 457`,
`crates/substrate/tests/roym_conversation_e2e.rs:828, 864`. Each must call
`enrol_signing` (parity: `fixtures.rs:52`) before exporting, or already
does — check each one.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C8-1** **[CONFIRM]** | **One payment per agreement.** A `payment-request` and both `payment-acknowledgement` halves carry exactly the agreement's `amount_minor` and `currency`. Deposits (a part payment plus a balance) are not in this release, the quote UI says so in one sentence, and a backlog row records it. | `task.md`'s open design point says C8 must close this before building the track, and recommends "one". `D-06C-13`'s track holds exactly one pair, so a second payment would need a second track or a track per instalment — a different state machine |
| **D-C8-2** **[CONFIRM]** | **Accepting a quote that names a slot *is* the booking request. The provider's node decides the booking when it files the consumer's agreement half.** It claims a seat under the fence (`D-C8-4`); on success it countersigns and writes `scheduled`; when the slot is full it **does not countersign** and writes a named `conflict`. A quote with no slot is `scheduled` on countersign. | The consumer's only signed statements reach the provider as cards (`D-C7-1`), and the card set is fixed at seven types (`D-06C-3`), none of which is a consumer booking request. The quote is the provider's offer of a specific slot, so accepting it is booking it. This keeps the transport, the card set and the "no wire surface" rule intact. The spec's scenario orders "accept" then "book" as two steps (§18-A) |
| **D-C8-3** | **A quote names a slot by an optional `slot_id` on `QuotePayload`, re-derivable from the quote's own `listing_id` and `terms.schedule`.** Capacity is read from `catalog` at decision time. `transaction` gains `depends_on = ["conversation", "catalog"]`. | The slot's time is already signed in `terms.schedule`; the id binds the provider's own availability row without trusting a free-form string (`F5`). Capacity is catalog state and can change; the decision must read the value in force when it is made. The edge is legal (`F6`) |
| **D-C8-4** | **The fence is a new `data-layer` host function, `create`: create several rows in one transaction, only if none of their ids exists; otherwise create nothing and return the first existing id.** Recorded as **Gap 9** in `task.md`. | `F2` makes it atomic for free; `F3` makes a new function safe. It is the smallest primitive that closes the booking race exactly and crash-safely, and it is the compare-and-set two older backlog rows are waiting for. "Not more substrate" (`task.md`) allows a genuinely missing capability named as a gap |
| **D-C8-5** | **Every booking decision and every transition is fenced by `create` in one `ledger` collection.** Row ids: `decision:<agreement>`, `seat:<slot_id>:<n>`, `step:<agreement>:<seq>`. A decision is one `create` of `[decision, seat, step 1]` (or `[decision, step 1]`); a transition is one `create` of `[step n+1]`. The `step` rows are the append-only audit log the spec requires. | One collection because `create` is per collection, and the decision must claim the seat and record the outcome in one transaction — otherwise a crash between the two leaks a seat or loses a decision |
| **D-C8-6** | **Idempotency keys are the records' own identities, not client tokens** (ADR-0023 §1: *"correctness comes from fences the caller already holds"*). A state-changing verb is keyed by `(record type, agreement, role)`: a second call returns the record already made with `"state": "already-recorded"`. A correction must name the record it replaces in `supersedes`, so a blind retry can never become a correction. A retried booking reaches the same final state because the decision is keyed by `decision:<agreement>`. | The spec says every state-changing request carries an idempotency key; a content-derived key is carried whether or not the client remembers to send one, and cannot be reused by mistake for a different action |
| **D-C8-7** | **`booking-progress` is signed under `principal::service` by the provider's `transaction` service, and a receiving node accepts it only when its `issuer` equals the `signer_did` of the quote that agreement answers.** It is not added to `RECORD_TYPES` (it is not evidence, `D-06C-12`). Every card carries the whole snapshot with a strictly increasing `seq`; a node keeps the highest `seq` it has seen. The consumer's Hub shows the writer's latest snapshot, labelled as the provider's system status. | `D-06C-12` says exactly this signer. `F4` makes the binding checkable with no new key. A full snapshot per card means a lost intermediate card loses nothing |
| **D-C8-8** | **The state machine.** `BookingState` = `scheduled` \| `in-progress` \| `completed` \| `cancelled` \| `conflict` \| `ended-unconfirmed`. Two tracks, `payment` and `fulfilment`, each `none` \| `claimed` \| `acknowledged` \| `unconfirmed`. Transitions in §4.4. It is a pure function in `roym_core::booking`, applied only on the provider's node. | `D-06C-13` fixes the tracks and the terminals; the spec's diagram fixes the main line. `conflict` and `ended-unconfirmed` are the two states the diagram lacks and the decisions require (§18-D) |
| **D-C8-9** | **`in-progress` is entered by `booking.start` (provider), or implicitly by the first payment or fulfilment event.** | The spec names the state and no trigger. An explicit verb lets a provider say "I have started"; the implicit edge means a flow that skips it still ends in the right place |
| **D-C8-10** | **`payment-request` carries no payee and no link.** It names the agreement, the parties and the amount. Every display of a payee reads it from the signed agreement this node holds. | Failure-matrix row 5 and R2's payment row. A payee field on the request would be a second place a payee could come from |
| **D-C8-11** | **`payment-acknowledgement` halves:** the provider's half (the payee confirms receipt) moves the payment track to `acknowledged`; the consumer's half (the payer says they paid) moves it to `claimed` and no further. The two halves must agree on agreement, parties, amount and currency, and nothing else (`D-06C-12`'s relaxed equality). Each carries the issuer's own `observed_at_secs`, `method` and `reference`. | `D-06C-13`, word for word |
| **D-C8-12** | **`fulfilment-receipt` halves are identical apart from `role`, and carry the agreed terms.** The consumer's half moves the fulfilment track to `acknowledged`; the provider's half moves it to `claimed`. | `D-06C-12` (identical apart from role) and `D-06C-13` (the against-interest half decides). Carrying the terms makes a receipt readable on its own, as `agreement-receipt` already is |
| **D-C8-13** **[CONFIRM]** | **Corrections exist only for `payment-acknowledgement`.** A correction is a new envelope from the same issuer, same role, same agreement, same amount and currency, with `supersedes` set to the version it replaces. Both are kept; the Hub shows the latest and marks it "corrected". A correction never moves a track. `agreement-receipt` and `fulfilment-receipt` have no correctable content by `D-06C-12`'s construction; a changed agreement is a new quote version, and a wrong fulfilment receipt is a matter for the dispute path. | Failure-matrix row 10 asks for a correction that references the old record and leaves both. The only receipt whose payload holds an issuer's own observations is the payment acknowledgement. Letting a correction move a track backwards would make the tracks non-monotonic and break `D-06C-13` |
| **D-C8-14** **[CONFIRM]** | **Only the provider's node can cancel (`booking.cancel`), and only while both tracks are `none`.** A consumer asks in the conversation. No consumer-signed cancellation exists in this release. | The single writer decides (spec, *Transaction state*). A consumer-signed cancellation needs a card type the fixed set does not have (`D-06C-3`). After money or work is claimed, cancelling is refund territory, which R2 excludes |
| **D-C8-15** **[CONFIRM]** | **Track windows: each track ends at its named terminal 30 days after the schedule's end, or 30 days after scheduling when the quote has no schedule.** The writer applies the time edge lazily, on any read or write of that booking. A track that has reached its terminal stays there; a late signed record is still stored and shown, but does not move the state. | `D-06C-13` requires each track to end so it cannot sit stuck, and gives no number. Lazy evaluation needs no scheduler (none exists for guests). Anchoring on signed data makes the edge testable (`F15`) |
| **D-C8-16** | **Every person-signed service signs its own bundle's manifest inside `*.export`** (a `bundle-manifest` record, person principal, subject = the bundle's `subject_did`). `*.import` refuses an unsigned bundle, a signature that does not verify, an issuer that is not the subject, or a signed manifest that differs from the one in the bundle. Applies to `profile`, `conversation`, `catalog`, `transaction`. **`directory` stays unsigned** — it signs nothing as the person, and its data is the SynOrg's (R3). | Closes the backlog's unsigned-bundle row (`F9`). Each service signs only what it itself produced, so there is no "sign whatever the client gives me" verb |
| **D-C8-17** **[CONFIRM]** | **The export stays one format, several bundles; the encrypted backup is one archive that composes them, built by `roymctl roym backup` on the client.** The archive holds the identity backup and every bundle, sealed under the identity backup's recovery key (`F10`). The Hub keeps its per-service signed downloads and gains no encryption in this slice (backlog row). | Answers `task.md`'s *"one format or several"*: one bundle format, one per service, and one composition. Sealing in `roymctl` needs no new substrate verb and no browser crypto that must match Rust byte for byte. The identity backup is already CLI-only (C4), so this matches its precedent |
| **D-C8-18** | **The transaction bundle gains the history sections (`F8`) and the new state:** `request_history`, `quote_history`, `ledger`, `bookings`, `progress`, `payments`, `fulfilments`. | R2's export row covers agreements and receipts; the provider's restore must bring back its seat claims and decisions, or a restored provider could double-book |
| **D-C8-19** **[CONFIRM]** | **The "durability suite" is defined here (§11.6).** It asserts that every agreement, booking, track, receipt, card and verification status on the restored node equals the original, and that the restored node can still write a new signed record and a new transition. **Continuing a live conversation with the same peer after restore is out of scope**: a clean node's services have new addresses, and a conversation id is derived from both addresses. Backlog row. | The spec names the suite and never defines it. Carrying service addresses across a restore means backing up per-service master keys, which is substrate and identity work outside this slice |
| **D-C8-20** | **`payment_timing` drives only the "what happens next" line** (`booking::next_step`). It never gates a transition. | `D-06C-13`, word for word |
| **D-C8-21** | **A card whose prerequisite is missing is deferred, not refused, for up to seven days after it was signed.** The watermark stays on it, so the next `sync` tries again. After seven days it is refused with the reason. | `F13`. The deferral reuses the watermark rule `D-C7-6` already has for the card cap, so it needs no new state |

---

## §3 Substrate — the `create` host function (Gap 9)

Land this first, as its own PR. It touches no Roym code.

### 3.1 WIT — `crates/wit_interfaces/wit/data-layer/data-layer.wit`

Add to `interface store`, directly after `batch-mutate`:

```wit
    /// Creates every row in `values` in one transaction, but only if none
    /// of their ids exists yet. Returns `none` when every row was created.
    /// When any id already exists -- including an id that appears twice in
    /// `values` -- creates nothing and returns that id.
    ///
    /// This is the one write in this interface that is not
    /// last-write-wins, so it is the fence a component uses when two
    /// concurrent calls must not both succeed. Same `MAX_BATCH_SIZE` limit
    /// as `batch-mutate`. Under a deployed FDAE policy (ADR-0017 §4) each
    /// row is authorized as a `put` that creates, inside the same
    /// transaction; the first denial rolls back every row.
    create: func(collection: string, values: list<record-write-value>)
        -> result<option<string>, data-layer-error>;
```

Every Roym crate and most test components symlink this file (`F3`); the
three with copies do not need it. `src/bindings.rs` in each Roym crate is
gitignored and regenerates on `mise run build:roym`.

### 3.2 `crates/data_db`

- `src/traits.rs` — add to `ServiceStore`, after `batch_mutate` (`:262`):

  ```rust
  /// See the WIT doc on `create`. Returns the first id that already
  /// existed, having written nothing, or `None` when every row was created.
  async fn create(
      &self,
      collection: &str,
      values: &[host_store::RecordWriteValue],
      creator_id: &str,
      auth: Option<&QueryAuth<'_>>,
  ) -> Result<Option<String>, host_store::DataLayerError>;
  ```

- `src/sqlite/service_store.rs`
  - `DbCommand` (`:30`) gains
    ```rust
    Create {
        collection: String,
        values: Vec<host_store::RecordWriteValue>,
        creator_id: String,
        sieve: Option<Box<CompiledSieve>>,
        resp: oneshot::Sender<Result<Option<String>, host_store::DataLayerError>>,
    },
    ```
  - `run_writer_loop` (`:145` area) gains the arm
    `DbCommand::Create { .. } => { let _ = resp.send(do_create(&mut conn, &collection, &values, &creator_id, sieve.as_deref())); }`
    — copy the `BatchMutate` arm's shape exactly.
  - `impl ServiceStore for SqliteServiceStore` (`:246`) gains `create`,
    a copy of `batch_mutate` (`:445`) that sends `DbCommand::Create`.
  - `impl ServiceStore for Arc<SqliteServiceStore>` (`:550`) gains the
    one-line delegate.

- `src/sqlite/mutation.rs` — new function beside `do_batch_mutate` (`:192`):

  ```rust
  pub(super) fn do_create(
      conn: &mut Connection,
      collection: &str,
      values: &[host_store::RecordWriteValue],
      creator_id: &str,
      sieve: Option<&CompiledSieve>,
  ) -> Result<Option<String>, host_store::DataLayerError> {
      validate_identifier(collection)?;
      if values.len() > MAX_BATCH_SIZE { /* same SchemaViolation as do_batch_mutate */ }
      let tx = conn.transaction().map_err(map_rusqlite_error)?;
      for value in values {
          if row_exists(&tx, collection, &value.id)? {
              // Dropping `tx` rolls back every row created above.
              return Ok(Some(value.id.clone()));
          }
          // The same arguments `do_batch_mutate` passes for a `Put` whose
          // row did not exist.
          authorize_and_mutate(&tx, collection, &value.id, sieve, false, true, false,
              |c| do_put(c, collection, value, creator_id))?;
      }
      tx.commit().map_err(map_rusqlite_error)?;
      Ok(None)
  }
  ```

  `do_put` inside the transaction, after `row_exists` said no, is an insert.
  A repeated id in `values` is caught by `row_exists` on its second
  appearance, because the first was already inserted in `tx`.

- Tests (`src/tests_crud.rs`, beside the `batch_mutate` tests):
  `create_inserts_every_row_when_none_exist`,
  `create_writes_nothing_and_names_the_first_existing_id`,
  `create_refuses_an_id_repeated_inside_one_call`,
  `create_of_an_empty_list_is_none`,
  `concurrent_creates_of_one_id_admit_exactly_one` (20 tasks on a shared
  `Arc<SqliteServiceStore>`, `tokio::join_all`, assert exactly one `None`).
  In `src/tests_fdae.rs`: `create_is_denied_and_rolled_back_when_one_row_is_unauthorized`,
  modelled on `batch_mutate_rolls_back_when_the_denied_mutation_is_a_put_create` (`:1352`).

### 3.3 `crates/sandbox_wasm` — `src/host_capabilities/capabilities_store.rs`

`impl store::Host for HostState` (`:103`) gains `create`, a copy of
`batch_mutate` (`:461`): refuse when `self.read_only`, compute
`creator_id`, `open_store`, `resolve_query_auth(.., DATA_LAYER_WRITE, Mode::Filter)`,
then `store.create(&collection, &values, &creator_id, query_auth.as_ref())`.

### 3.4 `crates/app_host` — the dual-build trait

- `src/lib.rs` `AppDataLayer` (`:141`) gains, after `batch_mutate`:
  ```rust
  fn create(
      &self,
      collection: String,
      values: Vec<RecordWriteValue>,
  ) -> impl Future<Output = Result<Option<String>, DataLayerError>> + Send;
  ```
- `src/guest.rs` `impl AppDataLayer for GuestHost` (`:53`): `dl::create(&collection, &values)`.
- `crates/app_host_native/src/host.rs` `impl AppDataLayer for NativeAppHost`
  (`:92`): copy `batch_mutate` (`:172`) — lock `state_mutex`, call
  `HostStore::create(&mut *state, collection, values.into_iter().map(<the converter put already uses>).collect())`,
  map the error with `convert::data_layer_error_out`.
- `crates/roym_core/src/signing.rs` test `TestHost` (`:320`): add
  `create`, `unimplemented!()`, as it does for `batch_mutate` (`:387`).

### 3.5 `crates/control_plane` — `src/synsvc_native/data.rs`

The native-dispatch verb table mirrors the interface function for function
(`task.md` Gap 1). Add `"create" => self.data_create(invocation, store.as_ref()).await`
beside `"batch-mutate"` (`:199`), and `data_create`, modelled on
`data_batch_mutate` (`:539`), taking `{ "collection": …, "values": [{ "id": …, "payload": … }] }`
in the same encoding `batch-mutate`'s `put` values use and answering
`{ "existing": <id or null> }`.

### 3.6 The dual-build shim proof

`test-components/dual-build-fixture` exercises `batch_mutate`
(`src/app.rs`, `src/app/dispatch.rs`); add one dispatch case that calls
`create` twice with one shared id and returns both results, and a scenario
in `crates/app_host_native/tests/dual_build_parity/` asserting both builds
answer `[null, "<id>"]`. This is the rule C1 set for every `AppHost` trait:
proven by the fixture built both ways.

---

## §4 `syneroym-roym-core` — the vocabulary and the state machine

### 4.1 `src/verdict.rs` — new, moved out of `transaction.rs`

Move `RecordVerdict<P>` and its `refused` constructor
(`transaction.rs:571-615`) here unchanged, and add one field:

```rust
/// The key that actually produced the signature. For a record signed
/// under a person's delegation this is the service key the person
/// certified -- the key a service-signed record from the same service
/// carries as its issuer.
#[serde(skip_serializing_if = "Option::is_none")]
pub signer_did: Option<String>,
```

Set it from `verified.signer_did` in every successful verdict (the three
in `transaction.rs` and the new ones below). In `transaction.rs`, replace
the moved block with `pub use crate::verdict::RecordVerdict;` so no C7
call site changes. This frees ~45 lines in `transaction.rs` (`F12`).

### 4.2 `src/record.rs`

```rust
pub const RECORD_TYPES: &[(&str, u32)] = &[
    /* existing ten */,
    ("payment-request", 1),
    ("bundle-manifest", 1),
];
pub const RECORD_PAYMENT_REQUEST: &str = "payment-request";
pub const RECORD_PAYMENT_ACKNOWLEDGEMENT: &str = "payment-acknowledgement";
pub const RECORD_FULFILMENT_RECEIPT: &str = "fulfilment-receipt";
pub const RECORD_BUNDLE_MANIFEST: &str = "bundle-manifest";
/// Signed by the provider's transaction service, not by a person, and
/// deliberately absent from `RECORD_TYPES`: it is derived state that
/// names its writer, never evidence of anything.
pub const RECORD_BOOKING_PROGRESS: &str = "booking-progress";
```

`test_known_record_types` covers the two new rows unchanged.

### 4.3 `src/listing.rs` — the slot id moves here

```rust
pub const SLOT_ID_PREFIX: &str = "slot_";

/// One definition for the slot id, shared by the catalog that stores the
/// slot and the quote that names it, so a consumer can re-derive it from
/// signed fields.
pub fn derive_slot_id(listing_id: &str, start_secs: u64, end_secs: u64)
    -> Result<String, EnvelopeError>
{
    content_digest(SLOT_ID_PREFIX,
        &json!({ "listing_id": listing_id, "start_secs": start_secs, "end_secs": end_secs }))
}
```

Byte-identical to `catalog`'s private `slot_id` (`availability.rs:7-15`), so
existing slot ids do not change. Unit test pins one known value computed
by the old function.

### 4.4 `src/booking.rs` — new (+ `src/booking/tests.rs`)

```rust
//! The booking a provider's transaction service writes, and the two
//! tracks that finish it. Written on the provider's node only; every
//! other node reads the writer's signed snapshots. Pure: no host calls.

pub const BOOKING_PROGRESS_VERSION: u32 = 1;
/// How long a track stays open after the work's window, before it ends
/// at its named terminal holding whatever claim exists.
pub const TRACK_WINDOW_SECS: u64 = 30 * 24 * 3600;
/// Seats one slot can hand out. Bounds the claim loop.
pub const MAX_SLOT_CAPACITY: u32 = 64;
pub const MAX_CANCEL_REASON_LEN: usize = 512;
/// How long a card may wait for the card it depends on before it is
/// refused rather than deferred.
pub const MAX_DEFER_SECS: u64 = 7 * 24 * 3600;

#[serde(rename_all = "kebab-case")]
pub enum BookingState { Scheduled, InProgress, Completed, Cancelled, Conflict, EndedUnconfirmed }

#[serde(rename_all = "kebab-case")]
pub enum TrackState { None, Claimed, Acknowledged, Unconfirmed }

#[serde(rename_all = "kebab-case")]
pub enum ConflictReason {
    /// Every seat of the slot the quote named is held by another booking.
    SlotTaken,
    /// The slot the quote named no longer exists in the provider's catalog.
    SlotUnavailable,
}

pub enum Track { Payment, Fulfilment }

/// What the writer is asked to apply. `now_secs` rides on every event so
/// the time edge is checked on every write, not only on reads.
pub enum BookingEvent {
    Start,
    Cancel { reason: String },
    Half { track: Track, role: Role },
    Tick,
}

/// One snapshot. Signed as the `booking-progress` payload, so every
/// field is an integer, a string, or an enum.
pub struct BookingProgressPayload {
    pub agreement: String,               // the quote's record_id (rec_…)
    pub conversation: String,
    pub consumer_did: String,
    pub provider_did: String,
    pub seq: u32,                        // >= 1, strictly increasing
    pub state: BookingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ConflictReason>,
    pub payment: TrackState,
    pub fulfilment: TrackState,
    pub track_window_ends_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_by: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
}

pub enum TransitionError {
    Terminal(BookingState),     // "booking is <state>"
    CannotStart(BookingState),
    CannotCancel,               // a track has left `none`
    ReasonTooLong,
}

/// The window end for a booking scheduled at `scheduled_at_secs`.
pub fn track_window_end(schedule: Option<&TimeWindow>, scheduled_at_secs: u64) -> u64 {
    schedule.map_or(scheduled_at_secs, |w| w.latest_secs).saturating_add(TRACK_WINDOW_SECS)
}

/// The first snapshot, seq 1: `scheduled`, or `conflict` with its reason.
pub fn open(agreement, conversation, consumer_did, provider_did,
            conflict: Option<ConflictReason>, window_end: u64) -> BookingProgressPayload;

/// `Ok(None)` means "no change" -- the idempotent answer to a repeated
/// event. The caller sets `seq` on the returned snapshot.
pub fn apply(s: &BookingProgressPayload, e: &BookingEvent, now_secs: u64)
    -> Result<Option<BookingProgressPayload>, TransitionError>;
```

`apply`, as pseudo-code (keep it a table of small helpers; no arm over ~10
lines):

```
fn apply(s, e, now):
    let mut n = s.clone()
    // The time edge runs first on every event, so a late Half cannot
    // revive a track whose window has passed.
    changed = close_expired_tracks(&mut n, now)        // helper below
    if is_terminal(n.state):
        return match e { Tick => Ok(changed.then_some(n)),
                         Cancel if n.state == Cancelled => Ok(None),
                         _ if changed => Ok(Some(n)),   // the edge itself is the change
                         _ => Err(Terminal(n.state)) }
    match e:
      Start  => match n.state { Scheduled => n.state = InProgress,
                                InProgress => return Ok(changed.then_some(n)),
                                other => Err(CannotStart(other)) }
      Cancel{reason} =>
          if reason.len() > MAX_CANCEL_REASON_LEN { Err(ReasonTooLong) }
          if n.payment != None || n.fulfilment != None { Err(CannotCancel) }
          n.state = Cancelled; n.cancelled_by = Some(Provider); n.cancel_reason = Some(reason)
      Half{track, role} =>
          let t = track_mut(&mut n, track)
          let against_interest = matches!((track, role),
                (Payment, Provider) | (Fulfilment, Consumer))
          *t = match (*t, against_interest) {
                (Unconfirmed, _)            => Unconfirmed,   // terminal
                (_, true)                   => Acknowledged,
                (None, false)               => Claimed,
                (other, false)              => other }
          if n.state == Scheduled { n.state = InProgress }
          if n.payment == Acknowledged && n.fulfilment == Acknowledged { n.state = Completed }
      Tick => {}
    finish_if_both_terminal(&mut n)   // both in {Acknowledged, Unconfirmed}, not both
                                      // Acknowledged -> EndedUnconfirmed
    Ok((n != *s).then_some(n))

fn close_expired_tracks(n, now) -> bool:
    if now < n.track_window_ends_at_secs || !matches!(n.state, Scheduled | InProgress) { return false }
    for t in [payment, fulfilment]: if t is None | Claimed { t = Unconfirmed }
    finish_if_both_terminal(n); true-if-anything-changed
```

`BookingProgressPayload::validate()`: `agreement` starts `rec_`; both DIDs
are `did:key` and differ; `conversation` non-empty; `seq >= 1`;
`conflict.is_some() == (state == Conflict)`; `cancelled_by` and
`cancel_reason` present only when `state == Cancelled`, reason within
`MAX_CANCEL_REASON_LEN`; `state == Completed` implies both tracks
`Acknowledged`.

```rust
/// Verifies a booking-progress envelope as a record. Binding it to the
/// writer is the caller's job: it must hold the quote and compare
/// `issuer` with that quote's verdict `signer_did`.
pub fn verify_booking_progress(envelope: &str, now_secs: u64)
    -> RecordVerdict<BookingProgressPayload>;
```
Same shape as `transaction::verify_agreement_receipt`: type
`booking-progress` v1, payload parses and validates, `subject == agreement`,
no expiry allowed.

What-happens-next, for the Hub and `roymctl` (never a gate, `D-C8-20`):

```rust
#[serde(rename_all = "kebab-case")]
pub enum NextStep { WaitForProvider, RequestPayment, PayOutsideRoym, ConfirmPaymentReceived,
                    MarkWorkComplete, ConfirmWorkComplete, Nothing }
pub fn next_step(s: &BookingProgressPayload, timing: PaymentTiming, me: Role) -> NextStep;
```
Rule: terminal → `Nothing`. For `me = Provider`: payment `none` and timing
`before-work` → `RequestPayment`; payment `claimed` → `ConfirmPaymentReceived`;
fulfilment `none` → `MarkWorkComplete`; else `Nothing`. For `me = Consumer`:
payment `none` → `PayOutsideRoym` (before-work first, after-work only once
fulfilment ≠ `none`); fulfilment `claimed` → `ConfirmWorkComplete`; else
`WaitForProvider`.

The Hub's fixed sentences live here as constants and are pinned by a Rust
test against a TypeScript file (§10.1), the way `card.rs` pins `registry.ts`:

```rust
pub const PAYMENT_NOTICE: &str = "This records what each side says about the payment. \
    Roym does not see the money move and cannot confirm that it did.";
pub const PROGRESS_NOTICE: &str = "This status comes from the provider's system. \
    It is not a signed statement by either person.";
pub const PAYMENT_CLAIMED: &str = "The customer says they paid.";
pub const PAYMENT_ACKNOWLEDGED: &str = "The provider confirms they received the payment.";
pub const FULFILMENT_CLAIMED: &str = "The provider says the work is done.";
pub const FULFILMENT_ACKNOWLEDGED: &str = "The customer confirms the work is done.";
pub const TRACK_UNCONFIRMED: &str = "No confirmation was recorded before the window closed.";
pub const ONE_PAYMENT_NOTICE: &str = "This quote is paid in one payment. \
    Deposits and part payments are not supported.";
```

No sentence anywhere says "verified", "paid" as a fact, or "complete" for
a single half.

Tests in `booking/tests.rs` (pure, fast): one test per row of the
transition table above, plus `a_repeated_event_is_no_change`,
`a_self_serving_half_never_acknowledges`, `completed_needs_both_acknowledged`,
`cancel_is_refused_once_a_track_moved`, `the_time_edge_runs_before_the_event`,
`a_terminal_track_ignores_a_late_half`, `one_acknowledged_one_unconfirmed_ends_unconfirmed`,
`validate_refuses_*` for each rule, `next_step_*` for each branch, and
`no_notice_says_verified` (asserts the substring `verif` appears in none
of the constants).

### 4.5 `src/payment.rs` — new (+ `src/payment/tests.rs`)

```rust
pub const PAYMENT_REQUEST_VERSION: u32 = 1;
pub const PAYMENT_ACKNOWLEDGEMENT_VERSION: u32 = 1;
pub const MAX_NOTE_LEN: usize = 512;
pub const MAX_REFERENCE_LEN: usize = 512;

/// The provider asking to be paid. No payee: the payee is the one the
/// signed agreement binds, and nothing else may name one.
pub struct PaymentRequestPayload {
    pub agreement: String, pub conversation: String,
    pub consumer_did: String, pub provider_did: String,
    pub currency: String, pub amount_minor: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One party's statement about a payment. The provider's half is the
/// payee confirming receipt; the consumer's half is the payer saying
/// they paid. Neither proves money moved.
pub struct PaymentAcknowledgementPayload {
    pub agreement: String, pub conversation: String,
    pub consumer_did: String, pub provider_did: String,
    pub role: Role,
    pub currency: String, pub amount_minor: i64,
    /// When the issuer says the payment happened. The issuer's own word.
    pub observed_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Whatever proof the issuer has, as text. Never fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}
```

Validation: `agreement` starts `rec_`; DIDs valid and different;
`conversation` non-empty; `money::currency_minor_exponent(currency)` is
`Some`; `amount_minor >= 0`; `observed_at_secs > 0`; `method` 1..=
`transaction::MAX_PAYMENT_METHOD_LEN`; `note`/`reference` within their
limits. New `PaymentError` enum (`thiserror`), one variant per rule.

Cross-checks the service runs against the agreement it holds:

```rust
/// One payment per agreement: the amount and currency are the agreed ones,
/// and a named method is one the terms list (when they list any).
pub fn matches_terms(currency: &str, amount_minor: i64, method: Option<&str>,
                     terms: &AgreedTerms) -> bool;
/// The relaxed pair equality `D-06C-12` allows for payment halves:
/// agreement, parties, amount and currency agree; roles differ.
pub fn acknowledgements_agree(a: &PaymentAcknowledgementPayload,
                              b: &PaymentAcknowledgementPayload) -> bool;
/// A correction may change only the issuer's own observations.
pub fn is_valid_correction(old: &PaymentAcknowledgementPayload,
                           new: &PaymentAcknowledgementPayload) -> bool;
   // same agreement, conversation, parties, role, currency, amount
```

Verifiers, same shape as `verify_agreement_receipt`, no expiry allowed,
`subject == agreement`:

```rust
pub fn verify_payment_request(envelope: &str, now: u64) -> RecordVerdict<PaymentRequestPayload>;
    // + issuer == provider_did
pub fn verify_payment_acknowledgement(envelope: &str, now: u64)
    -> RecordVerdict<PaymentAcknowledgementPayload>;
    // + issuer == the DID its role names
```

### 4.6 `src/fulfilment.rs` — new (+ `src/fulfilment/tests.rs`)

```rust
pub const FULFILMENT_RECEIPT_VERSION: u32 = 1;

/// Both halves are identical apart from `role`, and carry the agreed
/// terms so a receipt reads on its own.
pub struct FulfilmentReceiptPayload {
    pub agreement: String, pub conversation: String,
    pub consumer_did: String, pub provider_did: String,
    pub role: Role,
    pub terms: AgreedTerms,
}
pub fn verify_fulfilment_receipt(envelope: &str, now: u64)
    -> RecordVerdict<FulfilmentReceiptPayload>;
    // subject == agreement; issuer == role's DID; no expiry; terms.validate()
pub fn fulfilment_halves_agree(a, b) -> bool;  // everything equal except role
```

### 4.7 `src/transaction.rs` — the slot on a quote

`QuotePayload` gains:

```rust
/// The provider's availability slot this quote offers, when it offers one.
/// Re-derivable: `listing::derive_slot_id(listing_id, schedule.earliest_secs,
/// schedule.latest_secs)`. Accepting a quote that names one books it.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub slot_id: Option<String>,
```

`QuotePayload::validate` adds: when `slot_id` is `Some`, `listing_id` is
`Some`, `terms.schedule` is `Some` with `latest_secs > earliest_secs`, and
the id re-derives exactly (new error variants `SlotNeedsListingAndSchedule`,
`SlotIdMismatch`). `None` serializes to nothing, so every existing quote
envelope keeps its bytes. Nothing else in the file changes.

### 4.8 `src/backup.rs` and `src/signing.rs` — the signed manifest

`backup.rs`:

```rust
pub const BUNDLE_MANIFEST_VERSION: u32 = 1;

pub struct Bundle {
    pub manifest: BundleManifest,
    pub sections: BTreeMap<String, Vec<Value>>,
    /// A `bundle-manifest` envelope over `manifest`, signed by the person
    /// the bundle belongs to. Absent only for a service that signs nothing
    /// as the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_signature: Option<String>,
}

pub const SECTION_REQUEST_HISTORY: &str = "request_history";
pub const SECTION_QUOTE_HISTORY: &str = "quote_history";
pub const SECTION_LEDGER: &str = "ledger";
pub const SECTION_BOOKINGS: &str = "bookings";
pub const SECTION_PROGRESS: &str = "progress";
pub const SECTION_PAYMENTS: &str = "payments";
pub const SECTION_FULFILMENTS: &str = "fulfilments";

// new BundleError variants
Unsigned,
SignatureInvalid(String),
SignerNotSubject { issuer: String, subject: String },
SignedManifestDiffers,

impl Bundle {
    /// The manifest as the JSON object a `bundle-manifest` record signs.
    pub fn manifest_payload(&self) -> Result<String, BundleError>;
    /// Signature present, verifies, is a `bundle-manifest` v1, its issuer
    /// is `manifest.subject_did`, and its payload equals `manifest`
    /// (compared as canonical JSON values).
    pub fn verify_manifest_signature(&self, now_secs: u64) -> Result<(), BundleError>;
}
```

`BUNDLE_VERSION` stays `1`: pre-release, changed in place (no ladder).

`signing.rs` (already the home of `person_principal`):

```rust
/// Signs `bundle.manifest` as the person and stores the envelope in
/// `bundle.manifest_signature`. Refuses with the same `CertificateError`
/// words every signing verb uses when this service is not enrolled.
pub async fn sign_bundle<H: AppHost>(host: &H, bundle: &mut Bundle, now: u64)
    -> Result<(), CertificateError>;
```
Body: `person_principal(host, now)`, a host `RecordDraft { version:
BUNDLE_MANIFEST_VERSION, record_type: RECORD_BUNDLE_MANIFEST, subject:
manifest.subject_did, payload: manifest_payload(), expires_at_secs: None,
supersedes: None }` (`F7` of C7: the host draft's payload is a **string**),
`AppSigning::sign_record`, store. Import-side helper:

```rust
/// What every person-signed service's `import` runs before touching a row:
/// integrity, signature, and that the bundle is this node owner's.
pub fn check_signed_bundle(bundle: &Bundle, owner: &str, now: u64) -> Result<(), BundleError>;
```

### 4.9 `src/router.rs` and `src/lib.rs`

`ROUTES` gains, beside the other `TRANSACTION` rows:

```rust
("booking.", TRANSACTION, MethodAuth::Owner),
("payment.", TRANSACTION, MethodAuth::Owner),
("fulfilment.", TRANSACTION, MethodAuth::Owner),
```

`lib.rs`: `pub mod booking; pub mod fulfilment; pub mod payment; pub mod verdict;`.

---

## §5 `syneroym-roym-catalog`

- `src/app/availability.rs`: delete the private `slot_id` and
  `SLOT_ID_PREFIX`; call `listing::derive_slot_id` (§4.3).
- New verb `availability.get { slot_id }` → the slot row, or `null`. Add
  the arm beside `availability.list` (`src/app.rs:110`). Same
  `admit::require_internal` as every catalog verb; `transaction` reaches it
  as a sibling (`Local`).
- Parity scenario 5's wire-refusal list and `scenario_144`'s peer for
  catalog (if one enumerates catalog verbs — grep `availability.list` in
  `crates/roym_web/tests/`) gain `availability.get`.

---

## §6 `syneroym-roym-transaction` — the service

### 6.1 Files

| File | Holds |
|---|---|
| `src/app.rs` | new collection names, `ensure_collections` additions, the new `invoke` arms |
| `src/app/ledger.rs` (new) | `LEDGER` row types, `claim_decision`, `append_step`, `load_booking` (with crash recovery) |
| `src/app/booking_ops.rs` (new) | `decide_booking`, `transition`, `booking.get/list/start/cancel/history/verify` |
| `src/app/progress.rs` (new) | signing a snapshot as the service, sending it, filing a received one |
| `src/app/payment_ops.rs` (new) | `payment.request/acknowledge/get/verify` |
| `src/app/fulfilment_ops.rs` (new) | `fulfilment.sign/get/verify` |
| `src/app/sync.rs` → `sync.rs` + `src/app/sync/receipts.rs` (new) | the four new filing arms and `Deferred` |
| `src/app/backup.rs` → `backup.rs` + `src/app/backup/sections.rs` (new) | the seven new sections, export and import |
| `src/app/agreement_ops.rs` | the countersign hook (§6.6) |
| `src/app/quote_ops.rs` | `slot_id` on `quote.set` (§6.5) |
| `src/app/thread.rs` | payment-request enrichment (§6.10) |

Every new function stays under 100 lines; every match arm under ~10.

### 6.2 Collections

Add to `app.rs` beside `AGREEMENTS` and to `ensure_collections`:

| Const | Name | Indexes | Row |
|---|---|---|---|
| `LEDGER` | `ledger` | `kind` (string), `agreement` (string) | `LedgerRow` |
| `BOOKINGS` | `bookings` | `conversation`, `state`, `slot_id` (string) | `BookingRow` (provider node only) |
| `PROGRESS` | `progress` | `conversation` | `ProgressRow` (consumer node only) |
| `PAYMENTS` | `payments` | `conversation` | `PaymentsRow` |
| `FULFILMENTS` | `fulfilments` | `conversation` | `FulfilmentsRow` |

`SCHEMA_VERSION` goes `2 → 3`.

```rust
// ledger.rs
#[serde(rename_all = "kebab-case")]
pub(crate) enum LedgerKind { Decision, Seat, Step }

pub(crate) struct LedgerRow {
    pub(crate) kind: LedgerKind,
    pub(crate) agreement: String,
    #[opt] pub(crate) slot_id: Option<String>,
    #[opt] pub(crate) seat: Option<u32>,
    #[opt] pub(crate) step: Option<StepRow>,       // kind == Step
    pub(crate) created_at_secs: u64,
}
pub(crate) struct StepRow {
    pub(crate) seq: u32,
    pub(crate) event: String,                      // "open" | "start" | "cancel" | "payment-half" | …
    pub(crate) snapshot: BookingProgressPayload,
    pub(crate) envelope: String,                   // the signed booking-progress
    pub(crate) record_id: String,
    #[opt] pub(crate) message_id: Option<String>,  // set once the card is sent
}
pub(crate) fn decision_id(agreement: &str) -> String { format!("decision:{agreement}") }
pub(crate) fn seat_id(slot_id: &str, n: u32) -> String { format!("seat:{slot_id}:{n}") }
pub(crate) fn step_id(agreement: &str, seq: u32) -> String { format!("step:{agreement}:{seq}") }

// app.rs
pub(crate) struct BookingRow {
    pub(crate) agreement: String,
    pub(crate) conversation: String,                 // index copy
    pub(crate) state: BookingState,                  // index copy
    #[opt] pub(crate) slot_id: Option<String>,       // index copy
    #[opt] pub(crate) seat: Option<u32>,
    pub(crate) snapshot: BookingProgressPayload,
    pub(crate) progress_record_id: String,
    pub(crate) updated_at_secs: u64,
}
pub(crate) struct ProgressRow {
    pub(crate) agreement: String, pub(crate) conversation: String,
    pub(crate) seq: u32, pub(crate) snapshot: BookingProgressPayload,
    pub(crate) envelope: String, pub(crate) record_id: String,
    pub(crate) writer: String,                       // the service DID that signed it
    pub(crate) received_at_secs: u64,
}
pub(crate) struct PaymentsRow {
    pub(crate) agreement: String, pub(crate) conversation: String,
    #[opt] pub(crate) request: Option<ReceiptHalf>,
    #[default] pub(crate) consumer: Vec<ReceiptHalf>,   // versions, oldest first
    #[default] pub(crate) provider: Vec<ReceiptHalf>,
    pub(crate) updated_at_secs: u64,
}
pub(crate) struct FulfilmentsRow {
    pub(crate) agreement: String, pub(crate) conversation: String,
    #[opt] pub(crate) consumer: Option<ReceiptHalf>,
    #[opt] pub(crate) provider: Option<ReceiptHalf>,
    pub(crate) updated_at_secs: u64,
}
```

`#[opt]` above means `#[serde(default, skip_serializing_if = "Option::is_none")]`;
`#[default]` means `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
`ReceiptHalf` is `roym_core::transaction::ReceiptHalf`, reused.

### 6.3 Verb table

Every verb keeps `admit::require_internal` (unchanged top of `invoke`). No
new wire surface.

| Verb | Params | Who | Answers |
|---|---|---|---|
| `booking.get` | `agreement` | both | the booking view (§6.8) |
| `booking.list` | `conversation?`, `state?`, `limit`, `offset` | both | views, newest first |
| `booking.start` | `agreement` | provider | the view |
| `booking.cancel` | `agreement`, `reason` | provider | the view |
| `booking.history` | `agreement` | both | every `step` (provider) or every filed progress card (consumer), oldest first |
| `booking.verify` | `envelope` | both | `RecordVerdict<BookingProgressPayload>` |
| `payment.request` | `agreement`, `note?` | provider | `{record_id, message_id, state}` |
| `payment.acknowledge` | `agreement`, `observed_at_secs?`, `method?`, `reference?`, `supersedes?` | both | `{record_id, role, message_id, state}` |
| `payment.get` | `agreement` | both | request, both version lists, track |
| `payment.verify` | `envelope` | both | the verdict for whichever of the two payment types the envelope is |
| `fulfilment.sign` | `agreement` | both | `{record_id, role, message_id, state}` |
| `fulfilment.get` | `agreement` | both | both halves, track |
| `fulfilment.verify` | `envelope` | both | the verdict |

`state` on a write is the conversation send state, or `"already-recorded"`
for an idempotent repeat (`D-C8-6`). Refusals use `Response::invalid_params`
with these exact words, so the Hub and `roymctl` can match them:
`no-such-agreement`, `agreement-incomplete`, `not-a-party`,
`provider-only`, `no-booking`, `booking-<state>` (e.g. `booking-conflict`),
`cannot-cancel`, `nothing-to-pay`, `amount-mismatch`, `method-not-in-terms`,
`not-the-current-version`, `signing-not-enrolled` (existing),
and the `TransitionError` words.

The `receipt.ping` arm stays; no `receipt.*` verb is added (§18-L).

### 6.4 The booking decision — `booking_ops::decide_booking`

Called on the **provider's** node only, when a pair can complete (§6.6).
Signature:

```rust
pub(crate) async fn decide_booking<H: AppHost>(
    host: &H,
    agreement: &AgreementRow,
    quote: &QuotePayload,
    now: u64,
) -> Result<BookingRow, String>
```

```
if let Some(row) = ledger::load_booking(host, &agreement.quote_record_id).await? {
    return Ok(row)                                     // decided before: same answer
}
let window_end = booking::track_window_end(quote.terms.schedule.as_ref(), now)
let scheduled = booking::open(.., conflict: None, window_end)
let scheduled_env = progress::sign(host, &scheduled).await?      // principal::service

let outcome = match &quote.slot_id {
    None => ledger::claim_decision(host, q, None, &scheduled, &scheduled_env, now).await?,
    Some(slot) => claim_seat(host, q, slot, &scheduled, &scheduled_env, now).await?,
}
match outcome {
    Claim::Won(row)          => { put BOOKINGS; progress::send(host, &row).await; Ok(row) }
    Claim::AlreadyDecided    => ledger::load_booking(host, q).await?.ok_or("decision without steps")
    Claim::NoSeat(reason)    => {
        let conflict = booking::open(.., conflict: Some(reason), window_end)
        let env = progress::sign(host, &conflict).await?
        match ledger::claim_decision(host, q, None, &conflict, &env, now).await? {
            Claim::Won(row)       => { put BOOKINGS; progress::send(host, &row).await; Ok(row) }
            _                     => load_booking(..)
        }
    }
}

async fn claim_seat(host, q, slot, scheduled, env, now) -> Result<Claim, String>:
    let slot_row = catalog_call(host, "availability.get", {slot_id}).await?   // dependency call
    let Some(slot_row) = slot_row else { return Ok(Claim::NoSeat(SlotUnavailable)) }
    let capacity = min(slot_row.capacity, MAX_SLOT_CAPACITY)
    for n in 1..=capacity {
        let rows = [ decision row (seat n), seat row n, step row 1 (scheduled, env) ]
        match AppDataLayer::create(host, LEDGER, rows).await? {
            None                                   => return Ok(Claim::Won(booking_row(.., Some(n)))),
            Some(id) if id == decision_id(q)       => return Ok(Claim::AlreadyDecided),
            Some(id) if id == step_id(q, 1)        => return Ok(Claim::AlreadyDecided),
            Some(_seat_taken)                      => continue,
        }
    }
    Ok(Claim::NoSeat(SlotTaken))
```

`ledger::claim_decision` is the no-slot form: one `create` of
`[decision, step 1]`, same three-way answer. `catalog_call` is a copy of
`conversation_call` (`app.rs:427`) targeting
`CallTarget::Dependency(services::CATALOG.name)`; lift both into one
`sibling_call(host, service, method, params)` helper rather than writing a
second copy.

**Why this is exact.** The decision row and the seat row are created in one
transaction, so a seat is never held without a recorded outcome, and an
outcome never exists without its seat. Two deciders for the same agreement
cannot both win (`decision:` collides). Two agreements for the last seat
cannot both win (`seat:` collides). A crash after `create` and before the
`BOOKINGS` put is repaired by `load_booking` (below).

`ledger::load_booking(host, q)`:

```
if let Some(row) = get BOOKINGS q { roll_forward(host, row) }     // steps past row.snapshot.seq
else if get LEDGER decision_id(q) is Some {
    rebuild from the highest step: get step_id(q, 1), (q, 2), … until None;
    put BOOKINGS from the last step; return it
} else { Ok(None) }
```

`roll_forward` reads `step_id(q, row.seq + 1)` until `None` and puts the
latest. Both are bounded by the number of steps a booking can have (at most
~8 by the transition table).

### 6.5 `quote.set` — naming a slot

`QuoteSetParams` (`quote_ops.rs:29`) gains `slot_id: Option<String>`. When
present:

```
require params.listing_id.is_some()                     // else invalid_params
let slot = catalog_call("availability.get", {slot_id}) → None => "no-such-slot"
require slot.listing_id == params.listing_id            // else "slot-not-in-listing"
set terms.schedule = Some(TimeWindow { earliest_secs: slot.start_secs, latest_secs: slot.end_secs })
    // a caller-supplied schedule that differs is refused: "schedule-differs-from-slot"
payload.slot_id = Some(slot_id)
```

then `payload.validate()` (§4.7) re-derives the id. No capacity check here —
a quote is an offer, and the decision checks capacity when it is accepted.

### 6.6 Where the decision runs — `agreement_ops.rs`

1. **Automatic path.** `sync::file_agreement_receipt_card` calls
   `maybe_countersign` at its end (`sync.rs:457`). Change the order inside
   `maybe_countersign` (`agreement_ops.rs:355`): after its existing
   early-returns (provider half present, consumer half absent, not the
   provider, quote expired) and **before** signing:

   ```
   let booking = decide_booking(host, row, q_payload, now).await?
   if booking.state == BookingState::Conflict { return Ok(false) }   // no counter-half
   ... existing signing and sending ...
   ```
   Doc comment on `maybe_countersign`: add *"refused, and left to the person,
   when the slot the quote names is already full"* to the refusal list.

2. **Manual provider path.** In `agreement_accept` (`agreement_ops.rs:34`),
   after the half is stored: if `role == Provider` and `row.consumer` is
   `Some`, call `decide_booking`. If `role == Provider` and there is no
   consumer half, do nothing — the decision waits for the consumer.
   A pair that completes by this path and meets a full slot is
   `pair: complete` with `booking: conflict`; §18-E states it.

3. **Consumer node.** Never decides. `decide_booking` is not reachable
   when `owner != agreement.provider_did` — assert it at the top and return
   `"provider-only"`.

The response of `agreement.accept` gains `"booking": <view or null>`.

### 6.7 Transitions — `booking_ops::transition`

```rust
pub(crate) async fn transition<H: AppHost>(host: &H, q: &str, event: BookingEvent, now: u64)
    -> Result<BookingRow, Response>
```

```
for _attempt in 0..3 {
    let row = ledger::load_booking(host, q).await? .ok_or("no-booking")
    let next = match booking::apply(&row.snapshot, &event, now) {
        Ok(None)     => { progress::resend_if_unsent(host, &row).await; return Ok(row) }
        Ok(Some(n))  => n with seq = row.snapshot.seq + 1,
        Err(e)       => return Err(invalid_params(e.to_string())),
    }
    let env = progress::sign(host, &next).await?
    match AppDataLayer::create(host, LEDGER, [step row next.seq]).await? {
        Some(_) => continue,                 // another writer took this seq; re-read
        None    => {}
    }
    if next.state == Cancelled && row.seat.is_some() {
        AppDataLayer::delete(host, LEDGER, seat_id(slot, seat)).await   // free the seat
    }
    let new_row = BookingRow { snapshot: next, .. }
    put BOOKINGS; progress::send(host, &new_row).await
    return Ok(new_row)
}
Err(internal_error("booking is busy; try again"))
```

A freed seat is a delete, not part of the `create`. If the process dies
between the step and the delete, `load_booking` sees `Cancelled` with a
seat still held and deletes it (add that one check to `roll_forward`).

Callers:

| Caller | Event |
|---|---|
| `booking.start` | `Start` |
| `booking.cancel` | `Cancel { reason }` |
| `payment.acknowledge` on the provider's node, first version only | `Half { Payment, Provider }` |
| filing a consumer's payment half on the provider's node, first version only | `Half { Payment, Consumer }` |
| `fulfilment.sign` on the provider's node | `Half { Fulfilment, Provider }` |
| filing a consumer's fulfilment half on the provider's node | `Half { Fulfilment, Consumer }` |
| `booking.get` / `booking.list` on the provider's node | `Tick` (only when `now >= track_window_ends_at_secs` and the state is not terminal, so a plain read writes nothing) |

A `Half` event for an agreement with no booking row (a pair completed by
the manual path before any decision, or a conflict) records the half and
skips the transition; the response says `"booking": null`.

### 6.8 `progress.rs`

```rust
/// Signs a snapshot as this transaction service. No person certificate is
/// involved: the writer is the service, and the record says so.
pub(crate) async fn sign<H: AppHost>(host: &H, s: &BookingProgressPayload) -> Result<(String, String), String>;
    // RecordDraft { version: BOOKING_PROGRESS_VERSION, record_type: RECORD_BOOKING_PROGRESS,
    //   subject: s.agreement, payload: to_string(s), expires_at_secs: None, supersedes: None }
    // AppSigning::sign_record(host, draft, Principal::Service) -> (envelope, record_id)

/// Sends the latest step as a card and records its message id on the step.
pub(crate) async fn send<H: AppHost>(host: &H, row: &BookingRow);
    // send_card_and_file(conversation, "booking-progress", 1, envelope, now, None);
    // on a message id: put the step row back with message_id set

/// Re-sends the latest step when it has no message id (a crash between
/// the step and the send). Bounded: the latest step only.
pub(crate) async fn resend_if_unsent<H: AppHost>(host: &H, row: &BookingRow);
```

The booking view `booking.get` returns on both nodes:

```json
{ "agreement": "...", "writer": "self" | "peer" | null,
  "seq": 3, "state": "in-progress", "conflict": null,
  "payment": "claimed", "fulfilment": "none",
  "track_window_ends_at_secs": 0, "cancel_reason": null,
  "progress_record_id": "rec_...",
  "next": "confirm-payment-received",
  "payment_timing": "before-work",
  "pair": { "state": "complete" } }
```

On the provider's node the numbers come from `BOOKINGS`; on the consumer's
node from `PROGRESS`, with `"writer": "peer"`; with neither, `"writer":
null` and `"state": null` (the Hub says *"Waiting for the provider's
system"*). `next` is `booking::next_step` for this node's own role.

### 6.9 `sync` — the four new arms and `Deferred`

`sync.rs`'s `file_incoming_card` match (`:248-271`) keeps its three arms
and replaces the last one:

```rust
"booking-progress"        => receipts::file_progress_card(..).await,
"payment-request"         => receipts::file_payment_request_card(..).await,
"payment-acknowledgement" => receipts::file_payment_ack_card(..).await,
"fulfilment-receipt"      => receipts::file_fulfilment_card(..).await,
_ => refuse_card(host, &msg_id, row, "a known card type this build does not file").await,
```

The last arm is now unreachable for the seven known types; it stays as the
honest answer for a type added to `CARD_TYPES` later. This closes the
backlog §12 row *"A card of a known type with no producer files `known:
true, verified: false`"*.

`FileCardResult` gains `deferred: bool`; `SyncOutcome` gains `Deferred`,
handled exactly like `Declined` (the watermark stays on the first one,
`sync.rs:97-99`), and the response gains a `"deferred"` count. A filing
function returns `Deferred` only when its prerequisite is missing **and**
the envelope's `issued_at_secs + MAX_DEFER_SECS > now`; otherwise it
refuses with the prerequisite's reason (`D-C8-21`). A deferred card writes
**no** `CardRow`, so the next `sync` sees it again.

Each filer, as pseudo-code (all in `sync/receipts.rs`):

```
file_progress_card(env, conversation, now, owner):
    v = booking::verify_booking_progress(env, now); !v.verified -> refuse(reason)
    p = payload; p.conversation != conversation -> refuse("card names another conversation")
    owner != p.consumer_did -> refuse("progress for an agreement this node is not the customer of")
    quote_env = get QUOTE_HISTORY p.agreement -> None => defer-or-refuse("names a quote this node does not hold")
    qv = verify_quote(quote_env, now)
    v.issuer != qv.signer_did -> refuse("not signed by the service that signed the quote")
    p.provider_did != qv.issuer || p.consumer_did != q.consumer_did -> refuse("names the wrong parties")
    upsert PROGRESS if p.seq > stored.seq (keep the stored one otherwise)
    file card row verified (data = p)

file_payment_request_card:
    v = verify_payment_request; refuse on failure
    agr = get AGREEMENTS p.agreement -> None => defer-or-refuse("names an agreement this node does not hold")
    pair not complete -> defer-or-refuse("agreement not complete on this node")
    p.provider_did != agr.provider_did || p.consumer_did != agr.consumer_did -> refuse("names the wrong parties")
    !payment::matches_terms(p.currency, p.amount_minor, None, &agr.terms) -> refuse("amount-mismatch")
    payments.request is Some and differs -> keep the first, file this card verified but
        with reason "a second payment request for one agreement"   // shown, not acted on
    else store as payments.request; file card verified

file_payment_ack_card:
    v = verify_payment_acknowledgement; refuse on failure
    agr prerequisite as above (defer-or-refuse)
    parties check; matches_terms(currency, amount, method, &agr.terms) -> else refuse
    versions = payments.<role>
    match envelope.supersedes {
        None if versions.is_empty() => push; first = true
        None                        => refuse("a second first version for this party")
        Some(prev) if versions.last().record_id == prev
             && payment::is_valid_correction(last payload, p) => push; first = false
        Some(_)                     => defer-or-refuse("corrects a version this node does not hold")
    }
    file card verified
    if first && owner == agr.provider_did && p.role == Consumer {
        transition(q, Half{Payment, Consumer}, now)       // ignore "no-booking"
    }

file_fulfilment_card:
    v = verify_fulfilment_receipt; refuse on failure
    agr prerequisite; parties; p.terms != agr.terms -> refuse("terms differ from the agreement")
    fulfilments.<role> is Some(existing) -> existing.record_id == v.record_id ? skip : refuse("a second receipt from this party")
    store; file card verified
    if owner == agr.provider_did && p.role == Consumer { transition(q, Half{Fulfilment, Consumer}, now) }
```

### 6.10 The writing verbs

`payment.request` (`payment_ops.rs`):

```
(principal, owner) = resolve_principal_and_owner
agr = AGREEMENTS[agreement] -> "no-such-agreement"; pair complete -> else "agreement-incomplete"
owner != agr.provider_did -> "provider-only"
booking = load_booking -> None => "no-booking"; state not in {Scheduled, InProgress} -> "booking-<state>"
booking.payment == Acknowledged -> "booking-payment-acknowledged"
agr.terms.amount_minor == 0 -> "nothing-to-pay"
payments.request is Some -> return it with state "already-recorded"
payload = PaymentRequestPayload { .. amount/currency from agr.terms .. }; validate
sign (person principal, subject = agreement); store; send_card_and_file("payment-request", 1, ..)
```

`payment.acknowledge`:

```
(principal, owner); agr + pair complete; role = party(owner) -> else "not-a-party"
this node knows the booking is Conflict or Cancelled (BOOKINGS or PROGRESS) -> "booking-<state>"
versions = payments.<role>
match params.supersedes {
    None if !versions.is_empty() => return last with "already-recorded"
    None => first version
    Some(prev) if versions.last().record_id == prev => correction
    Some(_) => "not-the-current-version"
}
payload: currency/amount from terms; observed_at_secs = params or now; method; reference
!matches_terms(..) -> "method-not-in-terms"; correction && !is_valid_correction -> invalid_params
sign with supersedes = params.supersedes; push; send card
if first && owner == agr.provider_did -> transition(Half{Payment, Provider})
if first && owner == agr.consumer_did -> nothing locally (the provider's node transitions on filing)
```

`fulfilment.sign`: same frame; idempotent on an existing half; payload
with `terms = agr.terms`; on the provider's node `transition(Half{Fulfilment, Provider})`.

`booking.start` / `booking.cancel`: `owner == provider_did` else
`"provider-only"`, then `transition`.

`*.get`: read the rows, compute `pair_state` and the view.
`*.verify`: the pure verifier on `params.envelope`, like `verify_verb`
(`app.rs:230`) — extend `RecordKind` with `BookingProgress`,
`PaymentRequest`, `PaymentAcknowledgement`, `FulfilmentReceipt`;
`payment.verify` picks by the envelope's `record_type`.

### 6.11 `thread` enrichment

`thread.rs` already enriches quote rows. `CardRow` gains two optional
fields, set only for `payment-request` rows, from the `AGREEMENTS` row this
node holds:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub(crate) agreement_payee: Option<String>,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub(crate) agreement_payment_methods: Vec<String>,
```

`data` stays the signed payload only; the payee rides beside it, labelled
by its own name as coming from the agreement (`D-C8-10`).

### 6.12 Export and import (`backup.rs` + `backup/sections.rs`)

Export adds seven sections: `request_history` and `quote_history` (each
row `{ "id": record_id, "payload": { "envelope": "<json>" } }`), and
`ledger`, `bookings`, `progress`, `payments`, `fulfilments` via `collect`.
Then `signing::sign_bundle(host, &mut bundle, now)`; on
`CertificateError::NotEnrolled` answer `signing-not-enrolled`.

Import, in this order, refusing the whole import on the first failure
(unchanged C7 semantics — only verified records are ever stored as
records; a refused card is a `CardRow` and imports as one):

1. `signing::check_signed_bundle(&bundle, &owner, now)` (replaces the
   inline integrity + subject checks at `backup.rs:92-104`).
2. Schema version check (unchanged).
3. `request_history` / `quote_history`: every envelope verifies with the
   matching `verify_*`, and its `record_id` equals the row id; write with
   `put_bytes` into `REQUEST_HISTORY` / `QUOTE_HISTORY`; add each to
   `imported_history` so `write_imported_cards` re-verifies cards for
   superseded versions (`F8`).
4. Existing requests / quotes / agreements steps (unchanged).
5. `payments` / `fulfilments`: every half verifies, parties and amounts
   match the imported agreement, correction chains link. Add each half's
   `record_id` to `imported_history` with its kind.
6. `ledger`: every `step` envelope verifies as `booking-progress` and its
   `issuer` equals the imported quote's `signer_did`; `decision` and `seat`
   rows are copied as they are.
7. `bookings`: equals the highest imported step for its agreement
   (rebuilt from the ledger rather than trusted).
8. `progress`: each verifies and binds exactly as `file_progress_card`
   does.
9. Cards (unchanged function; `write_imported_cards` gains the four new
   kinds in its `match kind` at `backup.rs:539-544`).

`import` writes with `put`, not `create`: a restore onto a clean node has
nothing to collide with, and a re-import of the same bundle rewrites
identical rows.

---

## §7 The other three person-signed services

`profile`, `conversation`, `catalog` — in each `src/app/backup.rs`:

- `export`: after building `bundle`, `signing::sign_bundle(host, &mut bundle, now)`;
  map `NotEnrolled` to `signing-not-enrolled`, the same words as every
  signing verb.
- `import`: replace the service's own `check_integrity()` + `subject_did`
  comparison with `signing::check_signed_bundle(&bundle, &owner, now)`.

Files: `crates/roym_profile/src/app/backup.rs:67`,
`crates/roym_conversation/src/app/backup.rs:45`,
`crates/roym_catalog/src/app/backup.rs:29` (and each file's `import`).
`crates/roym_directory/src/app/backup.rs` is **not** changed (`D-C8-16`).

Call sites that export without enrolling are listed in `F17`; each gets an
`enrol_signing(&h, "<service>")` line before its first export.

---

## §8 The manifest and the native bindings

`crates/roym_core/app/roym.toml`, `[services.transaction]`:

```toml
# A card is sent over a conversation and read back from it, and a quote
# that names a slot reads that slot from the catalog -- at quote time to
# bind it, and at acceptance time to read its capacity.
depends_on = ["conversation", "catalog"]
```

`crates/substrate/src/runtime/roym.rs`, `wire_roym_topology`: save a
binding from `transaction` to `conversation` (missing since C7, `F7`) and
from `transaction` to `catalog`, in the same shape as the `directory →
catalog` block (`:483-497`). Lift the four near-identical
`TopologyEntry` blocks into one `singleton_entry(name)` helper while there
(the function is at its limit).

---

## §9 `roymctl`

New file `apps/roymctl/src/commands/roym/booking.rs` (the existing
`transaction.rs` is 510 lines). `TransactionCommands` gains three nested
groups, each a thin JSON-RPC call through the gateway exactly like the C7
subcommands:

```
roymctl roym transaction booking   get|list|start|cancel|history  --agreement <rec_…> [--reason …]
roymctl roym transaction payment   request|ack|get                --agreement <rec_…>
                                   [--observed-at <unix secs>] [--method …] [--reference …] [--supersedes <rec_…>]
roymctl roym transaction fulfilment sign|get                      --agreement <rec_…>
roymctl roym transaction quote … --slot <slot_…>                   (new flag on the existing quote command)
```

`payment get` and `booking get` print `PAYMENT_NOTICE` / `PROGRESS_NOTICE`
above the data, word for word from `roym_core::booking`.

New file `apps/roymctl/src/commands/roym/backup.rs`, `RoymCommands::Backup`:

```
roymctl roym backup create          --master <key file> --out <archive.json>
roymctl roym backup restore-identity --in <archive.json> --recovery-key <key> --out <key file>
roymctl roym backup restore-data    --in <archive.json> --recovery-key <key>
```

- `create`: load the master; `generate_recovery_key`; `identity::backup::export`;
  call `profile.export`, `conversation.export`, `catalog.export`,
  `transaction.export`, `directory.export`; seal the JSON
  `{ "bundles": { "<service>": <bundle> } }` with `identity::backup::seal`
  (below); write `RoymArchive`; print the recovery key once, with the same
  wording `identity export` uses.
- `restore-identity`: open the identity part only — the same code path as
  `identity import`, so a person can bring up the substrate as themselves
  before any app data exists.
- `restore-data`: open the data part; call each `*.import` in the order
  profile, catalog, conversation, transaction, directory; print each
  result. Refuses up front if `transaction.signing-status` says the
  installation is not enrolled. On success it prints, word for word:
  *"Your history and records are restored and can be read. Conversations
  from before the restore cannot continue: this installation has new
  addresses. Share your new address with the people you talk to, and
  start new conversations with them."* (`deferred-backlog.md` §5, the
  IMPORTANT row). Keep the sentence as a `const` in `backup.rs` and assert
  it in the CLI test.
- The archive deliberately does **not** include the member master keys:
  restoring them without a re-pin mechanism blocks even a fresh start
  (§18-S).

`crates/identity/src/backup.rs` gains the general form of what it already
does (refactor `export`/`import` to call it, behaviour unchanged):

```rust
pub struct SealedBlob { pub kdf: String, pub cipher: String,
                        pub salt_z32: String, pub nonce_z32: String, pub ciphertext_z32: String }
/// HKDF-SHA256(salt, recovery_key, info) -> AES-256-GCM over `plaintext`
/// with `aad`. `info` separates uses of one recovery key.
pub fn seal(plaintext: &[u8], recovery_key: &[u8; 32], info: &[u8], aad: &[u8])
    -> Result<SealedBlob, BackupError>;
pub fn open(blob: &SealedBlob, recovery_key: &[u8; 32], info: &[u8], aad: &[u8])
    -> Result<Zeroizing<Vec<u8>>, BackupError>;
```

The archive type lives in `roymctl` (it is a client file format, not a
product record):

```rust
pub const ARCHIVE_VERSION: u32 = 1;
const ARCHIVE_INFO: &[u8] = b"syneroym-roym-archive-v1";
pub struct RoymArchive {
    pub archive_version: u32,
    pub subject_did: String,        // bound into the AAD
    pub produced_at_secs: u64,
    pub identity: IdentityBackup,   // sealed under the same recovery key
    pub data: SealedBlob,           // aad = canonical {archive_version, subject_did, produced_at_secs}
}
```

Tests: `apps/roymctl/tests/` gains a CLI round trip (create → restore-
identity → the key file equals the original; a flipped ciphertext byte →
`could not decrypt`), and a unit test that `seal`/`open` round-trips and
refuses a wrong `info`.

---

## §10 The Hub (`crates/roym_web/ui`)

### 10.1 Templates and wording

- `src/cards/wording.ts` (new): the eight sentences of §4.4 as exported
  constants, and a Rust test in `crates/roym_core/src/booking/tests.rs`,
  `the_ui_wording_matches_this_crate`, that reads the file and compares
  each literal (modelled on `card::tests::the_ui_card_registry_matches_this_crate`).
- `src/cards/templates/booking_progress.ts`: state, the two tracks in the
  words above, conflict reason (*"Another booking took this slot first."*
  / *"The provider removed this slot."*), cancellation reason as text, and
  `PROGRESS_NOTICE` in every rendering.
- `payment_request.ts`: amount with `formatMinor`, the **agreement's**
  payee from the card row's `agreement_payee`, never from `data`; a payee
  that is an `https:` URL renders through `cards/link.ts` (shown in full,
  followed only on a click); `PAYMENT_NOTICE`.
- `payment_acknowledgement.ts`: who says what (`PAYMENT_CLAIMED` for the
  consumer's half, `PAYMENT_ACKNOWLEDGED` for the provider's), observed
  date, method, reference as text, "corrected" when `supersedes` is set,
  `PAYMENT_NOTICE`.
- `fulfilment_receipt.ts`: who says what, the scope from the terms.
- `templates.test.ts`: one test per template, plus: no rendering of any of
  the four contains the substring "verif"; a `javascript:` payee renders
  as text with no anchor; markup in `reference`/`note`/`cancel_reason`
  renders as literal text.

### 10.2 Actions — `src/screens/transaction_panel.ts` (new)

`messages.ts` is over 800 lines (`F12`). Move `renderCardActions`,
`openDeclineDialog` and `openQuoteForm` (`messages.ts:496-800`) into this
new file first, unchanged, then add:

- Under an `agreement-receipt` card whose pair is complete: a panel that
  calls `booking.get` and shows the view, `next` as one plain sentence, and
  only the buttons this role may use now: provider — *Start work*,
  *Request payment*, *I received the payment*, *The work is done*,
  *Cancel booking* (only while both tracks are `none`, with a reason box);
  consumer — *I paid*, *The work is done* (confirm). Each calls the verb,
  then `refresh()`.
- *I paid* / *I received the payment* open a small form: date (defaults to
  today), method (a select from the agreement's `payment_methods` when it
  has any), reference (text). A recorded half shows *Correct this* which
  opens the same form with `supersedes` set.
- The quote form gains a *Slot* select filled by `availability.list` for
  the chosen listing, and shows `ONE_PAYMENT_NOTICE` beside the price.
- A consumer whose own half is filed and who has no progress yet sees
  *"Accepted. Waiting for the provider's system to confirm the booking."*

### 10.3 Backup tab

`backup.ts`: the note changes to *"Each bundle below is signed by you when
it is exported, and is refused on import if it was changed. An encrypted
backup of everything, including your identity, is made with
`roymctl roym backup create`."* `BUNDLES` is unchanged (five entries).

---

## §11 Tests

Run `mise run build:roym` before any parity run: the parity suite loads
pre-built `wasm32-wasip2` artifacts and otherwise tests stale code.

### 11.1 Unit

§3.2 (data_db), §4.4–§4.8 (roym_core), §9 (identity, roymctl). Each new
`roym_core` module has its `tests.rs` sibling.

### 11.2 Shim parity

§3.6 (`create` both ways).

### 11.3 Roym parity — `crates/roym_web/tests/dual_build_parity/`

New files, each under 800 lines, declared in `dual_build_parity.rs`:
`booking.rs`, `payment.rs`, `fulfilment.rs`, `bundles.rs`. Fixtures they
need go in `fixtures.rs` (it has room): `peer_signed_payment_ack`,
`peer_signed_fulfilment`, `peer_signed_progress` (signs with a key the test
also uses to sign the matching quote, so `signer_did` binds), `slot_quote`.

| # | Scenario | File |
|---|---|---|
| 150 | A superseded request version's card verifies after export and import (fails before the §6.12 fix — write it first) | `bundles.rs` |
| 151 | `quote.set` with a slot binds `slot_id` and the slot's window; a slot from another listing is refused | `booking.rs` |
| 152 | A consumer half over a slot quote, filed by `sync` on the provider, schedules and countersigns; `booking.get` shows `scheduled`, seq 1 | `booking.rs` |
| 153 | **Two consumer halves for two quotes naming one capacity-1 slot, filed by two concurrent `transaction.sync` calls (`tokio::join!`) on two conversations: exactly one `scheduled` and one `conflict` / `slot-taken`, on both builds; the loser gets no counter-half** (compare the outcome multiset, not which won) | `booking.rs` |
| 154 | The same consumer half delivered twice in two messages: one booking, one seat, same final state | `booking.rs` |
| 155 | A slot removed before acceptance → `conflict` / `slot-unavailable` | `booking.rs` |
| 156 | `booking.start`; `booking.cancel` with both tracks `none` frees the seat, and a third consumer then schedules; cancel after a track moved → `cannot-cancel`; consumer calling `booking.cancel` → `provider-only` | `booking.rs` |
| 157 | A progress card from a key other than the quote's `signer_did` is filed refused; a lower `seq` does not replace a higher one | `booking.rs` |
| 158 | `payment.request` carries no payee; the thread row shows the agreement's payee; a chat message naming another payee changes nothing | `payment.rs` |
| 159 | Consumer's half alone → `claimed`; provider's half alone (no consumer half) → `acknowledged`; both, in either order, → `acknowledged` | `payment.rs` |
| 160 | A half with the wrong amount, wrong currency, or a method outside the terms is refused (verb and filing) | `payment.rs` |
| 161 | A correction: new record with `supersedes`, both versions kept, track unchanged; a correction naming a stale version → `not-the-current-version`; a correction changing the amount is refused | `payment.rs` |
| 162 | `payment.acknowledge` twice with no `supersedes` → one record, `already-recorded` | `payment.rs` |
| 163 | Fulfilment: provider half → `claimed`; consumer half → `acknowledged`; both tracks acknowledged → `completed`, in three orders: pay before work, pay after work, and interleaved. (All 24 orders of the four events are enumerated in `roym_core`'s pure unit test `every_order_of_the_four_halves_completes`, where they are cheap) | `fulfilment.rs` |
| 164 | Halves whose terms differ from the agreement are refused | `fulfilment.rs` |
| 165 | A quote whose schedule ended more than `TRACK_WINDOW_SECS` ago: after scheduling, a `booking.get` moves open tracks to `unconfirmed` and the state to `ended-unconfirmed`; a late half is stored and shown but moves nothing | `fulfilment.rs` |
| 166 | A payment half filed before its agreement card in one `sync` is deferred, not refused, and files on the next `sync` | `payment.rs` |
| 167 | Every new verb is refused over the wire with `-32013` (extend `scenario_144`'s list — do it in the new file as its own scenario, since `transaction_cards.rs` is capped) | `bundles.rs` |
| 168 | `transaction.export` / `.import` round trip with bookings, ledger, payments (with a correction), fulfilments and progress: every row and every card's `verified` equal after import | `bundles.rs` |
| 169 | Every person-signed export carries a `manifest_signature` whose issuer is the owner; import refuses an unsigned bundle, a flipped digest, a re-signed manifest from another identity, and a manifest edited after signing | `bundles.rs` |
| 170 | `directory.export` stays unsigned and still imports | `bundles.rs` |

`strip_volatile` (`helpers.rs:103`) gains the new volatile fields:
`updated_at_secs`, `received_at_secs`, `created_at_secs`,
`observed_at_secs` when defaulted to now, and `message_id` on step rows.
Compare every signed envelope byte for byte except where a scenario says
otherwise (§14).

### 11.4 The e2e harness lift (WO0b)

New `crates/substrate/tests/common/roym.rs`, declared in `common/mod.rs`,
holding what `F11` lists, lifted from `roym_transaction_e2e.rs` (the most
recent copy) with the variations turned into parameters:
`fast_conversation_role(max_pending_age_secs)`, `CertOverrides` (from the
directory file) as an optional argument of `certify_and_publish`, and
`RoymNode` for `Node`. Use `common/retry.rs` for `wait_until` if it already
has an equivalent. Then switch `roym_conversation_e2e.rs`,
`roym_transaction_e2e.rs` and `roym_directory_e2e.rs` to it, delete their
copies, and confirm each still passes alone. This PR changes no behaviour.

### 11.5 `crates/substrate/tests/roym_booking_e2e.rs` — new

Three substrates (consumer X, second consumer W, provider Y), the full
Roym SynApp on each, no directory source on any node (assert
`directory.sources` is empty on all three at the end, `F16`).

1. Y: profile, listing, one slot of capacity 1. X and W: profile.
2. X and W each open a conversation with Y and send a request; Y syncs
   both and sends each a quote naming the slot.
3. X and W each sync, then accept.
4. Y runs `transaction.sync` on both conversations **concurrently**.
5. Exactly one `scheduled` and one `conflict` on Y; each of X and W, after
   a sync, shows the matching progress; the loser's pair is `half`.
6. The winner: Y `payment.request`; X syncs and its thread shows Y's
   agreement payee; X `payment.acknowledge`; Y syncs (`claimed`); Y
   `payment.acknowledge` (`acknowledged`); Y `fulfilment.sign` (`claimed`);
   X syncs and `fulfilment.sign` (`acknowledged` on Y after a sync) →
   `completed`, and X's progress shows `completed`.
7. The loser retries its accept (`already-recorded`) and re-sends; Y's
   state for it does not change.
8. Restart Y between steps 3 and 4 (the booking decision runs after a
   restart with no work lost — failure-matrix row 17's half this slice
   can reach).

Assert field by field; never assert on the word "verified" anywhere.

### 11.6 `crates/substrate/tests/roym_restore_e2e.rs` — new — the durability suite

Two substrates, provider Y and consumer X, then a clean Y′.

1. Run steps 1–6 of §11.5 with one consumer, stopping after Y has
   acknowledged payment and claimed fulfilment (a transaction in flight,
   with acknowledged records on both tracks' inputs), plus one correction
   of Y's payment half.
2. On Y: `roymctl roym backup create` (call the command's functions, or
   `SyneroymClient` + the same `identity::backup` calls; do not shell out).
3. Boot a clean Y′ with a fresh data directory; `restore-identity`; deploy
   Roym as that identity; `roym enrol-signing`; `restore-data`.
4. **The durability suite** — on Y′, compared with what Y answered before
   the backup:
   - every `agreement.get`, `booking.get`, `booking.history`,
     `payment.get`, `fulfilment.get`, `request.history`, `quote.history`;
   - `transaction.thread` for the conversation: every card's
     `card_type`, `record_id`, `verified`, `reason`;
   - `conversation.history` bodies;
   - every envelope verifies with the matching `*.verify` verb;
   - Y′ can still act: `booking.start` (or a `Tick`) writes the next `seq`,
     and a new `payment.acknowledge` correction signs and stores;
   - a second `transaction.export` on Y′ has the same section digests as
     Y's, apart from sections whose rows carry a host write time
     (`strip_volatile` rules).
5. Tamper: flip one byte of the archive → `restore-data` refuses and Y′'s
   state is unchanged.

This file also closes the backlog §3 row *"No e2e test for deploying as a
restored identity and enrolling signing"*.

### 11.7 Playwright — `crates/substrate/tests/e2e/tests/roym-transaction.spec.ts` — new

Add it to `testMatch` in `playwright.config.ts`. Reuse `roym-hub.spec.ts`'s
setup helpers by moving them into a shared `tests/roym-helpers.ts` first
if they are file-local. Measure the run time after adding the cases; if the
whole run approaches `globalTimeout: 300_000`, raise it in the same PR and
say so.

| Case | Asserts |
|---|---|
| 33 | The four new templates render real fields as text; no element from markup in `reference`, `note`, `cancel_reason` |
| 34 | A payment-request card shows the agreement's payee, a `javascript:` payee is text, an `https:` payee is a link that is not fetched until clicked |
| 35 | No text node on any transaction screen contains "verified" (walk the DOM) |
| 36 | The action panel shows only the buttons the role may use; *Cancel booking* disappears once a track moves |
| 37 | Recording a payment and correcting it shows both versions and "corrected" |
| 38 | `PAYMENT_NOTICE`, `PROGRESS_NOTICE`, `ONE_PAYMENT_NOTICE` pinned character for character |
| 39 | A conflict progress card renders the conflict sentence and no action buttons |

---

## §12 Failure-and-security-matrix rows and exit criteria

| Row | Closed by |
|---|---|
| 3 — no directory anywhere (R2 half) | §11.5 (`F16`'s reading, §18-J) |
| 5 — payee contradicted by chat | `D-C8-10`; parity 158; Playwright 34 |
| 6 — acknowledgement never "verified" | §4.4 constants + `no_notice_says_verified`; Playwright 35 |
| 7 — two concurrent bookings | `D-C8-4/5`; data_db concurrency test; parity 153; §11.5 step 5 |
| 8 — retried booking | `D-C8-6`; parity 154; §11.5 step 7 |
| 9 — unaccepted quote expires | Already C7; unchanged |
| 10 — altering a receipt | Immutable halves (C7) + correction path (`D-C8-13`); parity 161, 164 |
| 13 — export/import reproduces verification | `F8` fix + `D-C8-16/18`; parity 150, 168, 169; §11.6 |
| 17 — restart mid-session | §11.5 step 8 (the booking half) |

Exit criteria this slice meets: **6** (R1+R2 with no directory, as read in
§18-J) and **7** (R2's five rows). Criterion 4 moves forward: `payment-request`,
`payment-acknowledgement` and `fulfilment-receipt` are produced signed and
verified by the receiving node; `membership-credential`, `revocation` and
`moderation-decision` stay C9's. Criterion 10's four remaining templates
become real.

---

## §13 Order of work

Each work order is one PR unless noted. Run `mise run verify -- --skip e2e
--skip nextest` while iterating; the last run of each PR is the full
`mise run verify` in the background.

| WO | Content | Depends on |
|---|---|---|
| **0a** | §3 — the `create` host function, every layer, data_db tests, shim parity | — |
| **0b** | §11.4 — lift the Roym e2e harness into `common/roym.rs`; migrate the three files | — |
| **0c** | Parity 150 (fails), then §6.12 steps 3 and 9 only (history sections), then 150 passes. Fixes a C7 defect on its own | — |
| **1** | §4 — `roym_core`: verdict move, record constants, slot id, `booking`, `payment`, `fulfilment`, bundle signing, router, wording constants; all unit tests | — (can run beside WO0) |
| **2** | §5, §8 — catalog `availability.get` and the slot-id move; manifest edge; native bindings | 1 |
| **3** | §6.1–§6.11 — the transaction service; §7 — signing in the other three exports | 0a, 1, 2 |
| **4** | §6.12 rest, §9 — sections, `roymctl` verbs, the archive, identity `seal`/`open` | 3 |
| **5** | §11.3 — parity 151–170 | 3, 4 |
| **6** | §11.5, §11.6 — the two e2e files | 0b, 4 |
| **7** | §10, §11.7 — the Hub and Playwright | 3 |
| **8** | §17 — documents and backlog; final `mise run verify` | all |

Execution notes from earlier slices: `cargo audit`, the e2e suites and any
commit/push need the sandbox off; clippy, fmt, deny and unit tests run
inside it.

---

## §14 What is compared across builds, and what is not

- **Byte for byte:** every signed envelope this slice adds — progress
  (seq 1 onward), payment request, both acknowledgement halves and every
  correction, both fulfilment halves, every `manifest_signature` — as
  signed, as stored, and as returned by `*.get` / `history` / `thread`.
- **After `strip_volatile`:** every row shape (`BookingRow`, `ProgressRow`,
  `PaymentsRow`, `FulfilmentsRow`, `LedgerRow`) and every view.
- **As a multiset, not by position:** the outcome of the concurrent booking
  (parity 153) — which consumer wins is not deterministic, and is not
  required to be.
- **Not compared:** conversation message ids (already normalised), a
  step's `message_id`.

## §15 Permitted differences (WASM vs native)

None. `create` goes through the same `store::Host` on both builds (the
native host calls `HostStore::…` on the sandbox's own state,
`crates/app_host_native/src/host.rs:172`). A scenario that needs a
difference to pass is a shim bug (failure-matrix row 19).

---

## §16 What C8 deliberately does not build

Each with a backlog row (§17):

- **Deposits and part payments** — `D-C8-1`.
- **A consumer-signed cancellation** — `D-C8-14`. A consumer asks in chat.
- **Cancellation deadlines computed from terms** — the spec's *"the UI
  shows when cancellation is no longer guaranteed"* needs a signed deadline
  field; `cancellation_terms` is free text. The Hub shows the text.
- **An encrypted backup made in the Hub** — `D-C8-17`.
- **Continuing a conversation from a restored node** — `D-C8-19`.
- **`max_per_booking` and multi-seat bookings** — one booking takes one
  seat.
- **A node-side trigger for `sync`** — still `D-C7-3`; the consumer's
  actions reach the provider's state only when the provider's client syncs
  that thread.
- **Seat counts in `availability.list`** — catalog does not know claims;
  the provider sees them through `booking.list`.
- **Fencing `catalog`'s and `conversation`'s older read-modify-writes** —
  the primitive now exists; applying it there is those rows' own work.
  Both rows are restated with "the primitive exists" rather than closed.

---

## §17 Documents and backlog owed

**Documents**

| Document | Edit |
|---|---|
| [status.md](status.md) | A C8 section: what shipped, `D-C8-1`…`D-C8-21` as built, evidence. **Correct the C7 WO2–WO5 section (`F1`)** to the real verbs, subcommands and scenario range, with a dated note that it was corrected |
| [task.md](task.md) | C8 row marked complete. **Gap 9** added under "The gaps": *no write in `data-layer` is not last-write-wins*, closed by this slice's `create`. The open design points "whether one agreement can carry more than one payment" and "whether the export bundle is one format or several" answered with `D-C8-1` and `D-C8-17`. "Owed as slices land" C8 row discharged. `D-06C-13`'s track terminals named as `ended-unconfirmed` for the booking as a whole |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | **R2 marked passed.** *Transaction state* diagram gains `conflict` and `ended-unconfirmed`, and a sentence that a quote naming a slot is booked by its acceptance (`D-C8-2`). *Records* gains a `bundle-manifest` row (signed by the person; proves who exported these sections with these hashes; does not prove the export is complete). *Cards* notes that `payment-request` carries no payee |
| [developer-guide.md](../../../developer-guide.md) | `roymctl roym backup` — the restore order (identity → deploy → enrol → data) |
| [AGENTS.md](../../../../AGENTS.md) / [CLAUDE.md](../../../../CLAUDE.md) | Nothing, unless the architecture paragraph lists `data-layer` functions (it does not today — check) |

**Backlog rows closed** (move to "Recently resolved")

- §3 *"The app-data export bundle carries no signature"* — four of five
  services; the directory remainder becomes its own row.
- §3 *"No e2e test for deploying as a restored identity and enrolling signing"* — §11.6.
- §12 *"Failure-matrix row 10's correction path is not built"* — `D-C8-13`.
- §12 *"`payment-request` is a signed record with no `RECORD_TYPES` row"*.
- §12 *"A card of a known type with no producer files `known: true, verified: false`"*.

**Backlog rows restated**

- §5 *"Three read-modify-write sequences … unfenced"* — the fence
  primitive exists (`create`); the three call sites are not changed.
  Trigger unchanged.
- §12 *"Two concurrent `request.set` / `quote.set` calls … derive the same
  id"* — same.
- §3 *"Dynamic record signer key re-derivation"* — C8 adds a second
  reliance on a stable service key: a progress card binds to the quote's
  `signer_did`. An owner change re-keys the service, and progress signed
  after it no longer binds to quotes signed before it. Invariant stated;
  trigger unchanged.

**Backlog rows opened**

| Item | Trigger |
|---|---|
| Deposits / more than one payment per agreement (`D-C8-1`) | A provider asks for a deposit |
| No consumer-signed cancellation; a consumer asks in chat (`D-C8-14`) | The card set is reopened |
| No signed cancellation deadline; the Hub cannot say when cancelling stops being guaranteed | A provider wants a deadline enforced |
| The encrypted archive is `roymctl`-only (`D-C8-17`) | A person without shell access needs a backup |
| ~~A restored node cannot continue its live conversations~~ — **already added 2026-09-24** as the IMPORTANT row at the top of `deferred-backlog.md` §5 (target `blocks-prod`, before C10). Do not add a second row; when C8 lands, only check that the row's claim about `restore-data`'s notice is true | — |
| `directory`'s bundle is unsigned | C9 gives `directory` a signing identity of its own |
| `max_per_booking` is unread; one booking = one seat | A listing sells more than one unit per booking |
| A late signed record after a track's terminal is shown but moves nothing (`D-C8-15`) | A payee confirms after the window and the state looks wrong to them |
| A track's time edge is applied only when the provider's node reads or writes that booking | A consumer sees a stale state because the provider went quiet |
| The counter-half is refused by the provider's clock even when the acceptance was in time (`F14`) | A consumer's in-time acceptance is lost to a late thread open |
| Seat freed by a delete outside the step's transaction; repaired on the next read | Never, unless a leaked seat is observed |

---

## §18 Ambiguities and staleness in the input documents

**Confirm before WO1** — the owner's calls, with this plan's recommendation:

- **A. [CONFIRM] Booking is the acceptance of a slot quote (`D-C8-2`).** The
  spec's journey (C14 accept, C15 book) and `task.md`'s reference scenario
  (step 12 accept, step 13 book) make them two steps. With the fixed card
  set and no wire surface, a separate consumer "book" step has no carrier.
  The alternative is a wire-reachable `booking.request` verb on the
  provider's `transaction`, with its own admission table and a signed
  consumer record type that is not in the Records table — much more work,
  and it reopens `D-C7-1`. Recommended: this plan's shape.
- **B. [CONFIRM] One payment per agreement (`D-C8-1`).**
- **C. [CONFIRM] Corrections only for payment acknowledgements (`D-C8-13`).**
  R2's receipt row says *"corrections appear as separate records"*, and
  `D-06C-12` leaves the other two receipts nothing to correct.
- **D. [CONFIRM] Track window of 30 days after the schedule (`D-C8-15`), and
  the name `ended-unconfirmed`** for a booking whose tracks both ended
  without both being acknowledged. `D-06C-13` names only the track
  terminals.
- **E. [CONFIRM] Provider-only cancellation (`D-C8-14`)**, and the
  consequence that a pair the provider completes by hand can be
  `complete` with a `conflict` booking (§6.6 point 2).
- **F. [CONFIRM] Encrypted archive in `roymctl` only; durability suite as
  defined; no conversation continuity after restore (`D-C8-17`, `D-C8-19`).**
  If the Hub must make the encrypted backup in this slice, add WebCrypto
  HKDF + AES-GCM in `src/backup_crypto.ts` with a Rust-generated test
  vector the vitest suite decrypts, and a `web`-free flow (the browser
  calls the five exports itself). That is roughly one more work order.

**Stale or inconsistent, decided here:**

- **G. `F14`, the counter-half clock.** Changing it means trusting a
  consumer's `issued_at_secs`, which the consumer's own substrate sets.
  Not changed; backlog row.
- **H. `status.md`'s C7 section (`F1`).** Corrected in §17.
- **I. The spec's service table lists Transaction's API as `request.*`,
  `quote.*`, `agreement.*`, `receipt.*`.** This plan uses `booking.*`,
  `payment.*`, `fulfilment.*`; `receipt.ping` stays. The table is
  illustrative ("Main API"); §17 updates it.
- **J. "No Directory deployed anywhere" (exit criterion 6, `D-06C-6a`).**
  Every Roym deployment includes the `directory` service. Earlier slices
  read the criterion as "no directory source configured and no directory
  verb in the path" (`F16`). This plan keeps that reading and asserts it.
  A literal reading needs a manifest variant without `directory`, which
  `web`'s `depends_on` forbids today.
- **K. "Idempotency key" (spec *Transaction state*, R2 row 1).** Read as the
  record's own identity, per ADR-0023 §1 (`D-C8-6`). No client-supplied
  token exists.
- **L. "Durability suite" (R2 row 5)** is named nowhere else in the tree
  or the docs. Defined in §11.6.
- **M. `task.md`'s C8 row: "with the payee bound into the signed
  agreement".** Already true since C7 (`AgreedTerms.payee`). C8's work is
  that nothing *else* can name one (`D-C8-10`).
- **N. The spec's diagram puts `cancelled` after `agreed`.** In this plan
  a booking only exists from `scheduled` on; an agreement that never gets
  a booking has no state row to cancel, and the provider simply does not
  proceed. Stated in §17's spec edit.
- **O. `transaction.export`'s doc comment (`F8`)** is wrong about the
  history collections; replaced when the fix lands.
- **P. `wire_roym_topology` (`F7`)** misses C7's own edge; fixed in §8.
- **Q. Three Roym e2e files break AGENTS.md's harness rule (`F11`)**;
  fixed in WO0b before any new e2e file is written.
- **R. `booking-progress` and `RECORD_TYPES`.** `RECORD_TYPES`' doc says
  *"every record type this product produces"*, and `booking-progress` is
  produced and signed. `D-06C-12` keeps it out of the Records list. This
  plan follows `D-06C-12` and says why in the constant's doc comment
  (§4.2); `RECORD_TYPES`' own doc line gains *"by a person"*.
- **S. What continuity after restore would need (`D-C8-19`), checked in the
  code 2026-09-24. Deferred; recorded so the backlog row starts from facts.**
  1. *The address survives if the member master key survives.* A
     conversation address is the `conversation` service's **member master
     DID** (ADR-0020 §1–§2), not an instance key and not the person's DID.
     `roymctl app deploy --mint-masters` reuses
     `<dir>/identities/member-<app-instance>#<service>-0.key` when the file
     exists (`apps/roymctl/src/commands/member_identity.rs`,
     `resolve_or_mint_member_master`); a supervisor-managed deploy keeps it
     in the vault and can move it with `supervisor export-master` /
     `import-master`. Endpoint records are published under the master DID
     (ADR-0020 §6), so peers can find the new machine. The C8 archive holds
     only the person's identity, not these six member keys.
  2. *The record-signing key does not survive.* `NodeRecordSigner` derives a
     service's signing key as `HKDF(node key, owner DID, service id)`
     (`crates/identity/src/keys.rs`, `derive_service_identity`). A clean
     machine has a new node key, so every service gets a new signing key:
     the person re-enrols (already in the restore order), and a
     `booking-progress` signed after restore does **not** bind to the
     `signer_did` of quotes signed before it (`D-C8-7`). Only local reads
     and writes of those agreements work on the restored node; the peer
     refuses its new progress cards.
  3. *The encryption session cannot be re-established with an old peer.*
     The messaging layer's own signing key and Olm account are generated at
     random on first use and kept in the host's `conversation.db`
     (`crates/conversation/src/crypto.rs`, `generate_identity_bytes`). A
     peer pins that key per address on first contact, and *"a later message
     presenting a different signing key for a pinned address is a hard
     failure, never a silent re-pin"* (`session_for_envelope`). So with the
     same address and a fresh store, both directions fail. Restoring
     `conversation.db` itself is not a fix: it hits the raw-DEK backlog gap,
     and replaying an old ratchet state after newer messages is unsafe.
     The likely fix is a **re-pin notice signed by the member master key**
     (the stable anchor both sides already know), which the receiving
     layer accepts in place of the old pin. That is messaging-layer work.

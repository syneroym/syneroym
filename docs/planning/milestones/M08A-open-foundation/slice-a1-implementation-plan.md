# M08A slice A1 — Participant rename: implementation plan

*Reader: the developer (or AI) who implements slice A1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: waiting for the owner's answers to Q1 to Q7 (section 2.1),
2026-10-29.** Do not start Step 2 before section 3 matches the answers.

**Size and PR.** M. One PR on `feat/m08a-a1-implementation`, with the
commits of section 4 in that order. The branch starts from `main` at
`43dd44d4` (after S0 merged, PR #329) plus one backlog commit.

**Merged alone.** A1 touches almost every Roym file. No other M08A slice
opens a PR until A1 is on `main` (task.md section 4). S0 is already merged,
so nothing runs beside A1.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - The import clean-up pass.
  - No planning ids in code: no "M08A", "A1", "OF-1" in code, comments or
    test names.
  - Simple English in comments, docs and UI text.
  - 800-line files, 100-line functions, and the size ratchet (section 5.4).
  - The deferred backlog, the living-docs close-out and the demo close-out.
- [task.md](task.md): section 1 (X1), "A1 — Participant rename", section 4.
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  OF-1, OF-2, "Participant rename".
- [roym-value-exchange change doc](../../changes/roym-value-exchange/change.md):
  "Terms" (participant, quote author, commitment), "Resources and intents",
  "Two ways to start an exchange". These define the words A1 brings in.

### Terms used in this plan

- **Deal:** today, one request, the quote that answers it, the agreement
  both sides sign, and the booking, payment and fulfilment records that
  follow. The value-exchange change later calls the agreement's parts
  "commitments".
- **Quote author:** the participant who writes and signs the quote. Today
  this is always the owner of the listing. It is also the only writer of
  the booking (the "single writer" rule).
- **Counterparty:** the other party of the deal. Today it is the person who
  sent the request and to whom the quote is addressed.
- **Giver / receiver:** said about **one resource** in a deal, not about a
  person. Today the quote author gives the work and receives the money. The
  counterparty gives the money and receives the work.
- **Listing owner:** the participant who publishes a listing. Not a deal
  role: a listing exists before any deal.
- **Track:** the booking keeps two tracks, payment and fulfilment. Each
  track has two "halves", one statement from each party
  ([booking.rs:74](../../../../crates/roym_core/src/booking.rs#L74)).
- **Signed record:** a record inside an `Envelope`. The signature and the
  `record_id` cover the canonical JSON of the whole envelope, **with its
  field names** (`signing_bytes`, `record_id`,
  [envelope.rs:190-202](../../../../crates/signed_record/src/envelope.rs#L190)).
  So renaming a payload field changes what is signed.
- **Card:** a signed record wrapped for a conversation message
  ([card.rs:37](../../../../crates/roym_core/src/card.rs#L37)). It travels to
  the other party's node, which verifies it again.

## 1. What A1 builds, in one paragraph

A1 removes the words "provider" and "consumer" (and "customer", Q5) as
deal roles from Roym. `Role { Consumer, Provider }`
([transaction.rs:85](../../../../crates/roym_core/src/transaction.rs#L85))
becomes `DealRole { QuoteAuthor, Counterparty }`. The DID fields
`provider_did` and `consumer_did` become `quote_author_did` and
`counterparty_did` in every signed record, stored row and RPC answer. The
Hub, `roymctl`, the card wording and the living docs say "participant" or
the deal role. The listing location values `at-provider` / `at-customer`
get new names (Q3). The meaning of every rule stays the same: who may
sign, who may write, who may cancel, what the next step is. Only names
change. Old dev data with the old names is not read after the change (Q2).

## How this is shown to an end user

- An end user can do nothing new. The same screens and steps work as
  before.
- On a quote, agreement or booking card, the person sees new words. The
  agreement card says "Quote by" and "Quote to" in place of "Provider" and
  "Consumer" (exact words in Q6).
- The payment and work lines on a booking say "The payer says they paid"
  and "The person doing the work says it is done" in place of "customer"
  and "provider".
- In the directory and SynOrg screens, "provider" becomes "participant"
  or "listing owner" (Q4).
- In the listing editor and the quote form, the place options read "At my
  place", "At their place" and "Remote" (Q3).

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-web --test dual_build_parity` runs
  the same 222 scenarios as before, and all pass.
- `cargo run --bin roymctl -- roym booking --help` shows the new help text
  (no "provider" or "consumer").
- The Hub's Components tab (`main.ts:344`) shows the sample agreement card
  with the new labels.

**Demo:** none. A1 adds no command, no screen and no working path. The two
demo stories (`10-substrate-main`, `20-roym-chat`) do not show cards or
deals, so no story text changes. The change doc has no `demo:` list yet.
A7 adds it at close-out with M08A's chat story; A1 adds nothing to it.

## 2. Points for the owner

### 2.1 Questions

**Q1. One deal role everywhere, or a giver/receiver role on the track
records?** Today one `Role` enum is used in three ways:

| Record | `Role::Provider` means | `Role::Consumer` means |
| --- | --- | --- |
| `agreement-receipt` half, `PairState::Half`, booking `cancelled_by`, `next_step(me)` | quote author | counterparty |
| `payment-acknowledgement` half ([payment.rs:90-99](../../../../crates/roym_core/src/payment.rs#L90)) | payee: **receiver** of the money | payer: **giver** of the money |
| `fulfilment-receipt` half ([fulfilment.rs:35-40](../../../../crates/roym_core/src/fulfilment.rs#L35)) | **giver** of the work | **receiver** of the work |

So `Provider` is a receiver on one track and a giver on the other. One
rename cannot say "giver" correctly in both. The `consumer_did` /
`provider_did` fields of these records always name the deal parties, not
the track sides.

- (a) **One enum, `DealRole { QuoteAuthor, Counterparty }`, in every
  record.** The DID fields are `quote_author_did` and `counterparty_did`
  everywhere. The rules keep their form: `apply_half`
  ([booking.rs:259-263](../../../../crates/roym_core/src/booking.rs#L259))
  stays "`(Payment, QuoteAuthor)` or `(Fulfilment, Counterparty)` is
  against interest". "Giver" and "receiver" are used only in docs and UI
  text in A1. They become code when the value-exchange change brings
  commitments, where each commitment names its own giver and receiver.
- (b) **Two enums.** `DealRole` for the agreement, the booking and
  `next_step`. A new `TrackSide { Giver, Receiver }` for the `role` field
  of `payment-acknowledgement` and `fulfilment-receipt`. Verification then
  needs a fixed table "payment giver = counterparty, fulfilment giver =
  quote author" to check the issuer
  ([payment.rs:276-279](../../../../crates/roym_core/src/payment.rs#L276),
  [fulfilment.rs:108-111](../../../../crates/roym_core/src/fulfilment.rs#L108)).
  `apply_half` becomes "`role == Receiver`".

*Recommendation:* (a). A1 must not change behaviour, and (a) is a pure
rename. (b) adds a table that encodes today's fixed deal shape (the author
always gives the work and receives the money). Commitments replace that
shape in the value-exchange change, and the table would then be deleted.
(b) also changes the type of a signed field, which is a design change, not
a rename. With (a), task.md's X1 ("`giver`, `receiver`" in `roym_core`) is
met only in the docs; section 3 records this as a deviation.

**Q2. Old data and older nodes.** Every renamed field is in a signed
record, a stored row, or both (section 2.3 lists them). The change doc
says the product is not released, records change in place with no
migration, and `version` stays 1.

- (a) **Rename in place.** No `serde(alias)`, no migration. Record
  versions stay 1. The transaction service's storage `SCHEMA_VERSION` goes
  from 3 to 4 ([app.rs:40](../../../../crates/roym_transaction/src/app.rs#L40)),
  so an old backup is refused with a clear message
  ([backup.rs:157-160](../../../../crates/roym_transaction/src/app/backup.rs#L157))
  and not with a "missing field" error. A dev node with old deal data must
  be reset.
- (b) **Accept both on read.** `#[serde(alias = "consumer_did")]` and the
  like on every renamed field and value. The signature still verifies,
  because `Envelope.payload` is a `serde_json::Value`
  ([envelope.rs:143](../../../../crates/signed_record/src/envelope.rs#L143))
  and is signed as stored. But rows rebuilt from typed payloads are then
  written with the new names, so one store holds both forms. The ledger
  fence ids (`ack:{agreement}:{role}:…`, `fulfil:{agreement}:{role}`,
  [fence.rs:142](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L142),
  [fulfilment_ops.rs:326](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L326))
  would need both forms too. The aliases stay forever, because nothing
  says when every node has updated.
- (c) **Keep the old wire names** (`#[serde(rename = "consumer_did")]`) and
  rename only Rust names and UI text. It breaks OF-1.

*Recommendation:* (a), with the `SCHEMA_VERSION` bump. It is what the
change doc already decided. (b) costs code in every record and row for
data that exists only on dev machines. A node on an older build and a node
on the new build cannot exchange deal cards after A1 (a card with the old
fields fails to parse, so it is refused). This is accepted: cross-version
compatibility is G5, deferred (backlog section 9, "Public contract
versioning (`[G5]`)").

**Q3. The location values `at-provider` and `at-customer`.**
`ServiceLocation { AtProvider, AtCustomer, Remote }`
([listing.rs:124-128](../../../../crates/roym_core/src/listing.rs#L124))
is a signed value in listings (sent to directories) and in quote terms
(`terms.location.where`, sent in quote cards). OF-1 says no enum value may
say "provider". In a listing it means "at the listing owner's place". In a
quote it means "at the place of the party who does the work".

- (a) `at-giver` / `at-receiver`: the place of the giver or receiver of
  the work. Correct in both records today, and still correct for a want in
  the value-exchange change (a want's owner receives).
- (b) `at-owner` / `at-requester`: reads well in a listing, wrong for a
  bid on a want.
- (c) Keep the values. Record an exception to OF-1 in the change doc's
  Deviations.

*Recommendation:* (a). The Hub never shows the raw value after A1: the
listing editor shows "At my place" / "At their place" / "Remote"
(`listings.ts:305` today prints the raw value as the option text). The
`roymctl … --where` flag takes the new values
([transaction.rs:239-240](../../../../apps/roymctl/src/commands/roym/transaction.rs#L239)).

**Q4. "Provider" and "consumer" that do not mean a deal role.** Many uses
mean the **listing owner** (for example "Message this provider",
`directory.ts:400`; `set_up_synorg_and_provider` in `roym_trust_e2e.rs`)
or the **searcher** who reads results ("what a consumer's node checks",
`synorg_members.ts:106-107`). OF-1 does not cover them (they are not wire
fields). OF-2 does: "The Hub and the user docs say 'participant', or a
deal role, everywhere."

- (a) A1 renames all of them in Roym code: user-visible text, Rust and TS
  identifiers, test names and comments. "Listing owner" for the one who
  publishes, "searcher" or "participant" for the one who reads. One
  separate commit (section 4, Step 6).
- (b) A1 renames only user-visible text (Hub, `roymctl`, living docs).
  Identifiers and comments in this sense stay. A backlog row tracks them.

*Recommendation:* (a). If "provider" stays in Roym code in any sense, the
next change copies it back as a deal role. The cost is mechanical. Not in
either option: ADRs (permanent records), archived and historical milestone
plans, `VISION.md` (market language, not product terms) and every
unrelated sense (section 2.4).

**Q5. Is "customer" part of the rename?** "Customer" is the same deal role
as "consumer". It appears in `PAYMENT_CLAIMED`, `FULFILMENT_ACKNOWLEDGED`
([booking.rs:30,33](../../../../crates/roym_core/src/booking.rs#L30)),
`AtCustomer`, the quote form option "At customer" (`messages.ts:678`) and
one error, "progress for an agreement this node is not the customer of"
(`sync/receipts.rs:69`).

*Recommendation:* yes. OF-1's intent is "no fixed seller and buyer roles".
Leaving "customer" keeps the old model under another word.

**Q6. The words the end user reads.** "Quote author" and "counterparty"
are exact but heavy for a non-technical reader. The Hub can use plain
words, as long as each one maps to exactly one deal role.

| Where | Today | Recommended |
| --- | --- | --- |
| Agreement card party labels (`agreement_receipt.ts:85,92`) | "Consumer: …", "Provider: …" | "Quote to: …", "Quote by: …" |
| Agreement card half state (`agreement_receipt.ts:59-67`) | "Only the consumer has signed so far." / "Only the provider has signed so far." | "Only the person the quote was sent to has signed so far." / "Only the person who wrote the quote has signed so far." |
| Agreement card `Role: ${data.role}` (`agreement_receipt.ts:78`), raw value | "Role: consumer" | "Signed by: the person the quote was sent to" / "Signed by: the person who wrote the quote" |
| `PROGRESS_NOTICE` (Rust and TS) | "This status comes from the provider's system. …" | "This status comes from the system of the person who wrote the quote. It is not a signed statement by either person." |
| `PAYMENT_CLAIMED` | "The customer says they paid." | "The payer says they paid." |
| `PAYMENT_ACKNOWLEDGED` | "The provider confirms they received the payment." | "The payee confirms they received the payment." |
| `FULFILMENT_CLAIMED` | "The provider says the work is done." | "The person doing the work says it is done." |
| `FULFILMENT_ACKNOWLEDGED` | "The customer confirms the work is done." | "The person receiving the work confirms it is done." |
| `booking_progress.ts:45` | "The provider removed this slot." | "The listing owner removed this slot." |
| `DEFAULT_DATA_USE_NOTICE` (signed into every new request) | "… sent to the provider you chose. …" | "… sent to the person or business you chose. …" |
| `roymctl roym booking` help (`booking.rs:44,53,77`) | "Provider starts work…", "Provider cancels…", "Request payment from consumer." | "Quote author starts work…", "Quote author cancels…", "Request payment from the counterparty." |
| Errors (`booking_ops.rs:32,333,375`, `payment_ops.rs:335`) | `provider-only` | `quote-author-only` |
| Error (`quote_ops.rs:499`) | "only the consumer can decline this quote" | "only the counterparty can decline this quote" |
| Error (`agreement_ops.rs:378`) | "provider cannot accept a slot quote before the consumer accepts" | "the quote author cannot accept a slot quote before the counterparty accepts" |

"Payer" and "payee" are used because `payee` is already a signed term
field (`AgreedTerms.payee`,
[transaction.rs:130](../../../../crates/roym_core/src/transaction.rs#L130)).
Errors and `roymctl` help are read by developers and power users, so they
use the exact deal-role words.

*Recommendation:* the right-hand column. The owner may change any string.
The Rust and TS copies of the five constants change in one commit,
because two Rust tests pin them character for character
([booking/tests.rs:258](../../../../crates/roym_core/src/booking/tests.rs#L258),
[transaction/tests.rs:779](../../../../crates/roym_core/src/transaction/tests.rs#L779)).
The changed `DEFAULT_DATA_USE_NOTICE` applies to new requests only. A
request already signed keeps the text the person saw.

**Q7. A guard test against the old words.** AGENTS.md: a rule nobody
checks drifts.

- (a) A Rust test in `roym_core` that builds one sample of every signed
  record payload, serializes it, walks every JSON key and every string
  value that is an enum value, and fails on "provider", "consumer" or
  "customer". Plus a Hub unit test (vitest) that checks the same words
  are not in `cards/wording.ts` and in the text the card templates render
  for the existing fixtures.
- (b) An `xtask` grep gate over Roym crates. It needs an allow list for
  every unrelated sense (`StorageProvider`, rustls `default_provider`,
  …), which is easy to get wrong.
- (c) No new check.

*Recommendation:* (a). It checks exactly OF-1's surface (records, card
words) and needs no allow list. Free text that a person types (for
example a listing description) is not checked.

**Not a design question, but blocking:** the patch file
`backlog-a3-slots.patch` (a deferred-backlog row for A3, to be committed as
`docs(planning): record call-all slot limit for A3`) was not found in the
repository or the usual folders. Its commit is not made yet.

### 2.2 Stale or unclear points in task.md and the change doc

- task.md section 2 says "provider" is in about 234 lines and "consumer"
  in about 200 lines; the change doc says about 430 lines in 52 files.
  The sweep for this plan found about 680 Rust lines that use the words
  as a deal role, plus about 85 Hub lines and about 600 lines in living
  docs (sections 2.3, 5). Much of it is tests. Size stays M, because the
  change is mechanical, but the PR diff is large.
- X1 writes `quote_author`. On the wire, field names are snake_case and
  enum values are kebab-case (`#[serde(rename_all = "kebab-case")]`, as
  `before-work` and `at-provider` today). So the field is
  `quote_author_did` and the enum value is `"quote-author"`. Not a change
  of design; section 3 states it.
- The value-exchange change calls the non-author "the other side", and
  defines no "counterparty". The open-foundation change and task.md use
  "counterparty". A1 adds "counterparty" to `TERMINOLOGY.md`.
- "Receiver" already means a **message** receiver (`TERMINOLOGY.md`
  Mailbox, Queue lifetime, Sealed message rows; ADR-0026; the
  `conversation` and `control-plane` WIT comments). The new
  `TERMINOLOGY.md` row says "receiver (of a resource in a deal)" and links
  the mailbox sense as "not to confuse with".

### 2.3 What is stored or sent with the old names

All signed records are at `version` 1. All seven card types cross nodes.
The transaction service is local-only (`admit::require_internal`), so its
RPC answers reach only the local Hub and `roymctl`.

| Item | Kind | Crosses nodes | Effect of the rename | Recommended handling |
| --- | --- | --- | --- | --- |
| `QuotePayload.consumer_did` ([transaction.rs:187](../../../../crates/roym_core/src/transaction.rs#L187)) | signed | yes, quote card | Old quote cards no longer parse. | Rename in place (Q2a). |
| `AgreementReceiptPayload.{consumer_did, provider_did, role}` (:196-199) | signed | yes | Same. The pair check compares the same DID values, so it is unchanged. | Rename in place. |
| `BookingProgressPayload.{consumer_did, provider_did, cancelled_by}` ([booking.rs:95-105](../../../../crates/roym_core/src/booking.rs#L95)) | signed | yes, author's node to counterparty's node | Same. | Rename in place. |
| `PaymentRequestPayload.{consumer_did, provider_did}` ([payment.rs:52-53](../../../../crates/roym_core/src/payment.rs#L52)) | signed | yes | Same. | Rename in place. |
| `PaymentAcknowledgementPayload.{consumer_did, provider_did, role}` (:97-99) | signed | yes | Same. | Rename in place. |
| `FulfilmentReceiptPayload.{consumer_did, provider_did, role}` ([fulfilment.rs:38-40](../../../../crates/roym_core/src/fulfilment.rs#L38)) | signed | yes | Same. | Rename in place. |
| `Role` values `"consumer"`, `"provider"` | signed | yes | Become `"counterparty"`, `"quote-author"`. | Rename in place. |
| `ServiceLocation` `"at-provider"`, `"at-customer"` | signed | yes, listings to directories, quotes in cards | A listing published before A1 no longer parses on a new node, and the reverse. | Rename in place (Q3a). Re-publish dev listings. |
| `RequestPayload.data_use_notice` text | signed free text | yes | No parse effect. Old requests keep the old sentence. | New text for new requests only (Q6). |
| `RecordPointerRow.consumer_did` ([app.rs:71](../../../../crates/roym_transaction/src/app.rs#L71)) | stored | local, and in the backup | Old rows fail to parse. | Rename in place; dev reset. |
| `AgreementRow.{consumer_did, provider_did, consumer, provider}` (:82-88) | stored | local, and in the backup | Same. | Rename in place. |
| `PaymentsRow.{consumer, provider}`, `FulfilmentsRow.{consumer, provider}` (:179-192) | stored | local, and in the backup | Same. | Rename in place. |
| `snapshot` in booking, progress and ledger rows; `CardRow.data` | stored copy of a signed payload | local, and in the backup | Same. | Rename in place. |
| Fence ids `ack:{agreement}:{role}:{ver}`, `fulfil:{agreement}:{role}` | stored record id | local, and in the backup | A claim made before A1 is not found after it, so it could be made twice. | Rename in place; dev reset. |
| Backup sections and the signed `bundle-manifest` | hashed and signed file | yes, as a file | An old export cannot be imported. | `SCHEMA_VERSION` 3 → 4 gives a clear refusal. |
| RPC keys `role`, `pair.role`, `consumer`, `provider`, `consumer_did`, `provider_did`; `next = "wait-for-provider"` | wire | local only | Hub and `roymctl` change in the same commit. | Rename in place. |
| `NextStep::WaitForProvider` ([booking.rs:374](../../../../crates/roym_core/src/booking.rs#L374)) | wire value | local only | Becomes `"wait-for-quote-author"`. The Hub does not match on it (checked by grep). | Rename in place. |
| `backup.rs:473` `"receipt has role {:?}"` (roym_core) | error text | local | Prints the Rust variant name. | Prints `QuoteAuthor` / `Counterparty`; acceptable. |

No collection name or index uses the words
([app.rs:42-53, 356-397](../../../../crates/roym_transaction/src/app.rs#L42)),
and no JSON-RPC method name contains them
([app.rs:228-271](../../../../crates/roym_transaction/src/app.rs#L228)).
No WIT file and no `.json` or `.toml` fixture uses them as a deal role.
Directory listings, membership credentials, revocations and profiles have
no role field; the only listing hit is the location value (Q3).

`unverified`: whether any tool outside this repository parses `roymctl`
JSON output with these keys. The product is not released, so none is
expected.

### 2.4 Exclusions: do not rename

These use the same words for something else:

- **Storage and blob traits:** `StorageProvider`, `SqliteStorageProvider`,
  `BlobProvider`, `ObjectStoreBlobProvider`, the `storage_provider` /
  `blob_provider` variables and the test types `RacingStorageProvider`,
  `FlakyStorageProvider`, `DekOnlyStorageProvider` (data_db, data_blob,
  app_host_native, control_plane, router, sandbox_wasm, conversation,
  substrate, smoke-tests, perf). The path
  `crates/data_db/src/sqlite/provider.rs`. `TableProvider` in the docs.
- **rustls:** `ring::default_provider()` (about 138 sites) and the
  `roymctl` message "Failed to install rustls default crypto provider"
  ([main.rs:54](../../../../apps/roymctl/src/main.rs#L54)).
- **Identity providers:** "identity provider" (`config.sample.toml:235`,
  the integrated spec, ADR-0024), external/OIDC providers (ADR-0015,
  ADR-0024, backlog section 3).
- **Node and config roles:** `AppSandboxRole`, `MailboxRole`,
  `SupervisorRole`, `[roles.*]`, `group_role`, the UCAN fact
  `"role": "admin"` (`ucan/src/session.rs:139`), the X3DH role
  (`conversation/crypto.rs:51`).
- **"Consumer" of an API, crate or stream:** WIT comments ("bindgen
  consumers"), `Cargo.toml` comments, ADRs ("first consumer"),
  `substrate/src/runtime/roym.rs:474-477`
  (`for consumer in [CONVERSATION, CATALOG]`),
  `developer-guide.md:907`, `system-architecture.md:898-900`
  (`CONSUMER_TOK`), app_orchestration and app_supervisor comments.
- **Hosting roles:** "Infrastructure Provider", "Relay providers"
  (`system-requirements-spec.md`, `system-architecture.md`).
- **The retired product name** "Provider Space"
  (`TERMINOLOGY.md:86`), `THESIS.md:46`.
- **"Receiver" of a message** (mailbox, conversation): not renamed, and
  not reused for a deal role without "of a resource".
- **History:** ADRs, archived and historical milestone plans (for example
  M06C `slice-c7-implementation-plan.md`), and `VISION.md`.
- **Unrelated fields** in the Hub: `author` (message author) and `payee`.
- **`.claude/worktrees/`** is a local copy of the repository, excluded by
  `.git/info/exclude`. A repository-wide find and replace must not touch
  it.

## 3. Decisions this plan makes

These follow the recommendations. Update them with the owner's answers
before Step 2.

- **D-A1-1 (Q1).** `pub enum DealRole { QuoteAuthor, Counterparty }` in
  `roym_core::transaction`, kebab-case on the wire (`"quote-author"`,
  `"counterparty"`). The name `DealRole` (not `Role`) keeps it apart from
  `AppSandboxRole`, `MailboxRole` and the substrate `[roles.*]`. Every
  `consumer_did` becomes `counterparty_did`; every `provider_did` becomes
  `quote_author_did`. Row halves `consumer` / `provider` become
  `counterparty` / `quote_author`. No giver/receiver type in A1.
- **D-A1-2 (Q2).** Rename in place. No alias. Record versions stay 1.
  `SCHEMA_VERSION` 3 → 4 in `roym_transaction`.
- **D-A1-3 (Q3).** `ServiceLocation { AtGiver, AtReceiver, Remote }`
  (`"at-giver"`, `"at-receiver"`, `"remote"`).
- **D-A1-4 (Q4, Q5).** Listing-owner and searcher senses are renamed in
  Roym code too. "Customer" is part of the rename.
- **D-A1-5 (Q6).** The UI and error strings of Q6's right-hand column.
- **D-A1-6 (Q7).** The guard tests of Q7 (a).
- **D-A1-7.** Other names (all local-only):
  - `NextStep::WaitForProvider` → `WaitForQuoteAuthor`.
  - `pair_state(consumer, provider)` → `pair_state(counterparty, quote_author)`.
  - `prepare_provider_booking` → `prepare_author_booking`;
    `maybe_transition_provider_fulfilment` / `_ack` →
    `maybe_transition_author_fulfilment` / `_ack`; `provider_bookings` /
    `consumer_progress` → `author_bookings` / `counterparty_progress`;
    `ImportedQuote.provider_did` → `quote_author_did`.
  - Error variants `InvalidConsumerDid` / `InvalidProviderDid` →
    `InvalidCounterpartyDid` / `InvalidQuoteAuthorDid`, with the field
    names in their messages.
  - Test fixture DIDs `did:key:zConsumer` / `did:key:zProvider` →
    `did:key:zCounterparty` / `did:key:zQuoteAuthor`; helpers
    `base_consumer` / `consumer_key` and the like follow.
  - Hub CSS classes `receipt-consumer`, `receipt-provider` →
    `receipt-counterparty`, `receipt-author` (no style rule and no test
    selects them).
  - Parity scenario names that contain the words are renamed. Their
    number and order stay.

Where each decision is written down (Step 8 does it):

| Decision | Change doc | task.md | Living docs | Backlog |
| --- | --- | --- | --- | --- |
| D-A1-1 | `## Decisions`; `## Deviations` (giver/receiver not in code yet; enum value is kebab-case) | A1 result line | TERMINOLOGY, architecture, integrated spec | §12 row (track records name parties, not giver/receiver) |
| D-A1-2 | `## Decisions` | — | — | G5 row note (§9) |
| D-A1-3 | `## Decisions` | — | integrated spec, architecture | — |
| D-A1-4, D-A1-5 | `## Decisions` | — | SRS, architecture, integrated spec, developer guide, traceability | — |
| D-A1-6 | `## Decisions` | — | — | — |

## 4. Step-by-step changes

**Rule for every commit:** the workspace builds and the checks of section
5.1 pass at the end of it. Commit 2 is the only one that changes the wire.
Commit 1 changes no JSON at all, which is the first proof of "no behaviour
change".

### Step 1 — Commit this plan

- Commit only this file: `docs(planning): plan M08A A1 participant rename`.
- Wait for the owner's answers to section 2.1. Update section 3, the
  end-user section and the table in Q6 with them.
- Separate commit, when the patch file is supplied:
  `docs(planning): record call-all slot limit for A3`.

### Step 2 — Rust names only, the wire unchanged

**Commit:** `refactor(roym): name deal roles by action in rust`.

- `roym_core`:
  - `transaction.rs`: `Role` → `DealRole { QuoteAuthor, Counterparty }`
    with `#[serde(rename = "consumer")]` on `Counterparty` and
    `#[serde(rename = "provider")]` on `QuoteAuthor` (temporary, removed in
    Step 3). Rust fields `consumer_did` / `provider_did` →
    `counterparty_did` / `quote_author_did` with
    `#[serde(rename = "consumer_did")]` / `#[serde(rename = "provider_did")]`.
    `PairState::Half`, `pair_state`
    ([transaction.rs:230-237](../../../../crates/roym_core/src/transaction.rs#L230)),
    the verify matches (:722-736).
  - `booking.rs`: fields, `BookingEvent::Half`, `apply_half` (:259-263),
    `cancelled_by` (:313), `NextStep::WaitForProvider` with
    `#[serde(rename = "wait-for-provider")]`, `next_step` (:384-413), the
    `open` arguments (:202-212), error variants (:126-130).
  - `payment.rs` (:24-28, :52-53, :97-99, :228, :276-279) and
    `fulfilment.rs` (:20-24, :38-40, :108-111): the same.
  - `listing.rs`: `ServiceLocation` variants with `#[serde(rename = …)]`
    holding the old values.
  - Tests under `src/{transaction,booking,payment,fulfilment}/tests.rs`:
    Rust names only (about 35 `Role::` sites and the helpers). JSON string
    literals and fixture DID strings stay.
- `roym_transaction`: the row structs in `app.rs` (:71, :82-88, :179-192)
  with `serde(rename)` holding the old keys; every `Role::` site listed
  in section 5.2; the function and variable names of D-A1-7.
- `roym_web/tests/dual_build_parity/`, `crates/substrate/tests/` (roym
  binaries and `common/roym_flow.rs`), `apps/roymctl`: Rust names only.
- No change to the Hub, to any JSON key or value in any test, or to any
  user text.

**Proof for review:** no JSON literal changes. This command prints nothing
except the added `serde(rename …)` lines:

```bash
git diff HEAD~1 -- '*.rs' | grep -E '^[+-]' | grep -E '"[^"]*(consumer|provider|customer)[^"]*"' | grep -v 'serde(rename'
```

### Step 3 — The wire rename

**Commit:** `refactor(roym)!: rename deal roles on the wire`.

- Remove every temporary `serde(rename …)` from Step 2 (not the ones that
  existed before, such as `QuoteLocation.where_`).
- `SCHEMA_VERSION` 3 → 4 ([app.rs:40](../../../../crates/roym_transaction/src/app.rs#L40)).
- Fence ids now use `quote-author` / `counterparty`
  ([fence.rs:142](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L142),
  [fulfilment_ops.rs:326](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L326)).
  They come from the enum's serde value; check that `role_str` is built
  from it and not from a literal (`unverified`).
- RPC answer keys in `agreement_ops.rs` (:116, :216, :311-314, :357-360),
  `quote_ops.rs` (:384, :421), `payment_ops.rs` (:289, :390, :462-463),
  `fence.rs` (:234, :255), `fulfilment_ops.rs` (:140, :176, :243-244,
  :406, :416), `booking_ops.rs` (:224, :239).
- Refusal reasons that **name a field** change with the field (they are
  stored in `CardRow.reason`): `transaction.rs:669,726,733`,
  `payment.rs:229`, `sync.rs:463` (asserted at
  `dual_build_parity/transaction_validation.rs:521`).
- Rust tests: every JSON literal by the mapping table below. Fixture DIDs
  by D-A1-7.
- Hub (`crates/roym_web/ui/src`):
  - Types: `consumer_did?` / `provider_did?` / `role?` / `cancelled_by?`
    in `cards/templates/{agreement_receipt,payment_acknowledgement,fulfilment_receipt,booking_progress,payment_request,quote}.ts`
    and `screens/messages.ts:78`.
  - Logic: `messages.ts:550` (`forMe` reads `counterparty_did`; it gates
    the Accept and Decline buttons), `agreement_receipt.ts:57-92`,
    `payment_acknowledgement.ts:41`, `fulfilment_receipt.ts:33`.
  - Fixtures: `main.ts:344-346`, `templates.test.ts`, `messages.test.ts`.
- Playwright: `roym-hub.spec.ts:1079,1123` (`consumer_did: 'did:key:testme'`
  → `counterparty_did`). Without it, test 31's Accept button check at
  `:1093` fails.

**Mapping table** (the whole wire change; a test diff outside it is a
review finding):

| Old | New |
| --- | --- |
| key `consumer_did` | `counterparty_did` |
| key `provider_did` | `quote_author_did` |
| key `consumer` (row half, RPC) | `counterparty` |
| key `provider` (row half, RPC) | `quote_author` |
| value `"consumer"` (`role`, `pair.role`, `cancelled_by`) | `"counterparty"` |
| value `"provider"` (same) | `"quote-author"` |
| value `"wait-for-provider"` | `"wait-for-quote-author"` |
| fence id part `consumer` / `provider` | `counterparty` / `quote-author` |

`at-provider` / `at-customer` change in Step 5, not here, so this commit
stays one idea.

### Step 4 — The words people read

**Commit:** `refactor(roym): say deal roles in hub and roymctl text`.

- The five constants in `roym_core` (`booking.rs:28-33`,
  `transaction.rs:35-38`) and their TS copies (`cards/wording.ts:12-22`,
  `screens/messages.ts:22`) together, as Q6. Run the two pin tests.
- Playwright `roym-hub.spec.ts:975-977` (test 29 pins the notice).
- Agreement card labels and half text (`agreement_receipt.ts:54-92`),
  `booking_progress.ts:45`.
- Errors of Q6 (`booking_ops.rs`, `payment_ops.rs`, `quote_ops.rs`,
  `agreement_ops.rs`, `sync/receipts.rs:69`), the backup import messages
  (`roym_transaction/src/app/backup.rs:512,549,563,566`,
  `backup/sections.rs:205-245,372`), and the `Display` strings of the
  error variants. The parity and e2e tests that assert them.
- `roymctl` help (`commands/roym/booking.rs:44,53,77`).
- Comments in the files above that use the deal-role sense.

### Step 5 — The location values

**Commit:** `refactor(roym): rename service location values`.

- `ServiceLocation { AtGiver, AtReceiver, Remote }`; remove its Step 2
  `serde(rename)`.
- Hub: `listings/editor.ts:12` (`ServiceLocationKind`), the listing editor
  options with labels (`screens/listings.ts:305,396`; the `select()`
  helper at `:48` today shows the value as the label), the quote form
  (`messages.ts:678-681,784`), the card templates (`agreement_receipt.ts:35`,
  `quote.ts:27,57`).
- `roymctl --where` accepts `at-giver`, `at-receiver`, `remote`
  ([transaction.rs:239-240](../../../../apps/roymctl/src/commands/roym/transaction.rs#L239)).
- Tests that build listings or quotes with a location.

### Step 6 — Listing owner and searcher

**Commit:** `refactor(roym): say listing owner, not provider`.

Only if Q4 is (a). Hub text (`screens/directory.ts:96,298,400`,
`memberships.ts:49,74,89`, `synorg.ts:24,147`,
`synorg_members.ts:106-107`), `roymctl` (`find.rs:343`, `trust.rs:2`),
`roym.toml:93`, comments in `roym_core` (`listing.rs:1,172,189,389,502`,
`directory.rs:190,196`, `membership.rs:4,399`, `area.rs:26`),
`roym_catalog` and `roym_directory` comments, and the test names and
helpers in `roym_trust_e2e.rs`, `roym_directory_e2e.rs`,
`roym_conversation_e2e.rs:16`, `trust_harness.rs`, `trust_sources.rs`,
the `zProviderConv` fixtures, the Playwright test title
`roym-hub.spec.ts:604` and the comment `roym-trust.spec.ts:7`.

### Step 7 — Guard tests

**Commit:** `test(roym-core): refuse old role words in records`.

- New file `crates/roym_core/src/record/tests/wire_words.rs`. Move the
  inline `mod tests { … }` of
  [record.rs:55](../../../../crates/roym_core/src/record.rs#L55) to
  `record/tests.rs` first, as the module-layout rule requires, and add
  `mod wire_words;` there. Not in `transaction/tests.rs`: it is at its
  ratchet cap of 834 lines (`xtask/oversized-test-files.txt:20`).
- One sample of each of the seven payload types plus a listing. Serialize
  with `serde_json::to_value`. Walk keys and string values. Fail on
  `provider`, `consumer` or `customer`, and name the path.
- Hub: one vitest in `cards/wording.test.ts` (new) over the exports of
  `wording.ts` and the rendered text of the card fixtures already in
  `templates.test.ts`.

### Step 8 — Docs and close-out

**Commit:** `docs: describe deal roles by action`.

- `docs/TERMINOLOGY.md`, Component names table (Term | Meaning | Not to
  confuse with), new rows: **Participant**, **Quote author**,
  **Counterparty**, **Giver / receiver (of a resource)** (not the message
  receiver of a mailbox), **Listing owner**. Say/don't-say table: "provider",
  "consumer", "customer" → "participant" or the deal role.
- `docs/system-requirements-spec.md`: glossary (Consumer `:156`, Provider
  `:170`), the deal workflow (`:683-714`), and the persona and
  listing-owner lines (`:43-63`, `:75-99`, `:135-141`, `:213-227`,
  `:472-507`, `:602-611`, `:674-680`, `:945`, `:1294-1423`). Keep
  "Infrastructure Provider" and "Relay providers".
- `docs/system-architecture.md`: `:242` (the role sentence), the cards
  table and booking section (`:1194-1307`), `:3138-3148`. Headings that
  change move their anchors (`:50-68`, `:1394`, `:1396`, `:1485`, `:1509`,
  `:1915`); fix every link to them.
- `docs/roym-integrated-experience-spec.md`: Roles (`:36-52`), D8/D9
  (`:73-74`), the module and release tables, the journey (`:296-415`),
  the records and transaction state sections (`:466-553`), the cards
  table (`:741-745`).
- `docs/developer-guide.md`: `:1632` heading and `:1668`, `:1677`.
- `docs/planning/traceability-matrix.md`: `:9`, `:10`, `:54`, `:65`, `:89`.
- Change doc: `## Decisions` D-A1-1 to D-A1-7; `## Deviations` (X1's
  giver/receiver in docs only; the kebab-case enum value; "customer"
  added; the location values); tick the rename task; add the files above
  to `living-docs-touched`. `status` stays `draft`.
- task.md: A1 result line. status.md: A1 row (implemented by, reviewed
  by, evidence: the PR, the parity count).
- Deferred backlog: see section 6.

### Step 9 — Finish

- Import clean-up pass over every edited Rust file.
- `cargo +nightly-2026-04-06 fmt --all`.
- `mise run verify` as a background job, no `--skip`, every gate green.
- Open the PR with the CLA line and the attribution line. The PR text
  lists the commits, the mapping table, the parity count before and after,
  and "dev nodes with deal data must be reset".

## 5. Tests that prove "no behaviour change"

### 5.1 After every commit

```bash
cargo clippy -q --workspace --all-targets --all-features
cargo nextest run -p syneroym-roym-core -p syneroym-roym-transaction -p syneroym-roym-web > target/a1.log 2>&1; echo "exit=$?"; tail -n 5 target/a1.log
cargo nextest run -p syneroym-substrate -E 'binary(~roym)' > target/a1-e2e.log 2>&1; echo "exit=$?"; tail -n 5 target/a1-e2e.log
```

From Step 3 on, also the Hub unit tests (`npm test` in
`crates/roym_web/ui`). `unverified`: whether the substrate `roym_*`
binaries need `--features roym` with this filter, and whether the parity
suite rebuilds the WASM components by itself or needs the build step that
`mise run test:rust` runs first.

### 5.2 What each proof shows

1. **Step 2 changes no JSON.** The `git diff` check in Step 2 prints
   nothing but `serde(rename …)` lines, and every existing test passes.
   The Rust code is renamed, and the bytes on the wire and in storage are
   the same as before.
2. **Step 3 is exactly the mapping table.** The number of assertions per
   test file is the same before and after (`grep -c 'assert'`), and the
   reviewer finds no test edit outside the table.
3. **The parity harness is unchanged in shape.** The same 24 module files
   under `crates/roym_web/tests/dual_build_parity/`, the same 222
   `scenario_` functions, and `helpers.rs` keeps `assert_eq!(stripped(&w),
   stripped(&n))` (exact JSON equality between the WASM and native builds,
   after removing clock fields). Count before Step 2 and after Step 8:
   ```bash
   grep -c "fn scenario_" crates/roym_web/tests/dual_build_parity/*.rs | awk -F: '{s+=$2} END {print s}'
   ```
4. **The rules are the same.** The existing verify and transition tests
   still pass under their new names: a receipt whose issuer is not the
   DID for its role is refused, a quote to its own author is refused, only
   the quote author may cancel, `next_step` gives the same step for each
   state, `apply_half` marks the same halves as acknowledged.
5. **The Hub works the same.** Playwright `roym-hub.spec.ts` passes (test
   31 still shows Accept for the counterparty), and both demo stories
   pass in `mise run verify`.
6. **OF-1 holds and stays held.** The guard tests of Step 7.
7. **OF-2.** A grep over the Hub source and the living docs for
   `provider|consumer|customer` returns only the exclusions of section 2.4.

### 5.3 Not tested

- No test reads a record written before A1. Q2 (a) says such records are
  not read. A test that an old card is refused would pin a format nobody
  keeps.

### 5.4 Size risks

- Longer names (`counterparty_did` is four characters longer than
  `consumer_did`) can make `rustfmt` wrap lines. Files on the ratchet list
  are capped at their recorded size: `roym_core/src/transaction/tests.rs`
  (834), `dual_build_parity/transaction.rs` (852), `helpers.rs` (1438),
  `directory.rs` (1135), `profile.rs` (1119). Never raise a cap. If a file
  grows past it, shorten the code in that file. If a file shrinks, lower
  its entry (`cargo xtask check-file-lengths` names the value).
- `cargo xtask check-duplication` can move in either direction; record
  the new value if it improves.

## 6. Deferred backlog

- §12, "A card's issuer is not bound to the conversation peer": the text
  names `consumer_did`; change it to `counterparty_did`.
- §9, "Public contract versioning (`[G5]`)": add "A record or backup made
  before the deal-role rename is not read after it; there is no
  migration, because the product was not released".
- §12, new row (only with Q1 (a)): "The payment and fulfilment records
  name the deal parties, not the giver and receiver of the resource".
  Reason: a pure rename could not move to giver/receiver without changing
  the type of a signed field. Target: M08C (commitments). Source: this
  plan's Q1, `payment.rs:97-99`, `fulfilment.rs:38-40`.
- §10, new row: "`docs/reputation-design-note.md` still uses provider and
  consumer" (41 lines). It is a design note, not a living doc. Target:
  M08G (reputation), which rewrites it.
- If Q4 is (b): a §10 row for the identifiers and comments left in the
  listing-owner and searcher senses.

## 7. Exit criteria

A1 is done when all of these are true:

1. The owner has answered Q1 to Q7, and section 3 matches the answers.
2. No Roym wire field, enum value, method name or card label says
   "provider", "consumer" or "customer" (OF-1), checked by the Step 7
   tests.
3. The Hub, `roymctl` text and the living docs say "participant", a deal
   role or "listing owner" (OF-2); the grep of 5.2 point 7 finds only the
   exclusions.
4. The parity harness has the same 24 modules and 222 scenarios, and
   passes.
5. Every commit passes the checks of 5.1; Step 2 changes no JSON.
6. TERMINOLOGY.md, the living docs, the change doc (Decisions, Deviations,
   Tasks, `living-docs-touched`), task.md, status.md and the backlog are
   updated as in Step 8 and section 6.
7. `mise run verify` passes with no `--skip`.
8. One PR, merged before any other M08A PR opens.

## 8. What A1 does not do

- No change in behaviour: no new rule, method, record type or field.
- No giver/receiver type in code (Q1 (a)). Commitments come in M08C.
- No migration and no alias for old data (Q2 (a)).
- No change to ADRs, historical plans, `VISION.md` or any excluded sense
  (section 2.4).
- No typed method inputs (A2) and no change to the RPC method names (none
  contains the old words).
- No new demo story.

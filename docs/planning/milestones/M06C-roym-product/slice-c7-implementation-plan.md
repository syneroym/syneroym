# M06C Slice C7 — A Need Becomes an Offer, and the Card Contract: Implementation Plan

> **Scope.** [task.md](task.md)'s **C7** row — R1 row 4. Signed `request` →
> `quote` → `agreement-receipt`, each versioned, with a material change
> producing a new version rather than an edit. The card contract on the
> producing side, and the real templates on the consuming side
> (`D-06C-3`). The signing shape is fixed by `D-06C-12` — a single-issuer
> attestation, with `agreement-receipt` as a pair of them — and must not be
> re-decided here. Gates: **C5**, **C6**. **R1's acceptance gate closes at
> the end of this slice.**
>
> **C7 also owes one thing an earlier slice handed it by name**
> ([deferred-backlog.md](../../deferred-backlog.md) §2): the currency
> minor-unit exponent table, which lives only in TypeScript today and is
> targeted at `M06C C7` because C7's quote and agreement records carry
> money too.
>
> **Read §16 first if you are executing this plan.** Four claims in the
> input documents need adjusting against the tree, and one of them —
> **F** — decides the whole shape of how a record reaches the other party.
>
> **Planning identifiers appear in this document and must not appear in
> the code it describes** (`D-06C-11`). No `C7`, `R1`, `M06C` or slice
> number in a crate name, module name, collection name, JSON-RPC method
> name, card type, record type, config key, metric name, test name, or
> comment. Every name and comment proposed below already has that applied.

---

## §0 What C5 and C6 handed C7, and what is missing

Verified 2026-09-06 against `feat/m06c-slice-c6` at `6117c00`.

| Handed over | Where |
|---|---|
| One canonical signed envelope, its host signer, and a pure verifier with tri-state revocation | `crates/signed_record/src/{envelope,verify}.rs`; `crates/wit_interfaces/wit/signing/signing.wit` |
| `request`, `quote`, `agreement-receipt` already present in `RECORD_TYPES` at version 1 | `crates/roym_core/src/record.rs:18` |
| The seven card types and their versions, plus a Rust test pinning the TS registry to them | `crates/roym_core/src/card.rs`; `crates/roym_web/ui/src/cards/registry.ts` |
| A renderer with seven placeholder templates, one neutral unknown block, and a safe link helper | `crates/roym_web/ui/src/cards/{render,unknown,link}.ts` |
| The per-service record-signing certificate: `install`, `status`, `person_principal`, and one mountable verb pair | `crates/roym_core/src/signing.rs` |
| `admit::require_internal` (local-only) and `admit::admit` (a named wire-exception table) | `crates/roym_core/src/admit.rs` |
| A method-prefix routing table that already maps `request.` / `quote.` / `agreement.` / `receipt.` to `transaction` at `MethodAuth::Owner` | `crates/roym_core/src/router.rs:34-37` |
| Roym's own copy of every message, with `content_type` preserved and a text body stored verbatim for any `+json` type | `crates/roym_core/src/conversation.rs` (`encode_body`, `is_text_content_type`); `crates/roym_conversation/src/app.rs` |
| `conversation.open` / `.send` / `.history`, a block-and-rate-limit-enforcing inbox, and a symmetric conversation id both nodes compute | `crates/roym_conversation/src/app.rs`; `crates/conversation/src/ids.rs:15` (`derive_conversation_id` sorts the pair) |
| The `write_version` / pointer-row + history-row idiom for a versioned signed record, with `supersedes` | `crates/roym_catalog/src/app.rs:275` |
| `Area`, `TimeWindow`-free geometry, `ListingPayload`, `PaymentTerms`, `ServiceLocation`, and the integer-only money rule | `crates/roym_core/src/{area,listing}.rs` |
| A bundle format with a manifest, per-section digests and an integrity check | `crates/roym_core/src/backup.rs` |
| A two-build parity harness with a pinned `RecordClock`, an `inbound()` message helper and a `deliver()` entry into Roym's own inbox | `crates/roym_web/tests/dual_build_parity.rs:659`, `:2981` |
| A two-substrate and a three-substrate e2e harness with boot / deploy / restart and per-service minted masters | `crates/substrate/tests/roym_conversation_e2e.rs`; `.../roym_directory_e2e.rs` |

Missing, and C7's to build:

1. **`transaction` has no state at all.** `roym_transaction::app::invoke` answers
   four pings and nothing else (`crates/roym_transaction/src/app.rs:22-32`);
   `SCHEMA_VERSION` is 1 and the service owns no collection.
2. **`transaction` mounts no signing certificate.** It is absent from
   `SIGNING_SERVICES` in `apps/roymctl/src/commands/roym.rs:178`, from both
   e2e files, and from `router.rs`'s
   `every_certificate_mounted_service_routes_under_its_own_name` test — and
   `router.rs` has no `transaction.` prefix, so the certificate verbs would
   not be routable even if mounted.
3. **No card ever travels.** `CARD_TYPES` names seven types; nothing produces
   one, nothing carries one, and nothing stores one. The Hub's gallery
   renders nine hand-written literals in `main.ts`'s `renderHome`.
4. **The card templates are placeholders.** `renderQuote` shows
   `Price: ${data.price}`; there is no payee, no expiry, no terms, and no
   distinction between a verified card and a refused one.
5. **`transaction` is deliberately unbound in the parity harness.**
   `dual_build_parity.rs:1050` filters it out of both topology loops, and
   scenario 5 depends on that (`crates/roym_web/tests/dual_build_parity.rs:1533`).
6. **The currency table lives only in TypeScript.**
   `ListingPayload::validate` accepts any three uppercase letters
   (`crates/roym_core/src/listing.rs:300`).

---

## §1 Findings from reading the tree

Verified 2026-09-06 at `6117c00`. Each is load-bearing for a decision in §2.

### F1 — `depends_on` is cycle-checked, so `conversation` and `transaction` cannot both depend on each other

`SynAppManifest::validate` runs an explicit depth-first cycle detector over
`depends_on` and fails with *"Circular dependency detected in services"*
(`crates/app_orchestration/src/models.rs:722-771`, with its own test at
`:1374-1389`). `transaction` must reach `conversation.send` to put a card
into a conversation. Therefore `conversation` **cannot** depend on
`transaction`, and a push from the inbox into the transaction service is
not available. Ingestion has to be a pull. This is the single largest
shape-setting fact in the slice.

### F2 — a sibling proxy call arrives `internal`, a wire call does not

`WasmHostState::caller` maps `InvocationOrigin::Local` to
`CallerOrigin::Internal` and everything else to `Verified`/`Anonymous`
(`crates/sandbox_wasm/src/host_capabilities.rs:622-635`). So a
`CallTarget::Dependency` call from `transaction` into `conversation` passes
`conversation`'s own `admit::require_internal`, exactly as `catalog`'s
`profile.get` call already does (`crates/roym_catalog/src/app.rs:155-177`).
C7 therefore needs **no** wire exception anywhere: every `transaction` verb
stays local-only under `require_internal`.

### F3 — the conversation id is symmetric, and it is the only cross-node identifier both parties compute independently

`derive_conversation_id(a, b)` sorts the two addresses before hashing
(`crates/conversation/src/ids.rs:15-24`), so the consumer and the provider
compute the same `conv:<hex>`. It is the one value that can be signed into
a record and re-derived on the other side. Nothing else about the pair is
symmetric: `peer_address` is a routing service id, and `peer_person_did` is
only known when a contact carries it (`crates/roym_conversation/src/app.rs:161-171`).

### F4 — a `+json` content type is stored verbatim as UTF-8

`is_text_content_type` returns true for anything ending `+json`
(`crates/roym_core/src/conversation.rs:76-78`), so a card body survives
Roym's own copy byte for byte with `BodyEncoding::Utf8`. The existing unit
test already uses `application/vnd.roym.card+json` as its example
(`crates/roym_core/src/conversation.rs`, `encode_body_picks_utf8_for_text_and_base64_otherwise`).
No change to the conversation service is needed to carry a card.

### F5 — `conversation.history`'s cursor is an integer offset into a re-sorted list

`history` reads every row of the conversation, reconciles delivery state,
sorts by `sort_key` = `(sender_timestamp_ms, author, id)`, then
`skip(cursor).take(limit)` (`crates/roym_conversation/src/app.rs:648-697`).
A late-arriving message with an older sender timestamp inserts *before* the
offset and shifts everything after it. An offset stored as a watermark can
therefore skip a message. `D-C7-6` deals with this.

### F6 — `RecordDraft::validate` refuses a past expiry and any non-integer number

`expires_at_secs <= now_secs` is `DraftError::ExpiryInPast`, and any
`Value::Number` that is not `i64`/`u64` is `PayloadNonIntegerNumber`
(`crates/signed_record/src/envelope.rs:71-125`). So a quote's expiry is
expressible on the envelope and is enforced at signing time, and every
money field must be minor units. `verify` then refuses an expired envelope
with `VerifyError::Expired` (`crates/signed_record/src/verify.rs:171-176`)
— which is failure-matrix row 9 for free, provided the expiry rides the
envelope rather than only the payload.

### F7 — the host's `RecordDraft` takes its payload as a JSON **string**

`syneroym_app_host::types::signing::RecordDraft.payload` is a `String`
(see `crates/roym_catalog/src/app.rs:343-350`, which passes
`serde_json::to_string(&payload)`), while
`syneroym_signed_record::RecordDraft.payload` is a `serde_json::Value`.
The two types share a name and must not be confused; `roym_core::signing`
imports the former, `roym_core::record` re-exports the latter.

### F8 — the parity harness leaves `transaction` unbound on purpose, and scenario 5 rests on it

`dual_build_parity.rs:1046-1050` comments the choice out loud and
`scenario_5_unbound_dependency_returns_32001` drives `receipt.ping` to
assert `-32001`. Once `web` must reach `transaction` for real, the filter
has to go and scenario 5 needs its own harness variant.

### F9 — a service's `visibility` and `topology_visibility` are independent of its verb admission

`transaction` is declared `visibility = "public"` with no
`topology_visibility` (`crates/roym_core/app/roym.toml`), the same as
`catalog` and `conversation`, both of which refuse every verb over the
wire. `visibility` governs the registry record, not admission. C7 changes
no `visibility` value (`D-C5-4`'s precedent) and every `transaction` verb
stays wire-refused.

### F10 — the Hub renderer is pure and has no verification of its own

`renderCard` switches on `(type, version)` against `CARD_TYPES` and falls
back to `renderUnknown` (`crates/roym_web/ui/src/cards/render.ts`). It has
no notion of "verified", so a card whose signature failed would today be
rendered by its real template as if trusted. The consumer's own **node**
verifies (`D-06C-6c`); the Hub displays that verdict and must have a
distinct block for a refused card, the same shape the Directory tab already
uses (`crates/roym_web/ui/src/screens/directory.ts`, `renderRefused`).

### F11 — the Hub's Backup tab hard-codes three bundles and the browser suite counts them

`BUNDLES` has three entries and a note saying "The three bundles below"
(`crates/roym_web/ui/src/screens/backup.ts:5-50`); `roym-hub.spec.ts` case
8 asserts `toHaveCount(3)`. A fourth bundle changes all three places plus
the test's own title.

### F12 — a guest dispatch's five-second wall-clock budget is spent while waiting on a host call

Recorded by C6 as `F6b` and carried as a backlog row. A *local* sibling
dispatch is in-process and far cheaper than C6's cross-node directory
query, but an unbounded loop over `conversation.history` pages is still the
same failure shape. `D-C7-6` bounds it.

### F13 — the certificate-mounted service list is duplicated in four places

`apps/roymctl/src/commands/roym.rs:178`,
`crates/substrate/tests/roym_conversation_e2e.rs:91`,
`crates/substrate/tests/roym_directory_e2e.rs:114`, and
`crates/roym_core/src/router.rs:214` each carry
`["profile", "catalog", "conversation"]`. Adding `transaction` means all
four.

### F14 — `content_digest` is the one content hash, and `listing_id` is its precedent

`content_digest(prefix, value)` is z-base-32 SHA-256 over key-sorted
canonical JSON, with a caller-chosen prefix so two families of id cannot
collide (`crates/signed_record/src/envelope.rs:210-224`).
`derive_listing_id(issuer, slug)` is that function applied to
`{"issuer":…, "slug":…}` and is re-derived by the verifier from the
signature's own issuer (`crates/roym_core/src/listing.rs:380`, checked at
`:504-512`). C7's ids follow the same pattern exactly.

---

## §2 Decisions

| # | Decision | Why |
|---|---|---|
| **D-C7-1** | **A record reaches the other party as a conversation message of the reserved content type `application/vnd.roym.card+json`, and by no other path.** C7 opens no new wire surface: every `transaction` verb keeps `admit::require_internal`, and no `transaction` service ever calls another node | The spec puts the request, the quote and the agreement receipt in the conversation as cards (journey C12–C14, and the Cards table's "Appears at" column). Reusing the conversation gives durable delivery, `pending`/`delivered`/`failed`, block enforcement and the first-contact rate limit for nothing. The alternative — a direct `transaction`-to-`transaction` call — would need a second wire-reachable Roym surface, a second admission table, and a second answer to "who is this caller", all for a message channel that already exists and that `D-06C-8` already made the enforcement point. It would also put the single-writer question (`D-06C-13`, C8's) into R1 by the back door |
| **D-C7-2** | **`transaction` declares `depends_on = ["conversation"]`. Nothing declares a dependency on `transaction`.** | Forced by `F1`: the manifest's cycle detector refuses the pair. `transaction` needs `conversation.send` to put a card out and `conversation.history` to read one in, so the edge points that way and ingestion is a pull |
| **D-C7-3** | **Ingestion is `transaction.sync { conversation }`, called by the client.** It is idempotent, keyed by the conversation message id, and safe to call as often as a client likes | The consequence of `D-C7-2`. A client (the Hub, `roymctl`) calls it when it opens a thread. The honest cost: a person who never opens a thread never files its cards. Recorded as a backlog row rather than papered over — a node-side trigger needs either the forbidden edge or a host-level hook, neither of which is C7's to build |
| **D-C7-4** | **A card on the wire carries the signed envelope and nothing else derived from it.** Body shape: `{"card_version":1,"type":"quote","version":1,"envelope":"<the signed envelope, as JSON text>"}`. There is no sender-supplied `data` field on the wire | A card carrying both an envelope and a rendered projection lets the two disagree, and the reader has no way to know which one the sender meant. The receiving **node** verifies the envelope and produces the projection; the Hub renders the node's projection. This is `D-06C-6c`'s rule ("the consumer's own node verifies") applied to cards, and it is why `renderCard`'s `data` parameter can stay exactly as it is |
| **D-C7-5** | **The card's declared `(type, version)` must equal the signed envelope's own `record_type` and `version`, or the card is refused.** | Otherwise a signed `request` could be presented in a `quote` card and rendered by the quote template. The check is one line and closes a whole class |
| **D-C7-6** | **`sync` scans a bounded, overlapping window of the conversation and dedupes by message id.** State per conversation: `scanned_count`. A run starts at `max(0, scanned_count - SYNC_OVERLAP)` and reads at most `MAX_SYNC_PAGES` pages of `SYNC_PAGE` messages. A message whose id is already in the `cards` collection is skipped without parsing. `transaction.sync { conversation, full: true }` starts at 0 | `F5`: `history`'s cursor is an offset into a list that a late message can shift, so a bare watermark can skip. Dedupe by id makes re-scanning free, and the overlap makes ordinary reordering invisible. `F12` is why the page count is bounded rather than "until a short page". The residual — a message landing more than `SYNC_OVERLAP` positions behind the watermark waits for a `full` sync — is a backlog row, not a silent hole |
| **D-C7-7** | **Stable ids are content-derived from signed fields, the `listing_id` way.** `request_id = content_digest("req_", {conversation, issuer, sequence})`; `quote_id = content_digest("quo_", {conversation, issuer, sequence})`. `conversation`, `issuer` and `sequence` are all in the signed payload (`issuer` implicitly, as the envelope's own), so the receiver re-derives the id and refuses a mismatch — the same check `verify_envelope` already makes for a listing (`F14`) | A new version of a request or a quote must keep one identity while `supersedes` chains the versions, and the id has to be derivable by the receiver rather than asserted by the sender. `F3` makes `conversation` the one symmetric ingredient. `sequence` (the nth record of that kind this issuer made in that conversation) makes two records in one conversation distinct without a clock — the guest clock is not reproducible and must never enter a derived id |
| **D-C7-8** | **`agreement-receipt` has no id of its own. Its envelope `subject` is the quote's `record_id`, and its identity is `(quote_record_id, role, issuer)`.** | `D-06C-12`: an attestation references its subject record by that record's derived `record_id`, and neither half references the other. The quote's `record_id` is a content hash of the whole quote envelope, so it pins the terms exactly; a second digest would add nothing |
| **D-C7-9** | **Both halves of an `agreement-receipt` carry the full `AgreedTerms`, copied verbatim from the quote, and are byte-identical apart from `role`.** | Two reasons, and both are required. R1 row 4's acceptance test is *"a signed agreement receipt containing every field listed in [Records]"*, and the Records table's `agreement-receipt` row names payee, expiry, cancellation and refund terms, and dispute path — a bare hash reference does not contain them. And `D-06C-12`'s completeness rule is *"the payloads are identical apart from `role`"*, which needs payloads with something in them to compare |
| **D-C7-10** | **The provider's half is produced automatically by the provider's own node, during `sync`, when it files a valid consumer half over a quote this node issued.** Refused, and left to the person, if the quote has expired, if the consumer half's `AgreedTerms` differ from the quote's by one byte, or if the issuer is not the `consumer_did` the quote names. `agreement.accept` remains available so the provider can make the half by hand | The provider already signed those exact terms in the quote; the counter-half asserts nothing the provider has not already said, so it is a second signature over an existing statement, not a synthesized one. `D-06C-13`'s *"the missing half is never synthesized"* governs `payment-acknowledgement` and `fulfilment-receipt`, where the two halves describe **different observations** made against different interests — that is not this case. Without the automatic half, R1 row 4's acceptance test ("accepting a quote produces a signed agreement receipt") could not close in one flow, and the ordering that would remain belongs to C8's state machine anyway |
| **D-C7-11** | **A card's issuer is recorded and shown; it is never assumed to be the conversation peer.** What C7 *does* enforce: the payload's `conversation` equals the conversation the card arrived in, and the derived `request_id`/`quote_id` matches the envelope's own issuer. What binds the two parties is the **quote**, which names `consumer_did` and is issued by the provider — so the agreement pair's "one attestation from each of the two named parties" is fully checkable | The conversation interface addresses a service, not a person (Gap 5), and `peer_person_did` is only known when a contact carries it. Claiming "this card is from the person you are talking to" would be exactly the confidence the carried-forward-limits table forbids: *"two different strengths, two different words in the UI"* |
| **D-C7-12** | **`amount_minor` is the total the consumer owes, inclusive. `tax_minor` and `fees_minor` are informational breakdowns and must satisfy `tax_minor + fees_minor <= amount_minor`.** | The listing's `PaymentTerms` leaves `tax_included` to say whether tax is inside `amount_minor`, which is ambiguous the moment two parties have to agree on one number. A quote is that agreement, so it states one total and checks the parts against it |
| **D-C7-13** | **A quote's expiry rides the envelope (`expires_at_secs`), and the agreement receipt copies it into `AgreedTerms.quote_expires_at_secs` as a record of what it was.** The receipt itself never expires | `F6`: an envelope expiry is enforced by the signer and by every verifier, which is failure-matrix row 9 with no product code. But an agreement made inside the window must stand after it, so the receipt must not inherit the quote's expiry — it records it instead |
| **D-C7-14** | **The currency minor-unit exponent table moves to `roym_core::money` and becomes the source of truth; `PaymentTerms::validate` refuses a code this build does not know.** A Rust test pins `editor.ts`'s copy to it, the way `card.rs` already pins `registry.ts` | The backlog row targeted at this slice. A non-Hub signer can otherwise mint a mis-scaled `amount_minor`, and a consumer rendering a quote must re-derive the same table to display it. Refusing an unknown code rather than assuming two is the honest floor; the product is pre-release, so the behaviour changes in place with no ladder |
| **D-C7-15** | **`transaction` gets its own export/import bundle, with four sections.** | Every other stateful Roym service has one, R1's identity row says a restore reproduces history, and R2's export row (C8) will need agreements and receipts to already be exportable. Four sections, not one composed bundle: `D-C4-…`'s "a single signed bundle that combines them comes later" still stands |
| **D-C7-16** | **`payment-request` does not enter `RECORD_TYPES` in this slice.** | `D-06C-12` settles that it *is* a signed record. C7 builds no producer for it (it is journey step C17, C8's). A record type with no producer is a claim the tree does not back; C8 adds the row with the verb that mints it. Recorded in §15 so C8 inherits it explicitly |

---

## §3 `syneroym-roym-core` — the shared vocabulary

### 3.1 `src/money.rs` — new file

```rust
//! ISO-4217 minor units. The one place this product decides how many
//! minor units a currency has, so a signed amount means the same thing to
//! whoever renders it. A signed payload may hold no non-integer number,
//! so an amount is always minor units plus a code, and the code has to be
//! one this build knows.

/// Currencies with no minor unit at all.
pub const EXPONENT_0: &[&str] = &[
    "BIF","CLP","DJF","GNF","ISK","JPY","KMF","KRW","PYG",
    "RWF","UGX","UYI","VND","VUV","XAF","XOF","XPF",
];
/// Currencies with three minor digits.
pub const EXPONENT_3: &[&str] = &["BHD","IQD","JOD","KWD","LYD","OMR","TND"];

/// `None` for a code this build does not know. Callers refuse rather than
/// assume two -- assuming two signs a Kuwaiti dinar a thousand times low.
#[must_use]
pub fn currency_minor_exponent(code: &str) -> Option<u32>;

/// Three uppercase ASCII letters. Shape only; `currency_minor_exponent`
/// decides whether the code is one this build knows.
#[must_use]
pub fn is_currency_shape(code: &str) -> bool;
```

`currency_minor_exponent` returns `Some(0)` / `Some(3)` for the two lists,
`Some(2)` for any other well-shaped three-letter uppercase code that is a
recognised ISO-4217 alphabetic code, and `None` otherwise. Keeping a full
ISO-4217 list in the crate is not worth it; take the pragmatic rule and say
so in the doc comment: **any well-shaped three-uppercase-letter code that
is not in either exception list is exponent 2**, and `None` is returned
only for a badly shaped code. This keeps the function total for real
currencies, matches `editor.ts` exactly, and is the shape the pinning test
below can actually check.

Tests in this module:

- `exponent_0_and_3_lists_are_sorted_and_disjoint`.
- `an_unshaped_code_has_no_exponent` — `"us"`, `"USDX"`, `"usd"`, `""`.
- `the_ui_currency_table_matches_this_crate` — reads
  `../roym_web/ui/src/listings/editor.ts`, parses the `EXPONENT_0` and
  `EXPONENT_3` `new Set([...])` literals, and asserts each equals the Rust
  slice. Modelled exactly on
  `card::tests::the_ui_card_registry_matches_this_crate`
  (`crates/roym_core/src/card.rs`).

### 3.2 `src/listing.rs` — one behaviour change

Replace the currency shape check in `ListingPayload::validate`
(`crates/roym_core/src/listing.rs:300-302`):

```rust
// before
if p.currency.len() != 3 || !p.currency.chars().all(|c| c.is_ascii_uppercase()) {
    return Err(ListingError::CurrencyShape(p.currency.clone()));
}
// after
if crate::money::currency_minor_exponent(&p.currency).is_none() {
    return Err(ListingError::CurrencyShape(p.currency.clone()));
}
```

The variant and its message stay as they are, so no caller changes. **Call
sites to check for a fixture using a made-up code:**
`crates/roym_web/tests/dual_build_parity.rs` (`full_listing_params`),
`crates/roym_core/src/listing.rs`'s own tests,
`crates/roym_web/ui/src/listings/editor.test.ts`,
`crates/substrate/tests/roym_conversation_e2e.rs`,
`crates/substrate/tests/roym_directory_e2e.rs`,
`crates/substrate/tests/e2e/tests/roym-hub.spec.ts`. Grep for `currency`
in each.

### 3.3 `src/card.rs` — the wire wrapper

Add to the existing file (leave `CARD_TYPES` and `is_known_card` alone):

```rust
use serde::{Deserialize, Serialize};

/// The reserved content type a card message carries. A client that does
/// not understand it sees an ordinary message of an unknown type, which
/// is the honest failure mode.
pub const CARD_CONTENT_TYPE: &str = "application/vnd.roym.card+json";

/// The wrapper's own version, distinct from the card type's version. It
/// says how to read the three fields below; the type's version says which
/// template renders them.
pub const CARD_WRAPPER_VERSION: u32 = 1;

/// A card as it travels. It carries the signed envelope and nothing
/// derived from it: the receiving node verifies and projects, so a sender
/// cannot supply a rendering that disagrees with what it signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub card_version: u32,
    #[serde(rename = "type")]
    pub card_type: String,
    pub version: u32,
    /// The signed envelope, exactly as the host returned it.
    pub envelope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardError {
    #[error("card body is not valid JSON: {0}")]
    Json(String),
    #[error("card wrapper version {0} is not understood by this build")]
    UnknownWrapperVersion(u32),
    #[error("card body is over the {MAX_CARD_BODY_BYTES}-byte maximum")]
    TooLarge,
}

/// Twice the envelope's own payload ceiling, leaving room for the
/// envelope's own fields and the wrapper. A body over this is refused
/// before it is parsed.
pub const MAX_CARD_BODY_BYTES: usize = 160 * 1024;

/// Parses a card body. Refuses an unknown wrapper version rather than
/// guessing; an unknown `(type, version)` is **not** refused here, because
/// the neutral unknown block is a rendering decision, not a parse failure.
pub fn parse_card(body: &str) -> Result<Card, CardError>;

/// The body a card message carries for `envelope`.
pub fn card_body(card_type: &str, version: u32, envelope: &str) -> Result<String, CardError>;
```

Tests: round trip; an unknown wrapper version is refused; an unknown
`(type, version)` parses; an oversize body is refused; a body with extra
keys parses (serde ignores them) but a body missing `envelope` does not.

### 3.4 `src/transaction.rs` — new file, the record vocabulary

```rust
//! The three signed records of an offer: what the consumer asked for,
//! the exact terms the provider offered, and each party's attestation
//! that they accepted those terms.
//!
//! Every number here is an integer, for the reason `listing` states:
//! a signed payload may hold no number that is not an integer, so money
//! is minor units with an explicit currency and geography is
//! micro-degrees.
//!
//! An attestation has one issuer and one signature. "Signed by both" is a
//! completeness rule over a pair of independent attestations of the same
//! record type, one from each party the quote names; neither half
//! references the other, and no signature carries a condition.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_signed_record::{EnvelopeError, content_digest};

use crate::{area::{Area, AreaError}, listing::ServiceLocation, money};

pub const REQUEST_VERSION: u32 = 1;
pub const QUOTE_VERSION: u32 = 1;
pub const AGREEMENT_RECEIPT_VERSION: u32 = 1;

pub const RECORD_REQUEST: &str = "request";
pub const RECORD_QUOTE: &str = "quote";
pub const RECORD_AGREEMENT_RECEIPT: &str = "agreement-receipt";

const REQUEST_ID_PREFIX: &str = "req_";
const QUOTE_ID_PREFIX: &str = "quo_";

pub const MAX_DESCRIPTION_LEN: usize = 4096;
pub const MAX_SCOPE_LEN: usize = 4096;
pub const MAX_TERMS_TEXT_LEN: usize = 2048;   // cancellation, refund, dispute
pub const MAX_NOTICE_LEN: usize = 2048;
pub const MAX_ADDRESS_LEN: usize = 512;
pub const MAX_CATEGORIES: usize = 8;
pub const MAX_CATEGORY_LEN: usize = 64;
pub const MAX_PAYEE_LEN: usize = 256;
pub const MAX_PAYMENT_METHODS: usize = 16;
pub const MAX_PAYMENT_METHOD_LEN: usize = 32;
/// A quote may not be offered open-endedly; failure-matrix row 9 is the
/// reason there is a ceiling as well as a floor.
pub const MIN_QUOTE_LIFETIME_SECS: u64 = 300;
pub const MAX_QUOTE_LIFETIME_SECS: u64 = 90 * 24 * 3600;
```

Types:

```rust
/// When the quote says money changes hands. A signed term, used to drive
/// what the product asks next, never to gate a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentTiming { BeforeWork, AfterWork }

/// Which of the two parties a receipt half is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role { Consumer, Provider }

/// A window in unix seconds. `latest_secs` is inclusive and must not be
/// before `earliest_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow { pub earliest_secs: u64, pub latest_secs: u64 }

/// Where the work happens, as a quote states it. `address` is the one
/// field the spec's disclosure rule is about: it is present only when the
/// work is at the customer, it is part of a signed record both parties
/// keep, and the product says so before it is filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteLocation {
    #[serde(rename = "where")]
    pub where_: ServiceLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<Area>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

/// The one struct the quote states and both halves of the agreement
/// receipt carry verbatim. Every field the Records table names for an
/// `agreement-receipt` is here: payee, expiry, cancellation and refund
/// terms, and dispute path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreedTerms {
    pub scope: String,
    /// ISO-4217, and a code `money::currency_minor_exponent` knows.
    pub currency: String,
    /// The total the consumer owes, inclusive, in minor units.
    pub amount_minor: i64,
    /// Informational breakdowns of `amount_minor`, never additions to it.
    pub tax_minor: i64,
    pub fees_minor: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payment_methods: Vec<String>,
    /// Bound here. A later chat message naming another payee changes
    /// nothing, and the product shows this one.
    pub payee: String,
    pub payment_timing: PaymentTiming,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<TimeWindow>,
    pub location: QuoteLocation,
    pub cancellation_terms: String,
    pub refund_terms: String,
    pub dispute_path: String,
    /// What the quote's own envelope expiry was. A record of the window
    /// acceptance had to fall inside; the receipt does not expire.
    pub quote_expires_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPayload {
    /// `content_digest("req_", {conversation, issuer, sequence})`.
    pub request_id: String,
    /// The conversation this request was made in. Both parties compute
    /// the same value, so it is the one thing a receiver can check the
    /// id against.
    pub conversation: String,
    /// The nth request this issuer made in this conversation, from 1.
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub description: String,
    /// Approximate, deliberately. An exact address is disclosed inside a
    /// quote, when the work needs one, and never here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<Area>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    /// What the consumer was told about how this data is used, recorded
    /// under their own signature so the notice is part of the record.
    pub data_use_notice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotePayload {
    /// `content_digest("quo_", {conversation, issuer, sequence})`.
    pub quote_id: String,
    pub conversation: String,
    pub sequence: u32,
    /// The `record_id` of the request version this answers.
    pub request_record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    /// The request's own issuer. This is what names the second party, and
    /// it is what makes the agreement pair checkable.
    pub consumer_did: String,
    pub terms: AgreedTerms,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreementReceiptPayload {
    /// The quote's own `record_id`. A content hash of the whole quote
    /// envelope, so it pins the terms exactly.
    pub quote_record_id: String,
    pub consumer_did: String,
    pub provider_did: String,
    /// The only field the two halves of a complete pair differ in.
    pub role: Role,
    pub terms: AgreedTerms,
}
```

Free functions:

```rust
pub fn derive_request_id(conversation: &str, issuer: &str, sequence: u32)
    -> Result<String, EnvelopeError>;   // content_digest("req_", {conversation, issuer, sequence})
pub fn derive_quote_id(conversation: &str, issuer: &str, sequence: u32)
    -> Result<String, EnvelopeError>;   // content_digest("quo_", …)

impl TimeWindow      { pub fn validate(&self) -> Result<(), TransactionError>; }
impl QuoteLocation   { pub fn validate(&self) -> Result<(), TransactionError>; }
impl AgreedTerms     { pub fn validate(&self) -> Result<(), TransactionError>; }
impl RequestPayload  { pub fn validate(&self) -> Result<(), TransactionError>; }
impl QuotePayload    { pub fn validate(&self) -> Result<(), TransactionError>; }
impl AgreementReceiptPayload { pub fn validate(&self) -> Result<(), TransactionError>; }

/// True when two halves of a pair agree on everything a pair must agree
/// on: every field except `role`.
#[must_use]
pub fn halves_agree(a: &AgreementReceiptPayload, b: &AgreementReceiptPayload) -> bool;
```

`AgreedTerms::validate` rules, in order:

1. `scope` non-empty and `<= MAX_SCOPE_LEN`.
2. `money::currency_minor_exponent(&currency).is_some()`, else
   `CurrencyUnknown`.
3. `amount_minor >= 0`, `tax_minor >= 0`, `fees_minor >= 0`.
4. `tax_minor.checked_add(fees_minor)` exists and is `<= amount_minor`
   (`D-C7-12`), else `BreakdownExceedsTotal`.
5. `payee` non-empty, `<= MAX_PAYEE_LEN`.
6. `payment_methods.len() <= MAX_PAYMENT_METHODS`, each
   `<= MAX_PAYMENT_METHOD_LEN`.
7. `schedule`, if present, validates.
8. `location` validates: `address` present only when
   `where_ == ServiceLocation::AtCustomer` (else `AddressNotApplicable`),
   `address` `<= MAX_ADDRESS_LEN`, `area` validates when present.
9. `cancellation_terms`, `refund_terms`, `dispute_path` each non-empty and
   `<= MAX_TERMS_TEXT_LEN`. Non-empty deliberately: the Records table says
   an `agreement-receipt` proves these were agreed, and an empty string is
   not an agreement.
10. `quote_expires_at_secs != 0`.

`QuotePayload::validate` adds: `quote_id` non-empty; `conversation`
non-empty; `sequence >= 1`; `request_record_id` starts with `rec_`;
`consumer_did` passes `person::is_did_key`; `terms.validate()`.

`RequestPayload::validate` adds: `request_id` non-empty; `conversation`
non-empty; `sequence >= 1`; `description` non-empty and
`<= MAX_DESCRIPTION_LEN`; `categories.len() <= MAX_CATEGORIES` with each a
lowercase `[a-z0-9-]` token `<= MAX_CATEGORY_LEN` (reuse the same rule
`listing.rs` uses — lift `valid_token` there to `pub(crate)` rather than
copying it); `area` validates when present; `window` validates when
present; `data_use_notice` `<= MAX_NOTICE_LEN`.

Verification bodies, mirroring `listing::verify_envelope` exactly — one
body per record type, pure, no host, no storage:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordVerdict<P> {
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub revocation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub issued_at_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub expires_at_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub supersedes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub payload: Option<P>,
}

pub fn verify_request(envelope: &str, now_secs: u64) -> RecordVerdict<RequestPayload>;
pub fn verify_quote(envelope: &str, now_secs: u64) -> RecordVerdict<QuotePayload>;
pub fn verify_agreement_receipt(envelope: &str, now_secs: u64)
    -> RecordVerdict<AgreementReceiptPayload>;
```

Each does, in order: `record::verify_json` with `VerifyOptions::new(now)`;
refuse unless `record_type` and `version` are this build's; deserialize the
payload; `payload.validate()`; for `request`/`quote`, re-derive the id from
`(payload.conversation, verified.issuer, payload.sequence)` and refuse a
mismatch (`F14`'s rule); for `agreement-receipt`, refuse unless
`envelope.subject == payload.quote_record_id`, unless
`payload.role == Consumer` implies `verified.issuer == payload.consumer_did`,
and unless `payload.role == Provider` implies
`verified.issuer == payload.provider_did`. `revocation_status` is
`listing::revocation_status_word(verified.revocation_status)` — reuse it,
do not spell the words a second time.

Unit tests in this module (all pure, no host):

- Each payload's `validate` rejects each of its own rules, one test per rule
  family.
- `derive_request_id` / `derive_quote_id` are stable and differ on any
  input change.
- `verify_quote` refuses a payload whose `quote_id` was derived from a
  different issuer.
- `verify_agreement_receipt` refuses a half whose `role` does not match its
  issuer.
- `halves_agree` is true for two halves differing only in `role`, false for
  any other single-field difference — write it as a table over every field.
- An expired quote envelope verifies `false` with an `Expired` reason
  (`F6`).

### 3.5 `src/agreement.rs` — or fold into `transaction.rs`

Keep the pair logic beside the payloads, in `transaction.rs`:

```rust
/// One party's attestation, as it is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptHalf {
    pub envelope: String,
    pub record_id: String,
    pub issuer: String,
    pub issued_at_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum PairState {
    /// Neither party has attested.
    None,
    /// Exactly one half exists.
    Half { role: Role },
    /// One attestation from each of the two parties the quote names, with
    /// identical terms and each inside its own validity window.
    Complete,
}
```

`pair_state(consumer: Option<&ReceiptHalf>, provider: Option<&ReceiptHalf>)
-> PairState` returns `Complete` only when both are present. The stronger
checks (identical terms, issuer matches role) are made when each half is
**filed**, not when the pair is read — a half that failed them is never
stored as a half. Say that in the doc comment.

### 3.6 `src/router.rs` — one new prefix

Add to `ROUTES`, after the four transaction prefixes:

```rust
    // The certificate verbs (`transaction.signing-status` /
    // `transaction.install-signing-certificate`) reach the transaction
    // service through its own name, and `transaction.sync` / `.thread` /
    // `.export` / `.import` ride the same prefix.
    ("transaction.", TRANSACTION, MethodAuth::Owner),
```

Extend `every_certificate_mounted_service_routes_under_its_own_name`
(`crates/roym_core/src/router.rs:214`) to
`["profile", "catalog", "conversation", "transaction"]`.

Extend `every_declared_dependency_names_a_sibling_and_the_three_edges_are_present`
to assert `transaction`'s `depends_on` contains `conversation`, and rename
it — it now checks four edges. Prefer a name that does not count:
`every_declared_dependency_names_a_sibling_and_the_named_edges_are_present`.

`no_prefix_is_a_prefix_of_another` already passes: `transaction.` shares no
prefix with `request.` / `quote.` / `agreement.` / `receipt.` / `catalog.`
/ `directory.` / `member.` / `conversation.` / `profile.` / `contacts.` /
`block.` / `report.` / `listing.` / `availability.`.

### 3.7 `src/backup.rs` — four new section names

```rust
pub const SECTION_REQUESTS: &str = "requests";
pub const SECTION_QUOTES: &str = "quotes";
pub const SECTION_AGREEMENTS: &str = "agreements";
pub const SECTION_CARDS: &str = "cards";
```

### 3.8 `src/lib.rs`

```rust
pub mod money;
pub mod transaction;
```
inserted in alphabetical order among the existing `pub mod` lines.

---

## §4 `syneroym-roym-transaction` — the service

No `Cargo.toml` change, no `wit/world.wit` change: the crate already
depends on `syneroym-app-host` and `syneroym-roym-core` and already
imports `syneroym:proxy`, `syneroym:signing`, `syneroym:data-layer` and
`syneroym:invocation`. `cargo xtask check-roym-deps`'s allowlist is
unchanged.

### 4.1 Collections

| Collection | Id | Payload | Indexes |
|---|---|---|---|
| `requests` | `request_id` | `RecordPointerRow` | `conversation` (string), `updated_at_secs` (numeric) |
| `request_history` | `record_id` | the envelope JSON, as bytes | none |
| `quotes` | `quote_id` | `RecordPointerRow` + `request_record_id`, `consumer_did` | `conversation` (string), `updated_at_secs` (numeric) |
| `quote_history` | `record_id` | the envelope JSON, as bytes | none |
| `agreements` | `quote_record_id` | `AgreementRow` | `conversation` (string) |
| `cards` | the conversation **message id** | `CardRow` | `conversation` (string), `sender_timestamp_ms` (numeric) |
| `sync_state` | conversation id | `{ "scanned_count": u64 }` | none |
| `signing_certificates` | `"current"` | `StoredCertificate` | (created by `roym_core::signing`) |

Declared indexes are still not used by `query` (backlog §11's own row); they
are declared for the same reason `catalog` declares them — the shape is
right and the fix is a substrate change.

Row types, in `roym_transaction::app`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordPointerRow {
    envelope: String,
    record_id: String,
    /// `request_id` or `quote_id`.
    id: String,
    conversation: String,
    sequence: u32,
    issuer: String,
    /// `mine` when this installation's owner issued it.
    mine: bool,
    updated_at_secs: u64,
    version_count: u64,
    /// Quotes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumer_did: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgreementRow {
    quote_record_id: String,
    conversation: String,
    consumer_did: String,
    provider_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumer: Option<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<ReceiptHalf>,
    updated_at_secs: u64,
}

/// One rendered row. Everything a client needs to draw a card, and the
/// node's own verdict on it -- never the sender's claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CardRow {
    message_id: String,
    conversation: String,
    direction: Direction,            // roym_core::conversation::Direction
    sender_timestamp_ms: i64,
    card_type: String,
    version: u32,
    /// False when `(card_type, version)` is not in `CARD_TYPES`. The
    /// client draws the neutral block; nothing here is parsed.
    known: bool,
    verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")] reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] revocation_status: Option<String>,
    /// The verified payload, projected by this node. Absent unless
    /// `verified`.
    #[serde(skip_serializing_if = "Option::is_none")] data: Option<Value>,
    stored_at_secs: u64,
}
```

`SCHEMA_VERSION` in `roym_transaction::app` goes 1 → 2, with the same
comment the other services carry: *"Bumped in this slice: the service
gains its first state."*

Constants, in `roym_core::transaction` so both the service and the tests
read one definition:

```rust
/// How many messages one `sync` page asks `conversation.history` for.
pub const SYNC_PAGE: u32 = 200;
/// How many pages one `sync` reads. Bounds the dispatch: a guest's
/// wall-clock budget is spent while it waits on a host call, and an
/// unbounded page loop is how that budget is exhausted.
pub const MAX_SYNC_PAGES: u32 = 5;
/// How far behind its own watermark a `sync` re-reads. `history` orders by
/// sender timestamp, so a message that arrives late inserts before the
/// watermark; this is the window in which that is invisible.
pub const SYNC_OVERLAP: u64 = 50;
/// Cards one conversation keeps. A conversation past this stops filing
/// new ones rather than growing without bound.
pub const MAX_CARDS_PER_CONVERSATION: usize = 2_000;
```

### 4.2 Verb table

```rust
pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    if let Some(resp) = signing::handle_certificate_verb(host, "transaction.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "request.ping" | "quote.ping" | "agreement.ping" | "receipt.ping" =>
            Response::ok(json!({ "service": services::TRANSACTION.name })),

        "request.set"      => request_set(host, &req).await,
        "request.get"      => request_get(host, &req).await,
        "request.list"     => request_list(host, &req).await,
        "request.history"  => request_history(host, &req).await,
        "request.verify"   => verify_verb(host, &req, RecordKind::Request).await,

        "quote.set"        => quote_set(host, &req).await,
        "quote.get"        => quote_get(host, &req).await,
        "quote.list"       => quote_list(host, &req).await,
        "quote.history"    => quote_history(host, &req).await,
        "quote.verify"     => verify_verb(host, &req, RecordKind::Quote).await,

        "agreement.accept" => agreement_accept(host, &req).await,
        "agreement.get"    => agreement_get(host, &req).await,
        "agreement.list"   => agreement_list(host, &req).await,
        "agreement.verify" => verify_verb(host, &req, RecordKind::AgreementReceipt).await,

        "transaction.sync"   => sync(host, &req).await,
        "transaction.thread" => thread(host, &req).await,
        "transaction.export" => export(host).await,
        "transaction.import" => import(host, &req).await,
        other => Response::method_not_found(other),
    }
}
```

The four pings stay: `request.ping` is `WIRE_REFUSED_VERBS`' representative
for this service and `receipt.ping` is scenario 5's probe. No `receipt.*`
verb beyond the ping exists in this slice — `fulfilment-receipt` is C8's,
and the prefix stays routed and empty rather than being removed and
re-added.

### 4.3 `request.set`

```
params: {
  conversation: String,            // required
  request_id: String | null,       // present => this is a new version of that request
  listing_id: String | null,
  categories: [String],
  description: String,             // required
  area: Area | null,
  window: { earliest_secs, latest_secs } | null,
  data_use_notice: String
}
result: { request_id, record_id, version_count, message_id, state }
```

Pseudo-code:

```
now = clock::now_secs()
(principal, owner) = resolve_principal_and_owner(host, now)?       // lifted from catalog, see 4.9
conversation = params.conversation or -32602

if params.request_id is Some(id):
    prior = load_pointer(host, REQUESTS, id)? or -32602 "no such request"
    if !prior.mine                       -> -32602 "this request is not yours to revise"
    if prior.conversation != conversation-> -32602 "request belongs to another conversation"
    sequence = prior.sequence
    supersedes = Some(prior.record_id)
    next_count = prior.version_count + 1
else:
    sequence = count_mine(host, REQUESTS, conversation, owner) + 1   // §4.10
    supersedes = None
    next_count = 1

request_id = transaction::derive_request_id(&conversation, &owner, sequence)?
payload = RequestPayload { request_id, conversation, sequence, ... }
payload.validate()? -> -32602

envelope_json = sign(host, principal, RecordDraft {
    version: REQUEST_VERSION,
    record_type: RECORD_REQUEST,
    subject: request_id.clone(),
    payload: serde_json::to_string(&payload)?,     // F7: the host draft takes a string
    expires_at_secs: None,
    supersedes,
})?
envelope = Envelope::from_json(&envelope_json)?
if envelope.issuer != owner -> internal_error
   "the host signed under an issuer this service did not ask for"
record_id = envelope.record_id()?

// Store before sending: a signed record that was never sent is
// recoverable; a card the peer holds that this node never stored is not.
put(REQUEST_HISTORY, record_id, envelope_json.bytes)
put(REQUESTS, request_id, RecordPointerRow { .., mine: true, .. })

body = card::card_body(RECORD_REQUEST, REQUEST_VERSION, &envelope_json)?
sent = conversation_call(host, "conversation.send", {
    conversation, body, content_type: card::CARD_CONTENT_TYPE })
// A send that fails leaves the record stored and unsent; the response
// says so rather than rolling the record back.
message_id / state from `sent`, or `state: "not-sent"` with the error
   surfaced in `send_error`

file_own_card(host, message_id, conversation, RECORD_REQUEST, REQUEST_VERSION,
              &envelope_json, sender_timestamp_ms)       // §4.11

ok { request_id, record_id, version_count: next_count, message_id, state }
```

Ordering note to put in the code as a comment: *"The record is stored
before the card is sent. A send that fails leaves a signed record this node
holds and the peer does not, which a later `set` or a retry repairs; the
reverse — a card the peer holds and this node cannot show — has no
repair."* This is the opposite of `catalog`'s pointer-then-history order
and for a different reason, so it needs its own sentence.

### 4.4 `quote.set`

```
params: {
  request_record_id: String,       // required; the request version being answered
  quote_id: String | null,         // present => a new version of that quote
  listing_id: String | null,
  expires_in_secs: u64,            // required; MIN..=MAX_QUOTE_LIFETIME_SECS
  terms: AgreedTerms-without-quote_expires_at_secs
}
result: { quote_id, record_id, version_count, expires_at_secs, message_id, state }
```

```
now = clock::now_secs()
(principal, owner) = resolve_principal_and_owner(host, now)?

req_envelope = get(REQUEST_HISTORY, request_record_id)? or -32602 "no such request"
verdict = transaction::verify_request(&req_envelope, now)
if !verdict.verified -> -32602 "the request this quote answers does not verify: {reason}"
consumer_did = verdict.issuer
if consumer_did == owner -> -32602 "a request cannot be quoted by the person who made it"
conversation = verdict.payload.conversation

if params.quote_id is Some(id):  (same revision shape as request.set, plus)
    if prior.request_record_id != Some(request_record_id)
        -> -32602 "a new version of a quote answers the same request"
else: sequence = count_mine(QUOTES, conversation, owner) + 1

if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(expires_in_secs)
   -> -32602
expires_at_secs = now + expires_in_secs

quote_id = derive_quote_id(&conversation, &owner, sequence)?
terms = AgreedTerms { ..params.terms, quote_expires_at_secs: expires_at_secs }
payload = QuotePayload { quote_id, conversation, sequence, request_record_id,
                         listing_id, consumer_did, terms }
payload.validate()? -> -32602

sign with RecordDraft { version: QUOTE_VERSION, record_type: RECORD_QUOTE,
                        subject: quote_id, payload: to_string(&payload),
                        expires_at_secs: Some(expires_at_secs), supersedes }
   // D-C7-13: the expiry rides the envelope, so every verifier enforces it
store, send as a card, file own card -- exactly as request.set
```

Note the interaction with `F6`/the pinned parity clock: the host stamps
`issued_at_secs` from its own `RecordClock`, and `RecordDraft::validate`
compares `expires_at_secs` against **that** stamp, not against the guest's
`clock::now_secs()`. In the parity harness the signing clock is pinned 240 s
ahead of wall time, so `expires_in_secs` must exceed 240 for a quote signed
there to be valid. `MIN_QUOTE_LIFETIME_SECS = 300` covers it; say so in the
constant's doc comment without naming a test.

### 4.5 `agreement.accept`

```
params: { quote_record_id: String }
result: { quote_record_id, role, record_id, pair: PairState, message_id, state }
```

```
now = clock::now_secs()
(principal, owner) = resolve_principal_and_owner(host, now)?

quote_envelope = get(QUOTE_HISTORY, quote_record_id)? or -32602 "no such quote"
v = verify_quote(&quote_envelope, now)
if !v.verified:
    if reason names an expiry -> -32602 "quote-expired"
    else                      -> -32602 "the quote does not verify: {reason}"
provider_did = v.issuer
consumer_did = v.payload.consumer_did
role = if owner == consumer_did { Consumer }
       else if owner == provider_did { Provider }
       else -> -32602 "this installation is neither party to that quote"

row = load(AGREEMENTS, quote_record_id) or new AgreementRow { .. }
if row.half(role).is_some():
    // Idempotent: accepting twice returns the half already made rather
    // than minting a second attestation over the same terms.
    return ok { .., record_id: existing.record_id, pair: pair_state(..) }

payload = AgreementReceiptPayload {
    quote_record_id, consumer_did, provider_did, role,
    terms: v.payload.terms.clone(),        // verbatim, D-C7-9
}
sign RecordDraft { version: AGREEMENT_RECEIPT_VERSION,
                   record_type: RECORD_AGREEMENT_RECEIPT,
                   subject: quote_record_id.clone(),
                   payload: to_string(&payload),
                   expires_at_secs: None,       // D-C7-13
                   supersedes: None }           // a correction is a new record
                                                // referencing the same subject
store the half in row, put(AGREEMENTS, quote_record_id, row)
send as a card, file own card
ok { .., pair: pair_state(&row.consumer, &row.provider) }
```

`supersedes: None` deliberately. An attestation is not versioned the way a
request or a quote is: `D-06C-12` fixes it as a single statement about a
fixed subject, and a change of mind is a different record type (C8's
cancellation), never an edit. Say so in a comment.

### 4.6 `transaction.sync`

```
params: { conversation: String, full: bool = false }
result: { scanned, filed, refused, unknown, countersigned, scanned_count }
```

```
now = clock::now_secs()
conversation = params.conversation or -32602
ensure collections

state = load(SYNC_STATE, conversation) or { scanned_count: 0 }
start = if params.full { 0 } else { state.scanned_count.saturating_sub(SYNC_OVERLAP) }

// One query, not one read per message: the ids this node already filed.
already = set of `cards` row ids where row.conversation == conversation
card_count = already.len()

offset = start
pages = 0
loop {
    page = conversation_call("conversation.history",
              { conversation, limit: SYNC_PAGE, cursor: offset })
    msgs = page.messages
    for m in msgs:
        offset += 1
        if already.contains(m.id): continue
        if m.content_type != card::CARD_CONTENT_TYPE: continue
        if m.deleted_at_secs.is_some(): continue
        if card_count >= MAX_CARDS_PER_CONVERSATION: break the loop
        file_incoming_card(host, &m, conversation, now)   // §4.7
        card_count += 1
    pages += 1
    if msgs.len() < SYNC_PAGE as usize or pages >= MAX_SYNC_PAGES: break
}

put(SYNC_STATE, conversation, { scanned_count: max(state.scanned_count, offset) })
```

`conversation.history`'s response shape is `{ "messages": [MessageRow] }`
with `cursor` as an integer offset (`crates/roym_conversation/src/app.rs:648-654`).
`MessageRow.body` is the card JSON as UTF-8 (`F4`), `direction` is
`incoming`/`outgoing`, `content_type` is preserved.

### 4.7 `file_incoming_card`

The one place a stranger's bytes become a stored row.

```
row = CardRow {
    message_id: m.id, conversation, direction: m.direction,
    sender_timestamp_ms: m.sender_timestamp_ms,
    card_type: "", version: 0, known: false, verified: false,
    reason: None, .., stored_at_secs: now,
}

body = m.body or -> row.reason = "no body";           put(CARDS, ..); return Ok
card = card::parse_card(&body) or -> row.reason = e;  put; return Ok
row.card_type = card.card_type; row.version = card.version
row.known = card::is_known_card(&card.card_type, card.version)
if !row.known:
    // The neutral block's data. Nothing is parsed and nothing is guessed.
    put(CARDS, ..); return Ok

match card.card_type:
  "request" =>
     v = verify_request(&card.envelope, now)
     if !v.verified                    -> refuse(v.reason)
     if v.payload.conversation != conversation
                                       -> refuse("card names another conversation")
     // D-C7-5: the card's declared type is the signed record's type, by
     // construction of verify_request. Nothing further to check.
     store_received_request(host, &card.envelope, &v)      // pointer + history
     row.verified = true; row.data = Some(json!(v.payload)); row.issuer = v.issuer;
     row.record_id = v.record_id; row.revocation_status = v.revocation_status

  "quote" =>
     v = verify_quote(&card.envelope, now)
     if !v.verified                    -> refuse(v.reason)
     if v.payload.conversation != conversation -> refuse(..)
     // The quote must answer a request this node holds, so the two-party
     // binding is anchored in something signed rather than in the
     // conversation's own addressing (D-C7-11).
     if get(REQUEST_HISTORY, v.payload.request_record_id).is_none()
                                       -> refuse("answers a request this node does not hold")
     store_received_quote(host, &card.envelope, &v)
     row.verified = true; row.data = Some(json!(v.payload)); ..

  "agreement-receipt" =>
     v = verify_agreement_receipt(&card.envelope, now)
     if !v.verified                    -> refuse(v.reason)
     q = get(QUOTE_HISTORY, v.payload.quote_record_id)
         or -> refuse("attests a quote this node does not hold")
     qv = verify_quote(&q, now)   // may be Expired; that is not a refusal here
     // The terms in the half must be the terms the quote stated, byte for
     // byte. A half that agrees to something else is not half of a pair.
     if qv.payload.terms != v.payload.terms -> refuse("terms differ from the quote")
     if v.payload.consumer_did != qv.payload.consumer_did
        or v.payload.provider_did != qv.issuer -> refuse("names the wrong parties")
     // The acceptance had to fall inside the quote's own window.
     if v.issued_at_secs >= v.payload.terms.quote_expires_at_secs
                                            -> refuse("accepted after the quote expired")
     row_agr = load(AGREEMENTS, quote_record_id) or new
     if row_agr.half(v.payload.role).is_none():
         row_agr.set_half(v.payload.role, ReceiptHalf { .. })
         put(AGREEMENTS, ..)
     row.verified = true; row.data = Some(json!(v.payload)); ..
     maybe_countersign(host, &row_agr, &qv, now)      // §4.8

  _ => refuse("a known card type with no producer in this build")
        // booking-progress / payment-request / payment-acknowledgement /
        // fulfilment-receipt: known to the renderer, produced by nothing
        // yet. Stored known and unverified, with a reason, rather than
        // silently.

put(CARDS, m.id, row)
```

Every arm returns `Ok`: a card that could not be verified is a stored
refusal, not a `sync` failure. A storage fault is the one thing that
propagates.

### 4.8 `maybe_countersign` (`D-C7-10`)

```
// The provider's own node completes the pair when the consumer's half
// arrives, because the provider already signed these exact terms in the
// quote it issued: the second attestation states nothing new. Refused,
// and left to the person, on any doubt at all.
if row.provider.is_some()                        { return }   // already paired
if row.consumer.is_none()                        { return }
owner = signing::owner_did(host)?
if owner != row.provider_did                     { return }   // not this node's to make
if qv.payload.terms != consumer_half_terms       { return }   // checked already, restated
if now >= qv.payload.terms.quote_expires_at_secs { return }   // expired: the person decides
(principal, _) = signing::person_principal(host, now)?  // not enrolled => leave the half
sign an AgreementReceiptPayload identical to the consumer half except
    `role: Provider`
store the half, send it as a card, file our own card row
```

`sync`'s result counts it in `countersigned`.

### 4.9 Helpers lifted from `catalog`

`resolve_principal_and_owner` (`crates/roym_catalog/src/app.rs:242-269`),
`ensure_coll`, `idx`, `collect` and the export/import bodies are the same
code in `catalog`, `conversation`, `profile` and `directory` already. **Do
not lift them into `roym_core` in this slice**: four copies exist today and
a fifth is consistent; consolidating them is a refactor touching every
service and belongs in its own change. Add a backlog row instead (§15).

### 4.10 `count_mine`

```
async fn count_mine<H: AppHost>(host, collection, conversation, owner) -> Result<u32, String>
```
Pages `AppDataLayer::query` with
`filter = {"conversation": <id>, "issuer": <owner>}` and counts rows.
Follows the same "sieve at the host, not in the guest" note the C5 review
imposed on `catalog`.

**Known race, stated rather than discovered:** two concurrent
`request.set` calls in one conversation read the same count, derive the
same `request_id`, and the second overwrites the first's pointer row. Same
shape as `catalog`'s `version_count` race, already a backlog row; add this
one beside it (§15).

### 4.11 `file_own_card`

An outgoing card is filed in `cards` too, with `verified: true` and the
payload this node just signed — so `transaction.thread` shows one ordered
list of both sides' cards and the Hub needs no second source. It is filed
without re-verifying: this node produced the envelope one statement ago.

### 4.12 `transaction.thread`

```
params: { conversation: String, limit: u64 = 200, cursor: u64 = 0 }
result: { cards: [CardRow] }
```
Reads `cards` filtered by conversation, sorts by
`(sender_timestamp_ms, message_id)` — the same rule `roym_core::conversation::sort_key`
uses, minus `author` which a `CardRow` does not carry; add
`sort_key`-equivalent ordering inline with a comment saying why the author
component is absent (a card row's author is its `issuer`, which is a person
DID and not the conversation author, so mixing them would order two
transcripts differently). Then `skip(cursor).take(limit)`.

`thread` does **not** call `sync`. A client calls `sync` then `thread`, and
the two verbs stay one job each.

### 4.13 `request.get` / `.list` / `.history`, and the quote trio

Exactly `catalog`'s `get_listing` / `list_listings` / `listing_history`,
retargeted:

- `request.get { request_id }` → `{ envelope, record_id, request_id, conversation, mine, updated_at_secs, version_count }` or `null`.
- `request.list { conversation?, mine? }` → newest first, `offset`/`limit`.
- `request.history { request_id }` → every envelope naming that
  `request_id`, oldest first by `issued_at_secs`, filtered at the host on
  `payload.request_id`.
- `quote.*` the same, plus `request_record_id` and `consumer_did` on the row.

### 4.14 `*.verify`

```
params: { envelope: String | Value }
result: RecordVerdict<…>
```
A thin, pure wrapper over `transaction::verify_request` /
`verify_quote` / `verify_agreement_receipt`, mirroring
`catalog`'s `listing.verify` (`crates/roym_catalog/src/app.rs:733-746`)
including the `let _ = host;` line and the `Value::String` / other handling
of the `envelope` parameter.

### 4.15 `agreement.get` / `.list`

- `agreement.get { quote_record_id }` →
  `{ quote_record_id, conversation, consumer_did, provider_did, consumer, provider, pair, terms }`
  where `terms` is read from the quote (the two halves are byte-identical
  on it) and `pair` is `PairState`. `null` when no row.
- `agreement.list { conversation? }` → the rows, newest first.

### 4.16 `transaction.export` / `.import`

The same body `catalog` has, with four sections:
`SECTION_REQUESTS` ← `requests`, `SECTION_QUOTES` ← `quotes`,
`SECTION_AGREEMENTS` ← `agreements`, `SECTION_CARDS` ← `cards`.

On import, in addition to the shared checks (`check_integrity`,
`subject_did == owner`, `schema_version == SCHEMA_VERSION`):

- A `requests` or `quotes` row's `envelope` is re-verified with
  `verify_request` / `verify_quote`, and a row that does not verify refuses
  the whole import with `-32602` naming the id. This is what makes
  failure-matrix row 13 ("import reproduces verification status") true for
  this service.
- A `cards` row is imported as stored, **not** re-verified, and its
  `verified` flag is recomputed from the envelope it names when the
  corresponding history row is present. Simpler and honest: recompute
  `verified` for every card row that carries a `record_id`, from the
  imported history, and leave `verified: false` with the recorded reason
  otherwise.

`request_history` and `quote_history` are **not** exported as their own
sections: every envelope in them is reachable from a `requests`/`quotes`
pointer row or an `agreements` half. On import, re-populate them from the
imported rows. State this in the export's doc comment — a bundle that
silently drops a collection is the C5 review's own `catalog.export` finding
and must not be repeated without saying so.

---

## §5 The manifest

`crates/roym_core/app/roym.toml`, the `[services.transaction]` block:

```toml
[services.transaction]
service_type = "wasm"
source = "target/wasm32-wasip2/release/syneroym_roym_transaction.wasm"
interfaces = ["syneroym-roym:transaction/api@0.1.0"]
visibility = "public"
# A signed request, quote or agreement receipt travels to the other party
# as a message of the reserved card content type, so this service puts one
# into a conversation and reads one back out of it. The edge points this
# way and only this way: the manifest refuses a cycle, and the inbox
# therefore cannot push into this service.
depends_on = ["conversation"]
```

`visibility` is unchanged (`F9`, `D-C5-4`). No `topology_visibility`:
nothing off this node reaches `transaction`.

---

## §6 `syneroym-roym-web` — nothing changes in Rust

`web` already declares `depends_on` on all five siblings and already routes
`request.` / `quote.` / `agreement.` / `receipt.` to `transaction`
(`crates/roym_core/src/router.rs:34-37`). The one router change is §3.6's
`transaction.` prefix. `crates/roym_web/src/app.rs` is untouched.

---

## §7 The Hub

### 7.1 `src/cards/templates/{request,quote,agreement_receipt}.ts` — rewritten

Each exports a data interface matching the Rust payload's serde shape and a
render function that builds only text nodes and a `renderLink`-guarded
anchor. Rules, restated in each file's header comment:

- Every value is `textContent`. No `innerHTML`, no `insertAdjacentHTML`, no
  `style` from data, no attribute built from data other than a `class` this
  file chose.
- No URL is fetched, prefetched, resolved or navigated to. The one place a
  URL may appear is a payee or a dispute path, and it goes through
  `renderLink`, which already refuses a non-`http(s)` scheme and sets
  `rel="noopener noreferrer"` and no `target`.
- A missing optional field renders as an omitted row, never as `undefined`
  and never as a guess.

`renderQuote` shows, in this order: scope; total (`formatMinor(amount_minor,
currency)`); the tax and fee lines when non-zero; payment timing, in words
(`"Payment before the work"` / `"Payment after the work"`); payee, labelled
*"Payee, as agreed in this quote"*; the schedule window; the location and,
when present, the address, prefixed *"Address given in this quote:"*;
cancellation, refund and dispute text; and the expiry, as an absolute time
plus *"expired"* when past.

`renderAgreementReceipt` shows the same terms plus the role, the two party
DIDs, and the pair state in words: *"Both parties have signed these terms."*
/ *"Only the consumer has signed so far."* / *"Only the provider has signed
so far."* It never says *verified*.

Add `src/cards/money.ts`:

```ts
export function formatMinor(minor: number, currency: string): string
```
using the same exponent table `editor.ts` holds — move
`currencyMinorExponent`, `EXPONENT_0` and `EXPONENT_3` from
`listings/editor.ts` into `src/money.ts`, re-export from `editor.ts` so its
own tests keep working, and point `roym_core::money`'s pinning test at the
new file path.

### 7.2 `src/cards/refused.ts` — new

```ts
export function renderRefusedCard(type: string, version: number, reason?: string): HTMLElement
```
A `.card.card-refused` block with `data-verified="false"`, naming the type,
saying this node could not verify it, and showing the reason as text.
Modelled on the Directory tab's `renderRefused`. `renderCard` stays pure and
gains nothing: the **caller** picks `renderCard` / `renderRefusedCard` from
the node's verdict (`F10`).

### 7.3 `src/screens/messages.ts`

- `MessageRow` gains nothing; the thread loader additionally calls
  `transaction.sync { conversation }` then `transaction.thread { conversation }`
  and builds a `Map<message_id, CardRow>`.
- In `messageElement`, when `m.content_type === "application/vnd.roym.card+json"`:
  look the row up; render `renderCard({type, version, data})` when
  `known && verified`, `renderRefusedCard(...)` when `known && !verified`,
  and `renderCard` (which falls through to `renderUnknown`) when `!known`.
  Never render the raw JSON body.
- A **"Send a request"** form in the thread: description, categories,
  optional listing id, optional window. Posts `request.set`.
- A **"Quote this request"** control on a verified `request` card the person
  did not issue: opens a form for the `AgreedTerms` fields plus
  `expires_in_secs`, converts the amount with `toMinorUnits(amount,
  currencyMinorExponent(currency))` so no decimal is ever sent, and posts
  `quote.set`.
- An **"Accept these terms"** button on a verified `quote` card whose
  `consumer_did` is this person and whose expiry is in the future. Posts
  `agreement.accept`. The button is absent, with a line of text saying the
  quote expired, when it is past.
- Refusals surface as text: `-32602 "quote-expired"` renders *"This quote
  has expired. Ask for a new one."*

### 7.4 `src/screens/backup.ts` (`F11`)

Fourth entry: `{ label: "Requests, quotes, and agreements", exportMethod:
"transaction.export", importMethod: "transaction.import", file:
"roym-transaction-bundle.json" }`. Change "The three bundles below" to
"The four bundles below".

### 7.5 `src/main.ts`

`renderHome`'s gallery samples for `request`, `quote` and
`agreement-receipt` become realistic fixtures matching the new interfaces,
so browser case 3 still finds one of each and case 4 has real fields to
attack. The four C8 samples stay as they are.

### 7.6 vitest

New `src/cards/templates/*.test.ts`:

- Each template renders every field it is given, as text.
- A payload whose every string field is `<img src=x onerror=alert(1)>`
  produces zero elements from that string, `textContent` equal to the raw
  string, and no `<img>` anywhere in the subtree.
- A `javascript:` payee renders as a text node, not an anchor.
- `formatMinor` for exponent 0, 2 and 3 currencies.
- `renderRefusedCard` carries `data-verified="false"` and never the type's
  own class.

Existing `src/listings/editor.test.ts` keeps passing after the money move.

---

## §8 `roymctl`

`apps/roymctl/src/commands/roym.rs`:

1. `SIGNING_SERVICES` becomes
   `&["profile", "catalog", "conversation", "transaction"]` (`F13`).
2. A new `RoymCommands::Transaction { command: TransactionCommands }`,
   mirroring `Directory`'s shape and its `--gateway-url` / `--host`
   argument pattern:

```
roym transaction request --conversation <id> --description <text>
        [--category <token>]... [--listing <id>]
        [--near <lat,lon,radius_m>] [--window <earliest,latest>]
        [--notice <text>]
roym transaction quote --request <record_id> --scope <text>
        --currency <ISO> --amount <decimal> --payee <text>
        [--tax <decimal>] [--fees <decimal>]
        [--method <name>]... --timing before-work|after-work
        [--schedule <start,end>]
        --where at-provider|at-customer|remote [--address <text>]
        --cancellation-file <path> --refund-file <path> --dispute <text>
        --expires-hours <n>
roym transaction accept --quote <record_id>
roym transaction sync --conversation <id> [--full]
roym transaction thread --conversation <id>
roym transaction agreement --quote <record_id>
```

`--amount`, `--tax` and `--fees` are decimals converted to minor units at
this boundary with `roym_core::money::currency_minor_exponent`, never
signed as a decimal — the same rule the Hub editor follows. `--near` uses
the existing `lat,lon,radius_m` → micro-degrees parser that
`directory find` already has; lift it to a shared helper in the same file
rather than copying it.

`thread` prints three blocks, in the `directory find` house style: verified
cards, refused cards with their reasons, and unknown-type cards. Printing
only the verified ones would hide exactly what the product is required to
show.

3. Existing `roym enrol-signing` and `roym signing-status` pick
   `transaction` up from `SIGNING_SERVICES` with no other change.

---

## §9 Tests

### 9.1 Parity harness changes (`crates/roym_web/tests/dual_build_parity.rs`)

1. **Bind `transaction`.** Delete the
   `.filter(|s| s.name != "transaction")` from both topology-registration
   loops (`:1050` and its native twin) and the comment above them.
2. **Parameterise the harness for scenario 5** (`F8`):
   ```rust
   async fn harness() -> Harness { harness_with_unbound(None).await }
   /// `skip` names a sibling left out of both topologies, so a scenario can
   /// drive `web` at a dependency that is declared and not resolvable.
   async fn harness_with_unbound(skip: Option<&'static str>) -> Harness { … }
   ```
   `scenario_5` calls `harness_with_unbound(Some("transaction"))` and keeps
   driving `receipt.ping`; its comment is rewritten to say the harness omits
   it rather than that the loop filters it.
3. **`enrol_signing(&h, "transaction")`** works unchanged.
4. **A foreign party.** Add:
   ```rust
   /// A second person, whose records this installation receives but never
   /// signs. Fixed bytes so both builds mint the same DID.
   fn peer_identity() -> Identity { Identity::from_bytes(&[7; 32]) }
   fn peer_did() -> String { derive_did_key(&peer_identity().public_key()) }
   /// Signs one envelope directly with `peer_identity`, at the same pinned
   /// clock the two stacks use, so a scenario can deliver a card the local
   /// node did not produce.
   fn sign_as_peer(record_type: &str, version: u32, subject: &str,
                   payload: Value, expires_at_secs: Option<u64>,
                   issued_at_secs: u64) -> String;
   ```
   `sign_as_peer` builds `syneroym_signed_record::Envelope::unsigned`,
   signs the bytes with `peer_identity()`, attaches the z-base-32 signature
   and returns the JSON — the same shape `crates/roym_core/src/listing.rs`'s
   own test helper `sign_listing_with_own_issuer` already uses.
5. **`inbound_card`**: `inbound()` with `content_type` set to
   `CARD_CONTENT_TYPE` and the body from `card::card_body`.
6. **`strip_volatile`**: no new names needed if every transaction row uses
   `updated_at_secs` / `stored_at_secs` / `sender_timestamp_ms`, which are
   already stripped. Keep it that way.
7. **`WIRE_REFUSED_VERBS`** unchanged.

### 9.2 Parity scenarios, 122 onward

The file's last scenario today is 121. Use 122+.

| # | Scenario |
|---|---|
| 122 | `transaction` certificate verbs: `signing-status` unenrolled, install, then `Installed` — parity, mirroring 66 |
| 123 | `request.set` signs, stores and sends: the envelope is byte-identical on both builds, `request_id` re-derives from the issuer, and a card message of the reserved content type appears in `conversation.history` |
| 124 | `request.set` without enrolment answers `signing-not-enrolled`, both builds |
| 125 | `request.set` with a `request_id` produces a second version carrying `supersedes`, keeps the `request_id`, and `request.history` returns both oldest-first |
| 126 | `quote.set` refuses a request this installation issued itself (`a request cannot be quoted by the person who made it`) |
| 127 | A request signed by the peer, delivered as a card, is filed by `transaction.sync`: `filed: 1`, `transaction.thread` shows one verified `request` card whose `issuer` is the peer DID |
| 128 | `quote.set` against that filed request signs a quote whose `consumer_did` is the peer, whose envelope carries `expires_at_secs`, and whose card lands in the conversation |
| 129 | `agreement.accept` on a quote this installation issued makes the **provider** half; on a peer-issued quote naming this owner as consumer it makes the **consumer** half. Both parity |
| 130 | The full pair, one stack at a time: the peer's consumer half is delivered as a card, `sync` files it **and countersigns**, `agreement.get` reports `pair: complete`, and the two halves' payloads differ only in `role` |
| 131 | A consumer half whose `terms` differ from the quote by one byte is filed refused, no half is stored, and nothing is countersigned |
| 132 | A consumer half issued by a DID the quote does not name is filed refused |
| 133 | `agreement.accept` on an expired quote answers `quote-expired`; the pair stays incomplete |
| 134 | `sync` is idempotent: running it three times leaves one card row, one request, one agreement half, and `filed: 0` on the second and third runs |
| 135 | `sync` with `full: true` after the watermark has moved re-scans from 0 and files nothing new |
| 136 | A card whose declared type is `quote` and whose envelope is a signed `request` is filed refused (`D-C7-5`) |
| 137 | A card whose `(type, version)` is not in `CARD_TYPES` is filed with `known: false`, `verified: false`, no `data`, and no parse attempt |
| 138 | A card of a known type with no producer (`payment-request`) is filed `known: true`, `verified: false`, with the "no producer in this build" reason |
| 139 | A card whose payload names another conversation is filed refused |
| 140 | A card body that is not JSON, and one over `MAX_CARD_BODY_BYTES`, are each filed refused without a panic |
| 141 | `transaction.thread` orders cards by sender timestamp then message id, identically on both builds |
| 142 | `transaction.export` / `.import` round-trip: integrity passes, a tampered `quotes` envelope refuses the whole import naming the id, and a card row's `verified` is recomputed rather than trusted |
| 143 | Guard, in the shape of scenario 73: every C7 verb driven once locally with params that reach the handler answers neither `-32601` nor `-32013` |
| 144 | Every C7 verb over the wire answers `-32013`, both builds (shape of 106b) |
| 145 | `listing.set` with a currency this build does not know is refused, and one with `JPY` signs an `amount_minor` the Hub's exponent table agrees with |

Scenarios 127–140 all run on one stack with the peer identity of §9.1 item
4 — the parity suite's job is that the two builds agree, and a second
installation is the e2e's job.

### 9.3 `crates/substrate/tests/roym_transaction_e2e.rs` — new

Two substrates, modelled directly on `roym_conversation_e2e.rs`. Ports
`PORTS_A = (14_800, 14_801, 14_802)` and `PORTS_B = (14_900, 14_901,
14_902)`, with the comment naming the blocks already claimed
(`conversation_e2e.rs` 14_000–14_102, `roym_conversation_e2e.rs`
14_200–14_302, `roym_directory_e2e.rs` 14_400–14_702). Reuse
`SUBSTRATE_TEST_LOCK`, `fast_conversation_role`, `mint_masters`,
`substitute_plan`, `certify_and_publish` and the `Node` struct verbatim
from `roym_conversation_e2e.rs`; `SIGNING_SERVICES` becomes the four-name
list.

One test, `an_offer_is_agreed_across_two_installations`, steps:

1. Boot X (consumer) and Y (provider); deploy Roym on both; enrol signing
   on all four services on each.
2. Y sets a profile and an active listing; X sets a profile.
3. X `conversation.open` to Y's conversation address, taken from Y's signed
   listing's `conversation_address` — the no-directory engage path.
4. X `request.set` → a card is sent. Wait for `delivered`.
5. Y `transaction.sync { conversation }` → `filed: 1`; `transaction.thread`
   shows one verified `request` whose `issuer` is X's owner DID.
6. Y `quote.set` with every `AgreedTerms` field filled and
   `expires_in_secs = 3600` → a card is sent.
7. X `transaction.sync` → files the quote; `transaction.thread` shows it
   verified with X's own DID as `consumer_did`.
8. X `agreement.accept { quote_record_id }` → `role: consumer`,
   `pair: half`.
9. Y `transaction.sync` → `countersigned: 1`; Y's `agreement.get` reports
   `pair: complete`.
10. X `transaction.sync` → X's `agreement.get` also reports
    `pair: complete`, and the two halves' payloads differ only in `role`.
11. **Every field the Records table names is present** on both halves:
    `payee`, `quote_expires_at_secs`, `cancellation_terms`,
    `refund_terms`, `dispute_path`, plus `consumer_did`, `provider_did` and
    `quote_record_id`. Assert each by name, not by a blanket
    `is_object()` — R1 row 4's acceptance test is this assertion.
12. Restart X, redeploy on `resume`, and re-read `agreement.get`: still
    complete, both envelopes byte-identical to what step 10 saw.
13. X sends a plain chat message claiming a different payee; the
    agreement's `terms.payee` is unchanged (failure-matrix row 5).
14. Y `quote.set` with `expires_in_secs = MIN_QUOTE_LIFETIME_SECS`; after
    that window, X `agreement.accept` answers `quote-expired` (row 9). Use
    a second quote and a short sleep rather than a real hour.

A second test, `a_tampered_card_is_filed_refused_and_never_verified`:
deliver a card whose envelope has one byte changed in its payload, and
assert `transaction.thread`'s row is `verified: false` with a reason, no
`data`, and no `requests` row created.

### 9.4 Browser cases in `roym-hub.spec.ts`

| # | Case |
|---|---|
| 24 | Components tab: the request, quote and agreement-receipt samples render their real fields — scope, total, payee, expiry, cancellation, dispute — as text |
| 25 | Card safety on the real templates: `window.RoymRegistry.renderCard` with every string field set to markup and a `javascript:` payee yields no element from the data, no network request, and literal text (extends case 4) |
| 26 | Messages tab: opening a thread and sending a request posts a card, and the thread renders it as `.card-request` with its description, never as raw JSON |
| 27 | Messages tab: a refused card renders as `.card-refused` with `data-verified="false"` and no engage affordance (driven through `RoymRegistry` plus the caller's own branch, since a single node cannot serve itself a forgery — the same limit the Directory tab's case 15 records) |
| 28 | Backup tab: four bundles, and the note says "four" — case 8's `toHaveCount(3)` becomes 4 and its title changes |

Case 27's limitation is the same one already carried as a backlog row for
the Directory tab; extend that row rather than adding a second.

### 9.5 Failure-and-security-matrix rows C7 closes

| Row | Closed by |
|---|---|
| 4 — an unknown card type, or a known type at an unknown version | Parity 137; browser case 3 (already) plus 25 |
| 5 — a quote's payee contradicted by a later chat message | e2e step 13 |
| 9 — an unaccepted quote expires rather than staying live | Parity 133; e2e step 14 |
| 10 — either party tries to alter a signed receipt | Parity 130/131 (a half that does not match the quote is refused) and the `supersedes: None` rule; a correction is a separate record referencing the same subject |
| 13 — an import reproduces verification status | Parity 142 |
| 14 — a record with no version field | Already structural; parity 143's guard keeps it so |
| 19 — any interface behaving differently on the two builds | Every parity scenario |

---

## §10 Order of work

Five work orders. Each compiles and its own tests pass before the next.

**WO1 — the vocabulary.**
1. `roym_core::money` and its two tests, then the `editor.ts` move and the
   pinning test (§3.1, §7.1's money half).
2. `listing.rs`'s currency check (§3.2) and the fixture sweep it forces.
3. `roym_core::card`'s `Card` wrapper, `CARD_CONTENT_TYPE`, `parse_card`,
   `card_body` (§3.3).
4. `roym_core::transaction` — payloads, validation, id derivation, the
   three verification bodies, `halves_agree`, `PairState` — with its unit
   tests (§3.4, §3.5). **This is the choke point**: every later step reads
   these shapes, and a payload field added after WO2 means re-signing every
   fixture.
5. `router.rs`'s `transaction.` prefix and the two test updates (§3.6);
   `backup.rs`'s four section names (§3.7); `lib.rs` (§3.8).

**WO2 — the service.**
6. Collections, row types, `SCHEMA_VERSION` 2, the verb table (§4.1, §4.2).
7. `request.set` / `quote.set` and the `write_version`-shaped body they
   share (§4.3, §4.4), with `count_mine` (§4.10) and `file_own_card`
   (§4.11).
8. `agreement.accept` (§4.5).
9. `transaction.sync`, `file_incoming_card`, `maybe_countersign` (§4.6–4.8).
10. `transaction.thread`, the get/list/history trio for each record, the
    three `*.verify` verbs (§4.12–4.15).
11. `transaction.export` / `.import` (§4.16).
12. The manifest edge (§5).

**WO3 — the parity suite.**
13. The four harness changes (§9.1), then scenarios 122–145 (§9.2). Land
    143 and 144 — the two guards — **with** the verb table, not after it,
    so the slice's first proof is that nothing new became wire-reachable.

**WO4 — the e2e and the CLI.**
14. `roymctl roym transaction` and the `SIGNING_SERVICES` change (§8).
15. `crates/substrate/tests/roym_transaction_e2e.rs` (§9.3), plus
    `transaction` added to the two existing e2e files' `SIGNING_SERVICES`.

**WO5 — the Hub, the gate, the documents.**
16. The three real templates, `refused.ts`, the Messages-tab flow, the
    Backup tab's fourth bundle, `main.ts`'s samples, and the vitest
    additions (§7). Rebuild with `mise run build:roym-ui` and
    `mise run build:roym`; run `mise run test:roym-ui`.
17. `roym-hub.spec.ts` cases 24–28 and case 8's count (§9.4).
18. `cargo xtask check-roym-deps`, then the full gate:
    `cargo +nightly fmt --all`,
    `cargo clippy --workspace --all-targets --all-features`,
    `cargo test --workspace`, `cargo audit`,
    `cargo deny check licenses`, `mise run test:e2e`.
19. Documents and backlog (§15).

WO2 and WO3 both touch behaviour the other asserts; they are ordered, not
parallel. WO4's two halves are independent of each other.

**Rebuild rule.** `dual_build_parity` loads pre-built `wasm32-wasip2`
artifacts. After **any** change under `crates/roym_*`, run
`mise run build:roym` before `cargo test -p syneroym-roym-web`, or the WASM
side of every scenario runs stale code and the failure looks like a shim
bug.

---

## §11 What is compared across builds, and what is not

- **Compared byte for byte:** every signed envelope — the request, the
  quote, both halves of the agreement receipt — at every hop, as signed, as
  stored, and as returned by `get` / `history` / `thread`. The card body
  (`card_body`'s output) as it appears in `conversation.history`. The two
  halves' payloads against each other.
- **Compared after `strip_volatile`:** every row shape — `CardRow`,
  `RecordPointerRow`, `AgreementRow`, `sync` counts — because each carries
  the host's own wall clock at write time, which is not the pinned signing
  clock.
- **Not compared:** conversation message ids (already normalised by
  `normalize_message_ids`), and the absolute `expires_at_secs` of a quote
  signed in two separate calls — assert the *relationship*
  (`expires_at_secs - issued_at_secs == expires_in_secs`) instead.

---

## §12 Permitted differences (WASM vs native)

C7 adds **none**. Every new surface is a local dispatch on both builds,
signs through the same host interface, and reaches `conversation` through
the same `CallTarget::Dependency`. If a scenario needs a permitted
difference to pass, that is a shim bug (failure-matrix row 19), not a new
row for `status.md`'s §14 list.

---

## §13 What C7 deliberately does not build

- **Any transaction state machine.** No `requested`/`quoted`/`agreed`/
  `scheduled`, no single named writer, no idempotency key, no booking, no
  conflict. All of that is C8 (`D-06C-13`), and C7 must not anticipate it:
  the two nodes are symmetric here and each holds what it signed and what
  it filed.
- **`booking-progress`, `payment-request`, `payment-acknowledgement`,
  `fulfilment-receipt` producers.** Their templates stay as C2 left them;
  their card types stay in `CARD_TYPES` so the registry is complete; a card
  of one of those types is filed `known: true, verified: false` with a
  stated reason (§4.7's last arm).
- **`payment-request` in `RECORD_TYPES`** — `D-C7-16`.
- **A node-side trigger for `sync`.** `D-C7-3`'s cost.
- **Binding a card's issuer to the conversation peer** — `D-C7-11`.
- **Negotiation history rendering, or quote templates** — excluded by R1
  row 4's own "Excluded" column.
- **Attachments on a request** — out of the first release.
- **Any wire-reachable `transaction` verb.**

---

## §14 What "done" means for C7

1. `transaction` signs, stores, sends and files all three record types on
   both builds, and every envelope is byte-identical across the two.
2. A `request` and a `quote` each re-derive their id from the signature's
   own issuer, and a mismatch is refused.
3. A quote's expiry rides its envelope; an accept after it answers
   `quote-expired`; an unaccepted quote stops verifying at its own expiry.
4. An `agreement-receipt` pair is complete on **both** installations after
   one consumer accept and one provider `sync`, and the two halves' payloads
   differ only in `role`.
5. The agreement receipt carries payee, quote expiry, cancellation terms,
   refund terms, dispute path, both party DIDs and the quote's `record_id`,
   asserted field by field in the e2e — **R1 row 4's acceptance test**.
6. A card that does not verify, or whose type is unknown, or whose declared
   type does not match its envelope, is stored refused and never rendered
   by a real template.
7. `transaction.sync` is idempotent, bounded, and safe to call repeatedly.
8. No `transaction` verb is reachable over the wire, proven on both builds.
9. `transaction.export` / `.import` round-trips and reproduces verification
   status.
10. The Hub renders the three real card templates safely — no markup
    inserted, no URL fetched — and shows a refused card distinctly from a
    verified one.
11. The currency exponent table lives in `roym_core::money`, `editor.ts`
    is pinned to it, and an unknown code is refused rather than assumed.
12. No planning identifier in any name or comment, checked by grep.
13. All six gates clean.
14. `status.md` gains a C7 section; `task.md`'s C7 row is marked complete;
    the spec's scope table marks **R1 passed**.

---

## §15 Documents and backlog owed

**Documents**

| Document | Edit |
|---|---|
| [status.md](status.md) | A C7 section: what shipped, the decisions `D-C7-1`…`D-C7-16` as built, verification evidence, and anything that shipped differently from this plan |
| [task.md](task.md) | C7's row marked Complete; the "Owed as slices land" table's C7 row discharged (*"R1 marked passed in the spec's scope table"*); the C7 scope row's stale *"C1's renderer"* corrected to C2's |
| [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md) | **R1 marked passed** in the First release scope table — all six rows, the gate this slice closes |
| [deferred-backlog.md](../../deferred-backlog.md) | The rows below |

**Backlog rows closed**

- §2's currency-exponent row (`M06C C7`) moves to "Recently resolved":
  `roym_core::money` is the source of truth, `PaymentTerms::validate`
  refuses an unknown code, and a Rust test pins the TypeScript copy.

**Backlog rows opened**

| Item | Trigger |
|---|---|
| **A card is filed only when a client calls `transaction.sync`** — the manifest's cycle check forbids the `conversation → transaction` edge that a push would need, so a person who never opens a thread never files its cards. Fixes: a host-level inbox hook a second service may subscribe to, or a node-side scheduler. | A person misses a quote because they did not open the thread |
| **`sync`'s overlap window can miss a badly reordered message** — `history` orders by sender timestamp, so a message landing more than `SYNC_OVERLAP` positions behind the watermark waits for `sync { full: true }`. | A conversation with real clock skew loses a card |
| **Two concurrent `request.set` / `quote.set` calls in one conversation derive the same id** — both read the same sequence and the second overwrites the first's pointer row. Same shape as `catalog`'s unfenced `version_count`; the fix is the same compare-and-set the data layer does not offer. | A client sends two records in one conversation concurrently |
| **A card's issuer is not bound to the conversation peer** — `D-C7-11`. What binds the parties is the quote's `consumer_did`, so a *request* card from an unexpected issuer is filed and shown with that issuer, not refused. Fixing it needs the peer's `profile` record and Gap 5's person↔address mapping applied at the inbox. | A person is confused by a card from a DID they do not recognise |
| **`resolve_principal_and_owner`, `ensure_coll`, `idx`, `collect` and the export/import bodies now exist in five services** — consolidating them into `roym_core` touches every service and was not folded into this slice. | A sixth copy appears, or one copy drifts |
| **`payment-request` is a signed record with no `RECORD_TYPES` row** — `D-06C-12` settles that it is one; C7 builds no producer, so the entry lands with C8's verb. | C8 |
| **A card of a known type with no producer files `known: true, verified: false`** — correct today and wrong the moment C8 ships the four producers. C8 must revisit §4.7's last arm. | C8 |
| Extend the existing "no single-node browser fixture serves a forged listing" row (§11) to cover a forged **card** — browser case 27 has the same limit for the same reason. | Same trigger as that row |

---

## §16 Ambiguities and staleness in the input documents

**A. `task.md`'s C7 row says the renderer is C1's.** It reads *"and in C1's
renderer on the consuming side (D-06C-3)"*. The renderer is C2's
(`crates/roym_web/ui/src/cards/`, shipped 2026-08-29); C1 built the
dual-build shim and no UI at all. **Stale reference, not a design
question** — corrected in `task.md` as part of §15.

**B. "The seven card types … land here on the producing side."** Four of
the seven — `booking-progress`, `payment-request`,
`payment-acknowledgement`, `fulfilment-receipt` — appear at journey steps
C16–C20, which are C8's. **Reading adopted:** the card *contract* (the
wrapper, the content type, the `(type, version)` rule, the unknown-type
rule, the refused-card rule) lands in C7, and three of the seven get real
producers and real templates. Building four producers with no state machine
underneath would mean writing them twice.

**C. `D-06C-12` puts `payment-request` in the record list; `RECORD_TYPES`
does not carry it.** Adding a row with no producer would put a claim in the
tree that nothing backs. `D-C7-16` defers it to C8 with a backlog row.
Flagged rather than decided silently, because the decision row does read as
"add it now".

**D. "Containing every field listed in [Records]".** The Records table
lists no field schema — only the prose *"Both parties accepted these exact
terms, including payee, expiry, cancellation and refund terms, and dispute
path"*. **Reading adopted:** those five, plus the two party DIDs and the
referenced quote's `record_id`, are the fields the acceptance test asserts
by name (§9.3 step 11). If a reviewer wants more, the place to say so is
before WO1 finishes, because adding a field afterwards re-signs every
fixture.

**E. Nothing in the input documents says how a record reaches the other
party.** The spec puts cards in the conversation; `task.md` does not say
whether the record travels inside the card or by a separate call. `D-C7-1`
decides it: inside the card, over the conversation, and by no other path.
This is the biggest open decision in the slice and is stated as a decision
rather than assumed.

**F. `depends_on` cycle detection makes the obvious design impossible.**
Both "the inbox pushes a card at `transaction`" and "`transaction` puts a
card into a conversation" cannot hold at once (`F1`, verified in
`crates/app_orchestration/src/models.rs:722`). `D-C7-2` and `D-C7-3` are
the consequence. Any executor who reaches for the push will hit a deploy
failure, not a compile error, so it is worth knowing first.

**G. `D-06C-7` says "C7/C9 must pass canonical DIDs, never registry
aliases."** C7 passes no DID over the wire at all — every record travels
inside a conversation message, and every DID in a payload is a `did:key`
from a signature or from a signed profile. The constraint is satisfied
vacuously, and §14 records that rather than leaving the row looking
untested.

**H. The "dynamic record signer key re-derivation" backlog row names
C7/C8.** `NodeRecordSigner::identity` re-derives a service's signing key
from its recorded owner on every call, so changing a service's owner
re-keys it and every stored certificate goes `Stale`. C7's storage is keyed
by `record_id` and `request_id`/`quote_id`, both content-derived from the
**issuer** (the person's master DID, not the service key), so an owner
change does not orphan a stored record — only the certificate. Nothing to
do beyond stating the invariant; §4's row types inherit it.

**I. `transaction` is declared `visibility = "public"` while every verb is
wire-refused.** Consistent with `catalog` and `conversation`, which are the
same. `D-C5-4` says no `visibility` value changed in C5, and C7 keeps that.
**Open for the executor:** if a reviewer prefers `private` here, it is a
one-line manifest change and a re-check that the deploy path still
registers the service; this plan does not make it because the value is
about the registry record, not about admission (`F9`).

**J. The spec's journey step C12 lists attachments in the request.**
Attachments are excluded from the first release by the spec's own scope
table. `RequestPayload` carries none, and the Hub's form offers none.

**K. R1's gate is six rows, not one.** Rows 1, 2, 3, 5 and 6 are already
marked passed by C4/C5/C6's evidence in `status.md`. C7 closes row 4 and
then marks R1 as a whole. Before writing that into the spec, re-read
`status.md`'s C4, C5 and C6 evidence sections and confirm each row's test
still passes on the merged branch — marking R1 passed on the strength of
five older claims and one new one is exactly the thing that rots.

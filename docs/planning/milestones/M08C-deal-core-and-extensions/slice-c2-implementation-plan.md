# M08C slice C2 — Selective disclosure: implementation plan

*Reader: the developer (or AI) who implements slice C2, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-18, by Claude Opus 5.5. Not reviewed yet.
The owner has not answered Q1 to Q7 yet. The plan is written for the
recommended answer of each question.**

**Size and PRs.** C2 is an L slice. This plan splits it in two parts, and
each part is one PR that passes `mise run verify` (Q1):

- **C2a — Hide (L).** Every signed deal record holds digests in place of
  its hidden fields. Disclosures travel in the card, are checked, stored,
  backed up and used for every view. The Hub's quote form shows the size
  line. Branch: `feat/m08c-c2-disclosure` (this branch; it holds the plan
  commit).
- **C2b — Show (M).** `deal.present`, `deal.verify-presentation`, and the
  four `*.verify` methods that take disclosures. A third node verifies a
  deal with the price hidden. Branch: `feat/m08c-c2b-presentation`, cut
  from `main` after C2a merges.

**Base.** Every file path and line number below was checked on `main` at
`ba5a5597` (2026-11-18). Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: C2 depends on C1 (C1a and
C1b). Both are on `main`: C1a as `3dfada58` (PR #367), C1b as `ba5a5597`
(PR #368). [status.md](status.md) said "Verified" for C1b although the
squash commit is on `main`; this plan's commit sets that row to "Merged".
C7 needs C2 (a forwarded quote carries no disclosures). C2 does not touch
`conversation`, the forwarding change doc or `docs/decisions/`.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments or test names: no "M08C", "C2a",
    "AG-17", "D-C2-4", "Q3" or review ids in `.rs`, `.ts`, `.wit`, `.ftl`,
    `.json` files. Note: `crates/roym_core/src/deal.rs:144` already has one
    ("C2 changes where these fields sit"). C2a deletes it (section 2.2,
    point 7).
  - Comments never cite size rules.
  - **File size.** Production `.rs` files at most 800 lines; new test `.rs`
    files at most 800 lines; a file in `xtask/oversized-test-files.txt` is
    capped at its recorded count. Files this slice touches that are near
    the cap are listed in section 7.8. Put new code in new files.
  - **Function size.** `clippy::too_many_lines` at 100. The suppression
    ratchet is at 38 (`xtask/src/lint_suppressions.rs:9`); add none.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1).
  - Do not commit on `main`.
- [task.md](task.md): section 1 (Z2 and the component size limit), "C2",
  section 4, section 5.
- [roym-agreements change doc](../../changes/roym-agreements/change.md):
  "Terms", the field tables of "Quote", "Commitment" and "Settlement
  records", the whole "Selective disclosure" section (`change.md:642` to
  `:715`), "Methods" (`:840`), and `## Deviations` (`:1055`).
- [roym-value-exchange change doc](../../changes/roym-value-exchange/change.md):
  VX-11 (`:146`) and "Disclosure of deal evidence" (`:316`).
- RFC 9901, "Selective Disclosure for JSON Web Tokens", sections 4.1.1,
  4.2.1 to 4.2.4 and 7.1 (verification). Only the disclosure, digest and
  `_sd` rules are used. No JWT, no key binding, no decoys.
- The previous slice plan: [slice-c1-implementation-plan.md](slice-c1-implementation-plan.md).
  This plan uses its section layout. Section 7.11 lists how C2 avoids the
  kinds of findings in [slice-c1-review.md](slice-c1-review.md).

**Change docs.** The prompt lists two: roym-agreements and
roym-value-exchange. task.md's C2 section names the same two. Each
criterion, decision and deviation is written in the doc that defines it:

- `AG-*` rows, the encoding and the record shapes: roym-agreements.
- VX-11: roym-value-exchange. C2 meets VX-11 fully through AG-17 to AG-21
  and AG-34, so that doc gets no deviation row.

**Do not copy ids into code.** The ids `D-C2-n`, `Qn` and `AG-n` in this
plan are for the plan and the change docs only.

### Terms used in this plan

- **Hidden field:** a field of a signed record whose value is not in the
  record. The record holds only its digest. The set of hidden fields is
  fixed in code per object (agreements doc, "What is hidden").
- **Salt:** 16 random bytes, encoded base64url with no padding (22
  characters). One fresh salt per hidden field per signed record.
- **Disclosure:** the string `base64url(JSON [salt, name, value])`. Whoever
  holds it can show that one field.
- **Digest:** `base64url(SHA-256(ASCII bytes of the disclosure string))`,
  43 characters.
- **`_sd`:** the sorted list of digests that replaces the hidden fields of
  one object.
- **Wire form, plain form.** The *wire form* of a record is what is signed:
  visible fields plus `_sd`. The *plain form* is the same record with every
  hidden field put back. Nodes keep the plain form in their own local rows;
  only the wire form is signed.
- **Strict reveal:** the check a party's own node runs before it files a
  record: every digest has a disclosure, every salt is 16 bytes, no salt
  repeats, sizes are within limits, and the full plain record validates.
- **Partial reveal:** what a third party gets: some fields shown, the
  others marked hidden. Each shown field is checked on its own.
- **Presentation:** signed records of one deal plus the disclosures a party
  chooses to give, for a third party (agreements doc, "Showing a deal to a
  third party").
- **Parity harness:** `crates/roym_web/tests/dual_build_parity`. It runs
  each scenario against the native build and the `wasm32-wasip2` build of
  every Roym service and compares the answers.
- **Test host:** `roym_core::testing::TestHost`, the in-memory `AppHost`
  for in-crate service tests (C1's D-C1-3).

## 1. What C2 builds, in one paragraph

After C1, every deal record carries its terms in plain form: the quote's
`terms: DealTerms` (`crates/roym_core/src/transaction.rs:163`), both
receipt halves (`:175`), and the settlement records' `note`, `method`,
`reference` and `settlement_note` (`crates/roym_core/src/payment.rs:55`,
`:96` to `:101`; `crates/roym_core/src/fulfilment.rs:45`). Anyone who sees a
receipt sees the price. C2 replaces each hidden field with a digest in the
object's `_sd` list, following RFC 9901. The signer's node picks a fresh
salt for every hidden field, signs only the digests, and sends the
disclosures beside the envelope in the card. The receiving node checks
every disclosure against the digests and stores them. Both nodes keep the
plain values in their own rows, so every screen, `agreement.get` and the
Hub keep working as today. The backup bundle carries the disclosures. The
Hub's quote form shows how much of the 96 KiB disclosure limit the quote
uses. Then C2b lets a party build a presentation that shows chosen fields,
for example the scope with the price hidden, and lets a third node verify
it.

### Why C2 is split (Q1)

The work has two kinds of risk, as C1 had:

- **C2a is wide.** It changes every signed deal record, every place that
  signs one (6 builders, section 2.2 point 3), every place that files one
  (7 filers), every place that reads a stored quote (5 copies of the same
  load, D-C2-6), the backup, the parity fixtures that hand-sign peer
  records, and the host signing interface (Q2). Each change is simple. A
  missed one shows a wrong value or refuses a good card.
- **C2b is narrow and is security code.** It verifies data from a stranger:
  records of possibly different deals, disclosures that may be crafted, a
  snapshot from an unknown key. It adds methods but changes no existing
  path.

Each part leaves a working system. After C2a every value is hidden and
every flow works; nobody can yet present a deal to a third party, and the
`*.verify` methods show only visible fields. C2b adds the showing.

Rejected cuts:

- **One C2.** One review would have to check a byte-level encoding, every
  record path and a stranger-facing verifier at once. The C1 review took 9
  rounds; narrow reviews found the deep issues.
- **Quote first, settlement records later.** A shown receipt would hide the
  price while a shown payment acknowledgement still showed the method and
  reference. Both are small; splitting them saves little.

## How this is shown to an end user

C2a:

- Bob writes a quote in the Hub, as today. Under the form he now sees a
  line like "Size: 2.1 KiB of 96 KiB". It changes as he types.
- If Bob writes far too much text, the line turns red and says the quote
  is too large to send.
- Alice receives the quote and sees it exactly as before: the same lines,
  the same prices. Accepting, paying and confirming look the same.
- What changed is not visible on screen: the signed records no longer
  contain the price, the texts or the payment details. Only the two
  parties' own devices hold them.

C2b:

- An end user sees nothing new. No Hub screen offers "show this deal to
  someone" yet (Q6). A later slice or milestone (M08G, evidence) adds the
  screen that uses it.

How a developer can see it work:

- `mise run test:e2e`: story 50 shows the size line on Bob's quote form;
  stories 40 and 50 and the Hub cases pass unchanged.
- `cargo nextest run -p syneroym-signed-record disclosure` and
  `cargo nextest run -p syneroym-roym-core disclosure`: the RFC 9901 test
  vectors and the refusal tables.
- C2b: `cargo nextest run -p syneroym-substrate --test roym_disclosure_e2e`:
  Bob shows his agreement with Alice to Carol's node, with the price
  hidden and the scope shown; Carol's node verifies both halves and the
  shown field, and reports the price as hidden.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for the
recommended answer. Answer-dependent rows go into the change docs'
`## Deviations` when the owner answers.

#### Q1. Split C2 into C2a (hide) and C2b (show)?

- **(a) Split as section 1 describes.** Recommended. Each part works on its
  own and has one kind of risk.
- (b) Keep one C2: one PR of roughly 5,000 changed lines (estimate:
  `roym_core` and `signed_record` +1,400 with tests, `transaction` ±1,200,
  parity and shape files ±900, two-node ±500, host and WIT ±300 plus
  regenerated bindings, Hub ±250).

#### Q2. Where do the salts come from?

The agreements doc says "16 bytes from the host's secure random source"
(`change.md:658`). Today no Roym code makes random bytes (a search for
`random`, `nonce`, `salt` in `crates/roym_*/src` and `crates/app_host*/src`
finds nothing). The parity harness compares the native and the WASM answer
byte for byte; record ids are equal today because signatures are
deterministic and the signing clock is pinned (`RecordClock::Fixed`,
`crates/roym_web/tests/dual_build_parity/helpers.rs:1042`). Random salts
make every quote record id, receipt, digest and envelope differ between
the two builds.

- **(a) A new host function `fresh-salts` in `syneroym:signing`, served by
  `NodeRecordSigner`, with a seedable source for tests.** Recommended. Both
  builds already reach the same host signer: WASM through
  `signing::Host for HostState`
  (`crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs:52`)
  and native through the same trait (`HostSigning::sign_record`,
  `crates/app_host_native/src/host.rs:675`). `NodeRecordSigner` already
  takes a test clock (`crates/core/src/record_signer.rs:56`, `:73`); a
  `RecordSalts::Seeded(u64)` beside it keeps parity exact. The signing WIT
  is one file; every `wit/deps/signing/signing.wit` is a symlink to it.
  Cost: one WIT function, four implementers (`GuestHost`,
  `NativeAppHost`, `HostState`, `TestHost`), and regenerated
  `bindings.rs` in the six Roym crates (a few hundred bytes per
  component).
- (b) `getrandom` in the guest. It is already in the WASM build graph
  (through `syneroym-identity`, `crates/identity/Cargo.toml`, used at
  `crates/identity/src/backup.rs:72`) and works on `wasm32-wasip2`. The
  parity harness must then rewrite every salted value by position, as it
  does for message ids (`normalize.rs:11`). Parity of deal records then
  proves much less, and the rewrite is large.
- (c) Salts derived from a vault secret and the record's place (an HMAC).
  Deterministic and testable, but the doc rules out salts "derived from
  data", and a leaked secret would reveal every hidden value by guessing.
  Not recommended.

#### Q3. Where does the encoding code live?

`check-roym-deps` lets a `roym_*` crate depend only on a fixed list
(`xtask/src/main.rs:181` to `:196`): no `sha2`, no `base64`.
`syneroym-signed-record` is on the list and already depends on `sha2`
(`crates/signed_record/Cargo.toml`).

- **(a) The RFC 9901 primitives (salt type, disclosure encode and decode,
  digest, base64url) go into a new `syneroym_signed_record::disclosure`
  module, with `base64` added to that crate. Roym's rules (which fields are
  hidden, limits, strict and partial reveal, presentations) go into
  `roym_core`.** Recommended. No change to the allow list; the primitives
  sit next to the envelope they serve.
- (b) Everything in `roym_core`, and add `sha2` and `base64` to the allow
  list.
- (c) Hand-written base64url in `roym_core` (about 40 lines), SHA-256 from
  `signed_record`. Less reuse of a standard crate for no gain.

#### Q4. How does the Hub's quote form get the size?

AG-40 asks the Hub to show the size against the limit before the person
signs. The exact size depends on how `serde_json` writes each field (field
order does not matter for length, but skipped empty fields and escapes do).

- **(a) A new local-only method `quote.measure { terms }` that runs the
  same Rust code `quote.set` runs, with zero salts (a salt is always 22
  characters, so the size does not depend on its value).** Recommended.
  One source of truth. The Hub calls it 300 ms after the person stops
  typing. It is a deviation: the doc's method table has no such method.
- (b) Compute the size in TypeScript. A second copy of the encoding that
  can drift.
- (c) Show the size only after `quote.set` refuses with `quote-too-large`.
  Does not meet "before the person signs".

#### Q5. Rules that read a hidden field

Two checks today read text that C2 hides:

- ML-18's rule "`viewed_via_mt` is false when `viewed_lang` is the
  original's language" is checked inside `AgreementReceiptPayload::validate`
  against `terms.scope.lang` (`transaction.rs:332`).
- AG-39 (one language and one translation set per record) is checked by
  `DealTerms::validate` over every `LocalText` (`deal.rs:568` to `:580`).

After C2 a verifier without disclosures cannot see `scope`. Options:

- **(a) Both checks run wherever the plain terms are held: `agreement.accept`
  (today's `viewed`, `crates/roym_transaction/src/app/agreement_ops.rs:296`),
  the filing of each half, the backup import, and `quote.set`. A third
  party's verifier runs ML-18's check when `scope` is shown and AG-39's
  check over the texts that are shown, and reports the rest as "not
  checked".** Recommended. The parties' nodes, the only ones that act on
  the rule, still enforce it fully.
- (b) Add a visible `terms.lang`. It shows the language of every text to
  anyone. Small, but the doc's hidden set is fixed by owner decision
  (open question 6).
- (c) Require `terms.scope` to be shown in every presentation of a receipt.
  A presenter could not hide the scope.

#### Q6. Demo for C2b

The demo close-out asks for a story when a viewer can see something new.
C2b adds two local JSON-RPC methods and no Hub screen and no `roymctl`
verb.

- **(a) `- none: deal.present and deal.verify-presentation have no Hub
  screen or command yet; the two-node test shows them`.** Recommended.
  The first screen that needs them is M08G's evidence.
- (b) Build a Hub "show without price" screen now. New product UI that no
  change doc describes.
- (c) Add `roymctl roym transaction deal present` and `verify-presentation`
  and a developer-guide block. A command nobody asked for.

#### Q7. Is the decline note hidden?

The doc says "hidden always: the free text" (`change.md:648`), but its
field tables hide nothing in `deal-decline`, and AG-17 lists only the
quote, the receipt, the settlement records and the snapshot. The decline
note (`DealDeclinePayload.reason`, `crates/roym_core/src/transaction/decline.rs:34`)
is free text.

- **(a) Keep it visible, as the field tables say.** Recommended. A decline
  is never part of a presentation (the doc's `Presentation.records` lists
  no decline), and the only reader is the other party, who sees it anyway.
- (b) Hide it with one digest, like a payment request's `note`. One more
  record path in C2a, for a record nobody presents.

### 2.2 Points found while reading

1. **The 96 KiB limit against a real card body.** task.md asks the plan to
   check it. Worst case on today's shapes (computed with the real field
   names and lengths: 16 commitments, each with the longest visible fields
   and 10 digests; `home_exchange` set; a `supersedes`; a delegation
   certificate; disclosures totalling exactly 96 KiB): payload 12,498
   bytes, envelope JSON 13,337 bytes, envelope as an escaped JSON string
   inside the card 14,711 bytes, disclosure array 98,800 bytes, card body
   113,583 bytes. The limit is `MAX_CARD_BODY_BYTES` = 163,840
   (`crates/roym_core/src/card.rs:63`). So 96 KiB fits with about 49 KiB to
   spare. Disclosures are base64url and need no escaping. C2a pins this
   with a test that signs a real envelope (section 7.2,
   `largest_quote_card_fits_in_the_card_body`). The C2 deviation rows in
   the agreements doc record the numbers.
2. **The 96 KiB limit binds, not the 8 KiB one.** Today the 64 KiB payload
   limit (`crates/signed_record/src/envelope.rs:7`) bounds a quote. With
   the texts at their maximum (scope 4,096 bytes and 4 translations,
   16 descriptions of 2,048 bytes and 4 translations each), the disclosures
   would be about 218 KiB. So `quote-too-large` is reachable with long
   texts in many languages, not only with large `details`. The Hub size
   line matters for real quotes.
3. **Who builds and who files each signed deal record** (non-test code):
   - builders: `quote_ops.rs:141` (quote), `agreement_ops.rs:423`
     (counterparty half), `decide.rs:683` (writer's countersign),
     `payment_ops.rs:124` (payment request), `payment_ops.rs:263`
     (payment acknowledgement), `fulfilment_ops.rs:130` (fulfilment
     receipt). The snapshot (`progress.rs:20`) and the decline
     (`decline_ops.rs:292`) have no hidden field.
   - filers (`crates/roym_transaction/src/app/sync.rs:286` to `:330`):
     `file_quote_card` (`:434`), `file_agreement_receipt_card` (`:540`),
     `settle_receipts::file_payment_request_card` (`sync/settle_receipts.rs:63`),
     `file_payment_ack_card` (`:190`), `file_fulfilment_card` (`:337`),
     `receipts::file_progress_card` (`sync/receipts.rs:141`),
     `decline::file_deal_decline_card`.
   - every send goes through `send_card_and_file` (`app.rs:537`), which
     builds the body with `card::card_body` (`app.rs:546`) and files this
     node's own copy with `file_own_card` (`app.rs:472`). Its callers:
     `enquiry_ops.rs:115`, `quote_ops.rs:169`, `decline_ops.rs:194`, `:236`,
     `fulfilment_ops.rs:219`, `:523`, `progress_ops.rs:112`,
     `payment_ops.rs:177`, `:644`, `payment_ops/fence.rs:136`, `:325`,
     `agreement_ops.rs:190`, `decide.rs:211`, `:243`, `progress.rs:47`.
4. **Settlement halves are sent again from storage.** On a repeat call the
   fence row's stored half is sent again
   (`ensure_payment_request_recorded`, `payment_ops/fence.rs:101` to `:142`;
   the same at `:325` and `fulfilment_ops.rs:219`, `:523`). A stored half
   is a `ReceiptHalf` (`transaction.rs:185`) with only the envelope. So the
   disclosures of a settlement record must be stored *before* its fence
   row, or a repeat sends a card with no disclosures (D-C2-4).
5. **Five copies of "load a stored quote and verify it".** `get_bytes(QUOTE_HISTORY)`
   then `verify_quote`: `sync.rs:509` to `:525`, `agreement_ops.rs:323` to
   `:345`, `sync/receipts.rs:53`, `sync/decline.rs:31`,
   `decline_ops.rs:81`, and in the backup at `backup.rs:480`. Each of them
   then reads `payload.terms` in plain form. C2a replaces them with one
   helper that also reveals the terms (D-C2-6).
6. **The card row's `data` is what the Hub draws.** `CardRow.data`
   (`app.rs:216`) is the verified payload; every Hub template reads
   `card.data` (`crates/roym_web/ui/src/cards/render.ts:22` to `:52`), and
   the Hub never parses a deal envelope (search for `envelope` and
   `payload` in `screens`, `cards`, `records`). So if `data` holds the
   *plain* form, no template changes. The backup import keeps `data` as
   the bundle gives it (`backup.rs:680` to `:727`) and checks only the
   envelope; after C2 the import rebuilds `data` from the envelope and the
   disclosures (D-C2-12), so a changed bundle cannot show a price the
   digests do not prove.
7. **Small things to clean while there.**
   - `deal.rs:144` has a planning-id comment ("C2 changes where these
     fields sit, not whether they are present"). C2a deletes it; the
     wire types make it true by construction.
   - `card.rs:6` says "The seven card types"; the table has eight
     (`:9` to `:18`). C2a fixes the comment when it adds `disclosures`.
   - `crates/roym_web/ui/src/cards/templates/booking_progress.ts` is not
     imported anywhere, and `render.ts:36` keeps a `booking-progress` case
     that `CARD_TYPES` never lets through. C2a deletes both.
   - `normalize.rs:21` to `:27` says a snapshot embeds
     `track_window_ends_at_secs`; C1 removed that field
     (`AgreementProgressPayload`, `crates/roym_core/src/progress.rs:78`).
     C2a corrects the comment.
   - `docs/TERMINOLOGY.md:69` says a declared value is "optional" with a
     "value basis". C1 made it a required integer. C2a fixes it with the
     new terms (section 9).
   - `docs/system-architecture.md:1209` to `:1211` still name
     `terms.lang` and `translations`, which C1 removed.
8. **Callers that read plain values out of a signed payload** and must
   change:
   - `crates/substrate/tests/roym_transaction_e2e.rs:293` to `:310` reads
     `payee`, `cancellation_terms.text` and `refund_terms.text` from each
     signed half's payload. After C2a it reads them from `agreement.get`
     and checks that the half's payload has no `payee` (an AG-17 check on
     real cards).
   - Parity fixtures build peer quotes, halves and settlement records from
     plain JSON and sign them: `fixtures.rs:268` to `:455`,
     `:696`, `:727`; `booking.rs:22`, `:50`, `:92` to `:137`;
     `agreement_lang.rs:22` to `:67`, `:216`; `deal_fixtures.rs:32` to
     `:48`. They now conceal before signing and put the disclosures in the
     card (`fixtures.rs:238`, `card::card_body`).
   - `roym_restore_e2e.rs:376` to `:452` calls `quote.history`,
     `quote.verify` and `agreement.verify` after a restore; it checks only
     `verified`, so it keeps passing, and it gains one check that
     `agreement.get` still shows the plain terms (backup carries the
     disclosures).
   - `roym_trust_e2e.rs:371` calls `agreement.verify` and checks
     `verified`; unchanged.
   - `roym_deal_e2e.rs:210`, `:523` to `:535` call `quote.get` and read
     `mine` and `declined_at_secs`; unchanged.
9. **`VerifyInput` is shared and strict.** It is `deny_unknown_fields` with
   one field (`crates/roym_core/src/method.rs:47` to `:51`) and every
   service's `*.verify` uses it. C2b adds a separate input type for the
   four deal verifies (D-C2-9).
10. **Component sizes.** C1b recorded `transaction` 1,833,741, `catalog`
    929,631, `conversation` 811,953, `directory` 1,467,518, `profile`
    771,791, `web` 310,017 bytes (task.md section 1). C2a re-measures on
    `main` at `ba5a5597` before step 2 and uses those numbers as the base.
    `transaction` gets new record code (+15% limit, about 275,000 bytes).
    The other five get only the new import in their bindings and unused
    `roym_core` code, which the linker drops: +40 KB limit each.
11. **The quote views have no plain terms.** `QuoteView` and
    `QuoteListItem` (`quote_ops.rs:69` to `:94`) return only the envelope.
    The doc's method table says the views hold the plain values
    (`change.md:848`). C2a adds `terms` to both, and a `versions` list to
    `quote.history` (D-C2-15).

## 3. What C2 meets

From [roym-agreements](../../changes/roym-agreements/change.md). "Part"
says which PR proves it.

| Criterion | Part | What C2 proves | Later |
| --- | --- | --- | --- |
| AG-17 | C2a | Unit: each signed payload (quote, both halves, payment request, payment acknowledgement, fulfilment receipt, snapshot) built from terms with marker values holds none of them, and not the derived track end. Two-node: the signed halves on real cards hold no `payee`. | |
| AG-18 | C2b | Unit: a presentation of both halves with the price hidden and the scope shown verifies. Two-node: Carol's node verifies Bob's presentation. | |
| AG-19 | C2a | Unit: the RFC 9901 digest vector, the RFC's spaced disclosure decodes, our encoder's pinned vector, and the five refusals of (4). | |
| AG-20 | C2a | Unit: both halves carry the quote's wire `terms` byte for byte. Service: a half whose `terms` differ is refused when filed (kept from C1, on the wire form). | |
| AG-21 | C2a | Unit: a salt of another length and a salt used twice are refused by the strict reveal. Service: two versions of one quote use disjoint salts. | |
| AG-33 | C2a | The hidden-field part: AG-17's test covers `settlement_note` in both settlement records. | Statement row: C6 |
| AG-34 | C2b | Unit: a quote alone, price hidden, scope shown, verifies; a quote and a half with different `terms` are refused. Two-node: Carol verifies a quote before any agreement. | |
| AG-38 | C2b | Unit: a snapshot bound to the quote author's half or the quote is "the writer's"; a snapshot from any other key is "writer not checked". | |
| AG-40 | C2a | Unit: an 8 KiB-plus `details` and a 96 KiB-plus record are refused. Service: `quote.set` answers `quote-too-large`. E2E: story 50 shows the size line. Hub unit test: the over-limit line. | |
| AG-8 | C2b | The presentation part that C1 left (agreements doc `## Deviations`, AG-8): a presentation with only the snapshot and the halves words `claimed: true/false` and the reason codes. | |

From [roym-value-exchange](../../changes/roym-value-exchange/change.md):
VX-11 (through AG-17 to AG-21 and AG-34), and the "outcomes need no
consent" rule of "Disclosure of deal evidence" (a presentation with no
disclosures shows each commitment's kind, giver and outcome; AG-8's
presentation test).

Not in C2: AG-14 to AG-16 and AG-36's snapshot part (C3b), AG-23's Hub part
and AG-27 to AG-30 (C6), AG-24 to AG-26 (M08E, M08H).

## 4. Behaviour-change audit

### Wire changes

Every Roym method stays local-only (`admit::require_internal`,
`crates/roym_transaction/src/app.rs:272`). No method becomes
wire-reachable. "Wire" here means the records and cards between the two
parties, the local JSON-RPC methods, and the host interface.

**Records (C2a).** Every record keeps its type and version 1; the shape
changes in place (the product is not released).

| Record | Today | After C2a |
| --- | --- | --- |
| `quote` | `terms: DealTerms`, plain | `terms`: `currency`, `quote_expires_at_secs`, `home_exchange` (absent in M08C), `commitments`, `_sd` (5 digests). Each commitment: `commitment_id`, `giver`, `resource_kind`, `kind`, `extension`, `after`, `_sd` (10 digests). |
| `agreement-receipt` | `terms: DealTerms`, plain | the quote's wire `terms`, byte for byte |
| `payment-request` | `note` | `_sd` (1 digest: `note`) |
| `payment-acknowledgement` | `method`, `reference`, `settlement_note` | `_sd` (3 digests) |
| `fulfilment-receipt` | `settlement_note` | `_sd` (1 digest) |
| `agreement-progress`, `enquiry`, `deal-decline` | | unchanged (Q7) |

A hidden field that is not used is still disclosed, with the value `null`,
so every object always has the same number of digests.

**Card wrapper (C2a).** `Card` (`card.rs:37`) gains `disclosures: [string]`,
written only when not empty. The wrapper version stays 1: a reader ignores
unknown fields (`card.rs` test `extra_keys_parse_but_missing_envelope_fails`).
Quote, payment-request, payment-acknowledgement and fulfilment-receipt
cards carry every disclosure of their record. Every other card carries
none; a receipt, snapshot, enquiry or decline card with disclosures is
refused when filed.

**Host interface (C2a, Q2).** `syneroym:signing@0.1.0` gains
`fresh-salts: func(count: u32) -> result<list<list<u8>>, signing-error>`.
The package version stays `0.1.0` (pre-release, changed in place; an old
guest that does not import the function still links).

**Methods (C2a):**

| Method | Today | After C2a |
| --- | --- | --- |
| `quote.set` | | refuses `quote-too-large` (data `{"code": "quote-too-large"}`) |
| `quote.measure` | | new (Q4): `{terms}` → `{bytes, limit, details: [{commitment_id, bytes, limit}], fits}` |
| `quote.get`, `quote.list` | envelope only | add `terms` (plain) |
| `quote.history` | `{history: [envelope]}` | adds `versions: [{record_id, terms}]` |
| `quote.verify`, `agreement.verify`, `payment.verify`, `fulfilment.verify` | payload plain | payload in wire form (`_sd`, no hidden values) |
| `transaction.thread` | card `data` = payload | card `data` = plain form, no `_sd` (unchanged for the Hub) |
| `transaction.export` / `.import` | | new section `disclosures` |

**Methods (C2b):**

| Method | After C2b |
| --- | --- |
| `quote.verify`, `agreement.verify`, `payment.verify`, `fulfilment.verify` | input `{envelope, disclosures?}`; output adds `shown`, `hidden`, `disclosure_refused` |
| `deal.present` | new: `{record_id, include?, show}` → `Presentation` |
| `deal.verify-presentation` | new: `Presentation` → `PresentationView` |

### Behaviour changes

C2a:

1. A signed deal record holds no hidden value. A shown receipt or
   settlement record no longer shows the price, the texts or the payment
   details.
2. The counterparty's node refuses a quote card that misses a disclosure,
   has a disclosure that matches no digest, a salt that is not 16 bytes, a
   salt used twice, or disclosures over the limits. Today it files any
   quote whose envelope verifies.
3. The same for settlement cards, for their own hidden fields.
4. `quote.set` refuses a quote whose disclosures pass 96 KiB, or whose one
   `details` passes 8 KiB.
5. ML-18's `viewed_via_mt` rule moves from the record verifier to the
   places that hold the plain terms (Q5). `agreement.verify` without
   disclosures no longer checks it.
6. The backup import rebuilds each card row's `data` and each agreement
   row's `terms` from the signed records and the disclosures, and refuses a
   bundle whose disclosures do not match.
7. The Hub's quote form shows a size line.
8. `SCHEMA_VERSION` of `transaction` goes from 5 to 6
   (`crates/roym_transaction/src/app.rs:52`): stored data of version 5 is
   not read.

C2b:

9. `*.verify` with disclosures shows the disclosed fields.
10. A party can build a presentation, and any node with `transaction` can
    verify one.

### Stays the same

- Every method is local-only. No wire-reachable method is added.
- Record ids are still the content hash of the envelope; disclosures are
  not in the envelope.
- The canonical bytes (`canonicalize_json_value`,
  `crates/signed_record/src/envelope.rs:102`), the 64 KiB payload limit and
  the 160 KiB card body limit.
- The plain `DealTerms` type and its full validator (`deal.rs:302`). Every
  local row keeps plain terms: `AgreementRow.terms` (`app.rs:157`), the
  writer's decision, the tick, settlement, cancel and the thread.
- What the Hub draws. Every card template, `agreement.get`, `agreement.list`
  and story 40.
- The snapshot: no hidden field, still signed by the service key.
- `roymctl`: it sends plain terms to `quote.set` and prints plain
  `agreement.get` output.
- `conversation`, `profile`, `directory`, `catalog` and `web` Rust code
  (only their regenerated bindings change).

## 5. Decisions this plan makes

Each decision is written into the agreements doc's `## Deviations` when it
differs from the doc's text (marked "deviation").

- **D-C2-1. Module layout.**
  - `crates/signed_record/src/disclosure.rs` (+ `disclosure/tests.rs`): the
    RFC 9901 primitives (Q3).
  - `crates/roym_core/src/disclosure.rs` (+ `disclosure/tests.rs`): hidden
    sets, `conceal_object`, `DisclosureIndex`, `reveal_object`, limits,
    `SdError`.
  - `crates/roym_core/src/deal/sd.rs` (+ `deal/sd/tests.rs`): `SignedTerms`,
    `SignedCommitment`, conceal and reveal of terms.
  - `payment.rs`, `fulfilment.rs`: the wire payloads and their `*Hidden`
    structs, in place.
  - C2b: `crates/roym_core/src/presentation.rs` (+ `presentation/tests.rs`).
  - `crates/roym_transaction/src/app/disclosures.rs`: the store, the salt
    fetch and `load_quote`.
  - C2b: `crates/roym_transaction/src/app/present_ops.rs`.
- **D-C2-2. Wire types are strict.** `SignedTerms`, `SignedCommitment` and
  the three settlement payloads use `#[serde(deny_unknown_fields)]`. A
  payload that carries a hidden field in plain form fails to parse, so the
  verifier refuses it. This is AG-17's guard in the type, not only in a
  test.
- **D-C2-3. Plain types stay for local data.** `DealTerms`, `Commitment`
  and `LocalText` keep their shape. They are no longer signed directly.
  `CardRow.data` holds the plain form of the record with every `_sd`
  removed.
- **D-C2-4. One local store for disclosures.** New collection
  `disclosures` in `transaction`, row id = the record's `record_id`:
  `DisclosureRow { record_id, record_type, conversation, disclosures:
  Vec<String>, stored_at_secs }`, index on `conversation`. Written *before*
  the record's own row: before `QUOTE_HISTORY` for a quote, before the
  fence row for a settlement record, before the half on filing. A crash
  after the disclosures and before the record leaves an unused row, which
  is harmless. A receipt has no row; its disclosures are the quote's.
- **D-C2-5. `send_card_and_file` takes an `OutgoingCard`.**
  `pub(crate) struct OutgoingCard<'a> { card_type: &'a str, version: u32,
  envelope: &'a str, send: &'a [String], view: &'a [String] }`. `send` goes
  into the card; `view` reveals this node's own card row (`view` = `send`,
  except for a receipt half, where `view` is the quote's disclosures). The
  new signature is `send_card_and_file(host, conversation, &OutgoingCard,
  now, version_count)`; `file_own_card` takes the plain `data` it stores.
  It drops `#[allow(clippy::too_many_arguments)]` (`app.rs:471`).
- **D-C2-6. One helper loads a stored quote.**
  `pub(crate) async fn load_quote(host, record_id, now) ->
  Result<HeldQuote, LoadQuoteError>` with `HeldQuote { envelope: String,
  verdict: RecordVerdict<QuotePayload>, signed: QuotePayload, terms:
  DealTerms, disclosures: Vec<String> }`. It reads `QUOTE_HISTORY` and
  `DISCLOSURES`, verifies, and runs the strict reveal. `LoadQuoteError` is
  `Missing | Storage(String) | Invalid(String)`, so each caller keeps its
  own words. It replaces the five copies of section 2.2 point 5.
- **D-C2-7. Filing rules.** A quote or settlement card is filed only after
  the strict reveal of its own disclosures passes, and the plain record
  passes its full validator. The reason text names the rule and the field,
  never a value (a refused card's reason is stored and shown; a value in it
  would leak).
- **D-C2-8. Salts are fetched first, then concealed by a pure function.**
  `DealTerms::hidden_field_count()` is `5 + 10 × commitments`. The service
  asks the host for that many salts in one call (`AppSigning::fresh_salts`),
  then calls the pure `DealTerms::conceal(&self, &mut salts)` with an
  iterator. Tests pass their own salts. The host refuses `count` 0 or over
  256 (`invalid-record`); a quote needs at most 165.
- **D-C2-9. Deal verify input (C2b).** `DisclosedVerifyInput { envelope:
  String, #[serde(default)] disclosures: Vec<String> }`,
  `deny_unknown_fields`, for `quote.verify`, `agreement.verify`,
  `payment.verify` and `fulfilment.verify`. Output
  `DisclosedVerdict<P> { #[serde(flatten)] verdict: RecordVerdict<P>,
  shown: Option<Value>, hidden: Vec<String>, disclosure_refused:
  Option<String> }`. `verified` still says only whether the envelope
  verifies. Deviation: the doc lists only `quote.verify` and
  `agreement.verify` as taking disclosures; the two settlement verifies
  take them too, so a settlement record's note can be checked the same
  way.
- **D-C2-10. `SCHEMA_VERSION` 5 → 6.**
- **D-C2-11. `quote.measure` (Q4).** Input `{ terms: DealTermsInput }`;
  it builds `DealTerms` with `quote_expires_at_secs` 0 and runs
  `DealTerms::disclosure_size()`, which conceals with zero salts and sums
  the encoded lengths. `quote.set` calls the same function before it asks
  for salts. Output `{ bytes: u64, limit: u64, details: [{ commitment_id,
  bytes, limit }], fits: bool }`. It does not validate the terms (the form
  is half filled while the person types); a body that does not parse
  answers `-32602`.
- **D-C2-12. Import rebuilds plain data.** `transaction.import` checks the
  `disclosures` section first (each row names a record in the bundle; each
  disclosure matches a digest of that record; a quote's set is complete and
  strict). Then it builds `AgreementRow.terms` from the quote and its
  disclosures (a bundle row whose `terms` differ is refused), and builds
  each deal card row's `data` from the envelope and the disclosures (a card
  whose disclosures are missing is imported with `verified: false` and the
  reason "disclosures missing").
- **D-C2-13. Seeded salts for tests (Q2).** `RecordSalts { System,
  Seeded(u64) }` in `crates/core/src/record_signer.rs`, beside
  `RecordClock`. Seeded salt number `n` is the first 16 bytes of
  `SHA-256(seed as 8 little-endian bytes ‖ n as 8 little-endian bytes)`,
  with `n` from an `AtomicU64`. `NodeRecordSigner::with_clock` gains a
  `salts` argument. The parity harness seeds both stacks with the same
  value. `TestHost` keeps its own counter, settable with
  `set_salt_seed(u64)`.
- **D-C2-14. Presentation (C2b).** `deal.present { record_id, include:
  { quote: bool = true, receipt: bool = true, progress: bool = false,
  settlement: bool = false }, show: [field path] }`. `record_id` is the
  quote version's `record_id`, which is also the agreement id, so one key
  names both a quote-only and an agreement presentation. `receipt` is
  ignored when no half is held. A path that names no hidden field, or a
  record not included, answers `-32602` with the path. At most 64 records
  per presentation.
- **D-C2-15. Field paths.** As the doc (`change.md:712`):
  `terms.<field>`, `terms.commitments[<commitment_id>].<field>`,
  `<record_id>.<field>`. Parsed by one function, `FieldPath::parse`, in
  `roym_core::disclosure`, used by `deal.present`, the partial reveal's
  `hidden` list and the verifier's output.
- **D-C2-16. Checks on a partial reveal (C2b).** Each shown field is
  parsed into its Rust type and runs its own limit check (the same
  functions the full validator uses). A rule that spans fields (`tax +
  fees ≤ value`, currency-only fields `null`, `payee` not empty, `details`
  matches `extension`, AG-39) runs only when all its fields are shown, and
  is reported in `not_checked` otherwise.
- **D-C2-17. Visible checks move to the wire type.**
  `SignedTerms::validate_visible()` checks what needs no disclosure: the
  count, the currency, `home_exchange`, the ids, the kind table, the
  extension shape, the `after` graph, and each `_sd` list (exact length 5
  or 10, strictly ascending, each 43 characters of base64url). To avoid a
  second copy, `validate_commitment_kind` (`deal.rs:345`) and
  `validate_after_graph` (`deal.rs:503`) take the visible fields as
  arguments and are called from both `DealTerms::validate` and
  `SignedTerms::validate_visible`.
- **D-C2-18. Size limits apply on both sides.** `quote.set` refuses, and
  the filing node also refuses a received record over the limits.
- **D-C2-19. Clean-ups of section 2.2 point 7.**

## 6. Step-by-step changes

Each step ends with `cargo build --workspace`, the named tests passing, and
a commit. Steps 5 and 6 are one pair: after step 5 the `roym_core` and
in-crate `transaction` tests pass; the parity, shape and two-node tests
pass again only after step 6. Run `mise run build:roym` before every parity
run.

### C2a

#### Step 1 — Commit this plan

This plan, the C2 deviation rows in the agreements doc, and the status.md
rows (this commit).

#### Step 2 — Disclosure primitives (`syneroym-signed-record`)

Add `base64` (workspace, `"0.23"`, `Cargo.toml:156`) to
`crates/signed_record/Cargo.toml`. New `src/disclosure.rs`:

```rust
pub const SALT_BYTES: usize = 16;
pub const SALT_CHARS: usize = 22;
pub const DIGEST_CHARS: usize = 43;

pub struct Salt([u8; SALT_BYTES]);
impl Salt {
    pub fn from_bytes(bytes: [u8; SALT_BYTES]) -> Self;
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, DisclosureError>; // len 16
    pub fn encoded(&self) -> String; // base64url, no padding, 22 chars
}

/// `base64url(compact JSON [salt, name, value])`.
pub fn encode_disclosure(salt: &Salt, name: &str, value: &Value) -> Result<String, DisclosureError>;
/// `base64url(SHA-256(ASCII bytes of `disclosure`))`, 43 chars.
pub fn disclosure_digest(disclosure: &str) -> String;

pub struct DecodedDisclosure { pub salt: String, pub name: String, pub value: Value, pub digest: String }
/// Accepts any JSON spacing (the RFC's own example has spaces). Refuses:
/// not base64url without padding, not a JSON array of exactly 3, a salt or
/// name that is not a string, any number in `value` that is not an integer.
pub fn decode_disclosure(disclosure: &str) -> Result<DecodedDisclosure, DisclosureError>;

pub enum DisclosureError { NotBase64Url, NotJson, NotThreeElements(usize),
    SaltNotString, NameNotString, NonIntegerNumber, SaltLength(usize) }
```

The integer check reuses the payload walk of `RecordDraft::validate`
(`envelope.rs:100`, `walk`): make it `pub(crate)` and call it on `value`.
Tests: section 7.1.

#### Step 3 — `fresh-salts` on the host (Q2)

- `crates/wit_interfaces/wit/signing/signing.wit`: add, after
  `sign-record`:
  ```wit
  /// Fresh salts for the hidden fields of a record this component is about
  /// to sign. Each is 16 bytes from the host's secure random source.
  /// `count` is 1 to 256; anything else is `invalid-record`.
  fresh-salts: func(count: u32) -> result<list<list<u8>>, signing-error>;
  ```
- `crates/app_host/src/lib.rs:120`: `AppSigning::fresh_salts(&self, count:
  u32) -> impl Future<Output = Result<Vec<Vec<u8>>, SigningError>> + Send`.
- Implement it in `GuestHost` (`crates/app_host/src/guest.rs:431`, calls
  the binding), `NativeAppHost` (`crates/app_host_native/src/host.rs:668`,
  calls `HostSigning::fresh_salts`), `HostState`
  (`capabilities_services.rs:52`, calls `NodeRecordSigner::fresh_salts`),
  and `TestHost` (`crates/roym_core/src/testing.rs:213`, the seeded counter
  of D-C2-13).
- `NodeRecordSigner::fresh_salts(&self, count) -> Result<Vec<[u8; 16]>,
  SigningError>`: `System` uses `getrandom::fill`; `Seeded` as D-C2-13. A
  read-only instance may ask for salts (it cannot sign, so they are
  useless, but harmless).
- Regenerate the six Roym `bindings.rs` with `mise run build:roym`.
- Tests: `fresh_salts_are_sixteen_bytes_and_differ`,
  `seeded_salts_repeat_for_the_same_seed`, `count_zero_and_257_are_refused`
  in `crates/core`; one WASM test in `crates/sandbox_wasm` that a component
  call reaches the host function.

#### Step 4 — Roym's hiding rules (`roym_core`, added beside today's code)

Nothing switches yet; the new code is used only by its tests.

`crates/roym_core/src/disclosure.rs`:

```rust
pub const MAX_RECORD_DISCLOSURE_BYTES: usize = 96 * 1024;
pub const MAX_DETAILS_DISCLOSURE_BYTES: usize = 8 * 1024;

pub struct HiddenSet { pub hidden: &'static [&'static str], pub visible: &'static [&'static str] }
pub const TERMS_SET: HiddenSet;           // listing_id, scope, cancellation_terms, refund_terms, dispute_path
pub const COMMITMENT_SET: HiddenSet;      // description … eep (10)
pub const PAYMENT_REQUEST_SET: HiddenSet; // note
pub const PAYMENT_ACK_SET: HiddenSet;     // method, reference, settlement_note
pub const FULFILMENT_SET: HiddenSet;      // settlement_note

/// Moves each hidden name out of `obj` (a missing key becomes `null`),
/// adds `_sd` (sorted). Refuses a key that is in neither list, so a new
/// field nobody classified cannot be signed by accident.
pub fn conceal_object(obj: &mut Map<String, Value>, set: &HiddenSet,
    salts: &mut impl Iterator<Item = Salt>) -> Result<Vec<String>, SdError>;

pub struct DisclosureIndex { /* digest -> DecodedDisclosure, encoded bytes */ }
impl DisclosureIndex {
    /// Decodes each; refuses the same digest twice.
    pub fn build(disclosures: &[String]) -> Result<Self, SdError>;
    /// Roym rules: every salt decodes to 16 bytes; no salt twice.
    pub fn check_salts(&self) -> Result<(), SdError>;
    pub fn total_bytes(&self) -> usize;
}

pub struct RevealedObject { pub fields: Map<String, Value>, pub hidden: Vec<&'static str>, pub used: Vec<String> }
/// Checks `_sd` (length = set.hidden.len(), strictly ascending, 43 chars
/// each). For each digest found in `index`: the name must be in
/// set.hidden, must not be a visible field, must not appear twice.
pub fn reveal_object(obj: &Map<String, Value>, set: &HiddenSet, index: &DisclosureIndex)
    -> Result<RevealedObject, SdError>;
/// A disclosure that no object used is refused (AG-19 (4), first case).
pub fn check_all_used(index: &DisclosureIndex, used: &[String]) -> Result<(), SdError>;

pub struct FieldPath { /* Terms(field) | Commitment(id, field) | Record(record_id, field) */ }
pub enum SdError { /* each with code(): "sd-shape", "disclosure-unknown-digest",
    "digest-repeated", "disclosure-name", "disclosure-visible-name",
    "disclosure-value", "salt-length", "salt-repeated", "quote-too-large",
    "details-too-large", "disclosure-missing" */ }
```

`crates/roym_core/src/deal/sd.rs`:

```rust
#[serde(deny_unknown_fields)]
pub struct SignedTerms { pub currency: String, pub quote_expires_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub home_exchange: Option<String>,
    pub commitments: Vec<SignedCommitment>, #[serde(rename = "_sd")] pub sd: Vec<String> }
#[serde(deny_unknown_fields)]
pub struct SignedCommitment { pub commitment_id: String, pub giver: DealRole,
    pub resource_kind: ResourceKind, pub kind: CommitmentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub extension: Option<ExtensionRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub after: Vec<String>,
    #[serde(rename = "_sd")] pub sd: Vec<String> }

pub struct ConcealedTerms { pub signed: SignedTerms, pub disclosures: Vec<String> }
pub struct DisclosureSize { pub total_bytes: usize, pub details: Vec<(String, usize)> }

impl DealTerms {
    pub fn hidden_field_count(&self) -> usize;
    pub fn conceal(&self, salts: &mut impl Iterator<Item = Salt>) -> Result<ConcealedTerms, SdError>; // checks both limits
    pub fn disclosure_size(&self) -> Result<DisclosureSize, SdError>; // zero salts
}
impl SignedTerms {
    pub fn validate_visible(&self) -> Result<(), DealError>; // D-C2-17
    pub fn reveal(&self, index: &DisclosureIndex) -> Result<PartialTerms, SdError>;
}
pub struct PartialTerms { pub value: Value, pub hidden: Vec<FieldPath>, pub used: Vec<String> }
/// The filing check: build, check_salts, both limits, reveal, all used,
/// nothing hidden, deserialize, `DealTerms::validate`.
pub fn reveal_terms_strict(signed: &SignedTerms, disclosures: &[String]) -> Result<DealTerms, SdError>;
```

Settlement records (`payment.rs`, `fulfilment.rs`): add `PaymentRequestHidden
{ note: Option<LocalText> }`, `PaymentAckHidden { method: Option<String>,
reference: Option<String>, settlement_note: Option<LocalText> }` and
`FulfilmentHidden { settlement_note: Option<LocalText> }`, each with no
`skip_serializing_if` (so `null` is disclosed) and a `validate()` that holds
today's checks of those fields (`payment.rs:79` to `:81`, `:126` to `:136`;
`fulfilment.rs:66` to `:68`). Generic helpers in `disclosure.rs`:
`conceal_hidden<T: Serialize>(hidden: &T, set, salts) -> Result<(Vec<String>,
Vec<String>), SdError>` (digests, disclosures) and
`reveal_hidden_strict<T: DeserializeOwned>(sd: &[String], set, disclosures)
-> Result<T, SdError>`.

`card.rs`: `Card.disclosures` and `card_body(card_type, version, envelope,
disclosures: &[String])`.

Tests: section 7.1.

#### Step 5 — The switch, Rust side

`roym_core`:

- `QuotePayload.terms` and `AgreementReceiptPayload.terms` become
  `SignedTerms`. `QuotePayload::validate` calls `terms.validate_visible()`.
  `AgreementReceiptPayload::validate` drops the scope check (Q5); new
  `pub fn check_viewed(viewed_lang: &LangTag, viewed_via_mt: bool, terms:
  &DealTerms) -> Result<(), TransactionError>` in `transaction.rs` holds it,
  used by `agreement_ops::viewed` (`agreement_ops.rs:296`), the half filing
  and the import.
- The settlement payloads drop their hidden fields and gain `#[serde(rename
  = "_sd")] sd: Vec<String>` (lengths 1, 3, 1, checked in `validate`).
  `commitment_accepts` (`payment.rs:143`) is unchanged; its caller passes
  the plain method.
- Delete the comment at `deal.rs:144`. Fix `card.rs:6`.

`transaction`:

- `app/disclosures.rs`: `DISCLOSURES`, `DisclosureRow`, `store`, `load`,
  `fresh_salts(host, count) -> Result<Vec<Salt>, Response>`, `load_quote`
  (D-C2-6), `plain_view(payload: &Value, disclosures: &[String]) -> Value`
  (generic: puts disclosed fields back where their digest sits, removes
  every `_sd`; used for `CardRow.data` only, after the strict check).
- `ensure_collections` (`app.rs:327`): add `DISCLOSURES` with an index on
  `conversation`. `SCHEMA_VERSION` = 6.
- `send_card_and_file` and `file_own_card` as D-C2-5; update all 15 callers
  (section 2.2 point 3).
- `quote.set` (`quote_ops.rs:101`): after `into_deal_terms`, run
  `DealTerms::validate`, then `disclosure_size` (refuse `quote-too-large` /
  `details-too-large` with the code in `data`, like `quote_ops.rs:149` to
  `:158`), then `fresh_salts(hidden_field_count)`, `conceal`, build
  `QuotePayload` with `signed`, `validate`, and in `sign_and_store_quote`
  store the disclosures between the signing (`:372`) and `put_bytes`
  (`:393`). Send with `send = view = disclosures`.
- `quote.measure` (D-C2-11), registered in `api.rs`.
- Quote views: `QuoteView.terms`, `QuoteListItem.terms` (via `load`, plain
  `reveal_terms_strict` of the stored disclosures), and `quote.history`'s
  `versions` (D-C2-15 in section 4's table).
- `agreement.accept` (`agreement_ops.rs:113`): `load_verified_quote_and_role`
  uses `load_quote`; the half copies `held.signed.terms`; `viewed` reads
  `held.terms`; send with `send = []`, `view = held.disclosures`.
- Writer's countersign (`decide.rs:683`): copies the quote's wire `terms`;
  `decide::run` gets the `HeldQuote`; send with `view` = the quote's
  disclosures.
- Payment request, acknowledgement and fulfilment sign paths
  (`payment_ops.rs:124`, `:263`, `:553`, `:586`; `fulfilment_ops.rs:130`,
  `:351`): conceal the `*Hidden` struct, store the disclosures, then take
  the fence. Every resend from a stored half (`payment_ops/fence.rs:136`,
  `:325`; `fulfilment_ops.rs:219`, `:523`; `payment_ops.rs:177`, `:644`)
  loads the disclosures by the half's `record_id`.
- Filing (`sync.rs` and `sync/*.rs`): `dispatch_known_card` passes
  `&card.disclosures` to each filer. Quote and settlement filers run the
  strict reveal (D-C2-7), store the disclosures, set `row.data =
  plain_view(...)`. The receipt filer refuses non-empty disclosures, loads
  the quote with `load_quote`, compares the wire `terms` (AG-20,
  `sync.rs:526`), runs `check_viewed` against `held.terms`, sets
  `AgreementRow.terms = held.terms`, and `row.data` from the quote's
  disclosures. Progress, enquiry and decline filers refuse non-empty
  disclosures.
- `sync/receipts.rs:53`, `sync/decline.rs:31`, `decline_ops.rs:81`: use
  `load_quote`.
- Backup (`backup.rs`, `backup/sections.rs`): export the new section;
  import as D-C2-12, with the disclosure checks in a new
  `backup/disclosures.rs` so `backup.rs` (727 lines) and `sections.rs`
  (707) do not grow. New `SECTION_DISCLOSURES = "disclosures"` in
  `crates/roym_core/src/backup.rs` beside `SECTION_FULFILMENTS` (`:40`),
  and in the section list at `:445` to `:455`.

In-crate tests: section 7.2.

#### Step 6 — The switch, callers and fixtures

- Parity (`crates/roym_web/tests/dual_build_parity`): both stacks seeded
  with the same `RecordSalts::Seeded` in `helpers.rs` (beside `:1042` and
  the native signer). Peer fixtures conceal with the test's own salts and
  pass the disclosures to `card_body` (section 2.2 point 8). New fixture
  helpers go into `deal_fixtures.rs` (233 lines), not `fixtures.rs` (773).
  New scenarios in a new file `disclosure.rs` (section 7.3).
  `transaction_validation.rs:27` lists `quote.measure` for the admission
  table (AG-31).
- Method shapes (`crates/roym_web/tests/method_shapes/`): update
  `quote_verify_good.json`, `agreement_verify_good.json`,
  `payment_verify_ack.json`, `payment_verify_request.json`,
  `fulfilment_verify_good.json`, `quote_get_found.json`,
  `quote_list_with_rows.json`, `quote_history.json`,
  `transaction_export.json`; add `quote_measure.json` and a row in
  `method_contract/shapes_transaction.rs`.
- Two-node: `roym_transaction_e2e.rs:293` to `:310` as section 2.2 point 8;
  `roym_restore_e2e.rs` gains the `agreement.get` check.
- `card.rs` test `quote_dense_envelope_near_max_payload_fits_in_card_body`
  (`card.rs:187`) passes `&[]`.

#### Step 7 — Hub size line

- `crates/roym_web/ui/src/screens/quote_size.ts` (new, so `messages.ts` at
  1018 lines does not grow much): `attachSizeLine(form, readTerms: () =>
  DealTermsInput | undefined)`. On `input` of the form, after 300 ms with
  no new input, call `quote.measure` with `readTerms()`; draw
  `t("quote-size-line", { used, limit })` with sizes in KiB to one decimal;
  when `fits` is false add class `quote-size-over` and
  `t("quote-size-over")`. On any error, or when `readTerms()` is
  `undefined` (a required field does not parse yet), hide the line.
- `messages.ts`: factor the term building of `submitBtn.onclick`
  (`messages.ts:861` to `:953`) into one `readQuoteForm(): {terms} |
  {error}` used by both submit and the size line, so the two cannot
  disagree. Append the line before `errP` in `form.append` (`:956` to `:975`).
- `en.ftl`, `hi.ftl`: `quote-size-line`, `quote-size-over`.
- Delete `cards/templates/booking_progress.ts` and the `booking-progress`
  case at `render.ts:36`.
- Story 50 (`demo/50-roym-deal.spec.ts:78`): in Bob's quote step, check
  that `.quote-size-line` shows "of 96 KiB" before he sends.

#### Step 8 — Sizes, docs and close-out of C2a

Component sizes (section 7.9), deviations, backlog, living docs (section 9),
status row, `mise run verify`.

### C2b

#### Step 1 — Partial reveal and presentations (`roym_core`, pure)

`crates/roym_core/src/presentation.rs`:

```rust
pub const PRESENTATION_VERSION: u32 = 1;
pub const MAX_PRESENTATION_RECORDS: usize = 64;

pub struct Presentation { pub presentation_version: u32, pub records: Vec<String>, pub disclosures: Vec<String> }

pub struct PresentationView {
    pub deal: DealHeader,              // quote_id?, agreement, quote_author_did, counterparty_did, currency, quote_expires_at_secs
    pub records: Vec<RecordCheck>,     // record_type, record_id, issuer, issued_at_secs, verified
    pub terms: Value,                  // shown fields only, no _sd
    pub hidden: Vec<String>,           // field paths not shown
    pub commitments: Vec<CommitmentOutcome>, // id, giver, resource_kind, kind, extension, state?, reason?, claimed?
    pub settlement: Vec<SettlementView>,     // record_id, commitment_id, role, shown, hidden
    pub writer: WriterCheck,           // Checked | NotChecked | NoSnapshot
    pub not_checked: Vec<String>,      // rules that need a field not shown
}

pub fn verify_presentation(p: &Presentation, now_secs: u64) -> Result<PresentationView, PresentationError>;
```

Rules, in order:

1. `presentation_version` is 1; 1 to 64 records.
2. Each record is parsed as an envelope and run through its own verifier
   (`verify_quote`, `verify_agreement_receipt`, `verify_agreement_progress`,
   the settlement verifiers). Any other type, or one that does not verify,
   refuses the presentation.
3. One deal: at most one quote, at most one half per role, at most one
   snapshot. Every half's `quote_record_id`, the snapshot's `agreement` and
   each settlement record's `agreement` equal the quote's `record_id` (when
   a quote is present) or each other. Two halves must `halves_agree`
   (`transaction.rs:372`). A quote and a half must have equal wire `terms`
   (AG-34).
4. Writer (AG-38): `Checked` only when the snapshot's issuer equals the
   `signer_did` of the quote author's half, or of the quote.
5. Disclosures: `DisclosureIndex::build`; reveal the terms (from the quote,
   or from a half when no quote) and each settlement record's own `_sd`;
   `check_all_used`. Salt length is not checked here (AG-19 says it is a
   record rule for the parties).
6. Field checks as D-C2-16; ML-18's check when `scope` is shown (Q5).

`transaction` partial reveal for the single-record verifies uses the same
`reveal` functions.

#### Step 2 — Methods

- `crates/roym_transaction/src/app/present_ops.rs`: `present` (D-C2-14) and
  `verify_presentation`. `deal.present` picks, for each path, the stored
  disclosure whose digest sits in the named object (`locate(signed, index,
  path)`), never a disclosure of another object.
- The four verifies take `DisclosedVerifyInput` (D-C2-9).
- `api.rs`: `deal.present`, `deal.verify-presentation`. Admission table
  rows (`transaction_validation.rs:27`, `:289`) and method shapes
  (`deal_present.json`, `deal_verify_presentation.json`, and the four
  verify shapes with `shown`/`hidden`).

#### Step 3 — Two-node with a third node

New file `crates/substrate/tests/roym_disclosure_e2e.rs` (the deal file is
at 714 lines). Three installations as in `roym_trust_e2e.rs:8` to `:57`,
using the shared harness (`SubstrateTestContext`, `alloc_ports`). Section
7.4.

#### Step 4 — Close-out of C2b

As C2a step 8. Developer guide section for `deal.present` (section 9).

## 7. Tests

Test names say what they prove, with no planning ids.

### 7.1 Unit tests

`crates/signed_record/src/disclosure/tests.rs` (C2a):

- `rfc9901_digest_vector`: `disclosure_digest("WyJfMjZiYzRMVC1hYzZxMktJNmNCVzVlcyIsICJmYW1pbHlfbmFtZSIsICJNw7ZiaXVzIl0")`
  is `X9yH0Ajrdm1Oij4tWso9UzzKJvPoDxwmuEcO3XAdRC0` (AG-19 (1)).
- `rfc9901_disclosure_with_spaces_decodes` to salt `_26bc4LT-ac6q2KI6cBW5es`,
  name `family_name`, value `Möbius` (AG-19 (2)).
- `our_encoder_output_is_pinned`: salt bytes `0x00..=0x0f`, name `scope`,
  a fixed `LocalText`; the exact string and digest, written once from the
  first run and checked by hand against the RFC's algorithm (AG-19 (3)).
- `decoder_refuses_padding_and_non_base64url`, `decoder_refuses_two_and_four_elements`,
  `decoder_refuses_a_non_string_salt_or_name`,
  `decoder_refuses_a_fractional_number_deep_in_the_value` (AG-19 (4), last case).

`crates/roym_core/src/disclosure/tests.rs` (C2a):

- `a_disclosure_whose_digest_is_not_in_the_record_is_refused`,
  `a_digest_listed_twice_is_refused`, `a_name_that_is_not_hidden_here_is_refused`,
  `a_name_equal_to_a_visible_field_is_refused` (AG-19 (4)).
- `a_salt_of_another_length_is_refused`, `a_salt_used_twice_in_one_record_is_refused`
  (AG-21).
- `an_unclassified_field_cannot_be_concealed`.
- `field_paths_parse_and_print`.

`crates/roym_core/src/deal/sd/tests.rs` (C2a):

- `no_signed_payload_holds_a_hidden_value` (AG-17, AG-33): terms with
  markers (scope text `scope-marker-71`, declared value `987654321`, payee
  `payee-marker-72`, window `earliest 1_700_000_000`, `latest
  1_700_003_600`, `settlement_note` `note-marker-73`, method
  `method-marker`, reference `ref-marker-74`); build the quote, both halves,
  the three settlement payloads and a snapshot; serialise each canonically;
  none contains any marker, nor the decimal digits of the derived track
  end.
- `every_object_always_has_all_its_digests` (5 and 10, `null` values
  disclosed).
- `strict_reveal_gives_back_the_same_terms`.
- `both_halves_copy_the_quote_terms_byte_for_byte` (AG-20).
- `details_over_8_kib_is_refused`, `disclosures_over_96_kib_are_refused`,
  `disclosure_size_equals_the_concealed_size` (AG-40).
- `visible_checks_refuse_a_bad_sd_list` (length, order, characters).
- The AG-3 test (`deal/tests.rs`, `quote_for_an_enquiry_and_for_a_want_give_equal_receipts`)
  uses fixed salts, as the doc asks.
- `transaction/tests/viewed.rs`: the ML-18 cases move to `check_viewed`.

`crates/roym_core/src/presentation/tests.rs` (C2b):

- `agreement_with_price_hidden_and_scope_shown_verifies` (AG-18).
- `quote_alone_with_price_hidden_verifies` (AG-34).
- `quote_and_half_with_different_terms_are_refused` (AG-34).
- `snapshot_bound_to_the_quote_authors_half_is_the_writers`,
  `snapshot_from_another_key_is_writer_not_checked` (AG-38).
- `presentation_without_disclosures_shows_kinds_and_outcomes`, with
  `claimed: true`, `claimed: false` and a `cancelled` reason (AG-8).
- `records_of_two_deals_are_refused`, `a_disclosure_of_another_object_is_refused`,
  `a_cross_field_rule_with_a_field_hidden_is_not_checked`,
  `more_than_64_records_are_refused`.

### 7.2 In-crate service tests (`crates/roym_transaction/src/app/tests/`)

New file `disclosure.rs` (C2a):

- `quote_card_carries_every_disclosure_and_the_peer_files_plain_terms`.
- `a_quote_card_missing_a_disclosure_is_refused`,
  `a_quote_card_with_a_seventeen_byte_salt_is_refused` (AG-21).
- `each_quote_version_gets_fresh_salts` (AG-21).
- `quote_set_refuses_quote_too_large` (AG-40), with the code in `data`.
- `largest_quote_card_fits_in_the_card_body`: 16 commitments with the
  longest visible fields, descriptions sized so the disclosures are just
  under 96 KiB, signed by the test host's real signer with a delegation
  certificate; `card_body` succeeds and is at most `MAX_CARD_BODY_BYTES`
  (section 2.2 point 1).
- `a_receipt_card_with_disclosures_is_refused`.
- `a_resent_payment_acknowledgement_carries_its_disclosures` (section 2.2
  point 4): crash after the fence, repeat the call, the second card has
  the same disclosures.
- `a_settlement_card_note_is_filed_plain_and_signed_hidden` (AG-33).
- `quote_measure_matches_quote_set`.

`backup/tests.rs` gains `mod disclosures;` → `backup/tests/disclosures.rs`:
`export_import_keeps_disclosures`, `import_rebuilds_card_data_from_disclosures`,
`import_refuses_disclosures_that_match_no_digest`,
`import_refuses_agreement_terms_that_differ_from_the_quote`.

C2b, new file `present.rs`: `present_shows_only_the_named_paths`,
`present_refuses_a_path_that_is_not_hidden`, `verify_with_disclosures_shows_them`,
`verify_with_a_bad_disclosure_keeps_verified_and_names_the_problem`.

### 7.3 Parity (both builds agree)

C2a: every existing scenario passes with seeded salts. New
`disclosure.rs`: a quote card carries disclosures; a peer quote with a
missing disclosure is refused; `quote.measure`; a payment acknowledgement
with a note. C2b: `deal.present` and `deal.verify-presentation` give equal
answers. Admission tables list every new method (AG-31).

### 7.4 Two-node (`crates/substrate/tests`)

C2a: `roym_transaction_e2e.rs` checks plain values through `agreement.get`
and that the signed halves hold no `payee` (AG-17 on real cards);
`roym_restore_e2e.rs` checks plain terms after a restore.

C2b, `roym_disclosure_e2e.rs`:

- `a_third_node_verifies_an_agreement_with_the_price_hidden` (AG-18): Bob
  and Alice agree; Bob calls `deal.present` with `show:
  ["terms.scope"]`; Carol's node calls `deal.verify-presentation`; both
  halves verify, `terms.scope` is shown, and every
  `terms.commitments[cN].declared_value_minor` is in `hidden`.
- `a_third_node_verifies_a_quote_before_any_agreement` (AG-34).

### 7.5 Hub unit tests (`npm test`)

- `quote_size.test.ts`: the line shows used and limit; the over class and
  text when `fits` is false; hidden on an error; one call after typing
  stops (fake timers).
- `messages.test.ts`: `readQuoteForm` gives the same terms submit sends.
- `templates.test.ts`: remove the `booking-progress` case.

### 7.6 Playwright

| Case | File:line | C2a change |
| --- | --- | --- |
| story 50 | `demo/50-roym-deal.spec.ts:78` | Bob's quote form shows "of 96 KiB" |
| story 40, Hub cases 3, 4, 24 to 31 | | none: `data` stays plain |

C2b: none.

### 7.7 Not tested

- That a salt is random. No test can check it; the strict reveal checks
  only length and repeats (the doc says the same, `change.md:695`).
- Salt reuse across quote versions by an author's node that is not ours
  (the doc: "the author's duty and is not checked").

### 7.8 Size and time risks

- Files near 800 lines that C2a touches: `decide.rs` 715, `backup.rs` 727,
  `backup/sections.rs` 707, `app.rs` 680, `payment_ops.rs` 656, `sync.rs`
  635, `deal.rs` 580; parity `fixtures.rs` 773 (cap 800, not in the
  oversized list) and `transaction.rs` 821 (capped at 821); two-node
  `roym_deal_e2e.rs` 714. New code goes into `disclosures.rs`,
  `backup/disclosures.rs`, `deal/sd.rs`, `present_ops.rs` and new test
  files. D-C2-5 shortens the 15 send calls.
- `transaction.wasm` gains `base64`, the reveal code and the presentation
  verifier. Check after C2a step 5; if over +15%, stop and tell the owner.
- The parity harness's two stacks must call `fresh_salts` in the same
  order. A scenario that runs two calls at once on one stack could take
  salts in a different order; today's scenarios run steps in sequence.
  If a scenario fails only on digests, check this first.
- `messages.ts` is 1018 lines: step 7 moves term building out, so it
  should shrink.

### 7.9 Component size

Build the six Roym components with `mise run build:roym` on `main`
(`ba5a5597`) and on the branch, in the same build folder, and write before
and after into task.md section 1 and the status row. Expected: `transaction`
grows; the other five change by the new import only. C2b reports again.

### 7.10 Hindi text

New `hi.ftl` messages need the owner's review before merge, as in M08B and
C1: `quote-size-line`, `quote-size-over`. C2b adds none.

### 7.11 How C2 avoids the kinds of findings in the C1 reviews

- **Planning ids in code** (R1): the one at `deal.rs:144` goes; none added.
- **Rows that prove less than they say** (R5, R36, R39): section 3 names the
  part each row proves; test names are checked with `git grep 'fn <name>'`
  before the agreements doc names them.
- **"Made once" broken by a resend** (R2, R28, P16): D-C2-4 stores
  disclosures before the fence; `a_resent_payment_acknowledgement_carries_its_disclosures`
  proves it.
- **Import trusts the bundle** (R13, R31): D-C2-12 rebuilds plain data and
  checks every disclosures row against its record.
- **Recorded shapes left stale** (R17): step 6 lists every shape file.
- **Copied helpers** (R20): one `load_quote`, one `OutgoingCard`, one
  `readQuoteForm`, one size function for `quote.set` and `quote.measure`.
- **Ratchets** (R16): no new `too_many_lines` suppression; lower the
  oversized cap if `dual_build_parity/transaction.rs` shrinks.
- **A wrong language tag in a signed record** (R33): conceal copies the
  `LocalText` as given; no tag is invented.
- **Living docs left false** (R6, R40): section 9 names the lines.
- **Status ahead of facts** (R42, R45): "Merged" only after the merge.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C2 action |
| --- | --- |
| "The forward action offers only three Roym record types" (`:315`) | Unchanged; C7 relies on C2's rule that a forwarded quote carries no disclosures. |
| New: "No Hub screen shows a deal to a third party" (Q6) | C2b adds it, target M08G (evidence). |
| New: "A third party cannot check ML-18 or one-language rules on hidden text" (Q5) | C2b adds it, target TBD (only if a visible language is ever wanted). |
| "`RECORD_TYPES` is not read by any verifier" (`:658`) | Not targeted; C2b's presentation verifier checks record types by its own table. Note it in the row. |

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass.

- `docs/system-requirements-spec.md:431`: field-level selective disclosure
  of deal records is built (C2a: hidden; C2b: presentations).
- `docs/roym-integrated-experience-spec.md`: the Records table (`:477`,
  `agreement-receipt` "these exact terms, including payee …" → the
  receipt signs the digests of those terms; the parties hold the values),
  and the Cards table (`:729` to `:760`).
- `docs/system-architecture.md`: Cards (`:1203` to `:1211`: a card carries
  the envelope and, for four types, the disclosures; remove `terms.lang`
  and `translations`), and the `transaction` collections (new
  `disclosures`).
- `docs/developer-guide.md`: "Show cards" (`:2537` to `:2550`: `data` is the
  plain form, never the signed bytes); the signing host interface section
  (`fresh-salts`); C2b: how to call `deal.present` and
  `deal.verify-presentation`.
- `docs/TERMINOLOGY.md`: hidden field, salt, disclosure, digest,
  presentation; fix "Declared value" (`:69`).
- `docs/planning/traceability-matrix.md`: the selective-disclosure rows.

## 10. Exit criteria

C2a:

- [ ] Every C2a row of section 3 names a test that exists and passes.
- [ ] Story 50 shows the size line; story 40 and Hub cases pass unchanged.
- [ ] Parity passes with seeded salts and no new normalisation.
- [ ] Component sizes recorded; `transaction` within +15%, others within
  +40 KB.
- [ ] Deviations, backlog rows and living docs done.
- [ ] `mise run verify` passes with no `--skip`.
- [ ] Owner reviewed the two Hindi messages.

C2b:

- [ ] Every C2b row of section 3 names a test that exists and passes.
- [ ] The third node verifies in `roym_disclosure_e2e.rs`.
- [ ] `mise run verify` passes with no `--skip`.

## 11. What C2 does not do

- A Hub screen or `roymctl` verb that builds or reads a presentation (Q6).
- The grant that lets a third party see amounts with the other party's
  consent (M08G disclosure ADR). Either party can still show any field; the
  format cannot stop it (`change.md:714`).
- Disclosures in a forwarded quote (C7 forwards none).
- Hiding single fields inside `details.data`, decoy digests, key binding,
  or array-element disclosures (the doc leaves them out).
- Hiding the decline note (Q7).
- Any wire-reachable method.

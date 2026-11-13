# M08B slice B5 — Translation interface: implementation plan

*Reader: the developer (or AI) who implements slice B5. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-13. Not reviewed. Owner questions Q1 to Q7
(section 2.1) are open.**

**Size and PR.** M. One PR on `feat/m08b-b5-translation`, with the commits
of section 6 in that order. The branch starts from `main` at `6083ddb3`
(B4b is merged, [PR #361](https://github.com/syneroym/syneroym/pull/361))
and already holds the plan commit.

B5 adds a place in the substrate where a machine translation engine will
plug in later, and it makes Roym ready to use one. It adds a new host
interface, `syneroym:translation`, and the one implementation that v1
ships: it has no engine, so it answers `unavailable` to every call and
gives an empty list of target languages. A Roym service can call it. The
Hub reaches it through two new local-only methods of the `web` service. The
Hub asks once, after sign-in, which languages the node can translate into.
With the v1 node the answer is "none", so the Hub shows no translate action
anywhere. Each of the four translation hook points of M08B (the directory
search box, record text, chat messages and the quote before acceptance) has
the code to offer the action and to show a labelled machine translation,
tested with a fake engine in Hub unit tests. B5 also adds two fields to the
signed acceptance of an agreement: `viewed_lang` (the language of the text
the person read) and `viewed_via_mt` (whether that text was a machine
translation). Before a person accepts terms that they read through machine
translation, the Hub warns that the original, in its own language, is what
binds.

**Base.** Every file path and line number below was checked on `main` at
`6083ddb3`. Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: B5 needs B3a (tags on
records), B3b (chat tags) and B4b (the search path). All three are "Merged"
in [status.md](status.md). B6 waits for B5.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - No planning ids in code, comments or test names: no "M08B", "B5",
    "ML-19", "D-B5-4", "Q3" or review ids (`P4`, `R7`) in `.rs`, `.ts`,
    `.wit`, `.ftl`, `.json` files (B2 R5, B3a R7, B4a R6).
  - Comments never cite size rules.
  - **File size.** Production files at most 800 lines; test files at most
    800 lines; a file in `xtask/oversized-test-files.txt` is capped at its
    recorded count. Section 7.5 lists the files near a limit
    (`app_host_native/src/host.rs` is at 738).
  - **Function size.** `clippy::too_many_lines` at 100.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out and the demo close-out.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run, and a `dual-build-fixture` change needs
    `mise run build:test-components` ([task.md](task.md) section 2,
    "Records").
- [task.md](task.md): section 1 (Y6, "Component size limit"), section 2
  ("Host capabilities"), "B5", section 4, section 5 (ML-26 is finished
  later), section 7.
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  the terms "Machine translation (MT)" and "Translation hook point"; "Rules
  for every layer" (rules 1, 2 and 4); "D. Machine translation" (the WIT
  draft and the four rules under it); "Agreements across languages";
  "Translation hook points"; criteria ML-15 to ML-19, ML-26 and ML-27; the
  deviation "Warning line `record-text-original-binds` covers gap before
  `viewed_lang` (B5)" (`change.md:546`).
- [ADR-0027](../../../decisions/0027-host-event-feed.md): the last host
  interface that was added. B5 copies its wiring (section 2.2, point 1).
- [slice-b4b-implementation-plan.md](slice-b4b-implementation-plan.md)
  (the layout of this plan) and the eight review files. Section 7.8 lists
  how B5 avoids their kinds of findings.

**Change docs.** The prompt lists one change doc, roym-multilingual.
[task.md](task.md) names no other change doc for B5. Every criterion B5
meets is in roym-multilingual (section 3), and every B5 decision and
deviation is written there, and only there.

One other change doc touches the same fields:
[roym-agreements](../../changes/roym-agreements/change.md) (M08C). Its
criterion **AG-32** (`:145`) refines ML-18 ("kept"): each acceptance half
carries `viewed_lang` and `viewed_via_mt` outside `terms`, and the two
halves of one agreement may differ in them. Its record sketch names the
types (`viewed_lang: LangTag`, `viewed_via_mt: bool`, `:335` to `:336`), and
its method table says `agreement.accept` "Input adds `viewed_lang`,
`viewed_via_mt`" (`:818`). Its gap table says the fields are not in the code
yet (`:170`). AG-32 belongs to roym-agreements and is built in M08C. B5
uses the same names, types and place, so M08C keeps them. B5 writes nothing
into roym-agreements. At M08C time, its `:170` row becomes false; M08C
updates it.

**Do not copy ids into code.** The ids `D-B5-n`, `Qn`, `Wn` and `ML-n` in
this plan are for the plan and the change doc only.

### Terms used in this plan

- **Machine translation (MT):** text translated by software, not by its
  author (change doc, terms). It is for display only and is never signed.
- **Engine:** the software that does machine translation. v1 has none.
- **Translation host interface:** the new WIT package
  `syneroym:translation@0.1.0`. A Roym service calls it like `signing` or
  `events`. The substrate answers.
- **No-op host:** the only v1 implementation of the interface. It answers
  `unavailable` to `detect-language` and `translate`, and an empty list to
  `supported-targets`.
- **Target language:** a language the engine can translate into. The list
  comes from `supported-targets`.
- **Translation hook point:** a place in a Roym flow where the interface is
  called (change doc, terms). B5 builds four (section 1).
- **Translate action:** a button the Hub shows at a hook point. It appears
  only when the target list is not empty (ML-27).
- **Author translation:** a translation the author signed inside the
  record (B3a). It is not MT.
- **Shown version:** the text a person has on screen for a record: the
  original, an author translation, or a machine translation.
- **Acceptance half:** one party's signed `agreement-receipt` record. Its
  payload type is `AgreementReceiptPayload`
  (`crates/roym_core/src/transaction.rs:233`). The stored wrapper
  `ReceiptHalf` (`:245`) holds the envelope; it is not signed itself.
- **Both builds:** a Roym service runs as a `wasm32-wasip2` component, or
  linked into the substrate through `syneroym-app-host-native`. The
  dual-build parity tests check that both give the same answer.
- **Stub engine:** a fake answer in a Hub unit test (through
  `src/testing/stub-rpc.ts`) that says the node can translate. No Rust stub
  engine exists in B5.

## 1. What B5 builds, in one paragraph

A new WIT package `crates/wit_interfaces/wit/translation/translation.wit`
(D-B5-1), its guest and host bindings in `syneroym-wit-interfaces`, a new
`AppTranslation` trait in `syneroym-app-host` that joins the `AppHost`
supertrait, the WASM guest call in `GuestHost`, the no-op host in
`sandbox_wasm` (linked into every component's `Linker`), the native
`impl AppTranslation for NativeAppHost` that calls the same no-op code, and
the import in all six Roym worlds and the dual-build fixture (D-B5-2, Q2).
The `web` service answers two new local-only methods itself,
`translation.targets` and `translation.translate`, through a new
`router::WEB_ROUTES` row (D-B5-3, D-B5-4). The Hub gets a `translation`
module that loads the target list once after sign-in and offers the
translate action at the four hook points: the directory search box (Q1),
`recordText` (listing hits, request cards, quote cards), `messageText`
(direct and group chat), and the quote card before acceptance (D-B5-6 to
D-B5-9). `AgreementReceiptPayload` gets `viewed_lang` and `viewed_via_mt`;
`agreement.accept` takes them (optional, Q3) and checks them against the
quote (Q4); the quote author's automatic countersignature records the
original language (D-B5-5). The Hub sends what the person had on screen and
shows the ML-19 warning before an acceptance through MT (D-B5-9). A new
ADR-0029 records the interface rules (Q7).

## How this is shown to an end user

With the v1 node, an end user sees almost nothing new. There is no engine,
so the Hub shows no translate action anywhere. That is the point of ML-16.

- Bob uses the Hub as before. The directory, the listing cards, the chat
  and the quote cards look the same. No "Translate" button appears.
- Bob gets a quote from Alice in Hindi, with an English translation by
  Alice. He reads the English translation and presses "Accept these
  terms". This works as before.
- Bob's signed acceptance now also says that he read the English text, and
  that it was not a machine translation. He does not see this on screen
  (Q5).
- When a later release adds an engine (M9A), Bob will see a "Translate"
  button at these places, the name of the engine next to every machine
  translation, and a warning before he accepts terms that he read through
  machine translation. B5 builds and tests that path with a fake engine;
  it does not ship it to users.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-web --test dual_build_parity
  translation` runs the web methods through both builds:
  `translation.targets` answers `{"targets": []}`, and
  `translation.translate` answers error `-32020` with `data.kind =
  "unavailable"`.
- `cargo nextest run -p syneroym-roym-web --test dual_build_parity
  agreement_lang` accepts a quote and reads `viewed_lang` and
  `viewed_via_mt` back from both halves of the signed receipt.
- `npm test` in `crates/roym_web/ui` runs the hook point tests: with the
  v1 answer no action is drawn; with a stub engine the action, the labelled
  translation and the warning before acceptance are drawn.

**Demo:** none. Nothing is visible with the v1 engine (task.md B5). Story
40 does not change. The change doc's `demo:` list keeps story 40 only.

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. Where does the search-query hook point run? (blocks Steps 3 and 5)**

The change doc says the search query is translated "on the searching
participant's node, before the query is sent". It does not say whether the
Hub or the directory service asks.

- (a) **In the Hub, at the directory search box.** When the target list is
  not empty, the search box has a "Translate search into …" choice. The Hub
  calls `translation.translate` through `web` (on the person's own node),
  puts the translated text into the box with a note naming the engine, and
  the person presses Search. With v1 nothing is drawn. *Recommended*: one
  way to reach the engine for all four hook points (the `web` method); the
  person sees and can change the translated query; only `web` calls the
  host. `roymctl roym directory find` gets no hook (it has no translation
  at all, change doc "Not in scope": localised `roymctl`).
- (b) In the directory's client half (`directory.find` in
  `roym_directory`), before it sends `directory.search` to each source.
  This covers `roymctl` too. But the service must pick the target
  languages with no input from the person (the languages of a stranger's
  index are unknown), and a translated query that the person never saw
  would change their search results silently.

**Q2. Which Roym components import `syneroym:translation`? (blocks
Step 2)**

`AppHost` is one supertrait of every capability trait
(`crates/app_host/src/lib.rs:40` to `:71`), and every Roym crate's
`generate!` must remap every interface that `syneroym-app-host` binds
(`crates/app_host/src/lib.rs:7` to `:14`; two bindgen passes cannot be
encoded into one component).

- (a) **All six Roym components and the dual-build fixture**, as M08A did
  for `events` (commit `daf112c7`: every Roym `world.wit`, `Cargo.toml`,
  `guest.rs` and a `wit/deps/events` symlink). *Recommended*: the proven
  pattern; one `AppHost`; later hook points that run in other services
  (the exchange search, IM-20 in
  [roym-intent-matching](../../changes/roym-intent-matching/change.md:80))
  need it there. Cost: a few KB of import glue per component (section
  7.6) and 28 small file edits.
- (b) Only `web`: a separate trait bound `H: AppHost + AppTranslation`, and
  a `translation` feature of `syneroym-app-host` that only `web` turns on.
  Fewer edits, but Cargo feature unification in one `cargo build` of all
  six guests can turn the feature on for every guest, and then the other
  five fail to build without the remap. This is a new way of wiring a
  capability that no other capability uses.

**Q3. Are the new `agreement.accept` inputs required? (blocks Step 4)**

- (a) **Optional.** `viewed_lang` absent means the quote's original
  language (`terms.lang`); `viewed_via_mt` absent means `false`. The signed
  record always carries both (they are required there, as AG-32 says).
  *Recommended*: every caller that shows the original (`roymctl roym
  transaction accept`, the substrate e2e tests, the parity deal fixtures,
  `crates/substrate/tests/common/roym_flow.rs:212`) states the truth with
  no change. The Hub always sends both.
- (b) Required. `roymctl` gets `--viewed-lang` and `--viewed-via-mt`; 4
  substrate e2e files, 8 parity call sites and 5 contract call sites
  change (section 2.2, point 9).

**Q4. Which `viewed_lang` values does `agreement.accept` accept? (blocks
Step 4)**

- (a) **When `viewed_via_mt` is `false`: the quote's `terms.lang` or the
  `lang` of one of its author translations, compared as exact canonical
  tags. When `viewed_via_mt` is `true`: any canonical tag except
  `terms.lang`.** Anything else is `-32602` with `data.field`.
  *Recommended*: without an engine, "not MT" means "I read a version the
  author signed", so the tag must name one. MT into the original language
  is not a translation.
- (b) Any canonical tag. Simpler, but a client bug (for example the UI
  language instead of the shown version) is signed for ever with no
  warning.

The check runs only in `agreement.accept`, where the quote with its
translations is at hand. `AgreementReceiptPayload::validate` checks only
that the tag is canonical (it has `terms`, not the translations). A
receiving node does not check the other party's statement: it is that
party's claim about what it read.

**Q5. Does the receipt card show what each party read? (blocks Step 5)**

- (a) **No, not in B5.** The fields are recorded (ML-18); the
  agreement-receipt card (`ui/src/cards/templates/agreement_receipt.ts`)
  is unchanged. A backlog row records it. *Recommended*: nothing in v1
  makes `viewed_via_mt` true, and showing "read in English" on every
  receipt adds a line that has no use yet. M08C redraws the receipt card
  for its digest-only `terms` anyway (roym-agreements `:875`).
- (b) Yes: one line per half, "Read in <language>" and "(machine
  translation)" when true. About +4 messages and one card test.

**Q6. Does the WIT keep `detect-language`? (blocks Step 2)**

No Roym code calls it in B5: every hook point knows the source language
(a record's `lang`, a message's first tag) or sends none (a message with no
tag, a search query), and `translate` takes `source-lang: option<string>`.

- (a) **Keep it in the WIT and the host (the change doc draft), with no
  `web` method.** *Recommended*: the change doc draft has it; an engine
  needs it to label untagged text; adding it later changes the package
  version. The no-op host answers `unavailable`, and the parity test calls
  it on both builds.
- (b) Drop it from `0.1.0` until a caller exists.

**Q7. Is there an ADR for the interface? (blocks Step 7)**

- (a) **A new ADR-0029, "Machine translation host interface"**, like
  ADR-0027 for `events`: the host owns the engine; a guest cannot choose a
  remote engine; v1 answers `unavailable`; every result names its engine
  and says whether it was remote; no translation is ever signed; a remote
  engine needs the node owner to name it in the substrate config.
  *Recommended*: a new host interface is an architecture decision, and
  M9A builds an engine on these rules.
- (b) No ADR; the change doc's "D. Machine translation" is the record.

### 2.2 Points found while reading

1. **How `events` was added.** M08A's commit `daf112c7` (PR #341) touched,
   for the capability only: `crates/wit_interfaces` (`Cargo.toml`,
   `src/lib.rs`, `src/events.rs`, `src/events_host.rs`,
   `wit/events/events.wit`); `crates/app_host` (`Cargo.toml`, `src/lib.rs`,
   `src/guest.rs`, `src/types.rs`); `crates/app_host_native`
   (`src/convert.rs`, new `src/convert/events.rs`, `src/host.rs`);
   `crates/sandbox_wasm` (`src/engine.rs`, `src/engine/init.rs`,
   `src/host_capabilities.rs`, new `capabilities_events.rs`, new
   `tests/tests_events.rs`); for each of the six Roym crates and
   `test-components/dual-build-fixture`: `Cargo.toml`, `src/guest.rs`,
   `wit/world.wit` and a new symlink `wit/deps/events/events.wit`;
   `crates/roym_core/src/signing.rs` (the test `AppHost` stub). The
   substrate runtime and the factory changed only because `events` has
   state (the event bus); a no-op translation host has none (D-B5-2).
2. **The WIT shape to copy.** `wit/events/events.wit:1` is
   `package syneroym:events@0.1.0;`, `:7` `interface events`, `:62` to
   `:64` `world events-import { import events; }` ("No guest export, so one
   world serves the guest and host views alike"). The host world
   `wit/host/host.wit` (`:14` to `:26`) does not import `events` or
   `signing`, so components that do not import them deploy unchanged
   (`crates/wit_interfaces/src/events_host.rs:1` to `:4`).
3. **Bindings.** `crates/wit_interfaces/Cargo.toml:23` lists `"events"` in
   the default features and `:29` declares `events = []`.
   `src/lib.rs:15` to `:16` (`#[cfg(feature = "events")] pub mod events;`)
   and `:47` to `:48` (`#[cfg(not(target_arch = "wasm32"))] pub mod
   events_host;`). Guest: `src/events.rs:3` to `:7`
   (`wit_bindgen::generate!` over `events-import`). Host:
   `src/events_host.rs:6` to `:11` (`wasmtime::component::bindgen!` with
   `imports: { default: async }`).
4. **The trait and its implementors.** `AppEvents` is at
   `crates/app_host/src/lib.rs:148` to `:161`; `AppHost` lists it at `:51`
   and in the blanket impl at `:67`. Types are re-exported in
   `crates/app_host/src/types.rs:52` to `:56`. Implementors: `GuestHost`
   (`crates/app_host/src/guest.rs:455` to `:472`), `NativeAppHost`
   (`crates/app_host_native/src/host.rs:716` to `:738`), and the test stub
   `TestHost` (`crates/roym_core/src/signing.rs:697`). There is no other
   (`git grep -n 'impl AppEvents for'`).
5. **The WASM linker.** `crates/sandbox_wasm/src/engine/init.rs:397`
   `build_wasm_linker`; `:412`
   `events::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state|
   state)?;`. The `use` of the host module is at `src/engine.rs:46`.
   `HostState` is at `src/host_capabilities.rs:342`; `mod
   capabilities_events;` at `:649`. A component that imports an interface
   the linker lacks fails to instantiate. No code checks a component's
   imports against a manifest (`roym.toml` lists only exported
   `interfaces`, `crates/roym_core/app/roym.toml:12`).
6. **Roym worlds.** `import syneroym:events/events@0.1.0;` is at
   `crates/roym_web/wit/world.wit:35` and `crates/roym_profile/wit/world.wit:35`,
   and at `:19` in `roym_catalog`, `roym_conversation`, `roym_directory`
   and `roym_transaction`; `test-components/dual-build-fixture/wit/world.wit:24`.
   The `with:` remap is at `crates/roym_web/src/guest.rs:46` to `:47` (and
   `src/guest.rs:32` in catalog, directory, profile, transaction; `:39` in
   conversation; `:60` in the fixture). The
   `[package.metadata.component.target.dependencies]` entry is at
   `crates/roym_web/Cargo.toml:74` (`:50` or `:51` in the others; fixture
   `:54`). The wasm32 `syneroym-wit-interfaces` feature list has
   `"events"` at `crates/roym_web/Cargo.toml:26` (and `:26`/`:27` in the
   others; fixture `:29`). `crates/roym_federated_query` has no world and
   was not touched by the `events` commit.
7. **`web` answers one method itself and forwards the rest.**
   `crates/roym_web/src/app.rs:22` `WHOAMI`; `rpc` (`:150` to `:187`)
   answers `session.whoami` before `admit`, then `admit` (`:114` to `:129`)
   reads `router::method_auth`, then `router::route` (`:177`) picks a
   sibling and `forward_rpc_call` (`:217`) calls it. `invoke` (`:267` to
   `:328`) is the internal path: `admit::require_internal` (`:271`) and then
   `admit(host, method, None)`, so an `Owner` method there is always
   `-32010` (no caller). `router::ROUTES`
   (`crates/roym_core/src/router.rs:22` to `:51`) maps prefixes to the five
   siblings only, and the test `every_prefix_maps_to_a_service_in_siblings`
   (`:72`) checks that. `method_auth` (`:62`) returns `None` for an unknown
   prefix, and `admit` treats `None` as `Yes` (`app.rs:116`). The web method
   table is `crates/roym_web/src/api.rs:7` to `:9` (one row); the contract
   test `web_has_one_typed_method_of_its_own`
   (`crates/roym_web/tests/method_contract/contract.rs:87`) asserts one row.
8. **The signed acceptance.** `AgreementReceiptPayload`
   (`crates/roym_core/src/transaction.rs:233` to `:243`) has
   `quote_record_id`, `counterparty_did`, `quote_author_did`, `role`,
   `terms`. No `deny_unknown_fields`, no `serde(default)`. `validate`
   (`:490` to `:507`). `halves_agree` (`:539` to `:548`) compares the quote
   id, both DIDs and `terms`, and needs different roles; the new fields do
   not take part (AG-32). `verify_agreement_receipt` (`:648` to `:704`)
   deserialises with `serde_json::from_value` and validates.
   `AGREEMENT_RECEIPT_VERSION = 1` (`:29`); `RECORD_TYPES` has
   `("agreement-receipt", 1)` (`crates/roym_core/src/record.rs:25`).
   Signing is on the node: the transaction service serialises the payload
   with `serde_json::to_string`, builds a `RecordDraft` and calls
   `AppSigning::sign_record`. Two production sites build the payload:
   `sign_agreement_receipt`
   (`crates/roym_transaction/src/app/agreement_ops.rs:265` to `:325`,
   literal at `:273`), called from `accept` (`:99` to `:163`) at `:130`;
   and `maybe_countersign` (`:402` to `:512`, literal at `:436`), the quote
   author's automatic countersignature during sync, where no person reads
   anything.
9. **Who calls `agreement.accept`.** All pass only `quote_record_id`:
   `roymctl` (`apps/roymctl/src/commands/roym/transaction.rs:95` to `:103`
   and `:486` to `:489`); the Hub (`ui/src/screens/messages.ts:643`);
   parity (`dual_build_parity/deal_fixtures.rs:171`; `transaction.rs:280`,
   `:282`, `:332`, `:614`; `transaction_cards.rs:340`, `:677`;
   `transaction_validation.rs:29`, `:534`); contract
   (`method_contract/shapes_transaction.rs:145`, `:149`, `:154`, `:186`,
   `:210`); substrate e2e (`crates/substrate/tests/common/roym_flow.rs:212`,
   `roym_booking_e2e.rs:188`, `roym_restore_e2e.rs:253`,
   `roym_transaction_e2e.rs:221`). Its input `QuoteRecordInput`
   (`agreement_ops.rs:35` to `:39`, `deny_unknown_fields`) is **shared
   with `agreement.get`** (`crates/roym_transaction/src/api.rs:48` to
   `:49`), so B5 gives `accept` its own input type (D-B5-5).
10. **Signed receipts built by hand in tests.** Literals of
    `AgreementReceiptPayload`: `crates/roym_core/src/transaction/tests.rs:291`,
    `:347`, `:354`; `transaction/tests/lang.rs:362`, `:369`;
    `transaction/tests/verify.rs:42`, `:96`, `:315`;
    `crates/roym_core/src/record/tests/wire_words.rs:342`. Parity fixtures
    that sign a receipt payload as `json!`: `peer_counterparty_receipt`
    (`dual_build_parity/fixtures.rs:328`, payload `:337` to `:343`),
    `peer_counterparty_receipt_with_expiry` (`fixtures.rs:430`, payload
    `:440` to `:446`), `peer_counterparty_receipt_with_identity`
    (`booking.rs:50`, payload `:60` to `:66`). `peer_counterparty_receipt`
    is used in ten files. With required fields, every one of these must
    add `viewed_lang` and `viewed_via_mt` or its receipt fails
    verification (D-B5-5). `fixtures.rs` is 799 lines (one under 800).
11. **Receiving and backup paths re-verify.** `file_agreement_receipt_card`
    (`crates/roym_transaction/src/app/sync.rs:448`; it compares only
    `terms` with the quote at about `:485`) and the backup import
    (`backup.rs:441` `verify_receipt_half`; re-verification at `:605`,
    `:609`, `:638`) both go through `verify_agreement_receipt`. A half
    without the new fields fails there after B5 (section 4, W5).
12. **The Hub cannot see the shown version today.** In `recordText`
    (`ui/src/records/record_text.ts:43` to `:94`) the index on screen is
    the closure variable `currentIdx` (`:57`), changed by the switch button
    (`:86` to `:89`). The accept button is drawn by `renderCardActions`
    (`ui/src/screens/messages.ts:607`), after `renderCard` (`:527`) put the
    quote card into the same `wrap` (`:520` to `:550`). The handler sends
    only `quote_record_id` (`:643`). D-B5-8 puts the shown version on the
    record-text element as `data-` attributes, which the accept handler
    reads from `wrap`.
13. **The binds line exists for author translations only.**
    `record-text-original-binds` (`en.ftl:82`, `hi.ftl:78`) is drawn by
    `ui/src/cards/templates/quote.ts:249` to `:257` when `v.byAuthor`. The
    change doc deviation at `change.md:546` says it "covers [the] gap
    before `viewed_lang` (B5)". B5 keeps the line and also draws it for a
    machine translation (D-B5-7), and rewrites that deviation.
14. **The four hook points in the Hub.**
    - Search query: `buildSearch` (`ui/src/screens/directory.ts:211`), the
      text box `.search-text-input` (`:216` to `:218`) and the Search
      button `.run-search` (`:224`); the query is sent by `findListings`
      (`ui/src/directory/search.ts:100`).
    - Record display: `recordText` is the one place for listing hits
      (`screens/directory.ts:334`), request cards
      (`cards/templates/request.ts:64`) and quote cards
      (`cards/templates/quote.ts:143`). `git grep -n 'recordText('` finds
      these three callers.
    - Chat message: `messageText` (`ui/src/chat/message_text.ts:16`),
      called by `screens/messages.ts:557` and `screens/groups.ts:119`.
    - Agreement review: the quote card with its accept button
      (`screens/messages.ts:624` to `:655`).
15. **No test clicks accept.** `screens/messages.test.ts` (738 lines) never
    calls `agreement.accept`. `roym-hub.spec.ts` tests 30 (`:1073`) and 31
    (`:1125`) check the accept button's presence only. No demo story
    accepts a quote.
16. **The `-32020` code is free.** `git grep -n '\-32020'` finds only an old
    plan (`M06C-roym-product/slice-c3-implementation-plan.md:1626`).
    `-32030` is taken by the control plane signing error
    (`crates/control_plane/src/synsvc_native/signing.rs:26`). The Hub's
    error table is `ui/src/dom.ts:18` to `:28`.
17. **The Hub start-up.** `main()` (`ui/src/main.ts:67`) awaits
    `initLanguage()` (`:70`) and `whoami()` (`:90`), then draws
    `renderTabs` (`:110`) for a signed-in person. `translation.targets`
    needs a signed-in owner (D-B5-4), so it is loaded there.
18. **No translation config exists.** `SubstrateConfig`
    (`crates/core/src/config.rs:58` to `:91`) has no translation key.
    `TransportBridgeRole.translations` (`crates/core/src/config/roles.rs:375`
    to `:386`) is protocol translation and unrelated; B5 does not touch
    it. So ML-17 holds by construction in v1: no engine, no remote engine,
    no config to name one.
19. **The proxy gate.** `NATIVE_CAPABILITY_INTERFACES`
    (`crates/core/src/local_registry.rs:44` to `:53`) lists `signing` but
    not `events`. Translation stays out of it too: it is a host import,
    not a service another node calls.
20. **Two backlog rows point at B5.** `deferred-backlog.md:557` ("A tagged
    chat message shows no language name", target "M08B B5") and `:611`
    ("Matching has no semantic score and no query translation", M9A,
    IM-20). Section 8 says what happens to each.
21. **Capped and near-limit files.** `crates/app_host_native/src/host.rs`
    738 lines; `crates/roym_core/src/signing.rs` 779 (tests inline from
    `:256`); `dual_build_parity/fixtures.rs` 799; `dual_build_parity/transaction.rs`
    846 (capped); `dual_build_parity/helpers.rs` 1290 (capped). The last
    parity scenario number is 235
    (`git grep -ohE 'fn scenario_[0-9]+' crates/roym_web/tests/ | sort -t_ -k2 -n | tail -1`).

## 3. What B5 meets

| Criterion | What B5 does | Test |
| --- | --- | --- |
| ML-15 | `syneroym:translation@0.1.0` exists; every Roym service can call it through `AppTranslation`; `web` calls it. | `sandbox_wasm` `tests_translation.rs`; app-host parity `translation_answers_unavailable_on_both_builds`; roym-web parity `scenario_236_translation_methods_parity` |
| ML-16 | The no-op host answers `unavailable` and `[]`. With it the Hub shows no translate action anywhere. | the same Rust tests; Hub `translation.test.ts` ("no target, no action, no translate call"); each hook point test's v1 case (section 7.1); Playwright "no translate action with a v1 node" |
| ML-17 | No engine and no config exist, so no code path sends text off the node. `engine-info.remote` is in every result for later engines. | `tests_translation.rs` (`no_op_host_makes_no_call`: the no-op answers with no proxy, no network and no state); ADR-0029; section 7.4 says what is not proven |
| ML-18 | Each acceptance half signs `viewed_lang` and `viewed_via_mt`. The Hub sends the shown version; the countersignature records the original. | `roym_core` `transaction/tests/viewed.rs`; parity `scenario_237_accept_records_what_was_viewed_parity`; `method_contract` `agreement_accept_viewed_fields_contract`; Hub `messages.test.ts` accept tests; Playwright "accepting a quote signs the version the person read" |
| ML-19 | Before an acceptance of terms shown through MT, the Hub shows a warning that the original, in its named language, binds, and asks again. | Hub `messages.test.ts` ("accept after a machine translation warns first", stub engine); `templates.test.ts` (binds line on an MT version) |
| ML-26 | The four M08B hook points call the interface: the Hub asks for targets once and each hook point offers the action from that list; a call that answers `unavailable` leaves the original text. Widget answers (M08C), opinions (M08G), offering, want and storefront display (M08D) are later (task.md section 5). | Hub: `translation.test.ts`, `directory.test.ts`, `record_text.test.ts`, `message_text.test.ts`, `messages.test.ts` (one v1 case and one stub-engine case each, plus an `unavailable` case) |
| ML-27 | A translate action appears only at a hook point and only when `supported-targets` is not empty. | the same Hub tests; `hindi-screens.test.ts` (each action and label drawn in Hindi with the stub engine) |

**Where decisions and deviations are written.** At close-out (Step 7):
every D-B5-n and every owner answer goes into a new list "Decided <date>,
translation interface" at the end of the change doc's section "D. Machine
translation", and the agreement fields into "Agreements across languages".
Deviations go under `## Deviations`: the search-query hook runs in the Hub
(Q1, if (a)); `detect-language` has no Roym method (Q6); every Roym
component imports the interface (Q2); the `viewed_*` inputs are optional
(Q3); the deviation at `change.md:546` is rewritten (the binds line now
also covers MT, and `viewed_lang` exists). The test column of ML-15 to
ML-19, ML-26 and ML-27 gets the real test names, each checked with
`git grep` (B3a R2, B3b R3).

**ML-26's later part.** task.md section 5 says to record it in the change
doc's `## Deviations` when the slice is planned. This plan commit adds
that entry (section 9).

## 4. Behaviour-change audit

### Wire changes

| # | Where | Before | After |
| --- | --- | --- | --- |
| W1 | New WIT package `syneroym:translation@0.1.0` | — | `supported-targets`, `detect-language`, `translate` (D-B5-1). Every Roym component imports it (Q2). |
| W2 | `web` JSON-RPC (Hub `/rpc`) | `translation.*` answers `-32601` "Method … not found" | `translation.targets` (no params) → `{"targets": []}`; `translation.translate` → `-32020` `data.kind = "unavailable"` after input checks; both `Owner` (D-B5-3, D-B5-4) |
| W3 | `agreement.accept` input | `{quote_record_id}`; any other field `-32602` | also `viewed_lang?: string`, `viewed_via_mt?: bool` (Q3, Q4); unknown fields still `-32602`. `agreement.get` keeps the old input and still refuses `viewed_lang` |
| W4 | `agreement-receipt` payload (signed) | 5 fields | also `viewed_lang` (canonical tag) and `viewed_via_mt` (bool), both required, outside `terms`. `version` stays 1 (change doc rule 4, D-S0-5) |
| W5 | Verifying an `agreement-receipt` | a payload without the new fields is valid | it is refused (missing field), on receive (`sync.rs:448`), on `agreement.verify` and on backup import |
| W6 | `agreement.verify` output (`RecordVerdict<AgreementReceiptPayload>`) | 5 payload keys | 7 keys; golden shape `method_shapes/agreement_verify_good.json` changes |

**Old and new nodes.** The product is unreleased (change doc rule 4).

- A new node refuses an acceptance half that an old node signed (W5): the
  receipt card shows as refused, and the pair never completes. An old
  node reads a new node's half: `AgreementReceiptPayload` has no
  `deny_unknown_fields`, so it ignores the two fields and the pair
  completes there. The PR text says this (B3b R5).
- A new Roym component imports `syneroym:translation`. A substrate built
  before B5 has no such import in its linker, so it cannot instantiate any
  B5 Roym component. Roym and the substrate are deployed together today;
  the PR text says so.
- An existing node's stored receipts and backups made before B5 fail to
  verify after the upgrade. No migration (rule 4); test nodes, e2e and the
  demo start empty.

### Behaviour changes

| # | Before | After |
| --- | --- | --- |
| 1 | The Hub makes no call after `whoami` before drawing the tabs | it calls `translation.targets` once (D-B5-6). A failure (an older node, `-32601`) counts as no targets; no error is shown |
| 2 | The accept button sends `{quote_record_id}` | it sends the shown version (`viewed_lang`, `viewed_via_mt`) (D-B5-8) |
| 3 | The quote author's countersignature signs 5 fields | it also signs `viewed_lang = terms.lang`, `viewed_via_mt = false` (D-B5-5) |
| 4 | `recordText`, `messageText` and the search box draw no translate action | the same, with the v1 node. With an engine: the action and a labelled MT (D-B5-7) |

### Stays the same

- Every other Roym record and its signature; `halves_agree`; the pair
  state; the booking and payment flows.
- `roymctl roym transaction accept` (Q3 (a)); every substrate e2e test.
- Every Hub screen as drawn with the v1 node; the English and Hindi text
  of every existing message; story 40.
- `directory.find`, `directory.search`, `conversation.search` and their
  inputs.
- The manifest `roym.toml`, `SubstrateConfig`, the native host factory,
  the runtime wiring and `NATIVE_CAPABILITY_INTERFACES`.

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3). The ones
that depend on an owner answer say so.

- **D-B5-1. The WIT.** New file
  `crates/wit_interfaces/wit/translation/translation.wit`, the change doc
  draft with fixed names:

  ```wit
  package syneroym:translation@0.1.0;

  /// Machine translation for display. The host owns the engine; a guest
  /// cannot choose one. A result is never input to signing.
  interface translation {
      variant translation-error {
          /// No engine on this node.
          unavailable,
          /// The engine cannot translate from the first tag to the second.
          unsupported-pair(tuple<string, string>),
          /// The text is longer than the engine accepts; the limit in bytes.
          too-long(u32),
          engine-failed(string),
      }

      record engine-info {
          /// Shown to the reader next to every translation.
          name: string,
          /// True when the text left this node.
          remote: bool,
      }

      record translated {
          text: string,
          /// The source language the engine used, stated or detected.
          source-lang: string,
          engine: engine-info,
      }

      record detected {
          lang: string,
          /// 0..=1000.
          confidence: u16,
      }

      /// Empty when no engine is configured.
      supported-targets: func() -> list<string>;
      detect-language: func(text: string) -> result<list<detected>, translation-error>;
      translate: func(text: string, source-lang: option<string>, target-lang: string)
          -> result<translated, translation-error>;
  }

  /// No guest export, so one world serves the guest and host views alike.
  world translation-import {
      import translation;
  }
  ```

  `detect-language` stays if Q6 (a). The comments carry no planning id.

- **D-B5-2. Wiring (Q2 (a)).** The `events` pattern of section 2.2 point 1,
  minus the state:
  - `syneroym-wit-interfaces`: feature `translation = []` and in the
    default list; `src/translation.rs` (guest `generate!` over
    `translation-import`, with `additional_derives: [Serialize,
    Deserialize, PartialEq, Eq]` like `events.rs`); `src/translation_host.rs`
    (host `bindgen!`, `imports: { default: async }`); two `mod` lines in
    `src/lib.rs`.
  - `syneroym-app-host`: the feature in `Cargo.toml`; `pub mod
    translation` in `types.rs` re-exporting `TranslationError`,
    `EngineInfo`, `Translated`, `Detected`; the trait

    ```rust
    /// Mirrors syneroym:translation/translation@0.1.0.
    pub trait AppTranslation {
        fn supported_targets(&self) -> impl Future<Output = Vec<String>> + Send;
        fn detect_language(&self, text: String)
            -> impl Future<Output = Result<Vec<Detected>, TranslationError>> + Send;
        fn translate(&self, text: String, source_lang: Option<String>, target_lang: String)
            -> impl Future<Output = Result<Translated, TranslationError>> + Send;
    }
    ```

    added to the `AppHost` supertrait and its blanket impl
    (`lib.rs:51`, `:67`); `impl AppTranslation for GuestHost` in
    `guest.rs`, calling the import as `AppEvents` does (`:455` to `:472`).
  - `sandbox_wasm`: new `src/host_capabilities/capabilities_translation.rs`
    with three associated functions on `HostState` that take no `self`
    (`translation_targets() -> Vec<String>` returns `vec![]`;
    `detect_language(_text) -> Result<_, TranslationError>` and
    `translate(..)` return `Err(TranslationError::Unavailable)`) and
    `impl translation::Host for HostState` that calls them; `mod
    capabilities_translation;` next to `host_capabilities.rs:649`; the
    `use` in `engine.rs` (the `:36` to `:59` block) and
    `translation::add_to_linker::<_, HasSelf<HostState>>(&mut linker,
    |state| state)?;` after `init.rs:412`. No `HostState` field, no engine
    field, no factory or runtime change.
  - `app_host_native`: `impl AppTranslation for NativeAppHost` in a new
    sibling file `src/host/translation.rs` (declared from `host.rs`;
    `host.rs` is at 738 lines), calling the same `HostState` functions
    with no lock (they read no state), and a new `src/convert/translation.rs`
    for the error and record types (registered in `convert.rs:75` to
    `:87`, like `convert/events.rs`). One no-op, two callers: the two
    builds cannot drift.
  - `crates/roym_core/src/signing.rs`: `impl AppTranslation for TestHost`
    (the no-op answers). The file is 779 lines with tests from `:256`;
    the production part does not grow.
  - Each of `roym_web`, `roym_profile`, `roym_catalog`,
    `roym_conversation`, `roym_directory`, `roym_transaction` and
    `test-components/dual-build-fixture`: `import
    syneroym:translation/translation@0.1.0;` in `wit/world.wit`; a symlink
    `wit/deps/translation/translation.wit ->
    ../../../../wit_interfaces/wit/translation/translation.wit` (mode
    `120000`, B4a P2); `"translation"` in the wasm32
    `syneroym-wit-interfaces` features; the
    `[package.metadata.component.target.dependencies]` entry; the `with:`
    remap `"syneroym:translation/translation@0.1.0":
    syneroym_wit_interfaces::translation::syneroym::translation::translation`.
  - Not touched: `wit/host/host.wit` (point 2), `roym.toml`,
    `SubstrateConfig`, `NativeHostFactory`, `crates/substrate/src/runtime/`,
    `NATIVE_CAPABILITY_INTERFACES`.

- **D-B5-3. Two `web` methods, answered by `web` itself.**
  New module `crates/roym_web/src/app/translation.rs` (declared from
  `app.rs`). Types:

  ```rust
  #[derive(Debug, Serialize, Deserialize)]
  pub struct TargetsOutput { pub targets: Vec<String> }

  #[derive(Debug, Deserialize, Serialize)]
  #[serde(deny_unknown_fields)]
  pub struct TranslateInput {
      pub texts: Vec<String>,
      #[serde(default)]
      pub source_lang: Option<String>,
      pub target_lang: String,
  }

  #[derive(Debug, Serialize, Deserialize)]
  pub struct TranslateOutput { pub items: Vec<TranslatedItem> }

  #[derive(Debug, Serialize, Deserialize)]
  pub struct TranslatedItem {
      pub text: String,
      pub source_lang: String,
      pub engine: EngineView, // { name: String, remote: bool }
  }
  ```

  - `translation.targets` takes `NoParams` and returns
    `{targets: AppTranslation::supported_targets(host)}`. An object, not a
    bare array (backlog §10 "Three list methods return a bare JSON
    array").
  - `translation.translate` checks, in this order, each failure `-32602`
    through `ParamError { field: Some(..) }.into_response()` (B3b P6):
    `texts` has 1 to `MAX_TRANSLATE_TEXTS = 8` entries (`field: "texts"`);
    each entry is at most `MAX_TRANSLATE_TEXT_BYTES = 8_192` bytes and the
    sum at most `MAX_TRANSLATE_TOTAL_BYTES = 32_768` (`field: "texts"`);
    `target_lang` is a canonical tag (`syneroym_text_lang::LangTag::parse_canonical`,
    `field: "target_lang"`); `source_lang`, when present, is a canonical
    tag (`field: "source_lang"`). Then it calls
    `AppTranslation::translate` once per text, in order, and stops at the
    first error. The limits cover the largest Hub call: a quote's four
    term fields (`scope` at most `MAX_SCOPE_LEN = 4096` bytes and the three
    term texts at most `MAX_TERMS_TEXT_LEN = 2048` each,
    `transaction.rs:58` to `:59`; 10,240 bytes in all) and a listing's
    title and summary (`MAX_TITLE_LEN = 160`, `MAX_SUMMARY_LEN = 2048`,
    `listing.rs:21` to `:22`).
  - A `TranslationError` becomes `-32020` with message
    "translation unavailable" / "translation pair unsupported" /
    "text too long for the engine" / "translation engine failed", and
    `data` `{"kind": "unavailable"}`, `{"kind": "unsupported-pair",
    "source": .., "target": ..}`, `{"kind": "too-long", "limit": ..}`,
    `{"kind": "engine-failed"}` (the engine's own text goes into the
    message after a colon, never into `data`) through
    `Response::with_data` (`crates/roym_core/src/envelope.rs:50`).
    `-32020` is free (section 2.2 point 16). It is a constant
    `TRANSLATION_ERROR: i64 = -32020` in the new module.
  - `api.rs::method_table` gets three rows: `session.whoami`,
    `translation.targets` (`NoParams => TargetsOutput`),
    `translation.translate` (`TranslateInput => TranslateOutput`).
  - Both methods run only on the HTTP `/rpc` path. On the internal
    `invoke` path, `admit(host, method, None)` answers `-32010` for an
    `Owner` method (section 2.2 point 7), as today for every routed
    method; nothing else changes there.

- **D-B5-4. Routing and auth.** `crates/roym_core/src/router.rs` gets

  ```rust
  /// Methods `web` answers itself, by prefix. Not routed to a sibling.
  const WEB_ROUTES: &[(&str, MethodAuth)] = &[("translation.", MethodAuth::Owner)];

  pub fn answered_by_web(method: &str) -> bool { .. }
  ```

  `method_auth` reads `PUBLIC_METHODS`, then `ROUTES`, then `WEB_ROUTES`.
  `route` does not change (it still answers `None` for `translation.*`).
  In `web`'s `rpc` (`app.rs:150`), after `admit` and before
  `router::route`: `if router::answered_by_web(&method) { return
  Ok(json_http_response(&translation::dispatch(host, &method,
  rpc_req.params, id).await)); }`, where `dispatch` answers an unknown
  `translation.*` name with `-32601` as `route` would. New router tests:
  `web_routes_do_not_overlap_sibling_routes` (no prefix of one table is a
  prefix of the other) and `translation_methods_need_the_owner`.
  `translation.*` needs the owner because a later engine may be remote or
  cost money (ADR-0029); an anonymous Hub page must not reach it.

- **D-B5-5. The acceptance fields (Q3 (a), Q4 (a)).**
  - `AgreementReceiptPayload` gains, after `terms`:

    ```rust
    /// The language of the text this party read before it signed.
    pub viewed_lang: LangTag,
    /// True when that text was a machine translation.
    pub viewed_via_mt: bool,
    ```

    Both required (no `serde(default)`), as AG-32 says. `LangTag`
    deserialises only a canonical tag (B3a, `syneroym-text-lang`), so
    `verify_agreement_receipt` refuses a non-canonical one with no new
    code. `validate` adds one rule: `viewed_via_mt` with `viewed_lang ==
    terms.lang` is `TransactionError::ViewedMtOfOriginal` (new variant in
    `transaction/error.rs`). `halves_agree` does not change.
  - `agreement.accept` gets its own input (`agreement.get` keeps
    `QuoteRecordInput`):

    ```rust
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct AgreementAcceptInput {
        pub quote_record_id: String,
        #[serde(default)]
        pub viewed_lang: Option<String>,
        #[serde(default)]
        pub viewed_via_mt: Option<bool>,
    }
    ```

    `api.rs:48` changes to `[agreement_ops::AgreementAcceptInput => ..]`.
    In `accept`, after `load_verified_quote_and_role` and before
    `already_accepted_response`: a new `fn viewed(input, &quote_payload)
    -> Result<(LangTag, bool), Response>`. Absent `viewed_lang` → the
    quote's `terms.lang`; absent `viewed_via_mt` → `false`. A
    non-canonical tag → `-32602` `field: "viewed_lang"` (naming the
    canonical form, as B3a does). Then the Q4 (a) rule: not MT → the tag
    equals `terms.lang` or a `translations[i].lang`, else `-32602`
    `field: "viewed_lang"` "viewed_lang names no version of this quote";
    MT → the tag differs from `terms.lang`, else `-32602` `field:
    "viewed_via_mt"`. An already-accepted half answers as today (the
    check runs first, so a bad repeat is still refused; this is a new
    refusal on a repeat call and is in the PR text).
  - `sign_agreement_receipt` takes the pair and puts it in the payload.
    `maybe_countersign` signs `viewed_lang: q_payload.terms.lang.clone()`,
    `viewed_via_mt: false`: the quote author wrote the original.
  - The quote author can also call `agreement.accept` (role
    `QuoteAuthor`, `accept` `:124`); the same rules apply.

- **D-B5-6. The Hub `translation` module** (`ui/src/translation/translation.ts`,
  new):
  - `loadTargets(): Promise<void>` calls `translation.targets` once and
    keeps the list in the module; any error keeps `[]` and logs nothing to
    the person. `main.ts` awaits it after `whoami` and before `renderTabs`
    (`main.ts:110`), only for a signed-in person.
  - `targetFor(shownLang: string): string | null` → the first of
    `readerLanguages()` (`records/record_text.ts:11`) that matches a target
    with `sameLanguage` (`i18n/text_lang.ts:10`), and that does not match
    `shownLang`; else `null`. `shownLang` may be `""` (unknown): then any
    reader target. No target → no action (ML-27).
  - `searchTargets(): string[]` → the target list, for the search box.
  - `translateTexts(texts, sourceLang, targetLang): Promise<MachineText>`
    calls `translation.translate` and returns `{ texts, sourceLang,
    engine: { name, remote } }` from the first item's engine. It throws an
    `RpcError` as `call` does.
  - `machineNote(targetLang, engine): string` → `translation-machine-note`
    or `translation-machine-note-remote` (`remote: true`).
  - `setTargetsForTest(list)` for unit tests (the module keeps no other
    state).
  - The error table in `dom.ts` (`:18` to `:28`) gets
    `[-32020, messageId("error-translation")]`. A translate click that
    fails shows `translation-failed` with `errText` as `$detail` under the
    text, and the shown text stays the original (ML-26).

- **D-B5-7. `recordText` and `messageText`.**
  - `TextVersion<F>` gains `machine?: { name: string; remote: boolean }`.
    `recordText` gains an optional fourth parameter
    `texts?: { get: (f: F) => string[]; set: (f: F, texts: string[]) => F }`
    that names the translatable strings of `F` in a fixed order. The three
    callers pass it: listing `[title, summary]`, request `[description]`,
    quote `[scope, cancellation_terms, refund_terms, dispute_path]`
    (empty strings are sent as they are and come back as they are; an
    absent field is `""`).
  - When `targetFor(shown.lang)` gives a target, the meta box gets a
    button `.record-text-translate` (`translation-translate-button`,
    `$language` = the target's name, `type="button"`). With one version
    and no target, the output is exactly as today (no meta box; B3a
    deviation "ML-12").
  - Click: `translateTexts(texts.get(shown.fields), shown.lang || null,
    target)`; on success a new version `{ lang: target, fields:
    texts.set(shown.fields, out.texts), byAuthor: false, machine:
    out.engine }` is appended and shown, the note is `machineNote(..)`, and
    the switch button reads `record-text-show-original`; on failure
    `translation-failed`, and nothing else changes. A second click does
    not call again (the MT version is kept).
  - `recordText` writes the shown version on its host element after each
    draw: `data-shown-lang` (the version's `lang`) and `data-shown-via`
    (`original`, `author` or `machine`). Only these two attributes; no
    module state.
  - The quote binds line (`quote.ts:249`) is drawn when `v.byAuthor ||
    v.machine`.
  - `messageText(body, languages, tag, opts?)`: for a text message (the
    two callers pass `{ translatable: true }` for a non-deleted UTF-8
    message), when `targetFor(messageLang(languages))` gives a target, a
    button `.message-translate` follows the body; a click puts the MT in a
    `<span class="message-translation" lang="<target>">` and a note
    `.message-translation-note` (`machineNote`). The body itself never
    changes. Cards, deleted and binary messages get no button.
- **D-B5-8. The accept button.** In `renderCardActions` the handler reads
  `const shown = wrap.querySelector<HTMLElement>("[data-shown-lang]")!`
  (the quote card always has one after D-B5-7; the `!` is asserted in a
  test, no `?.`, B3a R4) and sends `{ quote_record_id, viewed_lang:
  shown.dataset.shownLang, viewed_via_mt: shown.dataset.shownVia ===
  "machine" }`.
- **D-B5-9. The warning (ML-19).** When `shownVia === "machine"`, the
  first click on Accept does not call the node. It draws
  `.accept-mt-warning` (`messages-accept-mt-warning`: "You read a machine
  translation. The original, in { $language }, is what binds.") and a
  confirm button `.accept-mt-confirm` (`messages-accept-mt-confirm`,
  "Accept the original terms") plus the existing cancel words
  (`common-cancel`). Confirm calls `agreement.accept` as D-B5-8. With an
  author translation or the original, Accept works as today (the in-card
  binds line already says it for an author translation).
- **D-B5-10. The search box (Q1 (a)).** When `searchTargets()` is not
  empty, `buildSearch` adds after the text box a `<select
  class="search-translate-target">` (one option per target, by
  `languageName`) and a button `.search-translate`
  (`directory-search-translate-button`). A click translates the box text
  (source unknown: `null`), puts the result in the box, and shows
  `.search-translate-note` (`machineNote`). Search then runs as today on
  the box text. An empty box does not call. With v1 none of this is
  drawn.
- **D-B5-11. New messages** (English; Hindi written by the agent, owner
  review before merge, section 7.7): `translation-translate-button`
  ("Translate into { $language }"), `translation-machine-note` ("Machine
  translation into { $language } by { $engine }."),
  `translation-machine-note-remote` ("Machine translation into
  { $language } by { $engine }, sent off this device."),
  `translation-failed` ("Could not translate. { $detail }"),
  `error-translation` ("Translation is not available. { $detail }"),
  `messages-accept-mt-warning`, `messages-accept-mt-confirm`,
  `directory-search-translate-button` ("Translate search"),
  `directory-search-translate-target` (the select's `aria-label`,
  "Language to search in"). Nine messages, each with its own id
  (change doc "Shared messages"). `$engine` is a string argument.

## 6. Step-by-step changes

Each step builds and its tests pass. Rust: `cargo nextest run -p <crate>`
for the crates the step touches, output to a file (AGENTS.md "Context
Budget"). Paths are from the repository root.

### Step 1 — Commit this plan

Done in the planning session: this file, the B5 row of
[status.md](status.md), and the ML-26 entry in the change doc's
`## Deviations` (section 3).

### Step 2 — The interface and the no-op host, both builds

Split as B3b P9 asks: the host first, then the guests.

1. WIT file (D-B5-1); `syneroym-wit-interfaces` feature, guest and host
   modules (D-B5-2).
2. `sandbox_wasm`: `capabilities_translation.rs`, the `mod` line, the
   linker line; `tests/tests_translation.rs` (section 7.2). Build and run
   `cargo nextest run -p syneroym-sandbox-wasm translation`. No component
   imports it yet, so nothing else changes.
3. `syneroym-app-host`: `types.rs`, the trait, `AppHost`, `GuestHost`.
   `app_host_native`: `host/translation.rs`, `convert/translation.rs`.
   `roym_core/src/signing.rs`: `TestHost`.
4. The seven worlds, symlinks, `Cargo.toml` entries and `with:` remaps
   (D-B5-2). After this, `git ls-files -s 'crates/*/wit/deps/translation/*'
   'test-components/*/wit/deps/translation/*'` shows seven lines with mode
   `120000`.
5. The fixture verbs `TranslationTargets`, `DetectLanguage { text }`,
   `Translate { text, source_lang, target_lang }` in
   `test-components/dual-build-fixture/src/app.rs` (next to the signing
   verbs, `:242` to `:255`) and `src/app/dispatch.rs` (next to `:498` to
   `:516`); the parity test file (section 7.2).
6. `mise run build:test-components` and `mise run build:roym`, then
   `cargo nextest run -p syneroym-app-host-native --test dual_build_parity`,
   `cargo nextest run -p syneroym-roym-web --test dual_build_parity
   --test method_contract` (every Roym scenario still passes with the new
   import), and `cargo nextest run -p syneroym-substrate --test
   roym_conversation_e2e` (a full Roym deploy instantiates the new
   components).
7. Commit (`feat(translation): host interface with no engine`).

### Step 3 — The `web` methods

1. `router.rs`: `WEB_ROUTES`, `answered_by_web`, `method_auth` (D-B5-4) and
   the two router tests.
2. `roym_web/src/app/translation.rs` and the branch in `rpc` (D-B5-3);
   `api.rs` rows.
3. `method_contract/contract.rs`: `web_has_one_typed_method_of_its_own`
   becomes `web_answers_three_typed_methods_of_its_own` (three rows; each
   probe refuses an unknown field; `route` is `None` for all three); the
   contract test of section 7.2. Golden shapes: `translation_targets.json`
   (new, through `UPDATE_METHOD_SHAPES=1`, then read the file).
4. New parity file `crates/roym_web/tests/dual_build_parity/translation.rs`
   (declared in `dual_build_parity.rs`), scenario 236.
5. `mise run build:roym`; `cargo nextest run -p syneroym-roym-core -p
   syneroym-roym-web`.
6. Commit (`feat(roym-web): translation methods`).

### Step 4 — The acceptance fields

1. `roym_core`: the two fields, the `validate` rule and the error variant
   (D-B5-5); every payload literal of section 2.2 point 10 adds the two
   fields; new tests `transaction/tests/viewed.rs` (declared in
   `transaction/tests.rs`).
2. `roym_transaction`: `AgreementAcceptInput`, `viewed`, the two signing
   sites; `api.rs:48`.
3. The three parity fixture payloads (`fixtures.rs:337`, `:440`;
   `booking.rs:60`) add `"viewed_lang": "en", "viewed_via_mt": false` (the
   fixture terms are `lang: "en"`; check `fixtures.rs` before writing).
   `fixtures.rs` is 799 lines: put the two keys on the existing lines, or
   move one helper to `deal_fixtures.rs`, so it stays at most 800.
4. New parity file `dual_build_parity/agreement_lang.rs`, scenario 237;
   the contract test; `UPDATE_METHOD_SHAPES=1` for
   `agreement_verify_good.json` (read the diff: exactly two new keys).
5. `mise run build:roym`; `cargo nextest run -p syneroym-roym-core -p
   syneroym-roym-transaction`; `cargo nextest run -p syneroym-roym-web
   --test dual_build_parity --test method_contract`; `cargo nextest run -p
   syneroym-substrate --test roym_transaction_e2e --test roym_booking_e2e
   --test roym_restore_e2e` (they use the defaults of Q3 (a)).
6. Commit (`feat(transaction): record what each party viewed`).

### Step 5 — Hub

1. `src/translation/translation.ts` and its test (D-B5-6); `main.ts`; the
   `dom.ts` error row.
2. `records/record_text.ts` and the three callers (D-B5-7); `quote.ts`
   binds condition.
3. `chat/message_text.ts` and its two callers.
4. `screens/messages.ts` accept handler and warning (D-B5-8, D-B5-9).
5. `screens/directory.ts` search box (D-B5-10).
6. `en.ftl`, `hi.ftl` (D-B5-11); the Hindi sweep cases (section 7.1).
7. `npm test` and `npm run build` in `crates/roym_web/ui`. Then
   `git grep -n -E "\.first\(\)|nth\(" crates/substrate/tests/e2e` and
   check that no locator now hits a new control first (B3a P4, B4b P3);
   the new controls are not drawn with v1, so none should.
8. Commit (`feat(roym-web): translation hook points in the hub`).

### Step 6 — Playwright

1. `roym-hub.spec.ts`: the two tests of section 7.3.
2. `mise run test:e2e` (redirected, AGENTS.md) and `mise run demo` (story
   40 unchanged, still passes).
3. Commit (`test(e2e): accept records the viewed language`).

### Step 7 — Sizes, docs and close-out

1. Component size (section 7.6) and the Hub size (section 7.5). Write them
   into task.md as a B5 result line, in the form of the B4b line.
2. ADR-0029 (Q7 (a)).
3. Change doc: decisions, deviations and test columns (section 3). `demo:`
   stays story 40; `status` stays `draft` (B6 closes it); add any file of
   section 9 that is not in `living-docs-touched`.
4. Living docs (section 9), deferred backlog (section 8), status.md.
5. The import clean-up pass over every Rust file you edited.
6. `cargo dupes report` over the changed Rust files (AGENTS.md recipe):
   one no-op, used by both builds; one tag check per field.
7. `mise run verify` with no `--skip`, as a background job.
8. PR text: section 10's checklist.

## 7. Tests

### 7.1 Hub unit tests

Each hook point has three cases: v1 (`translation.targets` → `[]`: no
action drawn, and the stub records no `translation.translate` call), stub
engine (targets `["en"]` or `["hi"]`: the action is drawn; a click sends
the exact params; the answer is drawn with its engine label), and
`unavailable` (stub engine, but `translation.translate` answers `-32020`
`kind: "unavailable"`: the original stays, `translation-failed` is
drawn). Every element is found with a non-null assertion; no `?.` and no
`if (el)` (B2 R3, B3a R4, B3b R9).

- `src/translation/translation.test.ts`: `loadTargets` keeps the list;
  a `-32601` answer and a network failure both give `[]`;
  `targetFor` (shown `hi`, reader `["en"]`, targets `["en","hi"]` → `en`;
  shown `en` → `null`; shown `""` → `en`; `iw`/`he` through
  `sameLanguage`; a target `constructor` is not inherited, B1a P5);
  `translateTexts` sends `{texts, source_lang, target_lang}` and omits a
  `null` source; `machineNote` for `remote: false` and `true`.
- `records/record_text.test.ts`: the three cases for a one-version record
  and for a record with an author translation; after a stub translation,
  `data-shown-lang` is the target and `data-shown-via` is `machine`;
  after "Show the original", `original`; with an author translation shown,
  `author`; a second click on Translate makes no second call.
- `cards/templates/templates.test.ts`: the quote binds line on an MT
  version; listing hit and request card pass their text fields (assert the
  exact `texts` array sent).
- `chat/message_text.test.ts`: the three cases; a message tagged in the
  reader's language gets no button; an untagged message sends no
  `source_lang`; cards, deleted and binary rows get no button (through the
  callers' tests in `messages.test.ts` and `groups.test.ts`).
- `screens/messages.test.ts`: Accept on the original sends `{
  quote_record_id, viewed_lang: "<terms.lang>", viewed_via_mt: false }`;
  after "Show the author's translation", the translation's tag and
  `false`; after a stub MT, the first click draws `.accept-mt-warning`
  with the original's language name and makes no call; confirm sends the
  target and `true`; cancel removes the warning and makes no call.
- `screens/directory.test.ts`: the three cases for the search box; the
  translated text lands in `.search-text-input` and the next Search sends
  it as `query.text`; an empty box makes no call.
- `i18n/hindi-screens.test.ts`: with the stub engine, the record note, the
  message note, the warning, the confirm button, the search select and
  button are drawn in Hindi with no Latin word left (B3a R4: every new
  Hindi string is drawn by the sweep). The engine name is data and carries
  the `Zq` marker.
- `catalog.test.ts` and the visible-text check run unchanged and must pass
  with the nine new ids.

### 7.2 Rust

**Host** (`crates/sandbox_wasm/src/host_capabilities/tests/tests_translation.rs`,
new; declared next to `tests_events` in `tests.rs:40`)

- `targets_are_empty`, `translate_is_unavailable`,
  `detect_is_unavailable` (through the `Host` trait, on a `HostState` built
  as `tests_events.rs` builds one).
- `no_op_host_makes_no_call`: the three functions answer on a `HostState`
  with no proxy, no event bus and no network (ML-17's part that a test
  can show).

**Both builds of the host** (`crates/app_host_native/tests/dual_build_parity/translation.rs`,
new; registered in `dual_build_parity.rs` next to `:25` to `:26`)

- `translation_answers_unavailable_on_both_builds`: the three fixture
  verbs give byte-identical answers on both builds: `[]`,
  `Err(unavailable)`, `Err(unavailable)`.

**Router** (`crates/roym_core/src/router.rs` tests)

- `web_routes_do_not_overlap_sibling_routes`, `translation_methods_need_the_owner`
  (`method_auth("translation.targets") == Some(Owner)`;
  `route("translation.targets") == None`; `answered_by_web` true).

**`web`** (new `crates/roym_web/tests/dual_build_parity/translation.rs`)

- `scenario_236_translation_methods_parity`, through the HTTP `/rpc` path
  with an owner session (the harness that `whoami_shapes` uses,
  `method_contract/shapes_directory.rs:334`): `translation.targets` →
  `{"targets": []}`; `translate` with `{texts: ["नल वाला"], target_lang:
  "en"}` → `-32020`, `data.kind = "unavailable"`; `target_lang: "EN"` →
  `-32602` `data.field = "target_lang"`; 9 texts → `-32602` `texts`; one
  text of 8,193 bytes → `-32602`; `{}` extra field → `-32602`; no session
  → `-32010`. `assert_eq!` of the whole answers of the two builds (B3b R7).
- `method_contract/contract.rs`: `web_answers_three_typed_methods_of_its_own`.

**Acceptance** (`crates/roym_core/src/transaction/tests/viewed.rs`, new)

- `receipt_requires_viewed_fields`: a payload without `viewed_lang`, and
  one without `viewed_via_mt`, fail to deserialise.
- `receipt_refuses_non_canonical_viewed_lang` (`"HI-latn"`, `"hi_IN"`).
- `receipt_refuses_mt_of_the_original` (`ViewedMtOfOriginal`).
- `halves_differing_in_viewed_fields_still_agree` (AG-32: one half `hi`
  and `false`, the other `en` and `true`; `halves_agree` is true).
- `verify_round_trips_viewed_fields`: sign, verify, read both fields back.

**Acceptance through the service** (new
`crates/roym_web/tests/dual_build_parity/agreement_lang.rs`)

- `scenario_237_accept_records_what_was_viewed_parity`: a quote with
  `terms.lang: "hi"` and one author translation `en` (build it with the
  quote fixtures of `deal_fixtures.rs`; add a translation to the terms of
  that one quote, not to the shared fixture, B4b P7). Cases, each
  compared on both builds with `assert_eq!` of the whole answer:
  - no viewed fields → the counterparty half's payload has `viewed_lang:
    "hi"`, `viewed_via_mt: false`;
  - `viewed_lang: "en"` → `"en"`, `false`;
  - `viewed_lang: "ta"`, not MT → `-32602` `data.field = "viewed_lang"`;
  - `viewed_lang: "hi"`, `viewed_via_mt: true` → `-32602` `data.field =
    "viewed_via_mt"`;
  - `viewed_lang: "ta"`, `viewed_via_mt: true` → accepted;
  - `viewed_lang: "HI"` → `-32602` naming `hi`.
  Each accepted case uses a fresh quote. Then the author's node syncs and
  countersigns: its half has `viewed_lang: "hi"`, `viewed_via_mt: false`,
  and the pair is complete although the two halves differ.
- `method_contract` (`shapes_transaction.rs`, 443 lines):
  `agreement_accept_viewed_fields_contract`: `agreement.accept` takes the
  two fields and still refuses an unknown one with `data.field`;
  `agreement.get` refuses `viewed_lang` (its input did not change).
- `agreement_verify_good.json` has the two new keys.
- Existing tests keep passing unchanged: every caller of section 2.2 point
  9 (the defaults of Q3 (a)).

### 7.3 Playwright

- `roym-hub.spec.ts`, new test "no translate action with a v1 node": with
  a listing hit, a quote card and a chat message on screen,
  `.record-text-translate`, `.message-translate`, `.search-translate` and
  `.search-translate-target` each have count 0, and
  `page.evaluate` of `translation.targets` through `/rpc` answers
  `{"targets": []}`.
- New test "accepting a quote signs the version the person read": Alice
  sends Bob a quote in `hi` with an English author translation; Bob's
  Hub (English) shows the English text; Bob presses Accept; then
  `agreement.get` on Bob's node gives the counterparty half, and its
  envelope payload has `viewed_lang: "en"` and `viewed_via_mt: false`.
  Reuse the quote set-up of tests 30 and 31 (`:1073`, `:1125`); move the
  shared part into `hub-helpers.ts` if both tests need it. Check before
  the click that the note `record-text-by-author` is on screen (B3b R7).
- Story 40 runs unchanged.

### 7.4 Not tested

- A real engine. Only the Hub stub and the no-op host exist. M9A adds an
  engine and its tests.
- ML-17 for a later remote engine: B5 shows only that v1 has no engine and
  no config. The rule is in ADR-0029.
- The ML-19 warning in a real browser with a real MT: the warning is
  tested in jsdom with the stub engine only (task.md B5).

### 7.5 Size risks

| File or function | Now | Risk |
| --- | --- | --- |
| `crates/app_host_native/src/host.rs` | 738 lines | not grown: the impl goes into `host/translation.rs` |
| `crates/roym_core/src/signing.rs` | 779 lines (tests from `:256`) | `TestHost` +15 lines in the test part |
| `crates/roym_core/src/transaction.rs` | 707 lines | +10 (fields, rule); stays under 800 |
| `crates/roym_transaction/src/app/agreement_ops.rs` | 512 lines | `viewed` +35, input +10 |
| `accept` (`agreement_ops.rs:99` to `:163`) | 65 lines | +4 (one call); under 100 |
| `crates/roym_web/src/app.rs` | 342 lines | +6; the methods are in `app/translation.rs` |
| `dual_build_parity/fixtures.rs` | 799 lines | must stay at most 800 (Step 4.3) |
| `dual_build_parity/transaction.rs`, `helpers.rs` | capped | not touched (new files) |
| `ui/src/screens/messages.ts` | 957 lines (TypeScript is not under the file-length check) | +40 |
| Hub main script (`gzip -9`) | 53,216 bytes after B4b | expect under +2 KB; limit +5 KB (the B3b limit) |
| Hindi catalog (`gzip -9`) | 14,548 bytes after B4b | nine messages: expect about +600 bytes; limit +1 KB |

### 7.6 Component size

Build the six Roym components (`mise run build:roym`) on `main` and on the
branch, in the same build folder, and write before and after into task.md.
Limits (task.md section 1): `transaction` gets new record code (the two
fields and the `viewed` check), so +15%; the other five get only the
import glue (and `web` its two methods, no record code), so +40 KB each.
Expected: a few KB each. If one grows by more than its limit, stop and
tell the owner.

### 7.7 Hindi text

Nine messages (D-B5-11). `messages-accept-mt-warning` and
`messages-accept-mt-confirm` carry legal weight (what binds); mark them in
the PR text for the owner, as B1b did. The Hindi sweep and the catalog
test cover all nine.

### 7.8 How B5 avoids the kinds of findings S0 to B4b had

- **Wrong lines and counts** (B3a P13, B4a P14): every line above was read
  on `6083ddb3`; each list of call sites has its `git grep` (`impl
  AppEvents for`, `AgreementReceiptPayload {`, `"agreement.accept"`,
  `recordText(`, `messageText(`).
- **Missed callers** (B3b P3, B4a P2): the seven worlds and their four
  edits each; the three hand-signed parity receipts and the nine core
  literals; the shared `QuoteRecordInput`; `TestHost`.
- **Vendored WIT copies** (B4a P2): symlinks with mode `120000`, checked
  in Step 2.4.
- **Audit gaps** (B1a R1, B4a P9, B3b R5): section 4 lists the new
  refusals (W5), the old-node behaviour and the substrate/component pair.
- **Signed text** (B3a P1, B1b Q2): the Hub signs what was on screen,
  read from the element that drew it, and warns before MT.
- **Steps that do not build** (B3b P9, B1b P4): Step 2 does the host
  before the guests; Step 4 changes every receipt literal in the step that
  makes the fields required.
- **Tests that prove less than their name** (B3b R7, B4a R4, B3a R5): each
  parity test compares whole answers of both builds; the translation case
  uses a quote with a real translation; each hook test asserts that v1
  makes no `translate` call, not only that no button is drawn.
- **`?.` in tests** (B2 R3, B3a R4, B3b R9): every element is asserted
  with `!`.
- **Inherited names** (B1a P5): `targetFor` with `constructor`.
- **Change doc test column** (B3a R2, B3b R3): Step 7.3 checks every name
  with `git grep`.
- **A local-only method with no `roymctl` entry** (B4b P14): the developer
  guide and the PR text give the exact `/rpc` call for
  `translation.targets`.
- **Sizes** (B3a R13, B4a R8): section 7.6 measures before and after in
  one build folder, with the owner's two limits.

## 8. Deferred backlog

Changes to [deferred-backlog.md](../../deferred-backlog.md) at close-out:

**Change:**

- §10 "A tagged chat message shows no language name" (`:557`, target
  "M08B B5"): B5 adds the translate action at the chat hook point, shown
  only with an engine. The language name is still not shown. Target
  becomes TBD.
- §11 "Matching has no semantic score and no query translation" (`:611`):
  add that `syneroym:translation` now exists (M08B) and the exchange search
  calls it in M08F (IM-20). Target stays M9A for the engine.

**New rows:**

| Item | Why deferred | Target | Source |
| --- | --- | --- | --- |
| No machine translation engine; every call answers `unavailable` | Needs local inference | M9A | `crates/sandbox_wasm/src/host_capabilities/capabilities_translation.rs` |
| The receipt card does not show what each party read | Nothing in v1 makes `viewed_via_mt` true (Q5) | M08C (receipt card redraw) | `crates/roym_web/ui/src/cards/templates/agreement_receipt.ts` |
| `detect-language` has no Roym method | No caller in v1 (Q6) | M9A | `crates/roym_web/src/app/translation.rs` |
| Chat search, contact search and profile text have no translation hook | Not in the M08B hook list | TBD | change doc "Translation hook points" |
| A machine translation is not kept; a reload translates again | No engine in v1 | M9A | `crates/roym_web/ui/src/translation/translation.ts` |
| A receiving node does not check the other party's `viewed_lang` against the quote | It is that party's own statement (Q4) | TBD | `crates/roym_transaction/src/app/sync.rs` |

No new `TODO` or `FIXME` marker is written.

## 9. Living docs to update at close-out

- `docs/system-architecture.md`: `:548` (the substrate parts behind the
  host interfaces: add `syneroym:translation`, no engine in v1);
  `:1211` (the `agreement-receipt` row: each half also signs `viewed_lang`
  and `viewed_via_mt`); `:1319` (`agreement.accept` takes what the person
  read); `:1742` (the linker list).
- `docs/system-requirements-spec.md:1464` (`[APP-A11Y]`: the interface and
  the hook points are built with no engine; MT itself stays Envisioned).
- `docs/TERMINOLOGY.md`: "Machine translation (MT)" (next to `:76`),
  "Translation hook point".
- `docs/developer-guide.md`: `agreement.accept` input (the two optional
  fields, the Q4 rule); the two `translation.*` methods with the exact
  `/rpc` call; a short note on `AppTranslation` next to "Calling Another
  Service from a WASM Component" (`:568` to `:585`).
- `docs/roym-integrated-experience-spec.md:476` (the `agreement-receipt`
  row: also what each party read).
- `docs/planning/traceability-matrix.md:68` (`[APP-A11Y]`: the new tests).
- `docs/decisions/0029-machine-translation-host-interface.md` (new, Q7).
- `AGENTS.md`: the WIT package list (in "Architecture", the
  `syneroym-wit-interfaces` bullet) adds `translation`.
- `task.md`: the B5 result line.
- Change doc (this plan commit): one new `## Deviations` entry, "ML-26 is
  met for four hook points in M08B", per task.md section 5.

## 10. Exit criteria

- Every test of section 7 exists and passes.
- `git ls-files -s` shows the seven `wit/deps/translation/translation.wit`
  as symlinks (mode `120000`).
- `git grep -n 'impl AppTranslation for'` finds `GuestHost`,
  `NativeAppHost` and `TestHost`, and no other.
- `git grep -n -E '"agreement.accept"' crates/roym_web/ui/src/screens/messages.ts`
  shows one call that sends `viewed_lang` and `viewed_via_mt`.
- Component and Hub sizes within the limits, written in task.md.
- The change doc, living docs, ADR-0029, backlog and status.md updated
  (sections 3, 8, 9).
- `mise run verify` passes with no `--skip`.
- The PR text has: the CLA line; the nine Hindi messages for owner review,
  with the two legal ones marked; "nothing is visible with the v1 node";
  the old-node behaviour and the substrate/component pair (section 4);
  that stored receipts from before B5 no longer verify; the new refusal on
  a repeated `agreement.accept` with bad viewed fields (D-B5-5); the exact
  `/rpc` call for `translation.targets`; before and after sizes; test
  counts.

## 11. What B5 does not do

- No engine, local or remote, and no config to name one (M9A).
- No translate action with the v1 node: nothing new for an end user.
- No hook point for widget answers (M08C), opinions (M08G), offering,
  want and storefront display (M08D), chat search, contacts or profile
  text.
- No translation in `roymctl`.
- No cache of machine translations.
- No display of `viewed_lang` on the receipt card (Q5).
- No `web` method for `detect-language` (Q6).
- No check by the receiving node of the other party's `viewed_lang`.
- No change to `roym-agreements`; M08C keeps the fields as they are.

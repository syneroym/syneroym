# M08A slice A2b — Typed methods for catalog and transaction: implementation plan

*Reader: the developer (or AI) who implements slice A2b. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan revised, 2026-10-30.** The owner answered Q1 to Q6 (all
option (a)). Plan review round 1 (P1 to P10 in
[slice-a2b-review.md](slice-a2b-review.md)) is folded in. Section 6
follows the answers.

**Size and PR.** L. One PR on `feat/m08a-a2b-typed-methods`, with the
commits of section 6 in that order. The branch starts from `main` at
`c399aae4` (A2a is merged, PR #334). A2c is a separate PR and starts after
this one is merged.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`). In a worktree use `cargo xtask verify` with
    `CARGO_TARGET_DIR` set.
  - Functions of at most 100 lines. Match arms of at most 10 lines. Files
    of at most 800 lines.
  - No planning ids in code: no "M08A", "A2b", "OF-14", "D-A2b", "B23" in
    code, comments or test names.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the ratchets (`check-file-lengths`, `check-duplication`).
  - Tests never hardcode a port.
- [task.md](task.md): section 1 (X2) and "A2 — Typed methods".
- [roym-open-foundation change doc](../../changes/roym-open-foundation/change.md):
  OF-14, OF-15, OF-18, "Typed methods", Decisions D-A2a-1 to D-A2a-18,
  Deviations.
- [slice-a2a-implementation-plan.md](slice-a2a-implementation-plan.md): the
  machinery this slice reuses, and the model for this plan.
- [slice-a2a-review.md](slice-a2a-review.md): the findings of A2a. Section
  7.5 of this plan lists how A2b avoids each kind.
- [deferred-backlog.md](../../deferred-backlog.md) section 10, the rows
  "Roym's sibling boundary is untyped JSON" and "`transaction` still has its
  own sibling-call helper".

### Terms used in this plan

- **Method, input struct, output struct, local-only method, wire-reachable
  method, unknown field, sibling call, certificate verbs, method table,
  shape file:** as defined in the A2a plan, section 0.
- **X2:** the typed-method code A2a built in `roym_core`:
  `method::parse_value`, `method::typed`, `ParamError`, `Reply<T>`,
  `NoParams`, `PingOutput`, `MethodInfo`, the `service_methods!` macro,
  `method::call_sibling` with `SiblingError`, and
  `method::check::assert_service_table`
  ([method.rs](../../../../crates/roym_core/src/method.rs),
  [check.rs](../../../../crates/roym_core/src/method/check.rs)).
- **Record type:** a Rust type that is the payload (or part of the payload)
  of a signed record, for example `ListingPayload`, `BookingTerms`,
  `AgreedTerms`, `Area`, `TimeWindow`. The same type parses records that
  other nodes signed. It never has `deny_unknown_fields`.
- **Deal card:** a signed transaction record (request, quote, agreement
  receipt, booking progress, payment or fulfilment record) sent as a message
  in a conversation. `send_card_and_file` sends one and files this node's
  own copy.
- **Send state:** the `state` word that a method returns after it sends a
  deal card: a conversation delivery state (`pending`, `deposited`,
  `delivered`, `failed`), or `not-sent`, `already-accepted`,
  `already-recorded`.
- **Hand-made bundle:** a backup bundle that was not written by `*.export`
  of this build. Only such a bundle can put a malformed row in storage.

## 1. What A2b builds, in one paragraph

A2b gives every method of the `catalog` service (17, with the two
certificate verbs) and the `transaction` service (41, with the two
certificate verbs and seven `*.ping` names) an input struct and an output
struct, listed in one `service_methods!` table per service. It builds no
new machinery: every piece of X2 is used as A2a left it. A request with a
wrong field type, a missing field or an unknown field now gets `-32602`
with the field name. The three sibling call sites that still build JSON by
hand (`transaction` to `catalog` twice, `directory` to `catalog` once) move
to `call_sibling` with typed structs, and `transaction`'s own
`sibling_call` and `catalog_call` are deleted. The wire does not change for
a correct request. Only the text of wrong requests, and the few cases in
section 4, change.

## How this is shown to an end user

An end user sees nothing new. Every screen works as before.

- An operator who runs `roymctl roym transaction booking list` no longer
  has the `--agreement` option. That option never filtered anything.
- A developer can run `cargo nextest run -p syneroym-roym-web --test
  method_contract`. It lists every catalog and transaction method and shows
  that each has a typed input and a typed output, and that a wrong or
  unknown field gets `-32602` with the field name.
- A developer can run `cargo nextest run -p syneroym-roym-web --test
  method_contract shapes`. It shows that the JSON every method returns has
  the same keys and types as before the change.

**Demo:** none. A2b adds no command, no screen and no working path. The
change doc's `demo:` list keeps its current line.

## 2. Points for the owner

Each question has the options, a recommendation, the reason and the
owner's answer. Section 6 follows the answers.

Two choices are not questions, because A2a already decided them for every
service: closed word sets in inputs are enums (D-A2a-10), so
`listing.list`'s `status` becomes `Option<ListingStatus>` and
`booking.list`'s `state` becomes `Option<BookingState>`; and the "whole
`params` is the bundle" form of `*.import` is removed (D-A2a-11).

### Q1. Record types inside method inputs: what happens to an unknown key inside them

`listing.set` takes `booking`, `payment`, `product`, `service`, `location`,
`relationship` and `service_record`. They are record types from
`roym_core::listing`
([listing_ops.rs:55-79](../../../../crates/roym_catalog/src/app/listing_ops.rs#L55)).
`request.set` takes `area: Area` and `window: TimeWindow`
([request_ops.rs:26-41](../../../../crates/roym_transaction/src/app/request_ops.rs#L26)).
`quote.set`'s terms hold `PaymentTiming`, `TimeWindow` and `QuoteLocation`.
The same types parse records that other nodes signed. If one of them got
`deny_unknown_fields`, this node would refuse every record from a newer node
that adds a field to that part.

- (a) **Keep the record types as they are.** The top level of every input
  struct refuses unknown keys. A key unknown to a record type inside it is
  ignored. It is never signed: the service builds the payload from the
  struct, so the dropped key is not in the record. Write this limit in the
  developer guide (A2a already says the check sees only the outer type),
  and add a backlog row.
- (b) **Add input copies of the record types**, each with
  `deny_unknown_fields`, and convert them into the record types. About ten
  types (more than 60 fields) written twice, and kept equal by hand.
- (c) **Add `deny_unknown_fields` to the record types.** This breaks
  reading records from a newer node, against the rule that wire data must
  survive a newer sender.

**Recommendation: (a).** The rule protects against a client bug: a field
the client thinks it set but the service never saw. In (a) the top-level
names, where most bugs are, are still checked. A wrong key one level down is
dropped and not signed, so no false data reaches a record. (b) doubles the
largest types of Roym for a small gain, and the copies can drift. (c) breaks
records. A wrong *type* inside a nested block is still refused with
`-32602` in every option. `data.field` names the deepest field it can: for
a plain block such as a listing's `payment` it is the field
(`payment.amount_minor`); for a tagged block such as `area` (`Area` is
`#[serde(tag = "kind")]`) it is the block itself (`area`), because serde
reads a tagged block into a buffer first and the path stops there.

Owner answer: (a), 2026-10-30.

### Q2. `quote.set`'s `terms`: what type the input has

Today `terms` is `serde_json::Value`
([quote_ops.rs:39](../../../../crates/roym_transaction/src/app/quote_ops.rs#L39)).
`parse_quote_terms`
([quote_ops.rs:226](../../../../crates/roym_transaction/src/app/quote_ops.rs#L226))
inserts `quote_expires_at_secs` (computed from `expires_in_secs`) and then
parses `AgreedTerms`. The check of A2a does not see a `Value` inside a
struct, so `terms: Value` would pass it, but the input would not be typed.

- (a) **A `QuoteTermsInput` struct** with the 13 fields of `AgreedTerms`
  except `quote_expires_at_secs`, with `deny_unknown_fields`, and
  `fn into_agreed_terms(self, quote_expires_at_secs: u64) -> AgreedTerms`.
  Its fields keep the `AgreedTerms` serde attributes (`payment_methods`
  and `schedule` have `#[serde(default)]`).
- (b) **`terms: AgreedTerms` with `quote_expires_at_secs` made
  `#[serde(default)]`.** No new type, but a received quote record with no
  expiry would then parse with expiry 0. That changes record parsing.
- (c) **Keep `terms: Value`.** Record it as a known hole.

**Recommendation: (a).** The terms are the most important input of the
deal. With (a) a wrong field there gets `data.field = "terms.amount_minor"`,
and an unknown key (for example a misspelt `payment_method`) is refused
instead of silently dropped. A client that sends `quote_expires_at_secs`
inside `terms` is refused (today it is silently overwritten). No caller
sends it (checked: the Hub, `roymctl`, the parity and e2e tests). The cost
is one struct of 13 fields beside `AgreedTerms`, and a unit test that the
two stay equal (Step 5a).

Owner answer: (a), 2026-10-30.

### Q3. The `*.verify` methods accept the envelope as text or as a JSON object

All seven verify methods (`listing.verify`, `request.verify`,
`quote.verify`, `agreement.verify`, `booking.verify`, `payment.verify`,
`fulfilment.verify`) read `envelope` as a string, or turn any other JSON
value into text ([app.rs:320-330](../../../../crates/roym_transaction/src/app.rs#L320),
[listing_ops.rs:524-536](../../../../crates/roym_catalog/src/app/listing_ops.rs#L524)).
One caller sends an object: the Hub's "Verify a listing" box parses the
pasted text and sends the object
([listings.ts:535-550](../../../../crates/roym_web/ui/src/screens/listings.ts#L535)).
Every other caller (`roymctl`, the parity and e2e tests) sends the text.

- (a) **Text only.** `VerifyInput { envelope: String }`. Fix the Hub to send
  the trimmed pasted text. It still runs `JSON.parse` only to show "That is
  not valid JSON." first.
- (b) **Accept both** with an untagged enum `EnvelopeArg { Text(String),
  Object(Map<String, Value>) }`, turned into text as today.

**Recommendation: (a).** This is the same choice the owner made for A2a Q9
(`profile_envelope`): a signed record is text everywhere else, and one
field with two accepted shapes is a second thing to document and test.
Sending the text is also more correct: today the Hub sends the record as
re-serialized JSON, not the bytes the person pasted.

Owner answer: (a), 2026-10-30.

### Q4. `roymctl roym transaction booking list --agreement` sends a field the service does not read

`booking` is a subcommand of `roymctl roym transaction`
([transaction.rs:142-145](../../../../apps/roymctl/src/commands/roym/transaction.rs#L142)).
`roymctl` sends `agreement` to `booking.list` when the option is given
([booking.rs:163-182](../../../../apps/roymctl/src/commands/roym/booking.rs#L163)).
`booking.list` has no such field
([booking_ops.rs:463-473](../../../../crates/roym_transaction/src/app/booking_ops.rs#L463)),
so today the option is ignored and every booking is listed. After A2b the
call answers `-32602`, `data.field = "agreement"`.

- (a) **Remove the option from `roymctl`.** `roymctl roym transaction
  booking get --agreement <id>` already shows one booking.
- (b) **Add an `agreement` filter to `booking.list`.**

**Recommendation: (a).** A2 adds no features and renames nothing. The
option never worked, so removing it takes nothing away. If a later client
needs the filter, it is a small addition then.

Owner answer: (a), 2026-10-30.

### Q5. Closed word sets in outputs: `String` or `enum`

Two output words are written as strings today: the send state (`state` of
`request.set`, `quote.set`, `agreement.accept`, `payment.request`,
`payment.acknowledge`, `fulfilment.sign`) and the booking view's `writer`
(`"self"`, `"peer"` or `null`,
[booking_ops.rs:286, :304, :548](../../../../crates/roym_transaction/src/app/booking_ops.rs#L286)).

- (a) **Enums.** `SendState { Pending, Deposited, Delivered, Failed,
  NotSent, AlreadyAccepted, AlreadyRecorded }` and `BookingWriter { Mine
  (renamed "self"), Peer }`, both kebab-case, in `roym_transaction`.
  `send_card_and_file` returns `SendState` (from `StoredState` on success).
- (b) **Keep `String`.**

**Recommendation: (a).** It is what A2a did for every closed set
(D-A2a-10). A client and a later schema can list the words. The wire does
not change: the shape files record the exact words (section 7.2).

Owner answer: (a), 2026-10-30.

### Q6. `directory`'s call to `listing.get`

`directory.publish-to-source` calls `listing.get` with a hand-made
`host.call` ([client_sources.rs:217-243](../../../../crates/roym_directory/src/app/client_sources.rs#L217)).
It is the only call to a catalog or transaction method from another crate
that is not in `roym_transaction`.

- (a) **A2b moves it to `call_sibling`** with the typed `ListingKeyInput`
  and `Option<ListingGetOutput>`, and keeps its error handling (B33).
- (b) **A2c moves it**, with the rest of `directory`.

**Recommendation: (a).** A2a moved the sibling sites of its two services in
the same slice (A2a Q8), so the compiler sees a field rename on both sides
at once. The structs are made here. It is one site, and the backlog row
"`transaction` still has its own sibling-call helper" can then close.

Owner answer: (a), 2026-10-30.

## 3. Facts checked in the code

All paths are from the repository root. Line numbers are at `c399aae4`.

### 3.1 What A2a already did in these two crates

- `catalog`'s `address_from_profile` calls `profile.get` through
  `call_sibling`, and maps `Refused` to `Ok(None)`
  ([listing_ops.rs:36-53](../../../../crates/roym_catalog/src/app/listing_ops.rs#L36)).
  No change.
- `transaction`'s `send_card_and_file`
  ([app.rs:552](../../../../crates/roym_transaction/src/app.rs#L552)), the
  countersign send in `agreement_ops.rs:483` and `fetch_sync_messages`
  ([sync.rs:122](../../../../crates/roym_transaction/src/app/sync.rs#L122))
  call `conversation.send` and `conversation.changes` through
  `call_sibling` with `SendInput`/`SendOutput` and
  `ChangesInput`/`ChangesOutput`. No change, except that
  `send_card_and_file` returns `SendState` (Q5).
- The certificate verbs of both services are served by
  `signing::handle_certificate_verb`
  ([signing.rs:258](../../../../crates/roym_core/src/signing.rs#L258)), which
  already calls the typed `signing_status` and
  `install_signing_certificate` (A2a B21). A2b lists them as rows instead.
  `handle_certificate_verb` stays for `directory` until A2c.
- Both `invoke` functions run `admit::require_internal` first
  ([catalog app.rs:87](../../../../crates/roym_catalog/src/app.rs#L87),
  [transaction app.rs:227](../../../../crates/roym_transaction/src/app.rs#L227)).
  **No method of either service is wire-reachable.** The wire list of both
  tables is empty, so every input uses `deny_unknown_fields`.
- Neither crate has unit tests under `src/` or a `tests/` folder. Every test
  of these methods is in the parity harness and the substrate e2e tests.

### 3.2 Methods, handlers and what they read

**catalog** (dispatch at [app.rs:95-115](../../../../crates/roym_catalog/src/app.rs#L95)):

| Method | Handler | Reads from `params` today |
| --- | --- | --- |
| `listing.ping` | inline `app.rs:96` | nothing |
| `listing.set` | `listing_ops.rs:396` | `SetListingParams` (`:55`, no `deny_unknown_fields`, error text `invalid listing params: {e}`). Parse runs first, then `owner_did`. |
| `listing.withdraw` | `:416` | `listing_id` (`get`) |
| `listing.get` | `:438` | `listing_id` |
| `listing.list` | `:456` | `status` (any string, sent to the host filter), `offset`, `limit` (`as_u64() as usize`, default 0 and 100) |
| `listing.history` | `:497` | `listing_id` |
| `listing.verify` | `:524` | `envelope` (string, or any JSON turned into text) |
| `listing.limits` | inline `app.rs:103` | nothing |
| `listing.set-limits` | `limits.rs:35` | `window_secs` (u64), `max_per_window` (`as_u64() as u32`, cut without a message) |
| `availability.set` | `availability.rs:10` | `listing_id`; `slots: Vec<SlotInput>` (`:3`, no `deny_unknown_fields`, text `invalid slots: {e}`) |
| `availability.get` | `:102` | `slot_id` |
| `availability.list` | `:58` | `listing_id`, `from_secs`, `to_secs` |
| `availability.remove` | `:84` | `slot_id` |
| `catalog.export` | `backup.rs:5` | nothing |
| `catalog.import` | `backup.rs:29` | `bundle`, or the whole `params` |
| `catalog.signing-status`, `catalog.install-signing-certificate` | `handle_certificate_verb` | A2a typed |

**transaction** (dispatch at [app.rs:236-278](../../../../crates/roym_transaction/src/app.rs#L236)).
Every handler except the pings, the verify methods and
`transaction.import` parses a struct with
`serde_json::from_value(req.params.clone())` and the text `invalid params:
{e}`, with no `deny_unknown_fields` and no field name in `data`.

| Method | Handler | Reads | Check before the parse |
| --- | --- | --- | --- |
| 7 pings: `request.`, `quote.`, `agreement.`, `receipt.`, `booking.`, `payment.`, `fulfilment.ping` | inline `app.rs:236` | nothing | — |
| `request.set` | `request_ops.rs:43` | `RequestSetParams` `:26` (`area: Option<Area>`, `window: Option<TimeWindow>`) | `resolve_principal_and_owner` (`:45`) |
| `request.get` / `.history` | `:244` / `:310` | `request_id` | — |
| `request.list` | `:271` | `ListParams` (`app.rs:203`: `conversation`, `mine`, `limit: usize` default 200, `offset: usize`) | — |
| `request.verify` | `verify_verb` `app.rs:320` | `envelope` (`get`) | — |
| `quote.set` | `quote_ops.rs:42` | `QuoteSetParams` `:29` (`terms: Value`) | `resolve_principal_and_owner` (`:44`) |
| `quote.get` / `.history` | `:362` / `:433` | `quote_id` | — |
| `quote.list` | `:392` | `ListParams` | — |
| `quote.verify` | `verify_verb` | `envelope` | — |
| `quote.decline` | `:456` | `quote_record_id`, `note` | `signing::owner_did` (`:458`) |
| `agreement.accept` | `agreement_ops.rs:39` | `quote_record_id` | `resolve_principal_and_owner` (`:41`) |
| `agreement.get` | `:296` | `quote_record_id` | — |
| `agreement.list` | `:336` | `conversation`, `limit`, `offset` (no `mine`) | — |
| `agreement.verify` | `verify_verb` | `envelope` | — |
| `booking.get` / `.start` / `.history` | `booking_ops.rs:250` / `:316` / `:396` | `AgreementParam { agreement }` (`:245`) | — |
| `booking.cancel` | `:358` | `agreement`, `reason` | — |
| `booking.list` | `:475` | `conversation`, `state: Option<String>` (compared to the stored word), `limit`, `offset` | — |
| `booking.verify` | `verify_verb` | `envelope` | — |
| `payment.request` | `payment_ops.rs:35` | `agreement`, `note` | — (parse first) |
| `payment.acknowledge` | `:205` | `agreement`, `observed_at_secs`, `method`, `reference`, `supersedes` | — |
| `payment.get` | `:430` | `AgreementParam` (a second copy, `:425`) | — |
| `payment.verify` | `:468` | `envelope` (`get`) | — |
| `fulfilment.sign` / `.get` | `fulfilment_ops.rs:33` / `:213` | `AgreementParam` (a third copy, `:28`) | — |
| `fulfilment.verify` | `verify_verb` | `envelope` | — |
| `transaction.sync` | `sync.rs:43` | `conversation`, `full` | — |
| `transaction.thread` | `thread.rs:32` | `conversation`, `limit: usize` default 200, `cursor: usize` | — |
| `transaction.export` / `.import` | `backup.rs:28` / `:69` | import: `bundle`, or the whole `params` | — |
| `transaction.signing-status`, `transaction.install-signing-certificate` | `handle_certificate_verb` | A2a typed | — |

### 3.3 What each method returns today

This is the base of the output structs in section 5. "null" means the key is
written with `null` when the value is missing (a `json!` with an `Option`).
"absent" means the key is left out.

- **Several success shapes for one method.**
  - `agreement.accept`: the normal answer has `booking` (a booking view, or
    `null` for the counterparty and when no slot is held); the
    "already-accepted" answer
    ([agreement_ops.rs:210-227](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L210))
    has no `booking` key, `message_id: ""` and `state: "already-accepted"`.
    `send_error` is absent unless set.
  - `payment.request`, `payment.acknowledge`, `fulfilment.sign`: the normal
    answer has a string `message_id` (it may be `""`) and a send state. The
    "already-recorded" answers have `message_id: null` and `state:
    "already-recorded"`. `payment.request` has no `role`; the other two
    have `role`. None of the three returns `send_error`.
  - **The fences.** Each of the three methods claims a fence row in
    `LEDGER` (`payreq:<agreement>`, `ack:<agreement>:<role>:<version>`,
    `fulfil:<agreement>:<role>`) before it writes the shared row. When the
    fence row already exists, the fence returns an answer through its `Err`
    side, and the handler returns that answer as it is
    (`if let Err(resp) = claim_..(..) { return resp; }`). The answer comes
    from one of six sites:

    | # | Site | What it returns |
    | --- | --- | --- |
    | F1 | [fence.rs:54](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L54) `Err(ensure_payment_request_recorded(..))` | The fence holds a half: crash recovery (below). |
    | F2 | [fence.rs:66](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L66) `Err(Response::ok(..))` | The fence holds no half, the shared row does: already-recorded. |
    | F3 | [fence.rs:161](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L161) `Err(ensure_payment_ack_recorded(..))` | The fence holds a half: crash recovery. |
    | F4 | [fence.rs:168-181](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L168) `Err(early)` | `check_acknowledgement_version` answered `Ok(Some(..))`: already-recorded, after a booking transition for the quote author. Its `Err` (`not-the-current-version`) is ignored here and the fence answers `payment-acknowledgement-in-flight`. |
    | F5 | [fulfilment_ops.rs:346](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L346) `Err(ensure_fulfilment_recorded(..))` | The fence holds a half: crash recovery. |
    | F6 | [payment_ops.rs:380-404](../../../../crates/roym_transaction/src/app/payment_ops.rs#L380) `check_acknowledgement_version` | Returns `Result<Option<Response>, Response>`: `Ok(Some(Response::ok(already-recorded)))` (`:388`), `Ok(None)`, or `Err(not-the-current-version)`. Called by F4 and by `payment_acknowledge` (`payment_ops.rs:224`). |

    **Crash recovery.** The fence row and the shared `PAYMENTS` or
    `FULFILMENTS` row are written in two calls. If the node stops between
    them, the fence holds a signed half that no shared row has and no card
    carried. The next call of the method signs a new half, finds the fence,
    and calls `ensure_*_recorded` with the half *in the fence*
    ([fence.rs:78-119, :198-260](../../../../crates/roym_transaction/src/app/payment_ops/fence.rs#L78),
    [fulfilment_ops.rs:365-418](../../../../crates/roym_transaction/src/app/fulfilment_ops.rs#L365)).
    That function writes the shared row, **sends the card**, and returns
    the **normal** answer: the fence half's `record_id`, a real
    `message_id` and the send state. It returns already-recorded only when
    the shared row already holds that half, and `-32603` when a row read or
    write fails. The other `Response::ok` sites of these methods are the
    handlers' own early returns (`payment_ops.rs:77`,
    `fulfilment_ops.rs:74` through `already_recorded_fulfilment_response`
    `:165`) and their normal answers.
  - `booking.history`: for the quote author, steps of `{seq, event,
    snapshot, envelope, record_id}` from the ledger; for the counterparty,
    `{seq, snapshot, record_id}` from verified `booking-progress` cards,
    where `seq` is read from the card data (`null` if missing) and
    `snapshot` is the raw card data
    ([booking_ops.rs:396-461](../../../../crates/roym_transaction/src/app/booking_ops.rs#L396)).
  - `payment.verify`: a `RecordVerdict` of the payment request, of the
    acknowledgement, or `RecordVerdict::<()>::refused(..)`.
- **null keys:** `quote.get`'s `request_record_id`, `counterparty_did`,
  `declined_at_secs`, `decline_note`; `quote.list`'s `request_record_id`,
  `counterparty_did`; `quote.decline`'s `note`; `agreement.get`/`.list`'s
  `counterparty`, `quote_author`; `payment.get`'s `request`;
  `fulfilment.get`'s `counterparty`, `quote_author`; every `Option` of
  `booking_view` ([booking_ops.rs:208-243](../../../../crates/roym_transaction/src/app/booking_ops.rs#L208)):
  `writer`, `seq`, `state`, `conflict`, `payment`, `fulfilment`,
  `track_window_ends_at_secs`, `cancel_reason`, `progress_record_id`.
- **absent keys:** `send_error` of `request.set`, `quote.set`,
  `agreement.accept`. Inside nested types (`RecordVerdict`,
  `ListingVerdict`, `CardRow`, `ReceiptHalf`, `AgreedTerms`) the existing
  `skip_serializing_if` attributes stay as they are.
- **`null` for a missing row:** `listing.get`, `availability.get`,
  `request.get`, `quote.get`, `agreement.get`. Their output is an `Option`.
- **Stored rows returned as they are:** `availability.get` and
  `availability.list` return the stored slot row JSON unchanged.
  `availability.set` writes exactly five keys (`slot_id`, `listing_id`,
  `start_secs`, `end_secs`, `capacity`), but `catalog.import` writes slot
  rows from the bundle without checking them
  ([backup.rs:64-104](../../../../crates/roym_catalog/src/app/backup.rs#L64)).
  `transaction.thread` returns stored `CardRow`s.
- **Maps:** `catalog.import` returns `{"imported": {<collection>: <count>}}`
  for the sections present. `serde_json` has no `preserve_order` (A2a
  review), so a `BTreeMap<String, usize>` writes the same order.

### 3.4 Callers (what each sends)

A throw-away script (kept out of the repository) listed the top-level keys
of every literal call to these 58 methods in `crates/roym_web/tests`,
`crates/substrate/tests` and `apps/roymctl/src`. Calls that build the method
name at run time or loop over names were read by hand. The Hub and the
sibling crates were read by hand.

- **Hub (`crates/roym_web/ui/src`).**
  `screens/listings.ts`: `listing.list` (no params) :121, `listing.history
  {listing_id}` :158, `listing.withdraw {listing_id}` :182, `listing.set`
  :444 with `buildSetListingParams` ([editor.ts:253](../../../../crates/roym_web/ui/src/listings/editor.ts#L253):
  `title`, `summary`, `categories`, `status`, and when set `slug`,
  `conversation_address`, `booking`, `payment`, `product`, `service`,
  `location`, `relationship`, `service_record`), `availability.list
  {listing_id}` :480, `availability.remove {slot_id}` :489,
  `availability.set {listing_id, slots: [{start_secs, end_secs,
  capacity}]}` :504, `listing.verify {envelope: <parsed object>}` :550
  (**sends an object**, Q3).
  `screens/messages.ts`: `request.set {conversation, description,
  categories, listing_id?, data_use_notice}` :304, `transaction.sync
  {conversation}` :333, `transaction.thread {conversation}` :334,
  `agreement.accept {quote_record_id}` :569, `quote.decline
  {quote_record_id, note?}` :611, `quote.set {request_record_id,
  expires_in_secs, terms: {11 keys (no `payment_methods`, no `schedule`),
  location {where, address?}}}` :791.
  `screens/backup.ts`: `catalog.export`/`.import`,
  `transaction.export`/`.import` with `{bundle}`. `enrolment.ts`:
  `<svc>.signing-status`. All keys are known to the new structs.
- **`roymctl`** (`apps/roymctl/src/commands/roym/`). `transaction.rs`:
  `request.set` :176, `quote.set` :257-282 (terms with 12 keys, plus
  `schedule` when given; `slot_id` without `listing_id`, see B40), `quote.decline` :295,
  `agreement.accept`/`.get` :445, :459, `transaction.sync {conversation,
  full}` :452, `transaction.thread` :378. `booking.rs`: `booking.get`,
  `booking.list {limit, offset, agreement?, conversation?, state?}` :172
  (**`agreement` is not a field**, Q4), `booking.start`, `booking.cancel
  {agreement, reason}`, `booking.history`, `payment.request {agreement,
  note?}`, `payment.acknowledge {agreement, observed_at_secs?, method?,
  reference?, supersedes?}`, `payment.get`, `payment.verify {envelope:
  <text>}` :250, `fulfilment.sign`, `fulfilment.get`. `signing.rs` and
  `backup.rs`: the certificate verbs and `<svc>.export`/`.import`.
- **Sibling services (Rust).**
  - `transaction` to `catalog`: `availability.get {slot_id}` in
    `validate_and_apply_slot`
    ([quote_ops.rs:236-271](../../../../crates/roym_transaction/src/app/quote_ops.rs#L236))
    and `claim_seat`
    ([ledger.rs:130-160](../../../../crates/roym_transaction/src/app/ledger.rs#L130)),
    both through `catalog_call` (`app.rs:500`) and `sibling_call`
    (`app.rs:479`). They read `listing_id`, `start_secs`, `end_secs`,
    `capacity` with `.get(..)`.
  - `directory` to `catalog`: `listing.get {listing_id}` in
    `publish_to_source`
    ([client_sources.rs:207-243](../../../../crates/roym_directory/src/app/client_sources.rs#L207)),
    a direct `host.call`; it reads `envelope`.
  - No other crate names a catalog or transaction method (checked by grep
    over `crates/roym_*/src`).
- **`web`.** Forwards `params` unchanged. No change.
- **Parity harness** (`crates/roym_web/tests/dual_build_parity/`, 24
  modules, 222 `fn scenario_`): `catalog.rs`, `booking.rs`, `bundles.rs`,
  `card_sync.rs`, `fulfilment.rs`, `payment.rs`, `transaction.rs`,
  `transaction_cards.rs`, `transaction_validation.rs`, `trust.rs`,
  `directory.rs`, `group.rs`, `wire_origin.rs`, `fixtures.rs`. Every key
  sent is a field of the new structs. Every `envelope` sent to a verify
  method is a string. The loops in `bundles.rs:87` and
  `transaction_validation.rs:8-58` send `{}` from off the node and expect
  `-32013`, which does not change (B16 of A2a).
  **Scenarios that read changed text:** scenario 40
  ([catalog.rs:71-84](../../../../crates/roym_web/tests/dual_build_parity/catalog.rs#L71))
  asserts the message contains `listing params` (B24). Scenarios 37 and
  124 send a valid input with no certificate and assert
  `signing-not-enrolled`; they keep passing (B23). Scenario 145 asserts the
  message contains `XYZ`; the currency check runs after the parse, so it
  keeps passing.
- **Substrate e2e** (`crates/substrate/tests/`): `roym_app_e2e.rs`,
  `roym_booking_e2e.rs`, `roym_conversation_e2e.rs`,
  `roym_directory_e2e.rs`, `roym_restore_e2e.rs`,
  `roym_transaction_e2e.rs`, `roym_trust_e2e.rs`, `roym_identity_e2e.rs`,
  and the helpers `common/roym.rs`, `common/roym_flow.rs`. Every key sent is
  a field of the new structs. They assert codes and result keys, not error
  texts of these methods.
- **Playwright and demo stories.** `roym-hub.spec.ts` drives the Hub's
  listing editor and captures the `listing.set` body (:209). The demo
  stories (`crates/substrate/tests/e2e/demo/`) do not use these services.
  Checked, no change.

## 4. Behaviour-change audit

A correct request gets the same answer after A2b. These are the requests
that do not, found by reading every caller of section 3.4 and every
handler of section 3.2. The numbers continue A2a's list (B1 to B22), so a
number in the change doc names one row.

| # | What is sent today | What the service does today | After A2b | How the plan handles it |
| --- | --- | --- | --- | --- |
| B23 | `request.set`, `quote.set`, `agreement.accept` with a bad input and no signing certificate; `quote.decline` with a bad input and no recorded owner | `-32602` with `signing-not-enrolled` (or "no recorded owner"): the check runs before the parse | `-32602` for the bad field | Both are `-32602`. The parse runs before the handler (D-A2a-1), as A2a B11. Scenarios 37 and 124 send a valid input and still get `signing-not-enrolled`. **`quote.set` also:** today `terms` is parsed late, after the certificate check, the `expires_in_secs` range check, `ensure_collections`, the request lookup (`no such request`, "a request cannot be quoted by the person who made it") and the revision checks. Now a wrong `terms` field is reported before all of them. Only the text changes (`-32602` both times). No parity or e2e test sends bad terms together with another error. |
| B24 | A wrong or missing field | `-32602` with texts such as `invalid params: missing field ..` (no path, no `data`), `invalid listing params: ..`, `invalid slots: ..`, `invalid terms: ..`, `terms must be an object`, `listing_id is required`, `envelope is required`, `bundle is required`, `invalid bundle: ..` | `-32602 invalid params: <path>: <text>` with `data.field` (D-A2a-6) | Only the text changes. Scenario 40 changes its assertion (Step 4). No other test or caller reads these texts (grep of section 3.4). |
| B25 | A field of the wrong type to a catalog method that reads with `.get(..)`, for example `listing_id: 5`, `limit: "10"`, `from_secs: "x"` | Read as absent: "is required", or the default | `-32602` with `data.field` | No caller sends one. |
| B26 | An unknown top-level key, to any of the 58 methods | Ignored | `-32602`, `data.field = <key>` | One caller sends one: `roymctl roym transaction booking list --agreement` (Q4, Step 6). |
| B27 | An unknown key inside a record type (`listing.set`'s `payment`, `request.set`'s `area`, and so on) | Ignored, not signed | The same | Q1 (a). Backlog row and developer-guide sentence. |
| B28 | A verify method with `envelope` as a JSON object (Hub "Verify a listing") | Turned into text and verified | `-32602`, `data.field = "envelope"` | Q3 (a): the Hub sends the text (Step 4). |
| B29 | `catalog.import` / `transaction.import` with the bundle as the whole `params` | Read as the bundle | `-32602 missing field `bundle`` | As A2a B9. Every caller sends `{bundle}`. |
| B30 | `listing.set-limits` with `max_per_window` above `u32::MAX` | Cut to 32 bits without a message | `-32602` | As A2a B6. No caller does this. |
| B31 | `listing.list` with a `status` word that is not a `ListingStatus`; `booking.list` with a `state` word that is not a `BookingState` | An empty list | `-32602 unknown variant`, `data.field` | D-A2a-10. The Hub sends no `status`. `roymctl --state` passes the person's text: a wrong word now gets an error instead of an empty list. |
| B32 | A slot row from a hand-made bundle with a key missing or of the wrong type | `availability.get` returns it as it is. `availability.list` lists it if it has `listing_id`. `transaction` reads missing numbers as 0. | `availability.get`: `-32603`. `availability.list`: the row is skipped. `transaction`: see B33. | Only a hand-made bundle can make such a row, as A2a B22. `list` skips rather than fails, so one bad row does not hide the others. |
| B33 | The three sibling sites of section 3.4 | `validate_and_apply_slot`: transport or unreadable reply → `-32603`; an error reply or a non-object `result` → `no-such-slot`. `claim_seat`: transport or unreadable reply → `Err`; an error reply or a non-object `result` → `SlotUnavailable`. `publish_to_source`: transport → `-32603 "listing.get: {e:?}"`; unreadable reply → `-32603`; an error reply or `null` → `-32602 "no such listing on this installation"`. | Same as today. `Refused` and `Ok(None)` take the "no such" path. `Transport` keeps its text (`call_sibling` writes `"{method}: {e:?}"`, the same). `BadReply` (also a malformed row, B32) → `no-such-slot`, `SlotUnavailable`, and `-32603` in `publish_to_source`. | Step 4 and Step 5 write each mapping. |
| B34 | Missing or `null` `params` to a method whose fields are all optional (`request.list`, `quote.list`, `agreement.list`, `booking.list`, the pings, `listing.list`) | `transaction`: `-32602 invalid type: null, expected struct ..`. `catalog`: works. | Works: `null` is `{}` (D-A2a-7) | A request that failed now works. No caller depends on the failure. |
| B35 | `limit`, `offset` or `cursor` above `u32::MAX` to a transaction list | WASM build: `-32602` (`usize` is 32 bits on `wasm32`). Native build: accepted. | Both builds accept it. The field is `u64`, turned into `usize` with `usize::try_from(v).unwrap_or(usize::MAX)` (D-A2b-6). | Removes a two-build difference. No caller sends such a value. |
| B36 | `quote.set` with `quote_expires_at_secs` inside `terms` | Overwritten without a message | `-32602`, `data.field = "terms.quote_expires_at_secs"` | Q2 (a). No caller sends it. |
| B37 | `agreement.accept` that finds this role's half already on file | No `booking` key, `message_id: ""`, `state: "already-accepted"` | The same | `booking: Option<Option<BookingView>>` with `skip_serializing_if = "Option::is_none"` (D-A2b-8). A shape file covers both answers. |
| B38 | Certificate verbs of both services | Served by `handle_certificate_verb` with the typed functions | The same functions, now as table rows | No change (A2a B21). |
| B39 | `booking.history` as the counterparty, for a verified `booking-progress` card | `snapshot` is the raw card data, `seq` is read from it (`null` if missing) | The same. `PeerStep.snapshot` is the card data as `Value`, and `seq` is read from it as today (D-A2b-8). | No change. Parsing the snapshot into `BookingProgressPayload` would drop a field that a newer peer added to its signed card; the raw data keeps it, as `CardRow.data` does. |

Found while checking, not changed by A2b:

- **B40.** `roymctl roym transaction quote --slot <id>` sends `slot_id`
  without `listing_id`. `roymctl` has no option for `listing_id` on that
  command. `validate_and_apply_slot` then always answers `slot requires
  listing_id`. This is a `roymctl` bug, not a typing question. A backlog row
  records it (section 8).

No other difference was found. The parity harness keeps its 24 module files
and its 222 `fn scenario_` functions. Only scenario 40's last assertion
changes (B24).

## 5. Decisions this plan makes

These follow the owner's answers to Q1 to Q6 (all option (a), 2026-10-30).
Every A2a decision (D-A2a-1 to D-A2a-18) applies as it is. These are the
new ones. Each answer has its decision: Q1 → D-A2b-3, Q2 → D-A2b-16,
Q3 → D-A2b-7, Q4 → D-A2b-17, Q5 → D-A2b-5, Q6 → D-A2b-11.

- **D-A2b-1. No new machinery.** Both services use `service_methods!`,
  `typed`, `parse_value`, `call_sibling` and `assert_service_table` as A2a
  built them. The only additions to `roym_core` are structs (D-A2b-2).
- **D-A2b-2. Where types live** (the rule of D-A2a-3).
  - `roym_core::method` (in `method.rs`, beside `NoParams`): `VerifyInput {
    envelope: String }` and `EnvelopeHistoryOutput { history: Vec<String> }`,
    because both services use them.
  - New `roym_core::method::catalog` (`crates/roym_core/src/method/catalog.rs`):
    `ListingKeyInput { listing_id }`, `ListingGetOutput`, `SlotKeyInput {
    slot_id }`, `SlotRow`, because `transaction` and `directory` call these
    methods.
  - Every other struct lives in the service crate, beside the handler that
    reads it. Structs shared by several handler files of one service live in
    that service's `app.rs` (as `ListParams` does today): `AgreementRef {
    agreement }` (replaces the three `AgreementParam` copies) and
    `ListInput` (replaces `ListParams`).
  - `SendState`, `BookingWriter` and `BookingView` live in
    `roym_transaction` (`app.rs` and `app/booking_ops.rs`).
  - `QuoteTermsInput` lives in `app/quote_ops.rs`.
- **D-A2b-3. Every input refuses unknown fields at its top level**
  (`deny_unknown_fields` on every input struct and on `SlotInput`). Record
  types inside inputs are not changed (Q1 (a)).
- **D-A2b-4. Existing field attributes stay.** A field that has
  `#[serde(default)]` or `#[serde(default = "..")]` today keeps it, so an
  absent field behaves as today. An explicit `null` for a non-`Option`
  field stays an error, as today.
- **D-A2b-5. Closed sets.** Inputs: `listing.list` `status:
  Option<ListingStatus>`, `booking.list` `state: Option<BookingState>`
  (D-A2a-10). Outputs: `SendState` and `BookingWriter` (Q5 (a)). The words
  do not change. `booking.list` compares `BookingState` values, not
  serialized strings.
- **D-A2b-6. List numbers are `u64`.** `limit`, `offset` and `cursor` of
  every list method of both services are `Option<u64>` or `u64` with the
  existing default, turned into `usize` with `usize::try_from(v)
  .unwrap_or(usize::MAX)` (B35). `listing.set-limits` `max_per_window` is
  `u32`, `window_secs` is `u64`. `SlotInput.capacity` stays `u32`.
- **D-A2b-7. The verify methods take text** (Q3 (a)): `VerifyInput {
  envelope: String }`. Their outputs are the verdict types the verify
  functions return today: `ListingVerdict`, `RecordVerdict<RequestPayload>`,
  `RecordVerdict<QuotePayload>`, and so on. `payment.verify` returns
  `PaymentVerdict`, an untagged enum (serialize only) of
  `RecordVerdict<PaymentRequestPayload>`,
  `RecordVerdict<PaymentAcknowledgementPayload>` and `RecordVerdict<()>`.
  `verify_verb` and `RecordKind` are deleted. One helper sits beside
  `VerifyInput` in `roym_core::method`:
  ```rust
  /// Runs one record verifier on the envelope text, at the current time.
  pub fn verify_with<V>(input: VerifyInput, verify: fn(&str, u64) -> V) -> Reply<V> {
      Ok(verify(&input.envelope, clock::now_secs()))
  }
  ```
  Each verify handler is one line that calls it:
  `verify_with(input, transaction::verify_request)`,
  `verify_with(input, listing::verify_envelope)`, and the same for
  `verify_quote`, `verify_agreement_receipt`, `verify_booking_progress` and
  `verify_fulfilment_receipt` (all are `fn(&str, u64) -> V`).
  `payment.verify` keeps its own handler, because it picks the verifier by
  record type.
- **D-A2b-8. Outputs that have more than one shape.**
  - `agreement.accept`: one struct, `booking: Option<Option<BookingView>>`
    with `skip_serializing_if = "Option::is_none"`. `None` leaves the key
    out (already-accepted). `Some(None)` writes `null`.
  - `payment.request`: `PaymentRequestOutput { record_id, message_id:
    Option<String>, state: SendState }`. `payment.acknowledge` and
    `fulfilment.sign`: `HalfRecordedOutput { record_id, role: DealRole,
    message_id: Option<String>, state: SendState }`. `message_id` has no
    skip: `Some("")` or `Some(id)` on the normal path, `None` (`null`) on
    the already-recorded path.
  - `booking.history`: `BookingHistoryOutput { history:
    Vec<BookingHistoryStep> }`, where `BookingHistoryStep` is an untagged
    enum (serialize only) of `AuthorStep { seq, event, snapshot, envelope,
    record_id }` and `PeerStep { seq: Option<u32>, snapshot: Value,
    record_id: Option<String> }`. `snapshot` is the card data as it is
    today, and `seq` is read from it as today (`None`, written as `null`,
    when it is missing). `snapshot` stays `Value` for the same reason as
    `CardRow.data`: it is a record another node signed, and a newer peer's
    extra keys must not be dropped (B39). The check allows a `Value` inside
    an output struct. A serialize-only untagged enum is safe here: nobody
    reads this output back through `call_sibling` (the A2a P7 risk was a
    read-back).
- **D-A2b-9. No `Response::ok` in handler code, and the fences return
  typed answers.** A handler returns `Ok(Output)`. The six fence sites of
  section 3.3 (F1 to F6) change to these signatures (in
  `payment_ops/fence.rs`, `payment_ops.rs` and `fulfilment_ops.rs`):
  ```rust
  /// What a fence claim gives back: the row is ours now, or the method
  /// already has its answer (crash recovery or already-recorded).
  pub(crate) enum FenceOutcome<O> { Claimed, Answered(O) }

  async fn claim_payment_request_fence<H: AppHost>(..) -> Result<FenceOutcome<PaymentRequestOutput>, Response>;
  async fn ensure_payment_request_recorded<H: AppHost>(..) -> Reply<PaymentRequestOutput>;
  async fn claim_payment_ack_fence<H: AppHost>(..) -> Result<FenceOutcome<HalfRecordedOutput>, Response>;
  async fn ensure_payment_ack_recorded<H: AppHost>(..) -> Reply<HalfRecordedOutput>;
  async fn claim_fulfilment_fence<H: AppHost>(..) -> Result<FenceOutcome<HalfRecordedOutput>, Response>;
  async fn ensure_fulfilment_recorded<H: AppHost>(..) -> Reply<HalfRecordedOutput>;
  async fn already_recorded_fulfilment_response<H: AppHost>(..) -> Reply<HalfRecordedOutput>;
  fn check_acknowledgement_version(..) -> Result<Option<HalfRecordedOutput>, Response>;
  ```
  The arguments do not change. Each site maps as follows. "Same" means the
  same branch and the same values as today.

  | Site | Today | Typed |
  | --- | --- | --- |
  | F1, F3, F5 | `return Err(ensure_*_recorded(..).await)` | `return ensure_*_recorded(..).await.map(FenceOutcome::Answered)`. An `Ok` answer (normal or already-recorded) becomes `Answered`. A `-32603` stays `Err`. Never turn the crash-recovery answer into already-recorded. |
  | F2 | `return Err(Response::ok(json!{record_id, message_id: null, state: "already-recorded"}))` | `return Ok(FenceOutcome::Answered(PaymentRequestOutput { record_id, message_id: None, state: SendState::AlreadyRecorded }))` |
  | F4 | `if let Ok(Some(early)) = check_acknowledgement_version(..) { transition; return Err(early) }` | `if let Ok(Some(early)) = .. { transition; return Ok(FenceOutcome::Answered(early)) }`. The `Err` of the check stays ignored here, as today, so the fence still answers `payment-acknowledgement-in-flight`. |
  | F6 | `Ok(Some(Response::ok(json!{record_id, role, message_id: null, state: "already-recorded"})))` | `Ok(Some(HalfRecordedOutput { record_id, role, message_id: None, state: SendState::AlreadyRecorded }))` |
  | inside `ensure_*_recorded` | `Response::ok(..)` for already-recorded and for the normal answer; `Response::internal_error(..)` on a row error | `Ok(..)` with the output struct; `Err(Response::internal_error(..))`. Same branches, same order, the card is still sent in the same place. |
  | the "in-flight" and `create` errors | `Err(Response::..)` | Same. |

  Each handler reads a claim like this:
  ```rust
  match claim_payment_request_fence(host, ..).await? {
      FenceOutcome::Claimed => {}
      FenceOutcome::Answered(out) => return Ok(out),
  }
  ```
  and `payment_acknowledge` reads `check_acknowledgement_version` at
  `payment_ops.rs:224` with `Ok(Some(out))` → transition, then
  `return Ok(out)`; `Ok(None)` → go on; `Err(resp)` → `return Err(resp)`,
  as today. After Step 5b, `grep -rn "Response::ok"
  crates/roym_catalog/src crates/roym_transaction/src` prints nothing.
- **D-A2b-10. `booking_view` returns `BookingView`.** Its nine callers
  (eight in `booking_ops.rs`, one in `agreement_ops.rs:392`) use the
  struct. `prepare_author_booking` returns `Option<BookingView>`. This is
  done in Step 5a, because `agreement.accept` needs it.
- **D-A2b-11. Sibling sites keep their error handling** (B33). Each site
  names the mapping of `Ok(None)`, `Refused`, `Transport` and `BadReply` in
  a `match`. `sibling_call` and `catalog_call` are deleted. `directory`'s
  call to `listing.get` in `publish_to_source` also moves to
  `call_sibling` in this slice (Q6 (a)); the rest of that handler waits for
  A2c.
- **D-A2b-12. Exports use `backup::export_signed_bundle`**, which already
  returns `Reply<Bundle>` (A2a). `backup::export_signed` keeps serving
  `directory` until A2c.
- **D-A2b-13. The shape helper gains closed-word keys.** `role`, `pair`,
  `next`, `payment_timing`, `track`, `writer`, `conflict`, `card_type`,
  `revocation_status`, `payment`, `fulfilment`, `where`, `event` record
  their exact string value (D-A2a-14). For a key whose value is an object
  (`payment` of a listing), nothing changes: only strings are recorded by
  value. `event` (the author steps of `booking.history`: `start`, `cancel`,
  `half`, `tick`) stays a `String` in the output, because it is copied from
  the stored `StepRow` (stored rows keep their strings, D-A2a-10).
- **D-A2b-14. Shared deal set-up for tests.** The set-up helpers that reach
  an accepted quote, a held slot and an active booking move from the
  scenario files into a new parity module
  `crates/roym_web/tests/dual_build_parity/deal_fixtures.rs`. The parity
  binary and the `method_contract` binary both include it. One copy each
  is kept:
  - `setup_peer_request`: three copies (`booking.rs:114`,
    `fulfilment.rs:46`, `payment.rs:45`) become one. `bundles.rs:116-127`
    (scenario 168) writes the same set-up inline and calls it instead.
  - `setup_active_agreement(h, conv, timing)`: two copies
    (`fulfilment.rs:7`, `payment.rs:7`) become one. The `payment.rs` copy
    has no `timing` argument and keeps the sample terms' `"after-work"`, so
    `payment.rs` calls `setup_active_agreement(&h, &conv, "after-work")`:
    the same params as today.
  - `setup_listing_with_slot` (`booking.rs:96`) and
    `setup_accepted_counterparty_quote` (`booking.rs:573`) move as they
    are.
  - `seed_row` (`admission.rs:242`) moves too, with a `service` argument
    (today it always opens the `conversation` database). `admission.rs`
    calls it with `"conversation"`. Step 2 uses it for the crash-recovery
    cases.
- **D-A2b-15. The transaction conversion is two commits**, so each can be
  reviewed alone: deal records first, then booking, payment and
  fulfilment. Between them, `invoke` tries `api::dispatch` and falls back
  to the old `match` for the methods not yet listed. The second commit
  removes the fallback. The check test (Step 7) runs only after both.
- **D-A2b-16. `quote.set` takes typed terms** (Q2 (a)). `QuoteSetInput.terms`
  is `QuoteTermsInput`: the 13 fields of `AgreedTerms` except
  `quote_expires_at_secs`, with the same serde attributes, and
  `#[serde(deny_unknown_fields)]`. `into_agreed_terms(self,
  quote_expires_at_secs: u64) -> AgreedTerms` builds the record type with a
  struct literal, so a field added to `AgreedTerms` and not to the input
  does not compile. `parse_quote_terms` is deleted. The terms are parsed
  with the rest of the input, before any other check (B23).
- **D-A2b-17. `roymctl` loses `booking list --agreement`** (Q4 (a)).
  `booking.list` gains no field. `roymctl roym transaction booking get
  --agreement <id>` is the way to see one booking.

### Method tables

`Page` fields are written out in each struct (D-A2a-4: no `flatten` on
inputs). Every input struct has `#[serde(deny_unknown_fields)]`. "svc" means
the service crate, beside the handler. "core" means `roym_core`.

**catalog (17 methods)**

| Method | Input | Output | Where |
| --- | --- | --- | --- |
| `listing.ping` | `NoParams` | `PingOutput` | core |
| `listing.set` | `SetListingInput` (the fields of `SetListingParams` with their attributes) | `ListingWriteOutput { listing_id, record_id, version_count: u64 }` | svc |
| `listing.withdraw` | `ListingKeyInput` | `ListingWriteOutput` | core / svc |
| `listing.get` | `ListingKeyInput` | `Option<ListingGetOutput>` (`envelope`, `record_id`, `listing_id`, `status: ListingStatus`, `updated_at_secs`) | core |
| `listing.list` | `ListingListInput { status: Option<ListingStatus>, offset: Option<u64>, limit: Option<u64> }` (defaults 0 and 100) | `ListingListOutput { listings: Vec<ListingListItem> }` (`listing_id`, `slug`, `title`, `status`, `record_id`, `updated_at_secs`, `version_count`) | svc |
| `listing.history` | `ListingKeyInput` | `EnvelopeHistoryOutput` | core |
| `listing.verify` | `VerifyInput` | `ListingVerdict` | core |
| `listing.limits` | `NoParams` | `PublicationLimits` | core (exists, `roym_core::safety`) |
| `listing.set-limits` | `PublicationLimitsInput { window_secs: u64, max_per_window: u32 }` | `PublicationLimits` | svc |
| `availability.set` | `AvailabilitySetInput { listing_id, slots: Vec<SlotInput> }` | `AvailabilitySetOutput { listing_id, slot_ids: Vec<String> }` | svc |
| `availability.get` | `SlotKeyInput` | `Option<SlotRow>` (`slot_id`, `listing_id`, `start_secs: u64`, `end_secs: u64`, `capacity: u32`) | core |
| `availability.list` | `AvailabilityListInput { listing_id, from_secs: Option<u64>, to_secs: Option<u64> }` | `AvailabilityListOutput { slots: Vec<SlotRow> }` | svc |
| `availability.remove` | `SlotKeyInput` | `AvailabilityRemoveOutput { removed: bool }` | svc |
| `catalog.export` | `NoParams` | `Bundle` | core |
| `catalog.import` | `ImportInput` | `CatalogImportOutput { imported: BTreeMap<String, usize> }` | svc |
| `catalog.signing-status` | `NoParams` | `SigningStatusOutput` | core |
| `catalog.install-signing-certificate` | `InstallCertificateInput` | `InstallCertificateOutput` | core |

**transaction (41 methods)**

| Method | Input | Output |
| --- | --- | --- |
| `request.ping`, `quote.ping`, `agreement.ping`, `receipt.ping`, `booking.ping`, `payment.ping`, `fulfilment.ping` | `NoParams` | `PingOutput` (`service: "transaction"`) |
| `request.set` | `RequestSetInput` (the fields of `RequestSetParams`) | `RequestSetOutput { request_id, record_id, version_count: u64, message_id: String, state: SendState, send_error: Option<String> (absent) }` |
| `request.get` | `RequestKeyInput { request_id }` | `Option<RequestView>` (`envelope`, `record_id`, `request_id`, `conversation`, `mine`, `updated_at_secs`, `version_count`) |
| `request.list` | `ListInput { conversation: Option<String>, mine: Option<bool>, limit: u64 (default 200), offset: u64 (default 0) }` | `RequestListOutput { requests: Vec<RequestView> }` |
| `request.history` | `RequestKeyInput` | `EnvelopeHistoryOutput` |
| `request.verify` | `VerifyInput` | `RecordVerdict<RequestPayload>` |
| `quote.set` | `QuoteSetInput { request_record_id, quote_id: Option, listing_id: Option, slot_id: Option, expires_in_secs: u64, terms: QuoteTermsInput }` | `QuoteSetOutput { quote_id, record_id, version_count, expires_at_secs, message_id, state: SendState, send_error (absent) }` |
| `quote.get` | `QuoteKeyInput { quote_id }` | `Option<QuoteView>` (the 7 keys of `RequestView` with `quote_id`, plus `request_record_id`, `counterparty_did`, `declined_at_secs`, `decline_note`, all null) |
| `quote.list` | `ListInput` | `QuoteListOutput { quotes: Vec<QuoteListItem> }` (as `QuoteView` without `declined_at_secs` and `decline_note`) |
| `quote.history` | `QuoteKeyInput` | `EnvelopeHistoryOutput` |
| `quote.verify` | `VerifyInput` | `RecordVerdict<QuotePayload>` |
| `quote.decline` | `QuoteDeclineInput { quote_record_id, note: Option }` | `QuoteDeclineOutput { quote_record_id, declined: bool, note: Option<String> (null) }` |
| `agreement.accept` | `QuoteRecordInput { quote_record_id }` | `AgreementAcceptOutput { quote_record_id, role: DealRole, record_id, pair: PairState, booking: Option<Option<BookingView>>, message_id: String, state: SendState, send_error (absent) }` |
| `agreement.get` | `QuoteRecordInput` | `Option<AgreementView>` (`quote_record_id`, `conversation`, `counterparty_did`, `quote_author_did`, `counterparty` (null), `quote_author` (null), `pair`, `terms: AgreedTerms`) |
| `agreement.list` | `AgreementListInput { conversation: Option, limit: u64 (200), offset: u64 (0) }` | `AgreementListOutput { agreements: Vec<AgreementView> }` |
| `agreement.verify` | `VerifyInput` | the verdict of `transaction::verify_agreement_receipt` |
| `booking.get`, `booking.start` | `AgreementRef` | `BookingView` |
| `booking.cancel` | `BookingCancelInput { agreement, reason: String }` | `BookingView` |
| `booking.list` | `BookingListInput { conversation: Option, state: Option<BookingState>, limit: u64 (200), offset: u64 (0) }` | `BookingListOutput { bookings: Vec<BookingView> }` |
| `booking.history` | `AgreementRef` | `BookingHistoryOutput` (D-A2b-8) |
| `booking.verify` | `VerifyInput` | the verdict of `booking::verify_booking_progress` |
| `payment.request` | `PaymentRequestInput { agreement, note: Option }` | `PaymentRequestOutput` |
| `payment.acknowledge` | `PaymentAckInput { agreement, observed_at_secs: Option<u64>, method: Option, reference: Option, supersedes: Option }` | `HalfRecordedOutput` |
| `payment.get` | `AgreementRef` | `PaymentGetOutput { request: Option<ReceiptHalf> (null), counterparty: Vec<ReceiptHalf>, quote_author: Vec<ReceiptHalf>, track: TrackState }` |
| `payment.verify` | `VerifyInput` | `PaymentVerdict` |
| `fulfilment.sign` | `AgreementRef` | `HalfRecordedOutput` |
| `fulfilment.get` | `AgreementRef` | `FulfilmentGetOutput { counterparty: Option<ReceiptHalf> (null), quote_author: Option<ReceiptHalf> (null), track: TrackState }` |
| `fulfilment.verify` | `VerifyInput` | the verdict of `fulfilment::verify_fulfilment_receipt` |
| `transaction.sync` | `SyncInput { conversation, full: bool (default false) }` | `SyncOutput { scanned, filed, refused, unknown, countersigned, deferred: u32, last_seq: u64 }` |
| `transaction.thread` | `ThreadInput { conversation, limit: u64 (200), cursor: u64 (0) }` | `ThreadOutput { cards: Vec<CardRow> }` |
| `transaction.export` | `NoParams` | `Bundle` |
| `transaction.import` | `ImportInput` | `TransactionImportOutput { imported: bool }` |
| `transaction.signing-status` | `NoParams` | `SigningStatusOutput` |
| `transaction.install-signing-certificate` | `InstallCertificateInput` | `InstallCertificateOutput` |

`BookingView` is the object of `booking_view` today
([booking_ops.rs:228-242](../../../../crates/roym_transaction/src/app/booking_ops.rs#L228)):
`agreement: String`, `writer: Option<BookingWriter>`, `seq: Option<u32>`,
`state: Option<BookingState>`, `conflict: Option<ConflictReason>`,
`payment: Option<TrackState>`, `fulfilment: Option<TrackState>`,
`track_window_ends_at_secs: Option<u64>`, `cancel_reason: Option<String>`,
`progress_record_id: Option<String>`, `next: NextStep`, `payment_timing:
PaymentTiming`, `pair: PairState`. No `Option` has a skip (they are all
null today). Check each field type against `BookingProgressPayload` in
`roym_core::booking`; use the type the snapshot already has.

`CardRow` is a stored row and keeps its attributes (it has `data:
Option<Value>`, which is the signed payload of a card of any type). The
check does not look inside it (A2a R6 limit), and that is right here: a
card's data is a record, not an input.

## 6. Step-by-step changes

**Rule for every commit:** the workspace builds, and the checks of section
7.1 pass at the end of it. The shape files of Step 2 stay green from Step 2
to the end. Rebuild the WASM components with `mise run build:roym` before
you run the parity suite or the substrate e2e tests (they load built
artifacts; a stale artifact runs the old code).

### Step 1 — Commit this plan

- Commit this file and the `status.md` row: `docs(planning): plan M08A
  slice A2b`. Done by the planning session.

### Step 2 — Shared deal set-up and the output shapes from the unchanged code

**Commit:** `test(roym-web): record catalog and transaction shapes`.

- New `crates/roym_web/tests/dual_build_parity/deal_fixtures.rs`, declared
  in `crates/roym_web/tests/dual_build_parity.rs` with `#[path =
  "dual_build_parity/deal_fixtures.rs"] mod deal_fixtures;` like the other
  modules. Move into it the helpers of D-A2b-14, one copy each, and make
  them `pub(crate)`. `setup_peer_request` keeps the
  `format!("m-req-{req_id}")` message id of the `fulfilment.rs` and
  `payment.rs` copies; the `booking.rs` copy used `"m-req-setup"`, and no
  test reads that id (`grep -rn "m-req-setup" crates/roym_web/tests` finds
  only the definition). `setup_active_agreement` keeps the `timing`
  argument of the `fulfilment.rs` copy; `payment.rs` passes
  `"after-work"`. Scenario 168 in `bundles.rs` calls `setup_peer_request`
  in place of its inline copy. `seed_row` takes the service name. Delete
  every other copy. The scenario files import the helpers from
  `deal_fixtures`. `fixtures.rs` is 778 lines, so it does not take them.
- `crates/roym_web/tests/method_contract.rs`: add `#[allow(dead_code)]
  #[path = "dual_build_parity/deal_fixtures.rs"] mod deal_fixtures;` and
  the two new modules `#[path = "method_contract/shapes_catalog.rs"] mod
  shapes_catalog;` and `#[path = "method_contract/shapes_transaction.rs"]
  mod shapes_transaction;`. Always with `#[path]` (A2a P19).
- `crates/roym_web/tests/method_contract/shapes.rs`: add the keys of
  D-A2b-13 to `is_closed_word_key`. Make `rpc`, `enrol_native_signing`
  and `assert_shape` `pub(super)` so the new files reuse them. Do not copy
  them.
- `shapes_catalog.rs`: one test function `catalog_shapes`. Required cases:
  `listing.ping`; `listing.set` (new, and a second version);
  `listing.withdraw`; `listing.get` found and missing; `listing.list` empty
  and with an active and a withdrawn row; `listing.history`;
  `listing.verify` good and tampered; `listing.limits`;
  `listing.set-limits`; `availability.set`; `availability.get` found and
  missing; `availability.list` empty and with two slots;
  `availability.remove` found and missing; `catalog.export`;
  `catalog.import`; both certificate verbs (the unconfirmed and the
  confirmed `signing-status`).
- `shapes_transaction.rs`: one test function per area (`deal_shapes`,
  `booking_shapes`, `payment_fulfilment_shapes`, `sync_backup_shapes`).
  Build the state with the `deal_fixtures` helpers. Required cases: the
  seven pings; `request.set` (sent, and the `send_error` case if the
  harness can make the send fail; if it cannot, say so in the PR);
  `request.get` found and missing; `request.list` empty and with rows;
  `request.history`; `quote.set`; `quote.get` found, declined, and
  missing; `quote.list`; `quote.history`; `quote.decline`;
  `agreement.accept` by the quote author with a slot (booking object), by
  the counterparty (`booking: null`), and a second time (already-accepted,
  no `booking` key); `agreement.get` found and missing; `agreement.list`;
  `booking.get` as author and as counterparty; `booking.list`;
  `booking.start`; `booking.cancel`; `booking.history` as author and as
  counterparty; `payment.request` new and already-recorded;
  `payment.acknowledge` new and already-recorded; `payment.get` empty and
  with rows; `fulfilment.sign` new and already-recorded;
  `fulfilment.get`; every verify method with a good envelope and with a
  bad one (`payment.verify` with a request, an acknowledgement and a
  non-payment record); `transaction.sync`; `transaction.thread` with a
  request, a quote and a payment-request card; `transaction.export`;
  `transaction.import`; both certificate verbs.
- **Crash-recovery cases** (the F1, F3 and F5 paths of section 3.3). For
  each of `payment.request`, `payment.acknowledge` and `fulfilment.sign`:
  call the method once (it claims the fence, writes the shared row and
  sends the card). Then make the state of a crash between the two writes:
  with `seed_row(h, false, "transaction", ..)`, write the shared row again
  without this half — `payments/<agreement>` with no `request`
  (`{"agreement", "conversation", "updated_at_secs"}`); for the
  acknowledgement, the same row with `request` copied from `payment.get`
  and the role's list empty; `fulfilments/<agreement>` with no half for
  this role. Call the method again. Record the answer
  (`payment_request_crash_recovery`, `payment_ack_crash_recovery`,
  `fulfilment_sign_crash_recovery`), and assert that its `record_id` is
  the first call's `record_id` (the half in the fence, not the new one) and
  that `state` is not `already-recorded`. Recorded on the unchanged code,
  these prove the typed fences keep the path. Only the native build is
  seeded, as `rpc` calls only the native build.
- Record the files on the **unchanged** code:
  ```bash
  mise run build:roym > target/a2b-build.log 2>&1; echo "exit=$?"; tail -n 3 target/a2b-build.log
  UPDATE_METHOD_SHAPES=1 cargo nextest run -p syneroym-roym-web --test method_contract shapes
  ```
- **Proof:** `cargo nextest run -p syneroym-roym-web --test method_contract
  shapes` passes with no env var. `git diff --stat
  crates/roym_web/tests/method_shapes/` shows only new files: the 63 A2a
  files do not change with the longer key list. If one does change, find
  out why before you go on (it means an A2a output has one of the new keys
  as a string). The parity suite still has 24 modules and 222 scenarios
  and passes: the moved helpers did not change.

### Step 3 — Shared structs in `roym_core`

**Commit:** `feat(roym-core): add catalog method types`.

- `crates/roym_core/src/method.rs`: add `VerifyInput` (`deny_unknown_fields`)
  and `EnvelopeHistoryOutput`. Add `pub mod catalog;`.
- New `crates/roym_core/src/method/catalog.rs`: `ListingKeyInput`,
  `ListingGetOutput`, `SlotKeyInput` (inputs with `deny_unknown_fields`),
  `SlotRow` (a stored row: no `deny_unknown_fields`, derives `Serialize` and
  `Deserialize`).
- Unit tests in `method/tests.rs` (names by behaviour):
  `slot_row_reads_the_five_keys_the_catalog_writes` (the JSON of
  `availability.set`'s row), `listing_get_output_keeps_the_five_keys`,
  `verify_input_refuses_an_object_envelope`.
- **Proof:** `cargo nextest run -p syneroym-roym-core` passes.
  `cargo build -p syneroym-roym-core --target wasm32-wasip2` builds.

### Step 4 — Convert `catalog`, the `directory` call site and the Hub verify box

**Commit:** `refactor(roym-catalog): type every method`.

- New `crates/roym_catalog/src/api.rs` with `service_methods!` (17 rows)
  and `pub fn method_table() -> Vec<MethodInfo>`, the same shape as
  [profile api.rs](../../../../crates/roym_profile/src/api.rs). `ping` is
  a function in `api.rs`. Add `pub mod api;` to `lib.rs`.
- `app.rs`: `invoke` becomes `require_internal`, then `api::dispatch`, then
  `Response::method_not_found(&req.method)`. No `match` on names and no
  `handle_certificate_verb`. Remove the imports that are no longer used.
- Every handler in `listing_ops.rs`, `availability.rs`, `limits.rs`,
  `backup.rs` changes to `async fn name<H: AppHost>(host: &H, input:
  Input) -> Reply<Output>`. Rules:
  - Delete every `req.params.get(..)` and `from_value(req.params..)`.
  - Keep the order of the other checks and every other error text.
  - `listing.set`: `SetListingParams` becomes `SetListingInput` with
    `deny_unknown_fields`; `build_payload` takes it.
  - `listing.list`: filter by `status.map(|s| json!({"status": s}))` as
    today (`ListingStatus` serializes to the same word).
  - `listing.verify`: `listing::verify_envelope(&input.envelope, now)`.
  - `listing.limits`: a function `limits(host, NoParams) ->
    Reply<PublicationLimits>` in `limits.rs`.
  - `availability.list`: read rows as `Value`, keep the rows whose
    `listing_id` matches, parse each into `SlotRow`, skip a row that does
    not parse (B32), filter and sort as today.
  - `availability.get`: parse the stored row into `SlotRow`; an error is
    `-32603` (B32).
  - `catalog.export`: `backup::export_signed_bundle` (D-A2b-12).
    `catalog.import`: takes `ImportInput`, returns `CatalogImportOutput`
    (the map is built as today).
- `directory` call site (Q6 (a)),
  [client_sources.rs:217-243](../../../../crates/roym_directory/src/app/client_sources.rs#L217):
  ```rust
  let got = call_sibling::<_, ListingKeyInput, Option<ListingGetOutput>>(
      host, services::CATALOG, "listing.get", &ListingKeyInput { listing_id: listing_id.clone() },
  ).await;
  let listing = match got {
      Ok(Some(l)) => l,
      Ok(None) | Err(SiblingError::Refused(_)) =>
          return Response::invalid_params("no such listing on this installation"),
      Err(SiblingError::Transport(e) | SiblingError::BadReply(e)) =>
          return Response::internal_error(e),
  };
  ```
  Then read `listing.envelope` in place of `result.get("envelope")`. The
  rest of `publish_to_source` does not change (it stays a `&Request`
  handler until A2c).
- Hub (Q3 (a)): `crates/roym_web/ui/src/screens/listings.ts:535-550`. Add
  and export
  ```ts
  export function verifyParams(pasted: string): { envelope: string }
  ```
  that returns the trimmed text. The click handler keeps the `JSON.parse`
  check (only to show "That is not valid JSON.") and sends
  `verifyParams(area.value)`. New `listings.test.ts` (vitest): an object
  is never sent, the text is sent trimmed.
- Parity scenario 40
  ([catalog.rs:71-84](../../../../crates/roym_web/tests/dual_build_parity/catalog.rs#L71)):
  keep the name, the request and `assert_eq!(w, n)`. Replace the
  `contains("listing params")` assertion by `w["error"]["data"]["field"]
  == "payment.amount_minor"`. The message text is no longer about "listing
  params", so the comment of the assertion changes too.
- Rebuild WASM: `mise run build:roym`.
- **Proof:**
  ```bash
  cargo clippy -q --workspace --all-targets --all-features
  cargo nextest run -p syneroym-roym-catalog -p syneroym-roym-directory > target/a2b.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b.log
  cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2b-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b-par.log
  (cd crates/roym_web/ui && npm test) 2>&1 | tail -n 8
  grep -rn "params\.get\|req\.params" crates/roym_catalog/src
  ```
  The last command prints nothing. The shape files and all 222 parity
  scenarios pass.

### Step 5a — Convert `transaction`: deal records, sync, thread, backup

**Commit:** `refactor(roym-transaction): type deal record methods`.

- New `crates/roym_transaction/src/api.rs` with `service_methods!` listing,
  in this commit: the seven pings, the five `request.*`, the six
  `quote.*`, the four `agreement.*`, `booking.verify`, `payment.verify`,
  `fulfilment.verify`, `transaction.sync`, `.thread`, `.export`,
  `.import` and the two certificate verbs (31 rows).
- `app.rs` `invoke` (temporary, removed in Step 5b):
  ```rust
  if let Some(resp) = admit::require_internal(host).await { return resp; }
  if let Some(resp) = api::dispatch(host, &req).await { return resp; }
  match req.method.as_str() {
      // only the booking, payment and fulfilment arms not yet listed
      other => Response::method_not_found(other),
  }
  ```
  `handle_certificate_verb` is no longer called.
- `app.rs`: `ListParams` becomes `ListInput` (D-A2b-6), `AgreementRef`
  added. `SendState` and `BookingWriter` added (Q5 (a)).
  `send_card_and_file` returns `(String, SendState, Option<String>)`:
  `SendState::from(out.state)` on success, `SendState::NotSent` on every
  error, exactly where it writes `"not-sent"` today. `verify_verb` and
  `RecordKind` are deleted (D-A2b-7).
- **Callers in the Step 5b files change their types only** in this commit,
  so the crate builds. Their `json!` answers stay until Step 5b.
  - `send_card_and_file` is also called in `payment_ops.rs:123` and `:618`,
    `payment_ops/fence.rs:112` and `:244`, `fulfilment_ops.rs:133` and
    `:410`, and `progress.rs:54`. Where the state goes into a `json!`, it
    is a `SendState` now (it serializes to the same word). `progress.rs`
    ignores the state and needs no other change.
    `record_and_send_ack` (`payment_ops.rs:596`) returns
    `Result<(String, SendState), Response>`.
  - The eight `booking_view` calls in `booking_ops.rs` (`:282`, `:290`,
    `:300`, `:308`, `:342`, `:386`, `:510`, `:544`) pass
    `Some(BookingWriter::Mine)` or `Some(BookingWriter::Peer)` in place of
    `Some("self")` or `Some("peer")`, or `None`.
- Handlers in `request_ops.rs`, `quote_ops.rs`, `agreement_ops.rs`,
  `sync.rs`, `thread.rs`, `backup.rs` change to the typed signature. The
  structs of section 5 are added beside them. Rules:
  - Keep the order of the checks that run after the parse, and every other
    error text. In `request.set`, `quote.set`, `agreement.accept` and
    `quote.decline` the owner or certificate check now runs after the
    parse (B23); the check itself and its text do not change. In
    `quote.set` the terms are now part of the parse, so a wrong `terms`
    field is reported before the `expires_in_secs` range check and the
    request and revision checks (B23).
  - `quote.set`: `QuoteTermsInput::into_agreed_terms(expires_at_secs)`
    replaces `parse_quote_terms` (Q2 (a)). Delete `parse_quote_terms`.
    Add a unit test that turns a full `QuoteTermsInput` into
    `AgreedTerms` and compares the JSON of both minus
    `quote_expires_at_secs` (so a field added to `AgreedTerms` without the
    input fails the test). Put it in `crates/roym_transaction/src/app/quote_ops/tests.rs`
    with `#[cfg(test)] mod tests;` in `quote_ops.rs` (module layout:
    `quote_ops.rs` + `quote_ops/`).
  - `validate_and_apply_slot` (B33):
    ```rust
    let slot = match call_sibling::<_, SlotKeyInput, Option<SlotRow>>(
        host, services::CATALOG, "availability.get", &SlotKeyInput { slot_id: slot_id.to_string() },
    ).await {
        Ok(Some(s)) => s,
        Ok(None) | Err(SiblingError::Refused(_) | SiblingError::BadReply(_)) =>
            return Err(Response::invalid_params("no-such-slot")),
        Err(SiblingError::Transport(e)) => return Err(Response::internal_error(e)),
    };
    ```
    then compare `slot.listing_id`, `slot.start_secs`, `slot.end_secs`.
    Note: today an unreadable reply is `-32603`; `call_sibling` reports a
    reply that is not JSON as `BadReply`. A host never sends one, so
    `no-such-slot` for it is accepted (no test can make one).
  - `agreement.accept`: `already_accepted_response` returns the output
    struct with `booking: None`; the normal path has
    `booking: Some(prepare_author_booking(..)?)` (D-A2b-8).
  - `transaction.export`: `backup::export_signed_bundle`.
  - `transaction.thread`: `cursor` and `limit` as `u64` (D-A2b-6).
- `booking_ops.rs`: `booking_view` returns `BookingView` (D-A2b-10), so
  `agreement.accept` can hold it. The booking handlers that call it still
  wrap it with `serde_json::to_value` until Step 5b converts them. Its
  `writer` argument is `Option<BookingWriter>`.
- **Proof:** rebuild WASM, then:
  ```bash
  cargo clippy -q --workspace --all-targets --all-features
  cargo nextest run -p syneroym-roym-transaction > target/a2b.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b.log
  cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2b-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b-par.log
  ```

### Step 5b — Convert `transaction`: booking, payment, fulfilment

**Commit:** `refactor(roym-transaction): type booking and payment methods`.

- Add the remaining ten rows to `api.rs` (`booking.get`, `.list`,
  `.start`, `.cancel`, `.history`, `payment.request`, `.acknowledge`,
  `.get`, `fulfilment.sign`, `.get`; 41 rows in all). `invoke` becomes the
  final form: `require_internal`, `api::dispatch`,
  `Response::method_not_found(&req.method)`. Delete the fallback `match`.
- `booking_ops.rs`: the booking handlers return `BookingView` directly
  (no `to_value`). The three `AgreementParam` copies are deleted; use
  `AgreementRef`.
  `booking.list` compares `BookingState` values. `booking.history` builds
  `BookingHistoryStep` values (D-A2b-8, B39).
- `payment_ops.rs`, `payment_ops/fence.rs`, `fulfilment_ops.rs`: change
  the fences, the three `ensure_*_recorded` functions,
  `already_recorded_fulfilment_response` and
  `check_acknowledgement_version` to the signatures of D-A2b-9, and map
  F1 to F6 exactly as its table says. Keep where each early return happens,
  what it writes, and where the card is sent. The crash-recovery shape
  cases of Step 2 must stay green.
- **For the code reviewer:** read the F1 to F6 mappings line by line
  against the table in D-A2b-9, and each `ensure_*_recorded` against its
  old body. The crash-recovery cases cover F1, F3 and F5 once each; F2, F4
  and the in-flight errors have no test (section 7.3).
- `ledger.rs` `claim_seat` (B33):
  ```rust
  let slot_row = match call_sibling::<_, SlotKeyInput, Option<SlotRow>>(..).await {
      Ok(Some(s)) => s,
      Ok(None) | Err(SiblingError::Refused(_) | SiblingError::BadReply(_)) =>
          return Ok(Claim::NoSeat(ConflictReason::SlotUnavailable)),
      Err(SiblingError::Transport(e)) => return Err(e),
  };
  ```
  `slot_row.capacity` replaces the `.get("capacity")` read. Keep the
  `capacity == 0` check and the `MAX_SLOT_CAPACITY` cap.
- Delete `sibling_call` and `catalog_call` from `app.rs` (D-A2b-11).
- Rebuild WASM.
- **Proof:** the commands of Step 5a, and then:
  ```bash
  cargo nextest run -p syneroym-substrate -E 'binary(~roym)' > target/a2b-e2e.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b-e2e.log
  grep -rn "params\.get\|req\.params\|Response::ok\|sibling_call\|catalog_call" crates/roym_transaction/src crates/roym_catalog/src | grep -v bindings.rs
  ```
  Check in `target/a2b-e2e.log` that the substrate `roym_*` tests ran and
  were not skipped (they return early when `roym_artifacts_present()` is
  false). The grep prints nothing.

### Step 6 — `roymctl`

**Commit:** `fix(roymctl): drop the booking list agreement option`.

- `apps/roymctl/src/commands/roym/booking.rs`: remove `agreement` from
  `BookingCommands::List` (the field at `:30`), from the match pattern
  (`:164`), and the `if let Some(a) = agreement` block that sends it
  (`:172-174`) (D-A2b-17). The `--help` text of `roymctl roym transaction
  booking list` loses the option. No other `roymctl` change: it keeps
  sending raw JSON (D-A2a-13).
- The only `roymctl` CLI test that names `booking` is
  `apps/roymctl/tests/backup.rs:30`, which runs `roym transaction booking
  --help`. It does not name `--agreement`, so it still passes and no test
  changes.
- **Proof:** `cargo nextest run -p roymctl` passes.

### Step 7 — The check test and the error tests

**Commit:** `test(roym-web): check every catalog and transaction method`.

- `crates/roym_web/tests/method_contract/contract.rs`:
  - `every_catalog_method_has_typed_input_and_output` and
    `every_transaction_method_has_typed_input_and_output`: call
    `assert_service_table(services::CATALOG, &catalog_api::method_table(),
    &[], &Path::new(env!("CARGO_MANIFEST_DIR")).join("../roym_catalog/src"))`
    and the same for transaction (`../roym_transaction/src`). The path is
    built from `CARGO_MANIFEST_DIR` (A2a R1). Assert the row counts: 17
    and 41.
  - Extend `every_listed_method_is_dispatched` and
    `every_listed_method_is_refused_off_the_node_before_parsing` to the two
    new tables. If the four loops then look the same, turn them into one
    loop over `[(services::PROFILE, profile_api::method_table()), ..]`
    (four tables).
  - Error tests, each sent to `h.wasm_http` and `h.native_http` with
    `Some(caller())`; assert `wasm_res == native_res`, then the code and
    `data.field`:
    - `listing.get` with `listing_id: 5` gives `data.field ==
      "listing_id"`, text contains `invalid type`.
    - `availability.set` with a slot that has `capacity: "two"` gives
      `data.field == "slots[0].capacity"` (pin the real path text once in
      the test; `serde_path_to_error` writes the index as `[0]`).
    - `availability.set` with an unknown slot key `colour` gives
      `data.field == "slots[0].colour"`.
    - `listing.verify` with `envelope` as an object gives `data.field ==
      "envelope"` (B28).
    - `request.list` with an unknown key `agreement` gives `data.field ==
      "agreement"`.
    - `booking.list` with `agreement` gives `data.field == "agreement"`
      (the old `roymctl` call, B26).
    - `quote.set` with `terms.quote_expires_at_secs` gives `data.field ==
      "terms.quote_expires_at_secs"` (B36). Use a valid request so the
      parse is the only error.
    - `booking.list` with `state: "nonsense"` gives `data.field ==
      "state"` (B31).
    - `request.list` with `params` absent works (B34).
    - `request.set` with `area: {"kind": "circle", "lat_e6": 0,
      "lon_e6": 0, "radius_m": "x"}` gives `-32602` with `data.field ==
      "area"` (a tagged block: the path stops at the block, Q1). Pin the
      real text, so a later change of the path is seen.
- `contract.rs` is 257 lines. If the new tests take it past about 600
  lines, put the catalog and transaction error tests in a new
  `method_contract/contract_deals.rs` (declared with `#[path]`).
- **Proof:** `cargo nextest run -p syneroym-roym-web --test
  method_contract` passes. Once, by hand, add a hand-written
  `"listing.get" =>` arm to `crates/roym_catalog/src/app.rs` and see the
  catalog table test fail; remove it. Say so in the PR (A2a R1 showed why).

### Step 8 — Docs and close-out

**Commit:** `docs: describe typed catalog and transaction methods`.

- Section 9 lists the files. Section 8 lists the backlog rows.
- No planning ids in names or comments of the new tests and shape files.

### Step 9 — Finish

- Import clean-up pass over every edited Rust file (types by `use`,
  functions by their parent module, no paths with more than one `::`
  inline; A2a R9).
- `cargo +nightly-2026-04-06 fmt --all`.
- Ratchets: `cargo xtask check-file-lengths` (record a lower value for
  `dual_build_parity/transaction.rs` if it shrank; it is on the list at
  846), `cargo xtask check-duplication` (the three `AgreementParam` copies,
  the three `setup_peer_request` copies and the inline one in `bundles.rs`,
  the two `setup_active_agreement` copies, the seven verify handlers and
  `sibling_call` go away; record the new value if it improved).
- `cargo xtask check-module-layout`, `cargo xtask check-dep-versions`,
  `cargo shear`.
- The `cargo dupes` recipe from AGENTS.md over the changed files.
- `mise run verify` as a background job, no `--skip`, every gate green.
- Open the PR with the CLA line and the attribution line. The PR text lists
  the commits, the method tables, the audit table (section 4), the manual
  proof of Step 7, and "calls with a wrong or unknown field now fail". It
  asks the code reviewer to read the fence mappings F1 to F6 (D-A2b-9)
  line by line.

## 7. Tests

### 7.1 After every commit

```bash
cargo clippy -q --workspace --all-targets --all-features
cargo nextest run -p syneroym-roym-core -p syneroym-roym-catalog -p syneroym-roym-transaction > target/a2b.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b.log
cargo nextest run -p syneroym-roym-web --test method_contract --test dual_build_parity > target/a2b-par.log 2>&1; echo "exit=$?"; tail -n 5 target/a2b-par.log
```

From Step 4 on, also the Hub unit tests (`npm test` in
`crates/roym_web/ui`). After Step 5b, the substrate e2e of Step 5b. After
any `roym_*` code change, rebuild with `mise run build:roym` first.

### 7.2 What each test proves

1. **Shared structs (Step 3, Step 5a).** `SlotRow` and `ListingGetOutput`
   read and write the keys the catalog writes. `VerifyInput` refuses an
   object. `QuoteTermsInput` has every field of `AgreedTerms` but the
   expiry.
2. **OF-15 (Step 7).** Both tables have their row counts, typed input and
   output, no `Value` as the outer type, a route to the right service, a
   probe that refuses unknown fields, and no hand-written arm or
   `req.method` use in the service source. A name not listed answers
   `-32601`.
3. **OF-14 (Step 7).** A wrong type at the top level, a wrong type one array
   level down, and unknown keys at both levels give `-32602` with
   `data.field`, equal in both builds.
4. **OF-18 (Step 7).** The probe of the check for all 58 rows, plus the
   off-node test: every method is refused with `-32013` before the parse.
5. **No behaviour change.**
   - The shape files, recorded from the old code in Step 2 and passing at
     every later step. They record the exact words of `state`, `role`,
     `pair`, `next`, `writer`, `track`, `conflict`, `event` and the other
     keys of D-A2a-14 and D-A2b-13, every item shape of every array, and
     the null or absent state of every key. Both answers of
     `agreement.accept`, of the already-recorded methods and of
     `booking.history` have their own file. The three crash-recovery cases
     prove that F1, F3 and F5 still give the normal answer with the fence
     half's `record_id`.
   - The parity harness: 24 modules, 222 scenarios, the same two-build
     equality. One assertion changes, in scenario 40 (B24).
   - The substrate e2e tests for listings, deals, bookings, restore and
     trust, which send raw JSON and read results by key.
   - The Hub unit tests and the Playwright spec `roym-hub.spec.ts`.
   - The greps of Step 4 and Step 5b print nothing.
6. **The Hub fix (Step 4).** `listings.test.ts`.

### 7.3 Not tested

- A wrong type for every field of every method. The mechanism is one
  function, tested in A2a, and the check probes every row.
- An unknown key inside a record type (Q1 (a)): it is ignored by design.
- A malformed slot row from a hand-made bundle (B32, B33). The mappings are
  small `match` arms; a test would need a hand-made signed bundle.
- A reply from `catalog` that is not JSON (B33): the host never sends one.
- The fence sites F2 and F4, and the `*-in-flight` errors (section 3.3).
  They run only when a fence row exists but cannot be read or holds no
  half, and this build always writes the half with the fence row. The
  crash-recovery path (F1, F3, F5) is tested once per method by the shape
  cases of Step 2, but not its `-32603` branch (a row read or write that
  fails). The code reviewer reads F1 to F6 line by line (Step 5b).

### 7.4 Size risks

- `payment_ops.rs` is 630 lines, `app.rs` 622, `backup.rs` 672,
  `quote_ops.rs` 578, `booking_ops.rs` 558. Output structs add lines;
  deleted parse code and the deleted `match` and helpers remove some. If a
  file would pass 800, put its structs in a sibling module
  (`payment_ops/types.rs` with `mod types;`, module-layout rule).
- `shapes.rs` (495 lines) does not grow much: the new cases are in two new
  files.
- `dual_build_parity/transaction.rs` is on the oversized list (846). Do not
  make it longer.
- `service_methods!` with 41 rows. `clippy::too_many_lines` did not fire
  for 24 rows in A2a; the expanded `dispatch` comes from a macro defined in
  another crate, which clippy skips. If it fires anyway, do not raise the
  limit. Split the table: two sub-modules `api/deals.rs` and
  `api/booking.rs`, each with its own `service_methods!`, and `api.rs`
  writes `method_table()` as the two `method_rows()` joined and `dispatch`
  as the first `Some` of the two. Record it as a deviation.

### 7.5 How A2b avoids the kinds of findings A2a had

| A2a finding | The risk in A2b | What this plan does |
| --- | --- | --- |
| P1, P11 (field name for missing and duplicate fields) | none, the parser is reused | Step 7 pins one nested array path. |
| P3 (shape holes) | new closed words; methods with two shapes | D-A2b-13; a shape case for every shape (Step 2). |
| P4 (sibling error handling) | three more sites | B33 and D-A2b-11 give each mapping as code. |
| P6, P7 (`null` vs absent) | many null keys | Section 3.3 lists them; D-A2b-8 for the tri-state. |
| P9 (array params) | none, the parser is reused | — |
| P12, P19 (test binary modules) | new modules | Step 2 names each `#[path]` line. |
| P14 (d), R1 (a check that silently does nothing) | stale WASM; the scan path | Rebuild before every parity run; `CARGO_MANIFEST_DIR` paths; the manual proof of Step 7; the e2e log check. |
| P17, B22 (stored rows read into structs) | slot rows | B32. |
| R2, R3 (change doc decisions and deviations mixed) | new decisions | Section 9: decisions D-A2b-1 to D-A2b-17 with the same numbers; planned wire changes and real deviations in their two lists. |
| R6 (nested types not checked) | record types in inputs | Q1, said in the developer guide and the backlog. |
| R8 (living docs say more than is true) | `directory` and `web` stay untyped | Section 9 names the four typed services exactly. |
| R9 (inline paths) | many edits | Step 9 import pass. |
| R10 (numbered comments) | none | No comment numbers a list from this plan. |

## 8. Deferred backlog

All rows are in [deferred-backlog.md](../../deferred-backlog.md).

- **Narrow the §10 row "Roym's sibling boundary is untyped JSON".** New
  text: `profile`, `conversation`, `catalog` and `transaction` take and
  return typed structs, and their sibling calls use them. `directory` and
  `web` are still on loose JSON until A2c. The rest of the row (no WIT
  contract between services, the untyped `status` export) stays.
- **Resolve the §10 row "`transaction` still has its own sibling-call
  helper".** `sibling_call` and `catalog_call` are deleted, and
  `directory`'s `listing.get` uses `call_sibling` (Q6 (a)). Move the row to
  "Recently resolved" with the PR. If the owner picks Q6 (b), narrow the
  row to `directory` instead, target A2c.
- **New row, §10: "Unknown keys inside record types in method inputs are
  ignored".** `listing.set`'s seven blocks, `request.set`'s `area` and
  `window`, and the nested types of `quote.set`'s terms ignore an unknown
  key one level down; the key is never signed. Reason: the same types parse
  records from other nodes, which must survive a newer sender (Q1 (a)).
  Target: TBD, with G5 (cross-version compatibility). Source:
  `crates/roym_catalog/src/app/listing_ops.rs`.
- **New row, §12 (Roym Transaction): "`roymctl` quote with `--slot` is
  always refused".** `roymctl roym transaction quote` has no option for
  `listing_id`, so a quote with a slot always gets `slot requires
  listing_id` (B40). Reason: a `roymctl` bug found while checking callers,
  not part of typing. Target: TBD (A7's client guide is a natural place).
  Source: `apps/roymctl/src/commands/roym/transaction.rs:280`.
- **Append to the §10 row "The Roym API contract is not published":** the
  `catalog` and `transaction` tables can also feed a schema generator.
- **Resolved in code, no row needed:** the Hub's object `envelope` for
  `listing.verify` (B28), and `roymctl roym transaction booking list
  --agreement` (B26).
  Both were bugs, not deferrals.
- No new open `TODO` or `FIXME` marker is added.

## 9. Living docs to update at close-out

Check each by asking: is a statement now false or incomplete? Do not cite
change docs, milestone or slice ids in these files. Link ADRs only.

- [docs/developer-guide.md](../../../developer-guide.md) (around line
  1687): "Today `profile` and `conversation` have a method table" becomes
  "`profile`, `conversation`, `catalog` and `transaction`"; the other
  services (`directory`, `web`) still read their params by hand. In "Add a
  Roym method" step 1, add: "a record type inside an input (for example a
  listing's `payment` block) has no `deny_unknown_fields`, because it also
  parses signed records from other nodes; an unknown key inside it is
  ignored and not signed". In "Bad input", add: "a wrong type inside a
  nested block is refused with `-32602`; `data.field` names the deepest
  field it can, which for a tagged block such as `area` is the block
  itself". Also add: "send a signed envelope to
  a `*.verify` method as text, not as a JSON object".
- [docs/system-architecture.md](../../../system-architecture.md) (around
  line 1143): the same list of four services.
- [docs/planning/traceability-matrix.md](../../traceability-matrix.md) rows
  `[ROY-TYP]`, `[ROY-UNK]`, `[ROY-CHK]` (lines 91-93): "(profile and
  conversation)" becomes "(profile, conversation, catalog and
  transaction)", and the evidence names the two new `api.rs` files.
- [docs/roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md):
  only if it states how bad input is answered (A2a found no `-32602` text
  there; grep again).
- Change doc [roym-open-foundation](../../changes/roym-open-foundation/change.md):
  - **Tasks:** under "Typed input and output for every Roym method", add
    "- [x] catalog and transaction". The parent line stays open.
  - **Decisions:** add D-A2b-1 to D-A2b-17 with the same numbers and
    meaning as section 5, and one line with the owner's answers to Q1 to Q6
    ("Q1–Q6: option (a), 2026-10-30").
  - **Deviations:** under "Planned wire changes", add B23, B24 (with
    scenario 40), B26 (with `roymctl`), B28 (with the Hub), B29, B30, B31,
    B32, B34, B35, B36. Under "Different from the plan", add only what the
    implementation really did differently (for example a split table,
    7.4).
  - **`living-docs-touched`:** add any file above not yet listed.
  - `status` stays `draft`.
- [task.md](task.md): add the A2b result line under "A2".
- [status.md](status.md): row A2b. Later steps record the PR, the
  shape-file count, the parity count (24 and 222) and the
  `method_contract` result.

## 10. Exit criteria

A2b is done when all of these are true:

1. The owner has answered Q1 to Q6, and section 5 matches the answers.
2. Every one of the 17 catalog and 41 transaction methods has an input
   struct and an output struct (OF-14). No `req.params` is left in
   `roym_catalog` or `roym_transaction`, and no `Response::ok` in their
   handlers.
3. `method_contract` passes: the catalog table has 17 rows and the
   transaction table 41, typed, local-only, with refusing inputs and no
   hand-written arm (OF-15, OF-18).
4. A wrong type and an unknown field, at the top level and one array level
   down, give `-32602` with `data.field`, equal in both builds (OF-14).
5. `method_contract shapes` passes with the files recorded from the
   unchanged code, and the 63 A2a files are unchanged.
6. The parity harness has 24 modules and 222 scenarios and passes. Only
   scenario 40's last assertion changed.
7. `transaction` and `directory` call `catalog` through typed structs, and
   `sibling_call` and `catalog_call` are gone.
8. The Hub sends the `listing.verify` envelope as text and has a unit test
   for it. `roymctl roym transaction booking list` has no `--agreement`.
9. The change doc, the developer guide, the architecture doc, the
   traceability matrix, task.md, status.md and the backlog are updated as
   in sections 8 and 9.
10. `mise run verify` passes with no `--skip`.
11. One PR. A2c starts after it is merged.

## 11. What A2b does not do

- No typed methods for `directory` or `web` (A2c). `directory` keeps
  `handle_certificate_verb` and `backup::export_signed`; its
  `publish_to_source` stays a `&Request` handler, with only its
  `listing.get` call typed (Q6 (a)).
- No wire-reachable method. Both wire lists are empty.
- No change to record types, to stored rows, to signed records or to
  record verification. No storage schema version changes.
- No schema is generated or published. No client package.
- No WIT contract between services. The `status` export stays untyped.
- No method renames and no reshaping of results. `agreement.accept`,
  `booking.history` and the already-recorded answers keep their two shapes.
- No new filter or field on any method (Q4 (a)).
- No fix for `roymctl`'s quote `--slot` (B40); a backlog row records it.
- No change to what a correct request does, or to the order of any check
  other than B23.
- No `roymctl` change other than Step 6. It keeps sending raw JSON.
- No new demo story.

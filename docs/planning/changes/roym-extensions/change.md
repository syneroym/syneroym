---
status: draft
demo:
  - crates/substrate/tests/e2e/demo/60-roym-step.spec.ts
living-docs-touched:
  - docs/system-requirements-spec.md
  - docs/system-architecture.md
  - docs/roym-integrated-experience-spec.md
  - docs/developer-guide.md
  - docs/demo/demo.md
  - docs/TERMINOLOGY.md
  - docs/planning/traceability-matrix.md
  - docs/planning/deferred-backlog.md
---
# Roym extensions

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change splits Roym into a fixed **deal core** and **Roym extensions**. The deal core is what every peer-to-peer deal shares: offerings, quotes, agreements, commitments, settlement, EEPs and reputation. A Roym extension adds the work steps of one kind of business: a slot booking for a salon, a document upload for a doctor or an accountant, location sharing for a taxi. Extensions plug into the deal core. They never change it.

Terms used here:

- **Roym extension:** a package that adds work steps, chat widgets or helper logic to Roym. It belongs to the Roym family of applications. It is not presented as a separate SynApp or SynSvc, even when part of it runs as a service.
- **Deal core:** the fixed part of Roym that every deal uses. See the [Value Exchange](../roym-value-exchange/change.md) change for the agreement and commitment model.
- **Commitment:** one item in an agreement: one party gives one resource to the other. See the Value Exchange change.
- **Commitment details:** a signed block inside a commitment that holds data defined by an extension (for example the chosen slot).
- **Widget:** a small interactive item in a chat, such as a poll, a form or a checklist. It is data, never code.
- **Step record:** a signed record of one work step done through an extension inside a deal (for example "documents uploaded", "driver arrived"). It always names an agreement.
- **Widget answer:** a signed record of one person's answer to a widget (for example a vote in a poll). It names the conversation and the widget card. It does not need an agreement, because a widget can be used in any chat.
- **Reservation:** a hold on a resource (for example a seat in a slot) that the resource owner's node takes while an agreement is being formed. It ends on its own after a short time unless the agreement forms.
- **Hub:** the Roym web UI. **Client:** any program that uses Roym's methods (see [Roym open foundation](../roym-open-foundation/change.md)).

## Problem

Roym today has one business flow built into its core. A listing has `BookingMode::Slots`. `catalog` has `availability.*` with a seat count per slot (up to `MAX_SLOT_CAPACITY` = 64, `crates/roym_core/src/booking.rs:20`). `transaction` claims seats and runs the booking track. This fits a salon. It does not fit a doctor who needs reports before a visit, a taxi that shares its location, or a tutor who sends homework. If each new business adds its own flow to the core, the core grows without limit. Strangers then cannot compare deals, and reputation loses meaning.

The chat has the same limit. The Hub has seven fixed card templates (`crates/roym_core/src/card.rs:10`). There is no poll, no form, no checklist.

## Scope and non-goals

- In scope:
  - The rule that splits the deal core from extensions.
  - Commitment details: a signed, schema-named block in each commitment.
  - Step records: a generic signed record for extension work steps inside a deal.
  - Declarative widgets with semantic metadata, one generic card type that carries them, and a separate widget-answer record for answers in any chat.
  - Extension services: an optional service part, which acts on events, and which the counterparty of a deal can call with access scoped to that deal.
  - Reserve and release hooks, so that the deal core can ask an extension to hold a resource (a slot) before an agreement forms.
  - Two first extensions: **scheduling** (today's slots, moved out of the core) and **one-shot** (pay and deliver, with no slot).
  - Room in the extension manifest for **insight extensions** (read-only helpers on the participant's node). They are built later.
- Not in scope:
  - Extension code from third parties in v1. The format allows it. Only extensions that ship with Roym run in M8.
  - UI code from an extension (for example an iframe). Widgets are data only.
  - A marketplace or store for extensions.
  - Insight extensions themselves. They come with analytics, after reputation.

## The rule

1. **The deal core owns the lifecycle.** An agreement and its commitments move through fixed states. An extension cannot add, remove or rename a state.
2. **Reputation reads only what the deal core records:** whether each commitment settled, failed or ended unconfirmed. It does not read extension state. Feedback can still point at one step record, as text or a rating (see the Value Exchange change, opinions).
3. **Extensions add data and steps, signed and stored the same generic way for every extension.** Any client can show them, even a client that does not know the extension.
4. **No code from a sender ever runs on a receiver.** This is today's card rule. It holds for widgets.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **Deal core and details** | | |
| EX-1 | A commitment carries an optional `details` block: an extension id, a schema id with a version, and data. The block is inside the signed commitment. | `crates/roym_core/src/deal/tests/extension.rs` (`one_shot_goods_commitment_is_checked`, `a_slot_commitment_with_bad_fields_is_refused`, `reveal_strict_refuses_a_slot_id_that_does_not_match_its_window`), `crates/roym_core/src/extension/schema/tests.rs` (one case per field type), `crates/roym_transaction/src/app/tests/extension.rs` (`quote_set_refuses_details_that_fail_the_schema`) |
| EX-2 | Each party's node checks `details` against its schema before it signs, when it has the extension. A party without the extension sees the details as labelled plain fields marked "not checked on this device", and may still sign. Once the agreement forms, the agreement writer's check is the binding one. A later node that disagrees shows a warning. It never treats the signatures as invalid. | `crates/roym_transaction/src/app/tests/extension.rs` (`quote_set_refuses_an_unknown_extension`, `counterparty_files_a_quote_with_an_unknown_extension`, `accept_refuses_when_this_node_gives_an_unknown_extension`), `crates/roym_core/src/presentation/tests/extension.rs` (`an_unknown_extension_with_visible_details_is_verified_but_not_checked`), `crates/roym_core/src/deal/tests/extension.rs` (`reveal_strict_reveals_a_quote_with_an_unknown_extension`), `crates/roym_web/ui/src/cards/fields.test.ts` ("fieldView of an unknown extension"), `crates/roym_web/ui/src/screens/messages_translate.test.ts` ("Accept refused for an extension this device does not have") |
| EX-3 | `details` may also carry free extra fields (label and value text), so a party can add terms that no schema has. | `crates/roym_core/src/deal/tests/extension.rs` (`extra_limits_hold_for_every_extension`), `crates/roym_web/ui/src/cards/fields.test.ts` ("fieldView extra lines") |
| EX-4 | The deal core's states are the same for every extension. A test runs one agreement with scheduling and one with one-shot and checks that both pass through the same states. | |
| **Reserve and release** | | |
| EX-5 | When an agreement is accepted, the deal core calls the reserve hook of each extension named in a commitment. If one reserve fails, the agreement does not form, and every reserve that succeeded is released. | |
| EX-6 | A reserve is claimed under the resource owner's single-writer fence, also when the owner is not the quote author. This is today's slot rule (`D-C8-2` in the integrated spec), made generic. | |
| EX-20 | A reservation has an expiry set by the resource owner's node (a first value: 15 minutes). If the owner has not received the formed agreement by then, its node releases the reservation on its own. No other node is needed for this. | |
| EX-21 | Reserve and release are idempotent, keyed by (agreement id, commitment id). A repeated reserve returns the same reservation. A release of an unknown or already released reservation succeeds and changes nothing. | |
| EX-22 | A reservation becomes firm only when the resource owner's node receives the formed agreement and confirms it. A confirm after the expiry is refused if the resource was given to someone else, and the agreement writer then marks that commitment failed with a named reason. | |
| **Step records** | | |
| EX-7 | An extension step is a signed `step` record. It names the agreement, the commitment, the extension, the step type and its data. Either party can verify it. | `crates/roym_core/src/step/tests.rs` (`verify_accepts_a_good_envelope_and_refuses_a_tampered_or_wrong_type_one`, `data_at_the_limit_passes_and_one_byte_more_fails`), `crates/roym_transaction/src/app/tests/step.rs` (`the_giver_adds_a_step_and_it_is_listed`, `verify_shows_the_data_with_its_disclosure`), `crates/roym_transaction/src/app/tests/step_filing.rs` (`a_step_from_the_giver_is_filed_and_listed`, `an_agreement_holds_at_most_the_step_limit`), `crates/roym_transaction/src/app/tests/step_backup.rs` (`export_and_import_keep_a_step_with_its_data`), `crates/substrate/tests/roym_step_e2e.rs` (`the_giver_records_a_step_and_the_other_node_lists_it`), `crates/roym_web/tests/dual_build_parity/step.rs` (`scenario_246_step_add_list_and_verify_parity`) |
| EX-8 | A step record shows in the deal's conversation as one generic card type. A client that does not know the extension shows the step's fields as labelled text. | `crates/roym_web/ui/src/cards/templates/templates.test.ts` ("renderStep"), `crates/roym_web/ui/src/i18n/hindi-screens.test.ts` ("draws a step card with its name in Hindi, and an unknown extension's step as it came"), `crates/roym_transaction/src/app/tests/step_filing.rs` (`a_step_of_an_unknown_extension_is_filed_and_not_checked`), `crates/substrate/tests/roym_step_e2e.rs`, and the demo story `crates/substrate/tests/e2e/demo/60-roym-step.spec.ts` |
| EX-9 | An opinion can point at one step record. | |
| **Widgets** | | |
| EX-10 | A widget is a declarative definition: a widget type, a semantic tag (for example `poll`, `form`, `checklist`, `location-share`, `document-request`), typed fields, and message ids for its labels. It carries no markup and no script. | |
| EX-11 | A widget card carries its definition inline when the definition is up to a size limit (a first value: 8 KB). Only a larger definition travels as a content hash. The receiver then fetches it from the sender's node and keeps it. If the sender is offline and the definition is not cached, the card shows "widget not available yet" and tries again later. The receiver renders a definition from field types alone. | |
| EX-12 | An answer to a widget is a signed `widget-answer` record. It names the conversation and the widget card, not an agreement. A poll shows the count of answers per option to the people in the conversation. | |
| EX-13 | Widget labels come from the string catalog (see the [multilingual](../roym-multilingual/change.md) change), so a widget shows in the reader's language. | |
| **Extension services** | | |
| EX-14 | An extension may include a service. It ships in the Roym SynApp and is deployed with it. Users see it as part of the extension. | |
| EX-15 | An extension service receives deal and conversation events through the event feed (see [Roym open foundation](../roym-open-foundation/change.md)). It does not poll. | |
| EX-16 | The counterparty of a deal can call an extension service's declared methods only with a grant that names that agreement or conversation, that counterparty, and an expiry. Every other call is refused. The grant ends when the agreement closes, unless the owner extends it. | |
| **First extensions** | | |
| EX-17 | Scheduling: slots, seat capacity, slot claim and the booking track run as the scheduling extension. `catalog` and `transaction` hold no slot code. Today's booking tests pass through the extension. | |
| EX-18 | One-shot: an agreement for goods or a service with no slot settles with a payment acknowledgement and a fulfilment receipt, through the same deal core. A one-shot offering can be agreed as listed (agreements AG-44). It has nothing to reserve, so it is `instant` only when the owner marked it `not_limited` (AG-45). | `crates/roym_transaction/src/app/tests/extension.rs` (`one_shot_commitment_settles_like_today`). The agree-as-listed half is built in [as_listed.rs](../../../../crates/roym_core/src/as_listed.rs) and tested in [as_listed/tests.rs](../../../../crates/roym_core/src/as_listed/tests.rs); `listing.set` refuses `instant` on a one-shot offering that is not `not_limited`. |
| **Insight extensions** | | |
| EX-19 | The manifest has an `insight` kind. Its fields say what it may read. It may not sign, send, or reach the network. Only the manifest field is built in M8. | `crates/roym_core/src/extension/tests.rs` (`insight_manifest_loads_and_cannot_be_named`, `insight_and_workflow_keys_do_not_mix`), `crates/roym_core/src/deal/tests/extension.rs` (`a_commitment_naming_an_insight_is_not_checked`) |

## Design

### Extension manifest

Each extension has a manifest, roughly:

```toml
id = "scheduling"
version = "1"
kind = "workflow"            # or "insight" (later)

[[details]]                  # commitment detail schemas
schema = "scheduling/slot@1"

[[steps]]                    # step types and their fields
type = "scheduling/started"

[[widgets]]
type = "scheduling/pick-slot"
semantic = "form"

[service]                    # optional
methods = ["slots.list"]     # what a counterparty may call, with a deal grant
events = ["agreement", "commitment"]
hooks = ["reserve", "release"]
```

The exact format is set in the M08C plan. Schemas use a small field-type set: text, long text, number, money, date-time, duration, choice, location, file pointer, record pointer. Each field has a label message id.

### Commitment details

**Which check binds.** A commitment is checked against its schema only where the extension is installed. Two nodes can therefore disagree about one signed record. The rule (EX-2): each party checks before it signs, if it can. After the agreement forms, the agreement writer's check binds the deal. Another node's disagreement is shown as a warning, never as a broken signature.

```text
details: {
  extension: "scheduling",
  schema:    "scheduling/slot@1",
  data:      { slot_id: "...", start_secs: ..., end_secs: ..., seats: 1 },
  extra:     [ { label: "Bring", value: "old reports" } ]   // free, optional
}
```

The block is part of the commitment's signed bytes. Selective disclosure (Value Exchange change) covers it like other fields.

### Step records

A new record type `step`, signed by its author, with: agreement id, commitment id, extension id, step type, data, and an optional pointer to an earlier step. A step always belongs to a deal.

A second new record type, `widget-answer`, holds an answer to a widget: conversation id, widget card id, the answer data, signed by the person who answers. It needs no deal. A poll in a family chat uses it.

These travel as three new generic card types: `step`, `widget` and `widget-answer`. The fixed card table grows from seven types to ten. It does not grow with each extension.

### Widgets

A widget card carries a declarative definition, inline when it is small (EX-11). The sender may be offline when the receiver reads the card, so a fetch by hash is only for large definitions. The Hub has one generic widget renderer. It may have a richer renderer for a known semantic tag. For example, it can show a `poll` as bars. Every client can fall back to the plain field view.

Rejected: an iframe or other sandbox that runs extension UI. It is a large attack surface, and it breaks "any client can render it".

### Extension services and the deal grant

An extension service is one more service in `roym.toml`. It uses the same `roym_core` library and the same admission rules as other Roym services. Its methods are local-only by default. The counterparty reaches a declared method only with a **deal grant**: a capability token that the owner's node issues when the agreement forms. It names the agreement, the counterparty's DID, the methods and an expiry. It uses the existing UCAN-style capability tokens (`crates/ucan`). This is a new kind of access from another node, so it gets an ADR before it is built.

Example: a doctor's `documents` extension. The patient's node uploads reports to the doctor's node through `documents.upload`, with the grant of their agreement. The doctor's node stores them and writes a step record "documents received".

### Reserve and release hooks

Today a slot claim is one write under one fence, on one node. With extensions, a reserve can run on several nodes (a barter with a booking on each side), and maybe in a separate extension service. So a crash between "reserved" and "agreement formed" must not leak holds.

1. **Reserve.** When a party accepts, the agreement writer calls `reserve` on each extension named in a commitment, on the resource owner's node. The owner's node takes a **reservation** under its own fence, with an expiry (EX-20). The call is idempotent, keyed by (agreement id, commitment id) (EX-21).
2. **Form.** When every reserve succeeded, the writer forms the agreement and sends it to each resource owner.
3. **Confirm.** The owner's node confirms the reservation when it receives the formed agreement. Only then is the resource given for good (EX-22).
4. **Release.** If any reserve fails, the writer calls `release` on the ones that succeeded. Release is idempotent.

**Who cleans up.** Each resource owner's node cleans up its own reservations. If the writer crashes, or a release is lost, the reservation simply expires. No node depends on another node to free its resources. A confirm that arrives after the expiry succeeds if the resource is still free. Otherwise it is refused, and the writer marks that commitment failed with the reason.

This is the generic form of today's slot claim (VX-10 in the Value Exchange change).

### Scheduling and one-shot

- **Scheduling** takes `availability.*`, the seat claim and the booking track out of `catalog` and `transaction`. The listing's `BookingMode::Slots` becomes "this offering uses the scheduling extension".
- **One-shot** covers today's `Order` mode: pay and deliver, no slot.
- `Enquiry` mode becomes the deal core's normal enquiry and quote flow, with no extension.

### Insight extensions

The manifest reserves `kind = "insight"`, with a `reads` list (for example `event-log`, `records`, `exchange-aggregates`, never chats unless listed). An insight extension may not sign, send, or reach the network. It may show suggestions. The person acts on them. It is built with analytics, after reputation. A backlog row tracks it.

## Rejected options

- **A configurable state machine per offering.** Strangers could not compare states, and reputation would lose meaning. (Also rejected in the Value Exchange change.)
- **One card type per extension.** The fixed card table would grow with every extension, and a client would need a new template for each one.
- **Extension UI as code.** See [Widgets](#widgets).
- **Calling extensions "plugins".** "Plugin" suggests code loaded into the substrate. "Extension" says it extends the Roym workflow.

## Open questions

- **Extension discovery by a counterparty.** How a participant's node learns which extensions an offering uses before a deal. Recommendation: the offering lists its extensions by id and version. A client that lacks one shows the plain field view.
- **Reservation expiry (EX-20).** 15 minutes is a first value. If the resource owner's node sleeps right after it reserves (common on phones, M10), the reservation can expire before the formed agreement arrives. The commitment then fails after both parties signed, with a named reason. This is accepted, but the M08C plan sets the value, and decides whether the expiry should follow the quote's own expiry or a value per extension.
- **Third-party extension code.** When, and with what review or signing, a participant may install an extension that does not ship with Roym. Not in M8.
- **Grant scope for groups.** Whether a deal grant can name a group conversation. Recommendation: not in M8. Deals are between two parties.

## Tasks

- [ ] ADR: deal grants (counterparty access to an extension service, scoped to one deal).
- [ ] Write the M08C slice plans.
- [x] Manifest format and the extension registry inside Roym.
- [x] Commitment `details` in the deal core, and schema checks.
- [x] `step` record and card, the generic step view in the Hub.
- [ ] Widget definition (inline up to the size limit), `widget` and `widget-answer` records and cards, the generic renderer, the poll renderer.
- [x] Reserve, confirm and release hooks, with expiry, idempotency and owner-side cleanup.
- [ ] Extension services: event subscription, deal grants.
- [x] Move scheduling out of `catalog` and `transaction`.
- [x] Add one-shot.
- [x] The `insight` kind in the manifest only.

## Deviations

- **EX-15 and EX-16 are not built in M08C** (owner decision, 2026-11-15, [M08C task.md](../../milestones/M08C-deal-core-and-extensions/task.md), section 6, question 3). No M08C extension needs events or a deal grant: scheduling confirms and releases through local calls on its own node, and no counterparty calls it. Events to an extension service, deal grants and the deal-grant ADR come with the first extension that needs them. Backlog row: "Extension services cannot receive events or take calls from a counterparty".
- **EX-11 is built for inline definitions only** (owner decision, 2026-11-15, same task.md, question 4). A widget definition larger than 8 KB is refused. Fetch by hash from the sender's node would need a new wire-reachable method on `conversation`. Backlog row: "A widget definition larger than 8 KB is refused".
- **EX-1 and EX-3 (shape only) are built in M08C C1a, not C3a** (owner decision, 2026-11-16, [M08C slice C1 plan](../../milestones/M08C-deal-core-and-extensions/slice-c1-implementation-plan.md), question Q3). C1 must keep slot booking working, and the slot moves into `details.data`. So C1a adds a commitment's `extension` and `details` (with `extra`) in their final shape, and checks one case in code: extension `scheduling` version 1, schema `scheduling/slot@1`, data `{ listing_id, slot_id }`. Any other extension is refused. C3a replaces this fixed check with the manifest, the fixed registry and the schema check (EX-2), and adds one-shot; C3b moves the slot to the scheduling service.

- **EX-18's agree-as-listed half is built in M08C C3c2.** A one-shot listing can be agreed as listed. It has no reserve hook, so `listing.set` accepts `instant` on it only when the owner sets `not_limited`. `terms_from_listing` builds the same two commitments as a hand-written quote, and the automatic acceptance compares them field by field. The tests are `instant_needs_a_slot_offering_or_not_limited` and `instant_is_allowed_on_a_slot_offering_that_is_not_limited` in `crates/roym_core/src/listing/as_listed/tests.rs`.
- **EX-19 is built as the manifest field only** ([M08C task.md](../../milestones/M08C-deal-core-and-extensions/task.md), section 5). C3a's manifest format has `kind = "insight"` and its `reads` list, and a commitment or a listing cannot name an insight extension. No insight extension ships, and nothing enforces "may not sign, send, or reach the network" yet. Backlog row: "Analytics and insight extensions".
- **EX-2: a giver cannot accept a deal part whose extension its node does not know** (owner decision, 2026-11-20, [M08C slice C3a plan](../../milestones/M08C-deal-core-and-extensions/slice-c3a-implementation-plan.md), question Q9). EX-2 says a party without the extension "may still sign". That stays true for a receiver. When this node gives a commitment whose extension it does not know, `agreement.accept` refuses with `extension-unknown`, because the giver's node must run that extension's hooks and steps. The Hub asks the person to update Roym first.

**Rules and differences in C3a1 (2026-11-20, [slice plan](../../milestones/M08C-deal-core-and-extensions/slice-c3a-implementation-plan.md)).**

- **The manifest is JSON in `roym_core` (owner decision, question Q2).** The sketch under "Extension manifest" shows TOML and a string `version`. The manifests are JSON files in `crates/roym_core/extensions/`, and `version` is a number, as `ExtensionRef.version` is. `resource_kinds`, `hooks`, `checks` and `reads` are keys that the sketch does not show. `[[widgets]]` and `[service]` are not built: a manifest with a key that no code reads does not load. Widgets come with C4.
- **Eight field types, not ten (owner decision, question Q8).** `text`, `long-text`, `number`, `money`, `date-time`, `duration` and `choice` are built, with an eighth type, `code`, for identifiers such as a listing id. `location`, `file-pointer` and `record-pointer` are not built, and a manifest that names one does not load. Backlog row: "Extension field types `location`, `file-pointer` and `record-pointer` are not built".
- **A listing names its extension in place of a booking mode (owner decision, question Q3).** `BookingMode` is gone. A listing has an `extension` (`id` and `version`), or none for the plain enquiry flow. A listing that names `scheduling` must have booking terms (`booking-required`). A listing that names no extension may leave out `payment`: this is the old `enquiry` rule, and it now also holds for a listing with no booking block. `listing.set` refuses an extension this node does not know (`extension-unknown`). The directory filters by extension id, and cannot ask for "no extension". Backlog row: "The directory cannot search for listings with no extension".
- **`details` is checked against the manifest's schema, with one shape check for every extension.** The `extra` limits, the form of the extension id and of the schema name (`<id>/<name>@<version>`) hold for an unknown extension too. A schema name that does not start with the extension's id is `extension-shape`. The code `details-mismatch` is for a well-formed schema that the manifest does not list.
- **The list of manifest files is checked from the Rust side.** The slice plan asked for a Vitest test that reads `extension.rs`. Vite does not serve a `.rs` file, so `roym_core` has the test, as it has for the Hub's card registry: it reads `registry.ts` and compares the files, in order, with `MANIFESTS`. A Vitest test checks that the Hub holds every JSON file of the folder.
- **`listing.set` asks a small question.** `extension::is_known_workflow` reads only the header of each manifest. The catalog component would otherwise link all the schema code and grow by more than its size limit.
- **The Hub draws `details` under each commitment line.** The slice plan said once, outside the text-version switch. The view is drawn inside the card's commitment lines, and always with the quote's original language, so it never takes the tag of an author translation. Free text in `details` is not offered to machine translation. Backlog row: "Free text in `details` is not offered to machine translation".
- **The Hindi review has six messages in C3a1, not five.** `ext-scheduling-step-started` ships with the scheduling manifest, so the catalog test that checks every derived id needs it now. The Hub shows the step name in the step slice.

**Rules and differences in C3a2 (2026-11-21, [slice plan](../../milestones/M08C-deal-core-and-extensions/slice-c3a-implementation-plan.md)).**

- **The first step type, `scheduling/started`, ships in C3a2, not C3b (owner decision, question Q6).** The giver of the commitment records it with `step.add` or `roymctl roym step add`. It is a card, not a state: the commitment does not move to `in-progress`. The Hub has no button for it.
- **A step has no pointer to an earlier step.** The sketch under "Step records" has one. No step type in M08C needs it. Instead, a party records one step of a type once for each commitment (the key is the agreement, the commitment, the step type and the signer's role in that commitment). With `by: either`, each party may record it once. An agreement holds at most 64 step rows (`MAX_STEPS_PER_AGREEMENT`), and a step's `data` is at most 8 KiB (`MAX_STEP_DATA_BYTES`). Backlog row: "A step cannot point at an earlier step".
- **`data` is a hidden field**, like `details` (owner decision, question Q5). The record holds one `_sd` digest and the card carries the disclosure.
- **`step.add` refuses a commitment whose extension this node does not know with `extension-unknown`, not `step-type-unknown`.** A party cannot sign a step under an extension that this node cannot check.
- **`step.add` looks for a stored step before it checks `data` and the step count.** The plan checked them first. A repeat at the limit of 64 steps, or with other data, then answers with the stored record. It does not refuse an action that already happened.
- **A repeat of a step whose first send failed sends the card again.** The row keeps the send state. The plan only returned `already-recorded`. A node that stops during the send leaves the row `pending`, and no repeat sends the card. Backlog row: "A step card is not resent when the node stops during its send".
- **`step.list` also returns the signed `envelope` of each step**, so a client can call `step.verify` without another lookup. The plan did not list it.
- **A step's own fields have no derived label ids.** The plan derives the step's name only. The card draws a step field under its own name. No shipped step has fields. This needs a rule before the first step with fields ships.
- **The step card shows the signer's role** ("Giver" or "Receiver"), as the fulfilment card does. The plan said "you" or the other person's name, and the time. The message around the card already shows the direction and the time.
- **An import now also checks a `deal-decline` card.** The plan expected a missing re-verify arm. The test `a_decline_card_is_verified_after_export_and_import` showed that an imported decline card was marked not verified, because the import kept no history entry for a decline record either. Both are added.
- **The backup schema version of `transaction` is unchanged.** The new `steps` section is optional, and a bundle with no `steps` section imports as before. A development node that is older than this change ignores the section.
- **The demo story differs a little from the plan.** `quote --slot` needs the slot id, so the browser part reads it, with the window, from `availability.list` and the script derives nothing. The quote answers an enquiry, so Bob sends Alice one first. Alice's listing names no extension: only the slot matters for the quote. Story 50 and story 60 share `conversationWith` and `fetchAgreement` in `hub-helpers.ts`.
- **Hindi review.** One message is new in C3a2 for the owner to review: `ext-scheduling-step-started`. It already shipped with C3a1.

**Rules and differences in C3b1 (2026-11-21, [slice plan](../../milestones/M08C-deal-core-and-extensions/slice-c3b-implementation-plan.md)).**

- **The manifest names the service that answers its hooks (owner decision, question Q2).** A new key `service` is required when `hooks` is not empty and not allowed otherwise. It must name a service of `roym.toml` other than `web`. `transaction` finds the hook service from the manifest, so the deal core names no extension. The sketch under "Extension manifest" has no such key.
- **A fourth hook, `prepare` (owner decision, question Q3).** It is optional and only with the other three. `quote.set` and `quote.measure` call it for each commitment the author gives, so the slot checks (`no-such-slot`, `slot-not-in-listing`, `schedule-differs-from-slot`) and the window fill move out of `transaction`.
- **The slot methods keep their names (owner decision, question Q7).** `availability.set`, `.get`, `.list` and `.remove` have the same inputs and outputs. Only the service that answers them changes, from `catalog` to `scheduling`.
- **`confirm` never takes a seat (owner decision).** EX-22 says a confirm after the expiry succeeds if the resource is still free. A confirm finds the reservation that exists and confirms it, or is refused `reservation-lost`. After a release it is refused, also when the seat is free. This is narrower than EX-22. A reservation of this build never expires, so only the release case can happen now.
- **EX-5: each giver reserves on its own node.** The text says the agreement writer calls `reserve` on each extension "on the resource owner's node". Each giver's own `transaction` calls the hook of its own `scheduling`. A node never calls another node's extension service.
- **EX-21: a release also frees a confirmed reservation.** The agreements change says so ("Release after confirm"). A release of an unknown or released key succeeds with `released: false`.
- **The reservation never expires in this build.** `expires_at_secs` is empty and nothing lapses a reservation, as a seat hold never expired. The expiry (EX-20) and the lapse rules come in the next part of the slice.
- **No seat row is ever deleted or overwritten.** The data layer has a fence (`create`) and no compare-and-delete. A release that deletes two rows could free the seat of a newer booking if two releases of one reservation ran at the same time. So a seat is a row per round, a release adds a `free:` row, and the next booking takes the seat in the next round.
- **`scheduling` is enrolled for signing like the other five (owner decision, 2026-11-21).** The plan said `scheduling` signs nothing. But its `scheduling.export` signs the bundle as the person, with the service's own delegation certificate, like every other bundle. Without a certificate it could only answer `signing-not-enrolled`. So it has `signing-status` and `install-signing-certificate`, and it is in `roymctl`'s, the Hub's and the test harness's lists of services that sign.
- **The hooks have no route from `web`.** `assert_service_table` asks that every method of a service routes to it. A new `assert_service_table_with_unrouted` lets a service name the methods that only a sibling calls, and checks that they route nowhere. `scheduling` uses it for the four `reservation.*` methods.
- **A release error is not logged.** The plan asked for a log line. No Roym service has a way to log. The counterparty's release at the quote's expiry writes a `released:<agreement>` marker only when every release worked, so a failed one is tried again. The other release sites drop the error, as before. The release of an extension is idempotent, so a later repeat of the step is safe.
- **The `scheduling` import also refuses a changed `seat:` row.** The plan refuses an `rsv:` row that exists with other content (`reservation-differs`). A `seat:` row is checked the same way, because a bundle that changed a seat row would move a held seat to another reservation.
- **The step order of the plan was changed in two places.** `catalog` lost its slot code and `transaction` was bound to `scheduling` in the same step that added the service, because the router change (`availability.` now routes to `scheduling`) left the booking tests failing between the two steps. `transaction`'s edge to `catalog` is replaced by an edge to `scheduling` there.
- **The restore e2e test checks the live reservation in the `scheduling` bundle.** A second booking of the last seat after a restore needs a new conversation with the restored node. The test reads the exported `reservations` section and checks that each agreed agreement has a reservation that no `free:` row ends, in both import orders.
- **A changed Hindi message.** `backup-bundle-catalog` no longer says "and availability", because the catalog bundle holds listings only. English "Listings", Hindi "लिस्टिंग". The owner reviews it with the two new messages `backup-bundle-scheduling` and `backup-reservations-failed`.

**Rules and differences in C3b2 (2026-11-21, [slice plan](../../milestones/M08C-deal-core-and-extensions/slice-c3b-implementation-plan.md)).**

- **EX-20: a reservation expires lazily (owner decision, question Q4).** A reservation carries `expires_at_secs`, 15 minutes after it was taken. Nothing runs on a timer. When another booking needs the seat of an unconfirmed reservation that is past its expiry, that booking ends the old one with one unique insert (`decided` `lapsed` and `free`) and takes the seat. The owner's node needs no other node. Backlog row: "A reservation lapses only when another booking needs its seat".
- **EX-22 is narrower than the text: a confirm after the expiry succeeds only while nobody took the seat, and is refused after a takeover or a release.** A confirm never takes a seat (see the C3b1 notes above). A confirmed reservation never lapses.
- **The counterparty's node confirms its own reservations, and signs a `commitment-failure` when a confirm is refused.** See the agreements change, "Commitment failure" and the C3b2 notes.
- **A release error is not logged, and it is not dropped either.** The exit criterion "no `let _ =` drops a release error without a log line" cannot be met with a log line, because no Roym service can write one. It is met this way: a release at a snapshot writes a ledger row when it works, and the next sync releases again until the row exists. The writer's not-formed path is repeated by that same sync pass. A reservation left by a refused accept has no agreement row and is never confirmed, so nothing repeats its release: it lapses when another booking needs its seat. The release of an extension is idempotent. The marker `released:<agreement>:<commitment>` is local, has no signature, and is checked on import against a held agreement and commitment.
- **A free seat is taken before an expired reservation is lapsed.** The plan's pseudo-code lapsed the occupant of each seat as it walked the seats, so a booking could end a reservation while another seat was free. `claim` now makes two passes: the first takes only a seat that is free already, and the second lapses an expired, unconfirmed occupant. (The per-commitment release, `release_ended`, is described in the agreements change.)
- **The `scheduling` import keeps its write order with lapse rows.** It writes `seat:`, then `rsv:`, then `key:`, then `decided:` and `free:`, and refuses any changed row except `key:`. A lapse writes its `decided:` `lapsed` row and its `free:` row together in one unique insert; an import writes them as puts, and a stopped import can leave only one of them. A `decided:` row alone keeps the seat taken, and a `free:` row alone is the same as a release, so neither lets a second booking take a seat that its holder still has.
- **The `ReservationView` of a confirmed reservation still shows its expiry.** The value is not used after a confirm, because a confirmed reservation never lapses.
- **The 15 minutes are a constant, not a setting (owner decision, question Q5).** No Roym service reads the app configuration yet.

## Close-out

Before merge, update the living docs (requirements, architecture, integrated spec, developer guide, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

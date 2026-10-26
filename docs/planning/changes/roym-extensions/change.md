---
status: draft
living-docs-touched:
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
- **Step record:** a signed record of one work step done through an extension (for example "documents uploaded", "driver arrived").
- **Hub:** the Roym web UI. **Client:** any program that uses Roym's methods (see [Roym open foundation](../roym-open-foundation/change.md)).

## Problem

Roym today has one business flow built into its core. A listing has `BookingMode::Slots`. `catalog` has `availability.*` with a seat count per slot (up to `MAX_SLOT_CAPACITY` = 64, `crates/roym_core/src/booking.rs:20`). `transaction` claims seats and runs the booking track. This fits a salon. It does not fit a doctor who needs reports before a visit, a taxi that shares its location, or a tutor who sends homework. If each new business adds its own flow to the core, the core grows without limit. Strangers then cannot compare deals, and reputation loses meaning.

The chat has the same limit. The Hub has seven fixed card templates (`crates/roym_core/src/card.rs:10`). There is no poll, no form, no checklist.

## Scope and non-goals

- In scope:
  - The rule that splits the deal core from extensions.
  - Commitment details: a signed, schema-named block in each commitment.
  - Step records: a generic signed record for extension work steps.
  - Declarative widgets with semantic metadata, and one generic card type that carries them.
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
| EX-1 | A commitment carries an optional `details` block: an extension id, a schema id with a version, and data. The block is inside the signed commitment. | |
| EX-2 | The deal core checks `details` against its schema when the extension is installed on that node. Without the extension, the commitment still verifies and shows its details as labelled plain fields. | |
| EX-3 | `details` may also carry free extra fields (label and value text), so a party can add terms that no schema has. | |
| EX-4 | The deal core's states are the same for every extension. A test runs one agreement with scheduling and one with one-shot and checks that both pass through the same states. | |
| **Reserve and release** | | |
| EX-5 | When an agreement is accepted, the deal core calls the reserve hook of each extension named in a commitment. If one reserve fails, the agreement does not form, and every reserve that succeeded is released. | |
| EX-6 | A reserve is claimed under the resource owner's single-writer fence, also when the owner is not the quote author. This is today's slot rule (`D-C8-2` in the integrated spec), made generic. | |
| **Step records** | | |
| EX-7 | An extension step is a signed `step` record. It names the agreement, the commitment, the extension, the step type and its data. Either party can verify it. | |
| EX-8 | A step record shows in the deal's conversation as one generic card type. A client that does not know the extension shows the step's fields as labelled text. | |
| EX-9 | An opinion can point at one step record. | |
| **Widgets** | | |
| EX-10 | A widget is a declarative definition: a widget type, a semantic tag (for example `poll`, `form`, `checklist`, `location-share`, `document-request`), typed fields, and message ids for its labels. It carries no markup and no script. | |
| EX-11 | A widget card carries its definition or its content hash. A receiver that does not have the definition gets it from the sender's node by hash, and renders it from field types alone. | |
| EX-12 | An answer to a widget is a signed step record. A poll shows the count of answers per option to the people in the conversation. | |
| EX-13 | Widget labels come from the string catalog (see the [multilingual](../roym-multilingual/change.md) change), so a widget shows in the reader's language. | |
| **Extension services** | | |
| EX-14 | An extension may include a service. It ships in the Roym SynApp and is deployed with it. Users see it as part of the extension. | |
| EX-15 | An extension service receives deal and conversation events through the event feed (see [Roym open foundation](../roym-open-foundation/change.md)). It does not poll. | |
| EX-16 | The counterparty of a deal can call an extension service's declared methods only with a grant that names that agreement or conversation, that counterparty, and an expiry. Every other call is refused. The grant ends when the agreement closes, unless the owner extends it. | |
| **First extensions** | | |
| EX-17 | Scheduling: slots, seat capacity, slot claim and the booking track run as the scheduling extension. `catalog` and `transaction` hold no slot code. Today's booking tests pass through the extension. | |
| EX-18 | One-shot: an agreement for goods or a service with no slot settles with a payment acknowledgement and a fulfilment receipt, through the same deal core. | |
| **Insight extensions** | | |
| EX-19 | The manifest has an `insight` kind. Its fields say what it may read. It may not sign, send, or reach the network. Only the manifest field is built in M8. | |

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

A new record type `step`, signed by its author, with: agreement id, commitment id, extension id, step type, data, and an optional pointer to an earlier step. Steps travel as one new generic card type (`step`). This raises the fixed card table from seven types to eight, plus `widget`. It does not create one card type per extension.

### Widgets

A widget card carries a declarative definition. The Hub has one generic widget renderer. It may have a richer renderer for a known semantic tag. For example, it can show a `poll` as bars. Every client can fall back to the plain field view.

Rejected: an iframe or other sandbox that runs extension UI. It is a large attack surface, and it breaks "any client can render it".

### Extension services and the deal grant

An extension service is one more service in `roym.toml`. It uses the same `roym_core` library and the same admission rules as other Roym services. Its methods are local-only by default. The counterparty reaches a declared method only with a **deal grant**: a capability token that the owner's node issues when the agreement forms. It names the agreement, the counterparty's DID, the methods and an expiry. It uses the existing UCAN-style capability tokens (`crates/ucan`). This is a new kind of access from another node, so it gets an ADR before it is built.

Example: a doctor's `documents` extension. The patient's node uploads reports to the doctor's node through `documents.upload`, with the grant of their agreement. The doctor's node stores them and writes a step record "documents received".

### Reserve and release hooks

When a party accepts, the agreement writer calls `reserve` on each extension named in a commitment. The resource owner's node runs the claim under its own fence. If any reserve fails, the agreement does not form, and the writer calls `release` on the ones that succeeded. This is the generic form of today's slot claim (VX-10 in the Value Exchange change).

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
- **Third-party extension code.** When, and with what review or signing, a participant may install an extension that does not ship with Roym. Not in M8.
- **Grant scope for groups.** Whether a deal grant can name a group conversation. Recommendation: not in M8. Deals are between two parties.

## Tasks

- [ ] ADR: deal grants (counterparty access to an extension service, scoped to one deal).
- [ ] Write the M08C slice plans.
- [ ] Manifest format and the extension registry inside Roym.
- [ ] Commitment `details` in the deal core, and schema checks.
- [ ] `step` record and card, the generic step view in the Hub.
- [ ] Widget definition, `widget` card, the generic renderer, the poll renderer.
- [ ] Reserve and release hooks.
- [ ] Extension services: event subscription, deal grants.
- [ ] Move scheduling out of `catalog` and `transaction`. Add one-shot.
- [ ] The `insight` kind in the manifest only.

## Deviations

## Close-out

Before merge, update the living docs (requirements, architecture, integrated spec, developer guide, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

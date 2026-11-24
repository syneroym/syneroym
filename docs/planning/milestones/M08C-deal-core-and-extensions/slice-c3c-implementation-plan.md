# M08C slice C3c — Agree as listed: implementation plan

*Reader: the developer (or AI) who implements slice C3c, and the project
owner, who answers the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-23, by Claude Opus 5.5, and reviewed in
three rounds ([slice-c3c-review.md](slice-c3c-review.md)). The owner
answered the questions in section 2.1 on 2026-11-23, and changed Q1 to two
PRs the same day.**

**Size and PRs.** [task.md](task.md) sizes C3c as L. Checked against the
code, it is larger than any earlier M08C slice, because one part of it is
platform work that the change docs did not size: nothing on a node can run
a Roym method on its own today (section 2.2, point 1). This plan splits C3c
into two parts. Each part is one PR that passes `mise run verify` (Q1):

- **C3c1 — The driver (M).** A service may declare a **wake** in its
  manifest. The substrate that hosts the service calls the wake method when
  an event of a named kind is recorded in the app, and at a fixed interval.
  `transaction` declares one, and its new method `transaction.drive` syncs
  the conversations that changed, a bounded number per call. After C3c1, a
  deal moves on both nodes with no Hub open. An ADR records the wake (Q3).
  Branch: `feat/m08c-c3c-agree-as-listed` (this branch; it holds the plan
  commit).
- **C3c2 — Agree as listed (L).** Both nodes of the deal, in one PR:
  - *The owner's side:* the listing's new fields, the pure function
    `terms_from_listing`, the enquiry's `as_listed`, `quote.from-listing`
    with its rules (current version, consent time, instant limits, signing
    fallback), decline codes, `formed_as` in every progress snapshot, the
    directory's `can_agree_as_listed`, and the owner's Hub parts (listing
    form, confirm, the limit line, the cancel action).
  - *The counterparty's side:* the automatic acceptance with its checks
    immediately before signing, the Hub's "Agree as listed" on a directory
    hit with every term on the screen, the waiting, ended and refused
    lines, and the demo.

  Branch: `feat/m08c-c3c2-agree-as-listed`, cut from `main` after C3c1
  merges.

task.md asks for C3c to be reviewed "in two passes, by node: the owner's
side … and the counterparty's side". C3c2 keeps that advice inside one PR:
its code review reads the owner's side first (section 6.2, steps 1 to 5),
then the counterparty's side (steps 6 to 8). The driver is its own PR
because it is platform code, and its reviewer asks a different question.

**Base.** Every file path and line number below was checked on `main` at
`8fafab15` (2026-11-23). Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: C3c comes after C3b.
[status.md](status.md) says "Merged" for C3b1 (`a46cc98d`, PR #378) and
C3b2 (`7121b136`, PR #379). C3c changes no file of `roym_conversation`,
`crates/conversation`, the forwarding change doc or the conversation WIT.
So C7 may run beside it ([task.md](task.md) section 4: "C3c's driver may
touch `conversation`. If it does, C7 must not run beside C3c"). It does not.
The Engage fix is merged (PR #373).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish each PR (background job, about 20
    minutes, no `--skip` in the final run).
  - No planning ids in code, comments, test names or `.wit`, `.toml`,
    `.json`, `.ftl` files: no "M08C", "C3c2", "AG-44", "VX-54", "D-C3c-4",
    "Q2" or review ids. ADR numbers are allowed.
  - Comments never cite size rules.
  - **File size.** Production `.rs` files at most 800 lines; new test `.rs`
    files at most 800 lines. Section 7.8 lists the files near the cap.
    Put new code in new files.
  - **Function size.** `clippy::too_many_lines` at 100. Add no suppression.
  - **Module layout.** `foo.rs` beside `foo/`, never `mod.rs` in `src/`.
  - **Ports.** Two-node tests use `common::alloc_ports` and the shared
    harness (`common/roym.rs`), never a literal.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out, the demo close-out and the ratchet updates.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 1).
  - Do not commit on `main`.
- [task.md](task.md): "C3c", section 1 (the component size limit),
  section 4, section 5 (the AG-44, AG-45, AG-49, VX-54 row), section 7
  (the row "Agree as listed refuses …").
- [roym-agreements](../../changes/roym-agreements/change.md):
  AG-44, AG-45, AG-49, AG-50, AG-51 (`:156` to `:160`); "Agree as listed"
  (`:220` to `:322`); "Enquiry" (`:344`); "Deal decline" (`:474`);
  "Progress snapshot" (`:495`, `formed_as` at `:511` and `:524`); "Reason
  codes" (`:625`, the last four rows); "Methods" (`:975`, `enquiry.set`
  and `quote.from-listing`); "Differences" (`:1074`, `:1075`); open
  questions 13 and 14 (`:1166` to `:1178`).
- [roym-value-exchange](../../changes/roym-value-exchange/change.md):
  VX-54 (`:143`), principle 13 (`:98`), "Agree as listed" (`:295` to
  `:315`).
- [roym-extensions](../../changes/roym-extensions/change.md): EX-18
  (`:93`, the agree-as-listed half).
- [ADR-0023](../../../decisions/0023-durable-async-primitives.md) §6 and
  §7 (who schedules, and why long tasks are not allowed), and
  [ADR-0027](../../../decisions/0027-host-event-feed.md) (the event
  window). The new ADR of C3c1 sits next to them.
- The C3b plan ([slice-c3b-implementation-plan.md](slice-c3b-implementation-plan.md))
  for the layout and for the reserve hooks this slice relies on.
- The C1, C2, C3a and C3b review files, and the agreements review file
  ([review.md](../../changes/roym-agreements/review.md), findings P39 to
  P54, which shaped "Agree as listed"). Section 7.11 says how this plan
  avoids the kinds of findings they hold.

### Terms used in this plan

- **Offering:** today's `catalog` listing. One **version** is one signed
  listing envelope; each `listing.set` or `listing.withdraw` signs a new
  one with `supersedes` set to the one before
  ([listing.rs:1](../../../../crates/roym_core/src/listing.rs#L1)).
- **Owner:** the person whose node holds the offering. In a deal agreed as
  listed, the owner is the quote author, so the owner's `transaction` is
  the **writer** (the only node that writes the agreement state).
- **Counterparty:** the person who taps "Agree as listed".
- **Tap:** the counterparty's one action. It sends an enquiry with
  `as_listed`. It is the counterparty's **consent**, which holds until
  `valid_until_secs`.
- **Instant / confirm first:** the owner's choice per offering. In instant
  mode the owner's node answers with no action by the owner. In confirm
  first mode it answers when the owner confirms.
- **Driver:** what runs `transaction.sync` on a node with no Hub and no
  person. In this plan the driver is the substrate's **wake** of
  `transaction` (C3c1).
- **Wake:** a node-local call of one method of a service, made by the
  substrate that hosts the service, when an event of a named kind is
  recorded in the app, and at a fixed interval (D-C3c-1).
- **Fence:** a unique insert (`AppDataLayer::create`): of two calls that
  race, only one creates the row.
- **Build and compare:** the counterparty's node builds the deal terms
  itself from the offering version it stored at the tap, and compares them
  with the quote's disclosed terms (agreements doc, `:283`).

## 1. What C3c builds, in one paragraph

Today a deal needs three actions by people: an enquiry, a quote and an
acceptance. And a node moves a deal only when its person's Hub draws the
conversation, because the Hub is the only caller of `transaction.sync`
([messages.ts:367](../../../../crates/roym_web/ui/src/screens/messages.ts#L367)).
C3c adds a short path for the most common deal, an offering taken as
listed. The counterparty taps "Agree as listed" on a directory hit. Its
node sends an enquiry that names the offering version and says "as
listed". The owner's node answers with a quote built from the current
offering version, with no action by the owner when the offering is
`instant`, or after one confirm. The counterparty's node builds the same
terms itself and, when they are equal and the consent still holds, signs
its acceptance with no person at the screen. The writer then forms the
agreement as for any quote. Every record is one that exists today; only
two fields are new (`as_listed` in the enquiry, `formed_as` in the
snapshot) and one is new in a decline (`code`). To make the steps run
when the cards arrive, C3c first adds a driver on each node.

### Why C3c is split in two (Q1)

- **C3c1 is platform work.** It adds a field to the app manifest, a
  deploy fact on the substrate, a substrate worker and one `transaction`
  method. Its reviewer's question is "does every node run `transaction.sync`
  for every changed conversation, soon, safely, with a bound on each call,
  in both the native and the WASM build". It changes no record. Today's
  deals move sooner; nothing else changes. Everything in C3c2 depends on
  it, so it merges first.
- **C3c2 is the deal, on both nodes.** Its code review has two passes:
  - The owner's side: "does the owner's node answer only from the current
    version, only while the consent holds, only within the instant limits,
    and make at most one quote per tap".
  - The counterparty's side: "does the node sign only when the terms are
    exactly the terms the person saw, checked immediately before signing,
    at most once per tap".
- **Why the two sides are one PR.** The product is not released, so a
  state between the two sides that nobody can use is no loss. In one PR,
  the automatic acceptance never exists without the owner's rules next to
  it, so the order question of a split goes away. The steps still build
  the owner's side first: the counterparty's tests need a quote from the
  owner's node.
- Sizes (production code; tests are about the same again): C3c1 about
  650 lines (platform about 450, `transaction` about 200); C3c2 about
  1,950 (core 450, `transaction` 800, `catalog` and `directory` 80, Hub
  620).

## How this is shown to an end user

C3c1:

- A shop owner sends a quote and closes the app. The other person accepts
  it. The deal becomes "Agreed" on both phones within a few seconds,
  although the shop owner never opened the app again.
- Before this change, the deal waited until the shop owner opened that chat.
- Nothing on the screen is new.

C3c2, the shop owner:

- A shop owner edits an offering in the Hub. A new part "Agree as listed"
  asks: "Agree at once (instant)" or "I confirm each one", the texts for
  cancellation, refund and disputes, how long a quote stays valid, and
  when payment is due. "Instant" is offered only when the app can sign for
  the owner.
- When someone agrees to a "confirm first" offering, the owner sees the
  request with a "Confirm" button. One tap sends the quote.
- When an instant offering has too many open deals, the owner sees "This
  offering is waiting for you: it reached its limit of 20 open instant
  agreements", and a button to cancel the open instant agreements of that
  offering.

C3c2, the person who agrees:

- A person finds an offering in the directory, for example "a sack of
  wheat, ₹1,200". The hit has a button "Agree as listed".
- Before the tap, the screen shows the price, "will be paid to …", the
  cancellation, refund and dispute texts, when payment is due, how long
  the owner's quote stays valid, and how long the tap holds (one hour for
  an instant offering, two days for one the owner confirms).
- After the tap, the chat shows "Sent, waiting for the owner's node". When
  the owner's node answers, the deal becomes "Agreed" with no other tap.
- If the owner changed the offering since the tap, the person sees a normal
  quote with "Accept", and decides. If the owner withdrew the offering, or
  the tap ran out of time, the person sees why, in their own language.
- An offering with time slots shows only "Ask" for now (Q4).

How a developer can see it work:

- `cargo nextest run -p syneroym-substrate --test roym_driver_e2e` (C3c1):
  a quote is accepted on one node, and both nodes reach `agreed` with no
  `transaction.sync` call by the test.
- `cargo nextest run -p syneroym-roym-transaction as_listed` (C3c2): the
  owner's rules (consent expired, offering withdrawn, version changed,
  instant cap, one per counterparty, signing fallback), `formed_as`, and
  the counterparty's checks before it signs.
- `cargo nextest run -p syneroym-substrate --test roym_as_listed_e2e`
  (C3c2): an instant offering reaches `agreed` from one `enquiry.set` call
  on the counterparty's node, with no other call on either node.

## 2. Points for the owner

### 2.1 Questions

Each question has options and a recommendation. The plan is written for
the recommended answer.

**Owner answers, 2026-11-23.** Q1: (b), two PRs (changed from (a) the same day). Q2, Q3, Q4, Q7, Q8: agree with (a). Q7 is restated for two PRs. Q8 holds only once P16 is settled (P20 settles it). Q5: (a), but only with P11 fixed. Q6: do not accept (a) as written; count in-flight quotes (P12), then (a) is fine.

#### Q1. How many PRs?

Owner answer: (b), 2026-11-23. Two PRs: C3c1 (the driver), then C3c2
(agree as listed, both sides). The owner first chose (a), then changed it
the same day: the product is not released, so a working system between
PRs is not needed.

- (a) Three PRs: the driver, the owner's side, the counterparty's side.
  Each part has its own reviewer question, each leaves a working system,
  and the automatic signature comes last. Costs one more PR cycle (review
  rounds, close-out, `mise run verify`).
- **(b) Two PRs: the driver, then all of agree as listed.** Chosen. The
  driver is platform code with its own reviewer question, and the rest
  depends on it. The second PR is about 1,950 lines of production code
  across `roym_core`, three services and the Hub. Its code review reads
  the two nodes in two passes (section 1, "Why C3c is split in two").
- (c) One PR. About 2,600 lines of production code and as many of tests.
  A diff this large gets a weaker review, and the automatic signature is
  the part that most needs a careful one.

#### Q2. The driver: how does a node run `transaction.sync` with no Hub?

Owner answer: (a), 2026-11-23. A node-local wake, declared in the manifest, run by the substrate.

Facts (section 2.2, point 1): no Roym method is ever called on a timer or
on an event. The supervisor's schedules (ADR-0023 §6) are the only timer
that calls a guest, but Roym runs with no supervisor in both of its forms:
linked into the substrate (`init_roym`,
[roym.rs:76](../../../../crates/substrate/src/runtime/roym.rs#L76)) and
deployed as WASM with `apply_plan` by the tests and by `roymctl app deploy`
([common/roym.rs:393](../../../../crates/substrate/tests/common/roym.rs#L393)).
The backlog says a schedule deployed that way never runs
([deferred-backlog.md:415](../../deferred-backlog.md)). No WIT gives a guest
a timer, and every guest call is bounded by a 5-second epoch
([deferred-backlog.md:405](../../deferred-backlog.md)).

- **(a) A node-local wake, declared in the manifest, run by the
  substrate.** A service declares one `wake` (D-C3c-1): the method to call,
  the event kinds that trigger it, and an interval. The substrate that
  hosts the service calls it, as `run-scheduled` already calls a guest
  method ([lifecycle.rs:644](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L644)),
  when one of those events is recorded in the app (the host already
  records `conversation.message` for every message a conversation shows,
  and already has a change signal per app instance,
  [event_bus.rs:146](../../../../crates/core/src/event_bus.rs#L146)), and
  every `every_secs`. `transaction.drive` reads the app's events to find
  the conversations that changed and syncs a bounded number of them per
  call. **Recommended:** it works in both Roym forms and in every two-node
  test, reacts within about a second, needs no `conversation` →
  `transaction` edge, and closes the backlog row "A card is filed only when
  a client calls `transaction.sync`"
  ([deferred-backlog.md:654](../../deferred-backlog.md)) with the
  "node-side scheduler" that row names. Cost: about 450 lines of platform
  code and an ADR (Q3).
- (b) A supervisor schedule in `roym.toml`. No code in the platform, but it
  never runs for Roym (no supervisor in either form), the finest step is
  the supervisor's 30-second pass, and it would not run in any two-node
  test. Rejected.
- (c) A loop in the substrate's Roym code (`runtime/roym.rs`) that calls
  `transaction.sync` for the linked build only. About 150 lines, but the
  WASM build, which every two-node test and Playwright use, gets no driver,
  so AG-44 and AG-50 cannot be tested.
- (d) The owner's fallback in task.md: no driver. Confirm first only, and
  the Hub is the trigger. `instant` is not built (AG-45 partly, AG-50 not
  at all).

#### Q3. An ADR for the wake?

Owner answer: (a), 2026-11-23. Yes: ADR-0031 "Node-local wake of a service", written in C3c1 step 1.

The wake is a new way for the host to call a guest. ADR-0023 §6 says the
supervisor is the scheduler. A wake is not a cluster schedule: it is
node-local work that each member of a service does for its own data, so
"exactly one member runs a tick" does not apply.

- **(a) Yes: ADR-0031 "Node-local wake of a service", written in C3c1
  step 1 and accepted by the owner before any code.** Recommended:
  AGENTS.md asks for architectural decisions as ADRs, and this one bounds
  how later services may use the wake (one per service, short calls, never
  queued, no overlap).
- (b) No ADR; the decision lives in this plan and in the developer guide.
  The plan is archived later, so the reason would be lost.

#### Q4. Slot offerings: can the person pick a slot before the tap?

Owner answer: (a), 2026-11-23. Build the slot path in records and backend services; Hub shows "Ask" alone on slot offerings for now.

A slot offering needs `as_listed.slot_id`, so the person must pick a
slot. But slots live in the owner's `scheduling` service, and every
`availability.*` method is local-only
([router.rs:52](../../../../crates/roym_core/src/router.rs#L52)). No
directory hit, listing or wire-reachable method carries them (checked:
neither `roym_directory` nor `roym_federated_query` names a slot). So no
Hub on the counterparty's node can show the owner's free slots.

- **(a) Build the slot path in the records and services, but the Hub offers
  "Agree as listed" only for offerings whose extension has no reserve
  hook.** `terms_from_listing`, the `slot` in `as_listed`, `instant` for a
  slot offering and the `slot-taken` end are all built and tested (two-node
  tests read the slot from the owner's node, as story 60 does). The Hub
  shows "Ask" alone on a slot offering. A backlog row names the missing
  piece. **Recommended:** no new wire-reachable method and no change to the
  contract; the missing piece is a way to see another node's free slots,
  which belongs to M08D's offering page (D5).
- (b) Consent to a time range: the tap carries a `window` the person
  accepts, and the owner's node picks a free slot inside it. Needs a new
  "find a free slot" hook, a new decline code (no slot in the range), and
  a change to build and compare (the slot is not built by the counterparty
  but checked against the range). About 300 more lines and a contract
  change in the agreements doc.
- (c) The listing carries its open slots in the signed payload. Every slot
  change then signs a new listing version, `catalog` needs an edge to
  `scheduling`, and free seats are still not shown.
- (d) A wire-reachable `availability.list`. It is the first wire path into
  an extension service, which task.md moved out of M08C (C5).

#### Q5. `transaction` needs the current offering version on the owner's node

Owner answer: (a), 2026-11-23, with P11 fixed. Add catalog back to transaction's depends_on, and wire the binding in runtime/roym.rs for the linked build.

The owner's node answers "only from the current active version" (`:311`).
The current version is in `catalog` (`listing.get` returns the current
envelope, record id and status,
[listing_ops.rs:504](../../../../crates/roym_catalog/src/app/listing_ops.rs#L504)).
C3b1 removed `transaction`'s edge to `catalog`, when slots moved out
([roym.toml:82](../../../../crates/roym_core/app/roym.toml#L82)).

- **(a) Add `catalog` back to `transaction`'s `depends_on`**, and read the
  current version with one local `listing.get` call. No cycle: `catalog`
  depends only on `profile`
  ([roym.toml:67](../../../../crates/roym_core/app/roym.toml#L67)).
  Recommended: one sibling call, and `catalog` stays the only holder of
  listings. P11 fix: also wire `save_binding` in `wire_roym_topology`
  (`runtime/roym.rs`) for `transaction` -> `catalog` so the linked build
  can make the call.
- (b) The counterparty's enquiry carries the envelope, and the owner's node
  trusts it if its `record_id` is the newest it can find in its own
  `listing_history`. That still needs a read of `catalog`'s data.

#### Q6. The instant cap may be passed by one under a race

Owner answer: do not accept (a) as written; count in-flight quotes (P12), then (a) is fine, 2026-11-23.

The cap is checked when the owner's node answers. In addition to open
formed agreements, `answer_as_listed` counts in-flight quotes
(`in_flight_instant`, P12): quotes answered under `AsListedInstant` with no
decision fence yet and not expired. The limits check `open agreements +
in_flight quotes`. Two answers can still run at the same time (the wake and
a Hub sync), and each can see `cap - 1` open + in-flight quotes.

- **(a) Accept it with in-flight quotes counted, and say so.** The wake
  never runs twice at once for one service (D-C3c-1), so only a Hub sync
  can race it. With in-flight quotes counted (P12), multiple rapid taps are
  properly bounded, and the remaining race between two syncs at the exact
  same instant passes the cap by at most one. A backlog row records it.
  Recommended: the cap is an abuse limit, not a stock count; a slot's seats
  have their own fence.
- (b) A fence per cap place (`instant:<listing>:<n>`), released when an
  agreement stops counting. About 120 more lines, and "stops counting"
  happens on a timer (7 days), which would need its own release pass.

#### Q7. Demo

Owner answer: (a), 2026-11-23, for three PRs: C3c1 `- none`, the
counterparty's PR extends story 50. With two PRs (Q1) this reads: C3c1
`- none`; C3c2 extends story 50 with a deal agreed as listed.

The rule: a change a viewer can see gets a story.

- **(a) C3c1: `- none` (a deal that moves with the Hub closed shows no new
  screen; the two-node test proves it). C3c2: extend story 50 with a deal
  agreed as listed.** In story 50, Bob already publishes a listing and
  Alice finds it. C3c2 adds: Bob publishes a second listing, "a sack of
  wheat, ₹1,200", `instant`, `not_limited`, with deal terms; Alice taps
  "Agree as listed", sees the terms and the consent time, taps, sees
  "waiting", then "Agreed". Recommended: task.md asks to extend story 50.
- (b) As (a), and C3c2 also shows Bob's listing form and a confirm-first
  offering that Bob confirms. Longer story; the confirm path is already
  covered by Hub tests and Playwright.

#### Q8. A slot that is removed before the owner answers

Owner answer: (a), 2026-11-23, with P16 settled (P20 settles it). The owner's node declines the enquiry with the hook's error code when the prepare hook refuses on the owner's node.

`terms_from_listing` builds the slot terms from `as_listed.slot`, and
`answer_as_listed` calls the scheduling extension's `prepare` hook to
verify the slot (`check_author_slots`, D-C3c-12). If the owner removed the slot after the tap, `prepare`
refuses with `no-such-slot` or `slot-not-in-listing`.

- **(a) The owner's node declines the enquiry, with the hook's code as the
  decline `code`.** Recommended: the person learns at once that the slot is
  gone, in their own language; the codes already exist. Settled in P16 and
  P20: `answer_as_listed` runs the prepare hook on the owner's node for
  author commitments before the `answer:` fence, and declines the
  enquiry when the hook refuses. `quote.set` is unchanged.
- (b) The enquiry waits for the owner, who answers with a normal quote or
  declines by hand. The person waits for a slot that no longer exists.

### 2.2 Points found while reading

1. **Nothing calls a Roym method on its own.**
   - The only production caller of `transaction.sync` is the Hub's
     `fillThread`
     ([messages.ts:367](../../../../crates/roym_web/ui/src/screens/messages.ts#L367);
     the agreements doc cites `:424`, an older line).
   - The substrate's "owner loop" in `init_roym` is a one-time `for` loop
     ([roym.rs:115](../../../../crates/substrate/src/runtime/roym.rs#L115)),
     not a periodic one.
   - The conversation host worker runs every 5 seconds
     ([services.rs:236](../../../../crates/substrate/src/runtime/services.rs#L236))
     and calls only the `conversation` guest's `on-message`.
   - The supervisor's `run-scheduled` is the only timer that calls a guest
     ([lifecycle.rs:575](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L575)),
     and Roym has no supervisor (Q2).
   - The host records `conversation.message` (subject: the conversation
     id) when a conversation admits a visible message, in the app's
     shared event window
     ([events.wit](../../../../crates/wit_interfaces/wit/events/events.wit),
     [event_bus.rs:151](../../../../crates/core/src/event_bus.rs#L151),
     `:189`). Every service of the app can read every service's events.
     The window is in memory: at most 10,000 events or 24 hours, and a
     restart gives `reload`.
   So the driver (Q2) is the first piece of C3c, not a detail of it.
2. **Signing with no session already works.** `quote.set` signs with the
   delegated person principal from `signing::person_principal`
   ([signing.rs:192](../../../../crates/roym_core/src/signing.rs#L192)),
   called through `resolve_principal_and_owner`
   ([app.rs:288](../../../../crates/roym_transaction/src/app.rs#L288)),
   not from a session. `agreement.accept` does the same
   ([agreement_ops.rs:140](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L140)),
   and so does the countersign (`sign_author_receipt_half`,
   [decide.rs:677](../../../../crates/roym_transaction/src/app/decide.rs#L677);
   the agreements doc cites `:695`, an older line). A local call is
   admitted by origin, not by caller:
   [admit.rs:20](../../../../crates/roym_core/src/admit.rs#L20) admits
   `CallerOrigin::Internal`, which every non-wire dispatch sets. So the
   wake and the automatic steps need no new signing path.
3. **There is no `ListingEnvelope` type.** The agreements doc writes
   `terms_from_listing(version: ListingEnvelope, …)`. A listing travels as
   the envelope string, and `listing::verify_envelope`
   ([listing.rs:572](../../../../crates/roym_core/src/listing.rs#L572))
   returns the payload. So the pure function takes the verified
   `ListingPayload` (D-C3c-6).
4. **The counterparty cannot build a slot's window.** `slot_id` is
   `derive_slot_id(listing_id, start, end)`
   ([listing.rs:458](../../../../crates/roym_core/src/listing.rs#L458)), a
   digest, so the window cannot be read back from it. But the mapping
   table puts the slot's window into `c1.window` (`:270`). So `as_listed`
   carries the slot's window with its id (D-C3c-5). The person must see the
   time anyway before the tap.
5. **The directory hit drops every block the tap screen needs.** `MergedHit`
   ([merge.rs:21](../../../../crates/roym_federated_query/src/merge.rs#L21))
   has the title, summary and translations, but no `payment`, `location`,
   `extension` or the new `deal_terms`. The verified envelope is kept in
   the run (`directory.run-envelope`,
   [client_runs.rs:145](../../../../crates/roym_directory/src/app/client_runs.rs#L145)),
   and the Hub already reads it in the "Ask" action
   ([directory.ts:446](../../../../crates/roym_web/ui/src/screens/directory.ts#L446)).
   So the Hub reads the envelope from the run and the payload from
   `listing.verify` on its own node; the hit carries only the flag
   `can_agree_as_listed` (D-C3c-11).
6. **`DealDeclinePayload` has no `code`.** The agreements doc's sketch has
   one (`:483`); the code does not
   ([decline.rs:35](../../../../crates/roym_core/src/transaction/decline.rs#L35)).
   C3c2 adds it (D-C3c-9).
7. **`formed_as` exists only in docs** (agreements doc `:511`, `:524`).
   `AgreementProgressPayload` has no such field
   ([progress.rs:76](../../../../crates/roym_core/src/progress.rs#L76)).
8. **An enquiry's `description` is required** (`LocalText::validate`
   refuses a blank text,
   [transaction.rs:252](../../../../crates/roym_core/src/transaction.rs#L252)).
   The agreements doc makes it optional with `as_listed` (`:351`). Every
   reader of `description` changes (D-C3c-7).
9. **No Rust function writes an amount with a `.` decimal sign.** The
   mapping table needs `c2.description = "<currency> <amount>"` (`:275`),
   the same on both nodes. `roymctl` copies the user's text
   ([transaction.rs:373](../../../../apps/roymctl/src/commands/roym/transaction.rs#L373)),
   the Hub has `minorToDecimal`
   ([money.ts:68](../../../../crates/roym_web/ui/src/money.ts#L68)).
   C3c2 adds one pure Rust function (D-C3c-6).
10. **Tests read `transaction.sync`'s counts.** With a driver, a card may
    already be filed when the test calls `transaction.sync`, so `filed` can
    be 0. The sites: `common/roym_flow.rs:147`,
    `roym_booking_e2e.rs:239`, `:240`, `roym_transaction_e2e.rs:164`,
    `:230`, `:254`, `:567`. The parity harness runs services in-process
    with no substrate, so its counts (`dual_build_parity/transaction.rs:172`
    and others) do not change. C3c1 changes the seven substrate sites
    (D-C3c-4).
11. **Files near the 800-line cap** in `transaction`: `sync.rs` 759,
    `backup.rs` 785, `decide.rs` 724, `app.rs` 710, `backup/sections.rs`
    703, `quote_ops.rs` 695. Proactively extract `file_enquiry_card` and
    `file_quote_card` from `sync.rs` into `sync/cards.rs` (~140 lines) to keep
    it safely below the 800-line cap (P4). New code goes in new sibling files
    (section 7.8).
12. **No `roymctl` command makes a listing**, and the enquiry command
    takes no envelope
    ([transaction.rs:72](../../../../apps/roymctl/src/commands/roym/transaction.rs#L72)).
    C3c2 adds what the owner's side needs (D-C3c-15).

## 3. What C3c meets

| Change doc | Criterion | Part | Where it is proved (section 7) |
| --- | --- | --- | --- |
| roym-agreements | AG-50 | C3c1: every node runs `transaction.sync` for a changed conversation with no session and no Hub, within a bound; no `conversation` → `transaction` call | two-node `roym_driver_e2e`; service `tests/drive.rs` (the bound per call); substrate unit tests of the wake worker |
| roym-agreements | AG-45 | C3c2: `listing.set`'s refusals; "open" (open agreements + in-flight quotes, P12); the cap; one per counterparty; the signing fallback; a full slot ends `not-formed` with `slot-taken`; the owner's Hub (form, limit line, cancel); the counterparty's waiting line | unit `listing/as_listed` tests; service `tests/as_listed_owner.rs`; Hub unit tests; Playwright |
| roym-agreements | AG-49 | C3c2: the owner's node declines `consent-expired` and `offering-withdrawn`, answers from the current version; the counterparty's three checks immediately before signing; a refused first contact shows as refused | service `tests/as_listed_owner.rs`, `tests/as_listed_accept.rs`; Playwright |
| roym-agreements | AG-51 | C3c2: every snapshot carries `formed_as`, the same in every snapshot | service `tests/as_listed_owner.rs`; unit `progress` and presentation tests |
| roym-agreements | AG-44 | C3c2: `terms_from_listing` (one row per refusal and per mapping row), `enquiry.set`'s `as_listed`, `quote.from-listing`; the automatic acceptance, build and compare generated from the structs, the tap screen, `viewed_via_mt` | unit `as_listed` tests; service `tests/as_listed_accept.rs`; two-node `roym_as_listed_e2e`; Playwright story 50 and `roym-hub.spec.ts` |
| roym-value-exchange | VX-54 | C3c2, through AG-44, AG-45, AG-49 | as above |
| roym-extensions | EX-18 | C3c2: a one-shot offering marked `not_limited` can be `instant`, and it is agreed as listed | service `tests/as_listed_owner.rs`; two-node `roym_as_listed_e2e` |

Not met here: AG-46 to AG-48 and AG-52 (C6). The slot picker on the Hub
(Q4, a backlog row). The parts that task.md section 5 gives to M08D (the
mode in the intent record, variants, the offering page).

## 4. Behaviour-change audit

### Wire changes (between nodes)

- **C3c1: none.** No record, card or method changes. Cards are filed
  sooner, by the wake, with the same code.
- **C3c2: `enquiry` gains an optional `as_listed`, and `description`
  becomes optional when `as_listed` is set** (D-C3c-5, D-C3c-7). The
  record type and version stay (`enquiry` 1; the product is not
  released). An older node refuses such an enquiry when it files it (it
  cannot read the field), as for every in-place change of this milestone.
- **C3c2: `deal-decline` gains an optional, visible `code`** (D-C3c-9).
- **C3c2: `agreement-progress` gains a required `formed_as`** (D-C3c-10).
  Every snapshot changes its signed bytes.
- **C3c2: the listing payload gains `acceptance`, `not_limited`,
  `instant_cap` and `deal_terms`** (D-C3c-8). They are signed and travel
  in the listing envelope to directories. A listing that does not set them
  keeps today's signed bytes (each is left out at its default).
- **C3c2: the counterparty's node sends its acceptance half on its own**,
  during `transaction.sync`, when the checks pass. The half is the same
  record as a person's.

### Local method and manifest changes (one node)

- **The app manifest gets an optional `wake` per service** (C3c1,
  D-C3c-1). It goes into `ServiceConfig`, so the substrate gets it in the
  deploy, and a change to it redeploys the service (as a change to
  `health_check` does).
- **`roym.toml`:** `[services.transaction.wake]` (C3c1); `transaction`'s
  `depends_on` becomes `["conversation", "scheduling", "catalog"]` (C3c2,
  Q5).
- **New local-only methods on `transaction`:** `transaction.drive` (C3c1),
  `quote.from-listing` and `deal.as-listed-status` (C3c2). All through
  `admit::require_internal`. The router already routes `transaction.`,
  `quote.` and `deal.` to `transaction`. `transaction.drive` stays routed,
  so a Hub call reaches it; it does nothing a sync does not do.
- **Changed inputs and outputs:**
  - `enquiry.set` takes `as_listed` (C3c2).
  - `listing.set` takes the four new listing fields (C3c2).
  - `directory.find`'s hits gain `can_agree_as_listed` (C3c2).
  - `deal.decline`'s output and the `deal-decline` card view carry `code`
    (C3c2).
  - `agreement.get`, `.list` and `.progress` views carry `formed_as`
    (C3c2).
  - `enquiry.get`, `.list` and `transaction.thread` views carry the
    counterparty's local as-listed state (C3c2).
- **Backup:** `transaction`'s `SCHEMA_VERSION` goes from 7 to 8
  ([app.rs:58](../../../../crates/roym_transaction/src/app.rs#L58)),
  because stored snapshots gain a required field. New: the `as_listed`
  collection and the ledger kinds `answer:` and `autoaccept:` (C3c2).
  `catalog`'s version stays: the new listing fields have defaults, so an
  older bundle still reads.
- **`transaction.sync`'s counts** can be 0 for a card the wake already
  filed (C3c1). The outcome is the same.

### Behaviour changes

- **C3c1: every node with Roym syncs a conversation within about a second
  of a new message, and every 30 seconds** (D-C3c-2). So today's writer
  countersigns, refusal snapshots, confirms, ticks, recoveries and
  release retries run with no Hub open. Before, each waited for the Hub.
- **C3c1: a card can be filed by the wake and the Hub at the same time.**
  Every filing step is already safe for this: rows are made with fences
  and repeats are filed as repeats (C1b, C2a). Two syncs of one
  conversation at once already happen today (two Hub tabs).
- **C3c2: the owner's node may answer an enquiry with a quote and sign it
  with no person present**, only for an `as_listed` enquiry of an
  `instant` offering within its limits.
- **C3c2: the owner's node may decline an enquiry with a signed
  `deal-decline` on its own**, with `consent-expired`, `offering-withdrawn`
  or a slot code (Q8).
- **C3c2: the counterparty's node may sign an acceptance with no person
  present**, only after the tap and only when the checks pass.

### Stays the same

- Every deal state, transition, reason code (four are added) and
  settlement rule. The writer's decision and its recovery.
- Every record of C1 to C3b other than the three named above. The quote
  built from a listing is an ordinary quote: `answers.kind = "enquiry"`,
  ordinary disclosures, ordinary card.
- The normal enquiry, quote and accept path, in the Hub and in `roymctl`.
  Stories 40, 50 and 60 keep their steps (C3c2 adds steps to story 50).
- `conversation`, `profile` and `scheduling` code, and every WIT
  interface. The wake calls an existing export (`api.invoke`).
- The supervisor's schedules (ADR-0023 §6).

## 5. Decisions this plan makes

### C3c1 — The driver

**D-C3c-1. The wake (Q2, Q3).** A new optional manifest key per service,
next to `health_check`:

```toml
[services.transaction.wake]
interface = "syneroym-roym:transaction/api@0.1.0"
method = "invoke"
params = '[ "{\"method\":\"transaction.drive\",\"params\":{}}" ]'
on_events = ["conversation.message"]
every_secs = 30
```

The `params` string is the frame `call_sibling` sends
([method.rs:385](../../../../crates/roym_core/src/method.rs#L385)), which
both builds read (`extract_request_param`,
[dual_build.rs:34](../../../../crates/roym_core/src/dual_build.rs#L34)).
Check the exact frame against `call_sibling` when you write it.

In `crates/app_orchestration/src/wake.rs` (new; sibling of `schedule.rs`):

```rust
/// A node-local call the hosting substrate makes on events and on a timer.
/// See ADR-0031.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WakeSpec {
    pub interface: InterfaceName,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    /// Event kinds of this app that trigger a call. May be empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_events: Vec<String>,
    /// The call also runs when this much time passed since the last one.
    pub every_secs: u32,
    #[serde(default = "default_wake_timeout_ms")]
    pub timeout_ms: u32,
}

pub const MIN_WAKE_EVERY_SECS: u32 = 5;
pub const MAX_WAKE_EVERY_SECS: u32 = 86_400;
pub const MAX_WAKE_EVENT_KINDS: usize = 8;
pub const DEFAULT_WAKE_TIMEOUT_MS: u32 = 10_000;
pub const MAX_WAKE_TIMEOUT_MS: u32 = 30_000;   // as MAX_SCHEDULE_TIMEOUT_MS

impl WakeSpec {
    pub fn validate(&self, interfaces: &[InterfaceName]) -> Result<(), String>;
}
```

Validation, one failing test per rule: `interface` is one of the
service's `interfaces`; `method` is not empty; `params`, when set, is valid
JSON; each `on_events` entry has the event kind's shape (lowercase
letters, digits, `.` and `-`, at most 64 bytes, as `events.wit` says) and
there are at most 8; `every_secs` in 5..=86,400; `timeout_ms` in
1..=30,000.

Where it goes:

| Place | Change |
| --- | --- |
| `crates/app_orchestration/src/models/service.rs:152` (`ServiceConfig`) | `#[serde(default, skip_serializing_if = "Option::is_none")] pub wake: Option<WakeSpec>`, next to `health_check`. In `ServiceConfig`, not on the planned service as `schedule` is ([service.rs:463](../../../../crates/app_orchestration/src/models/service.rs#L463)): the substrate uses it, so a change must reach the substrate (the reason `schedule` stays out does not hold here) |
| `crates/app_orchestration/src/models/manifest.rs:129` | the manifest's validation calls `WakeSpec::validate`, as it does for `schedule` |
| `crates/control_plane/src/service/orchestration/deploy/commit.rs:168` | `wake_json` beside `health_check_json`, passed to `set_deploy_facts` |
| `crates/core/src/local_registry.rs:470` (`set_deploy_facts`), `crates/core/src/storage.rs`, `crates/data_db/src/registry_store.rs` | In `storage.rs`, replace `DeployFacts` tuple with struct `DeployFacts { pub service_type: String, pub health_check_json: Option<String>, pub manifest_hash: Option<String>, pub visibility: Option<String>, pub wake_json: Option<String> }` (P10); update `RegistryStorage` trait and in-memory store; in `data_db`, add `ALTER TABLE service_deploy_facts ADD COLUMN wake_json TEXT` migration and update `save_deploy_facts` (P2); update storage mocks in `failing_storage.rs` and `service_ownership.rs`; in `clear_deploy_hash_after_binding_push` (`lifecycle.rs:193`), copy `wake_json` so binding pushes preserve the wake (P10); cache `wake_json` in `LocalRegistry.service_deploy_facts`; new `wake_targets()` lists `(service_id, WakeSpec)` |
| `crates/substrate/src/runtime/wake.rs` (new) | the wake worker (D-C3c-2); plumbing passes `shared.event_bus()` and endpoint registry to `RuntimeServices`; skips worker when proxy is `None` (P15) |
| `crates/substrate/src/runtime/services.rs:236` | spawn the worker beside the conversation worker, store its `JoinHandle`, race in `run_until_shutdown`, with cancel token (P15) |
| `crates/substrate/src/runtime/roym.rs` | the linked build registers `transaction`'s wake from `roym.toml` (D-C3c-3) |
| `crates/roym_core/app/roym.toml:84` | `[services.transaction.wake]` as above |

The ADR (Q3) states: a wake is node-local work for the member's own data;
every member of a service is woken on its own node; one call at a time
per service; never queued; never retried (the next wake is the retry); a
call is an ordinary guest call, bounded by the epoch and `timeout_ms`; it
is not a long task (ADR-0023 §7) and not a schedule (ADR-0023 §6). It
records that a wake is strictly local (never routed outbound via community
registry, P1), that proxy `None` mode skips worker spawn, and that `on_events`
for a service with no app context is ignored and logged once (P15). It
records the rejected options of Q2.

**D-C3c-2. The wake worker** (`crates/substrate/src/runtime/wake.rs`).

```rust
pub(crate) struct WakeWorker {
    targets: Arc<dyn WakeTargets>,   // the registry rows, and the linked Roym wake
    bus: Arc<EventBus>,
    proxy: Arc<ProxyRouter>,
    registry: Arc<LocalEndpointRegistry>,
}

impl WakeWorker {
    pub(crate) async fn run(self, cancel: CancellationToken);
}
```

Each loop, once per second:

```text
for each target (service_id, spec):
    state = per-target {bus_cursor, last_run_at, running, has_pending}
    if running: skip                         // one call at a time
    triggered = false
    if state.has_pending:                    // P14: work left from previous call
        triggered = true
        state.has_pending = false
    if spec.on_events not empty:
        read the app's events after state.bus_cursor (host side, no guest)
        triggered = any event.kind in spec.on_events
        reload (restart, or the cursor fell out of the window) -> triggered = true
        state.bus_cursor = the new cursor
    due = now - state.last_run_at >= spec.every_secs
    if triggered or due:
        // P1: strictly node-local; never resolve via remote/community registry
        if registry.lookup(&service_id, &spec.interface).is_none():
            tracing::warn!(service_id, interface = %spec.interface, "wake target has no local endpoint registered; skipping");
            continue
        state.running = true; state.last_run_at = now
        // P6: scopeguard / RAII guard resets state.running = false on completion or panic
        spawn with running_guard:
            res = proxy.invoke(ProxyRequest {
                      target_service: service_id,
                      interface: spec.interface, method: spec.method,
                      params: spec.params or "[]",
                      caller: CallerContext::service_system(service_id),
                      origin: CallOrigin::Native { service_id },
                      protocol: JsonRpcV1, idempotent: false,
                      idempotency_key: None,
                      timeout: Some(spec.timeout_ms) })
            match res:
                Ok(val) => {
                    // the wake reply contract (ADR-0031), P14, P18
                    for w in val["warnings"] (strings): tracing::warn!(service_id, "{w}")
                    if val["again"] == true: state.has_pending = true
                }
                Err(err) => tracing::warn!(service_id, error = %err, "wake call failed")
```

- The call is the same as `run-scheduled`'s
  ([lifecycle.rs:644](../../../../crates/control_plane/src/service/orchestration/lifecycle.rs#L644)).
  The callee sees `CallerOrigin::Internal`, so `require_internal` admits
  it.
- The worker verifies `registry.lookup(&service_id, &spec.interface).is_some()`
  before invoking (P1). A miss in the endpoint registry is never treated as
  remote for a node-local wake (matching `lifecycle.rs:625`).
- The worker reads the event bus on the host side. It finds the app
  instance of a service using `EventBus::app_instance_of`
  ([event_bus.rs:124](../../../../crates/core/src/event_bus.rs#L124)), which
  already exists on `EventBus` (P5). If a standalone service with no app
  context declares `on_events`, it is ignored and logged once with
  `tracing::warn!` because events are app-scoped (P15).
- `state.running` is guarded by an RAII guard / `JoinHandle` tracking so that
  a task panic or cancellation resets `running` to `false` and does not wedge
  the worker (P6).
- **The wake reply contract** (ADR-0031). A wake method's reply may hold
  two fields, and the worker reads only these: `again: true` makes it call
  once more on the next 1-second tick, not after `every_secs` (P14); each
  string in `warnings` is logged with `tracing::warn!`. A reply without
  them is fine. `transaction.drive` sets `again` only after a full batch
  with rows left (P18), so a backlog drains at about 8 conversations per
  second and an idle node calls once per `every_secs`.
- A burst of messages gives one call per second at most: the 1-second
  loop is the debounce.
- The tick of the loop is 1 second. `every_secs` is at least 5, so a
  timer wake is never finer than the loop.
- The targets are read again on each loop, so a deploy or undeploy takes
  effect within a second. A target whose service is gone is dropped with
  its state.
- The worker logs with `tracing`, as the other substrate workers do. It
  never fails the substrate. If the node runs in coordinator mode where
  `connection_router.proxy()` is `None`, worker spawning is skipped (P15).
- A config switch is not added. Tests run with the wake on, as a real node
  does (D-C3c-4).

**D-C3c-3. The linked build gets the same wake.** `init_roym` does not
read `roym.toml` today. It registers the wake of `transaction` from the
manifest text itself (`include_str!` of `crates/roym_core/app/roym.toml`,
parsed with the same `SynAppManifest` type the tests use,
[common/roym.rs:396](../../../../crates/substrate/tests/common/roym.rs#L396)),
so there is one source. A test parses the manifest and checks that every
service that declares a wake is registered by `init_roym` (the same idea as
the service-list test of C3b, D-C3b-15). If the substrate's `roym` feature
does not already link `toml`, put the parse behind a small function in
`syneroym-roym-core` that returns the wake specs; check which crate already
links `toml` first.

**D-C3c-4. Tests that read sync counts.** The seven substrate sites of
section 2.2, point 10 assert the state that follows (the agreement or the
filed card), not `filed` or `countersigned`. The parity harness is not
changed. No test turns the wake off.

**`transaction.drive`** (in `crates/roym_transaction/src/app/drive.rs`,
new):

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveInput {}

#[derive(Debug, Serialize)]
pub struct DriveOutput {
    pub synced: u32,          // conversations synced by this call
    pub pending: u32,         // conversations left for the next call
    pub rescanning: bool,     // a full pass after `reload` is still running
    /// The wake reply contract (ADR-0031): call again on the next loop tick.
    /// True only when this call took a full batch and rows are left (P18).
    pub again: bool,
    /// The wake reply contract: the worker logs each one with `tracing`.
    /// The guest has no logger of its own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// One row of `DRIVE_PENDING`, keyed by the conversation id.
#[derive(Debug, Serialize, Deserialize)]
struct PendingRow {
    since: u64,
    #[serde(default)]
    failures: u32,
}

pub const MAX_DRIVE_CONVERSATIONS: usize = 8;
pub const MAX_DRIVE_FAILURES: u32 = 5;
pub const DRIVE_EVENT_READ: u32 = 500;
pub const DRIVE_RESCAN_PAGE: u64 = 16;
```

In `SyncOutput` ([sync.rs:49](../../../../crates/roym_transaction/src/app/sync.rs#L49)),
add `pub more: bool`, so `drive` knows when a conversation has further
unread messages (P13). It is true only when `scanned == SYNC_WINDOW` **and**
the stored `last_seq` grew in this sync (P18). `sync_at` sets `last_seq`
back to just before the first declined or deferred message
([sync.rs:105](../../../../crates/roym_transaction/src/app/sync.rs#L105)
to `:122`). So a full window that starts with such a message reads the same
window again on every sync. Then `more` is false, and the row is not kept
for a quick re-run.

```text
drive_at(host, now):
    recover_deciding_rows(host, now)  // P14: node-wide recovery runs ONCE per drive, not per conversation
    state = DRIVE_STATE["drive"]  // {event_cursor: Option<String>,
                                  //  rescan_offset: Option<u64>}
                                  // (a new collection, local, not backed up; registered in ensure_collections, P7)
    match AppEvents::read(host, state.event_cursor, DRIVE_EVENT_READ):
        events(list, next):  for e in list where e.kind == "conversation.message":
                                 put DRIVE_PENDING[e.subject] = {since: now, failures: 0}
                             state.event_cursor = next
        reload(head):        state.event_cursor = head
                             state.rescan_offset = Some(0)   // events were lost
    if let Some(off) = state.rescan_offset:
        page = call_sibling(CONVERSATION, "conversation.list",
                            {kind: "direct", offset: off, limit: DRIVE_RESCAN_PAGE})
        put DRIVE_PENDING for each id in page
        state.rescan_offset = if page.len() < DRIVE_RESCAN_PAGE { None } else { off + page.len() }
    add the conversations that hold timed work (below) to the candidates
    // P7: sort in memory, oldest `since` first (the data layer query has no sort order)
    sort candidates in memory by `since` ascending
    take at most MAX_DRIVE_CONVERSATIONS from candidates
    for each (conv, cand): match sync_direct(host, conv, now):
                  Ok(out) if out.more -> put DRIVE_PENDING[conv] = {since: now, failures: 0}
                                         // P13, P18: more to read, and last_seq grew
                  Ok(_) | Err(NotDirect) ->
                      // P13: compare-and-delete: delete only if `since == cand.since`,
                      // so a mark put by a concurrent call is kept
                      delete DRIVE_PENDING[conv] if since == cand.since
                  Err(other) ->          // P18: never retried at the head of the queue
                      if cand.failures + 1 >= MAX_DRIVE_FAILURES:
                          delete DRIVE_PENDING[conv] if since == cand.since
                          warnings.push("gave up on <conv>: <code>")
                      else:
                          put DRIVE_PENDING[conv] = {since: now, failures: cand.failures + 1}
                          // only if since == cand.since; else a newer mark is kept
    pending = count of DRIVE_PENDING
    again = synced == MAX_DRIVE_CONVERSATIONS && pending > 0       // P18
    store state; return {synced, pending, rescanning, again, warnings}
```

- **A row that keeps failing** (P18). On an error other than `NotDirect`,
  the row is put again with `since = now` and one more failure. So it moves
  to the end of the queue and never blocks the other rows. After
  `MAX_DRIVE_FAILURES` (5) it is deleted, and `warnings` names it; the
  worker logs it. A new message event in that conversation puts the row
  again with `failures: 0`. Example: a conversation that was deleted is
  refused by `conversation.changes` on every sync; it is tried five times,
  then dropped.
- **When the worker calls again at once** (P14, P18). Only when `again` is
  true: the call took a full batch and rows are left. A call that synced
  fewer than 8 conversations has no backlog to work through, so the worker
  waits for the next event or for `every_secs`. So rows that fail, or a
  window that does not move, never make the wake run every second.

- **Timed work** is work that needs a sync with no new message: a quote
  that expires (`tick`), a release retry, and in C3c2 an as-listed
  enquiry whose consent time passes. The interval wake (30 s) gives these a
  trigger. `recover_deciding_rows` runs once at the start of `drive_at`
  (P14), avoiding 8 duplicate node-wide scans. For per-conversation work,
  `drive` adds to candidates every conversation holding an unfinalized
  agreement or open as-listed row, up to `MAX_DRIVE_CONVERSATIONS` in
  round-robin order in `DRIVE_STATE`.
- **Collections in `ensure_collections`** (P7). `DRIVE_STATE`,
  `DRIVE_PENDING`, and `as_listed` are registered in `ensure_collections`
  ([app.rs:335](../../../../crates/roym_transaction/src/app.rs#L335)).
- **`sync_direct`** is `sync_at` with a typed refusal for a group
  conversation. Today `sync_at` answers `invalid_params` with a text
  ([sync.rs:86](../../../../crates/roym_transaction/src/app/sync.rs#L86)).
  Split it: `sync_at` keeps its reply; the inner function returns
  `SyncError::NotDirect`, and `drive` matches on that, never on the text
  (C1 R3).
- **Recovery runs once per `drive`, not once per sync** (P14, P21). Today
  `sync_at` calls `recover_deciding_rows` at the end of every sync
  ([sync.rs:135](../../../../crates/roym_transaction/src/app/sync.rs#L135)),
  and the recovery reads every agreement of the node. So the call moves out
  of the inner function: `sync_direct` (the inner function) does not call
  it, and `sync_at` calls it after the inner function returns. So
  `transaction.sync` keeps today's behaviour, and one `drive` runs the
  recovery once, at its start. `confirm_own_reservations` and
  `tick_conversation_at` stay in the inner function: they are per
  conversation.
- **Proactive card extraction** (P4). `sync.rs` is at 759 lines. To prevent
  breaching the 800-line limit across C3c1 and C3c2, proactively
  extract `file_enquiry_card` and `file_quote_card` into sibling file
  `crates/roym_transaction/src/app/sync/cards.rs` (~140 lines saved).
- **The bound.** One `drive` call syncs at most 8 conversations and reads
  one page of events and at most one page of conversations. Each sync reads
  one window of messages (`SYNC_WINDOW`). **Stop rule:** in C3c1 step 4,
  measure one `drive` call that syncs 8 conversations with one card each, on
  a node with 200 agreements (P14), in the WASM build (parity harness), and
  record the time. If it is over 2 seconds, lower `MAX_DRIVE_CONVERSATIONS`
  until it is under, and say so in the Deviations.
- **Events of `transaction` itself** (`transaction.card`,
  `transaction.progress`) are not in `on_events`, so a drive does not wake
  itself.
- **`drive` is not in the backup.** `DRIVE_STATE` and `DRIVE_PENDING` are
  local caches: after a restore, the first `reload` starts a full pass.

### C3c2, the owner's side

**D-C3c-5. `as_listed` in the enquiry** (`crates/roym_core/src/as_listed.rs`,
new):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsListed {
    /// The offering version the person agreed to.
    pub record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<SlotChoice>,
    /// Reserved. Always absent in this build; `validate` refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// The consent holds until then.
    pub valid_until_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotChoice {
    pub slot_id: String,
    pub window: TimeWindow,
}

pub const MAX_CONSENT_SECS: u64 = 7 * 86_400;
```

- `EnquiryPayload` gets `#[serde(default, skip_serializing_if =
  "Option::is_none")] pub as_listed: Option<AsListed>`
  ([transaction.rs:129](../../../../crates/roym_core/src/transaction.rs#L129)).
- `EnquiryPayload::validate`: when `as_listed` is set, `listing_id` must be
  set (`as-listed-needs-listing`); `variant` must be absent
  (`variant-not-supported`); `slot.slot_id` must equal
  `derive_slot_id(listing_id, window.earliest_secs, window.latest_secs)`
  (`slot-differs`). `valid_until_secs` is checked against the issue time by
  the verifier: after `issued_at_secs`, at most `MAX_CONSENT_SECS` later.
- **This is a deviation** from the agreements doc's `slot_id: string?`
  (`:290`): the counterparty must build `c1.window` (section 2.2, point 4),
  and the person must see the time. The Deviations get a row.

**D-C3c-6. `terms_from_listing`** (`crates/roym_core/src/as_listed.rs`):

```rust
pub fn terms_from_listing(
    listing: &ListingPayload,
    owner_did: &str,          // the listing issuer; checked, not copied
    as_listed: &AsListed,
    now_secs: u64,
) -> Result<DealTerms, NotComplete>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotComplete {
    PriceNotFixed, TaxNotIncluded, AtReceiver, SlotRequired, SlotNotOffered,
    NoDealTerms, ListingNotActive, QuoteTooLarge,
    Invalid(&'static str),    // a mapping bug; see "Last, it validates"
}
impl NotComplete { pub fn code(self) -> &'static str; }  // "price-not-fixed", …

/// The listing-only part, without a slot: what a directory can know.
pub fn as_listed_ready(listing: &ListingPayload) -> Result<(), NotComplete>;
```

- The mapping is the agreements doc's table (`:258` to `:279`), in that
  order of fields. Two notes on today's types:
  - `location.where` is `ServiceLocation` (`at-giver`, `at-receiver`,
    `remote`, [listing.rs:117](../../../../crates/roym_core/src/listing.rs#L117));
    `QuoteLocation.where_` is the same type
    ([transaction.rs:119](../../../../crates/roym_core/src/transaction.rs#L119)),
    so `c1.location = QuoteLocation { where_, area: None, address: None }`.
    `at-receiver` is refused before this point.
  - `PaymentTerms.methods` is a `Vec`; an empty one maps to `None`.
- **Refusals, in this order** (the first one wins, so the code is
  stable): `ListingNotActive` (`status` is not `active`);
  `PriceNotFixed` (`payment` absent, or `model` not `fixed`, or no
  `amount_minor`, or `payment.payee.trim()` is empty, P9); `TaxNotIncluded`
  (`tax_included` false); `AtReceiver` (`location.where_` is `at-receiver`);
  `NoDealTerms` (`deal_terms` absent); `SlotRequired` (the extension has a
  reserve hook and `slot` is absent); `SlotNotOffered` (`slot` set and the
  extension has no reserve hook, or there is no extension); `QuoteTooLarge`
  (below). `as_listed_ready` runs the first five (including the non-empty
  payee check, P9). The `scheduling` name is never written here: "has a
  reserve hook" is a registry question (`Registry::workflow`, used since
  C3b1).
- **Last, it validates.** It runs `DealTerms::validate`
  ([deal.rs:333](../../../../crates/roym_core/src/deal.rs#L333)) and the
  size check through `DealTerms::disclosure_size`
  ([sd.rs:86](../../../../crates/roym_core/src/deal/sd.rs#L86)) against
  `MAX_RECORD_DISCLOSURE_BYTES` and `MAX_DETAILS_DISCLOSURE_BYTES`
  ([disclosure.rs:20](../../../../crates/roym_core/src/disclosure.rs#L20)).
  `disclosure_size` needs no salts. A size over a limit gives
  `QuoteTooLarge`. A `validate` error would be a bug in the mapping. It is
  returned as a ninth variant, `Invalid(&'static str)`, which carries the
  `DealError` code, so a person sees a normal refusal and never a quote
  that `quote.set` refuses. The test `a_built_quote_never_fails_validate`
  (section 7.1) checks that `Invalid` never happens for a listing that
  `ListingPayload::validate` and `as_listed_ready` accept.
- **The amount text.** `money::plain_amount(amount_minor, currency) ->
  Option<String>` (new, `money.rs`): the minor units written with `.` as
  the decimal sign and exactly `currency_minor_exponent` digits after it,
  no grouping; `None` for an unknown currency. `c2.description.text` is
  `format!("{currency} {plain_amount}")`, and the same text for each
  translation language of the listing (AG-39: one set of languages per
  record).
- **The expiry.** `quote_expires_at_secs = now_secs +
  deal_terms.quote_valid_secs`. `listing.set` already bounds
  `quote_valid_secs` (D-C3c-8), so no clamp is needed here; a test checks
  the bound holds at the top value.
- **One test per mapping row and per refusal**, and the largest-listing
  test of the agreements doc (`:256`): five languages, every text at its
  limit; it passes `validate` and the size check.

**D-C3c-7. `description` becomes optional with `as_listed`.** The field
becomes `Option<LocalText>` (`#[serde(default, skip_serializing_if =
"Option::is_none")]`). `validate` requires it when `as_listed` is absent
(`description-required`, a new code). Every reader changes; find them all
with `git grep -n 'description' -- crates/roym_transaction/src crates/roym_web/ui/src/cards/templates/enquiry.ts crates/roym_web/ui/src/screens apps/roymctl/src crates/roym_web/tests`
and read each hit. Known ones: the enquiry card template
([enquiry.ts](../../../../crates/roym_web/ui/src/cards/templates/enquiry.ts)),
the enquiry form's required-description check
([messages.ts:461](../../../../crates/roym_web/ui/src/screens/messages.ts#L461)),
the enquiry view in `enquiry_ops.rs`, and the parity fixtures. With no
description, the enquiry card shows the offering's title from the stored
row on the counterparty's node, and "Agree as listed: <listing id>" on the
owner's node (the owner's Hub reads its own listing).

**D-C3c-8. The listing's new fields**
([listing.rs:179](../../../../crates/roym_core/src/listing.rs#L179)):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Acceptance { Instant, #[default] ConfirmFirst }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentTiming { BeforeWork, AfterWork }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingDealTerms {
    pub cancellation_terms: LocalText,
    pub refund_terms: LocalText,
    pub dispute_path: LocalText,
    pub quote_valid_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_timing: Option<PaymentTiming>,
}

// in ListingPayload:
#[serde(default, skip_serializing_if = "is_default")] pub acceptance: Acceptance,
#[serde(default, skip_serializing_if = "is_false")]   pub not_limited: bool,
#[serde(default = "default_instant_cap", skip_serializing_if = "is_default_cap")]
pub instant_cap: u16,                                  // 1..=500, default 20
#[serde(default, skip_serializing_if = "Option::is_none")]
pub deal_terms: Option<ListingDealTerms>,
```

- `roym_core` has no `PaymentTiming` type any more (C1 replaced it with
  `after`). The new one lives in `listing/as_listed.rs`.
- **`ListingPayload::validate`** gets, in a new file
  `crates/roym_core/src/listing/as_listed.rs`:
  - `instant` needs `relationship.open_to == anyone`
    (`instant-not-allowed`, reason `not-open-to-anyone`), and an extension
    with a reserve hook or `not_limited` (`instant-not-allowed`, reason
    `not-limited-or-slot`). A listing with no `relationship` block is not
    open to anyone. The editor sends the block only when its toggle is on
    ([editor.ts:323](../../../../crates/roym_web/ui/src/listings/editor.ts#L323)),
    so the form's "Agree as listed" section turns the block on with
    `anyone` when the owner picks `instant`.
  - `instant_cap` in 1..=500 (`instant-cap-range`).
  - `deal_terms` texts: each `LocalText` is checked with `MAX_TERMS_TEXT_LEN`
    ([deal.rs:30](../../../../crates/roym_core/src/deal.rs#L30)); each
    `lang` equals the listing's `lang`; each set of translation languages
    equals the set of the listing's `translations`
    (`deal-terms-languages`). This is AG-39's rule, so a quote built from
    the listing keeps one set.
  - `quote_valid_secs` in `MIN_QUOTE_LIFETIME_SECS..=7 days`
    (`quote-valid-range`). `MIN_QUOTE_LIFETIME_SECS` is already in
    `roym_core::transaction` (`quote_ops.rs:28` imports it), so both use
    one value.
- `ListingError::code()` ([listing.rs:283](../../../../crates/roym_core/src/listing.rs#L283))
  returns the new codes, and `instant-not-allowed` carries its reason, so
  `listing.set` answers `{code, reason}` in `data`.
- `catalog`'s `SetListingInput`
  ([listing_ops.rs:82](../../../../crates/roym_catalog/src/app/listing_ops.rs#L82))
  gets the four fields with `#[serde(default)]`; `build_payload` (`:397`)
  copies them. `catalog` cannot check signing (`:305`).

**D-C3c-9. `code` in `deal-decline`.** `DealDeclinePayload` gets
`#[serde(default, skip_serializing_if = "Option::is_none")] pub code:
Option<String>` (visible). `validate` allows only the codes of the
reason-code table that a decline may carry: `consent-expired`,
`offering-withdrawn`, and (Q8) `no-such-slot`, `slot-not-in-listing`.
`DealDeclineInput` does not take a code: a person's decline has none. The
owner's node sets it through an internal function
`decline_ops::decline_with_code(host, declines, code, now)`, the same path
as `decline_at`
([decline_ops.rs:133](../../../../crates/roym_transaction/src/app/decline_ops.rs#L133))
with the code passed in. The fence `decline:<deal>` makes it once.

**D-C3c-10. `formed_as` in every snapshot.**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormedAs { Quote, AsListedInstant, AsListedConfirmed }
```

- `AgreementProgressPayload` gets `pub formed_as: FormedAs` (required,
  no default) ([progress.rs:76](../../../../crates/roym_core/src/progress.rs#L76)).
- `progress::open` (`:174`) takes it; `progress::apply` (`:214`) copies it
  from the previous snapshot and never changes it.
- `AgreementRow` ([app.rs:158](../../../../crates/roym_transaction/src/app.rs#L158))
  gets `formed_as: Option<FormedAs>` (`None` reads as `Quote`).
- The writer learns it from the quote pointer: `RecordPointerRow`
  ([app.rs:136](../../../../crates/roym_transaction/src/app.rs#L136)) gets
  `#[serde(default)] formed_as: Option<FormedAs>`, written by the as-listed
  answer (D-C3c-12) on the owner's node. `finalize_agreed_deal` and
  `handle_not_formed` ([decide.rs:438](../../../../crates/roym_transaction/src/app/decide.rs#L438),
  `:620`) read it when they call `open`.
- The counterparty's filing (`file_progress_card` in `sync/receipts.rs`)
  refuses a snapshot whose `formed_as` differs from the one it holds for
  that agreement (`formed-as-changed`, filed as a refused card, as a fork
  is).
- A `not-formed` snapshot also carries it: the field says how the writer
  answered, also when no agreement formed.
- **Workspace-wide compilation in Step 1 (P3):** struct literal instantiations
  and fixtures of `AgreementProgressPayload` (with `formed_as: FormedAs::Quote`),
  `EnquiryPayload` (`as_listed: None`, `description: Some(...)`), and
  `DealDeclinePayload` (`code: None`) across `roym_transaction` (production
  calls like `progress::open` in `decide.rs:620` and tests) and `roym_core` are
  updated in C3c2 Step 1 so `cargo check --workspace` compiles cleanly before
  Step 2.

**D-C3c-11. `can_agree_as_listed` on a directory hit.**
`build_verified_row`
([verify.rs:118](../../../../crates/roym_federated_query/src/verify.rs#L118))
has the verified payload. It sets `Row.can_agree_as_listed =
as_listed_ready(&payload).is_ok()`; `build_hits`
([merge.rs:211](../../../../crates/roym_federated_query/src/merge.rs#L211))
copies it to `MergedHit` (`#[serde(default)] pub can_agree_as_listed:
bool`); the Hub's `MergedHit`
([search.ts:16](../../../../crates/roym_web/ui/src/directory/search.ts#L16))
gets the field. `as_listed_ready` does not build `DealTerms`, so
`directory` links no deal code for it. **Stop rule:** measure `directory`
after this step alone; it has no new record code, so its limit is +40 KB.
Over the limit, stop and tell the owner.

**D-C3c-12. The owner's answer** (`crates/roym_transaction/src/app/as_listed_ops.rs`,
new, with `as_listed_ops/answer.rs`, `as_listed_ops/status.rs`).

When it runs:

- **The driver:** `file_enquiry_card`
  ([sync.rs:475](../../../../crates/roym_transaction/src/app/sync.rs#L475))
  files an enquiry with `as_listed` for one of this node's listings, then
  calls `answer_as_listed(host, enquiry, Trigger::Driver, now)`. The same
  call runs for every held as-listed enquiry with no `answer:` row in the
  `drive` pass (the timed work of D-C3c-1), so an enquiry that waited for a
  cap place or for signing is answered when the reason goes.
- **The owner:** `quote.from-listing { enquiry_record_id }` calls
  `answer_as_listed(…, Trigger::Owner, now)`.

```text
answer_as_listed(host, enquiry, trigger, now) -> Answer:
    require: this node's owner issued the listing named by enquiry.listing_id,
             and the enquiry's counterparty is not the owner
    if a row answer:<enquiry_record_id> exists:  return its stored outcome
                                                 (resend an unsent card first)
    current = call_sibling(CATALOG, "listing.get", {listing_id})   // Q5
    if current is none or current.status == withdrawn:
        return decline(code "offering-withdrawn")
    if now > as_listed.valid_until_secs:
        return decline(code "consent-expired")
    payload = verify_envelope(current.envelope, now)               // own data, still verified
    terms = terms_from_listing(payload, owner, as_listed_for(current), now)
        // as_listed_for: the enquiry's as_listed, with record_id replaced by the
        // current version's. So a changed offering gives a quote from the current
        // version, which the counterparty's check will not accept (it built from
        // the version it stored).
        Err(reason) -> return Waiting(reason)    // the owner answers by hand
    // P16, P20: the prepare hook runs here, and only here (not in issue_quote)
    match check_author_slots(host, &terms):
        Refused(code) -> return decline(code)    // Q8: no-such-slot, slot-not-in-listing
        Unavailable   -> return Waiting(SlotCheckFailed)   // the next drive tries again
        Ok            -> continue
    if trigger == Driver:
        if payload.acceptance == confirm-first:                                  return Waiting(ConfirmFirst)
        // P12: count open agreements + in-flight quotes against instant limits
        if open_instant(listing) + in_flight_instant(listing) >= instant_cap:     return Waiting(CapReached{cap})
        if open_instant_with(listing, counterparty) + in_flight_instant_with(listing, counterparty) > 0: return Waiting(CounterpartyHoldsOne)
        if person_principal(host, now) is Err(e):                                return Waiting(SigningNotReady(e))
        formed_as = AsListedInstant
    else:
        formed_as = AsListedConfirmed
    quote_id = derive_quote_id(conversation, owner, next_sequence)
    create answer:<enquiry_record_id> = {state: "answering", quote_id, formed_as, at_secs}
        exists -> return its stored outcome            // another sync won
    out = issue_quote(host, enquiry_record_id, quote_id, terms, formed_as, now)
    put answer:<enquiry_record_id> = {state: "answered", quote_id, record_id}
    return Quoted(out)
```

- **One quote per tap.** The fence `answer:<enquiry_record_id>` is created
  before any signature, with the quote id fixed in it. A second sync that
  loses the fence signs nothing. A crash between the fence and the
  `answered` put leaves `answering` with a fixed quote id: the next call
  runs `issue_quote` again with the same id, and `issue_quote` returns the
  stored quote when `QUOTES[quote_id]` exists, else signs it. The
  writer's own fence `decision:<quote_id>` then allows at most one
  agreement for that id (C1b).
- **`issue_quote` takes finished `DealTerms`** (P20). It is the body of
  `set_at` from `validate_and_conceal_terms` on
  ([quote_ops.rs:187](../../../../crates/roym_transaction/src/app/quote_ops.rs#L187)
  to `:241`), moved into a function that takes the terms, the quote id,
  the sequence, `supersedes`, the version count and `formed_as`. It does
  not run the prepare hook. `set_at` keeps everything before that line as
  it is today, `prepare_terms` included, and then calls `issue_quote`. So
  there is one signing path for every quote, and `quote.set` runs the hook
  once, as today: a normal `quote.set` for a removed slot still returns
  the hook's error to the owner and declines nothing. The existing
  `quote.set` test for a removed slot is the guard; keep it unchanged.
  `issue_quote` writes `formed_as` into the quote's `RecordPointerRow`
  (D-C3c-10; `None` from `set_at`). It goes in `quote_ops/issue.rs` (new),
  because `quote_ops.rs` is at 695 lines.
- **`check_author_slots`** (P16, P20; in `as_listed_ops/answer.rs`). Only
  `answer_as_listed` calls it. `DealTerms.commitments` is
  `Vec<Commitment>` ([deal.rs:148](../../../../crates/roym_core/src/deal.rs#L148)),
  and `hooks::prepare` takes a `&CommitmentInput`
  ([hooks.rs:280](../../../../crates/roym_transaction/src/app/hooks.rs#L280)).
  The hook reads only three fields of it: `extension`, `details` and
  `window`. So split `hooks::prepare` in two: `prepare_window(host,
  extension, details, window, listing_id)` does the work, and `prepare`
  becomes a two-line wrapper that passes the `CommitmentInput`'s fields.
  No conversion from `Commitment` to `CommitmentInput` is needed.
  `check_author_slots` calls `prepare_window` for each commitment with
  `giver == QuoteAuthor` and `has_prepare_hook(extension)`
  ([hooks.rs:52](../../../../crates/roym_transaction/src/app/hooks.rs#L52)):
  - the hook refuses (a `Refused` error, whose `data.code` is
    `no-such-slot` or `slot-not-in-listing`): `Refused(code)`, and the
    enquiry is declined with it (Q8). A refusal with any other code is
    declined with `slot-not-in-listing`, so the decline code stays in the
    allowed list (D-C3c-9);
  - the returned window differs from the commitment's window (built from
    `as_listed.slot`): `Refused("slot-not-in-listing")`;
  - the hook cannot be reached (an internal error): `Unavailable`. The
    enquiry waits, nothing is written, and the next `drive` tries again.
  It runs before the `answer:` fence and before the instant checks: it
  only reads, and so a removed slot is declined at the next drive in both
  modes, not only when the owner confirms. A slot removed after this check
  and before the counterparty accepts ends `not-formed` with `slot-taken`,
  as any full slot does (AG-14).
- **"Open" for the limits and in-flight quotes** (`:313`, P12): an agreement
  on this node with `formed_as == AsListedInstant`, `terms.listing_id == listing`,
  that is not final, has no settlement half in any commitment's `halves`, and
  whose `agreed_at_secs` is less than 7 days ago. In addition,
  `in_flight_instant(listing)` counts `answer:` rows on this node with
  `formed_as == AsListedInstant` and state `answered`, whose quote has no
  `decision:` fence yet and has not ended:
  `is_deal_ended(quote_id, quote_expires_at_secs, now)` is false
  ([app.rs:663](../../../../crates/roym_transaction/src/app.rs#L663);
  P19). That helper already covers expiry, a decline (`declined_at_secs`
  or `decline:<deal>`), a `not-formed` decision and a terminal agreement.
  So a quote the counterparty declined frees its place at once; a decline
  writes `decline:<quote>`, not `decision:`, so the fence check alone
  would miss it. The two instant limits check `open_instant +
  in_flight_instant`. A quote that is never answered holds a place until it
  expires (up to 7 days), so rapid taps cannot pass the cap before
  acceptance.
  `open_instant` scans the `agreements` collection with the existing paging
  helper; the count is small (the cap is at most 500). A unit test of the pure
  predicate `is_open_instant(row, listing, now)` has one case per condition
  (AG-45's unit row).
- **A `Waiting` outcome writes nothing.** The owner's Hub computes the
  reason again with `deal.as-listed-status`.
- **The decline** uses D-C3c-9, and writes the `answer:` row with state
  `declined` and the code, after the decline row exists.
- **A full slot is not a reason to wait** (`:314`): `issue_quote` does not
  reserve; the writer reserves when the counterparty accepts, and a full
  slot ends `not-formed` with `slot-taken` as today (AG-14).
- The `answer:` ledger kind is added to `LedgerKind`
  ([ledger.rs:8](../../../../crates/roym_transaction/src/app/ledger.rs#L8))
  and to the import's check
  ([sections.rs:487](../../../../crates/roym_transaction/src/app/backup/sections.rs#L487)
  to `:546`): it must name a held enquiry; an `answered` row must name a
  held quote that answers it.

**D-C3c-13. `deal.as-listed-status`** (local-only):

```rust
pub struct AsListedStatusInput { pub listing_id: String }
pub struct AsListedStatusOutput {
    pub acceptance: Acceptance,          // from the current version
    pub instant_cap: u16,
    pub open_instant: Vec<String>,       // agreement ids, oldest first
    pub in_flight_instant: u32,          // P12: quotes issued, waiting for accept/expire
    pub signing_ready: bool,
    pub waiting: Vec<WaitingEnquiry>,    // {enquiry_record_id, conversation, reason}
}
```

`reason` is one of `confirm-first`, `cap-reached`,
`counterparty-holds-one`, `signing-not-ready`, `slot-check-failed` (P20),
or a `NotComplete` code. The
owner's Hub shows each, and its "cancel unconfirmed instant agreements"
action calls `agreement.cancel` with `cancel_reason: other` for each id in
`open_instant` (AG-9 applies: one that moved is refused with
`commitment-moved`, and the Hub says how many it cancelled).

**D-C3c-14. `enquiry.set` with `as_listed` (counterparty's node, used by
the tap and by the owner's-side tests).** `EnquirySetInput`
([enquiry_ops.rs:27](../../../../crates/roym_transaction/src/app/enquiry_ops.rs#L27))
gets `#[serde(default)] pub as_listed: Option<AsListedInput>`:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsListedInput {
    pub envelope: String,               // the offering version, as the hit gave it
    #[serde(default)] pub slot: Option<SlotChoice>,
    pub valid_for_secs: u64,            // 60 ..= MAX_CONSENT_SECS
    pub viewed_lang: String,
    pub viewed_via_mt: bool,
}
```

`enquiry.set`:

1. Verifies the envelope (`listing::verify_envelope`); refuses a listing
   of this node's own owner; refuses when the listing's
   `conversation_address` is not this conversation's peer address
   (`as-listed-wrong-conversation`).
2. Runs `terms_from_listing(payload, issuer, as_listed, now)` and refuses
   with `as-listed-not-complete` and the reason in `data.reason`
   (`:647`).
3. Builds the enquiry with `listing_id = payload.listing_id`,
   `as_listed = {record_id, slot, variant: None, valid_until_secs: now +
   valid_for_secs}`, `description` absent, and signs and sends it as
   today.
4. Stores a local row in the new collection `as_listed`, keyed by the
   enquiry id: `{enquiry_record_id, envelope, record_id, slot,
   valid_until_secs, viewed_lang, viewed_via_mt, state: "waiting",
   reason: None, quote_record_id: None}`. The envelope is stored, not the
   payload: the automatic acceptance (D-C3c-17) verifies it again.
5. Is idempotent as today: a repeat with the same input returns the same
   enquiry.

The `as_listed` collection is in `transaction`'s backup (a new section in
`backup/as_listed.rs`, two lines in `backup.rs`); the import checks that
each row names a held enquiry of this node and that its envelope verifies.

**D-C3c-15. `roymctl`.** `roym transaction enquiry` gets `--as-listed
<envelope-file>`, `--slot <id> --slot-window <a>..<b>`, `--valid-hours`
(default 1), and sends `as_listed`; `--description` becomes optional with
it. A new `roym transaction from-listing --enquiry <record id>` calls
`quote.from-listing`. The `viewed_*` fields are sent as the original
language and `false`. Tests in `apps/roymctl/src/commands/roym/tests.rs`
check the params.

**D-C3c-16. The owner's Hub.**

- **Listing form** ([listings.ts:224](../../../../crates/roym_web/ui/src/screens/listings.ts#L224),
  [editor.ts:272](../../../../crates/roym_web/ui/src/listings/editor.ts#L272)):
  a section "Agree as listed" with the three texts (with the listing's
  translation rows), "Quote valid for" (hours), "Payment" (before the work,
  after the work, not set), "Acceptance" (`confirm-first` or `instant`),
  "Never runs out" (`not_limited`) and "Open instant agreements at most"
  (`instant_cap`). `instant` is offered only while
  `transaction.signing-status` answers `installed`; the form calls it when
  it opens (the enrolment code already calls it,
  [enrolment.ts:21](../../../../crates/roym_web/ui/src/session/enrolment.ts#L21);
  reuse that call). The new section goes in a new file
  `listings/as_listed_section.ts`, because `listings.ts` is 693 lines.
- **The enquiry card on the owner's node**: an enquiry with `as_listed`
  shows "Agree as listed" and the reason from `deal.as-listed-status`;
  with reason `confirm-first`, `cap-reached` or `counterparty-holds-one`,
  a "Confirm" button calls `quote.from-listing`. The usual "Answer with a
  quote" stays.
- **The listing page**: the line "This offering is waiting for you: it
  reached its limit of N open instant agreements." when any waiting reason
  is `cap-reached`, and the action "Cancel unconfirmed instant agreements"
  when `open_instant` is not empty, with a confirm dialog that lists them.
- **The decline card** ([deal_decline.ts:13](../../../../crates/roym_web/ui/src/cards/templates/deal_decline.ts#L13))
  shows the code's sentence when `code` is set.
- **The progress card** shows nothing for `formed_as` in C3c (M08G
  decides how a reader uses it); the agreement view carries it.

### C3c2, the counterparty's side

**D-C3c-17. The automatic acceptance** (`crates/roym_transaction/src/app/as_listed_ops/accept.rs`,
new).

When it runs: `file_quote_card`
([sync.rs:525](../../../../crates/roym_transaction/src/app/sync.rs#L525))
files a quote whose `answers.record_id` is an enquiry of this node with an
`as_listed` row in state `waiting`, and calls `auto_accept(host, row,
quote, now)`. The `drive` pass also calls it for each `waiting` row that
has a held quote and no `autoaccept:` row (for example after a signing
error).

```text
auto_accept(host, row, quote_record_id, now):
    // all checks run here, immediately before signing (AG-49)
    quote = load the verified quote and its stored disclosures
    if quote.issued_at_secs > row.valid_until_secs:   -> shown("consent-expired")
    if the enquiry is declined or withdrawn (its pointer's declined_at_secs,
       or a held deal-decline of it):                  -> shown("withdrawn")
    if quote is not the latest version held for its quote id:
                                                       -> shown("superseded")
    if any disclosure is missing or does not verify:   -> shown("not-disclosed")
    listing = verify_envelope(row.envelope, now)       // the version stored at the tap
    built = terms_from_listing(listing, quote.issuer, row.as_listed(), quote.issued_at_secs)
        Err(_) -> shown("not-complete")
    if !same_terms(built, quote.plain_terms, quote.issued_at_secs, now):
                                                       -> shown("terms-differ")
    create autoaccept:<enquiry_record_id> = {quote_record_id, at_secs}
        exists with this quote -> continue (crash repeat)
        exists with another quote -> shown("already-accepted")   // one tap, one acceptance
    accept_at(host, {quote_record_id, viewed_lang: row.viewed_lang,
                     viewed_via_mt: row.viewed_via_mt}, now)
        Ok -> put row state "accepted"
        Err(signing error) -> keep state "waiting", reason "signing-not-ready";
                              the next drive tries again
        Err(other code) -> shown(code)
shown(reason): put row state "shown", reason; the Hub shows a normal quote with "Accept"
```

- **`same_terms`** compares the two `DealTerms` as one value with
  `PartialEq` after one change: `built.quote_expires_at_secs` is replaced
  by the quote's, and the quote's must lie after `now` and at most 7 days
  after `issued_at_secs` (`:283`). Nothing else is excepted.
- **The struct test** of AG-44: a test that serialises a `DealTerms` and a
  `Commitment` built with every field set, walks the JSON keys, and checks
  that each key has a row in a fixed table of the test (the field and the
  case that changes it). A new field with no row fails. For each row, the
  test changes that one field in the quote and checks that `same_terms` is
  false.
- **The `autoaccept:` fence** makes one acceptance per tap, also when two
  quote ids answer one tap (an owner who also answers by hand). It is a new
  ledger kind with the import check: it must name a held enquiry and a
  held quote that answers it.
- **`accept_at`** is today's path
  ([agreement_ops.rs:135](../../../../crates/roym_transaction/src/app/agreement_ops.rs#L135))
  with all its checks. The counterparty's commitment in v1 is `currency`,
  so it reserves nothing.
- **Only the writer's records follow.** The rest of the deal is the flow
  of C1 to C3b.

**D-C3c-18. The tap on the Hub** (`crates/roym_web/ui/src/screens/as_listed_tap.ts`,
new).

- **The button.** A verified hit with `can_agree_as_listed` whose listing
  names no extension with a reserve hook (Q4; the Hub reads the hooks from
  its manifest registry) shows "Agree as listed" beside "Ask"
  ([directory.ts:446](../../../../crates/roym_web/ui/src/screens/directory.ts#L446)).
- **Before the tap.** The Hub reads the envelope with `directory.run-envelope`
  and the payload with `listing.verify` on its own node. It shows, in the
  viewed language: the price, "will be paid to …" (the payee, never as
  agreed terms), the cancellation, refund and dispute texts, the payment
  timing, how long the owner's quote stays valid, and how long the tap
  holds (1 hour for `instant`, 48 hours for `confirm-first`, `:319`). If
  any is missing, it shows "Ask" alone. When the hit is shown in a machine
  translation, the existing warning (`mtWarning`,
  [messages.ts:634](../../../../crates/roym_web/ui/src/screens/messages.ts#L634))
  comes first and the person confirms it.
- **The tap** opens the conversation (`conversation.open`, as "Ask" does)
  and calls `enquiry.set` with `as_listed`.
- **The waiting, ended and refused lines** on the enquiry card, from the
  `as_listed` row (D-C3c-14) and the enquiry's delivery state:
  "Sent, waiting for the owner's node" while `waiting` and the consent
  holds; "No answer, the request ended" with a new tap action after
  `valid_until_secs`; "The owner's node did not accept this conversation"
  for a refused first contact (`rate-limited`, `blocked`, from
  `conversation.delivery-status`); the decline code's sentence; and, for
  `shown`, a normal quote with "Accept" and the line "The terms differ from
  the offering you agreed to. Read them before you accept." The deal is
  never shown as agreed before the writer's snapshot says `agreed`.

## 6. Step-by-step changes

Each step builds, and its tests pass, before the next starts.

### 6.1 C3c1 — The driver

#### Step 1 — Commit this plan, and the ADR

Write `docs/decisions/0031-node-local-wake.md` (Q3), from D-C3c-1 and Q2.
The ADR records that the wake is strictly node-local (never routed via the
remote/community registry, P1), that coordinator mode skips worker spawn,
that `on_events` for a standalone service with no app context is ignored
and logged once (P15), and the wake reply contract (`again`, `warnings`,
P18). The owner accepts it before step 2.

#### Step 2 — `WakeSpec` and the manifest (`app_orchestration`, pure)

D-C3c-1: the type, its validation, the `ServiceConfig` field, the manifest
validation call. Unit tests: one per validation rule; a manifest with a
wake round-trips; `ServiceConfig` with no wake serialises as before (the
deploy hash of every existing service stays the same).

#### Step 3 — The deploy fact and the worker (`control_plane`, `core`, `substrate`)

In `crates/core/src/storage.rs`, replace the `DeployFacts` tuple with struct
`DeployFacts { pub service_type: String, pub health_check_json: Option<String>, pub manifest_hash: Option<String>, pub visibility: Option<String>, pub wake_json: Option<String> }`
and update the `RegistryStorage` trait and in-memory store (P10). In
`crates/data_db/src/registry_store.rs`, update `save_deploy_facts` and add
the column migration `ALTER TABLE service_deploy_facts ADD COLUMN wake_json TEXT`
(P2). Update mock storage in `failing_storage.rs` and `service_ownership.rs` (P2).
In `crates/control_plane/src/service/orchestration/lifecycle.rs:193`
(`clear_deploy_hash_after_binding_push`), copy `wake_json` (P10). Cache `wake_json`
in `LocalRegistry.service_deploy_facts`.

Implement `wake.rs` with `WakeWorker`: pass `shared.event_bus()` and endpoint
registry to `RuntimeServices`; skip worker when proxy is `None` (P15);
verify local endpoint exists before dispatching (`registry.lookup().is_some()`,
P1); guard `state.running` with RAII/scopeguard so task panic or cancellation
resets `running = false` (P6); read the reply's `again` to call again on
the next 1-second loop tick, and log its `warnings` (P14, P18); spawn worker with
cancel token, store `JoinHandle`, and race in `run_until_shutdown` (P15).
Add linked-build registration (D-C3c-3). Add test `a_binding_push_keeps_the_wake`
(P10).

Before this step, read every writer and every reader of the deploy facts
(P22): `git grep -n set_deploy_facts` and `git grep -n 'deploy_facts('`.
The readers use tuple patterns or `.1`/`.3`, so each one changes to a
field name:

- `control_plane/src/service/orchestration.rs:315`;
- `orchestration/deploy/admission.rs:198`;
- `orchestration/lifecycle.rs:194` (the re-stamp) and `:541`;
- `orchestration/status.rs:34`, `:202` and `:335`;
- `orchestration/tests/deploy.rs:1032`, `:1060` and `:1067`;
- `data_db/src/registry_store/tests.rs:44`, `:92` and `:99`
  (`load_all_deploy_facts`).

`orchestration/cert.rs:122` only checks `is_none()` and does not change.
That is 13 sites in six files, on top of the writers.

#### Step 4 — `transaction.drive` and the manifest line (`transaction`, `roym.toml`)

Proactively extract `file_enquiry_card` and `file_quote_card` from `sync.rs`
into sibling file `crates/roym_transaction/src/app/sync/cards.rs` (~140 lines
saved) to keep `sync.rs` safely below 800 lines (P4). Register `DRIVE_STATE`,
`DRIVE_PENDING`, and `as_listed` in `ensure_collections` (`app.rs:335`) (P7).
Move the `recover_deciding_rows` call out of the inner sync function into
`sync_at` (P21). In `drive_at`, run `recover_deciding_rows` once per drive
call (P14), sort candidate pending rows in memory by `since` (P7), add
`more: bool` to `SyncOutput` (true only when the window was full and
`last_seq` grew, P18) and keep the conversation pending when it is true
(P13), use compare-and-delete on `since` when deleting pending rows (P13),
move a failing row to the end with a failure count and drop it after five
(P18), and set `again` only after a full batch with rows left (P18).

`drive.rs`, the `api.rs` row, the method-contract count (44 → 45,
[contract.rs:83](../../../../crates/roym_web/tests/method_contract/contract.rs#L83)),
its shape file, `[services.transaction.wake]`. Service tests (`tests/drive.rs`).
A parity scenario (252). The time measurement of the stop rule: measure 8
conversations on a node with 200 agreements (P14).

#### Step 5 — Tests that read sync counts, and the two-node test

D-C3c-4; `roym_driver_e2e.rs`. Then `mise run build:roym`, parity, the
Playwright suite.

#### Step 6 — Sizes, docs and close-out of C3c1

Component sizes (only `transaction` changes), living docs (section 9),
backlog (section 8), Deviations, status.md.

### 6.2 C3c2 — Agree as listed

Steps 1 to 5 are the owner's side; steps 6 to 8 are the counterparty's
side.

#### Step 1 — Core types (`roym_core`, pure)

`as_listed.rs` (`AsListed`, `SlotChoice`, `terms_from_listing`,
`as_listed_ready`, `NotComplete`), `money::plain_amount`, the listing
fields and their validation (`listing/as_listed.rs`), `EnquiryPayload`'s
`as_listed` and optional `description`, `DealDeclinePayload.code`,
`FormedAs` and `AgreementProgressPayload.formed_as`. In `terms_from_listing`
and `as_listed_ready`, verify `!payment.payee.trim().is_empty()`, returning
`NotComplete::PriceNotFixed` if empty (P9).

Struct literal instantiations and fixtures of `AgreementProgressPayload`
(with `formed_as: FormedAs::Quote`), `EnquiryPayload` (`as_listed: None`,
`description: Some(...)`), and `DealDeclinePayload` (`code: None`) across
`roym_transaction` (production calls like `progress::open` in `decide.rs:620`
and tests) and `roym_core` are updated here so `cargo check --workspace`
compiles cleanly at Step 1 (P3). Unit tests of section 7.1.

#### Step 2 — `catalog` and `directory`

`SetListingInput`, `build_payload`, the catalog tests; `can_agree_as_listed`
in `roym_federated_query` and the Hub type. Measure `directory` and
`catalog` (stop rule of D-C3c-11).

#### Step 3 — `transaction`: records and filing

`formed_as` in `open`, `apply`, the rows, `file_progress_card`'s check;
`description` readers (D-C3c-7); `enquiry.set` with `as_listed` and the
`as_listed` collection with its backup section (D-C3c-14); `SCHEMA_VERSION`
8; the decline code (D-C3c-9). Service tests for these.

#### Step 4 — `transaction`: the answer

`roym.toml` `depends_on` (Q5): in `wire_roym_topology` in
`crates/substrate/src/runtime/roym.rs`, add the `catalog` binding for
`transaction` so the linked native build can call `listing.get` (P11). Add
a test verifying that every manifest `depends_on` has a matching `save_binding`
in `roym.rs`, and a substrate native test calling `listing.get` from
`transaction` (P11).

`issue_quote` moved out of `set_at` into `quote_ops/issue.rs`, starting at
`validate_and_conceal_terms`; `set_at` keeps `prepare_terms` (P20).
`hooks::prepare` split into `prepare_window` and a thin wrapper (P20).
`check_author_slots` in `as_listed_ops/answer.rs` runs the hook, compares
the window and declines with the hook's code (P16, P20, Q8). In `as_listed_ops` (`answer_as_listed`, `quote.from-listing`,
`deal.as-listed-status`), count in-flight quotes (`in_flight_instant`, P12)
in addition to open agreements against instant limits. Add tests
`many_taps_before_any_acceptance_make_only_cap_quotes` and
`one_counterparty_tapping_twice_gets_one_instant_quote` (P12). Wire
`answer_as_listed` in `sync/cards.rs` (`file_enquiry_card`) and in `drive`;
add the `answer:` ledger kind and its import check. Method-contract count
45 → 47 and shapes. Service tests `tests/as_listed_owner.rs`. Parity scenarios.

#### Step 5 — `roymctl` and the owner's Hub

D-C3c-15, D-C3c-16, the owner's Hindi messages of section 7.10. Hub unit
tests. The owner's half of `roym_as_listed_e2e.rs` (section 7.4: the
counterparty accepts by hand). This is the end of the owner's side: the
code review's first pass covers steps 1 to 5.

#### Step 6 — The automatic acceptance (`transaction`)

D-C3c-17: `auto_accept`, wired in `sync/cards.rs` (`file_quote_card`) and in
`drive` (P4), the `autoaccept:` ledger kind and its import check, the
`as_listed` row states. Service tests `tests/as_listed_accept.rs`, including
the struct test. Parity scenario.

#### Step 7 — The counterparty's Hub

D-C3c-18, the counterparty's Hindi messages. Hub unit tests.

#### Step 8 — Two-node, Playwright, sizes, docs and close-out of C3c

`roym_as_listed_e2e.rs` in full (section 7.4), the `roym-hub.spec.ts` cases
(section 7.6), story 50 (Q7). `mise run build:roym`, parity, sizes, living
docs, backlog, Deviations of all three change docs, task.md's C3c result
line, status.md. `status: done` is not set (C8 does it).

## 7. Tests

Test names say the outcome they check. Before a change doc names a test,
check it exists: `git grep -n 'fn <name>'` (C3a R3, R4).

### 7.1 Unit tests (`roym_core`)

`crates/roym_core/src/as_listed/tests.rs` (new):

- `each_mapping_row_builds_its_field` — one test per row of the mapping
  table, or one table test with a case per row.
- `each_refusal_has_its_code` — one case per `NotComplete` value, and the
  order: a listing that breaks two rules gets the first one's code.
- `ready_agrees_with_the_full_build_without_a_slot` — for every listing of
  a generated set (each refusal, each extension, with and without
  translations), `as_listed_ready(l).is_ok()` equals
  `terms_from_listing(l, no slot).is_ok()` for a listing with no reserve
  hook.
- `the_largest_listing_builds_terms_that_pass_the_size_check` (`:256`).
- `a_built_quote_never_fails_validate` — every listing that
  `ListingPayload::validate` accepts and `as_listed_ready` accepts gives
  terms that pass `DealTerms::validate`; `Invalid` never appears.
- `the_amount_text_uses_a_dot_and_the_currency_exponent` — INR, JPY
  (exponent 0), a three-digit currency, a value below one unit.
- `expiry_is_now_plus_quote_valid_secs`.
- `a_listing_with_empty_payee_refuses_price_not_fixed` (P9).

`listing/as_listed` tests: one per new `listing.set` refusal
(`instant-not-allowed` with each reason, `instant-cap-range`,
`deal-terms-languages`, `quote-valid-range`), and a listing with all
defaults keeps today's signed bytes
(`a_listing_without_the_new_fields_signs_the_same_bytes`).

Enquiry: `as-listed-needs-listing`, `variant-not-supported`,
`slot-differs`, `description-required`, and an as-listed enquiry with no
description verifies.

Progress: `formed_as_is_kept_by_every_apply`;
`a_presentation_with_only_the_latest_snapshot_shows_formed_as` (AG-51's
unit row; through `deal.verify-presentation`'s view).

Decline: a code outside the allowed list is refused; a decline with no
code keeps today's bytes.

### 7.2 Wake tests (`app_orchestration`, `substrate`)

- `app_orchestration`: one test per `WakeSpec::validate` rule;
  `a_service_without_a_wake_keeps_its_deploy_hash`.
- `substrate` (`runtime/wake/tests.rs`, with a test target that counts its
  calls and can be made slow):
  - `a_burst_of_events_gives_one_call` (ten events in 100 ms);
  - `an_event_of_another_kind_gives_no_call`;
  - `the_interval_calls_with_no_event`;
  - `a_slow_call_is_never_overlapped`;
  - `a_reload_of_the_event_window_gives_a_call`;
  - `an_undeployed_service_is_not_called_again`;
  - `the_linked_build_registers_every_wake_of_the_manifest` (D-C3c-3);
  - `a_panic_in_invoke_resets_running_guard` (P6);
  - `a_missing_local_endpoint_skips_dispatch_with_no_outbound_call` (P1);
  - `a_binding_push_keeps_the_wake` (P10);
  - `again_in_the_reply_gives_a_call_on_the_next_tick` and
    `a_reply_without_again_waits_for_the_interval` (P18);
  - `warnings_in_the_reply_are_logged` (P18).

### 7.3 Service tests (`crates/roym_transaction/src/app/tests/`)

`drive.rs` (C3c1):

- `drive_syncs_each_conversation_named_by_a_message_event`;
- `drive_syncs_at_most_eight_conversations_and_keeps_the_rest` (AG-50's
  service row);
- `drive_skips_a_group_conversation_without_an_error`;
- `drive_after_reload_walks_every_conversation_in_pages`;
- `drive_ignores_events_of_transaction_itself`;
- `drive_gives_timed_work_a_sync_with_no_new_message` (a quote that
  expires, a `deciding` row);
- `drive_keeps_conversation_pending_when_more_messages_in_window` (P13);
- `drive_compare_and_delete_preserves_concurrent_event_mark` (P13);
- `drive_runs_recovery_once_per_call` (P14, P21);
- `sync_still_runs_recovery_once` (P21: `transaction.sync` keeps today's
  behaviour);
- `a_failing_conversation_does_not_starve_the_others` (P18);
- `a_failing_row_is_dropped_after_five_failures_with_a_warning` (P18);
- `a_failing_row_does_not_set_again` (P18: the wake does not run every
  second);
- `more_is_false_when_last_seq_does_not_move` (P18: a full window that
  starts with a deferred message).

The test host's `conversation.changes` and `conversation.list` stubs and an
events stub: check `roym_core::testing` (`TestHost`,
[testing.rs:45](../../../../crates/roym_core/src/testing.rs#L45)) for
`read_events` support first (the stubs file has one,
`roym_core/src/testing/stubs.rs`); extend it, do not write a second host.

`as_listed_owner.rs` (C3c2, AG-45, AG-49, AG-51):

- `an_instant_offering_is_answered_by_the_driver_with_no_owner_action`;
- `a_confirm_first_offering_waits_and_from_listing_answers_it`;
- `a_withdrawn_offering_is_declined_with_offering_withdrawn`;
- `an_expired_consent_is_declined_with_consent_expired`;
- `a_changed_offering_is_answered_from_the_current_version`;
- `the_cap_makes_an_instant_offering_wait` and
  `an_agreement_with_a_filed_half_no_longer_counts` and
  `an_agreement_older_than_seven_days_no_longer_counts`;
- `a_second_open_instant_agreement_with_one_counterparty_waits`;
- `many_taps_before_any_acceptance_make_only_cap_quotes` (P12);
- `one_counterparty_tapping_twice_gets_one_instant_quote` (P12);
- `a_declined_instant_quote_frees_its_place` (P19: the counterparty
  declines, taps again, and gets an instant quote);
- `a_failing_certificate_makes_the_offering_wait` (one case each for
  `NotEnrolled`, `Expired`, `Stale`);
- `a_full_slot_ends_not_formed_with_slot_taken` (two counterparties, one
  seat; no fallback to confirm first);
- `a_removed_slot_is_declined_with_the_hooks_code` (Q8, P16, P20: the
  prepare hook runs before the `answer:` fence);
- `an_unreachable_slot_hook_makes_the_enquiry_wait` (P20);
- `quote_set_runs_the_prepare_hook_once` (P20: the existing `quote.set`
  path is unchanged);
- `two_answers_of_one_tap_make_one_quote` (two calls in a row, and a
  crash after the fence with the injected-error host of C1b);
- `formed_as_is_the_same_in_the_first_and_a_later_snapshot` (one agreement
  of each of the three kinds);
- `a_snapshot_whose_formed_as_changed_is_refused`;
- `cancel_of_unconfirmed_instant_agreements_frees_their_slots`;
- `the_import_checks_answer_rows`.

`as_listed_accept.rs` (C3c2, AG-44, AG-49):

- `a_quote_with_the_listed_terms_is_accepted_with_no_person`;
- `a_quote_that_differs_in_any_field_is_shown` (the struct test: one case
  per field, generated from the JSON keys);
- `a_missing_disclosure_shows_the_quote`;
- `a_quote_issued_after_the_consent_time_is_shown`;
- `a_withdrawn_enquiry_is_never_accepted`;
- `a_quote_from_a_changed_offering_is_shown`;
- `the_acceptance_carries_the_stored_viewed_fields`;
- `one_tap_accepts_at_most_one_quote` (two quote ids answer one tap);
- `a_signing_error_keeps_the_row_waiting_and_the_next_drive_accepts`;
- `an_expiry_more_than_seven_days_after_issue_is_shown`;
- `the_import_checks_autoaccept_rows_and_as_listed_rows`.

### 7.4 Two-node (`crates/substrate/tests`)

- `roym_driver_e2e.rs` (C3c1): X has a listing; Y sends an enquiry; X sends
  a quote; Y accepts. Both reach `agreed` within 30 s, and the test calls
  `transaction.sync` on neither node (it polls `agreement.get` only, which
  does not sync). A second case stops X before Y accepts, starts it again,
  and checks it countersigns with no call (the `reload` path).
- Native linked-build dependency test (C3c2, P11): in `crates/substrate`, call
  `listing.get` from `transaction` to verify `wire_roym_topology` binding.
- `roym_as_listed_e2e.rs`:
  - The owner's side (step 5): an instant one-shot offering; Y sends an as-listed enquiry
    (`rpc` with the envelope from `directory.run-envelope`); X's node
    answers with no call on X; Y accepts by hand; the snapshot says
    `as-listed-instant`. A confirm-first offering waits until X calls
    `quote.from-listing`; `as-listed-confirmed`.
  - The counterparty's side (step 8): the same instant offering reaches
    `agreed` from one `enquiry.set` call on Y, with no other call on either node (AG-44);
    a quote that differs in one term waits for Y (X answers by hand with a
    different price); Y's acceptance half says `viewed_via_mt: true` when
    the tap said so, and `false` for the original (AG-44's language row).
  - A slot offering (Q4): the test reads the slot from X's
    `availability.list`, as story 60 does, and the deal reaches `agreed`.

"Both Hubs closed" is every two-node Rust test: they run no Hub.

### 7.5 Parity (both builds agree)

`dual_build_parity/as_listed.rs` (new): scenario 252 (`transaction.drive`
with two changed conversations), 253 (an as-listed enquiry answered by the
driver path), 254 (`quote.from-listing` for confirm first), 255 (the
automatic acceptance), 256 (`enquiry.set` refuses `as-listed-not-complete`).
`mise run build:roym` first.

### 7.6 Hub unit tests (`npm test`) and Playwright

- Hub unit tests: the listing form hides `instant` while signing is not
  installed (AG-45's E2E row is met by the Hub test plus one Playwright
  case); the owner's confirm button; the limit line; the cancel action;
  the decline sentences per code; the tap screen shows every term and the
  consent time (AG-44's E2E row); the MT warning comes before the tap; the
  waiting, ended and refused lines; a slot offering shows no "Agree as
  listed" (Q4). Use `stubRpc`
  ([stub-rpc.ts](../../../../crates/roym_web/ui/src/testing/stub-rpc.ts)).
- Playwright `roym-hub.spec.ts`: the tap screen's terms; the waiting line
  while the owner's node is stopped; a refused first contact shows as
  refused (AG-49's E2E row); the owner's "limit reached" line; the cancel
  frees the slots.
- Story 50 (Q7, C3c2).

### 7.7 Not tested

- The wake's timing under load (many apps on one node). Recorded as a
  backlog row with the substrate-side concurrency row of ADR-0023.
- The cap race of Q6 (a backlog row).

### 7.8 Size and time risks

- **Files near the cap.** `sync.rs` is at 759 lines; proactively extract
  `file_enquiry_card` and `file_quote_card` into sibling file `sync/cards.rs`
  (~140 lines saved), keeping `sync.rs` safely below the 800-line cap across
  C3c1 and C3c2 (P4). `backup.rs` (785) gets two lines; `decide.rs`
  (724) gets the `formed_as` read in two places (+6); `app.rs` (710) gets two
  fields (+6); `backup/sections.rs` (703) gets two match arms (+6);
  `quote_ops.rs` (695) loses the body of `issue_quote` (−45). New code goes
  in `drive.rs`, `sync/cards.rs`, `as_listed_ops.rs` and its folder,
  `quote_ops/issue.rs`, `backup/as_listed.rs`, `roym_core/src/as_listed.rs`,
  `roym_core/src/listing/as_listed.rs`, `substrate/src/runtime/wake.rs`.
  Check `wc -l` before and after each step.
- **The wake's time per call** (stop rule in D-C3c-1).
- **A test that was quietly slow.** Two-node tests that today wait for a
  manual sync may now finish sooner; none should get slower. Compare the
  run time of the seven changed tests before and after C3c1.

### 7.9 Component size

Section 1 of task.md. Before and after, from one `target/` folder:

| Component | Part | Limit |
| --- | --- | --- |
| `transaction` | C3c1, C3c2 | C3c1 +40 KB (no new record code); C3c2 +15% (new record code) |
| `catalog` | C3c2 | +15% (the listing record changes) |
| `directory` | C3c2 | +40 KB (stop rule, D-C3c-11) |
| `web`, `profile`, `conversation`, `scheduling` | all | +40 KB; expected unchanged |

### 7.10 Hindi text

New messages, each in `en.ftl` and `hi.ftl`; the owner reviews the Hindi
before each merge:

- C3c2, the owner's side: the listing form section and its fields (about 12 messages:
  `listing-as-listed-heading`, `listing-acceptance-instant`,
  `listing-acceptance-confirm`, `listing-not-limited`, `listing-instant-cap`,
  `listing-quote-valid-hours`, `listing-payment-timing-*`,
  `listing-instant-needs-signing`, the three text labels); the owner's lines
  (`as-listed-waiting-confirm`, `as-listed-cap-reached`,
  `as-listed-counterparty-holds-one`, `as-listed-signing-not-ready`,
  `as-listed-slot-check-failed`,
  `as-listed-confirm-button`, `as-listed-cancel-open`); the decline
  sentences (`decline-code-consent-expired`,
  `decline-code-offering-withdrawn`, `decline-code-no-such-slot`,
  `decline-code-slot-not-in-listing`).
- C3c2, the counterparty's side: `directory-agree-as-listed-button`, the
  tap screen's labels
  (`as-listed-will-be-paid-to`, `as-listed-quote-valid`,
  `as-listed-tap-holds`, …), the counterparty's lines
  (`as-listed-sent-waiting`, `as-listed-ended`, `as-listed-refused-contact`,
  `as-listed-terms-differ`).

The exact ids are fixed when the code is written; the catalog test
(`i18n/catalog.test.ts`) checks both files.

### 7.11 How C3c avoids the kinds of findings in earlier reviews

- **Facts in the plan that are wrong** (C1 P1, C2 P3, P6): every line here
  was checked on `8fafab15`; three doc citations were found stale and are
  named (section 2.2). The method-contract count is named in each step.
- **A step that does not finish after a crash** (C1 R2, R21; C3b P1): the
  answer and the automatic acceptance each take a fence before any
  signature, and a repeat reuses the stored record (D-C3c-12, D-C3c-17).
- **Missed callers** (C1 P7, R10; C3b P2, R1): D-C3c-7 gives the `git grep`
  for `description`; D-C3c-4 lists the seven count sites; D-C3c-3 tests
  that the linked build has every wake.
- **Who signed is not checked** (C2 R14, R26): the owner's node answers
  only for its own listing; the counterparty's node builds from the
  envelope it verified itself, and compares with disclosures it verified.
- **Errors classed by text** (C1 R3): `drive` matches `SyncError::NotDirect`;
  `auto_accept` branches on the error's code, never on its message.
- **Silent failures** (C2 R21, R22): a `Waiting` reason and a `shown` reason
  are stored or recomputed and shown; the wake logs every failed call.
- **Language of signed text** (C1 P6, R33): the quote's texts are the
  listing's originals with their own `lang`; the decline's sentence comes
  from the code in the reader's language.
- **Hub state that does not match the service** (C1 R38, C2 R2): the
  counterparty's Hub never shows "agreed" before the writer's snapshot;
  the waiting reason comes from `deal.as-listed-status`, not from the Hub's
  own guess.
- **Tests that prove less than their names** (C3a R3, R4): each AG row
  names its test file and case in section 3 and 7.
- **Living docs left false** (C1 R6, C2 R40): section 9 names the lines.
- **Ratchets** (C1 R16, R17): the shapes are regenerated in the step that
  adds each method; no suppression is added.

## 8. Deferred backlog

[deferred-backlog.md](../../deferred-backlog.md):

| Row | C3c action |
| --- | --- |
| "A card is filed only when a client calls `transaction.sync`" (`:654`) | C3c1 resolves it: move it to "Recently resolved" with the wake. |
| "Services can only pull events" (`:547`) | Unchanged. The wake triggers a pull; there is still no push export. Add a note that the wake exists. |
| "Agree as listed refuses some offerings" (`:576`) | Unchanged, and C3c builds the refusals it names. |
| "A schedule deployed by `roymctl app deploy` never runs" (`:415`) | Unchanged; the wake is not a schedule. A note points at ADR-0031. |
| New: "A person cannot pick another node's slot before agreeing as listed" (Q4) | C3c2 adds it. The Hub shows "Ask" on a slot offering. Target: M08D (the offering page, D5). |
| New: "The instant cap can be passed by one" (Q6) | C3c2 adds it. Target: TBD, when a deployment shows it. |
| New: "Many woken services on one node are not pooled" | C3c1 adds it, beside ADR-0023's row "Substrate-side concurrency for scheduled runs is unbounded across app instances". Target: TBD. |
| New: "The reader does not use `formed_as` yet" | C3c2 adds it. Target: M08G. |

No new `TODO` or `FIXME` marker is planned. If one is added, it gets a row
in "Open in-code markers".

## 9. Living docs to update at close-out

Each PR fixes the statements it makes false. C8 does the full pass.

- `docs/decisions/0031-node-local-wake.md` (new, C3c1).
- `docs/system-architecture.md`: the substrate's background workers (the
  wake worker), the app manifest's fields (`wake`), and the Roym section's
  sentence that deals move when a client syncs (C3c1). Agree as listed in
  the Roym deal flow (C3c2).
- `docs/developer-guide.md`: the manifest key `wake` with its rules; the
  `transaction` method table (`transaction.drive`, `quote.from-listing`,
  `deal.as-listed-status`, `enquiry.set`'s `as_listed`); the listing's new
  fields; the new ledger kinds and the `as_listed` collection in the backup
  section; the two-node test note that tests no longer need to call
  `transaction.sync` to move a deal (C3c1, C3c2).
- `docs/system-requirements-spec.md`: the agree-as-listed requirement and
  the node-side driver.
- `docs/roym-integrated-experience-spec.md`: steps C12 and C14, as the
  agreements doc's Differences say (`:1074`, `:1075`); the listing's new
  fields; the directory hit's "Agree as listed".
- `docs/TERMINOLOGY.md`: "agree as listed", "instant", "confirm first",
  "wake" (check which exist first).
- `docs/demo/demo.md`: story 50's new steps (C3c2).
- `docs/planning/traceability-matrix.md`: the rows of section 3.
- `AGENTS.md` "Architecture": one sentence that the substrate wakes a
  service that declares a `wake` (C3c1).
- Change docs (not living docs, but owned here):
  - roym-agreements `## Deviations`: the driver is the wake (Q2, Q3);
    `as_listed.slot` with its window, not `slot_id` (D-C3c-5);
    `terms_from_listing` takes the verified payload (point 3); the
    `Invalid` variant (D-C3c-6); the slot codes as decline codes (Q8); the
    Hub offers the tap only for offerings with no reserve hook (Q4); the
    definition of "open" for instant limits counts in-flight quotes (P12);
    the cap may be passed by at most one under two concurrent syncs (Q6);
    `formed_as` also in a `not-formed` snapshot (D-C3c-10);
    `deal.as-listed-status` (a method the table does not list); the stale line
    citations `messages.ts:424` and `decide.rs:695` (point 1, point 2).
  - roym-value-exchange `## Deviations`: VX-54 met, with the slot gap of
    Q4.
  - roym-extensions `## Deviations`: EX-18's agree-as-listed half built
    (and the "not built yet" note in its test column removed).
  - [task.md](task.md): the C3c result lines, the split, and section 1's
    size tables.

## 10. Exit criteria

C3c1:

- [x] ADR-0031 accepted by the owner before step 2 (commit `561e42b9`, `docs/decisions/0031-node-local-wake.md`).
- [x] Every C3c1 row of section 3 names a test that exists and passes (AG-50: `roym_driver_e2e`, `tests/drive.rs`, `runtime::wake`).
- [x] `roym_driver_e2e` passes with no `transaction.sync` call in it (2 tests pass: `driver_e2e_advances_quotes_without_client_sync` and `stopped_quote_author_restarts_and_countersigns`).
- [x] No test turns the wake off; the seven count sites assert state (`roym_flow.rs`, `roym_booking_e2e.rs`, `roym_transaction_e2e.rs`).
- [x] One `drive` call's time in the WASM build recorded, under 2 s (69.8 ms for 8 conversations on a node with 200 agreements).
- [x] Parity and method-contract pass after `mise run build:roym` (`dual_build_parity` 272 passed; `method_contract` 38 passed).
- [x] Sizes recorded and within limits (`transaction` wasm 2,390,790, +52,370 bytes, +2.24% vs limit +15%; `scheduling` 715,723, +864 bytes; others unchanged; all within limits).
- [x] Deviations, backlog rows and living docs done (ADR-0031, `AGENTS.md`, `developer-guide.md`, `system-architecture.md`, `change.md`, `deferred-backlog.md`).
- [x] `mise run verify` passes with no `--skip` (CI passed on `27a9fa18` across all jobs, including Playwright E2E and 6 test shards; accepted by owner).

C3c2:

- [ ] Every C3c2 row of section 3 names a test that exists and passes.
- [ ] `directory` measured after D-C3c-11, within +40 KB.
- [ ] A listing with no new field signs the same bytes as before.
- [ ] One tap gives at most one quote (`two_answers_of_one_tap_make_one_quote`).
- [ ] The struct test fails when a field is added to `DealTerms` or
  `Commitment` with no row (checked once by adding a field locally).
- [ ] `roym_as_listed_e2e`'s instant case reaches `agreed` from one call.
- [ ] Story 50 passes with the new steps (`mise run demo`).
- [ ] Owner reviewed the Hindi messages of C3c2.
- [ ] Sizes, Deviations, backlog, living docs; `mise run verify` with no
  `--skip`.

## 11. What C3c does not do

- A slot picker for another node's slots (Q4).
- `agreement.close`, the totals and the difference, the combined summary
  of versions, the quote form defaults and "Suggest changes" (C6).
- Variants in `as_listed` (M08D), and the mode in an intent record (M08D).
- Offerings priced per hour, per unit or by quote, priced before tax, or
  done at the counterparty's address (backlog row `:576`).
- A reader that uses `formed_as` (M08G).
- A push of events into a guest (`on-event`); the wake only triggers a
  pull.
- A supervisor change, or any change to ADR-0023's schedules.
- Any wire-reachable method.
- A change to `conversation`, so C7 may run beside C3c.

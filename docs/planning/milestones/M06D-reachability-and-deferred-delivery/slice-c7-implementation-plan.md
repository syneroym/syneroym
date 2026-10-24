# M06D slice C7 — Open mailbox list and `auto`: implementation plan

*Reader: the developer (or AI) who implements slice C7. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: ready, 2026-10-24. The owner accepted the recommendations for
Q1 to Q7 (section 2.1) on 2026-10-24. A plan review on the same day changed
Q2, Q3 and Q5, and added C7-15 to C7-19 (section 2.3 lists each review point
and what was done). The owner accepted the changed Q2, Q3 and Q5 and every
review answer on 2026-10-24. You can start Step 1 once C6 has merged (see
Base).**

**Size and PRs.** `task.md` sizes C7 as S. With the receipt cap, the
supervisor wiring, the status field and five end-to-end tests it is L. Ship
it as two PRs, in this order (C7-19):

- **PR 1 — receipt cap** (Step 6 and its tests, its docs and backlog rows).
  Only `crates/mailbox` and `crates/core/src/config/mailbox.rs`. It must
  merge first: `task.md` asks for the cap before strangers can find open
  mailboxes.
- **PR 2 — list and `auto`** (every other step).

**Base.** C7 builds on C6 (branch `feat/m06d-c6-receiver-path`, not merged
into `main` when this plan was written; the branch still had uncommitted C6
fixes in `crates/router/src/mailbox_pull*`,
`crates/substrate/src/runtime/services.rs` and the mailbox change doc). Start
the C7 branches from `main` after the C6 PR has merged:
`feat/m06d-c7-receipt-cap` for PR 1, then `feat/m06d-c7-open-mailbox-list`
for PR 2 after PR 1 merges. File paths and line numbers below were checked
against `feat/m06d-c6-receiver-path` at `a8bc910d`. C7 touches none of the
files that had uncommitted C6 changes, except the mailbox change doc.

## 0. Read first, and the rules

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish (background job, about 20 minutes),
  the import clean-up pass, the deferred backlog, the living docs close-out,
  no planning references in code (no "M06D", "C7", "K-3", "D-C3-9", "C7-4"
  in code, comments or test names; ADR ids are fine), simple English in
  comments, 800-line files, 100-line functions, short match arms, the module
  layout rule, and no hard-coded ports in substrate tests.
  **This plan names its decisions `C7-1` to `C7-19`. Those ids are for this
  plan only. When a step says "for the reason in C7-n", write the reason
  itself in the comment.**
- `task.md` in this folder: §1 (X2, X3), §3 A2 (the "second list is one
  route" rule), C3, C4 (forward note), C7, C8.
- `docs/planning/changes/service-mailbox/change.md`: K-3, K-5, M-4, M-9,
  Design "Registry list", "Store", "Limits", Tasks, Close-out,
  `## Deviations` (`D-C3-9`, `D-C3-14`, `D-C4-6`, `D-C4-7`, `D-C4-20`,
  `D-C5-2`).
- `docs/planning/changes/registry-listed-coordinators/change.md`: AC-3,
  AC-4, AC-13, AC-15, Design "Registry side" and "URL form and ranking",
  Deviations `D-A2-1` to `D-A2-13`, `D-A4-9`.
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) Decisions 1
  and 8, and the consequence "A mailbox is chosen at deploy time".
- The deferred-backlog rows that name this slice or its subject: "Service
  mailbox: `auto` mailbox choice", "Receipt table size is not capped by
  `max_total_bytes`" (target `C7`), "The mailbox republishes only to its own
  registry", "The mailbox cap does not evict across queues".
- The code this slice builds on (read first):
  - `crates/core/src/curated_list.rs` (`ListEntry`, `CuratedList`,
    `check_entries`, `fetch`, `list_request_timeout`), `curated_list/rank.rs`
    (`rank`, `rank_key`), `curated_list/coordinator.rs` (the one instance of
    `ListEntry` today).
  - `crates/community_registry/src/curated_list.rs` (`router`, `list_route`,
    `serve_list`) and its `tests.rs` (`spawn_list_registry`);
    `crates/community_registry/src/registry.rs:226-230`
    (`propagate_registration`: a registration goes up to the parent) and
    `:307-315` (`lookup_endpoint`: a lookup never asks the parent).
  - `crates/core/src/config/roles.rs:39-63` (`ServiceRegistryRole`).
  - `crates/app_orchestration/src/models/service.rs:303-355` (`MailboxChoice`),
    `models/plan.rs:73-76` (`PlannedService.mailbox`),
    `compiler.rs:126-166` (`validate_plan_member_settings`),
    `reconcile.rs:42-130` (`compute_diff`, `diff_plans`).
  - `crates/sdk/src/deploy/certify.rs:198-220` (`member_mailbox`).
  - `apps/roymctl/src/commands/svc/deploy.rs` (`check_mailbox_args`,
    `build_publication`, `handle_deploy`), `commands/svc.rs:113-157` (the
    `--registry-url` and `--mailbox` help), `commands/registry.rs:55-90`
    (`current_record`, `refuse_if_mailbox`).
  - `apps/roymctl/src/commands/app/deploy.rs:318-430` (preflight,
    `registry_facts`) and `:612-732` (`handle_deploy`), `app/settings.rs`,
    `app/reconcile.rs:17-70`, `commands/app.rs:150-172` (the `app deploy`
    flags), `commands/supervisor.rs` (`submit` and `status` printing).
  - `crates/app_supervisor/src/service/verbs/submit.rs:180-342`
    (`handle_submit`, `validate_submit_preflight`),
    `crates/app_supervisor/src/service.rs:313-560` (`SupervisorService`,
    `new`), `crates/app_supervisor/src/service/status.rs:250-280`
    (`managed_services_from_report`), `crates/app_supervisor/src/lib.rs`
    (re-exports), `crates/substrate/src/runtime/supervisor.rs:26-108`
    (`init_supervisor`), `crates/wit_interfaces/wit/supervisor/supervisor.wit:42-55`
    (`managed-service`).
  - `crates/mailbox/src/store.rs` (schema, `remove_message`, `put_receipt`),
    `store/deposit.rs` (`deposit`, `make_room`, `check_existing_receipt`),
    `store/sweep.rs` (`sweep`, `totals`), `store/claim.rs`
    (`close_over_claimed`, `claim`, `ack`), `store/queue.rs` (`unregister`),
    `limits.rs`, `crates/core/src/config/mailbox.rs` (`MailboxRole`),
    `crates/mailbox/src/worker.rs:60-95` (metrics).
  - `crates/substrate/tests/mailbox_sender_e2e.rs` (`World`, `Restart`,
    `Options`), `tests/common/mailbox_receiver.rs`, `tests/common/node.rs`
    (`restart_with` keeps the ports and the identity),
    `tests/durable_outbox_e2e.rs:180-215` (`submission`, `submit_with_retry`),
    `tests/curated_list_e2e.rs`.

### Terms used in this plan

- **Open mailbox list:** the second curated list of a community registry,
  `[[roles.community_registry.mailboxes]]`, served as `GET /mailboxes`.
- **Pin:** replace `MailboxChoice::Auto` in a plan with
  `MailboxChoice::Service(<node DID>)` before the plan is stored. A stored
  plan never holds `Auto`.
- **Chooser:** the first input of `rank_key`. It decides which entry a
  deployer gets, so every deployer must pass the same form.
- **Settled receipt:** a receipt row whose message body is gone (delivered,
  rejected, expired or evicted). A receipt of a held message is not settled.
- **List registry:** the registry `auto` reads the list from. **Host
  registry:** the registry the hosting substrate publishes the service record
  to (`substrate.registry_url` of that node).

## 1. What C7 builds, in one paragraph

A registry operator lists open mailboxes in the registry config. The registry
serves the list at `GET /mailboxes`, with the same checks and cache time as
`GET /coordinators`, but it never forwards this list to its parent (C7-6). A
deployer that writes `mailbox = "auto"` (manifest), `--mailbox auto`
(`svc deploy`) or `--mailbox <service>=auto` (`app deploy`) gets one entry of
that list, chosen by the rendezvous ranking helper, and the choice is pinned:
roymctl writes it into the plan before the journal records it, and the
supervisor writes it into the plan at `submit` before it stores the desired
state. A later deploy keeps the earlier mailbox (from the stored plan, or
from the service's current record), because a different mailbox means a new
record, and messages that wait at the old mailbox are lost. roymctl refuses
`auto` when it can see that the host registry is not the list registry.
`supervisor status` shows each member's mailbox. Before strangers can find
open mailboxes, the mailbox caps the receipts of messages it no longer holds.
C7 also adds the end-to-end tests that the mailbox change doc still lists as
missing, and updates the living docs for the mailbox (all but the
dedicated-mailbox part, which C8 does).

## 2. Points for the owner

### 2.1 Questions (accepted 2026-10-24; Q2, Q3 and Q5 changed by the review and accepted again the same day)

**Q1. The chooser for a service with several members.** The change doc says
the rank key is "SHA-256 over the service id and the mailbox service id", and
the `rank_key` doc comment says the chooser is "the picking service's
`service_id` as it appears in its endpoint record". But a scaled service has
one `service_id` (member master DID) for each member, and K-3 says all
members of a service share the mailbox host. Also, roymctl journals the plan
**before** it substitutes the member masters (`substitute_and_certify_members`
returns a copy), so at pin time the plan holds only the compiler's made-up
ids.
**Accepted:** for a plan service, the chooser is the logical service ref in
its text form (`LogicalServiceRef`'s `Display`:
`<app_instance_id>/<service_name>`). For `svc deploy`, it is `--svc-id`
(which is the record's `service_id`). One chooser for all members, known
before substitution, the same in roymctl and the supervisor. The cost: two
owners who use the same instance id and service name get the same ranking.
That only changes how load spreads over the list, and the pin makes it
matter only at the first deploy.

**Q2. Which registry `auto` reads, and the same-registry rule.** Nothing
says which registry. Also, a mailbox keeps a sleeping receiver's record alive
only in its own node's registry, and a registry lookup never asks the parent
(`lookup_endpoint`). So `auto` works only when the list registry, the
mailbox node's registry and the host registry are one registry.
**Accepted, as changed by the review:**
- roymctl reads `--registry-url`; the supervisor reads its own node's
  `substrate.registry_url`. `auto` without a pin and without a list registry
  is an error before anything is signed or placed.
- The list does not forward to a parent (C7-6), so an entry always comes
  from the list registry's own operator, who is told to list only mailboxes
  of that registry (Step 2 doc comment).
- roymctl compares the list registry with the host registry when it can read
  the host's node facts, and refuses a known difference (C7-15). The
  supervisor cannot read its hosts' facts before it stores the plan; that
  case is a backlog row.

**Q3. What "resolve once" means across later deploys.**
**Accepted, as changed by the review:** a deploy keeps the mailbox an
earlier deploy chose, and reads the list only for a service with no earlier
mailbox. Sources, in order:
1. the stored plan of this instance, same `logical_ref`: the latest journal
   record (`app deploy`), or the stored desired state (supervisor);
2. the current record of a member at the list registry, if it verifies and
   names a mailbox: the supervisor (its plan holds the real member masters at
   pin time, so this covers an instance first deployed with `app deploy` and
   then submitted to the supervisor) and `svc deploy` (`--svc-id`). `app
   deploy` skips this source: its plan holds made-up ids at pin time. The
   handover from the supervisor back to `app deploy` is a backlog row. The
   supervisor reads the record at its own node's registry, so the handover
   case works only when the earlier hosts published to that registry; this
   is part of the backlog row on the supervisor's same-registry check.
3. the list.
The earlier mailbox is kept even when it has left the list. An explicit node
DID in the manifest or on the command line always wins. Moving a service to
another mailbox stays an explicit act.

**Q4. Check that the chosen mailbox answers?**
**Accepted:** no probe in C7. Take the first ranked entry. Backlog row.

**Q5. The receipt cap (the C4 forward note).**
**Accepted, as changed by the review:** cap only **settled** receipts:
`[roles.mailbox] max_settled_receipts` (default 100,000). When a message
settles and the count goes above the cap, the mailbox drops the oldest
settled receipts (soonest `expires_at` first). A deposit is never refused by
this cap. Receipts of held messages are not counted: each one belongs to a
held message, and the message limits (`max_total_bytes`, `max_queues` ×
`max_queue_messages`) already bound them. So the cap cannot block deposits
and cannot limit a dedicated mailbox's raised quotas in C8. Disk: a receipt
JSON is at most 4 KiB (`MAX_RECEIPT_JSON_BYTES`), so the settled part is at
most about 450 MB, a budget separate from `max_total_bytes`; a typical row is
near 1 KB. Effects of a dropped receipt, all safe:
- the sender's next `status` reads `unknown`, and the sender goes back to
  direct attempts (S-5);
- deposit idempotency (`D-C4-20`) lasts only while the receipt exists: an
  identical retry with the same `deposit_id` after the drop is stored as a
  new message. A sender seals each deposit once (`D-C2-9`), so this needs a
  repeated call, and the receiver fence runs a second delivery once (R-7);
- M-9 ("kept for 14 days") becomes "kept for 14 days, or less when the node
  holds more than `max_settled_receipts` settled receipts".

**Q6. Which end-to-end tests C7 adds.** Group push
(`mailbox_conversation_e2e.rs`, `a_group_push_from_the_mailbox_reaches_the_member`)
and expiry then direct (`mailbox_sender_e2e.rs`,
`an_expired_deposit_goes_back_to_direct_and_lands_once_when_the_receiver_returns`)
already exist. A real 2-hour registry expiry cannot run in a test.
**Accepted:** add four tests (Step 11c): offline sender, a receiver with no
address (stands for "asleep longer than 2 hours"), relocation, and a full
queue. Add one test for `auto` through the supervisor (Step 11d). Cite the
two existing tests in the change doc. Say that the 2-hour case is
simulated.

**Q7. Close-out scope.**
**Accepted:** C7 updates the living docs and the traceability matrix for
everything but D-1 to D-3. The change doc keeps `status: approved` (C8
closes it). Every `roym-integrated-experience-spec.md` statement that
depends on its decision D4 is rewritten.

### 2.2 Stale or unclear points in `task.md` (no decision needed, but read them)

1. **"Remove the three refusals … `certify_placed_members`".** The third one
   (`member_mailbox`, `crates/sdk/src/deploy/certify.rs:209`) is not a
   "not supported" refusal. It says "has an unresolved `auto` mailbox". After
   C7 it is the guard that a plan was pinned before certification. Keep it;
   change only its text (Step 4). The other two
   (`validate_plan_member_settings`, `check_mailbox_args`) go.
2. **The `public` check has a hole once the `auto` arm goes.**
   `validate_plan_member_settings` refuses a non-public service only in the
   arm `Some(MailboxChoice::Service(_))`. If the `Auto` arm is just deleted,
   `auto` on a private service passes the plan check and fails late, in
   certify. Step 3 makes the check cover any mailbox.
3. **`build_publication` drops `auto` silently.** `svc/deploy.rs:213` maps
   `Some(MailboxChoice::Auto) | None => (None, None)`. After C7 an unpinned
   `auto` must never reach it. Step 7 changes the parameter to
   `Option<&ServiceId>`, so the type says so.
4. **A list with no filter cannot use `type Filter = ()`.** `serve_list`
   reads `Query<E::Filter>`, and the query always has `hops`.
   `serde_urlencoded` 0.7.1 (`de.rs:119`) reads a unit type only from an
   empty query, so `GET /mailboxes?hops=0` would answer `400`. Use an empty
   braced struct (Step 2).
5. **Help texts.** `svc deploy --mailbox` (`svc.rs:155`), `app deploy
   --mailbox` (`app.rs:163`) and the doc of `parse_service_mailbox`
   (`mailbox_arg.rs:7`) still say `auto` is refused. Steps 7 and 8 fix them.
6. **The C4 forward note about the instance key.** "Before the registry lists
   open mailboxes for strangers, the router must check that a caller holds
   the instance key of its certificate. C4b does this." Done: C4b signs the
   preamble, and the backlog has no open proof-of-possession row. `D-C4-4` in
   the change doc still says it is deferred; Step 1 adds one sentence.
7. **The task.md status line** (line 5) still says "C6 comes next" (Step 13).
8. **"The same forwarding and `502` rule" (change doc, Design "Registry
   list") is wrong for this list.** Forwarding would hand out a mailbox that
   republishes into the parent registry, where senders on this registry do
   not look (`lookup_endpoint` never asks the parent), so K-5 would fail
   after 2 hours. C7-6 turns forwarding off for this list. The backlog row
   "The mailbox republishes only to its own registry" changes to match.
9. **`app deploy` deploys only the target plan** (`plans.last()`). Plans of
   spawned apps are compiled and checked but not applied here, so C7 pins
   only the target plan. Whoever later submits a spawned plan pins it.
10. **"Use `limits.max_ttl_secs`" and similar notes for C6 and C8** are not
    C7's. C7 does not touch the puller.
11. **The over-quota case.** The integration list does not say which quota.
    C7 tests the per-queue message limit end to end: the second deposit gets
    `queue_full` and stays on direct attempts. The node limit
    (`mailbox_full`) and the receipt cap are tested in the mailbox crate.
12. **C7 is sized S.** It is L; see "Size and PRs" at the top.

### 2.3 Plan review, 2026-10-24: what was done with each point

| # | Review point | Done |
| --- | --- | --- |
| 1 | Parent forwarding of `/mailboxes` breaks the same-registry rule; nothing checks host registry against list registry. | Incorporated. No forwarding for this list (C7-6, Step 2, Step 5). roymctl compares the two registries when the host's node facts can be read and refuses a known difference (C7-15, Steps 7 and 8). Supervisor case: backlog row. |
| 2 | The receipt cap also caps held messages, can block deposits, conflicts with C8, and is not in `max_total_bytes`. | Incorporated in a different form: the cap counts only settled receipts and never refuses a deposit (Q5, C7-11, C7-12, Step 6). So no check relating it to the message limits is needed, and C8's raised quotas are not touched. Pushback on counting receipts inside `max_total_bytes`: that would make the receipt store a reason to refuse deposits again. The settled part gets its own documented disk budget instead. |
| 3 | Receipt-drop effects not fully listed; M-9, M-4 and `system-architecture.md` text become false. | Incorporated: the effects are listed in Q5; M-9 and the architecture paragraph "Receipts and claims limit" change (Steps 12, 13). M-4 does not change: with the new cap no deposit gets a new refusal. |
| 4 | `app reconcile --manifest-path` shows a false `Update` for every `auto`. | Incorporated: reconcile keeps the earlier mailboxes before it diffs (C7-16, Step 8). |
| 5 | Living docs missed: `roym-integrated-experience-spec.md` lines 384 and 702-706; `developer-guide.md:1730` config example. | Incorporated (Step 13). |
| 6 | The supervisor's choice cannot be seen; Step 11d.6 cannot be done; `roymctl supervisor submit` not listed. | Incorporated: `managed-service` gets `mailbox: option<string>` from the stored plan, shown by `supervisor status` (C7-17, Step 9), which prints the JSON as it comes; `roymctl supervisor submit` is a call site for a pin hint (Step 9). Not put in `submit-result`: a submit whose apply fails returns an error, not a result, but the pin is already stored, and `status` shows it in both cases. |
| 7 | A handover from `app deploy` to `supervisor submit` can change the mailbox. | Incorporated: the supervisor keeps the mailbox a member's current record names (Q3 source 2, C7-3). The opposite direction is a backlog row. |
| 8 | Wrong step numbers. | Fixed. |
| 9 | Size is not S. | Incorporated: L, two PRs (C7-19). `task.md` is updated in Step 13. |
| 10 | The supervisor holds the instance lock during the list read (up to 40 s). | Pushback, with a note in Step 9. `submit` already holds this lock across its best-effort apply, which connects to every placed substrate and can take longer than 40 s, so the list read adds no new kind of wait. Reading the list before taking the lock would read the stored desired state outside the lock, and a concurrent write could then change the mailbox. Only the loop of that one instance waits. |
| 11 | `service.rs` is 761 lines. | Noted: the field, its doc and the builder add about 15 lines (about 776). Step 9 says so. If it would pass 790, put the builder in a sibling file. |

## 3. Decisions this plan makes (check them before you start)

- **C7-1 (Q1).** Chooser: `LogicalServiceRef` text for a plan service,
  `--svc-id` for `svc deploy`. One choice per logical service; every member
  gets the same host.
- **C7-2 (Q2).** roymctl reads `--registry-url`; the supervisor reads its
  node's `substrate.registry_url`. The list is read with
  `curated_list::fetch::<MailboxEntry>(…, hops = 0)`, so the 40 s client time
  limit of `D-A4-9` applies.
- **C7-3 (Q3).** Earlier-mailbox sources in order: stored plan, then a
  member's current record (supervisor and `svc deploy` only), then the list.
  A kept mailbox that has left the list stays. The list is read at most once
  per deploy, only when some service needs it.
- **C7-4 (Q4).** No probe. An empty list (after bad entries are dropped) is
  an error that names the service.
- **C7-5.** `MailboxEntry { service_id, operator }`. `check` decodes
  `service_id` as a node DID and writes back its canonical form
  (`derive_did_key(resolve_did_key(..))`), so two spellings of one key are a
  duplicate. The id is the canonical DID.
- **C7-6.** `GET /mailboxes` takes no filter (`MailboxFilter {}`, §2.2
  item 4) and **does not forward to the parent**: `ListEntry` gets
  `const FORWARDS_TO_PARENT: bool` (default `true`; `false` for
  `MailboxEntry`). An empty own list answers an empty list. The answer is
  `{ "max_age_secs": <u64>, "mailboxes": [ { "service_id", "operator" } ] }`.
- **C7-7.** The registry side of `auto` is a trait, `MailboxDirectory`, in
  `syneroym-sdk`, with one HTTP implementation, `RegistryMailboxDirectory`:
  `open_mailboxes()` (the list) and `named_mailbox(service_id)` (the mailbox
  a verified current record names). roymctl and the supervisor both use it;
  tests use a fake. The SDK gets a direct `reqwest` dependency (it is in the
  workspace and in `syneroym-core`). The record check moves to one SDK
  function, `verified_mailbox`, which roymctl's `refuse_if_mailbox` also
  uses.
- **C7-8.** The supervisor gets the directory through a builder method,
  `SupervisorService::with_mailbox_directory`, not a new `new` argument. The
  test fixture file `service/tests/helpers.rs` is on the oversized-test list
  at its cap (1156 lines), so it must not grow; tests call the builder on
  the service that `service()` returns.
- **C7-9.** The supervisor pins after `mint_and_substitute` (a locked vault
  still fails before any network call, as the existing comment requires) and
  before `store.submit`, under the instance lock it already holds (§2.3
  item 10). A failed list or record read refuses the submit with nothing
  stored. A stored desired state whose plan does not parse refuses the
  submit too: guessing a new mailbox could lose messages.
- **C7-10.** roymctl `app deploy` pins after the placement preflight (it
  needs `registry_facts` for C7-15) and before `build_deploy_plan` and
  `resume_or_create_record`, so the journal records the pinned plan and a
  re-run with an unchanged manifest resumes the same record.
- **C7-11 (Q5).** `[roles.mailbox] max_settled_receipts` (default 100,000,
  must be above 0). `receipts` gets a column `settled` (0 while the message
  is held, 1 after `remove_message`) and an index `(settled, expires_at)`;
  `mailbox_stats` gets `settled_receipts`, counted again at open like
  `total_bytes`. The product is not released, so the schema changes in place
  with no migration; an old `mailbox.db` fails at open and must be deleted.
- **C7-12.** `remove_message` settles the receipt and then drops the oldest
  settled receipts while the count is above the cap. A deposit is never
  refused by the cap. The worker sets the gauge
  `substrate.mailbox.settled_receipts`. There is no drop counter: a drop
  inside a transaction that later rolls back would be counted wrongly, and
  the gauge staying at the cap already shows that drops happen.
- **C7-13 (Q6).** The `World` of `mailbox_sender_e2e.rs` moves to
  `tests/common/mailbox_world.rs`; the four new tests go in a new file
  `mailbox_lifecycle_e2e.rs`; the `auto` test goes in a new file
  `mailbox_auto_e2e.rs`.
- **C7-14 (Q7).** Living docs and the traceability matrix are updated; the
  change doc stays `approved`.
- **C7-15.** Same-registry check in roymctl. For each service whose mailbox
  is chosen from the list, compare `normalize_url(list registry)` with
  `normalize_url(host registry)`. `app deploy`: the host registry comes from
  `preflight.registry_facts` of the service's alias; `svc deploy`: from
  `client.node_facts()`. A known difference is an error before anything is
  signed. Unknown facts (credential without node-wide `orchestrator/status`,
  or a service with no alias) print a note and go on, like
  `build_placement_clients` does today. A kept mailbox is not checked (it is
  not a new choice).
- **C7-16.** `app reconcile --manifest-path` applies the earlier mailboxes
  of the Active plan to the compiled plan before `compute_diff`
  (`keep_earlier_mailboxes`, no network). An `auto` with no earlier mailbox
  stays `auto` and shows as a real change.
- **C7-17.** `supervisor status` shows each member's mailbox from the stored
  plan: WIT `managed-service` gets `mailbox: option<string>` (the node DID).
  `roymctl supervisor status` prints the answer JSON as it comes, so it
  shows the field with no change.
- **C7-18.** The supervisor logs each pin at `info` (service, mailbox,
  source: `stored plan`, `current record` or `list`).
- **C7-19.** Two PRs: the receipt cap first, then the list and `auto`.

## 4. Step-by-step changes

Steps 1 (receipt-cap part), 6, the receipt-cap tests of Step 10, and the
receipt-cap parts of Steps 12 and 13 are PR 1. Everything else is PR 2.

### Step 1 — Write the decisions down first (docs only)

`docs/planning/changes/service-mailbox/change.md`:

- `## Deviations`: add `D-C7-1` to `D-C7-n`, written as facts about the
  result (no "C7-n" ids). At least: the chooser (C7-1), the registry and the
  same-registry check (C7-2, C7-15), the earlier-mailbox sources (C7-3), no
  probe (C7-4), the entry check (C7-5), no filter and no forwarding (C7-6),
  the settled-receipt cap and its effects (C7-11, C7-12, Q5), reconcile
  (C7-16), the status field (C7-17).
- `D-C4-4`: add "C4b closed this: the route preamble is signed by the
  caller's key."
- Design "Registry list": replace "the key is `SHA-256` over the service id
  and the mailbox service id" with the C7-1 chooser, and "the same
  forwarding and `502` rule" with "it is not forwarded to a parent registry"
  and the reason (§2.2 item 8).
- Design "Limits": add the settled-receipt cap and its disk budget.
- K-3: remove "`auto` is parsed but refused until the registry list exists"
  and "`auto`: open"; add the pin and the earlier-mailbox rule; fill the test
  column in Step 12.
- M-9: the new text from Q5.

`crates/core/src/curated_list/rank.rs`, the `rank_key` doc comment: replace
"for the mailbox list, the picking service's `service_id` as it appears in
its endpoint record" with "for the mailbox list, the logical service ref
text (`<app_instance_id>/<service_name>`) of a plan service, or the
service's own `service_id` for a single service deployed without a plan".

### Step 2 — `crates/core`: the list entry, no forwarding, the config field

`crates/core/src/curated_list.rs`, `ListEntry`, add after `LIST_KEY`:

```rust
/// Whether a registry with an empty list of this kind asks its parent.
const FORWARDS_TO_PARENT: bool = true;
```

New file `crates/core/src/curated_list/mailbox.rs`:

```rust
//! The mailbox list: open mailboxes that a deployer picks from with `auto`.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use syneroym_identity::substrate;

use crate::curated_list::ListEntry;

/// One entry of `[[roles.community_registry.mailboxes]]`, and of the
/// `GET /mailboxes` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxEntry {
    /// The node DID of the mailbox host, which serves interface `mailbox`.
    /// The entry id.
    pub service_id: String,
    /// Who runs the mailbox. Shown to people; not checked.
    pub operator: String,
}

/// `GET /mailboxes` has no filter. A struct with no fields, not `()`: the
/// query always carries `hops`, and a unit type reads only an empty query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct MailboxFilter {}

impl ListEntry for MailboxEntry {
    const LIST_KEY: &'static str = "mailboxes";
    // A mailbox keeps a sleeping receiver's record alive only in its own
    // node's registry, and a registry lookup never asks the parent. A
    // mailbox from the parent's list would leave senders here without the
    // record once it expires.
    const FORWARDS_TO_PARENT: bool = false;
    type Filter = MailboxFilter;

    fn matches(&self, _filter: &MailboxFilter) -> bool { true }

    fn filter_pairs(_filter: &MailboxFilter) -> Vec<(&'static str, String)> { Vec::new() }

    fn check(&mut self) -> anyhow::Result<String> {
        let key = substrate::resolve_did_key(&self.service_id)
            .with_context(|| format!("service_id {:?} is not a node DID", self.service_id))?;
        // The canonical spelling, so two spellings of one key are a duplicate.
        self.service_id = substrate::derive_did_key(&key);
        Ok(self.service_id.clone())
    }
}
```

`crates/core/src/curated_list.rs`: add `mod mailbox;` and
`pub use mailbox::{MailboxEntry, MailboxFilter};` next to the coordinator
lines.

`crates/core/src/config/roles.rs`:

- import `crate::curated_list::{CoordinatorEntry, MailboxEntry}`;
- `ServiceRegistryRole` gets, after `coordinators`:

  ```rust
  /// `[[roles.community_registry.mailboxes]]`: open mailboxes this registry
  /// lists for `mailbox = "auto"`. Only list a node that runs
  /// `[roles.mailbox]` with the open limits and whose
  /// `substrate.registry_url` is this registry: a mailbox keeps a sleeping
  /// receiver's record alive only in its own node's registry. This list is
  /// never forwarded to `parent_registry_url`.
  pub mailboxes: Vec<MailboxEntry>,
  ```

- `Default`: `mailboxes: Vec::new()`.

Every struct literal of `ServiceRegistryRole` that does not end in
`..Default::default()` must get the field. Find them with
`rg -n 'ServiceRegistryRole \{' crates apps` (at least
`crates/community_registry/src/curated_list/tests.rs:30`,
`crates/substrate/tests/common/node.rs:263`,
`crates/substrate/tests/common/mod.rs:291`; most already use
`..Default::default()`).

### Step 3 — `crates/app_orchestration`: let `auto` through the plan check

`compiler.rs`, `validate_plan_member_settings`:

```rust
// before: match with arms None / Some(Auto) => error / Some(Service(_)) if not public => error / Some(Service(_))
if service.mailbox.is_some() && service.config.visibility != Visibility::Public {
    errors.push(format!(
        "'{name}' sets a mailbox but declares visibility '{}'; only a 'public' service can \
         have a mailbox",
        service.config.visibility.as_str()
    ));
}
```

Doc comment: remove the `auto` bullet. Add one line: "`auto` stays in the
plan. A deployer pins it to a node DID before it stores the plan
(`syneroym_sdk::deploy::pin_auto_mailboxes`)." Drop `MailboxChoice` from the
`use` list if it is no longer used in the file.

`models/service.rs`, the doc of `MailboxChoice::Auto`: "Pick one from the
registry's list of open mailboxes. A deployer replaces it with `Service`
before it stores the plan, and a later deploy keeps that choice."

### Step 4 — `crates/sdk`: the shared pin logic

`crates/sdk/Cargo.toml`: add `reqwest.workspace = true` under
`[dependencies]`.

New file `crates/sdk/src/deploy/mailbox_auto.rs`; in `deploy.rs` add
`pub mod mailbox_auto;` and `pub use mailbox_auto::*;` beside `certify`.

```rust
//! `mailbox = "auto"` (ADR-0026 Decision 8): pick one of a registry's open
//! mailboxes, and keep that choice for every later deploy. A different
//! mailbox means a new signed record, and the messages that wait at the old
//! one are lost.

/// The registry side of `auto`.
#[async_trait]
pub trait MailboxDirectory: Send + Sync {
    /// The registry's open mailboxes, unranked and unchecked.
    async fn open_mailboxes(&self) -> Result<Vec<MailboxEntry>>;
    /// The mailbox host the current record of `service_id` names, if that
    /// record verifies and is for `service_id`. `Ok(None)` when there is no
    /// record or it names no mailbox.
    async fn named_mailbox(&self, service_id: &str) -> Result<Option<ServiceId>>;
}

/// One registry, over HTTP.
pub struct RegistryMailboxDirectory {
    client: reqwest::Client,
    registry_url: String,
}

impl RegistryMailboxDirectory {
    #[must_use]
    pub fn new(registry_url: impl Into<String>) -> Self;   // reqwest::Client::new(); fetch sets its own limit
    #[must_use]
    pub fn registry_url(&self) -> &str;                    // for the same-registry check
}

// impl MailboxDirectory for RegistryMailboxDirectory:
//   open_mailboxes: curated_list::fetch::<MailboxEntry>(&client, &url, &MailboxFilter {}, 0)
//                   .await.with_context(|| format!("reading the open mailboxes of {url}"))?.entries
//   named_mailbox:  GET {url}/lookup/{service_id} with timeout registry_http::HTTP_REQUEST_TIMEOUT;
//                   404 => Ok(None); other non-2xx => Err; body SignedEndpointInfo;
//                   verified_mailbox(service_id, Some(&record))
//                       .and_then(|m| ServiceId::try_new(m.service_id.clone()).ok())

/// The mailbox `record` names, if it is a record of `service_id` that
/// verifies. A wrong or hostile registry must not decide by lying.
#[must_use]
pub fn verified_mailbox<'a>(
    service_id: &str,
    record: Option<&'a SignedEndpointInfo>,
) -> Option<&'a EndpointMailbox> {
    record
        .filter(|r| r.verify().is_ok() && r.info.service_id == service_id)
        .and_then(|r| r.info.mailbox.as_ref())
}

/// The first open mailbox for `chooser` by the rendezvous ranking, after bad
/// and duplicate entries are dropped. `None` when no entry is left.
#[must_use]
pub fn pick_open_mailbox(chooser: &str, entries: Vec<MailboxEntry>) -> Option<ServiceId> {
    curated_list::rank(chooser, entries)
        .into_iter()
        .find_map(|entry| ServiceId::try_new(entry.service_id).ok())
}

/// Where a pinned mailbox came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource { StoredPlan, CurrentRecord, List }

/// One `auto` that was replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedMailbox {
    pub service: LogicalServiceRef,
    pub mailbox: ServiceId,
    pub source: PinSource,
}

/// What a deploy can read to pin `auto`.
#[derive(Clone, Copy, Default)]
pub struct PinSources<'a> {
    /// The plan an earlier deploy of this instance stored.
    pub previous: Option<&'a DeploymentPlan>,
    /// The list registry. `None` when the deployer has none.
    pub directory: Option<&'a dyn MailboxDirectory>,
    /// Whether the plan's `service_id`s are the real member masters, so
    /// their current records can be read. `false` for a plan that still
    /// holds the compiler's made-up ids.
    pub members_are_masters: bool,
}

/// Replaces every `auto` in `plan` with the mailbox of an earlier deploy of
/// the same logical service in `previous`. No network. Returns what it
/// replaced; any other `auto` stays.
pub fn keep_earlier_mailboxes(
    plan: &mut DeploymentPlan,
    previous: Option<&DeploymentPlan>,
) -> Vec<PinnedMailbox>;

/// Replaces every `auto` in `plan` with a node DID, the same one for every
/// member of a service: from the stored plan, else from a member's current
/// record, else from the list, which is read at most once.
///
/// # Errors
/// When a service needs the list and there is no directory, a registry
/// read fails, or the list has no usable entry.
pub async fn pin_auto_mailboxes(
    plan: &mut DeploymentPlan,
    sources: PinSources<'_>,
) -> Result<Vec<PinnedMailbox>>;
```

Pseudo-code (split into small helpers so each function stays short):

```text
keep_earlier_mailboxes(plan, previous):
    pins = []
    for service in auto_services(plan):            // BTreeSet<LogicalServiceRef> with mailbox == Some(Auto)
        if let Some(host) = earlier_mailbox(previous, &service):
            set_mailbox(plan, &service, &host)
            pins.push({ service, mailbox: host, source: StoredPlan })
    pins

pin_auto_mailboxes(plan, sources):
    pins = keep_earlier_mailboxes(plan, sources.previous)
    listed: Option<Vec<MailboxEntry>> = None        // read lazily, once
    for service in auto_services(plan):              // only the ones still `auto`
        directory = sources.directory.ok_or_else(|| anyhow!(
            "'{service}' uses mailbox 'auto', but no registry is given to read the open \
             mailboxes from"))?
        if sources.members_are_masters
            && let Some(host) = first_named_mailbox(plan, &service, directory).await?:
            set_mailbox(plan, &service, &host); pins.push({ .., source: CurrentRecord }); continue
        entries = match &listed { Some(e) => e.clone(),
                                  None => { let e = directory.open_mailboxes().await?; listed = Some(e.clone()); e } }
        host = pick_open_mailbox(&service.to_string(), entries).ok_or_else(|| anyhow!(
            "'{service}' uses mailbox 'auto', but the registry lists no open mailbox; name the \
             mailbox host's node DID instead"))?
        set_mailbox(plan, &service, &host); pins.push({ .., source: List })
    Ok(pins)

earlier_mailbox(previous, service) -> Option<ServiceId>:
    previous?.services.iter()
        .filter(|s| &s.logical_ref == service)
        .find_map(|s| match &s.mailbox { Some(Service(host)) => Some(host.clone()), _ => None })

first_named_mailbox(plan, service, directory) -> Result<Option<ServiceId>>:
    for member in plan.services where logical_ref == service, by member_index:
        if let Some(host) = directory.named_mailbox(member.service_id.as_str()).await?: return Ok(Some(host))
    Ok(None)

set_mailbox(plan, service, host):
    for member in plan.services where logical_ref == service:
        member.mailbox = Some(MailboxChoice::Service(host.clone()))
```

The future must be `Send` (the supervisor awaits it inside a
`NativeService` call); holding `&dyn MailboxDirectory` across an await is
fine because the trait is `Send + Sync`. Do not write `unwrap` (clippy
denies it).

`crates/sdk/src/deploy/certify.rs`, `member_mailbox`, the `Auto` arm keeps
its `bail!`, with new text:
`"service '{}' still has mailbox 'auto'; pin it (pin_auto_mailboxes) before its members are certified"`.

### Step 5 — `crates/community_registry`: the second route

`crates/community_registry/src/curated_list.rs`:

- `router`:

  ```rust
  Ok(Router::new()
      .merge(list_route(role.coordinators.clone(), role, client.clone())?)
      .merge(list_route(role.mailboxes.clone(), role, client)?))
  ```

- `serve_list`, after `let Some(parent) = …`: the existing `else` returns
  the own list; change the guard to also return the own (empty) list when
  `!E::FORWARDS_TO_PARENT`:

  ```rust
  if !E::FORWARDS_TO_PARENT {
      return Ok(Json(own));
  }
  ```

  (put it right after the `if !own.entries.is_empty()` return.)

- `list_route`: still checks `parent_registry_url` for both lists, so a bad
  parent URL stops the registry as before.

### Step 6 — `crates/mailbox`: the settled-receipt cap (PR 1)

`crates/core/src/config/mailbox.rs`, `MailboxRole`, after
`receipt_keep_secs`:

```rust
/// Maximum receipts kept for messages this node no longer holds
/// (delivered, rejected, expired or evicted). Above it the oldest go first,
/// before `receipt_keep_secs` ends. Receipts of held messages are not
/// counted: the message limits bound them.
pub max_settled_receipts: u64,
```

Default `100_000`. In `crates/core/src/config/tests.rs`, add one override
line for it to `a_mailbox_role_parses_with_its_defaults`.

`crates/mailbox/src/limits.rs`: `Limits` gets `pub max_settled_receipts:
u64`; `from_role` refuses `0` with `"[roles.mailbox] max_settled_receipts
must be greater than 0"` and copies the value. `test_support.rs:163`
(`small_limits`) gets a value (for example `1_000`).

`crates/mailbox/src/store.rs`:

- `init_schema`:
  - `receipts` gets `settled INTEGER NOT NULL DEFAULT 0` and
    `CREATE INDEX IF NOT EXISTS receipts_settled ON receipts (settled, expires_at);`
  - `mailbox_stats` gets `settled_receipts INTEGER NOT NULL DEFAULT 0`; the
    `INSERT OR IGNORE` and the `UPDATE` at the end of the batch also set
    `settled_receipts = (SELECT COUNT(*) FROM receipts WHERE settled = 1)`.
    (`receipts` is created before `mailbox_stats` in the batch, so the
    subquery works on a new file.)
- New type, next to `ReceiptEntry`:

  ```rust
  /// How long settled receipts stay, and how many may stay.
  #[derive(Debug, Clone, Copy)]
  pub(crate) struct ReceiptPolicy {
      pub(crate) keep_secs: u64,
      pub(crate) max_settled: u64,
  }
  ```

  and `Limits::receipt_policy(&self) -> ReceiptPolicy` (in `limits.rs`, or a
  `MailboxStore` helper if `Limits` should not know the store type; pick the
  one with no import cycle).
- `remove_message(tx, seq, receipt, keep_secs, now)` becomes
  `remove_message(tx, seq, receipt, policy: ReceiptPolicy, now)`. Its
  receipt `UPDATE` also sets `settled = 1` and uses
  `policy.keep_secs`. After it, when the update changed one row:

  ```text
  UPDATE mailbox_stats SET settled_receipts = settled_receipts + 1 WHERE id = 1
  drop_settled_over_cap(tx, policy.max_settled)?
  ```

- New function:

  ```text
  /// Drops the settled receipts that expire first until at most `max` are
  /// left. A sender that asks about a dropped receipt reads `unknown` and
  /// goes back to direct attempts.
  fn drop_settled_over_cap(tx, max: u64) -> Result<(), MailboxError>:
      count = SELECT settled_receipts FROM mailbox_stats WHERE id = 1   (from_sql)
      if count <= max: return Ok(())
      dropped = tx.execute(
          "DELETE FROM receipts WHERE rowid IN (SELECT rowid FROM receipts WHERE settled = 1 \
           ORDER BY expires_at ASC LIMIT ?1)", [to_sql(count - max)])
      UPDATE mailbox_stats SET settled_receipts = MAX(0, settled_receipts - dropped) WHERE id = 1
  ```

  The receipt that was just settled expires at now + keep time, so it is
  usually not the one dropped. It can be dropped when several receipts tie
  on `expires_at` (they settled in the same second), or when
  `receipt_keep_secs` was lowered since older receipts settled. Both cases
  are harmless: the sender reads `unknown` and goes back to direct attempts.

Callers of `remove_message` and the functions that pass the keep time on —
each changes its `keep_secs: u64` parameter to `policy: ReceiptPolicy`, and
passes `self.limits.receipt_policy()` where it read
`self.limits.receipt_keep_secs`:

| Function | File |
| --- | --- |
| `make_room` (and its call in `deposit`) | `store/deposit.rs` |
| `close_over_claimed` (called from `sweep` and `claim`) | `store/claim.rs:18` |
| `claim` (`remove_message` at line 93) and `ack` (line 186) | `store/claim.rs` |
| `sweep_expired_messages`, `sweep_dead_queues`, their calls in `sweep` | `store/sweep.rs` |
| `unregister` (line 141) | `store/queue.rs` |

`store/sweep.rs`, `sweep`: the receipt delete becomes two parts, so the
counter stays exact:

```text
removed_settled = DELETE FROM receipts WHERE expires_at <= now AND settled = 1
UPDATE mailbox_stats SET settled_receipts = MAX(0, settled_receipts - removed_settled)
removed_held    = DELETE FROM receipts WHERE expires_at <= now AND settled = 0   // should be 0: messages expire first
receipts_removed = removed_settled + removed_held
```

`Totals` gets `pub settled_receipts: u64`, read in `totals()`.
`crates/mailbox/src/worker.rs:88-90` adds
`gauge!("substrate.mailbox.settled_receipts").set(t.settled_receipts as f64);`.

Nothing changes in `deposit` itself, and no deposit gets a new error.

### Step 7 — roymctl `svc deploy`

New module `apps/roymctl/src/commands/mailbox_auto.rs` (declare
`pub(crate) mod mailbox_auto;` in `commands.rs` next to `mailbox_arg`),
with tests in `commands/mailbox_auto/tests.rs`:

```rust
//! `svc deploy --mailbox auto`: a single service has no stored plan, so the
//! mailbox its current record names is the earlier choice to keep.

/// The mailbox host `choice` means for `svc_id`. For `auto`: the mailbox
/// the service's current record names, else the first open mailbox of
/// `registry_url` ranked for `svc_id`. `host_registry` is the hosting
/// node's `substrate.registry_url`, when its node facts could be read.
pub(crate) async fn resolve_svc_mailbox(
    choice: Option<&MailboxChoice>,
    svc_id: &str,
    registry_url: Option<&str>,
    host_registry: HostRegistry<'_>,
) -> anyhow::Result<Option<ServiceId>>;

/// The testable half: no HTTP of its own.
pub(crate) async fn pick_svc_mailbox(
    svc_id: &str,
    directory: &dyn MailboxDirectory,
    list_registry: &str,
    host_registry: HostRegistry<'_>,
) -> anyhow::Result<ServiceId>;

/// What the deployer knows about the registry the hosting node publishes to.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HostRegistry<'a> {
    /// Its node facts could not be read.
    Unknown,
    /// The node has no `substrate.registry_url`.
    None,
    Url(&'a str),
}

/// Refuses a new `auto` choice whose list registry is not the host
/// registry: the mailbox would keep the record alive in one registry while
/// senders look in the other.
pub(crate) fn check_same_registry(
    service: &str,
    list_registry: &str,
    host_registry: HostRegistry<'_>,
) -> anyhow::Result<()>;
```

Pseudo-code:

```text
resolve_svc_mailbox:
    None => Ok(None); Some(Service(h)) => Ok(Some(h.clone()))
    Some(Auto) =>
        url = registry_url.ok_or("--mailbox auto needs --registry-url: that registry lists the open mailboxes")?
        directory = RegistryMailboxDirectory::new(url)
        pick_svc_mailbox(svc_id, &directory, url, host_registry).await.map(Some)

pick_svc_mailbox:
    if let Some(host) = directory.named_mailbox(svc_id).await.context("reading the current record")?:
        println!("--mailbox auto keeps {host}, which the current record of {svc_id} names"); return Ok(host)
    check_same_registry(svc_id, list_registry, host_registry)?
    host = deploy::pick_open_mailbox(svc_id, directory.open_mailboxes().await?)
        .ok_or("--mailbox auto: the registry lists no open mailbox; give the mailbox host's node DID")?
    println!("--mailbox auto chose {host} from {list_registry}"); Ok(host)

check_same_registry(service, list, host):
    match host:
        Unknown => eprintln!("note: cannot check that '{service}' publishes its record to {list} \
                    (the credential cannot read the node's facts); the mailbox works only if it does"); Ok(())
        None    => bail!("'{service}': the hosting node has no registry, so a mailbox from {list} \
                    cannot keep its record findable")
        Url(h)  => if normalize_url(h)? != normalize_url(list)? { bail!("'{service}' publishes its \
                    record to {h}, but mailbox 'auto' reads the list of {list}; use one registry, or \
                    name the mailbox host's node DID") } else Ok(())
```

`apps/roymctl/src/commands/registry.rs`: `refuse_if_mailbox` uses
`syneroym_sdk::deploy::verified_mailbox(service_id, current).is_some()`
instead of its own predicate (one verification rule).

`apps/roymctl/src/commands/svc/deploy.rs`:

- `check_mailbox_args`: delete the `if *mailbox == MailboxChoice::Auto {
  bail!(…) }` block. Signature unchanged. The other checks (needs a master,
  needs `public`) still apply to `auto`.
- `build_publication`: parameter `mailbox: Option<&MailboxChoice>` becomes
  `mailbox: Option<&ServiceId>`. The arm becomes:

  ```rust
  let (route, mailbox_key) = match mailbox {
      Some(host) => {
          let key = MailboxSecretKey::derive_from_master(id);
          (Some(EndpointMailbox { service_id: host.to_string(), key: key.public_key() }), Some(key))
      }
      None => (None, None),
  };
  ```

- `handle_deploy`, after `check_mailbox_args(...)?;`:

  ```rust
  // Before anything is signed or minted: `auto` reads the registry.
  let host_registry = match mailbox {
      Some(MailboxChoice::Auto) => host_registry_of(client).await,
      _ => HostRegistry::Unknown, // not read: nothing to check
  };
  let mailbox_host = mailbox_auto::resolve_svc_mailbox(
      mailbox.as_ref(), svc_id, registry_url.as_deref(), host_registry_ref).await?;
  ```

  where `host_registry_of` calls `client.node_facts().await.ok().flatten()`
  and maps `Some(facts)` to `None`/`Url(..)` by `facts.registry_url`, and a
  failed or empty read to `Unknown`. Keep the owned `String` in a local so
  `HostRegistry::Url` can borrow it. Pass `mailbox_host.as_ref()` to
  `build_publication`.

`apps/roymctl/src/commands/svc.rs`:

- `--mailbox` help: replace "`auto` is not supported yet." with "`auto`
  keeps the mailbox the service's current record names, or picks one of the
  open mailboxes `--registry-url` lists; that registry must be the one this
  substrate publishes to."
- `--registry-url` help: add "Also where `--mailbox auto` reads the open
  mailboxes and the current record; `auto` needs it on both certificate
  paths."

Call sites that change in tests: `apps/roymctl/src/commands/svc/tests.rs`
lines 495, 527 and 666 (`build_publication` now takes `Option<&ServiceId>`;
add a `host_id()` helper next to `host_choice()`), and line 574
(`an_auto_mailbox_is_refused_for_now`, replaced in Step 10).

### Step 8 — roymctl `app deploy` and `app reconcile`

`apps/roymctl/src/commands/app/settings.rs`, new function:

```rust
/// Pins every `auto` mailbox of `plan` before the journal records it, so a
/// re-run resumes the same record and keeps the same mailbox. A service that
/// had a mailbox in this instance's latest journal record keeps it. A new
/// choice must come from the registry its hosting node publishes to.
pub(crate) async fn pin_auto_mailboxes(
    plan: &mut DeploymentPlan,
    journal: &DeploymentJournal,
    directory: Option<&RegistryMailboxDirectory>,
    host_registries: &BTreeMap<SubstrateAlias, (Option<String>, bool)>,
) -> anyhow::Result<()>;
```

Pseudo-code:

```text
previous = journal.get_latest(&plan.app_instance_id)?.map(|r| r.plan)
pins = deploy::pin_auto_mailboxes(plan, PinSources {
           previous: previous.as_ref(),
           directory: directory.map(|d| d as &dyn MailboxDirectory),
           members_are_masters: false,      // the plan still holds made-up ids
       }).await.context("app deploy reads the open mailboxes from --registry-url")?
for pin in &pins where pin.source == List:
    host = host registry of the pin's service:
        alias = first member of pin.service with `substrate` set
        alias -> host_registries.get(alias) -> Some((Some(url), _)) => Url(url)
                                             -> Some((None, _))      => None
                                             -> missing / no alias   => Unknown
    mailbox_auto::check_same_registry(&pin.service.to_string(), directory.registry_url(), host)?
for pin in &pins: println!("mailbox 'auto' for '{}': {} ({})", pin.service, pin.mailbox, source text)
```

The check runs after the pin in memory but before anything is journaled,
signed or placed, so an error leaves nothing behind.

`apps/roymctl/src/commands/app/deploy.rs`, `handle_deploy`:

```text
let mut target_plan = plans.last().cloned().context(...)?;      // now `mut`
... schedule warning, open journal, preflight, namespace check,
    fallback target, placement-change refusal (all unchanged) ...
let directory = registry_url.as_deref().map(RegistryMailboxDirectory::new);
settings::pin_auto_mailboxes(&mut target_plan, &journal, directory.as_ref(),
                             &preflight.registry_facts).await?;
let (deploy_plan, members) = build_deploy_plan(..., &target_plan, ...).await?;   // unchanged
let record_id = resume_or_create_record(&journal, &instance_id, &target_plan)?; // unchanged
```

`resolve_targets` and `check_no_placement_change` read only placement, so
running them before the pin is fine. The "Everything that can bail runs
BEFORE the journal is written" rule holds.

`apps/roymctl/src/commands/app/reconcile.rs:47-50` (C7-16):

```rust
let compiled = compile(instance_id.clone(), &manifest, &catalog).await?;
if let Some(compiled_plan) = compiled.plans.last() {
    // A deploy pinned each `auto`; compare like with like.
    let mut target_plan = compiled_plan.clone();
    deploy::keep_earlier_mailboxes(&mut target_plan, active.as_ref().map(|r| &r.plan));
    let diff = reconciler.compute_diff(&target_plan)?;
```

(`active` is the Active record already read at line 33.)

`apps/roymctl/src/commands/app.rs`, help texts:

- `--registry-url`: add "Also where a mailbox `auto` reads the open
  mailboxes."
- `--mailbox`: "as `<service-name>=<mailbox-host-node-did>` or
  `<service-name>=auto`. `auto` picks one of the open mailboxes
  `--registry-url` lists (it must be the registry the service's substrate
  publishes to), and a later deploy of this instance keeps it."

`apps/roymctl/src/commands/mailbox_arg.rs:7`, doc of
`parse_service_mailbox`: "`auto` is pinned later, before the plan is
journaled."

### Step 9 — Supervisor `submit` and `status`

`crates/app_supervisor/src/service.rs` (761 lines now, about 776 after):

- field: `mailbox_directory: Option<Arc<dyn MailboxDirectory>>` with doc
  "Where `submit` reads the open mailboxes and current records for a
  mailbox `auto`. `None` when this node has no registry configured; a plan
  with an `auto` that no stored plan or record answers is then refused."
- `new`: `mailbox_directory: None` (no new argument, C7-8).
- builder:

  ```rust
  #[must_use]
  pub fn with_mailbox_directory(mut self, directory: Arc<dyn MailboxDirectory>) -> Self {
      self.mailbox_directory = Some(directory);
      self
  }
  ```

`crates/app_supervisor/src/lib.rs`: re-export
`pub use syneroym_sdk::deploy::{MailboxDirectory, RegistryMailboxDirectory};`
next to the other registry writers. The substrate has `syneroym-sdk` only
as a dev-dependency (`crates/substrate/Cargo.toml:125`, under
`[dev-dependencies]`); do not add it.

`crates/substrate/src/runtime/supervisor.rs`, `init_supervisor`: build the
service without `Arc::new`, then

```rust
let supervisor = match config.substrate.registry_url.as_deref() {
    Some(url) => supervisor.with_mailbox_directory(Arc::new(RegistryMailboxDirectory::new(url))),
    None => supervisor,
};
let supervisor = Arc::new(supervisor);
```

importing `RegistryMailboxDirectory` from `syneroym_app_supervisor`.

`crates/app_supervisor/src/service/verbs/submit.rs`:

- `validate_submit_preflight` returns `RpcResult<Option<DesiredState>>`
  instead of `RpcResult<()>`: it already reads `self.store.get(..)`; return
  that value at the end. Its one caller keeps it:
  `let existing = self.validate_submit_preflight(&s, &plan, &inventory).await?;`
- In `handle_submit`, right after `keys::mint_and_substitute(..)` and before
  `plan.to_json()`:

  ```rust
  // Under the instance lock, like the rest of submit: the stored plan read
  // above must still be the one the pin keeps.
  self.pin_auto_mailboxes(&mut plan, existing.as_ref()).await?;
  ```

- New method:

  ```rust
  /// Pins a mailbox `auto` before the desired state is stored, so the loop
  /// and every later submit see one mailbox. A service keeps the mailbox the
  /// stored desired state or a member's current record names.
  async fn pin_auto_mailboxes(
      &self,
      plan: &mut DeploymentPlan,
      existing: Option<&DesiredState>,
  ) -> RpcResult<()> {
      let previous = existing
          .map(|d| DeploymentPlan::from_json(&d.plan_json))
          .transpose()
          .map_err(|e| RpcError::InternalError(format!(
              "the stored plan of this instance does not parse, so its mailbox cannot be \
               kept: {e}")))?;
      let sources = deploy::PinSources {
          previous: previous.as_ref(),
          directory: self.mailbox_directory.as_deref(),
          members_are_masters: true,
      };
      let pins = deploy::pin_auto_mailboxes(plan, sources).await.map_err(|e| {
          let hint = if self.mailbox_directory.is_none() {
              " (this supervisor's node has no substrate.registry_url)"
          } else { "" };
          RpcError::InternalError(format!("{e:#}{hint}; nothing was recorded"))
      })?;
      for pin in pins {
          info!(service = %pin.service, mailbox = %pin.mailbox, source = ?pin.source,
                "mailbox 'auto' pinned");
      }
      Ok(())
  }
  ```

Status (C7-17):

- `crates/wit_interfaces/wit/supervisor/supervisor.wit`, `managed-service`:
  add at the end
  `/// The member's mailbox host (node DID), from the stored plan.` and
  `mailbox: option<string>,`. Check with
  `rg -n 'supervisor' test-components crates/*/wit` that no guest imports
  this interface; if one does, rebuild it (`mise run build:test-components`).
- `crates/app_supervisor/src/service/status.rs`,
  `managed_services_from_report`: it builds one row per health-report
  service. Pass the stored plan in (its caller has the desired state) and
  set `mailbox` from the plan member with the same `member_ref()`:
  `plan.services.iter().find(|p| p.member_ref() == s.member_ref())
   .and_then(|p| match &p.mailbox { Some(MailboxChoice::Service(h)) => Some(h.to_string()), _ => None })`.
  Every other constructor of `ManagedService` (search
  `rg -n 'ManagedService \{' crates apps`) sets `mailbox: None` or the same
  lookup.
- `apps/roymctl/src/commands/supervisor.rs`: no change for `status`. It
  prints the answer JSON as it comes (`call_and_print_json`, line 475), so
  the new `mailbox` field shows by itself. The `submit` handler, after a
  successful call, prints "mailbox 'auto' is pinned by the supervisor; see
  `supervisor status`" when the submitted plan had an `auto` (it compiled the
  manifest itself, so it knows).

The lock: `submit` holds the per-instance lock for the whole call already,
including the best-effort apply. The list read (at most 40 s) adds no new
kind of wait, and only that instance's loop waits (§2.3 item 10).

### Step 10 — Tests (unit and crate level)

Write each test name as a sentence, like the existing ones.

`crates/core/src/curated_list/tests.rs` (412 lines now):
- `a_mailbox_entry_takes_a_node_did_and_writes_its_canonical_form`
- `a_mailbox_entry_that_is_not_a_node_did_is_refused` (`""`,
  `"did:web:example.com"`, `"did:key:not-z32"`)
- `two_mailbox_entries_with_one_did_are_a_duplicate`
- `a_mailbox_list_uses_its_own_json_key` (`"mailboxes"`, round trip)
- `the_mailbox_ranking_is_fixed` (three fixed DIDs and a fixed chooser; the
  expected order is a literal, computed once by the first run, like the
  existing coordinator vector)

`crates/core/src/config/tests.rs`:
- `community_registry_mailboxes_list_parses_from_toml`
- the default test gets `assert!(reg.mailboxes.is_empty())`.

`crates/community_registry/src/curated_list/tests.rs` (489 lines):
`spawn_list_registry` takes coordinators today. Add a `mailboxes:
Vec<MailboxEntry>` parameter, or a second small helper that sets
`role.mailboxes`; pick the one that keeps the file under 800 lines and
passes `cargo dupes`.
- `registry_serves_its_configured_mailbox_list` (`GET /mailboxes` and
  `GET /mailboxes?hops=0` both answer the list: the test for §2.2 item 4)
- `an_empty_mailbox_list_is_not_forwarded_to_the_parent` (the parent lists
  a mailbox; the child answers an empty list and the parent gets no call)
- `the_coordinator_list_still_forwards_when_the_mailbox_list_does_not`
- `a_bad_or_duplicate_mailbox_entry_stops_initialization`

`crates/app_orchestration/src/compiler/tests/member_settings.rs`:
- replace `an_auto_mailbox_parses_but_is_refused_for_now` with
  `an_auto_mailbox_compiles_and_stays_auto_in_the_plan`
- `an_auto_mailbox_on_a_service_that_is_not_public_is_refused` (§2.2
  item 2)

`crates/sdk/src/deploy/tests/mailbox_auto.rs` (new; add `mod mailbox_auto;`
next to `mod mailbox;` in `deploy/tests.rs:759`). A fake
`MailboxDirectory` with fixed entries, fixed records per `service_id`, and
call counters:
- `a_plan_without_auto_reads_nothing`
- `auto_takes_the_first_ranked_open_mailbox_for_every_member` (two members;
  both get `rank("<inst>/<svc>", entries)[0]`)
- `a_stored_mailbox_is_kept_without_any_read`
- `a_stored_mailbox_is_kept_after_it_left_the_list`
- `a_mailbox_in_a_members_current_record_is_kept_when_members_are_masters`
- `current_records_are_not_read_for_made_up_ids`
- `a_named_mailbox_is_never_changed`
- `the_list_is_read_once_for_many_services`
- `an_empty_list_is_an_error_that_names_the_service`
- `a_list_with_only_bad_entries_is_an_error`
- `auto_without_a_directory_is_an_error`
- `keep_earlier_mailboxes_leaves_an_auto_with_no_earlier_mailbox`
- `verified_mailbox_ignores_a_record_that_does_not_verify_or_is_for_another_service`
- `an_unpinned_auto_is_refused_at_certification` (only if
  `deploy/tests/mailbox.rs` has no such test yet)

`apps/roymctl/src/commands/mailbox_auto/tests.rs` (new):
- `auto_keeps_the_mailbox_the_current_record_names`
- `auto_without_a_current_record_takes_the_first_ranked_mailbox`
- `auto_without_a_registry_url_is_refused_before_any_request`
- `a_named_mailbox_needs_no_registry`
- `a_host_on_another_registry_is_refused`
- `a_host_with_no_registry_is_refused`
- `an_unknown_host_registry_only_prints_a_note`
- `the_same_registry_in_another_spelling_passes` (`http://R:80/` and
  `http://r`)

`apps/roymctl/src/commands/svc/tests.rs`: replace
`an_auto_mailbox_is_refused_for_now` with
`an_auto_mailbox_passes_the_argument_checks` (and still fails without a
master, or on a non-public service).

`apps/roymctl/tests/cli_args.rs:634-640`: the test still expects failure (no
`--master`, and `localhost:1` is not reachable); fix the comment: "`auto`
passes the parser; this fails for the missing `--master`."

`apps/roymctl/src/commands/app/tests/settings.rs` (134 lines):
- `a_rerun_keeps_the_mailbox_auto_chose_first` (temp journal; first pin
  with list `[Y, Z]`; append the plan; second pin with a changed list keeps
  the first host)
- `auto_with_no_registry_and_no_journal_record_is_refused`
- `a_new_choice_for_a_host_on_another_registry_is_refused_and_nothing_is_journaled`
- a reconcile test, where the reconcile tests live
  (`rg -n 'handle_reconcile|compute_diff' apps/roymctl/src`):
  `reconcile_shows_no_change_for_a_pinned_auto`.

`crates/app_supervisor/src/service/tests/mailbox_auto.rs` (new; add
`mod mailbox_auto;` to `service/tests.rs`). Build the plan JSON like
`member_settings.rs:58-90`, with `"mailbox": "auto"`, and use
`service().with_mailbox_directory(Arc::new(fake))`. The best-effort apply
fails (nothing is placed), but the desired state is stored first, so read
`s.store.get("inst-…")` after the call:
- `submit_pins_auto_in_the_stored_plan`
- `a_second_submit_keeps_the_pinned_mailbox_when_the_list_changes`
- `a_first_submit_keeps_the_mailbox_a_members_record_names` (the handover
  case)
- `auto_on_a_node_without_a_registry_is_refused_and_nothing_is_stored`
- `an_empty_open_list_refuses_the_submit_and_stores_nothing`
- `a_locked_vault_refuses_before_the_registry_is_read` (fake counts zero
  calls)
- `status_shows_the_pinned_mailbox` (in the status tests' file if that fits
  better)

`crates/mailbox` (PR 1):
- `store/tests/claim.rs` or `deposit.rs`:
  `settling_past_the_cap_drops_the_settled_receipt_that_expires_first`
  (settle each message at a different `now`, so no two receipts tie on
  `expires_at` and the dropped one is always the same),
  `a_receipt_of_a_held_message_is_never_dropped`,
  `deposits_are_accepted_when_the_settled_receipts_are_at_the_cap`,
  `a_dropped_receipt_reads_unknown_in_status`,
  `an_identical_retry_after_its_receipt_was_dropped_is_stored_again` (pins
  the documented effect).
- `store/tests/sweep.rs`: `the_settled_count_follows_settles_drops_and_sweeps`.
- a file-backed reopen test:
  `the_settled_count_is_counted_again_at_open`.
- `limits/tests.rs`: `max_settled_receipts_zero_is_refused`.

### Step 11 — Substrate tests (end to end)

All new tests use `SubstrateNode` and dynamic ports. No port literal.

**11a. `crates/substrate/tests/curated_list_e2e.rs`** (79 lines):
- `booted_substrate_serves_curated_mailboxes_list`
- `duplicate_mailbox_ids_stop_substrate_init`

**11b. Move the mailbox `World`.** Move `World`, `Restart`, `Options`,
`mailbox_role`, `apply_all`, `greet_entry`, `world()` and the `impl World`
block from `mailbox_sender_e2e.rs` (lines 40-321) to
`crates/substrate/tests/common/mailbox_world.rs` (`pub` items,
`#![allow(dead_code)]` like the other common files), and declare it in
`tests/common/mod.rs`. `mailbox_sender_e2e.rs` keeps its tests and imports
the world. Add to `World`:
- the sending guest's master (`caller_master: Identity`), so a test can
  deploy the guest again after the sender restarts;
- `Options.registry_on_mailbox: bool` (default `false`): the mailbox node
  boots first and hosts the registry, and the sender and the receiver use
  `shared_registry(..)`. The offline-sender test needs it, because a stopped
  sender must not take the registry down with it;
- `Options.deploy_receiver: bool` (default `true`);
- `Options.max_queue_messages: Option<u32>`;
- `restart_sender()`: `self.sender = self.sender.restart().await` (keeps
  ports and identity), then deploy the guest again with `caller_master`.

Check after the move: is the proxy outbox of a guest worked off after a node
restart before the guest is deployed again? If yes, `restart_sender` needs
no redeploy. Write down what you found in the change doc deviations.

**11c. New file `crates/substrate/tests/mailbox_lifecycle_e2e.rs`:**

1. `a_sender_that_was_offline_completes_its_row_when_it_returns`
   (`registry_on_mailbox: true`): stop receiver; enqueue `msg-1`; wait
   deposited; stop the sender; restart the receiver and pull (delivered and
   acked while the sender is away); restart the sender; wait for an empty
   outbox (its poll reads the signed `delivered`); no dead letter; the
   method ran once.
2. `a_message_waits_for_a_receiver_that_has_no_address` (stands for a
   receiver asleep longer than 2 hours, `deploy_receiver: false`): the
   `MailboxReceiver` double registers the queue with a record whose
   `substrate_id` is a fresh DID that has no record (as
   `mailbox_record_e2e.rs:150` does); enqueue; wait deposited and check the
   row used no direct attempt beyond the enqueue probe; deploy the receiver
   on the receiver node (a newer instance certificate takes the queue) and
   pull; empty outbox; the method ran once.
3. `a_moved_receiver_opens_messages_sent_before_the_move`: stop the
   receiver node A; enqueue; wait deposited; boot a new node B (new
   directory, `shared_registry`), deploy the same `MailboxReceiver` master
   there (same derived mailbox key, newer certificate), pull on B; empty
   outbox; ran once on B.
4. `a_full_queue_keeps_the_second_call_on_direct_attempts_and_both_land_once`
   (`max_queue_messages: Some(1)`): stop receiver; enqueue `msg-1` and
   `msg-2`; wait until exactly one row is deposited and the other is not
   (`queue_full` keeps it on direct attempts); restart receiver and pull;
   empty outbox; two runs in total.

**11d. New file `crates/substrate/tests/mailbox_auto_e2e.rs`:**

`a_submit_with_auto_pins_a_listed_mailbox_and_its_record_names_it`:
1. Boot node N with `.supervisor(..)`, `[roles.mailbox]` (through
   `configure`), `inject_kek_bytes`, and a base path.
2. `let did = N.did().to_string();` then
   `N = N.restart_with(move |b| b.configure(move |c| { c.roles.mailbox = Some(..);
   c.roles.community_registry.as_mut().unwrap().mailboxes = vec![MailboxEntry { service_id: did.clone(), operator: "test".into() }]; }))`.
   `restart_with` keeps the ports and the identity, so the registry URL is
   unchanged. The new `configure` hook replaces the old one, so it must set
   the mailbox role again.
3. `GET {registry}/mailboxes` lists N.
4. Submit a one-service public plan placed on N with `"mailbox": "auto"`
   (the plan and inventory shape of `durable_outbox_e2e.rs`; if you copy
   `submission` and `submit_with_retry`, move them to
   `tests/common/supervisor.rs` instead, or `cargo dupes` flags them).
5. The member master DID comes from the submit answer (`masters[0]`). Look
   up its record in the registry: `info.mailbox.service_id == N.did()`.
6. `supervisor status` (C7-17) shows `mailbox == N.did()` for the member.

### Step 12 — Change doc test columns

In `docs/planning/changes/service-mailbox/change.md`:
- K-3: the tests of Steps 10 and 11 (`mailbox_auto` in the SDK, roymctl
  and supervisor; `curated_list` in core and the registry;
  `curated_list_e2e.rs`; `mailbox_auto_e2e.rs`; the reconcile test).
- M-9 (PR 1): the new text (Q5) and the settled-cap tests.
- Add `mailbox_lifecycle_e2e.rs` tests where they prove a criterion:
  offline sender → S-5; no address → K-5 and S-2; relocation → K-2 and
  M-12; full queue → M-4 and S-4.
- Tasks: tick "Registry list and `GET /mailboxes`", "Integration tests"
  (with the note that the 2-hour case is simulated and that expired
  certificate and duplicate delivery are covered below the end-to-end
  level), "Update living docs". Leave "Resolve the open questions" and
  "dedicated mailboxes" open.

### Step 13 — Living docs, backlog, status

Living docs (each must describe the result; link ADRs only, no change-doc or
slice ids):

- `docs/system-architecture.md`:
  - config table (~382): add `[[roles.community_registry.mailboxes]]`
    (PR 2) and `[roles.mailbox] max_settled_receipts` (PR 1).
  - curated lists (~2208): the mailbox list, its entry fields, the
    canonical DID check, no forwarding and why, and the rule that an
    operator lists only a mailbox whose node publishes to this registry.
  - "Receipts and claims limit" (~2114, PR 1): receipts stay 14 days, or
    less when more than `max_settled_receipts` settled receipts are held;
    the oldest go first; what a sender then sees (`unknown`, back to direct
    attempts); deposit idempotency lasts as long as the receipt.
  - the mailbox section (~2060-2100): `auto`, the chooser, the earlier
    mailbox sources, the same-registry check, no probe, `supervisor status`.
- `docs/system-requirements-spec.md:1084`: remove `--mailbox auto` and the
  open list from the "Envisioned" note; only dedicated mailboxes stay
  envisioned.
- `docs/developer-guide.md`: `--mailbox auto` for `svc deploy` and `app
  deploy` (~412-460), which registry it reads, the same-registry check, the
  pin; a `curl 'http://localhost:7961/mailboxes'` example next to the
  coordinator one (~319); the `[roles.mailbox]` config example (~1730) gets
  `max_settled_receipts = 100000` with its comment (PR 1); the
  `[roles.community_registry]` example, if there is one, gets a
  `[[roles.community_registry.mailboxes]]` entry.
- `docs/TERMINOLOGY.md:44`: "the coordinator list is one" → "the coordinator
  list and the open mailbox list are two". Add "mailbox", "queue lifetime"
  and "sealed message" if they are still missing.
- `docs/planning/traceability-matrix.md:56` (`[PLT-ASY]`): open listing and
  `auto` are implemented; only dedicated mailboxes are Envisioned; add the
  new test files. Line 25 (`[TOP-REG]`): the mailbox list.
- `docs/roym-integrated-experience-spec.md`: every statement that depends on
  decision D4:
  - line 69 (D4 itself): a message waits in the sender's outbox and, for a
    receiver that names a mailbox, at that mailbox (ADR-0026); the mailbox
    sees only metadata.
  - line ~384 (C11): "the message waits in the sender's own outbox, or at
    the receiver's mailbox, and shows as `pending` or `deposited` until it
    is delivered".
  - lines ~702-706 ("There is no third-party buffer (D4)"): with a
    mailbox, the two sides do not need to be online together; without one,
    the old consequence stays and must be shown.
  - search the file for other `D4` references (`rg -n 'D4' …`) and fix each.

`docs/planning/deferred-backlog.md`:
- move to "Recently resolved": "Service mailbox: `auto` mailbox choice"
  (PR 2) and "Receipt table size is not capped by `max_total_bytes`" (PR 1),
  with the date, what was done and the files.
- edit "The mailbox republishes only to its own registry": the list is not
  forwarded, the config tells the operator to list only mailboxes of the
  same registry, and roymctl checks the host registry when it can.
- add:
  - "`auto` does not check that the chosen mailbox answers" (TBD;
    `crates/sdk/src/deploy/mailbox_auto.rs`).
  - "The supervisor does not check that its hosts publish to its registry
    before it pins `auto`" (TBD; it reads host facts only after the desired
    state is stored; `crates/app_supervisor/src/service/verbs/submit.rs`).
    Name both effects in the row: a new choice may come from a list that
    the hosts' senders do not use, and the handover from `app deploy` keeps
    the earlier mailbox only when the earlier hosts published to the
    supervisor's own registry, because that is the only registry where it
    reads the members' current records.
  - "`app deploy` after a supervisor release can pick a new mailbox for
    `auto`" (its plan holds made-up ids when it pins, so it cannot read the
    members' current records; TBD; `apps/roymctl/src/commands/app/settings.rs`).
  - "A dropped settled receipt sends its sender back to direct attempts"
    (PR 1; an offline sender may come back to `unknown` and deposit again;
    TBD; `crates/mailbox/src/store.rs` `drop_settled_over_cap`).

`task.md`: line 5 status; the C7 entry gets its new size (L, two PRs) and,
when both PRs have merged, "Done <date>. Decisions `D-C7-1` to `D-C7-n` are
in the mailbox change doc."; the §4 table row for C7 says `L`.
`status.md`: replace the `C7 and C8` row with a `C7` row (what was built,
by whom, evidence: test files and counts, `mise run verify` result) and a
`C8 | Not started` row.

### Step 14 — Finish (each PR)

1. `cargo +nightly-2026-04-06 fmt --all`.
2. Import clean-up pass over every edited file (types by `use`, functions
   through their parent module, no inline paths with more than one `::`).
3. `cargo dupes report` filtered to your files (recipe in `AGENTS.md`). The
   new list test helpers and the moved `World` are the likely hits.
4. Ratchets: if `mailbox_sender_e2e.rs` or any listed file shrank below its
   recorded value, update `xtask/oversized-test-files.txt`; lower
   `MAX_EXACT_DUPLICATE_PERCENT` if the gate says so.
5. PR 2 changes `supervisor.wit`: if any guest imports it (Step 9), run
   `mise run build:test-components` first. No Roym crate changes, so no
   `mise run build:roym`.
6. `mise run verify` as a background job, no `--skip`. All gates pass.
7. Do not commit on `main`. On the branch, commit with a Conventional
   Commits subject, for example `feat(mailbox): cap settled receipts` (PR 1)
   and `feat(mailbox): list open mailboxes and pin auto` (PR 2).

## 5. Call sites that change (one list)

| Item | Where | PR |
| --- | --- | --- |
| `MailboxRole.max_settled_receipts` | `crates/core/src/config/mailbox.rs` | 1 |
| `Limits.max_settled_receipts`, `receipt_policy` | `crates/mailbox/src/limits.rs`; `test_support.rs:163` | 1 |
| Schema (`receipts.settled`, index, `mailbox_stats.settled_receipts`); `ReceiptPolicy`; `remove_message(…, policy, …)`; `drop_settled_over_cap` | `crates/mailbox/src/store.rs` | 1 |
| `keep_secs` → `policy` | `store/deposit.rs` (`make_room`, `deposit`), `store/claim.rs` (`close_over_claimed`, `claim`, `ack`), `store/sweep.rs` (`sweep_expired_messages`, `sweep_dead_queues`, `sweep`), `store/queue.rs` (`unregister`) | 1 |
| Sweep counts settled; `Totals.settled_receipts`; gauge | `store/sweep.rs`; `worker.rs:88-90` | 1 |
| `ListEntry::FORWARDS_TO_PARENT` | `crates/core/src/curated_list.rs` | 2 |
| New `MailboxEntry`, `MailboxFilter` | `crates/core/src/curated_list/mailbox.rs` (new), `curated_list.rs` | 2 |
| `ServiceRegistryRole.mailboxes` | `crates/core/src/config/roles.rs`; literals without `..Default::default()` | 2 |
| `router` merges the mailbox route; `serve_list` checks `FORWARDS_TO_PARENT` | `crates/community_registry/src/curated_list.rs` | 2 |
| `rank_key` doc | `crates/core/src/curated_list/rank.rs` | 2 |
| `validate_plan_member_settings` | `crates/app_orchestration/src/compiler.rs:126-166` | 2 |
| `MailboxChoice::Auto` doc | `crates/app_orchestration/src/models/service.rs:308` | 2 |
| New `MailboxDirectory`, `RegistryMailboxDirectory`, `verified_mailbox`, `pick_open_mailbox`, `PinSource`, `PinnedMailbox`, `PinSources`, `keep_earlier_mailboxes`, `pin_auto_mailboxes` | `crates/sdk/src/deploy/mailbox_auto.rs` (new), `deploy.rs`, `crates/sdk/Cargo.toml` (`reqwest`) | 2 |
| `member_mailbox` message | `crates/sdk/src/deploy/certify.rs:209` | 2 |
| `check_mailbox_args` (drop `auto` refusal) | `apps/roymctl/src/commands/svc/deploy.rs:107` | 2 |
| `build_publication(…, mailbox: Option<&ServiceId>, …)` | `svc/deploy.rs:167`, caller `svc/deploy.rs:456`, tests `svc/tests.rs:495,527,666` | 2 |
| `handle_deploy` reads host facts and calls `resolve_svc_mailbox` | `svc/deploy.rs` after line 448 | 2 |
| New `resolve_svc_mailbox`, `pick_svc_mailbox`, `HostRegistry`, `check_same_registry` | `apps/roymctl/src/commands/mailbox_auto.rs` (new), `commands.rs` | 2 |
| `refuse_if_mailbox` uses `verified_mailbox` | `apps/roymctl/src/commands/registry.rs:77-90` | 2 |
| Help texts | `commands/svc.rs:113-119,150-157`; `commands/app.rs:150-165`; `commands/mailbox_arg.rs:7` | 2 |
| New `settings::pin_auto_mailboxes` and its call after preflight | `app/settings.rs`; `app/deploy.rs` `handle_deploy` (`target_plan` becomes `mut`) | 2 |
| Reconcile keeps earlier mailboxes | `apps/roymctl/src/commands/app/reconcile.rs:47-50` | 2 |
| `SupervisorService.mailbox_directory`, `with_mailbox_directory` | `crates/app_supervisor/src/service.rs` | 2 |
| Re-export `MailboxDirectory`, `RegistryMailboxDirectory` | `crates/app_supervisor/src/lib.rs` | 2 |
| `init_supervisor` sets the directory | `crates/substrate/src/runtime/supervisor.rs:68-89` | 2 |
| `validate_submit_preflight` returns the stored state; `pin_auto_mailboxes` method; call in `handle_submit` | `crates/app_supervisor/src/service/verbs/submit.rs:224,233,304-342` | 2 |
| `managed-service.mailbox` | `crates/wit_interfaces/wit/supervisor/supervisor.wit:42-55`; `crates/app_supervisor/src/service/status.rs` (`managed_services_from_report` and every `ManagedService {`) | 2 |
| `supervisor submit` prints the pin hint (`status` needs no change: it prints the JSON as it comes) | `apps/roymctl/src/commands/supervisor.rs` | 2 |
| Shared e2e world | `crates/substrate/tests/common/mailbox_world.rs` (new), `common/mod.rs`, `mailbox_sender_e2e.rs` | 2 |

## 6. What C7 does not do

- No dedicated mailboxes, no passport on `register`, no child token in
  `mailbox-credentials` (C8).
- No probe of the chosen mailbox (Q4), no automatic move to another
  mailbox, no failover.
- No forwarding of the mailbox list to a parent registry.
- No same-registry check in the supervisor, and no current-record source in
  `app deploy` (backlog rows).
- No change to the puller, the sender path or the wire formats.
- No `auto` for a plan of a spawned app inside `app deploy` (§2.2 item 9).
- No end-to-end test of a real 2-hour registry expiry, an expired
  certificate, or a lost direct answer (simulated, or covered below the
  end-to-end level).

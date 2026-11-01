# Prompt templates for slice steps

*Reader: the project owner, who starts a new agent session for each step of a milestone or slice.*

This file has one prompt for each step of the work in [session-strategy.md](session-strategy.md). You copy a template, change the values in its **Fill in** block, and send it to a new session. You change nothing else. The text below the line uses the names in braces, for example `{SLICE}`. The agent replaces each name with your value. File paths are built from the values, so you never edit a path.

## Values

| Name | Example | Meaning |
|---|---|---|
| `MILESTONE` | `M08A` | The milestone id. |
| `MILESTONE_DIR` | `M08A-open-foundation` | The milestone folder under `docs/planning/milestones/`. |
| `SLICE` | `A3` | The slice id. File names use it in lower case (`slice-a3-...`). |
| `TITLE` | `the batched outbound call (call-all)` | A few words on what the slice builds. |
| `BRANCH` | `feat/m08a-a3-call-all` | The branch of the slice. |
| `PR` | `338` | The pull request number. |
| `CHANGE_DOCS` | `docs/planning/changes/roym-open-foundation/change.md` | Every change doc the slice builds part of, separated by commas. |
| `ME` | `Claude Opus 5.5` | The tool that runs this prompt. |
| `AUTHOR` | `Gemini 3.8 Flash` | Who wrote the plan or the code that is reviewed. |
| `ROUND` | `1`, `2`, or `1 — fixes confirmed` | The review round. |
| `FOCUS` | `none` | Extra points for this slice or round. |

## Default tools

The rule: a reviewer is never the author of what it reviews.

| Step | Tool |
|---|---|
| Plan | Claude Opus 5.5 |
| Plan review | Claude Sonnet 5.5, high effort |
| Plan fix | The plan author |
| Implementation | Gemini 3.8 Flash |
| Code review | Claude Opus 5.5 |
| Code fix | The implementer |
| Fix confirm round | Any tool except the one that made the fixes |

If Sonnet implements a slice, it does not review that slice's plan. Gemini reviews the plan then.

## Order

- **A slice:** T1 → T2 → T3 → (T2 and T3 again, until no item is open) → T4 → T5 → T6 → (T5 and T6 again, until no item is open) → T7 → the owner merges.
- **A milestone:** T0 first, then each slice in order, then T8.
- **Before M08C, M08E, M08F and M08G:** T9 for each new change doc, then T2 and T3 on the change doc in place of a plan.

Every session works in the main checkout on one normal branch. Each template says so, and the agent stops if it is in a git worktree.

## T0 — Milestone start

```text
=== Fill in ===
MILESTONE: M08B
MILESTONE_DIR: M08B-<name>
PREV_MILESTONE_DIR: M08A-open-foundation
CHANGE_DOCS: docs/planning/changes/<name>/change.md
ME: Claude Opus 5.5
=== Do not edit below this line ===
{NAME} in the text means the value above.
DIR = docs/planning/milestones/{MILESTONE_DIR}
CHANGE_DOCS is a list. Read every doc in it. If task.md names a
change doc for this milestone that is not in the list, read it too
and tell me.

Start milestone {MILESTONE}: refresh its task list and create its
status file, before any slice is planned.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym.
  git switch main && git pull && git switch -c docs/{MILESTONE}-start
(branch name in lower case). If you are in a git worktree, stop and
tell me. I am {ME}.

Read first: AGENTS.md ("Writing for readers"); session-strategy.md;
docs/planning/milestones/M08-overview.md; everything in {DIR}; each
doc in {CHANGE_DOCS}; task.md and status.md in
docs/planning/milestones/{PREV_MILESTONE_DIR} (what it built and
deferred); docs/planning/deferred-backlog.md (rows that target
{MILESTONE}).

1. Refresh {DIR}/task.md against the code on main and what earlier
   milestones built or changed: fix facts that are now false, add
   deferred rows that target {MILESTONE}, and make the slice list,
   order, sizes and criteria right. For each slice, name the change
   docs it builds part of. Do not add features; ask me if the scope
   should change.
2. Create {DIR}/status.md in the layout of
   docs/planning/milestones/M08A-open-foundation/status.md: one row
   per slice, Step "Not started", the four tool columns, Evidence.
3. Commit (docs(planning): start milestone {MILESTONE}), push, open a
   PR whose body contains this exact line on its own:
   - [x] I have read and agree to the [Syneroym CLA](https://github.com/syneroym/syneroym/blob/main/CLA.md).
   Do not merge.

Finish with: what changed in task.md and why, open questions for me,
and the first slice to plan.
```

## T1 — Plan a slice

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A4
TITLE: the federated query library and one search call
BRANCH: feat/m08a-a4-federated-query
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Claude Opus 5.5
FOCUS: none
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
CHANGE_DOCS is a list. Read every doc in it. If task.md names a
change doc for this slice that is not in the list, read it too and
tell me. Each acceptance criterion, decision and deviation belongs to
the change doc that defines the criterion or the design it is about.
Write it there, and only there. If unsure which doc owns something,
ask me.

Write the implementation plan for slice {SLICE} of {MILESTONE}:
{TITLE}.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym.
  git switch main && git pull
Check in {DIR}/status.md that every slice {SLICE} depends on (see
{DIR}/task.md) is "Merged". If not, stop and tell me. Then:
  git switch -c {BRANCH}
If you are in a git worktree, stop and tell me. I am {ME}.

Read first: AGENTS.md (all of it; "Writing for readers": the plan
needs the section "How this is shown to an end user");
session-strategy.md; {DIR}/task.md (section {SLICE} and its
dependencies) and {DIR}/status.md; each doc in {CHANGE_DOCS}
(criteria, design and decisions for this slice); the plan of the
previous slice in {DIR} (use its section layout); every
slice-*-review.md in {DIR} (learn from their findings; do not repeat
them).

Extra points for this slice: {FOCUS}

Rules for the plan:
- Check every fact in the code on main, with file:line links:
  functions, types, callers (UI, roymctl, other services, other
  nodes, tests, parity scenarios, e2e and demo stories).
- List which criteria of which change doc the slice meets.
- Reuse existing helpers and machinery; no new machinery without a
  reason.
- A behaviour-change audit: every wire or behaviour change listed;
  everything else stays the same.
- Owner decisions as numbered questions, each with options and your
  recommendation.
- Step-by-step changes (each step builds and its tests pass), tests,
  size risks, deferred backlog, living docs, exit criteria, and
  "what {SLICE} does not do".

Write it to {PLAN}. Set the {SLICE} row in {DIR}/status.md to "Plan
written" with your tool name. Commit (docs(planning): plan
{MILESTONE} slice {SLICE}) and push. No code changes, no PR yet.

Finish with: the owner questions with your recommendations, and the
biggest risks you see.
```

## T2 — Plan review, any round

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A3
TITLE: the batched outbound call (call-all)
BRANCH: feat/m08a-a3-call-all
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Claude Opus 5.5
AUTHOR: Claude Sonnet 5.5
ROUND: 1
FOCUS: none
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
REVIEW = {DIR}/slice-{slice}-review.md
CHANGE_DOCS is a list. Read every doc in it. If task.md names a
change doc for this slice that is not in the list, read it too and
tell me. Each acceptance criterion, decision and deviation belongs to
the change doc that defines the criterion or the design it is about.

Plan review round {ROUND} of slice {SLICE} of {MILESTONE} ({TITLE}).

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym on
branch {BRANCH} (git pull first). If you are in a git worktree, stop
and tell me. I am {ME}. The plan was written by {AUTHOR}.

Read first: AGENTS.md ("Writing for readers" and the code rules);
session-strategy.md ("The review file"); {PLAN}; {DIR}/task.md and
{DIR}/status.md; each doc in {CHANGE_DOCS}; {REVIEW} if it exists;
the other slice-*-review.md files in {DIR} (kinds of findings
before).

If {REVIEW} has earlier rounds: first check every item marked Fixed
or Rejected in the last round. A wrong or partial fix goes back to
Open with a note; a rejection you disagree with gets your reason.
Then review the whole plan again for new items.

Spend most of your time on what the plan does not cover: callers,
code paths, edge cases, limits, security and wire changes that are
missing. Not on rewording what is there. Check facts against the
code on main.

Check at least: facts and file:line links; missed callers; security
and access control; both builds (WASM and native) where it applies;
the behaviour-change audit; that the plan names the right change doc
for each criterion and decision; that steps build in order and size
rules hold; tests, backlog, living docs, exit criteria, "How this is
shown to an end user". Give your view on each open owner question
(do not answer for the owner).

Extra points for this round: {FOCUS}

Write into {REVIEW} (create it if missing), following "The review
file": a new section "Plan review round {ROUND} — {ME}, <today>",
what was checked and found correct, counts (blocker / major / minor),
your view per question, then new findings with the next free P
number, each with severity, where, problem, failure case, concrete
fix, *Status:* Open. Never renumber. Do not edit {PLAN}.

Set the {SLICE} row in {DIR}/status.md to Step "Plan reviewed", your
tool in the plan-review column, open count in Evidence. Commit
(docs(planning): {SLICE} plan review round {ROUND}) and push.

Finish with: the counts, each blocker and major in one line, your
view per question, and "0 open" if nothing is open.
```

## T3 — Plan fix round

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A3
BRANCH: feat/m08a-a3-call-all
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Claude Sonnet 5.5
ROUND: 1
OWNER_ANSWERS: Q1 (a), Q2 (a), Q3 (a)   (or: none new)
ANSWER_DATE: 2026-11-01
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
REVIEW = {DIR}/slice-{slice}-review.md
CHANGE_DOCS is a list. Each decision and open question belongs to
the change doc that defines the design it is about. Write it there,
and only there. If unsure which doc owns something, ask me.

Fold the owner answers and the open plan review items into the plan
of slice {SLICE} of {MILESTONE}.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym on
branch {BRANCH} (git pull first). If you are in a git worktree, stop
and tell me. I am {ME}.

Read first: AGENTS.md ("Writing for readers"); session-strategy.md
("The review file"); {PLAN}; {REVIEW} (every item with *Status:*
Open); each doc in {CHANGE_DOCS}.

Owner answers ({ANSWER_DATE}): {OWNER_ANSWERS}

1. Record each answer in {PLAN}: under the question write "Owner
   answer: (x), {ANSWER_DATE}", and make the decisions and steps
   follow it. If a doc in {CHANGE_DOCS} lists this slice's decisions
   or open questions, update that doc the same way.
2. For each open P-item: check it against the code yourself. If you
   agree, fix the plan as the item says (or more simply, same
   result). If you disagree, set `Rejected: <reason>` and do not
   change the plan for it.
3. No code changes. Do not rewrite parts no item or answer touches.
4. In {REVIEW} set each item to `Fixed (<sha>)` or
   `Rejected: <reason>`, tick fixed boxes. Never renumber.
5. {DIR}/status.md {SLICE} row: keep Step "Plan reviewed"; Evidence:
   answers recorded, items closed (short).
6. Commit {PLAN} and any change doc first (docs(planning): fold
   {SLICE} review round {ROUND}), then {REVIEW} and status.md
   (docs(planning): close {SLICE} plan review round {ROUND}). Push.

Finish with: a table of the P-items with Fixed or Rejected and the
sha or reason, and where the answers changed the steps.
```

## T4 — Implement

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A3
TITLE: the batched outbound call (call-all)
BRANCH: feat/m08a-a3-call-all
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Gemini 3.8 Flash
FOCUS: none
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
REVIEW = {DIR}/slice-{slice}-review.md
CHANGE_DOCS is a list. Read every doc in it. Each decision and
deviation belongs to the change doc that defines the design it is
about. Write it there, and only there. If unsure which doc owns
something, ask me.

Implement slice {SLICE} of {MILESTONE} ({TITLE}), following its
reviewed plan.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym on
branch {BRANCH} (git pull first). If you are in a git worktree, stop
and tell me. I am {ME}.

Read first: AGENTS.md (all of it: code quality, imports, comments,
context budget, the Mandatory items); {PLAN} (the source of truth:
owner answers, decisions, steps, tests); {REVIEW}; each doc in
{CHANGE_DOCS}; the code review rounds in the other slice-*-review.md
files in {DIR} (mistakes made before; do not repeat them).

Rules:
- Follow the plan's steps in order, one or more commits per step.
  Conventional commit messages. No milestone, slice, step, decision
  or review ids in code, comments or test names.
- Change only what the plan asks. Do not rewrite function bodies,
  reorder checks or swap in other helpers unless the plan says so.
  Keep every "why" comment that is still true.
- If the plan is wrong or the code differs from it, choose the
  simplest correct fix and record it under Deviations, "Different
  from the plan", in the doc of {CHANGE_DOCS} that owns that design,
  with the reason. If it changes the wire, security or a decision,
  stop and ask me.
- If a roym_* crate changed, run `mise run build:roym` before the
  parity or substrate roym tests; check those tests ran and did not
  skip.
- After each step: targeted `cargo nextest run -p <crates you
  touched>`, `cargo clippy -q --workspace --all-targets
  --all-features -- -D warnings`, `cargo +nightly-2026-04-06 fmt
  --all`. Use the AGENTS.md context-budget recipes. Do not run the
  full `mise run verify`; CI runs the full suite on push, and the
  slice close step handles verify.
- Docs step: living docs, deferred backlog rows, change doc
  Deviations, `cargo dupes` over your files, the import cleanup pass,
  the demo close-out, and the ratchets if a gate says they improved.

Extra points for this slice: {FOCUS}

When all steps are done:
1. {DIR}/status.md {SLICE} row: Step "Implemented", your tool in the
   implementation column, short Evidence (PR link, test counts as the
   tests print them).
2. Push and open a draft PR to main (`gh pr create --draft`). Body:
   what the slice does, wire changes (link to each doc in
   {CHANGE_DOCS} whose wire changes apply), the deviations, how it
   was tested, and this exact line on its own:
   - [x] I have read and agree to the [Syneroym CLA](https://github.com/syneroym/syneroym/blob/main/CLA.md).
   Add the PR link to status.md in a last commit and push. Do not
   mark ready, do not merge, do not review your own code.

If a step does not converge after a few attempts, stop and tell me.

Finish with: the PR link, commits per step, deviations, tests run
with pass counts, and anything left open.
```

## T5 — Code review, any round

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A3
TITLE: the batched outbound call (call-all)
BRANCH: feat/m08a-a3-call-all
PR: 338
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Claude Opus 5.5
AUTHOR: Gemini 3.8 Flash
ROUND: 1
FOCUS: none
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
REVIEW = {DIR}/slice-{slice}-review.md
CHANGE_DOCS is a list. Read every doc in it. Each decision and
deviation belongs to the change doc that defines the design it is
about.

Code review round {ROUND} of slice {SLICE} of {MILESTONE} ({TITLE}),
PR #{PR}.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym on
branch {BRANCH} (git pull first). If you are in a git worktree, stop
and tell me. I am {ME}. The code was written by {AUTHOR}.

Read first: AGENTS.md (all rules); session-strategy.md ("The review
file"); {PLAN}; {REVIEW}; the code review rounds in the other
slice-*-review.md files in {DIR}; each doc in {CHANGE_DOCS}.

Do not change code. Review `git diff origin/main...HEAD` against the
plan and the rules. Spend most of your time on what is wrong or
missing, not on wording.

CI: use `gh pr checks {PR}` as proof that the full suite passes. Do
not run the full suite or `mise run verify`. Run only small targeted
tests to prove a suspicion (`mise run build:roym` first if the test
needs the roym WASM; check it did not skip).

If {REVIEW} has earlier code review rounds: first check every item
marked Fixed or Rejected in the last round, against its fix commit. A
wrong or partial fix goes back to Open with a note. Then look at the
fix commits for new problems.

Check at least: the code does what the plan's decisions say, and
nothing more (compare changed function bodies with main: no
reordered checks, new errors or lost "why" comments); security and
access control; both builds where it applies; every wire change is in
the plan or in the Deviations of the doc in {CHANGE_DOCS} that owns
it; the decisions in each change doc match the plan's (same numbers,
same meaning) and sit in the right doc; tests exist and prove what
they claim, and status.md counts match what the tests print;
AGENTS.md rules (sizes, match arms, imports, comments, duplication,
deps, living docs without over-claims, backlog rows).

Extra points for this round: {FOCUS}

Write into {REVIEW} a new section "Code review round {ROUND} — {ME},
<today>": what was checked and found correct, counts (blocker / major
/ minor), then new findings with the next free R number, each with
severity, where (file:line at the reviewed sha), problem, failure
case, concrete fix, *Status:* Open. Never renumber.

Set the {SLICE} row in {DIR}/status.md to Step "Code reviewed", your
tool in the code-review column, open count in Evidence. Commit
(docs(planning): {SLICE} code review round {ROUND}) and push. Do not
mark the PR ready. Do not merge.

Finish with: the counts, each blocker and major in one line, and "0
open" if nothing is open.
```

## T6 — Code fix round

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A3
BRANCH: feat/m08a-a3-call-all
PR: 338
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Gemini 3.8 Flash
ROUND: 1
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
REVIEW = {DIR}/slice-{slice}-review.md
CHANGE_DOCS is a list. Each decision and deviation belongs to the
change doc that defines the design it is about. Write it there, and
only there. If unsure which doc owns something, ask me.

Fix the open code review items of slice {SLICE} of {MILESTONE} (PR
#{PR}).

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym on
branch {BRANCH} (git pull first). If you are in a git worktree, stop
and tell me. I am {ME}.

Read first: AGENTS.md (code quality, imports, comments, context
budget); session-strategy.md ("The review file"); {REVIEW} (every
item with *Status:* Open); {PLAN} and {CHANGE_DOCS} where an item
points at them.

1. For each open item: check it against the code yourself. If you
   agree, fix it as the item says (or more simply, same result). If
   you disagree, do not change code for it; set `Rejected: <reason>`.
   Change nothing an item does not ask for.
2. Blockers and majors first. Where an item fixes a test or a check,
   prove once that it now fails when it should (break it locally, see
   it fail, undo), and write the result in the item's status line.
3. Small commits grouped by topic. Conventional messages, no item ids
   in code, comments or test names.
4. Tests, only for what you changed: `mise run build:roym` if a
   roym_* crate changed; targeted `cargo nextest run -p <crates>`
   (check parity tests ran and did not skip); clippy -D warnings;
   nightly fmt; `cargo xtask check-file-lengths`,
   `check-duplication` and `check-change-docs` (update a ratchet only
   if a gate says it improved). No full `mise run verify`.
5. In {REVIEW} set each item to `Fixed (<sha>)`, `Rejected: <reason>`
   or `Deferred: <backlog link>`, tick fixed boxes, never renumber.
   {DIR}/status.md {SLICE} row: Step "Fixes done", short Evidence.
   Commit these last (docs(planning): close {SLICE} code review round
   {ROUND}) and push.

If a fix does not converge after a few attempts, stop and tell me.
Do not review your own fixes.

Finish with: a table of the items with Fixed or Rejected and the sha
or reason, the tests run with pass counts, and the proof results.
```

## T7 — Close a slice

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
SLICE: A3
BRANCH: feat/m08a-a3-call-all
PR: 338
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Claude Sonnet 5.5
=== Do not edit below this line ===
{NAME} in the text means the value above. {slice} is SLICE in lower
case.
DIR = docs/planning/milestones/{MILESTONE_DIR}
PLAN = {DIR}/slice-{slice}-implementation-plan.md
REVIEW = {DIR}/slice-{slice}-review.md
CHANGE_DOCS is a list. Read every doc in it. If task.md names a
change doc for this slice that is not in the list, read it too and
tell me. Each acceptance criterion, decision and deviation belongs to
the change doc that defines it. Write it there, and only there.

Close slice {SLICE} of {MILESTONE} (PR #{PR}). All review items are
closed. This step does not merge; I merge it myself.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym on
branch {BRANCH} (git pull first). If you are in a git worktree, stop
and tell me. I am {ME}.

Read first: AGENTS.md (all "Mandatory" items); session-strategy.md;
{PLAN} (sections "How this is shown to an end user", deferred
backlog, living docs, exit criteria, the docs and finish steps);
{REVIEW} (stop if any item is Open); {DIR}/task.md and status.md;
each doc in {CHANGE_DOCS}.

Do not change code unless a gate fails or a check finds a real gap;
then say why in the commit.

1. If origin/main has new commits, merge origin/main into the branch.
2. Close-out checks against {PLAN}: exit criteria (tick each with its
   proof; report any not met); living docs say what the slice built,
   with no over-claims and no change-doc or slice ids; deferred
   backlog rows and TODO/FIXME rows present, resolved rows moved to
   "Recently resolved"; import cleanup pass over
   `git diff --name-only origin/main...`; demo close-out (a story, or
   `- none: <reason>`) in the `demo:` list of every doc in
   {CHANGE_DOCS} the slice changed, and `living-docs-touched`
   complete in each; a short, true "{SLICE} result" line in
   {DIR}/task.md.
3. For each doc in {CHANGE_DOCS}: if this slice completes a group of
   slices or a set of its acceptance criteria, check those criteria
   against the code, write the proof in their rows, and check that
   its Decisions and Deviations read as one consistent record. If
   this slice ends that change, set `status: done` only if every
   criterion is met; else tell me.
4. Checks: find the last commit with a full green CI
   (`gh pr checks {PR}` and its head sha), then
   `git diff --stat <that sha>`. Only *.md files changed since: run
   `cargo xtask check-change-docs` and `cargo xtask
   check-file-lengths`; no full verify. Any code changed since: run
   the full `mise run verify` once in the background, no --skip,
   sandbox off; fix and rerun until every gate passes.
5. {DIR}/status.md {SLICE} row: Step "Merged" (I merge right after);
   keep the tool names; Evidence: PR, CI run link and sha (or "verify
   passed, <date>"), main test counts. Short.
6. Commit (docs(planning): close {MILESTONE} slice {SLICE}), push,
   and wait until `gh pr checks {PR}` shows every job passed.
7. Update the PR body: summary, wire changes, deviations, how it was
   tested, and this exact line on its own:
   - [x] I have read and agree to the [Syneroym CLA](https://github.com/syneroym/syneroym/blob/main/CLA.md).
   Then `gh pr ready {PR}`. Do not merge.

Finish with: the exit criteria table, the check or verify result, the
final sha, and anything you could not close.
```

## T8 — Close a milestone

```text
=== Fill in ===
MILESTONE: M08A
MILESTONE_DIR: M08A-open-foundation
NEXT_MILESTONE: M08B
CHANGE_DOCS: docs/planning/changes/roym-open-foundation/change.md
ME: Claude Opus 5.5
=== Do not edit below this line ===
{NAME} in the text means the value above.
DIR = docs/planning/milestones/{MILESTONE_DIR}
CHANGE_DOCS is a list. Read every doc in it.

Close milestone {MILESTONE}. Every slice is merged. This is the
close-out audit.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym.
  git switch main && git pull && git switch -c docs/{MILESTONE}-close
(branch name in lower case). If you are in a git worktree, stop and
tell me. I am {ME}.

Read first: AGENTS.md; session-strategy.md;
docs/planning/milestones/M08-overview.md; everything in {DIR} (task,
status, every plan and review file); each doc in {CHANGE_DOCS};
docs/README.md (which docs are living);
docs/planning/deferred-backlog.md;
docs/planning/traceability-matrix.md.

1. {DIR}/status.md: every slice row is "Merged" with a PR. Stop if
   not.
2. Find every change doc in docs/planning/changes/ that has a
   criterion, task or slice of {MILESTONE}, not only those in
   {CHANGE_DOCS}. Name any that the list missed.
3. For each of those change docs: check every criterion that
   {MILESTONE} owns against the code on main, with the test that
   proves it. List any not met. If the change ends with {MILESTONE},
   every criterion is met, Deviations is complete, `status: done`,
   and `living-docs-touched` and `demo:` are complete. Else say what
   is left and for which milestone.
4. Living docs (requirements, architecture, developer guide,
   traceability matrix, terminology): each statement touched by
   {MILESTONE} is true on main; nothing still describes the old
   behaviour; no change-doc or slice ids.
5. Deferred backlog: every deferral in {DIR}'s plans and review files
   is a row (or resolved); rows that target {MILESTONE} are done or
   moved to a named later milestone with a reason.
6. Demo: the stories under crates/substrate/tests/e2e/demo/ show what
   {MILESTONE} made visible.
7. M08-overview.md and {DIR}/task.md: mark {MILESTONE} done; note
   what moved to later milestones.
8. Do not change code. If you find a code gap, list it and add a
   backlog row; I decide whether it needs a fix slice.
9. Run `cargo xtask check-change-docs` and `cargo xtask
   check-file-lengths`. Commit (docs(planning): close milestone
   {MILESTONE}), push, open a PR whose body contains this exact line
   on its own:
   - [x] I have read and agree to the [Syneroym CLA](https://github.com/syneroym/syneroym/blob/main/CLA.md).
   Do not merge.

Finish with: the criteria table per change doc, gaps found, backlog
changes, and what {NEXT_MILESTONE} needs first.
```

## T9 — Write a change doc

Use it before M08C, M08E, M08F and M08G. It writes one doc, so it has `CHANGE_DOC`, not a list.

```text
=== Fill in ===
MILESTONE: M08C
MILESTONE_DIR: M08C-<name>
TITLE: <module name, for example Agreements>
BRANCH: docs/<module>-change-doc
CHANGE_DOC: docs/planning/changes/<module>/change.md
ME: Claude Opus 5.5
=== Do not edit below this line ===
{NAME} in the text means the value above.
DIR = docs/planning/milestones/{MILESTONE_DIR}

Write the change doc for {TITLE}, needed before milestone
{MILESTONE}.

Setup: work in the main checkout /Users/pari/gitSyneroym/syneroym.
  git switch main && git pull && git switch -c {BRANCH}
If you are in a git worktree, stop and tell me. I am {ME}.

Read first: AGENTS.md ("Writing for readers"; change doc rules);
session-strategy.md; docs/planning/milestones/M08-overview.md (which
change docs {MILESTONE} needs); {DIR}/task.md; the parent design
docs/planning/changes/roym-value-exchange/change.md; an existing
change doc for the layout
(docs/planning/changes/roym-open-foundation/change.md); every other
change doc that {MILESTONE}'s slices build part of; the living docs
it touches.

Write {CHANGE_DOC} with front matter (status: draft,
living-docs-touched, demo), a reader line, problem, scope and
non-goals, numbered acceptance criteria, design (checked against the
code on main, with file:line links), open questions as numbered
owner questions with options and a recommendation, tasks mapped to
the slices of {MILESTONE}, and empty Decisions and Deviations
sections. Say plainly where it depends on or overlaps another change
doc, and which doc owns each shared criterion. Do not decide what the
parent design leaves open; ask.

Commit (docs(planning): add {TITLE} change doc) and push. No PR yet.
It is reviewed with T2, with the change doc in place of the plan.

Finish with: the owner questions with recommendations, and the
biggest risks.
```

# Rewriting a comment that cites a planning document

`AGENTS.md` bans code comments that name a milestone, slice, task, design id,
review finding, test number, or planning-doc section. Those documents get
archived and renumbered, so the comment stops meaning anything. ADR references
are fine, because ADRs are permanent.

There are about 1,222 comment blocks to fix. This page says how, so that every
session writes them the same way.

## The one rule

**Drop the pointer. Keep the claim.**

The citation is a pointer to where a decision was written down. The prose around
it usually explains a real constraint, and that part must survive. The danger in
this work is not breaking the build — comments cannot do that. The danger is
quietly deleting a reason nobody can recover afterwards.

If you cannot tell what a comment is claiming, do not guess. Read the code it
sits on, and write what is actually true. If it is still unclear, leave the
comment alone and note the file in the pull request so a human can look.

## What counts as a citation

| Family | Looks like |
| --- | --- |
| Milestone | `M04A`, `M05A A5a` |
| Slice or task | `Slice B7a`, `A5e's resident loop` |
| Design id | `D-B1-9`, `D-06C-3` |
| Section number | `§11.2`, `A7 §0.5` — but **not** `ADR-0022 §7` |
| Review finding | `M05B B1 review finding 4`, `review round 2` |
| Test or matrix row | `Test 97`, `failure-matrix row 13` |
| Planning doc | `` `status.md` ``, `` `task.md` ``, `implementation-plan` |

A section number anchored to an ADR is allowed and must be left alone. About
325 comment lines carry one.

## The four shapes, and what to do

### 1. The citation is the whole parenthetical — delete it

```rust
// before
/// scan of every payload (M05B B1 review finding 15).
// after
/// scan of every payload.
```

Watch for a sentence that now reads oddly. `alert groups by (D-B1-6). Opaque
to ...` becomes `alert groups by. Opaque to ...`, which is broken. Fix the
sentence, do not just delete the characters.

### 2. The parenthetical holds a citation *and* real content — keep the content

```rust
// before
/// same reasoning (D-A1-2: resolution requires a configured HTTP registry,
/// the DHT copy is best-effort backup, checked second by `lookup`). Bites
// after
/// same reasoning: resolution requires a configured HTTP registry, and the
/// DHT copy is best-effort backup, checked second by `lookup`. Bites
```

### 3. The citation is the subject of the sentence — name the thing instead

```rust
// before
// A5e's resident loop reclassifies a member as a push candidate on
// after
// The resident loop reclassifies a member as a push candidate on
```

```rust
// before
/// D-C-2's own local pin: a locked vault stops the signer before any
/// writer would be reached
// after
/// A locked vault stops the signer before any writer would be reached
```

A test whose doc comment is only a citation needs a real one. Say what the test
proves and why it matters:

```rust
// before
/// Test 77: the mapping S2's post-merge finding 12 established for
/// `NoSuchService`, extended to `AmbiguousHash`.
// after
/// An unknown service hash is a caller mistake, so it maps to
/// `NoSuchService` (invalid params) rather than an internal error.
```

### 4. The claim is now false — this is the important one

Some of these comments do not merely point at a dead document. They state
something that has since stopped being true. These are worth finding on their
own, because they actively mislead.

```rust
// before -- claims the loop does not exist
//! `supervisor` WIT interface. This slice (M05A A5b) is the role, the
//! store, the interface, and master custody -- no autonomy: `status` sweeps
//! on demand, and the resident reconcile loop is a later slice.

// after -- the loop has been running in service.rs for months
//! `supervisor` WIT interface. It also holds custody of each managed
//! instance's master key, and runs a resident loop that reconciles the
//! substrates it manages against that desired state.
```

Whenever a comment describes what a slice "does not do yet", check the code
before rewriting. If the thing now exists, the comment is a bug.

## Writing style

Follow the communication rules in `AGENTS.md`. The reader has solid computer
science fundamentals but simple English.

- Explain **why**, not what. The code already says what.
- Short sentences. One idea per sentence.
- Keep the invariant, the constraint, or the trade-off. Drop the history.
- Do not replace a planning reference with a git or pull-request link. That
  rots the same way.
- Never write "as decided in review" or "per the plan". State the decision.

## Checking your work

```bash
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo nextest run -p <the crate you touched>
```

Comment-only changes cannot alter behaviour, so a failing test means you edited
code by accident.

To confirm a crate is clean, run the search from
[README.md](README.md#finding-planning-references) against it and expect zero.

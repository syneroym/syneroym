# Code quality baseline

This folder holds measured data about the health of the codebase. It exists so
that a cleanup round can prove it worked: take a baseline, do the work, run the
same commands again, compare the numbers.

The current baseline is [`baseline-2026-09-08/`](baseline-2026-09-08/), measured
on commit `8d0444a` (main, right after slice C7 merged).

## What is in a baseline folder

| File | What it holds |
| --- | --- |
| `summary.json` | The headline numbers. Read this first. |
| `ranking.json` | Every source file with 300+ production lines, with its size, lint count, long-function count, age, and commits per month. |
| `long-functions.json` | Every function over 100 lines, longest first, as `[lines, file, line_number]`. |
| `duplication.json` | Duplicate code groups, ranked by wasted lines. Top 120 whole functions and top 60 sub-function blocks. |
| `clippy-lints.json` | Count of every clippy lint. Full findings kept only for structural lints. |
| `planning-refs.json` | Every code comment that cites a milestone or slice ID. `AGENTS.md` forbids these. |
| `docs-readability.json` | Average sentence length, share of long sentences, and jargon count per markdown file. |

## Tools

None of these are in `mise.toml` yet. Install them by hand:

```bash
cargo install cargo-dupes tokei cargo-shear
```

`cargo-dupes` parses Rust into a syntax tree, then replaces identifier names and
erases literal values before hashing. That is why it finds blocks that are the
same shape but use different variable names and different config values. A
token-based tool misses those.

Do not try to install `rust-code-analysis-cli`. It is unmaintained and no longer
builds. Clippy's own `too_many_lines` covers the same ground.

## Two traps to avoid

**1. Generated files inflate duplication.** `wit-bindgen` writes `bindings.rs`
files into `crates/roym_*/src/` and `test-components/*/src/`. They are
gitignored and marked `DO NOT EDIT`, but they only exist after you build, and
`cargo-dupes` does not read `.gitignore`. Counting them reports about 49%
duplication instead of the real 11%. Always pass `--exclude 'bindings.rs'`.

For the same reason, count files with `git ls-files`, not by walking the
directory tree. That way the numbers do not change depending on what you have
built locally.

**2. Git churn does not mean what it usually means here.** The normal way to
rank refactoring work is change frequency times complexity: a file that is both
complex and edited often is worth fixing first. That does not work on this
repository yet, because every file is young. The oldest source file is about
five months old and the `roym_*` crates are days old, so a low commit count
means "written recently", not "stable and safe to ignore".

Ranking by raw commit count wrongly pushed `app_supervisor/src/service.rs` (the
largest source file, and one of the fastest changing per month) down the list,
and pushed the single worst function in the codebase down with it.

So: rank by size and complexity. Use commits per month since the file was
created only to break ties. Ignore change frequency completely for the `roym_*`
crates. Revisit this once the codebase is a year or so old.

## How to re-run

From the repository root, on a clean checkout:

```bash
cargo dupes report --format json --exclude-tests --exclude 'bindings.rs' --exclude 'target' > dupes-prod.json
```

```bash
cargo dupes report --format json -s --exclude 'bindings.rs' --exclude 'target' > dupes-all-sub.json
```

```bash
cargo clippy --workspace --all-targets --all-features --message-format=json -- -W clippy::pedantic -W clippy::nursery -W clippy::cargo > clippy.json
```

```bash
cargo shear
```

`cargo-dupes` writes five JSON documents one after another into a single file:
stats, exact groups, near groups, sub-function exact groups, sub-function near
groups. A normal JSON parser stops after the first one. Read them in a loop with
`json.JSONDecoder().raw_decode`.

The clippy run needs the sandbox off, because `cargo install` and the registry
write to `~/.cargo`.

## Guarding the gains

Once a cleanup step lands, lock it in so the problem cannot grow back:

- Add a `clippy.toml` with `too-many-lines-threshold` set just under the current
  worst function, and lower it after each batch.
- Add `cargo dupes check --max-exact-percent` set just under the current
  duplication percentage.
- Add a grep in CI for the milestone-reference pattern, once the count reaches
  zero.

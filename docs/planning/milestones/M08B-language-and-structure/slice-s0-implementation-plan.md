# M08B slice S0 — Checks and owner decisions: implementation plan

*Reader: the developer (or AI) who implements slice S0. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-05. Plan review round 1 folded in. The
owner's answers to Q1 to Q7 are recorded under each question (section
2.1).**

**Size and PR.** S. One PR on `feat/m08b-s0-checks-decisions`. The branch
already holds the plan commits (the plan, the owner answers, the review and
its fixes). The commits after them: the spelling test set, and the results
(written into the docs).

**Base.** `main` at `0e016d8c` (2026-11-05). The branch also holds the
plan commits. Every file path and line number below was checked on that
commit. The commits after `76b52fe6`
(the commit that task.md section 2 was checked on) change only planning
docs, so task.md's code facts still hold.

**Docs only.** S0 commits no code, no `Cargo.toml` change and no
`package.json` change. Every check runs in a throwaway folder outside the
repository (section 4). The only new file in the repository is
`spelling-pairs.tsv`, a data file under `docs/`.

**Parallel work.** None. Every other M08B slice comes after S0
([task.md](task.md) section 4).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most: `mise run verify` before
  you finish (background job, no `--skip`), simple English in docs, the
  deferred backlog and the living-docs close-out, no planning references in
  code (S0 has no code, so this applies only if a later slice copies text
  from here).
- [task.md](task.md): section 1 (shared parts Y1 to Y6, and where Y3 and
  Y4 live), section 2 (facts already checked), S0, B3a, B4a, B4b, and
  section 6 (owner questions 1 to 8, all answered except 8).
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  "Which catalog format", "C. Language-tagged free text" (tag
  validation), "Search", "Rules for every layer" (rule 4), "Open
  questions", "Deviations".
- task.md names one more change doc,
  [demo-stories](../../changes/demo-stories/change.md), in its "Demo"
  paragraph. It is about the Hub demo steps. S0 changes no Hub text and no
  story, so S0 does not touch it.

### Terms used in this plan

- **Language tag:** a code for a language in BCP 47 form, such as `hi`,
  `ta` or `hi-Latn`. See the change doc's term list.
- **Canonical form (of a tag):** the one spelling the node stores for a
  tag. Two levels exist. *Syntax form:* fixed letter case and order
  (`HI-latn` becomes `hi-Latn`). *CLDR form:* the syntax form plus
  replacement of old codes by new ones (`iw` becomes `he`). CLDR is the
  Unicode project's locale database.
- **ICU4X:** the Unicode project's Rust libraries for languages and text.
  `icu_locale_core` (tags) and `icu_normalizer` (NFC) are two of them.
- **NFC:** Unicode Normalization Form C. Two texts that look the same but
  use different code point sequences become the same sequence.
- **Search key, transliteration, script:** see the change doc's term list.
- **ISO 15919:** a standard way to write Indic scripts in Latin letters.
  It marks long vowels and other sounds with accents (`ā`, `ṭ`).
- **Schwa deletion:** in Hindi, a consonant letter carries a short "a"
  sound that people often do not say. `नल` is `nala` letter by letter, but
  people say and type `nal`.
- **Fluent:** Mozilla's message format for UI text (`@fluent/bundle` is its
  JavaScript library). The owner chose it (task.md question 2).
- **Scratch folder:** `$TMPDIR/m08b-s0/`. All check code lives there and is
  deleted after S0. Nothing in it is committed.

## 1. What S0 does, in one paragraph

S0 answers four technical questions before any M08B code is written, and
drafts one test data file. The questions: does `icu_locale_core` build for
`wasm32-wasip2` with only its `alloc` feature, what canonical form does it
give, and where does the browser's `Intl.Locale` disagree with it; which crate gives NFC (added by Q3 below); is there a pure-Rust
transliteration crate for Devanagari and Tamil that M08B should use, or
does M08B write its own fixed tables; and how many bytes does
`@fluent/bundle` add to the Hub. The data file is `spelling-pairs.tsv`:
200 Hindi and 200 Tamil words, each in its own script and in one common
Latin spelling, plus other spellings that must match and some that must
not. S0 writes each result into task.md and into the change doc, in place
of the `unverified` notes.

## How this is shown to an end user

- Nothing new. An end user sees no change from S0. It is a set of checks,
  written decisions and a test data file.
- The first visible step is B1a: a person switches the Hub to Hindi, and
  the Hub stays in Hindi after a reload (demo story
  `demo/40-roym-languages.spec.ts`, task.md "Demo").

How a developer can see it work:

- Open [task.md](task.md), slice S0: each bullet has a "Result:" line with
  the number or the decision.
- Open `spelling-pairs.tsv` in this folder. Its header lines say who wrote
  it and whether the owner has checked it.
- Repeat any check with the commands in section 4. Each step gives the
  exact command and what it prints.

## 2. Points for the owner

### 2.1 Questions (answered; see "Owner answer" under each)

task.md section 6 has the owner's questions 1 to 8 for the whole
sub-milestone. The questions below are new, for S0 only. Each names the
step it blocks.

**Q1. Transliteration: a crate, or our own tables? (blocks Step 3's
decision, not the survey)**

Search keys need only one direction: from Devanagari or Tamil to Latin
letters. Nothing in M08B turns Latin letters back into a native script
(task.md Y4; change doc "Search").

Options:

- (a) Our own fixed tables in `syneroym-text-lang`, one per script,
  starting from ISO 15919, with our own folding rules on top. Each table
  covers one Unicode block of 128 code points: Devanagari (U+0900 to
  U+097F) and Tamil (U+0B80 to U+0BFF, fewer of them assigned).
- (b) A crate for the script-to-Latin step, with our own folding rules on
  top.

**Owner answer: (a), 2026-11-01.** The owner also agrees with the plan
review: the second reason below is the weak one, because a crate can be
pinned to an exact version with a `# patch-pin:` reason (AGENTS.md). The
first and third reasons carry the choice. The survey also lists
`icu_experimental` and `lipilekhika` (Step 3).

*Recommendation:* (a), unless the survey finds a crate that meets every
rule in Step 3 **and** gives output that our folding cannot easily get
from (a). Reasons:

- The folding rules are ours in both options. They must handle schwa
  deletion, Tamil letters that stand for two sounds (`க` is both `k` and
  `g`), and nasal signs. No general transliteration crate does this for
  search, because it is lossy on purpose.
- Every stored search key depends on the exact output. If a crate upgrade
  changes one letter of output, old keys no longer meet new queries, and
  every index must be rebuilt. With our own tables, output changes only
  when we change the table, in a reviewed PR with its tests.
- The tables are small data. A crate adds a dependency for a few hundred
  lines of data.

S0 still does the survey (Step 3) and records it, so the decision has
evidence.

**Q2. Canonical form of a language tag. (blocks Step 2's result text)**

`icu_locale_core` with no data gives the syntax form (`hi-latn` becomes
`hi-Latn`). The CLDR form (`iw` becomes `he`, `hi-Deva` may become `hi`)
needs the `icu_locale` crate and its CLDR data, which is much larger.
`unverified`: the exact outputs; Step 2 records them.

Options: (a) syntax form only; (b) CLDR form.

**Owner answer: (a), 2026-11-01.** The owner notes that review item P2
weakens the premise below. A record from another node can carry any tag.
And the browser's `Intl.Locale` turns `iw` into `he`, while
`icu_locale_core` keeps `iw`. So Step 2 also runs the same inputs through
`Intl.Locale`, and D-S0-1 gets two rules for the Hub (Step 6).

*Recommendation:* (a). The Hub offers a fixed list of tags (from its
catalogs and a language picker), so people on this node do not type old
codes. Tags on records from other nodes can still be anything: Step 2
records where the two parsers differ. A record with `iw` stays readable.
The size cost of (b) would land in every Roym guest,
because `roym_core` validates tags (task.md B3a). With (a), S0 adds a
backlog row "Language tags are not CLDR-canonicalised" (target TBD).

**Q3. Add an NFC check to S0? (blocks Step 2b)**

task.md's S0 does not list it. But B4a computes NFC in `data_db`, and two
crates can do it. `icu_normalizer` 2.4.0 is already in `Cargo.lock`
([Cargo.lock:3171](../../../../Cargo.lock#L3171)), pulled in through
`url` and `idna` in the host crates (for example `syneroym-data-db` and
`syneroym-conversation`). `unicode-normalization` is not in `Cargo.lock`.

**Owner answer: yes, prefer `icu_normalizer`, 2026-11-01.** Step 2b also
measures what a guest pays when `syneroym-text-lang` holds both
dependencies but the guest uses only the tag type.

*Recommendation:* yes. It costs one more build in the same scratch
harness. Prefer `icu_normalizer`, because it is already in the host build
and comes from the same project as `icu_locale_core`. Choose
`unicode-normalization` only if `icu_normalizer` fails a check.

**Q4. The spelling test set: format and content. (blocks Step 5)**

task.md asks for 200 word pairs per language, plus spelling variants that
must match each other. Section 4, Step 5 gives the exact format.
Two parts go beyond task.md and need a yes:

- **`joined` rows:** the same words written with no space (`nalwala`
  for `nal wala`). ML-22 names this case. They are kept apart from other
  variants because they need a different fix in search (see section 2.2,
  last point).
- **`distinct` rows:** spellings that must **not** match a word. Without
  them, a folding rule could merge too much (for example, if it merged
  `p`, `t` and `k`, every test would still pass). At least 20 per
  language, each one guarding one folding rule.

**Owner answer: yes to both, 2026-11-01, with two changes from the plan
review (P3, P11).** `joined` rows exist only for groups of more than one
word. And the check script reports conflicts between the `variant` rows
of one group and the `distinct` rows of another, across both languages
(Step 5).

*Recommendation:* yes to both. And the file starts with
`# owner check: pending`. S0 merges with this line. The owner changes it
to `# owner check: done <date>` when the check is done, before the B4b
plan is written (task.md question 6).

**Q5. When is `@fluent/bundle` too large? (blocks Step 4's decision)**

Today the Hub JavaScript is 114,463 bytes, 31,769 bytes after `gzip -9`
(the `dist/` build on this machine, 2026-11-05). The hard limit is far
away: an asset archive may be up to 2 MiB compressed
(`MAX_ASSET_BUNDLE_BYTES`,
[deploy_docs.rs:146](../../../../crates/core/src/deploy_docs.rs#L146)),
and the packed Hub is 34,257 bytes today. The real cost is the first load
on a slow phone.

**Owner answer: yes, 25 KB after `gzip -9` and no `eval`, 2026-11-01.**
The 25 KB covers the library only. The two catalogs add more (Hindi text
is 3 bytes per character before gzip). So B1a reports the total (library
and catalogs) as well; Step 6 writes this into task.md's B1a bullet.

*Recommendation:* Fluent stays chosen if it adds at most 25 KB after
`gzip -9`. If it adds more, S0 stops and asks the owner, with the number.
S0 also checks that the built code has no `eval`, no `Function(...)` and
no `new Function`, because the Hub's Content Security Policy is
`script-src 'self'`
([index.html:16](../../../../crates/roym_web/ui/index.html#L16)). If it
has either, S0 stops and asks, whatever the size.

**Q6. Record versions stay at 1. (blocks Step 6)**

The change doc's rule 4 says that M08B keeps `version` at 1 for every
record type it changes, and marks it `unverified` ("confirm the team's
rule"). The project's position is written down: no migrations before
release
([ADR-0018 §5](../../../decisions/0018-service-record-visibility.md),
"No migration"). That ADR is about endpoint-record visibility, so it is
the project-wide position, not a rule about `version`. Every record type
is at version 1 today
([record.rs:18](../../../../crates/roym_core/src/record.rs#L18)).

**Owner answer: yes, 2026-11-01.** Cite ADR-0018 §5 as the project-wide
"no migration" position, not as a rule about `version`.

*Recommendation:* S0 removes the `unverified` note and cites ADR-0018 §5
as the project-wide "no migration" position. No other change.

**Q7. Old facts in the change doc. (blocks Step 6)**

The change doc's "Problem" and "Design" sections cite code that has moved
or counts that are now known to be wrong (section 2.2 lists them).

Options: (a) fix them in place; (b) fix them and add a `## Deviations`
entry for each.

**Owner answer: (a), 2026-11-01.** The same edit also fixes the font
sentence (the last row of the table in section 2.2) and keeps the
font-size `unverified` note (exit criterion 4).

*Recommendation:* (a). `## Deviations` records where the code differs from
the design. A wrong line number about today's code is not a design
difference. The design changes that S0 may cause (a crate in place of
tables, Q1) do get a Deviations entry.

### 2.2 Points found while reading

- **The FTS5 `unverified` note is still in the change doc.** task.md
  section 2 records that FTS5 and the trigram tokenizer work in this build
  (the chat store already creates a trigram table,
  [schema.rs:115](../../../../crates/conversation/src/store/schema.rs#L115)).
  The change doc's "Search" section still says `unverified`. Step 6
  replaces it with the result. That result covers the trigram table only.
  Y5 also needs a second table with the `unicode61` tokenizer and
  `prefix='1 2'` (ML-29), and nothing in `crates/` uses either today
  (`grep -rn 'unicode61\|prefix=' crates` finds nothing). S0 does not check
  it. Step 6 says so, and makes it the first check of B4a.
- **The change doc still puts the transliteration table in `roym_core`**
  ("Search", "The transliteration table is a fixed table in `roym_core`").
  The owner moved it to `syneroym-text-lang` (task.md section 1; change
  doc `## Deviations`). Step 6 fixes the "Search" and "Tag validation"
  text to name the new crate.
- **Old facts in the change doc (Q7):**

  | Change doc says | On `main` |
  | --- | --- |
  | `profile.set` stores `locale` at `profile_ops.rs:99` | [profile_ops.rs:140](../../../../crates/roym_profile/src/app/profile_ops.rs#L140) |
  | categories at `listing.rs:185` | [listing.rs:188](../../../../crates/roym_core/src/listing.rs#L188) |
  | `normalize_text` at `directory.rs:232` | [directory.rs:225](../../../../crates/roym_core/src/directory.rs#L225) |
  | the `-32602` refusal at `search_ops.rs:256` | [search_ops.rs:265](../../../../crates/roym_directory/src/app/search_ops.rs#L265) |
  | dates at `screens/listings.ts:468` | [listings.ts:487](../../../../crates/roym_web/ui/src/screens/listings.ts#L487) |
  | `ReceiptHalf` at `transaction.rs:205` | [transaction.rs:220](../../../../crates/roym_core/src/transaction.rs#L220) |
  | "37 Hub source files", "about 104 `textContent` assignments" | 41 files and about 420 places (task.md section 2) |
  | "About 25 places in the Hub read `.message`" | 22 |
  | a font loaded on demand from the node "is allowed" (change.md line 358) | blocked: the CSP has `default-src 'none'` and no `font-src` ([index.html:16](../../../../crates/roym_web/ui/index.html#L16)) |

- **`icu_locale_core` is not new to the build, but it is new to the
  guests.** `icu_locale_core` 2.4.0 is in `Cargo.lock`
  ([Cargo.lock:3158](../../../../Cargo.lock#L3158)) through `idna`, in the
  host crates (`cargo tree -p syneroym-data-db --target wasm32-wasip2 -i
  icu_locale_core` shows it). `syneroym-roym-core` and the Roym service
  crates do not have it today. Its only normal dependencies are `serde`,
  `serde_json`, `serde_path_to_error`, `thiserror`, `syneroym-app-host`
  and `syneroym-signed-record`. So the `wasm32-wasip2` check is a real
  check, and its size result is what every Roym guest pays.
- **Where Y4 runs.** By task.md section 1, `data_db` and
  `crates/conversation` compute search keys in the host. No Roym guest
  calls the transliteration code. But `syneroym-text-lang` is one crate,
  and `roym_core` (a guest crate) uses it for Y3. So the transliteration
  code must also build for `wasm32-wasip2`, even if guests never call it.
  The release profile uses `lto = true` and `opt-level = "s"`
  ([Cargo.toml:42](../../../../Cargo.toml#L42)), so code a guest never
  calls is removed from its `.wasm`. B3a's plan decides whether a cargo
  feature is still needed. S0 records the build result, and Step 2b
  measures the guest cost of both dependencies, so B3a decides from a
  number.
- **Joined words and the trigram index (for the B4a and B4b plans).** A
  trigram match finds the query as a substring of the key. `nalwala` is
  not a substring of `nal wala`. So ML-22's `nalwala` case cannot pass
  with the same key in both tables. One way: the trigram table indexes the
  key with spaces removed, and the `unicode61` table indexes the key with
  spaces. That is a change to the key function, not to the table shape,
  so task.md's claim "B4a's tables need no change for B4b" can still
  hold. S0 does not decide this. Step 6 writes one sentence about it under
  task.md's B4b bullet, so the B4a and B4b plans see it.
- **Fluent and the CSP.** The Hub has two CSP tags, and the browser
  applies both. [index.html:16](../../../../crates/roym_web/ui/index.html#L16)
  sets `default-src 'none'`, `script-src 'self'` and a wide `connect-src`.
  [boot-csp.ts](../../../../crates/roym_web/ui/src/boot-csp.ts) adds a
  second tag at run time ([csp.ts](../../../../crates/roym_web/ui/src/csp.ts))
  that narrows `connect-src` to `'self'` and the auth origin. If B1a loads
  `.ftl` catalog files with `fetch`, both tags allow it (`'self'`). If B1a
  imports them as text with Vite's `?raw`, they are inside the JavaScript.
  Either works. B1a decides. S0 only checks `eval` (Q5). There is no
  `font-src`, so `default-src 'none'` blocks every web font, even from the
  node itself. Step 6 fixes the change doc's font sentence (Q7).

## 3. What S0 meets, and what it decides

**Criteria.** S0 meets no `ML-*` criterion. It gives inputs:

| Result | Used by | For criteria |
| --- | --- | --- |
| `icu_locale_core` build and canonical form | B3a (Y3) | ML-10, ML-13, ML-14 |
| NFC crate | B4a (Y5) | storage half of ML-20, ML-29 |
| Transliteration decision | B4b (Y4) | ML-20, ML-21, ML-22, ML-23 |
| `spelling-pairs.tsv` | B4b tests | ML-21, ML-22, ML-29 |
| `@fluent/bundle` size and CSP check | B1a (Y1) | ML-1 to ML-3 |

**Decisions S0 records.** Where each is written (Step 6 does it):

| Decision | Change doc | task.md | Backlog |
| --- | --- | --- | --- |
| D-S0-1 tag crate, its features, canonical form, and the two Hub rules (Q2) | "C. Language-tagged free text", tag validation paragraph | S0 result line; B3a bullet | new row, §10, if Q2 (a) |
| D-S0-2 NFC crate (Q3) | "Search", search key bullet | S0 result line; B4a bullet | — |
| D-S0-3 transliteration: tables or crate (Q1) | "Search", transliteration bullet; `## Deviations` only if a crate is chosen | S0 result line; B4b bullet | — |
| D-S0-4 Fluent stays (Q5) | "Which catalog format": the size line | S0 result line; B1a bullet (report the total with catalogs) | — |
| D-S0-5 record versions stay 1 (Q6) | "Rules for every layer", rule 4 | — | — |

The change doc has no `## Decisions` section. S0 does not add one. Each
decision goes into the design text it is about, as the owner's earlier
decisions do ("Decided 2026-11-05: ..." in "Open questions").

## 4. Behaviour-change audit

- No code changes. No wire format, WIT, method, record, error code,
  config, CLI or Hub change.
- No dependency changes: `Cargo.toml`, `Cargo.lock`, `package.json` and
  `package-lock.json` stay the same. The checks add dependencies only in
  the scratch folder.
- The repository gets one new data file (`spelling-pairs.tsv`) and doc
  edits. Nothing reads the TSV file until B4b moves it into the test data
  of `syneroym-text-lang`.
- `mise run verify` treats the change as docs only: every path is under
  `docs/` ([verify.rs:186](../../../../xtask/src/verify.rs#L186)), so
  the `nextest`, `doctests` and `e2e` gates skip. The other gates still
  run.

## 5. Step-by-step changes

Each step leaves the branch in a state where `mise run verify` passes,
because every step changes only docs.

**Network.** Steps 2 to 4 download crates and npm packages. In the
sandbox, give the commands `allowed_domains` for `index.crates.io`,
`static.crates.io`, `crates.io` and `registry.npmjs.org`. If a download is
refused, stop and tell the owner. Do not copy packages in by another way.
If a build fails with a write error (not a network error), set
`CARGO_HOME` and `npm_config_cache` to folders under `$TMPDIR/m08b-s0/`,
or ask the owner to allow the command outside the sandbox. The cached
crate sources are then under that `$CARGO_HOME/registry/src`.

### Step 1 — Commit this plan (done)

- Done: `4ba8325c` (the plan and the S0 row in [status.md](status.md))
  and `b2bf4e1c` (the owner's first answers). The plan review and its
  fixes are also committed. Do not run this step again.
- The owner answered section 2.1. Step 2 can start.

### Step 2 — Language tags: `icu_locale_core` for `wasm32-wasip2`

**Harness.** Make `$TMPDIR/m08b-s0/tag-check/`, a binary crate:

```toml
[package]
name = "tag-check"
version = "0.0.0"
edition = "2024"

[dependencies]
icu_locale_core = { version = "2.4", default-features = false, features = ["alloc"], optional = true }

[features]
icu = ["dep:icu_locale_core"]

# Same as the workspace release profile (Cargo.toml:42).
[profile.release]
codegen-units = 1
lto = true
opt-level = "s"
panic = "abort"
strip = true
```

`src/main.rs`: for each command-line argument, print one line
`<input>\t<result>`. With the `icu` feature, the result is
`Locale::try_from_str(arg)` printed with `to_string()`, or `ERR <error>`.
Without the feature, the result is the argument in lower case (a
baseline of about the same code size, so the size difference is the
crate). Check the exact constructor name in the cached crate source, not
on docs.rs. In 2.4.0, `Locale::try_from_str`, `try_from_utf8` and
`normalize` all need the `alloc` feature
(`icu_locale_core-2.4.0/src/locale.rs:154`, `:162`, `:205`), and the
crate has no parse API without it. Its default features are empty. Do
not add `serde`, `zerovec` or `databake`.

**Commands and what to record:**

```bash
cd "$TMPDIR/m08b-s0/tag-check"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/tag-check.wasm base.wasm
cargo build --release --target wasm32-wasip2 --features icu
ls -l base.wasm target/wasm32-wasip2/release/tag-check.wasm
cargo tree --target wasm32-wasip2 --features icu -e normal
wasmtime run target/wasm32-wasip2/release/tag-check.wasm \
  hi HI hi-latn hi-Latn-IN ta-IN en-us hi_IN iw und hi-Deva \
  zh-hant-tw en-u-nu-deva x-private toolongsubtag ""
```

The `wasmtime` CLI is installed on this machine (`~/.wasmtime/bin`).

Then run the same 15 inputs through the browser's parser, with Node
(Node 22 uses the same `Intl.Locale` as current browsers):

```bash
node -e 'for (const t of process.argv.slice(1)) { try { console.log(t + "\t" + new Intl.Locale(t).toString()) } catch (e) { console.log(t + "\tERR " + e.name) } }' \
  hi HI hi-latn hi-Latn-IN ta-IN en-us hi_IN iw und hi-Deva \
  zh-hant-tw en-u-nu-deva x-private toolongsubtag ""
```

Record:

1. Builds: yes or no, with the `icu_locale_core` version, the exact
   feature list that B3a must write in `Cargo.toml` (expected:
   `default-features = false, features = ["alloc"]`), and the `cargo tree`
   list (no C code, no `build.rs` that needs a C compiler).
2. Size: the two `.wasm` sizes and the difference, in bytes.
3. The output table for the 15 inputs, with two result columns: Rust
   (inside `wasmtime`) and `Intl.Locale` (Node). Then list every input
   where the two differ: one accepts and the other refuses, or the
   spelling differs.
4. Whether `hi_IN` (underscore) is accepted. The crate's docs say the old
   `_` separator is refused since 2.0 (`locale.rs:142`), so expect `ERR`.
   Record what the run shows. B3a must know whether to refuse it.
5. Whether `iw` and `hi-Deva` stay as given in Rust (they should, with no
   data). This confirms Q2's premise for the Rust side. For `Intl.Locale`,
   expect `iw` to become `he`.

**Pass rule:** builds for `wasm32-wasip2` with only the `alloc` feature
(no `serde`, `zerovec` or `databake`), runs in `wasmtime`, and gives the
syntax form for valid tags. A difference from `Intl.Locale` does not fail
the step; it goes into D-S0-1 (Step 6). If it fails, stop and ask the owner
(no fallback is planned; a fallback would be our own small BCP 47 parser,
which is a design change).

### Step 2b — NFC crate (only if Q3 is yes)

Same harness, a second binary crate `$TMPDIR/m08b-s0/nfc-check/` with two
features, `icu` (`icu_normalizer = { version = "2.4", default-features =
false, features = ["compiled_data"] }`) and `un`
(`unicode-normalization`, latest stable). Check the exact feature names
in the cached crate source. For each argument, print the NFC form's code
points as `U+XXXX` values.

Inputs (each written as escapes in a small shell `printf`, so the bytes
are exact):

| Input | Why |
| --- | --- |
| `U+0958` (क़, one code point) | Composition exclusion: NFC gives `U+0915 U+093C`. Both crates must agree. |
| `U+0915 U+093C` | Must stay `U+0915 U+093C`. |
| `U+0B95 U+0BC6 U+0BBE` (கொ in parts) | NFC gives `U+0B95 U+0BCA`. |
| `U+0928 U+0932 U+0020 U+0935 U+093E U+0932 U+093E` (नल वाला) | Must not change. |
| `U+00E9` and `U+0065 U+0301` (é) | Latin case: both give `U+00E9`. |

Record: builds for `wasm32-wasip2` (yes or no), `.wasm` size of each, and
the output table. Also run it natively (`cargo run --features icu`),
because B4a uses NFC in the host. **Pass rule:** `icu_normalizer` builds
and gives the expected output. Then D-S0-2 is `icu_normalizer`.

**Guest cost of both dependencies.** `syneroym-text-lang` may hold both
the tag type and NFC, while a guest uses only the tag type. To measure
that case, add the chosen NFC crate to `tag-check` as a second
dependency under the `icu` feature, with no code that calls it. Build
for `wasm32-wasip2` as in Step 2 and record the `.wasm` size next to the
Step 2 size. B3a uses the two numbers to decide on a cargo feature.

### Step 3 — Transliteration crate survey

**Find candidates.** Search crates.io for: `transliteration`,
`transliterate`, `devanagari`, `tamil`, `indic`, `iso 15919`,
`romanization`, `unidecode`. Also check by name: `icu_experimental` (the
ICU4X crate with a `transliterate` module; same project as the tag and
NFC crates), `lipilekhika`, `vidyut-lipi`, `deunicode`, `any_ascii`
(`unverified`: these names exist and do what their names suggest). Keep
`icu_experimental` in the table whatever its downloads. Keep at most 6
others, the best by downloads and by recent releases.

**Rules a crate must meet** (all of them):

1. Pure Rust: no C code, no `build.rs` that needs a C compiler.
2. Builds for `wasm32-wasip2` (same harness as Step 2, a third crate
   `$TMPDIR/m08b-s0/translit-check/`).
3. License is on the `deny.toml` allow list
   ([deny.toml:93](../../../../deny.toml#L93)).
4. Covers Devanagari to Latin **and** Tamil to Latin.
5. A release in the last 12 months, or a clear statement that it is
   finished and kept.
6. Its output for the same input is fixed for a fixed version (no data
   downloaded at run time).
7. Stable output over time: the API is not marked unstable, or the
   version can be pinned exactly with a `# patch-pin:` reason. And its
   output for the sample words does not change in the last three
   releases (read the changelog; record what it says).

For `icu_experimental`, also record whether its compiled data has a
Devanagari-to-Latin and a Tamil-to-Latin rule set at all, and the data
size.

**Samples** (the output of each candidate is recorded, unchanged):

`नल वाला`, `राम`, `कमला`, `क़िला`, `प्लंबर`, `தமிழ்`, `காந்தி`,
`சென்னை`, `பிளம்பர்`.

**Record** a table: crate, version, license, last release date, rules 1
to 7 (pass or fail each), `.wasm` size difference, and the sample
outputs. Then apply Q1's answer. With Q1 (a) and no crate that is clearly
better, D-S0-3 is "own tables, ISO 15919 as the start". If a crate is
chosen, add a `## Deviations` entry to the change doc (the design said
"a fixed table").

### Step 4 — `@fluent/bundle` size and CSP check

```bash
mkdir -p "$TMPDIR/m08b-s0"
rsync -a --exclude dist --exclude bundle.tar.gz --exclude node_modules \
  crates/roym_web/ui/ "$TMPDIR/m08b-s0/hub/"
cd "$TMPDIR/m08b-s0/hub"
npm ci
npx --no-install vite build
for f in dist/assets/*.js; do ls -l "$f"; gzip -9c "$f" | wc -c; done
grep -o -E 'new Function|[^a-zA-Z_.]Function\(|[^a-zA-Z_]eval\(' dist/assets/*.js | wc -l
cp package-lock.json package-lock.before.json
npm install @fluent/bundle@latest
git diff --no-index --stat package-lock.before.json package-lock.json
npm ls vite rollup
```

`npm ci` installs the exact versions from the lock file, so the build
uses the Hub's own Vite (`^5.2.0` in `package.json`; 5.4.21 on this
machine). Before adding Fluent, check that the scratch build gives
114,463 bytes raw and 31,769 bytes after `gzip -9` (the figures in Q5).
If not, stop and find why. After the `npm install`, check that the lock
file diff adds only `@fluent/bundle` and its own dependencies, and that
`npm ls` shows the same Vite and Rollup as before.

Then add these lines at the end of `src/main.ts` in the scratch copy, so
the bundler keeps the code (a real use, not only an import):

```ts
import { FluentBundle, FluentResource } from "@fluent/bundle";
const probe = new FluentBundle("hi");
probe.addResource(new FluentResource("hello = नमस्ते, { $name }"));
const msg = probe.getMessage("hello");
if (msg?.value) console.log(probe.formatPattern(msg.value, { name: "x" }));
```

Build again (`npx --no-install vite build`) and measure the same way.
Then count the matches (not the lines: the built file is one long line,
so `grep -c` would give only 0 or 1):

```bash
grep -o -E 'new Function|[^a-zA-Z_.]Function\(|[^a-zA-Z_]eval\(' dist/assets/*.js | wc -l
npm ls --omit=dev --all
```

The first block above already counted the "before" build the same way.

Record: the `@fluent/bundle` version and license, its runtime
dependencies (`npm ls`), the JavaScript size before and after (raw and
`gzip -9`), the difference, and the `eval` count (before and after). Use
`npx --no-install vite build` and not `npm run build`: lint and `tsc` do
not change the size, and the scratch `main.ts` lines are not lint-clean.

**Pass rule:** Q5. Any new `eval` or `Function` match stops S0 and asks
the owner, whatever the size. If it fails, stop and ask the owner.

### Step 5 — Draft `spelling-pairs.tsv`

**File:** `docs/planning/milestones/M08B-language-and-structure/spelling-pairs.tsv`
(new).

**Format.** UTF-8, NFC, LF line ends, tab between columns, no quotes.
Lines that start with `#` are comments. Then one header line, then data:

```text
# spelling-pairs.tsv: test data for Roym search keys (Hindi and Tamil typed in Latin letters)
# Reader: developers who build and test search keys.
# written by: <tool name>, <date>
# owner check: pending
# kinds: pair = a word in its script and one common Latin spelling;
#   variant = another Latin spelling that people expect to match the group's word;
#   joined = the words of a multi-word group with spaces removed, expected to match;
#   distinct = a Latin spelling that must NOT match the group's word.
# A Latin query has no language, so one set of Latin folding rules serves
# Hindi and Tamil. A rule that makes a variant meet in one group can make
# a distinct row meet in another. The check script lists these conflicts.
# The owner decides each one below. B4b may not change a row without the owner.
# conflicts: (none yet)
lang	group	kind	native	latin	note
hi	nal-wala	pair	नल वाला	nal wala	plumber (spoken)
hi	nal-wala	variant		naal waala
hi	nal-wala	joined		nalwala
```

Column rules:

| Column | Rule |
| --- | --- |
| `lang` | `hi` or `ta`: the language of the native word. |
| `group` | `[a-z0-9-]`, unique within one `lang`. One group is one word or one short phrase. |
| `kind` | `pair`, `variant`, `joined` or `distinct`. A `joined` row only in a group whose `pair` Latin value has a space. |
| `native` | Only in `pair` rows. Characters of the language's script block (Devanagari U+0900 to U+097F, Tamil U+0B80 to U+0BFF) and spaces. Also U+200C (ZWNJ) and U+200D (ZWJ), which phones put inside Hindi conjuncts: at most 5 such rows per language, each with `note` = `zwnj` or `zwj`. B4b's key is expected to remove them. Empty in every other row. |
| `latin` | Lower-case `a` to `z` and single spaces only (no accents, no digits). |
| `note` | Optional. English meaning, and for a `distinct` row, the folding rule it guards. `homophone` when two groups of one language share a `pair` Latin value on purpose. |

**Counts per language:** exactly 200 `pair` rows (one per group). At least
100 `variant` rows, at least 20 `joined` rows, at least 20 `distinct`
rows. At least 20 groups per language have a `pair` Latin value with a
space (so the 20 `joined` rows have groups to belong to).

**What the 200 pairs cover, per language:**

| Part | Rows | Examples of the kind (not a fixed list) |
| --- | --- | --- |
| Services and goods for local trade | 70 | trades, repairs, food, household goods, units |
| English loanwords in the native script | 20 | `प्लंबर` / `plumber`, `பிளம்பர்` / `plumber` |
| Person names | 40 | common given and family names |
| Place names | 30 | cities, and parts like `nagar`, `puram`, `colony` |
| Common chat and query words | 40 | at least 10 Hindi and 5 Tamil words that are 1 or 2 letters long in Latin (`ek`, `do`, `na`), for ML-29 |

Across these parts, at least 20 groups per language are two or more
words (for example `nal wala`, or a trade and its place).

**Required rows** (from the criteria): Hindi group `nal-wala` with
`नल वाला` / `nal wala`, variant `naal waala` and joined `nalwala`
(ML-21, ML-22). Hindi group `ram` with `राम` / `ram` (ML-23).

**Content rules:**

- Use your own knowledge of how people type these words. Do not copy a
  published word list (copyright, and its license would apply to the
  repository).
- The `pair` Latin spelling is the most common way people type the word
  on a phone, not ISO 15919.
- `variant` rows are spellings real people use: long vowels doubled
  (`naal`), `w` and `v`, `sh` and `s`, `ee` and `i`, final `a` dropped or
  kept.
- `distinct` rows guard against folding too much. Each one is a real word
  or spelling that a careless rule would merge with the group's word (for
  example vowels of a different quality, or a consonant from a different
  place in the mouth). Write the rule in `note`.
- Do not mark which variants should match by which rule. The file says
  what people expect. Where two expectations conflict, the conflict
  report below finds it, and the owner decides.

**Check the file** with a short throwaway script in the scratch folder
(not committed; for example `python3 -I check.py spelling-pairs.tsv`):

- Format: column count 6 on every data line, the kinds, the counts
  above, unique groups, one `pair` per group, NFC
  (`unicodedata.is_normalized`), the character rules for `native` and
  `latin` (with the ZWNJ/ZWJ limit), and the required rows.
- `joined` rows only in groups whose `pair` Latin value has a space.
- A `distinct` Latin value differs from every `pair`, `variant` and
  `joined` value of its own group.
- No Latin value is used by two `pair` rows of one language, unless
  both rows have `note` = `homophone`.
- **Conflict report.** For every `variant` and `joined` row, compute the
  character edits from the group's `pair` Latin value
  (`difflib.SequenceMatcher(None, pair, row).get_opcodes()`, giving
  edits such as `k→g` or `aa→a`). These are "must meet" edits. Do the
  same for every `distinct` row: "must not meet" edits. Print every edit
  that is "must meet" in one group and "must not meet" in another, in
  either language, with both group names.

Paste the script's summary output, with the conflict report, into the
PR text. Each conflict goes into the file's `# conflicts:` block with the
owner's decision (which side to keep). The owner changes the line to
`# owner check: done <date>` only when that block is empty or every
conflict in it is decided.

**Commit:** `docs(planning): draft M08B spelling test set`.

### Step 6 — Record the results

**task.md:**

- S0's five bullets each get a "Result:" line: the number or decision and
  a link to the PR.
- B3a bullet: "Y3 uses `icu_locale_core` <version> with
  `default-features = false, features = [<list from Step 2>]`, syntax
  form only (D-S0-1)". Note whether `_` is accepted. Add the two Hub
  rules from D-S0-1 (see the change doc item below), and the two `.wasm`
  sizes from Step 2b (tag type alone, and with the NFC crate added).
- B1a bullet: B1a reports the gzip size of `@fluent/bundle` and the two
  catalogs together, not only the library (D-S0-4, Q5).
- B4a bullet: "NFC from `icu_normalizer` <version> (D-S0-2)". And: "First
  check that the `rusqlite` `bundled-sqlcipher` build supports a
  `unicode61` table with `prefix='1 2'` (one- and two-letter prefix
  queries return the right rows). S0 checked only the trigram table."
- B4b bullet: the transliteration decision (D-S0-3), and one sentence on
  joined words (section 2.2, last point but one).
- Section 2 "Search": no change (its FTS5 line is already the result).

**Change doc** ([change.md](../../changes/roym-multilingual/change.md)):

- "Tag validation": the `unverified` sentence becomes the Step 2 result,
  with the exact feature list (only `alloc` is expected; "no extra
  features" is not true, because the parse API needs `alloc`). The text
  names `syneroym-text-lang` in place of "the Rust side". Add D-S0-1's
  two rules for the Hub, with the list of inputs where Rust and
  `Intl.Locale` differ: (1) the Hub wraps every `new Intl.Locale(tag)`
  in `try`/`catch`, and treats a throw as "unknown tag" (it shows the
  raw tag and does no best-match); (2) language matching in the Hub
  compares both the stored tag and its `Intl.Locale` form (so a stored
  `iw` matches a reader whose browser asks for `he`).
- "Search": the transliteration bullet names `syneroym-text-lang` and
  D-S0-3 in place of "a fixed table in `roym_core`" and its `unverified`
  note. The FTS5 `unverified` bullet becomes the checked fact for the
  trigram tokenizer (task.md section 2), and says that the `unicode61`
  table with `prefix='1 2'` is not checked yet: B4a checks it first.
  The search key bullet names the NFC crate.
- "Which catalog format": "Bundle size of `@fluent/bundle` is
  `unverified`" becomes the measured number and the CSP result. Add that
  the 25 KB limit covers the library only, and B1a reports the total
  with the catalogs.
- "Rules for every layer", rule 4: per Q6. Cite ADR-0018 §5 as the
  project-wide "no migration" position, not as a rule about `version`.
- "Problem" and the design text: the old facts in section 2.2, per Q7.
- "Open questions", "Font size and bundle cost" (line 358): rewrite the
  last sentence to "A font file is loaded only if `index.html` also gets
  `font-src 'self'`; today `default-src 'none'` blocks every font." Keep
  "Size per script is `unverified`": S0 does not measure it.
- "Open questions", "Spelling test data": add "Drafted in
  `spelling-pairs.tsv` (M08B task folder); owner check pending."
- `## Deviations`: an entry only if D-S0-3 chose a crate.
- `status` stays `draft`. `## Tasks` boxes stay as they are (S0 finishes
  none of them).

**status.md:** the S0 row's evidence names the PR and says "results in
task.md S0".

**Deferred backlog** ([deferred-backlog.md](../../deferred-backlog.md)):

- If Q2 is (a): new row in §10 (Product surfaces & UX), "Language tags
  are not CLDR-canonicalised": the node stores old codes such as `iw` as
  given. The Hub's `Intl.Locale` reads `iw` as `he`, so the Hub matches
  on both forms, and it shows the raw tag only for a tag that
  `Intl.Locale` refuses. Add the list of inputs where the two parsers
  differ (Step 2). Target TBD. Source: the change doc's tag validation
  paragraph.
- Nothing else. The rows M08B closes (task.md section 7) stay as they
  are; their own slices change them.

**Living docs:** none change. S0 changes no behaviour that the
requirements, architecture doc, developer guide or user guide describe.
`syneroym-text-lang` enters the architecture doc in B3a or B4a, when the
crate exists.

**Commit:** `docs(planning): record M08B S0 results`.

### Step 7 — Finish

- Delete `$TMPDIR/m08b-s0/`.
- `mise run verify` as a background job, no `--skip`. It ends with every
  gate passing (the docs-only gates skip on their own, section 4).
- Push and open one PR from `feat/m08b-s0-checks-decisions`. The PR text
  has the CLA line exactly as AGENTS.md gives it, the results in one line
  each, the TSV check summary, and the attribution line.

## 6. Tests

S0 adds no automated test. Its checks are the commands in Steps 2 to 4,
whose outputs are recorded, and the TSV check script in Step 5. The tests
that use the results come in B3a (tag parsing), B4a (NFC keys) and B4b
(the spelling set as test data).

## 7. Size risks

None for the size gates: no `.rs` file changes. task.md and the change
doc grow by a few lines each. `spelling-pairs.tsv` is about 800 to 900
lines (400 `pair` rows plus variants). No size gate covers `.tsv` files,
and B4b moves it into `syneroym-text-lang`'s test data, where it is data,
not a test source file.

## 8. Exit criteria

S0 is done when all of these are true:

1. The owner has answered Q1 to Q7, and section 2.1 records the answers.
2. Steps 2, 3 and 4 (and 2b if Q3 is yes) have run, and each result is
   recorded as Step 6 says. Every pass rule passed, or the owner decided
   what to do after a failure.
3. `spelling-pairs.tsv` exists, passes the Step 5 check, and has
   `# owner check: pending` (or `done`, if the owner checked it already).
4. The change doc has no `unverified` note left about the four checks,
   the trigram tokenizer or rule 4. Its other `unverified` notes stay,
   because S0 does not check them: Indian digit grouping in every browser
   (line 142), Indic fonts on older Android (line 146),
   `Intl.MessageFormat` (line 336), font size per script (line 358), and
   the new note that the `unicode61` prefix table is not checked yet.
5. task.md, status.md and the backlog are updated as in Step 6.
6. Nothing outside `docs/` changed.
7. `mise run verify` passes with no `--skip`.
8. One PR with the plan commits and the commits after them (the spelling
   set and the results), after the plan review and a code (docs) review
   by a different tool.

## 9. What S0 does not do

- No code: no `crates/text_lang`, no catalog, no taxonomy, no FTS5 table.
  Those are B1a to B5.
- No dependency added to any `Cargo.toml` or `package.json`.
- No check of Indian digit grouping in browsers or of Indic fonts. B1a
  checks what it needs.
- No owner check of the spelling set. That is the owner's step, before the
  B4b plan.
- No decision on named area codes (task.md question 8). It stays open.
- No change to the demo stories or `hub-helpers.ts`.

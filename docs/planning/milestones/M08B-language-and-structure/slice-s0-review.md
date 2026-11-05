# M08B slice S0 — review

*Reader: developers and review agents working on this slice.*

Plan: [slice-s0-implementation-plan.md](slice-s0-implementation-plan.md).
Rules for this file: [session-strategy.md](../../session-strategy.md), section
"The review file".

## Plan review round 1 — Claude Sonnet 5.5 (Claude Code desktop), 2026-11-05

Checked on `feat/m08b-s0-checks-decisions` at `b2bf4e1c`, in the main checkout
(not a worktree). `git pull` first failed in the sandbox (GitHub was not
allowed) and then said "Already up to date" when run outside it. The two
commits after the plan's base (`0e016d8c`) change only the plan and
planning docs, so the code facts below hold on this commit.

**Who reviewed.** This session runs on Claude Sonnet 5.5. The plan was written
by Claude Opus 5.5 (the `status.md` row says the same). Both are Claude models,
so the "different tool" rule holds by model name only.

**Change docs.** The prompt lists one change doc,
[roym-multilingual](../../changes/roym-multilingual/change.md). `task.md` also
names [demo-stories](../../changes/demo-stories/change.md), in its "Demo"
paragraph. The plan says S0 does not touch it, and I agree: S0 changes no Hub
text and no story. All criteria (`ML-*`) and decisions S0 feeds belong to
roym-multilingual.

**Summary:** 0 blocker, 3 major, 9 minor, 12 items. All 12 Open.

Limits of this review: I read code and docs and ran `cargo tree` (offline) and
a few file reads. I did not run any Step 2 to 5 command, so every result the
steps will produce is still `unverified`. Two web searches (ICU4X transliterator,
`vidyut-lipi`) are the source for P7; I did not open the crates.

### What was checked and found correct

- **Line links in the plan.** Every `file:line` in sections 2.1, 2.2 and 5
  points at the right code on this commit: `Cargo.lock:3158` (`icu_locale_core`
  2.4.0) and `:3171` (`icu_normalizer` 2.4.0), `deploy_docs.rs:146`,
  `index.html:16`, `record.rs:18`, `deny.toml:93` (the licence list starts
  there; `Unicode-3.0` is in it), `Cargo.toml:42` (the release profile is
  exactly the five lines the harness copies), `verify.rs:186`
  (`is_docs_only_path`), `schema.rs:115` (trigram table). The eight rows of the
  Q7 table all match (`profile_ops.rs:140`, `listing.rs:188`,
  `directory.rs:225`, `search_ops.rs:265`, `listings.ts:487`,
  `transaction.rs:220`; the "37 files / 104" and "25 places" texts are at
  change.md lines 32 and 148). The other lines task.md section 2 cites that I
  tried also match (`listing.rs:88`, `:104`, `area.rs:29`, `person.rs:26`,
  `filter.rs:286`, `money.ts:66`, `style.css:2`, `conversation.wit:67`,
  `api.rs:35`, `messages.ts:626`, `editor.ts:181`, `dom.ts:12`,
  `search.rs:47`).
- **Hub size baseline.** `dist/assets/*.js` is 114,463 bytes and 31,769 bytes
  with `gzip -9`; `bundle.tar.gz` is 34,257 bytes. These are the plan's numbers.
  `src/main.ts` exists and is the module `index.html` loads, so the Step 4 probe
  lines go in the right file.
- **Crate facts.** `icu_locale_core` and `icu_normalizer` are in the host build
  through `idna_adapter` (`cargo tree -p syneroym-data-db -i icu_locale_core`
  shows it). `syneroym-roym-core` has neither, and its normal dependencies are
  the six the plan lists. `unicode-normalization` is not in `Cargo.lock`.
  `icu_normalizer` 2.4.0 has the feature `compiled_data` that Step 2b names.
  `wasm32-wasip2` is installed here, and `wasmtime` is at `~/.wasmtime/bin`.
- **FTS5 and trigram.** The chat store creates a trigram table
  (`schema.rs:115`), so the change doc's `unverified` note on this can be
  replaced by the checked fact (but see P8 for the other tokenizer).
- **Decision placement table (section 3).** All five decisions go to the right
  change doc paragraph. Nothing in S0 belongs to demo-stories. No `ML-*`
  criterion is met by S0, and the plan says so.
- **Behaviour-change audit (section 4).** Right. Every changed path is under
  `docs/`, so `is_docs_only_change` is true. The `fmt`, `clippy`, `file-lengths`,
  `lint-suppressions`, `module-layout` and `change-docs` gates are not
  docs-skippable and still run; the plan says the same. No `.rs`, `Cargo.toml`
  or `package.json` file changes, so no size, ratchet or duplication gate can
  move.
- **Security and access control.** S0 adds no method, record, wire field or
  config. Nothing to check. The plan keeps every download inside a scratch
  folder and refuses to copy packages in by another way.
- **Both builds.** S0 adds no code. The `wasm32-wasip2` build check is the right
  one for the tag crate. The native build of the NFC crate is in Step 2b. (The
  harness is a binary, not a `cdylib` built with `cargo component` like the real
  guests; the size difference it gives is a good estimate, not exact. The plan
  calls it "about the same code size", which is fair.)
- **Task list, backlog and living docs.** The one new backlog row (§10, tags not
  CLDR-canonicalised) is the only deferral S0 creates. §10 exists and is the
  right section. Living docs do not change. Both statements are right.
- **Terms and writing.** Terms are defined where first used. "How this is shown
  to an end user" is present, plain and honest (nothing visible).

### Views on the owner's questions

All seven were answered on 2026-11-05. My view of each answer:

- **Q1. (a), own tables.** Agree. One reason in the plan is weaker than it
  reads: "a crate upgrade changes output" is also true for a crate pinned to an
  exact version, and a pin is allowed (`# patch-pin:` in AGENTS.md). The strong
  reasons are the other two: the folding rules are ours in both options, and the
  tables are small data. The survey should still list the two candidates in P7.
- **Q2. (a), syntax form only.** Agree, and the backlog row is the right
  record. The premise "people do not type old codes" is not enough alone: a
  record from another node can carry any tag, and the Hub's `Intl.Locale` turns
  `iw` into `he` while Rust keeps `iw`. P2 asks S0 to measure this difference.
- **Q3. Yes, NFC check, prefer `icu_normalizer`.** Agree. One addition: measure
  the guest-side cost if `syneroym-text-lang` has both dependencies but a guest
  uses only the tag type (P7 does not cover this; see the note in P1's fix).
- **Q4. Yes to both.** Agree, with two changes: `joined` rows only for
  multi-word groups, and a script check that finds a conflict between `variant`
  rows of one group and `distinct` rows of another, across both languages (P3,
  P11).
- **Q5. 25 KB after `gzip -9`, and no `eval`.** Agree. The 25 KB is for the
  library alone. The two catalogs add more (Hindi text is 3 bytes per character
  before gzip), so B1a should report the total, not only this number. The
  `eval` grep needs a fix (P5), and the CSP text needs the point in P6.
- **Q6. Cite ADR-0018 §5, remove `unverified`.** Agree. §5 is titled "No
  migration" and says the product is unreleased, so it supports rule 4. Its
  subject is endpoint-record visibility, not Roym records, so cite it as the
  project-wide position, not as a rule about `version`. `developer-guide.md:1049`
  is a note about a master-DID rename; it says the same policy, but I would not
  link it for this rule.
- **Q7. (a), fix in place.** Agree. Add the two facts in P6 and P10 to the same
  edit.

### Findings

#### P1 (major) — Step 2 cannot build as written: `Locale::try_from_str` needs the `alloc` feature

- **Where:** Step 2, harness `Cargo.toml` (`default-features = false`, no
  feature) and `src/main.rs` ("`Locale::try_from_str(arg)` printed with
  `to_string()`"); the pass rule ("builds with no extra features"); change.md
  line 195 ("`unverified`: that it builds ... with no extra features").
- **Problem:** In `icu_locale_core` 2.4.0, `Locale::try_from_str`,
  `try_from_utf8`, `normalize` and `normalize_utf8` are all behind
  `#[cfg(feature = "alloc")]`
  (`~/.cargo/registry/src/*/icu_locale_core-2.4.0/src/locale.rs:154`, `:162`,
  `:183`). The crate's default features are empty, and `alloc` is a separate
  feature (`Cargo.toml`: `alloc = ["litemap/alloc", "tinystr/alloc",
  "writeable/alloc", "serde?/alloc"]`). So the harness does not compile, and
  there is no parse API without `alloc`. The doc comment on the same function
  also says that the old `_` separator is not accepted since 2.0.
- **Failure case:** The implementer follows the pass rule literally: the build
  fails, "no extra features" is false, so the plan says "stop and ask the
  owner". The question that matters is not "does it build with nothing" but
  "what is the smallest feature set, and is it small in a guest". The change doc
  would also keep a wrong sentence if the result were written as "builds with no
  extra features".
- **Fix:** In the harness, use `features = ["alloc"]` and nothing else. Change
  the pass rule to: "builds for `wasm32-wasip2` with only the `alloc` feature
  (no `serde`, `zerovec`, `databake`), runs in `wasmtime`, and gives the syntax
  form for valid tags". Record the exact feature list that B3a must write in
  `Cargo.toml`. Write that list in change.md line 195 and the B3a bullet in
  task.md. Also build `tag-check` a second time with the NFC crate from Step 2b
  added as a dependency but unused by the code path (only the tag type used), and
  record the `.wasm` size. That shows what a guest pays when
  `syneroym-text-lang` holds both (plan section 2.2, "Where Y4 runs"), so B3a can
  decide on a cargo feature from a number, not a guess. In item 4 of "Record",
  say that `_` is expected to be refused (docs line above), and record what the
  run shows.
- *Status:* Fixed (3040e6fc).

#### P2 (major) — The Hub and Rust disagree on tags, and S0 does not measure it

- **Where:** Step 2 (the 15 inputs only run in `wasmtime`); D-S0-1 and Q2
  (syntax form only); change.md "Tag validation" ("The Hub uses `Intl.Locale`
  to parse tags"); task.md B3a, B1a (UI language from `navigator.languages`) and
  ML-10, ML-12.
- **Problem:** Two parsers will judge the same tag: `icu_locale_core` in the
  Roym guest, and `Intl.Locale` in the browser. They differ. `new
  Intl.Locale("iw")` returns `he` in current browsers (it applies the CLDR
  aliases), while `icu_locale_core` without data keeps `iw`. `new
  Intl.Locale("x-private")` throws a `RangeError` (no language subtag); whether
  `icu_locale_core` accepts it is not known. The plan runs only the Rust side.
  Q2's reason ("the Hub offers a fixed list of tags") does not cover tags on
  records from other nodes.
- **Failure case:** (1) A stranger's listing carries `lang: "x-private"` or
  another tag that Rust accepts and `Intl.Locale` rejects. B3a's best-match code
  calls `new Intl.Locale(tag)` without a guard, throws, and the listing screen
  renders nothing. (2) A listing in `iw` is never matched to a reader whose
  browser sends `he`, so the Hub never shows it as "in your language" (ML-12).
  Both come out in B3a, after S0 said the question was settled.
- **Fix:** Add to Step 2 a Node run (Node 22 is installed) over the same 15
  inputs: `node -e 'for (const t of process.argv.slice(1)) { try { console.log(t
  + "\t" + new Intl.Locale(t).toString()) } catch (e) { console.log(t + "\tERR "
  + e.name) } }' hi HI hi-latn ...`. Record a second column next to the Rust
  result, and list every input where the two differ (accepts or rejects, or
  spelling). Add to D-S0-1 two rules for B3a: the Hub wraps every `Intl.Locale`
  call in `try`/`catch` and treats a throw as "unknown tag" (show the raw tag, no
  best-match); and language matching in the Hub compares both the stored tag and
  its `Intl.Locale` form. Note the result under the Q2 backlog row too: the row
  says "the Hub shows the raw tag for them", which is not what `Intl.Locale`
  does for `iw`.
- *Status:* Fixed (3040e6fc).

#### P3 (major) — The spelling file says variants "must match", but the folding is shared by both languages and the plan has no check for conflicts

- **Where:** Step 5 (header text "variant = another Latin spelling that must
  match", "Do not mark which variants should match by which rule. B4b's tests
  decide whether its rules make them meet"); Q4 (`distinct` rows); the check
  script list.
- **Problem:** A query typed in Latin letters has no language, so the Latin
  folding rules must be the same for Hindi and for Tamil text. But the two
  languages need opposite things. Tamil has one letter for `k` and `g` (the plan
  says so in Q1), so a Tamil group needs a variant such as `gadai` for `kadai`.
  Hindi has `क` and `ग` as different letters, so a Hindi `distinct` row needs
  `gal` not to match `kal`. A language-neutral fold that makes the first pair
  meet makes the second pair meet too. The header makes both rows "must", and
  the plan leaves the decision to B4b, after the owner has checked the file. The
  planned script checks counts and characters only.
- **Failure case:** The file is drafted and owner-checked as is. In B4b the
  folding either passes the Tamil variants and fails the Hindi `distinct` rows,
  or the reverse. B4b then has to change data that the owner already signed off,
  or drop rows silently. ML-22 ("tested with real samples") has no clean
  answer.
- **Fix:** (1) Add to the check script a conflict report: for every `variant`
  and `joined` row, compute the character edits against the group's `pair` Latin
  spelling (`difflib.SequenceMatcher` opcodes give pairs such as `k→g`, `aa→a`);
  do the same for every `distinct` row; print every edit that appears as a
  "must meet" edit in one group and as a "must not meet" edit in another group,
  in either language. Paste the report in the PR text. (2) Change the header
  kinds to say that Latin folding is shared by both languages, that a conflict
  from the report is listed in a `# conflicts:` comment block with the owner's
  decision (keep which side), and that B4b may not change a row without the
  owner. (3) The owner check (`# owner check: done`) must come after the
  conflict list is empty or decided.
- *Status:* Fixed (3040e6fc).

#### P4 (minor) — Step 4 may not reproduce the baseline

- **Where:** Step 4 (`rsync ... crates/roym_web/ui/ "$TMPDIR/m08b-s0/hub/"`,
  `npx vite build`, `npm install @fluent/bundle@latest`).
- **Problem:** The rsync copies the 99 MB `node_modules` (it is git-ignored, but
  present on this machine). On a machine without it, `npx vite build` has no
  local Vite and downloads the newest Vite, which is not `^5.2.0`. Even with
  `node_modules`, `npm install <pkg>` can change other installed versions. Either
  way the "before" number may not be 114,463 bytes.
- **Failure case:** The "before" and "after" come from different Vite or
  Rollup versions. The difference mixes the library size with a toolchain
  change, and the Q5 limit (25 KB) is checked against the wrong number.
- **Fix:** Exclude `node_modules` from the rsync, run `npm ci` in the scratch
  copy, and use `npx --no-install vite build`. Before adding Fluent, check that
  the scratch build gives 114,463 bytes raw and 31,769 gzip (the figures in
  section 2.1); if not, stop and find why. After `npm install
  @fluent/bundle@latest`, run `git diff --no-index` on the two `package-lock.json`
  files (or `npm ls vite rollup`) and record that only `@fluent/bundle` was added.
- *Status:* Fixed (3040e6fc).

#### P5 (minor) — The `eval` check counts lines, and a minified bundle is one line

- **Where:** Step 4: `grep -c -E 'new Function|eval\(' dist/assets/*.js`.
- **Problem:** `grep -c` counts lines that match. The Vite output is one long
  line, so the result is 0 or 1 whatever the number of matches. It also misses
  `Function("...")` without `new`.
- **Failure case:** The bundle has 3 `eval(` calls from a dependency. The check
  prints `1`, the plan records "one", and nobody can tell before and after
  apart.
- **Fix:** Use `grep -o -E 'new Function|[^a-zA-Z_.]Function\(|[^a-zA-Z_]eval\('
  dist/assets/*.js | wc -l`, and record the count before and after. Q5's stop
  rule stays: any new match stops S0 and asks the owner.
- *Status:* Fixed (3040e6fc).

#### P6 (minor) — The Hub's real CSP is narrower than the plan says, and the change doc's font sentence is wrong under it

- **Where:** plan section 2.2 "Fluent and the CSP" and Q5 (only
  `index.html:16` is cited); change.md line 358 ("Loading on demand means a
  network fetch from the node, which is allowed").
- **Problem:** The Hub has two CSP tags. `index.html` sets `default-src 'none'`
  with `script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src
  'self' http: https:`. [`src/boot-csp.ts`](../../../../crates/roym_web/ui/src/boot-csp.ts)
  adds a second tag at run time that narrows `connect-src` to `'self'` plus the
  auth origin. The plan's `fetch` for `.ftl` files is still fine (`'self'`). But
  there is no `font-src`, so the fallback is `default-src 'none'`: a web font is
  blocked even from the same origin. The change doc says a font fetch from the
  node "is allowed". It is not, with this CSP.
- **Failure case:** B1a or a later font slice follows the change doc, adds an
  `@font-face` for a Noto Devanagari font, and the browser blocks it with a CSP
  error. The "bundled web font" fallback in the change doc cannot work until
  `index.html` gets `font-src 'self'`.
- **Fix:** In Step 6, add this row to the Q7 fix list and rewrite change.md line
  358: "a font file is loaded only if `index.html` also gets `font-src 'self'`
  (today `default-src 'none'` blocks fonts)". In section 2.2 of the plan, name
  both CSP tags (`index.html:16` and `boot-csp.ts`) and say that `.ftl` through
  `fetch` or `?raw` is allowed by both.
- *Status:* Fixed (3040e6fc).

#### P7 (minor) — The transliteration survey leaves out two candidates, and has no rule for stability

- **Where:** Step 3, "Find candidates" and the six rules.
- **Problem:** The search terms and by-name list do not include
  `icu_experimental` (the ICU4X crate; its `transliterate` module runs UTS #35
  rules; the project that already supplies the tag and NFC crates; the crate
  says all its code is unstable) or `lipilekhika` (a Rust crate for Indian
  scripts, Devanagari and Tamil among 15+, with a two-way API). `vidyut-lipi`
  is listed; its releases are marked "unstable". Rule 5 asks only for a recent
  release. Neither sentence of Q1 covers an unstable API, which is the case the
  Q1 argument is about.
- **Failure case:** The survey is recorded without the one candidate from the
  same project as the other two crates. The owner reads "no crate fits", and
  M08B writes tables, when a rule set from ICU4X was available. Or a candidate
  passes six rules and then changes its output between 0.x releases.
- **Fix:** Add `icu_experimental` and `lipilekhika` by name to the list. Add a
  seventh rule: "the API is not marked unstable, or the version can be pinned
  exactly with a `# patch-pin:` reason, and its output for the sample words does
  not change in the last three releases" (read the changelog; record it). For
  `icu_experimental`, also record whether a Devanagari-to-Latin rule set is
  baked into the compiled data at all, and the data size.
- *Status:* Fixed (3040e6fc).

#### P8 (minor) — The `unicode61` prefix index (ML-29) is not checked by anything

- **Where:** plan section 2.2 first bullet and Step 6 ("The FTS5 `unverified`
  bullet becomes the checked fact"); task.md section 2 "Search" (FTS5 and
  trigram done); Y5 and ML-29.
- **Problem:** The only FTS5 use in the tree is the trigram table
  (`schema.rs:115`; `grep` finds no `unicode61` and no `prefix=` anywhere in
  `crates/`). Y5 and ML-29 need a second table with the `unicode61` tokenizer
  and `prefix='1 2'`. The change doc note being closed says "FTS5 and the
  trigram tokenizer". Closing it with "FTS5 works in this build" invites B4a to
  trust the second table.
- **Failure case:** B4a is planned with a `unicode61` + prefix design. Its first
  test shows the SQLCipher build behaves in an unexpected way (for example for a
  one-letter prefix, or for `remove_diacritics` on keys that hold Latin letters
  only). The plan for B4a is wrong, and B4b is blocked.
- **Fix:** Either add a 10-line probe to S0 (a scratch crate with `rusqlite` 0.38
  `bundled-sqlcipher`, create `fts5(k, tokenize='unicode61', prefix='1 2')`, insert
  `nal wala`, `ek do`, query `n*`, `e*`, `wa*`, print the rows), or write in
  Step 6 that the change doc note is closed for trigram only and that B4a's first
  step must check `unicode61` and `prefix='1 2'` before the plan is final.
  Either way, say which in the closing text for the FTS5 line.
- *Status:* Fixed (3040e6fc).

#### P9 (minor) — Step 1, the commit count and the base are out of date

- **Where:** plan header ("three commits: this plan, the spelling test set, the
  results"; "Base. `main` at `0e016d8c`"); Step 1; exit criterion 8.
- **Problem:** The branch already has two commits for the plan
  (`4ba8325c` plan, `b2bf4e1c` owner answers), and Step 1 ("Commit this plan")
  is done. Review and fix commits will follow. So "three commits" and "Step 1"
  describe a state that no longer exists.
- **Failure case:** An implementer runs Step 1 again and makes an empty or
  duplicate commit, or counts three commits and thinks the PR is complete after
  the TSV.
- **Fix:** Mark Step 1 done with the two shas. Change the header and exit
  criterion 8 to "the commits after the plan: the spelling set, the results, and
  the review fixes". Say "Base: `main` at `0e016d8c`; the branch also holds the
  plan commits".
- *Status:* Fixed (3040e6fc).

#### P10 (minor) — Exit criterion 4 forgets one `unverified` note

- **Where:** plan exit criterion 4 ("Its other `unverified` notes (Indian digit
  grouping in every browser, Indic fonts on older Android, `Intl.MessageFormat`)
  stay").
- **Problem:** change.md has a fourth note that stays: line 358, "Size per
  script is `unverified`" (font size and bundle cost). Lines 142, 146 and 336
  are the three the plan names.
- **Failure case:** The code-review session counts the notes after S0, finds
  one more than the plan lists, and cannot tell if it is a missed check.
- **Fix:** Add "font size per script (line 358)" to the list. P6 changes the
  same line, so keep the `unverified` word in the rewritten sentence.
- *Status:* Fixed (3040e6fc).

#### P11 (minor) — The spelling file format has three gaps

- **Where:** Step 5, column rules and counts.
- **Problem:** (1) `joined` is defined for "the same words with no space", but
  the plan does not say that a one-word group has none. At least 20 `joined`
  rows per language need at least 20 two-word `pair` rows, and the content table
  does not make sure of that (names and places are often one word). (2) The
  `native` rule allows only the script block and spaces. Real titles typed on
  phones often hold U+200C (ZWNJ) or U+200D (ZWJ) in Hindi conjuncts, and these
  are outside both blocks. B4b's key must remove or keep them, and the file has
  no row to test either. (3) The script's check does not say that a `distinct`
  Latin spelling must differ from every spelling of its own group, or that a
  `latin` value must not be used in two `pair` rows of the same language (two
  groups for one string would make the "must not match" rows contradict a
  "pair").
- **Failure case:** (1) The script passes with 20 `joined` rows that point at
  one-word groups: `nalwala` is meaningful, `ram` + `ram` is not. (2) A real
  title with ZWNJ is never tested. (3) A `distinct` row copies a `variant`
  string by mistake and the file contradicts itself.
- **Fix:** In the format: "`joined` only in a group whose `pair` Latin value has
  a space; at least 20 such groups per language (say so in the content table)".
  Allow U+200C and U+200D in `native` for at most 5 rows per language, each with
  `note = zwj` or `zwnj`, and say that B4b's key removes them. In the script:
  `distinct` ∩ own-group spellings = ∅; no repeated `latin` in `pair` rows of one
  language unless the `note` says "homophone".
- *Status:* Fixed (3040e6fc).

#### P12 (minor) — The network note may not be enough for `cargo` in the sandbox

- **Where:** Step 5 "Network".
- **Problem:** The note lists four download hosts. `cargo build` also writes to
  `~/.cargo/registry` (and `npm` to `~/.npm`), which may be outside what a
  sandboxed command can write. I did not test this; I only ran offline `cargo
  tree`, which reads. The plan also tells the implementer to read "the cached
  crate source", which is under that registry folder.
- **Failure case:** Step 2 stops with a write error that looks like a network
  error; the implementer follows "stop and tell the owner" for the wrong reason.
- **Fix:** Add one sentence: "If the build fails with a write error, set
  `CARGO_HOME` and `npm_config_cache` to folders under `$TMPDIR/m08b-s0/`, or ask
  the owner to allow the command outside the sandbox; the crate sources are then
  under that `CARGO_HOME/registry/src`."
- *Status:* Fixed (3040e6fc).

## Code review round 1 — Claude Opus 5.5, 2026-11-05

Reviewed `git diff origin/main...HEAD` on `feat/m08b-s0-checks` at `b09111ad`
(PR #348), in the main checkout (not a worktree). The work was done by Claude
Sonnet 5.5. The prompt listed two change docs:
[roym-multilingual](../../changes/roym-multilingual/change.md) and
[roym-open-foundation](../../changes/roym-open-foundation/change.md). I read
both. roym-open-foundation (M08A, `status: done`) says nothing about language
tags, NFC, transliteration or Fluent. S0 changes nothing in it, and nothing
in S0 belongs there. Every S0 decision belongs to roym-multilingual, and that
is where it is written.

**Summary:** 0 blocker, 1 major, 4 minor. 5 items. Fixes round 1: 4 fixed, 1 open (R5, needs the owner).

### What was checked and found correct

- **Scope.** The diff changes 7 files, all under `docs/`: the change doc,
  the backlog, the plan, the review file, `spelling-pairs.tsv`, `status.md`
  and `task.md`. No code, `Cargo.toml`, `Cargo.lock`, `package.json` or
  `package-lock.json` change (exit criterion 6). No wire, WIT, record, method,
  error code, config or CLI change, so there is nothing for security, access
  control or "both builds" to check.
- **CI.** `gh pr checks 348`: every job passes on `b09111ad` (Lints, licenses
  & checks; Test 1/6 to 6/6; Playwright E2E; CI Pipeline; CLA). Benchmarks and
  Code Coverage skip, as for every PR. This is the proof that the full suite
  passes (exit criterion 7).
- **Tag table (D-S0-1).** I re-ran the 15 inputs through Node 22.11
  `Intl.Locale` and through `icu_locale_core` 2.4.0 with only `alloc`
  (native build, same parser as the `wasm32-wasip2` build). Both columns of
  the table in change.md "Tag validation" match my output, including the four
  refusals (`hi_IN`, `x-private`, `toolongsubtag` and the empty string all
  give `InvalidLanguage` in Rust and `RangeError` in Node). `iw` is the only
  difference. The two Hub rules and the backlog row say what the plan asked
  for.
- **Number arithmetic.** 92,962 − 75,645 = 17,317; 125,906 − 114,463 =
  11,443; 35,974 − 31,769 = 4,205. The change doc's rounded numbers (17 KB,
  69 KB, 124 KB) match task.md.
- **Guest cost of an unused NFC function.** The plan's method (the NFC crate
  in the dependency list, no code that names it) gives 0 bytes by
  construction: `rustc` does not link a crate that nothing references. So I
  measured the case B3a really has. A library crate with `pub fn tag()` and
  `pub fn nfc()` (which calls `icu_normalizer`), and a `wasm32-wasip2` binary
  with the workspace release profile that calls only `tag()`: 93,827 bytes,
  with or without `nfc()` in the library. When the binary also calls `nfc()`:
  162,486 bytes (+68,659, close to S0's +68,734). So the conclusion in task.md
  (no cargo feature is needed on size grounds) holds. R4 asks to write this
  down.
- **`spelling-pairs.tsv`.** My own script (not committed) agrees with the
  PR's summary: 200 `pair` rows per language; Hindi 274 `variant`, 20
  `joined`, 44 `distinct`; Tamil 266, 20, 29. Six columns on every data line,
  LF line ends, NFC everywhere, `latin` only `[a-z]` and single spaces,
  `native` only in `pair` rows and only in its script block, 3 Hindi ZWNJ/ZWJ
  rows with the right `note`, 0 Tamil. 20 multi-word groups per language;
  every `joined` row is exactly its `pair` value without spaces. No `distinct`
  value repeats a spelling of its own group. No `pair` Latin value repeats
  inside one language. The required rows (`nal-wala` with `naal waala` and
  `nalwala`; `ram`) are there. At least 10 Hindi and 5 Tamil `pair` values are
  1 or 2 letters long. The header matches the plan's template, with
  `# owner check: pending` and the 18 conflicts, each "owner decision:
  pending".
- **Plan review fixes are in the results.** P1 (`alloc` feature list), P2
  (Node column and the two Hub rules), P3 (conflict report and header), P4
  (`npm ci`, baseline reproduced, Vite 5.4.21 and Rollup 4.63.0 unchanged),
  P5 (`grep -o … | wc -l`, 0 before and after), P6 (font sentence), P7
  (`icu_experimental` and `lipilekhika` surveyed, rule 7), P8 (`unicode61`
  note kept, first check of B4a), P10 (five `unverified` notes left: change.md
  lines 142, 146, 317, 354, 376, exactly the plan's list), P11 (ZWNJ/ZWJ rows,
  `joined` only in multi-word groups), P12 (`CARGO_HOME` fallback used).
- **Change doc edits.** The Q7 facts are fixed in place (eight rows of the
  plan's table). Rule 4 cites ADR-0018 §5 as the project-wide position; the
  link resolves and §5 is "No migration". The NFC and FTS5 bullets say what
  the plan asked. `status` stays `draft`, `## Tasks` is untouched, and no
  `## Deviations` entry is added (no crate was chosen). The
  `## Deviations` entry that "Tag validation" points to exists.
- **Backlog row.** §10, four columns like the rest of the table, target TBD,
  source links the change doc. It is the only deferral S0 makes.
- **Living docs.** None change, and none needs to: S0 changes no behaviour.
- **Status row.** The counts in the S0 row match the script output. Plan,
  plan review and implementation tools are recorded; the code reviewer
  (Claude Opus 5.5) is not the implementer.
- **Branch name.** The plan names `feat/m08b-s0-checks-decisions`; the PR is
  from `feat/m08b-s0-checks`. The PR text records this. Not a finding.

### Findings

#### R1 (major) — The change doc says no crate met every rule, but the survey table says `lipilekhika` met all seven

- **Where:** at `b09111ad`:
  [change.md:316](../../changes/roym-multilingual/change.md#L316) ("No crate
  met every rule we set … The closest, `lipilekhika`, gives plain
  letter-by-letter ISO 15919 output"); [task.md:79](task.md#L79) ("No crate
  met all seven rules"); against [task.md:84](task.md#L84) (`lipilekhika`
  row: P for rules 1 to 7) and [task.md:91](task.md#L91) ("`lipilekhika`
  passes all rules"). The PR text repeats "none met all seven rules".
- **Problem:** Two parts of the decision record contradict each other. The
  change doc is the design record for D-S0-3, and it gives a reason that the
  survey table says is false. By the owner's rule for Q1, the decision is
  own tables *unless* a crate meets every rule **and** gives output that our
  folding cannot easily get. If `lipilekhika` passes every rule, the decision
  rests only on the second condition, and the doc must say so. The second
  reason is also not right as written: `lipilekhika`'s sample output is not
  plain ISO 15919. It gives `kzilā` for `क़िला` (ISO 15919: `qilā`),
  `cheṉṉai` for `சென்னை` (ISO 15919: `ceṉṉai`, as the other two working
  crates give) and `piḷaṁpar` for `பிளம்பர்` (an anusvara that Tamil does not
  have). Rule 7 for it rests on a `WebFetch` summary of the changelog, and
  its 1.1.0 release broke its API inside a 1.x line (the PR says both).
- **Failure case:** The B4b planner, or a later reader who wants to swap our
  tables for a crate, reads the change doc: "no crate met every rule". They
  do not look at `lipilekhika` again, or they find the task.md table and
  cannot tell which of the two records is right. The reason "it gives
  ISO 15919, which our tables give just as well" also hides the three places
  where the crate's output differs, which are the places our folding tests
  must cover.
- **Fix:** Make the three places say the same thing. Recommended text for
  change.md:316: "One crate, `lipilekhika`, passed all seven rules (rule 7
  from a summary of its changelog only; its 1.1.0 release broke the API). We
  still write our own tables, by the owner's rule for this choice: its output
  is not something our tables cannot give. It is close to ISO 15919, and
  where it differs (`kzilā` for `क़िला`, `cheṉṉai`, `ṁ` for Tamil `ம்`) we do
  not want its form." Change task.md:79 the same way ("One crate,
  `lipilekhika`, met all seven rules; …"). If the fix session instead decides
  that rule 7 is not proven for `lipilekhika`, mark it `?` in the table at
  task.md:84 and say why at :91; then "no crate met every rule" is true and
  can stay. Either way, fix the PR text.
- *Status:* Fixed (75502bd1). Took the first option: `lipilekhika` did pass all seven rules, so task.md, the change doc and the PR text now say that, mark rule 7 as read from a changelog summary only, and name the three places its output differs from ISO 15919. The decision to write our own tables stays.

#### R2 (minor) — The owner-answer date 2026-11-01 is before the questions existed

- **Where:** at `b09111ad`: plan lines 127, 160, 182, 206, 227, 252, 266
  ("Owner answer: …, 2026-11-01"); [task.md:109](task.md#L109) ("answered on
  2026-11-01"); [change.md:195](../../changes/roym-multilingual/change.md#L195)
  and [:316](../../changes/roym-multilingual/change.md#L316) ("Decided
  2026-11-01"); [deferred-backlog.md:535](../../deferred-backlog.md#L535)
  ("Owner decision 2026-11-01").
- **Problem:** The plan with Q1 to Q7 was first committed on 2026-11-05 16:11
  (`4ba8325c`). The answers were committed on 2026-11-05 17:05 (`b2bf4e1c`).
  The plan review round 1 above says "All seven were answered on 2026-11-05",
  and [task.md:108](task.md#L108) says the same. change.md:316 says "Decided
  2026-11-01, after a survey of seven crates", but the survey ran on
  2026-11-05 (task.md:79 says the crates.io dates are from 2026-11-05). So the
  date is wrong, and it is now in a change doc and the backlog.
- **Failure case:** A reader orders events by these dates, for example to see
  whether the owner decided D-S0-3 before or after the survey. The record says
  the decision came first, four days before the questions were written.
- **Fix:** Ask the owner for the real date. If it is 2026-11-05 (the commit
  history says so), replace 2026-11-01 with 2026-11-05 in all the places
  above. In change.md:316, also put the survey before the decision ("After a
  survey of seven crates (2026-11-05), we write our own tables").
- *Status:* Fixed (0fb74f5f). The plan, the commit times and the earlier A4 fix (2026-11-01 to 2026-11-02) all show the answers cannot be older than 2026-11-05. Changed the date in the plan (7 places), task.md, change.md (2 places) and the backlog row. The survey now comes before the decision in change.md. I did not ask the owner: the commit history is clear. Please say so if the real date is different. `docs/planning/prompt-templates.md:248` still shows `ANSWER_DATE: 2026-11-01` as an example; I left it, because it is a template and not in this slice.

#### R3 (minor) — `spelling-pairs.tsv` has one `distinct` row that no key can pass, and one that guards two rules

- **Where:** at `b09111ad`,
  [spelling-pairs.tsv:61](spelling-pairs.tsv#L61) (`hi nai distinct nahi`)
  and [:210](spelling-pairs.tsv#L210) (`hi pankha distinct banka`, note
  "consonant p/b, aspiration h/-"); the `edit h→` entry of the `# conflicts:`
  block ([:21](spelling-pairs.tsv#L21)).
- **Problem:** (1) `nai` is the `pair` of `नाई` (barber) and a `variant` of
  `नहीं` (`hi/nahi`, line 510). `nahi` is the `pair` of `नहीं`. Search applies
  the same key function to the query and to the text. So key(`nai`) =
  key(`नाई`) and key(`nai`) = key(`नहीं`) = key(`nahi`). Then the query `nahi`
  meets `नाई`, and the `distinct` row at line 61 fails for every possible key
  function. The edit-level conflict report does show `hi/nai` under `edit →h`,
  but as one side of a choice ("keep must meet or must not meet"), not as a
  row that cannot pass. (2) The plan (Q4) says each `distinct` row guards one
  folding rule. `banka` differs from `pankha` by two rules. The script counts
  both edits as "must not meet", so `hi/pankha` appears on the "must not meet"
  side of `edit h→`, against 32 groups on the "must meet" side. That entry is
  partly false: `banka` is already kept apart by `p/b` alone.
- **Failure case:** (1) The owner signs the file off with this row. B4b's
  test for it fails whatever folding B4b writes, and B4b must change
  owner-checked data, which the file header forbids. (2) The owner reads the
  `h→` conflict as "aspiration must sometimes be kept", and may keep a rule
  that the data does not need.
- **Fix:** (1) Delete line 61, or replace `nahi` with a one-rule spelling that
  no other group uses as `pair`, `variant` or `joined`. (2) Change line 210 to
  `bankha` (guards `p/b` only). Re-run the check script and rewrite the
  `# conflicts:` block from its output. Add one check to the script: join
  every Latin value to its group's word through `pair`, `variant` and `joined`
  rows of all groups (union-find over both languages), and report a `distinct`
  row whose value is already joined to its own word. Paste the new summary
  into the PR text. Two points for the owner check, not for the fix session to
  decide: `ta venum` pairs the written `வேண்டும்` with the spoken spelling
  `venum` (the spoken word is `வேணும்`), and `ta sakkarai` pairs the written
  `சர்க்கரை` with the spoken `sakkarai`. Each adds a "must meet" edit (drop
  `d`, drop `r`) that the folding then needs.
- *Status:* Fixed (3fac0f9c). Deleted the `hi nai distinct nahi` row. Changed `banka` to `bankha`. Added the union-find check to the throwaway script and rebuilt the `# conflicts:` block: 17 conflicts now (the `→h` entry is gone and `pankha` left the `h→` entry); Hindi `distinct` rows 44 to 43. Proof: the new check run on the file before the fix printed `ERROR line 61: distinct 'nahi' is already joined to 'nai'` and nothing else (1 error); after the fix it printed 0 errors. The two points for the owner (`ta venum`, `ta sakkarai`) are not changed.

#### R4 (minor) — task.md gives two sizes for the same "tag type alone" build, and the size-0 method is not explained

- **Where:** at `b09111ad`: [task.md:75](task.md#L75) ("75,645 to 92,962"),
  [task.md:76](task.md#L76) and [task.md:155](task.md#L155) ("92,994 bytes"
  for the tag type alone and with the NFC crate), and
  [change.md:312](../../changes/roym-multilingual/change.md#L312) ("the same
  size (92,994 bytes)").
- **Problem:** The same build appears as 92,962 and 92,994 bytes. Only the PR
  text explains the 32 bytes (a different `CARGO_HOME`, so different paths in
  the binary). The PR text is not a lasting record. Also, the method behind
  "92,994 both ways" (the crate in the dependency list, no code that names it)
  gives 0 bytes by construction, because `rustc` does not link an unreferenced
  crate. The real case for B3a is a `syneroym-text-lang` with a public NFC
  function that a guest does not call. My check in this round measured that
  case and got the same answer (93,827 bytes both ways; +68,659 when called).
- **Failure case:** The B3a planner sees two numbers for one build and cannot
  tell which to compare against. Or a reviewer of B3a sees that the S0 method
  could not have shown a cost, and asks for the measurement again.
- **Fix:** In task.md:76, add one sentence: "92,994 and 92,962 (line above)
  are the same build with a different `CARGO_HOME`; the paths in the binary
  differ by 32 bytes. Compare only numbers from one build folder." In
  task.md:155, replace "With the NFC crate also in the dependency list and not
  called" with the case measured in this review: "With a library that also
  has a public `nfc()` calling `icu_normalizer`, not called by the guest, the
  size does not change (93,827 bytes both ways in code review round 1;
  +68,659 bytes when the guest calls it)." Keep "the plan decides".
- *Status:* Fixed (75502bd1). Re-measured the case B3a has (a library with a public `nfc()`, a guest that calls only the tag function): 93,305 bytes with and without `nfc()`, and 162,069 bytes (+68,764) when the guest calls `nfc()`. This agrees with the review (93,827 and +68,659; the difference is the build folder). task.md and change.md now use this case, and task.md explains the 32-byte path difference between 92,962 and 92,994.

#### R5 (minor) — Step 7: the scratch folder is not deleted

- **Where:** plan Step 7, first bullet ("Delete `$TMPDIR/m08b-s0/`"). The PR
  text says the delete command was denied.
- **Problem:** `/tmp/claude-501/m08b-s0` still exists and holds 1.1 GB (build
  output for 4 harnesses, 7 crates and a copy of the Hub with
  `node_modules`).
- **Failure case:** Disk space is lost until a reboot clears `/tmp`. A later
  S0 re-run in the same folder may reuse old build output and lock files.
- **Fix:** The owner runs `rm -rf /tmp/claude-501/m08b-s0` (outside the
  sandbox if needed). No repository change. Mark this item `Fixed (no
  commit)` when done.
- *Status:* Fixed (no commit). The owner deleted `/tmp/claude-501/m08b-s0` on 2026-11-05; confirmed gone by Claude Opus 5.5 (code review round 2).

## Code review round 2 — Claude Opus 5.5, 2026-11-05

Checked the round 1 fix commits `0fb74f5f`, `75502bd1`, `3fac0f9c` and
`b1757a66` on `feat/m08b-s0-checks` at `b1757a66`, in the main checkout.
`gh pr checks 348` passes on `b1757a66` (every job; Benchmarks and Code
Coverage skip as usual). The fix commits change only `docs/`.

**Summary:** 0 blocker, 0 major, 0 minor new. R1 to R4 confirmed fixed. R5
was still Open at first; the owner then deleted the folder, so it is fixed
too (see R5 below). 0 open.

### Round 1 items

- **R1 — confirmed fixed (`75502bd1`).** task.md:79, task.md:91, change.md:316
  and the PR text now all say that `lipilekhika` met all seven rules, with
  rule 7 from a changelog summary only and the 1.1.0 API break. The reason for
  own tables is now Q1's second condition, with the three places where its
  output differs from ISO 15919. "No crate met", "plain letter-by-letter" and
  "our own tables give the same" are gone from task.md and the change doc.
- **R2 — confirmed fixed (`0fb74f5f`).** `git grep -F 2026-11-01` finds no
  owner-answer date left in the plan, task.md, the change doc or the backlog
  row. change.md:316 now puts the survey (2026-11-05) before the decision.
  Leaving the example value in `prompt-templates.md` is right; it is not part
  of this slice.
- **R3 — confirmed fixed (`3fac0f9c`).** The `hi nai distinct nahi` row is
  gone, and `pankha` now has `bankha` ("consonant p/b"). I re-ran my own two
  scripts from round 1 on the new file. Format: 0 errors; Hindi `distinct`
  43, Tamil 29, the other counts as before. Union-find: no `distinct` row is
  joined to its own word any more. The `# conflicts:` block has 17 entries,
  each "owner decision: pending"; the `→h` entry is gone and `hi/pankha` left
  the "must not meet" side of `h→`. One `distinct` row still has two edits
  (`chini` → `chana`), but both are the same rule (`i/a`), so it guards one
  rule; not a finding. task.md and the PR text say 43 and 17.
- **R4 — confirmed fixed (`75502bd1`).** task.md:76 explains the 32-byte path
  difference between 92,962 and 92,994 and says to compare only numbers from
  one build folder. task.md:155 and the change doc now give the case with a
  public, uncalled `nfc()` (93,305 bytes both ways, +68,764 when called), which
  agrees with my round 1 numbers from another folder.
- **R5 — still Open.** `/tmp/claude-501/m08b-s0` still exists (checked
  2026-11-05 after the fix commits). The owner must delete it; then set R5 to
  `Fixed (no commit)`.

### Other checks on the fix commits

- The S0 row in status.md still said "18 conflicts" in its first part after
  the fix (the later part said 17). I changed it to 17 while setting the row
  for this round, so it is not a finding.
- **R5 — fixed after this round was first written.** The owner deleted the
  folder. `ls -d /tmp/claude-501/m08b-s0` now says "No such file or
  directory". R5 is `Fixed (no commit)`.

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

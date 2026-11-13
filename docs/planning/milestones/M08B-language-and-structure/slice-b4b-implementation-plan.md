# M08B slice B4b — Search in every script: implementation plan

*Reader: the developer (or AI) who implements slice B4b. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-12; plan review rounds 1 to 4 (P1–P19)
folded in, 0 open. Owner answers Q1 to Q8 recorded, 2026-11-13: all (a)
(section 2.1). `spelling-pairs.tsv` says `# owner check: done` (Step 2
done). Implementation can start at Step 3.**

**Size and PR.** L. One PR on `feat/m08b-b4b-search-scripts`, with the
commits of section 6 in that order. The branch starts from `main` at
`bc3dd8de` (B4a is merged, [PR #357](https://github.com/syneroym/syneroym/pull/357))
and already holds the plan commit.

B4b makes text search work in every script, and across scripts. A person
types `nal wala` in Latin letters and finds a listing titled `नल वाला`. A
person types `नल वाला` and finds a listing titled `nal wala`. Common Latin
spellings of one word meet: `naal waala`, `nalwala` and `nal wala`. The
same search runs in three places: the directory (listings), chat search,
and the contact list. It does this by changing the **search key**, the
form of a text that search compares. Today the key is NFC and lower case.
B4b adds two steps: a fixed table turns Devanagari and Tamil letters into
Latin letters, and a fixed set of rules folds Latin spellings together.
The directory stops using its old ASCII-only index and uses the B4a
`$search` operator. Chat search gets a short-word index, so a 1- or
2-letter query no longer scans every message. `contacts.list` gets a text
query. The Hub sorts names with the browser's collation, grouped by script.

**Base.** Every file path and line number below was checked on `main` at
`bc3dd8de`. Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: B4b depends on B3b and B4a.
Both are "Merged" in [status.md](status.md). B5 waits for B4b.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - No planning ids in code or comments: no "M08B", "B4b", "ML-21",
    "D-B4b-3" or review ids (`P4`, `R7`) in `.rs`, `.ts`, `.wit`, `.ftl`,
    `.json` files or test names (B2 R5, B3a R7).
  - Comments never cite size rules.
  - **No code cites a planning doc.** The spelling test data moves into the
    crate (D-B4b-9). No test reads a file under `docs/planning/`.
  - **File size.** Production files at most 800 lines; test files at most
    800 lines; a file in `xtask/oversized-test-files.txt` is capped at its
    recorded count. Section 7.5 lists the files near a limit.
  - **Function size.** `clippy::too_many_lines` at 100. Section 7.5 lists
    the functions that grow.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out and the demo close-out.
  - Every `roym_*` crate change needs `mise run build:roym` before you
    trust a parity run, and the `dual-build-fixture` change needs
    `mise run build:test-components` ([task.md](task.md) section 2,
    "Records").
- [task.md](task.md): section 1 (Y4, "Where Y3 and Y4 live", "Component
  size limit"), section 2 ("Search"), "B4a" and its result line, "B4b",
  section 4, section 7 (the backlog rows).
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  the term "Search key"; "Rules for every layer" (rule 4 "No
  migrations"); the section "Search" (every bullet, the decision of
  2026-10-26 and the list "Decided 2026-11-11, the full-text option in
  data-layer"); "Names in any script"; criteria ML-20 to ML-24 and ML-29;
  `## Deviations` (the last entry says cross-script search is deferred to
  B4b). Every B4b decision and deviation is written there, and only there
  (section 3).
- [spelling-pairs.tsv](spelling-pairs.tsv): the header (lines 1 to 30) and
  the column rules in [slice-s0-implementation-plan.md](slice-s0-implementation-plan.md)
  Step 5. The test data of this slice.
- [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md), its
  amendment "2026-11-11 (Full-text option in data-layer)" (`:225` to
  `:244`), and [ADR-0025](../../../decisions/0025-conversation-capability-owns-history.md)
  (the conversation capability owns history and its search).
- [slice-b4a-implementation-plan.md](slice-b4a-implementation-plan.md) (the
  layout of this plan, and D-B4a-2 to D-B4a-9) and the seven review files.
  Section 7.8 lists how B4b avoids their kinds of findings.

**Change docs.** The prompt lists one change doc, roym-multilingual.
[task.md](task.md) names no other change doc for B4b. Every criterion B4b
meets is in roym-multilingual (section 3).

### Terms used in this plan

- **Search key:** the form of a text that search compares (change doc,
  terms). After B4b: Unicode NFC, lower case, a Latin transliteration of
  Devanagari and Tamil letters, then the Latin fold. Made by
  `syneroym_text_lang::search_key`.
- **Transliteration table:** a fixed table in `syneroym-text-lang` that
  turns the letters of one script into Latin letters, by sound. One per
  script: Devanagari and Tamil. It is lossy on purpose: two spellings of
  one word must meet, nothing more.
- **ISO 15919:** the standard way to write Indic scripts in Latin letters
  (for example `नल` is `nala`, `ā` is a long `a`). Our tables start from it
  (D-S0-3) and differ where search needs it (D-B4b-3, D-B4b-4).
- **Latin fold:** a fixed, ordered list of rewrite rules over Latin letters
  (for example `w` becomes `v`, a doubled letter becomes one). It runs on
  Latin text that a person typed, and on the output of a transliteration
  table. One fold serves Hindi and Tamil, because a Latin query has no
  language.
- **Schwa:** the short `a` that every Devanagari consonant carries unless a
  sign changes it. Hindi does not say it at the end of a word and in some
  places inside a word (`नल` is said `nal`, not `nala`). Removing it in the
  table is **schwa deletion**.
- **Key words / joined key:** the search key is a list of words with one
  space between them (the key words). The joined key is the same with the
  spaces removed. The `unicode61` word table indexes the key words; the
  trigram table indexes the joined key (task.md B4b, "Joined words").
- **Long term / short term:** a word of the query, after the search key is
  applied. A long term has 3 or more `char`s and goes to the trigram table.
  A short term has 1 or 2 and goes to the word table (B4a).
- **Text index:** the two FTS5 tables that hold the keys of one collection
  (B4a, in `data_db`) or of the chat messages (new in B4b, in
  `crates/conversation`).
- **Both builds:** a Roym service runs as a `wasm32-wasip2` component, or
  linked into the substrate through `syneroym-app-host-native`. The
  dual-build parity tests check that both give the same answer.
- **Collation:** the order in which a language sorts text. The browser
  gives it through `Intl.Collator`.

## 1. What B4b builds, in one paragraph

`syneroym-text-lang` gets two transliteration tables (Devanagari, Tamil),
the Latin fold, and a new `search_key` that returns key words; `query_terms`
splits that key; `SEARCH_KEY_VERSION` becomes 2 (D-B4b-1 to D-B4b-7). A
small `fts` module in the same crate holds the two FTS5 tokenizer strings,
the joined-key function and the match-expression builder, so `data_db` and
`crates/conversation` share one copy (D-B4b-8). `data_db` indexes the
joined key in the trigram table (one line of `sync_row`); every text index
rebuilds itself on the next `create-collection` because the version
changed. The directory declares `text` as a text field of `search_index`,
stores the raw title, summary, categories and author translations in it,
and searches with `$search`; `normalize_text` and the `-32602` refusal for
a non-Latin query go away (D-B4b-10 to D-B4b-13). The chat store replaces
`messages_fts` with a trigram table and a word table over search keys,
removes the `LIKE` scan, and rebuilds its index when the key version
changes (D-B4b-14 to D-B4b-17). `contacts.list` gains an optional `text`
field that searches the display name (D-B4b-18). The Hub sorts the contact
list and the group member list with `Intl.Collator`, grouped by script,
adds a search box to the contact list, and updates the directory search
help (D-B4b-19 to D-B4b-21). Story 40 gets the B4b step: a Latin search
finds a Devanagari title (D-B4b-22).

## How this is shown to an end user

- Alice writes a listing in Hindi, with the title `नल वाला`, and publishes
  it to a directory.
- Bob opens the Directory screen in an English Hub and types `nal wala` in
  Latin letters. He presses Search. Alice's listing is in the results.
- Bob also tries `naal waala` and `nalwala`. Both find the same listing.
- In the Contacts screen, Bob types `ram` in the new search box. The
  contact `राम` shows (Ram wrote his name in Devanagari in his own profile,
  and Bob added him from it). The contact list is in alphabetical order, with the
  names in Bob's own script first and the other scripts after them.
- In Messages, Bob searches his chats for `ek` (a 2-letter word). Every
  message with the word `ek` (or `एक`) shows.

How a developer can see it work:

- `cargo nextest run -p syneroym-text-lang` runs the table tests and the
  spelling test: every row of the spelling data is checked, and the report
  names each row that does not match as the owner decided.
- `cargo nextest run -p syneroym-roym-web --test dual_build_parity
  search_scripts` runs the directory, contact and chat searches through
  both builds: `nal wala` finds `नल वाला`, `ram` finds `राम`.
- `mise run demo` runs story 40. Its new last step publishes a listing with
  a Devanagari title and finds it with a Latin search in the Hub.

**Demo:** story 40 (`crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts`)
gets one step (D-B4b-22). It is the step that task.md promised for B4b.

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option. Q1 to Q3 also close the owner check of `spelling-pairs.tsv`
(Step 2).

**Plan-time measurement.** To answer Q1 to Q3 with numbers, the planning
session wrote a throwaway Python model of the tables and the fold of
D-B4b-3 to D-B4b-5 (not committed) and ran it over every row of
`spelling-pairs.tsv`, with the B4b matching rules (every query word must
match; a long word as a substring of the joined key, a short word as a
word prefix). Results:

| Check | Hindi | Tamil |
| --- | --- | --- |
| `pair` Latin finds the native word (ML-21, first half) | 166 / 200 | 144 / 200 |
| native word finds the `pair` Latin (ML-21, second half) | 166 / 200 | 142 / 200 |
| `variant` row finds the group's word (ML-22) | 174 / 274 | 133 / 266 |
| `joined` row finds the group's word | 17 / 20 | 11 / 20 |
| `distinct` row does **not** find the group's word | 41 / 43 | 28 / 29 |

All four required rows pass: `nal wala`, `naal waala` and `nalwala` find
`नल वाला`, and `ram` and `Ram` find `राम` (ML-21, ML-22, ML-23).

Most misses are of four kinds, and each needs a rule that would also merge
words that must stay apart:

1. **English loanwords** (`plumber` / `प्लंबर`, `driver` / `டிரைவர்`,
   `doctor` / `डॉक्टर`): the English spelling and the spoken form differ in
   vowel quality (`u`/`a`, `o`/`a`, `ai`/`e`). About 30 groups per language.
2. **Vowel quality in Latin spellings** (`mohun` / `mohan`, `lehsun` /
   `lahsun`, `nariyel` / `nariyal`): the S0 conflict list (`edit a→e`,
   `e→a`, `u→a`, `o→a` and others).
3. **Tamil `ச` typed as `ch` or `s`, and `க` as `k` or `g`** (`chennai` /
   `சென்னை`, `milakai` / `milagai`).
4. **Retroflex `ड़`/`ढ़` typed as `d` or `r`** (`kapde` / `kapre`, `jharu` /
   `jhadu`).

The S0 conflict list (17 edits) compares letters, not rules, so it
overstates the conflicts. Under the rules of D-B4b-5, every `distinct` row
of the data stays apart except three: `hi/jharu`/`jaru` (Q1),
`hi/saree`/`sadi` and `ta/ennai`/`enna` (Q3). Twelve of the 17 edits are
vowel-quality edits (`a→e`, `e→a`, `e→i`, `a→o`, `a→u`, `i→u`, `o→a`,
`o→u`, `t→d`, `u→a`, `u→i`, `u→o`) that no B4b rule makes, so their "must
meet" rows stay misses (kind 2). The other five (`a→`, `e→`, `i→`, `h→`,
`→i`) are length, doubling and aspiration edits that the rules make where
a row needs only them.

The gains below come from removing one rule at a time from the model and
counting the rows that stop matching. They are approximate: the model is
not the Rust code, and Step 3 measures again with the golden file
(D-B4b-9).

**Q1. Which Latin fold rules? (blocks Steps 2 and 3)**

*Owner answer, 2026-11-13:* (a). Use the rules of D-B4b-5 and never fold vowel quality, because that merges real words (`kela`/`kala`). This decides the 17 conflicts: 5 keep the "must meet" side, 12 keep the "must not meet" side. Accepted cost: English loanwords (`plumber`/`प्लंबर`) do not meet, and some English words merge (`sheet`, `seat`, `sit`).

The rules are listed in D-B4b-5. The table gives, for each rule that
changes a result, how many rows it makes meet (gain) and which `distinct`
rows it merges (cost), from the plan-time model (Hindi + Tamil rows,
`variant` + `joined` + `pair` checks).

| Rule | Gain | Cost (`distinct` rows merged) | S0 conflict it settles |
| --- | --- | --- | --- |
| a doubled letter becomes one (`aa`→`a`, `nn`→`n`) | +200 | none | `edit a→` (must meet) |
| `ee`→`i`, `oo`→`u` | +119 | none (they keep `meen`/`men` and `koorai`/`korai` apart) | `edit e→`, `edit i→` |
| `h` after a consonant is dropped (`th`→`t`, `kh`→`k`, `bh`→`b`) | +96 | `jharu`/`jaru` | `edit h→` (must meet) |
| `w`→`v` | +63 | none | — |
| `zh`→`l`, `z`→`j`, `ph`→`f` | +24 | none (`ph`→`f` keeps `phal`/`pal` apart) | — |
| `ei`/`ey` at word end→`ai`, `ay`→`ai`, `yi`/`iy`→`i`, word-final `y`→`i` | +37 | none | `edit →i` (must meet) |
| word-final `er`→`ar` (`teacher`/`टीचर`) | +8 | none | — |
| `x`→`ks`, `q`→`k`, `sch`→`sk`, `ea`→`i`, `c` before `e`/`i`/`y`→`s`, else `k` | +14 | none | — |

- (a) **The rules of D-B4b-5, as listed above. Vowel quality is never
  folded** (kinds 1 and 2 stay misses). The 17 S0 conflicts are decided:
  `edit a→`, `edit e→`, `edit i→`, `edit h→` and `edit →i` keep the "must
  meet" side; the twelve vowel-quality edits keep the "must not meet"
  side; the `distinct` row `hi/jharu`/`jaru` is marked "accepted merge".
  *Recommended*:
  every rule above has no cost except one row, and a vowel-quality rule
  merges real words (`kela`/`kala`, `dudh`/`dadh`, `sunar`/`sonar`).
- (b) (a) plus a loanword rule set (`u`→`a` before `mb`/`lb`, `o`→`a`
  before `ct`/`kt`, `ai`→`e`). About +40 rows, but it merges `bulb`/`balb`
  with `bal`-words and cannot be tested without an English word list.
- (c) Only the doubled-letter and long-vowel rules. Fewest false matches,
  but ML-22's own examples (`nal vala`, `naal waala`) depend on `w`→`v`,
  and about 160 more rows miss.

Cost of (a) that is not in the spelling data: the fold also runs on English
text, because a Latin query has no language. English words that differ
only by these rules now meet: `sheet`, `seat` and `sit` all become `sit`;
`fill` and `fil` meet. Directory and chat search are for local commerce,
where finding a little too much is better than finding nothing. This goes
into the PR text and the behaviour audit (section 4).

**Q2. Tamil `ச` at the start of a word, and Tamil `க`. (blocks Step 3)**

*Owner answer, 2026-11-13:* (a). The form is picked by position, with no Latin rule. Typed `chennai` misses `சென்னை`; a `ch`→`s` rule would merge Hindi `च` and `स` words.

Tamil writes one letter, `ச`, for the sounds people type as `s`, `ch` and
`j`, and one letter, `க`, for `k`, `g` and `h`. The table picks one form by
position (D-B4b-4). The Latin side cannot know which language a word is.

- (a) **By position, no Latin rule.** `ச` at the start of a word and
  between vowels is `s`; doubled (`ச்ச`) is `ch`; after a nasal is `j`.
  `க` between vowels or after a nasal is `g`, else `k`. Typed `chennai`
  then misses `சென்னை` (`senai`), and `milakai` misses `மிளகாய்`
  (`milagai`). *Recommended*: most spellings in the data follow these
  positions (`selai`, `saravanan`, `murugan`, `manjal`), and a Latin rule
  `ch`→`s` would merge Hindi `च` and `स` words (`chal`/`sal`, `chor`/`sor`).
- (b) (a) plus the Latin rule `g`→`k`. About +15 Tamil rows, but it merges
  Hindi `gali`/`kali`, `gaana`/`kana`.

**Q3. Rows that substring search cannot keep apart, and `ड़`/`ढ़`.
(blocks Step 2)**

*Owner answer, 2026-11-13:* (a). `ta/ennai distinct enna` is an accepted merge (a query that is the start of a word finds it). `ड़` is `d` and `ढ़` is `dh`; `hi/saree distinct sadi` is an accepted merge.

A long query word matches anywhere inside the joined key (B4a's trigram
rule). So a query that is the start of the group's word always matches:
`enna` finds `ennai`. Separately, Hindi `ड़` and `ढ़` are typed both as `d`
and as `r`, and the table must pick one.

- (a) **Mark `ta/ennai distinct enna` as "accepted merge" (the owner
  cannot keep it apart with substring search). Map `ड़` to `d` and `ढ़` to
  `dh`.** Most Latin spellings in the data use `d` (`kapde`, `lakdi`,
  `khidki`, `badhai`, `kabadi`); `saree`/`sadi` and `jhadu`/`jharu` then
  go the other way, and `hi/saree distinct sadi` is marked "accepted
  merge". *Recommended.*
- (b) Map `ड़` to `r`. Then `kapde`, `lakdi` and `khidki` miss their
  native words.

Every "accepted merge" and every owner choice of Q1 to Q3 is written into
the header of `spelling-pairs.tsv` (the `# conflicts:` block) when the
owner checks the file. Rows do not change (the header says "B4b may not
change a row without the owner").

**Q4. Which scripts get a table in B4b? (blocks Step 3)**

*Owner answer, 2026-11-13:* (a). Devanagari and Tamil only. Text in other scripts is found only by a query in the same script, and a backlog row records them.

- (a) **Devanagari and Tamil only** (task.md Y4, the change doc). Text in
  any other script (Bengali, Telugu, Kannada, Gujarati and others) gets
  the NFC and lower-case key, as in B4a. It is found by a query in the
  same script, never by a Latin query. A backlog row records the other
  scripts. *Recommended*: the test data exists only for Hindi and Tamil.
  Note: Marathi (`mr`, in the Hub's record language list) and Nepali are
  written in Devanagari, so their text gets the Hindi table and Hindi
  schwa deletion. That is mostly right for search. Marathi `ऱ` (U+0931)
  gets the value `r` (D-B4b-3, P9); no Marathi test data exists.
- (b) Also the seven other Brahmic scripts of India. They share the
  Devanagari structure, so the tables are small. But there is no test
  data, and each table would ship untested.

**Q5. Which Hub name lists are sorted and grouped by script? (blocks
Step 8)**

*Owner answer, 2026-11-13:* (a). The contact list and the group member list.

ML-24 says "the Hub sorts names". Today no Hub list is sorted
(`localeCompare` and `Intl.Collator` appear nowhere in `src`).

- (a) **The contact list and the group member list.** These are the two
  lists of person names. Directory hits stay newest first (the help text
  promises this, `en.ftl:667`); the SynOrg roster shows DIDs only
  (`screens/synorg.ts:185` to `:192`). The Hub has no contact picker today:
  the group add box takes an address or a DID
  (`screens/groups.ts:560` to `:575`), so the change doc's "contact picker
  compares keys" has nothing to change in B4b. *Recommended.*
- (b) Only the contact list.

**Q6. In which order do the script groups come? (blocks Step 8)**

*Owner answer, 2026-11-13:* (a). The script of the Hub language first, then Latin, Devanagari, Tamil, then the others.

- (a) **The script of the Hub language first, then Latin, Devanagari,
  Tamil, then every other script in the order of its ISO 15924 code, then
  names with no letter. Inside a group, `Intl.Collator(<Hub language>)`
  with `numeric: true`.** A Hindi Hub shows Devanagari names first; an
  English Hub shows Latin names first. *Recommended.*
- (b) One `Intl.Collator` sort of all names. The browser's root order
  already keeps scripts together, but always puts Latin first, even in a
  Hindi Hub.

**Q7. What happens to the directory index of an existing node? (blocks
Step 5)**

*Owner answer, 2026-11-13:* (a). No automatic rebuild. The node owner runs `directory.reindex`, and the PR text says so (no migrations before release).

The `search_index` rows of a node built before B4b hold ASCII-folded
`text`. After B4b they must hold the raw text.

- (a) **Nothing automatic.** The product is unreleased (change doc rule 4,
  no migrations). On first use, `data_db` rebuilds the text index of
  `search_index` from the old rows (the key version changed), so ASCII
  searches still work. Listings in other scripts are found after the node
  owner runs `directory.reindex` (it exists: `crates/roym_directory/src/api.rs:60`)
  or publishes again. `directory.reindex` has no `roymctl` command and no
  Hub button. It is a local-only method, so the owner calls it over
  JSON-RPC with a signed-in owner session. The developer guide and the PR
  text give the exact call (D-B4b-13, P14). Test nodes, e2e and demo start
  empty. *Recommended.*
- (b) The directory stores a projection version and rebuilds
  `search_index` on the first call after an upgrade. Safer for a running
  node, but it is a migration mechanism that the project rule says not to
  build before release.

**Q8. Are author translations indexed by the directory? (blocks Step 5)**

*Owner answer, 2026-11-13:* (a). Yes. The directory indexes author translations, and the backlog row closes.

- (a) **Yes: the `text` field holds the title, summary and categories, and
  the title and summary of every author translation.** This closes the
  backlog row "Directory search does not index author translations"
  (`deferred-backlog.md:569`, target B4b). *Recommended.* Cost: the
  index text of one row can reach about 11 KB (title and summary, 160 +
  2,048 bytes, plus the same for each of up to 4 translations), and one
  listing has up to 8 rows (one per area), so about 88 KB of index text per
  listing at most. The PR text states this.
- (b) No. The row stays open.

### 2.2 Points found while reading

1. **The key is computed in the host, so no guest grows.** `data_db`
   computes the key of a declared text field
   ([text_index.rs:249](../../../../crates/data_db/src/sqlite/text_index.rs#L249))
   and of a `$search` query
   ([text_match.rs:55](../../../../crates/data_db/src/text_match.rs#L55)).
   The chat store is host code. No Roym guest calls `search_key` or
   `query_terms` today (`git grep -n 'search_key\|query_terms' crates/roym_*`
   finds nothing), and B4b keeps it so: the directory sends raw text, and
   `contacts.list` sends raw text. The tables add size to the substrate
   binary, not to a component. Section 7.6 still measures the six
   components.
2. **`normalize_text` has one caller.** `crates/roym_core/src/directory.rs:225`
   is used only by `crates/roym_directory/src/app/search_ops.rs` (import
   `:13`, write `:94`, query `:259`). Its tests are
   `directory.rs:335` and `:342`. B4b removes all of them.
3. **The `-32602` refusal for a non-Latin query has no test.** The text
   `"the directory's text index holds only ASCII letters, …"`
   (`search_ops.rs:266`) is in no test, Hub file or doc. The Hub shows any
   `-32602` through `error-invalid-input` (`ui/src/dom.ts:27`), so no Hub
   text changes when it goes away.
4. **Six places create `search_index`.** `ensure_coll(host, SEARCH_INDEX,
   &search_index_indexes())` at `search_ops.rs:176`, `:459`, `:525` and
   `publication_ops.rs:128`, `:180`, `:487`. `ensure_coll` always sends
   `text_fields: vec![]` (`crates/roym_directory/src/app.rs:113`). The
   backup path calls `ensure_coll(host, collection, &[])` for every
   collection (`backup.rs:56`); with B4a's rule "an empty list keeps the
   index" (change doc, "Decided 2026-11-11"), that call is harmless.
5. **A `$search` refusal reaches the directory as a string.** The
   directory pages with `paging::Pages::next_page`, which turns the host
   error into a `String` (`crates/roym_core/src/paging.rs:53`). Then
   `search` answers `-32603` (`search_ops.rs:203`). After B4b a query with
   more than 16 words (`text_match.rs:60`) would be an internal error. The
   directory's own limit is 256 bytes (`roym_core/src/directory.rs:30`),
   which can hold more than 16 words. D-B4b-12 keeps the error typed.
6. **The chat index is an external-content FTS5 table.**
   `messages_fts(body, content='messages', content_rowid='rowid',
   tokenize='trigram')` ([schema.rs:116](../../../../crates/conversation/src/store/schema.rs#L116)
   to `:121`). An external-content table reads its column values from
   `messages.body` for a `'delete'` and for `rebuild`, so it cannot hold a
   key that differs from the body. Its rows are written by hand, with no
   trigger: inserts at `store/message.rs:129`, `:185`, `:327`,
   `group.rs:429` and `store/backup.rs:442`; deletes (`'delete'` command)
   at `message.rs:357` and `:414`; `'optimize'` at `store/scrub.rs:100`.
   The content-type gate is `is_searchable_content_type`
   (`message.rs:27`). D-B4b-14 replaces the table.
7. **The chat store has no schema version.** `init_schema`
   (`schema.rs:46`) runs `CREATE … IF NOT EXISTS` only. A changed table
   definition does not reach an existing `conversation.db`. B3b's PR asked
   for a reset of an old database (B3b R5). B4b needs a rebuild path anyway
   for later key changes (D-B4b-17).
8. **Chat search errors become internal errors.** `host_impl.rs:411` maps
   every store error with `internal`. `from_host`
   (`crates/roym_conversation/src/app.rs:131`) maps
   `ConversationError::InvalidArgument(m)` to `-32602`. D-B4b-16 checks the
   term limit in `host_impl.rs` before the store call.
9. **`contacts.list` reads every row.** `contacts::list`
   (`crates/roym_profile/src/app/contacts.rs:67`) calls
   `paging::query_all(host, CONTACTS, None)` (`:78`), filters favourites in
   memory (`:82`) and applies `offset` and `limit` in memory (`:83` to
   `:88`). `ContactsListInput` (`crates/roym_core/src/method/profile.rs:28`)
   is `deny_unknown_fields` and derives `Default`. `roym_conversation`
   calls `contacts.list` with `ContactsListInput::default()`
   (`roym_conversation/src/app.rs:149`); `git grep -n 'ContactsListInput {'`
   finds only the definition. So a new `Option` field needs no other
   change.
10. **The B4a parity expectation changes.** The fixture
    (`test-components/dual-build-fixture/src/app/dispatch.rs:611`) indexes
    `नल वाला` (t1) and `சென்னை வாலா` (t4). With B4b keys both hold the word
    `vala`, so the queries `वाला` and `வாலா` each find t1 and t4. The test
    `both_builds_text_search_round_trip`
    (`crates/app_host_native/tests/dual_build_parity/data_layer.rs:133`)
    expects `[["t1"], ["t2"], ["t3"], ["t4"]]` today; it becomes
    `[["t1","t4"], ["t2"], ["t3"], ["t1","t4"], ["t1"]]` with a fifth query
    `nal` (Latin finds Devanagari). The change doc's last deviation says
    cross-script search is deferred to B4b; that entry is updated.
11. **B4a tests that depend on the NFC-only key.** The tests in
    `crates/data_db/src/sqlite/text_index/tests/search.rs` and
    `crates/text_lang/src/search_key/tests.rs` assert keys and matches.
    `search_key_case_and_idempotence` (`search_key/tests.rs:17`) and
    `query_terms_classification_and_cleaning` (`:47`) assert the old key
    text and "interior punctuation is kept"; D-B4b-6 changes both rules.
    `search_case_insensitive_and_substring` (`search.rs:110`) and
    `search_term_cleaning_finds_words` (`:180`) must be read again in
    Step 4: a test that only checks that rows are found keeps passing; a
    test that asserts a key string changes. These tests change for a known
    reason, and each update names it in the commit:
    - `search_finds_devanagari_and_tamil_words` (`search.rs:31`, the test
      ML-20 names): `வாலா` becomes `vala` and now finds the Devanagari row
      too. The expectation becomes both rows (cross-script search is the
      point of B4b).
    - `writes.rs:55` and `:68` run `MATCH 'apple'` on the word table; the
      key is now `aple`. `declare.rs:75` and `:84` run `MATCH 'world'` and
      `MATCH 'hello'`; the keys are `vorld` and `helo`. Raw-`MATCH` tests
      take the word from `fts::index_keys(...)`, so they stay right after a
      later key change.
    - `search_escapes_like_special_characters`
      (`conversation/src/store/tests/search_and_delete.rs:115`): `100%`
      becomes the long term `100`, a substring of `1000`, so both messages
      match. This is a real change (W1, W4, behaviour change 1). The test
      keeps its data, gets a new name and asserts the new result (section
      7.2, "Chat").
12. **NFC decomposes the Devanagari nukta letters.** `क़`, `ख़`, `ग़`, `ज़`,
    `ड़`, `ढ़`, `फ़`, `य़` (U+0958 to U+095F) are composition exclusions, so
    NFC turns each into the base letter plus U+093C NUKTA. The table must
    read base + nukta, not the single code points (D-B4b-3). Tamil two-part
    vowel signs (`ொ`, `ோ`, `ௌ`) are composed by NFC (U+0BCA to U+0BCC), so
    the table reads the composed signs.
13. **No Hub list is sorted, and no script helper exists.** No
    `localeCompare` or `Intl.Collator` in `crates/roym_web/ui/src`. The
    contact screen draws rows in service order
    (`screens/contacts.ts:38` to `:44`); the group member list draws
    `info.members` in order (`screens/groups.ts:527` to `:555`). The
    closest script code is `/\p{L}/u` in `i18n/scan.ts:39`. The Hub
    compiles to `ES2022` (`tsconfig.json:3`), so Unicode property escapes
    (`\p{Script=Devanagari}`) work.
14. **The directory help text describes the old folding.**
    `directory-search-help` (`en.ftl:667`, `hi.ftl:644`) says "Matching is
    on whole words … Punctuation is ignored, so searching for 50% finds
    50". After B4b a word matches anywhere inside the text, and spellings
    meet across scripts. Both catalogs change.
15. **The chat snippet looks for the query literally.** `snippetAround`
    (`ui/src/screens/message_search.ts:21` to `:28`) uses
    `toLowerCase().indexOf`. When a Latin query finds a Devanagari message,
    `indexOf` gives `-1` and the snippet starts at the beginning of the
    body (`Math.max(0, …)`). That is acceptable, and it is recorded
    (section 8).
16. **No e2e or demo step searches the directory by text.** Every
    directory search in `roym-hub.spec.ts` is by category; story 30 sends
    `{"text":"plumber"}` and checks only that the source answered
    (`demo/30-roym-client.sh:147` to `:149`). Two Rust tests send text:
    parity scenario 87 (`"HEDGE"`, `dual_build_parity/directory.rs:270`)
    and the contract test `directory_wire_unknown_fields_and_parameters`
    (`method_contract/contract_directory.rs:118`, text `"hedge"`). Both
    keep passing: `hedge` folds to `hedge` on both sides.
17. **The demo nodes have no directory set-up.** Story 30 reads Bob's
    directory DID itself (`demo/30-roym-client.sh:83`); story 20 sets no
    `*_DIRECTORY_DID` (`demo/20-roym-setup.sh:35`). Publishing a listing
    needs a SynOrg and a membership credential on the directory's node. The
    e2e suite does this in `ensureSynOrgWithListing`
    (`tests/roym-hub.spec.ts:514`), with English button names. D-B4b-22
    moves it into `hub-helpers.ts` with a language parameter.
18. **Capped test files.** `dual_build_parity/directory.rs` (981 lines)
    and `dual_build_parity/profile.rs` (1112) are at their caps in
    `xtask/oversized-test-files.txt`. New parity scenarios go into a new
    file, `dual_build_parity/search_scripts.rs`. The last scenario number
    is 230 (`git grep -ohE 'fn scenario_[0-9]+' crates/roym_web/tests/`).

## 3. What B4b meets

| Criterion | What B4b does | Test |
| --- | --- | --- |
| ML-20 | A directory search finds a listing by a word of its title in Devanagari and in Tamil script, on both builds. | parity `scenario_231_directory_finds_titles_in_every_script_parity`; `text_lang` table tests; the B4a storage-half tests stay |
| ML-21 | `nal wala` finds `नल वाला`, and `नल वाला` finds a `hi-Latn` listing titled `nal wala`, in the directory. | `text_lang` `required_spelling_rows_meet`; parity scenario 231; Playwright "a Latin search finds a Devanagari listing, and the other way"; story 40 |
| ML-22 | `naal waala` and `nalwala` find `nal wala`. The fold is a fixed table, tested with the 1,083 rows of `spelling-pairs.tsv`. | `text_lang` `spelling_rows_match_the_owner_decisions`, `required_spelling_rows_meet`, `fold_rules_table` |
| ML-23 | A contact name and a display name are stored as written (byte for byte, also a name sent in NFD form). `contacts.list` with `text` finds `राम` with `ram`, and `Ram` with `राम`. The Hub has no contact picker today (Q5), so the change doc's "contact picker compares keys" has nothing to change. | parity `scenario_234_contacts_list_text_finds_names_across_scripts_parity` (search and stored-as-written for contacts and for `profile.set`); `contacts.test.ts`; Playwright "contact search finds a name in another script" |
| ML-24 | The Hub sorts the contact list and the group member list with `Intl.Collator` for the Hub language, grouped by script (Q5, Q6). | `ui/src/names.test.ts`; `contacts.test.ts`; `groups.test.ts` |
| ML-29 | A chat query word of 1 or 2 characters (after folding) is a word or prefix match in the chat word table; the `LIKE` scan is gone, and the query plan never scans `messages`. The directory's short words go through B4a's word table. | `conversation` `store/tests/search_keys.rs` (`short_terms_use_the_word_table_and_never_scan`); parity `scenario_235_chat_search_short_and_cross_script_parity`; parity scenario 231 (a 2-letter word) |

**Where decisions and deviations are written.** Every criterion is in the
roym-multilingual change doc. Every B4b decision and deviation goes there
and only there, at close-out (Step 10): decisions into the section
"Search" as a new list "Decided <date>, search in every script", after the
B4a list (`change.md:408`), and into "Names in any script" for D-B4b-18 to
D-B4b-21; deviations under `## Deviations`. The deviations, if the owner
takes the recommendations: the tables start from ISO 15919 and differ
from it on purpose, so a stored key is not ISO output: `च` is written `ch`
and `ड़` is `d` (D-B4b-3, Q3); Hindi schwa deletion (`नल` is `nal`, ISO
`nala`); dental and retroflex letters are merged and long vowels lose the
macron, so `ā` meets `a`; `क़` is `q` and the fold makes it `k` (`kila`,
ISO `qilā`); Tamil `ழ` is `zh` and the fold makes it `l`; Tamil stops are
voiced by position (D-B4b-4, Q2); the fold runs to a fixed point
(D-B4b-5, P6); vowel quality is not folded, so English loanwords
in a native script do not meet their English spelling (Q1); only
Devanagari and Tamil have tables (Q4); punctuation breaks a word, so a
query word with inner punctuation becomes two terms (D-B4b-6, a change of
the B4a rule "interior punctuation is kept"); the FTS5 tokenizer strings
moved to `syneroym-text-lang` (D-B4b-8); the chat index is two ordinary
FTS5 tables, not an external-content table (D-B4b-14); an existing
directory index is not rebuilt automatically (Q7); the chat snippet starts
at the beginning of a cross-script hit (2.2 point 15). The test column of
ML-20 to ML-24 and ML-29 gets the real test names, checked with
`git grep` (B3a R2, B3b R3). The ML-20 and ML-29 "storage half" entries
stay. The last deviation ("Cross-script transliteration … is deferred to
B4b") is rewritten with the new parity result.

## 4. Behaviour-change audit

### Wire changes

No WIT package, WIT record, Roym record or method name changes.

| # | Where | Before | After |
| --- | --- | --- | --- |
| W1 | `directory.search` (and so `directory.find`) `text` | folded to `[a-z0-9 -]`; the whole folded text is one substring (`$regex` → `LIKE`); a query that folds to nothing is `-32602` "the directory's text index holds only ASCII letters, …" | every word must match, in any order, over search keys (D-B4b-11); a query in any script is accepted; a query with no word after the key (only punctuation) is `-32602` "$search needs at least one word"; more than 16 words is `-32602` "$search has more than 16 words" (D-B4b-12). The 256-byte limit is unchanged. A number is a substring like any word: `50%` becomes `50` and finds `500` and `5000` |
| W2 | `directory.search` results | a listing in Devanagari or Tamil is never found by text; author translations are not searched | it is found by a query in its own script or in Latin letters; author translations are searched (Q8) |
| W3 | `contacts.list` input | `favourites_only`, `offset`, `limit` | also `text: string?` (D-B4b-18). A wrong type is `-32602`, as for the other fields. Absent or blank means "every contact", as today. A text with no word after the key (`!!!`) is `-32602` "$search needs at least one word", the same answer as the directory and chat (P11) |
| W4 | `conversation.search` | 3 or more characters: the query is one case-insensitive substring of the raw body; fewer: a `LIKE` scan | every word must match over search keys; a long word anywhere in the joined key, a short word as a word or word prefix (D-B4b-15); `lo` no longer finds `hello`; `100%` finds `1000`; more than 16 words or more than 1,024 bytes is `-32602`; a query with no word after the key (only punctuation, for example `!!!`) is `-32602` "search needs at least one word" (D-B4b-16; before, `!!!` found a literal `!!!`) |
| W5 | data-layer `$search` (any guest) | key = NFC + lower case; words split on white space with inner punctuation kept | key of D-B4b-2 to D-B4b-6; punctuation breaks a word; the trigram table holds the joined key, so a long word can match across a word break (`lwa` finds `nal wala`) |
| W6 | `SEARCH_KEY_VERSION` | 1 | 2: every text index rebuilds on its next `create-collection` with text fields (B4a `declare`, `text_index.rs:87`) |

Old and new nodes: W1 changes what a stranger's directory answers, not the
shape. An old client half sends the same `SearchQuery`; an old directory
still refuses a non-Latin query with `-32602`, and the Hub already shows
that through `error-invalid-input`. Mixed versions keep working, with the
old matching on the old node. The PR text says this (B3b R5).

### Behaviour changes

| # | Before | After |
| --- | --- | --- |
| 1 | English text meets only its exact letters (case-folded); a number with a `%` or other sign is matched with the sign | the Latin fold also merges some English words (`sheet`, `seat`, `sit`) (Q1); punctuation breaks a word, so `50%` finds `500` (P5) |
| 2 | `search_index.text` holds folded ASCII | it holds the NFC raw text (title, summary, categories, translations); `data_db` keeps its key |
| 3 | `conversation.db` has `messages_fts` | it has `messages_tri`, `messages_word` and `text_index_version`; `messages_fts` is dropped on open (D-B4b-17) |
| 4 | Opening a `conversation.db` | also rebuilds the chat text index when its key version differs (D-B4b-17); one pass over the text messages, on the store's own thread |
| 5 | Hub contact list in service order (`person_did` order) | sorted and grouped by script (Q5, Q6); a search box above it |
| 6 | Hub group member list in `info.members` order | sorted and grouped by script; the owner row is not moved first (the badge shows it) |
| 7 | Hub directory help text: "whole words … Punctuation is ignored" | the new text of D-B4b-21, in English and Hindi |

### Stays the same

- Category, area, `open_to` and `booking_mode` filters of the directory;
  hit order (newest first); the hit and outcome shapes.
- `directory.find`'s client half and `roymctl roym directory find --text`
  (the text goes through unchanged, `apps/roymctl/src/commands/roym/find.rs:63`).
- Every other data-layer operator; B4a's limits (16 words, 1024 bytes, no
  NUL), table names, `$search` placement rules and `+c.id` ordering.
- `conversation.search` input and output shapes; its kind filter; its
  order (`sender_timestamp DESC, author DESC, id DESC`); which messages are
  searchable (text content types, not system, accepted or outgoing, not
  deleted).
- Every Roym record and signature. No search key is signed or sent between
  nodes.
- Every other Hub screen; the Hindi catalog except the new and changed
  messages of D-B4b-21.

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3). The ones
that depend on an owner answer say so.

- **D-B4b-1. Where the code lives.** `crates/text_lang/src/`:
  - `search_key.rs` (exists): `search_key`, `query_terms`, the constants.
  - `search_key/script.rs` (new): `Script` and the run splitter.
  - `search_key/devanagari.rs` (new): the Devanagari table.
  - `search_key/tamil.rs` (new): the Tamil table.
  - `search_key/fold.rs` (new): the Latin fold.
  - `fts.rs` (new, D-B4b-8), declared `pub mod fts;` in `lib.rs`.
  - Tests: `search_key/tests.rs` (exists) plus
    `search_key/tests/{devanagari,tamil,fold,spelling}.rs`, declared from
    `search_key/tests.rs` (AGENTS.md module layout).
  - Test data: `crates/text_lang/src/search_key/tests/spelling-pairs.tsv`,
    read with `include_str!` (D-B4b-9).

- **D-B4b-2. The key, step by step.** `search_key(text)`:

  ```text
  t = nfc(lowercase(text))                       // B4a, unchanged
  raw_words = []
  for each run of t split on white space, punctuation and symbols:
      // a "word" is a maximal run of chars that are alphanumeric or a
      // Unicode Mark (B4a's is_retained_boundary_char); everything else
      // is a break. ZWJ (U+200D) and ZWNJ (U+200C) are removed, not a break.
      for each script run inside the word (script.rs, rules below):
          Devanagari -> devanagari::to_latin(run)    // D-B4b-3
          Tamil      -> tamil::to_latin(run)         // D-B4b-4
          Latin      -> latin_base(run)              // rules below
          other      -> run unchanged (NFC, lower case)
      raw_words.push(concat of the run outputs, and the Tamil end marker of D-B4b-4)
  words = tamil::sandhi(raw_words)                 // needs the next word, so after the loop
  words = words.map(fold::fold_word)               // D-B4b-5, Latin letters only
  return the non-empty words, joined by " "
  ```

  - **Script runs** (`script.rs`). Each `char` has a script by its
    Unicode block: Devanagari U+0900 to U+097F, Tamil U+0B80 to U+0BFF,
    Latin = ASCII letters and the Latin-1, Latin Extended-A/B and Latin
    Extended Additional blocks. A combining mark (general category `M`) or
    a digit that is not in the Devanagari or Tamil block joins the run
    before it; at the start of a word it starts an "other" run. Every other
    `char` is "other". So `é` written as `e` + U+0301 stays one Latin run.
  - **`latin_base`**: NFD (`icu_normalizer` `DecomposingNormalizerBorrowed::new_nfd()`,
    already a dependency), then drop every `char` of general category `Mn`,
    then NFC again. `é`, `ñ`, `ā` become `e`, `n`, `a`. A Latin letter with
    no decomposition (`ø`, `æ`, `ß`, `ł`) is kept as it is; the fold does
    not change it.

  - Signature unchanged: `pub fn search_key(text: &str) -> String`.
  - Devanagari digits `०`–`९` and Tamil digits `௦`–`௯` become `0`–`9`.
  - Idempotent: `search_key(&search_key(s)) == search_key(s)` for every
    row of the spelling data (a test). The fold writes the `ch` sound as
    `ch` (not `c`), so a second pass reads it the same way.
  - A word in a script with no table (Q4) is kept as it is, so a Bengali
    title is found by a Bengali query.
  - A Latin word inside a Devanagari word (rare) is split into script
    runs and each run is converted; the outputs are joined into one word.

- **D-B4b-3. The Devanagari table** (`devanagari.rs`). A pure function
  over the `char`s of one Devanagari run (U+0900 to U+097F, after NFC).
  - Units: the run is read as a list of units. A unit is a consonant (with
    an optional nukta), followed by a vowel sign, a virama, or nothing
    (the inherent `a`); or an independent vowel; or a sign
    (anusvara `ं`, candrabindu `ँ`, visarga `ः`).
  - Letter values, ISO 15919 except where marked:

    | Kind | Values |
    | --- | --- |
    | independent vowels | `अ a, आ a, इ i, ई i, उ u, ऊ u, ऋ ri, ए e, ऐ ai, ओ o, औ au, ऑ o, ऍ e` |
    | vowel signs | `ा a, ि i, ी i, ु u, ू u, ृ ri, े e, ै ai, ो o, ौ au, ॉ o, ॅ e` |
    | consonants | `क k, ख kh, ग g, घ gh, ङ n, च ch*, छ chh*, ज j, झ jh, ञ n, ट t, ठ th, ड d, ढ dh, ण n, त t, थ th, द d, ध dh, न n, प p, फ ph, ब b, भ bh, म m, य y, र r, ल l, ळ l, व v, श sh, ष sh, स s, ह h` |
    | consonant + nukta | `क़ q, ख़ kh, ग़ g, ज़ z, ड़ d*, ढ़ dh*, फ़ f, य़ y` |
    | signs | anusvara `ं` and candrabindu `ँ` → `n`; visarga `ः` → `h`; nukta alone, virama alone → nothing |

    `*` differs from ISO 15919 (`c`, `ch`, `ṛ`, `ṛh`): the fold reads `ch`
    as the "ch" sound (D-B4b-5), and `ड़` is `d` (Q3 (a)). Retroflex and
    dental letters both map to plain `t`, `d`, `n`, and long vowels to
    plain `a`, `i`, `u` (ISO `ā`, `ī`, `ū`): the ISO dots and macrons are
    removed in the table itself, because the Latin side has none. So every
    value in the letter and sign tables is only the letters `a` to `z`.
    Chars with no value (below) and digits (D-B4b-2) are not table values.
  - Schwa deletion (Hindi), applied to the units of one word before they
    are written:
    1. The inherent `a` of the last unit is removed, when the word has two
       or more units with a vowel (`नल` → `nal`; `न` stays `na`).
    2. Then, from the second-to-last unit back to the second unit: an
       inherent `a` is removed when the unit before it has a vowel and the
       unit after it is a consonant with a vowel (`कमरा` → `kamra`,
       `बिजली` → `bijli`).
  - Letters that NFC keeps composed and that the table above lacks:
    `ऩ` (U+0929) → `n`, `ऱ` (U+0931, Marathi) → `r`, `ऴ` (U+0934) → `l`.
    Avagraha `ऽ` (U+093D) is dropped. `ॐ` → `om`. Any other Devanagari
    letter with no value (`ॠ`, `ऌ`, `ॲ` and the rest) is kept as it is, so it
    is found by the same letter and never breaks a word. A sign with no
    value (for example `ॄ`, U+0944, a mark of category `Mn`) is kept as it
    is in the same way, after the unit it follows.
  - Danda `।` and double danda `॥` are punctuation (a word break, D-B4b-2).
  - Test cases (from the data and from S0 R1): `नल वाला` → `nal vala`;
    `क़िला` → `kila`; `सड़क` → `sadak`; `दर्ज़ी` → `darji`; `बढ़ई` → `badhai`
    → after fold `badai`; a word with ZWJ or ZWNJ gives the same key as the
    word without it (the three rows of the data with note `zwnj`/`zwj`).

- **D-B4b-4. The Tamil table** (`tamil.rs`, Q2 (a)). A pure function over
  one Tamil run (U+0B80 to U+0BFF, after NFC).
  - Units as in Devanagari, with pulli `்` as the virama. A Tamil consonant
    with no sign has the vowel `a` (no schwa deletion: Tamil marks a dead
    consonant with pulli).
  - Letter values: vowels `அ a, ஆ a, இ i, ஈ i, உ u, ஊ u, எ e, ஏ e, ஐ ai,
    ஒ o, ஓ o, ஔ au` and the same as signs; consonants `ங n, ஞ n, ண n, ந n,
    ம m, ய y, ர r, ல l, வ v, ழ zh, ள l, ற r, ன n`; Grantha `ஜ j, ஷ sh, ஸ s,
    ஹ h, ஶ sh`; `ஃ` before `ப` → `f`, else `h`. As in Devanagari, long
    vowels lose the ISO macron (`ஆ` is `a`, ISO `ā`), so every value in the
    table is only the letters `a` to `z`. A Tamil letter or sign with no
    value is kept as it is, as in Devanagari.
  - The five stops `க`, `ச`, `ட`, `த`, `ப` take their form by position, for
    a unit that has a vowel:

    | Position | `க` | `ச` | `ட` | `த` | `ப` |
    | --- | --- | --- | --- | --- | --- |
    | first unit of the word | k | s | d | t | p |
    | after a vowel | g | s | d | dh | b |
    | after a nasal with pulli (`ங் ஞ் ண் ந் ம் ன்`) | g | j | d | d | b |
    | after the same stop with pulli (doubled) | k | ch | t | t | p |
    | after any other pulli consonant | k | s | t | t | p |

    A stop with pulli itself is written in its unvoiced form (`k`, `ch`,
    `t`, `t`, `p`).
  - Pairs: `ற்ற` → `tr`; `ன்ற` → `ndr`.
  - Word end: `-எய்` / `-ெய்` (`ey`) → `ai` (`எண்ணெய்` → `ennai`).
  - Loan clusters: a word that starts with `பி`, `டி` or `கி` followed
    by `ர`, `ள` or `ல` drops that `i` (`பிரியா` → `priya`, `டிரைவர்` →
    `draivar`); `தி` and `சி` are not changed (`திருச்சி` stays `tiruchi`).
  - Sandhi between words: a word that ends in a stop with pulli, followed by
    a word that starts with the same stop, drops that last consonant
    (`கறிக் கடை` → `kari kadai`). This needs the next word, so
    `tamil::to_latin` marks a word that ends in a stop with pulli, and
    `tamil::sandhi` runs over the list of words after the loop and before
    the fold (D-B4b-2 pseudocode). A Latin or Devanagari word is never
    changed by it.
  - Test cases: `முருகன்` → `murugan`; `மஞ்சள்` → `manjal`; `பச்சை` →
    `pachai`; `நன்றி` → `nandri`; `சென்னை` → `senai`; `ம்` → `m`.

- **D-B4b-5. The Latin fold** (`fold.rs`, Q1 (a)). `fold_word(w)` runs
  these rules in this order on the Latin letters of one word. Each rule is
  a plain string rewrite; there is no regex crate. `C` means any of
  `b c d g j k l m n p r s t v` (consonant letters that take a following
  `h` as aspiration).

  | # | Rule | Example |
  | --- | --- | --- |
  | 1 | none in the fold. `latin_base` (D-B4b-2) removes the marks of a Latin run, and every table value is only `a` to `z` (D-B4b-3, D-B4b-4). The number is kept so the other rules keep theirs | `cafè` → `cafe` by `latin_base` |
  | 2 | `x`→`ks`, `q`→`k` | `taxi` → `taksi` |
  | 3 | `sch`→`sk` | `school` → `skool` |
  | 4 | `ea`→`i` | `tea` → `ti` |
  | 5 | `chh`→`ch`; then `ch` is kept as one unit for rules 6 to 9 | `chhat` → `chat` |
  | 6 | `sh`→`s`, `zh`→`l`, `ph`→`f`, `ck`→`k` | `shankar` → `sankar` |
  | 7 | `c` before `e`, `i` or `y` → `s`; any other `c` (not in `ch`) → `k` | `cement` → `sement`, `doctor` → `doktor` |
  | 8 | `w`→`v`, `z`→`j` | `wala` → `vala`, `darzi` → `darji` |
  | 9 | `h` after a letter of `C` is removed (not the `h` of `ch`) | `thakkali` → `takkali` |
  | 10 | `ee`→`i`, `oo`→`u` | `naal` stays, `meen` → `min` |
  | 11 | a run of one letter becomes one letter | `naal waala` → `nal vala` |
  | 12 | `yi`→`i`; `y` at word end → `i`; `ay` not before a vowel → `ai` | `badhayi` → `badhai`, `trichy` → `trichi` |
  | 13 | `er` at word end → `ar`; `ei` or `ey` at word end → `ai`; `iy` before `a`, `e`, `o`, `u` → `i` | `teacher` → `tichar`, `kadei` → `kadai`, `takiya` → `takia` |

  - **The fold runs to a fixed point.** Rules 12 and 13 can make a new
    doubled letter or a new `ea` after rules 4 and 11 ran (`iyi` → `ii`;
    `kaey` → `kaai`). So `fold_word` runs the whole list again until the
    word does not change. There is no fixed limit on the passes.
  - **Why the passes end.** Give a word three numbers: (A) its count of
    `x` and `q`; (B) its length; (C) its count of `c`, `w`, `z`, `y` and
    `e`. Every rule that changes the word makes A smaller, or keeps A and
    makes B smaller, or keeps A and B and makes C smaller:
    - rule 2 removes an `x` or a `q` (A; the word gets longer, which is
      allowed because A comes first);
    - rules 3 (`sch`→`sk`), 4, 5, 6, 9, 10 and 11, `yi`→`i` in rule 12
      and `iy`→`i` in rule 13 make the word shorter (B);
    - rule 7, rule 8, the rest of rule 12 (`y`→`i`, `ay`→`ai`) and the
      rest of rule 13 (`er`→`ar`, `ei`/`ey`→`ai`) keep the length and
      replace a letter of `c w z y e` by a letter outside that set (C).

    No rule writes `x`, `q`, `w`, `z`, `y` or `e`, and only rule 5 writes a
    `c` (`chh`→`ch`, which makes the word shorter). So no rule makes an
    earlier number larger. Each pass that changes the word makes (A, B, C)
    smaller in this order, so the loop ends. The fold changes only the
    letters `a` to `z`, so a char outside them (a digit, a kept letter or
    mark, another script) cannot affect the end.
  - **How many passes in practice.** A plan-time search over every word of
    up to 10 letters over `a e i y`, up to 8 letters over `a e i y r c`,
    and up to 6 letters over `a e i o u y h c s z x` found no word that
    needs more than 3 changing passes. The longest is `aeiy` → `aeii` →
    `aai` → `ai`. So a fixed point costs at most a few passes, and the
    64 KiB measurement (section 7.2) covers the time.
  - This makes `search_key` idempotent (`fold_reaches_a_fixed_point`,
    section 7.2), and makes `kaey` and `kai` meet as the rules mean.
  - The fold runs on the output of the tables too, so a native word and its
    Latin spelling go through the same last steps.
  - Letters outside `a` to `z` (digits, other scripts) are not changed.
  - The rules are data in one `const` table where that is possible
    (`RULES: &[(&str, &str)]` for 2, 3, 4, 6, 8, 10), and small functions
    for the rules with a position (7, 9, 11, 12, 13). One test per rule
    (`fold_rules_table`, section 7.2).
  - If the owner picks Q1 (b) or (c), this table changes and Step 3 adds or
    removes rules; nothing else in the plan changes.

- **D-B4b-6. `query_terms` reads the key.** `query_terms(query)` becomes:
  split `search_key(query)` on its single spaces; drop duplicates; a word
  of `SHORT_TERM_MAX_CHARS` (2) or fewer `char`s is short, the rest long.
  B4a's trim of edge punctuation is no longer needed, because D-B4b-2
  already breaks words at punctuation: `plumber's` becomes the two terms
  `plumbar` and `s`, `50%` becomes `50`, `a-b` becomes `a` and `b`. This is
  a change of the B4a rule "interior punctuation is kept" (a deviation).
  The document side breaks words the same way, so both sides agree. The
  private helpers `is_retained_boundary_char` and `trim_term`
  (`search_key.rs:39`, `:50`) become the word splitter of D-B4b-2.
  `QueryTerms` keeps its shape. ML-29's "1 or 2 characters (after
  folding)" is measured on the key: `नल` has 2 letters but its key `nal`
  has 3, so it is a long term.

- **D-B4b-7. `SEARCH_KEY_VERSION = 2`.** The doc comment says it changes
  whenever `search_key`, `query_terms` or `fts::joined` gives a different
  output for some input. B4a's `declare` then rebuilds each text index on
  its next `create-collection` with text fields
  (`text_index.rs:110` to `:121`), and the chat store rebuilds its index on
  open (D-B4b-17). A test (`constants_match_specification`,
  `search_key/tests.rs:94`) asserts the value 2.

- **D-B4b-8. One shared FTS5 module** (`crates/text_lang/src/fts.rs`).
  Both `data_db` and the chat store need the same tokenizers, the same
  joined key and the same match syntax. One copy keeps them equal and keeps
  `cargo dupes` quiet (B4a R6, R9).

  ```rust
  /// The options of the trigram table, as written inside `fts5(…)`.
  /// The key is already lower case.
  pub const TRI_TABLE_OPTIONS: &str = "tokenize = 'trigram case_sensitive 1'";
  /// The options of the word table: tokenizer and prefix index.
  pub const WORD_TABLE_OPTIONS: &str =
      "tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\", prefix = '1 2'";
  /// Most query words one search may have.
  pub const MAX_SEARCH_TERMS: usize = 16;
  /// Longest search text, in bytes.
  pub const MAX_SEARCH_BYTES: usize = 1024;

  /// What the two tables index for one text.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct IndexKeys {
      /// For the word table: the key words, one space between them.
      pub words: String,
      /// For the trigram table: the key words with no space.
      pub joined: String,
  }
  pub fn index_keys(text: &str) -> IndexKeys;

  /// FTS5 MATCH texts for the two tables, for one column. `None` when the
  /// query has no term of that kind. Long terms: `col : "t1" AND col : "t2"`;
  /// short terms: `col : "t1"* AND …`. A `"` inside a term is doubled.
  pub fn match_exprs(column: &str, terms: &QueryTerms) -> (Option<String>, Option<String>);
  ```

  - The two constants keep the exact clause strings of
    `crates/data_db/src/sqlite/text_index.rs:13` to `:15`
    (`TRI_TOKENIZE`, `WORD_TOKENIZE`), under the new names, so no table
    definition changes. Their four users change their import:
    `declare` in `text_index.rs` and the three tests in
    `text_index/tests/sqlite_build.rs` (`:8`, `:34`, `:54`). The old names
    are removed from `data_db`, not re-exported.
  - `text_match.rs:66` to `:94` call `match_exprs` instead of building the
    strings. `MAX_SEARCH_TERMS` and `MAX_SEARCH_BYTES` move from
    `text_match.rs:13` and `:14`; no other file uses them
    (`git grep -n 'MAX_SEARCH_TERMS\|MAX_SEARCH_BYTES'`). Chat search
    checks the same two limits (D-B4b-16).
  - The module holds no SQL other than these strings, and no `rusqlite`
    dependency. A guest that does not call it does not grow (section 7.6).

- **D-B4b-9. The spelling test.** The owner-checked
  `spelling-pairs.tsv` is copied byte for byte into
  `crates/text_lang/src/search_key/tests/spelling-pairs.tsv` (code must not
  read `docs/planning/`). The planning copy stays as the owner's record.
  `spelling.rs` parses it with `include_str!` and runs:
  - `required_spelling_rows_meet`: `nal wala`, `naal waala`, `nalwala`
    find `नल वाला` and the other way; `ram` and `Ram` find `राम` and the
    other way. Hard assertions.
  - `spelling_rows_match_the_owner_decisions`: every `distinct` row stays
    apart, except the rows the owner marked "accepted merge" (Q1, Q3),
    listed in a `const ACCEPTED_MERGES: &[(&str, &str, &str)]` (lang, group,
    latin). Every `variant`, `joined` and `pair` row result is compared with
    a committed golden file `spelling-results.tsv` in the same folder: one
    line per row, `lang group kind latin key-of-latin key-of-native meets`.
    The test fails on any difference and prints the changed lines. This
    makes every change to a table or rule visible as a reviewed diff, which
    is the point of D-S0-3 ("output changes only in a reviewed change").
  - A floor: at least 160 of 200 Hindi pairs and 140 of 200 Tamil pairs
    meet in both directions (the plan-time model gave 166 and 142). If the
    Rust tables land below the floor, stop and tell the owner.
  - The golden file is written by the test itself when the environment
    variable `UPDATE_SPELLING_RESULTS=1` is set, and never otherwise. Its
    first comment line says so.
  - The matching rule in the test is the real one: `index_keys` and
    `query_terms`, a long term as a substring of `joined`, a short term as
    a prefix of one of `words` (the same as the FTS5 tables).

- **D-B4b-10. The directory text field.** In
  `crates/roym_directory/src/app.rs`, next to `ensure_coll` (`:106`):

  ```rust
  /// `search_index` with its text field. Every place that creates the
  /// collection calls this, so the text index exists before the first row.
  pub(in crate::app) async fn ensure_search_index<H: AppHost>(host: &H) -> Result<(), String>;
  pub(in crate::app) const SEARCH_TEXT_FIELDS: [&str; 1] = ["text"];
  ```

  It calls `create_collection` with `indexes: search_index_indexes()` and
  `text_fields: vec!["text".into()]`. The six calls of 2.2 point 4 use it.
  `ensure_coll` keeps `text_fields: vec![]` for every other collection.

- **D-B4b-11. The directory text and filter** (`search_ops.rs`, Q8 (a)).
  - `build_index_rows` (`:78`): `text` becomes `index_text(payload)`, a new
    private function that joins with one space the title, the summary, the
    categories, and the `title` and `summary` of each entry of
    `payload.translations`. It joins the raw strings and applies no
    normalisation: the guest does not call `nfc` or `search_key` (2.2
    point 1), and `data_db` makes the key. `normalize_text` is not called.
  - `validate_search_query` (`:236`): the `-32602` block (`:254` to `:270`)
    goes; the function returns `Result<(), Response>`. The comment above
    the function (`:229` to `:235`) loses the sentence about ASCII
    folding.
  - `build_search_filter` (`:274`): the text clause (`:288` to `:290`)
    becomes `json!({ "text": { "$search": text } })` when `query.text` is
    `Some` and not blank after `trim()`. A blank text adds no clause, as
    today.
  - `normalize_text` and its two tests are removed from
    `crates/roym_core/src/directory.rs` (`:219` to `:242`, `:334` to
    `:346`).

- **D-B4b-12. A `$search` refusal is `-32602`.** `paging::Pages` gains

  ```rust
  /// The next page, with the host's own error.
  pub async fn next_page_typed(&mut self) -> Result<Option<Vec<RecordReadValue>>, DataLayerError>;
  ```

  and `next_page` (`paging.rs:39`) calls it and maps the error to a string,
  so no other caller changes. `collect_search_candidates`
  (`search_ops.rs:322`) uses `next_page_typed` and returns a small enum
  `CandidateError { Refused(String), Host(String) }`; `search` answers
  `Refused` with `Response::invalid_params(m)` (`-32602`) and `Host` with
  `internal_error`, as today. `Refused` is
  `DataLayerError::SchemaViolation(m)`. Only `$search` can cause it here:
  every other clause is built by the directory itself.

- **D-B4b-13. No automatic rebuild of `search_index`** (Q7 (a)). After an
  upgrade, the first `ensure_search_index` declares the text field, and
  `data_db` builds the index from the existing rows. The `text` of rows
  written before B4b is ASCII-folded; it is still found by Latin queries.
  `directory.reindex` (`search_ops.rs:507`) rewrites every row with raw
  text. It has no `roymctl` command and no Hub button; the node owner calls
  it with the developer guide's `roym` shell function (the
  `roym-client=rpc` block, `docs/developer-guide.md:2415` to `:2421`),
  signed in as the owner:

  ```bash
  roym directory.reindex | jq .result     # {"rebuilt": <rows>, "standing": <n>}
  ```

  The PR text and the developer guide's search section give this exact
  call.

- **D-B4b-14. The chat text index** (new file
  `crates/conversation/src/store/text_index.rs`, declared in
  `crates/conversation/src/store.rs`):

  ```sql
  CREATE VIRTUAL TABLE IF NOT EXISTS messages_tri USING fts5(
      body, <fts::TRI_TABLE_OPTIONS>);
  CREATE VIRTUAL TABLE IF NOT EXISTS messages_word USING fts5(
      body, <fts::WORD_TABLE_OPTIONS>);
  CREATE TABLE IF NOT EXISTS text_index_version (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      key_version INTEGER NOT NULL);
  ```

  - Ordinary FTS5 tables (they keep the key), keyed by the `messages`
    `rowid`, as in `data_db` (B4a Q6). `messages_fts` goes. Each ordinary
    table also keeps a `_content` copy of the key. That is acceptable
    because `open_connection` sets `PRAGMA secure_delete = ON`
    (`schema.rs:254`), so a deleted index row is zeroed on disk, and every
    path that hides a message for good also removes its index rows (the
    `remove` bullet below). Rejected for now: `content='',
    contentless_delete=1` (SQLite 3.43 and later) would keep no copy at
    all, but its rows cannot be read back in a test, as B4a found.
  - Functions, each taking the open `Connection` (the callers already hold
    the store lock or a transaction):

    ```rust
    /// Replace the index rows of `rowid` with the keys of `body`.
    pub(super) fn sync(conn: &Connection, rowid: i64, body: &str) -> rusqlite::Result<()>;
    /// Remove the index rows of `rowid` (no error when there are none).
    pub(super) fn remove(conn: &Connection, rowid: i64) -> rusqlite::Result<()>;
    /// Create the tables; drop `messages_fts`; rebuild when the stored key
    /// version differs from `SEARCH_KEY_VERSION`.
    pub(super) fn ensure(conn: &Connection) -> rusqlite::Result<()>;
    ```

  - `sync` deletes both rows of `rowid`, then inserts
    `fts::index_keys(body)`: `joined` into `messages_tri`, `words` into
    `messages_word`. Delete-then-insert also replaces `INSERT OR IGNORE`
    (`message.rs:327`, `backup.rs:442`): a second `sync` of one row is
    harmless.
  - Every write site of 2.2 point 6 calls `sync` or `remove` instead of its
    own SQL: the five inserts call `sync(conn, rowid, body)`; the two
    `'delete'` commands call `remove(conn, rowid)`. `scrub.rs:100` runs
    `'optimize'` on both tables.
  - **`remove` has no guard.** Today the drop path removes the index row
    only when `prior == "accepted"` (`message.rs:350` to `:360`) and
    `delete_message_conn` only when `admission == "accepted"` and the body
    is not empty (`:406` to `:416`), because an FTS5 `'delete'` of text that
    was never indexed corrupts an external-content table (the comments at
    `:350` and `:406`). With ordinary tables, `DELETE … WHERE rowid = ?` of
    a missing row is safe. So both paths call `remove(conn, rowid)` for
    every dropped or deleted row, whatever its admission, and the two
    comments go. This also covers a row that a rebuild indexed by mistake.
  - **The live rule.** A row is in the index while it is visible: not a
    system row, a searchable content type, a UTF-8 body, and `admission =
    'accepted'` (outgoing rows have `'accepted'` by the column default,
    `schema.rs:92`; an incoming row gets it from `apply_admission(Accept)`,
    `message.rs:318` to `:330`), and not deleted. The insert sites keep
    their gates (`message.rs:124`, `group.rs:423`, the accept path, the
    backup import). The rebuild uses the same rule (D-B4b-17).
  - The body is the message bytes read as UTF-8; a body that is not UTF-8
    is not indexed (today it is cast to text). `String::from_utf8_lossy` is
    not used, so no replacement character enters the index.

- **D-B4b-15. Chat search** (`crates/conversation/src/store/search.rs`).
  `search(query, conversation, limit)` keeps its signature. It computes
  `query_terms(query)`; with no term it returns `Ok(vec![])` (the host
  refuses that case first, D-B4b-16, so this is only a guard). It gets the
  two MATCH texts from `fts::match_exprs("body", &terms)` and runs one
  query:

  ```sql
  SELECT <MESSAGE_COLUMNS with the m. prefix> FROM messages m
  WHERE <text clauses>
    AND m.system = 0 AND (m.outgoing = 1 OR m.admission = 'accepted')
    AND m.deleted_at IS NULL AND (?c IS NULL OR m.conversation_id = ?c)
  ORDER BY m.sender_timestamp DESC, m.author DESC, m.id DESC LIMIT ?l
  ```

  - `<text clauses>` holds only the clauses that exist, joined by `AND`,
    as `text_match.rs:96` does: `m.rowid IN (SELECT rowid FROM messages_tri
    WHERE messages_tri MATCH ?)` for the long terms and the same over
    `messages_word` for the short terms. Do not pass a `NULL` MATCH text:
    FTS5 refuses it. There is at least one clause.
  - The `LIKE` branch (`search.rs:46` to `:62`) and its comment go. The
    module doc (`search.rs:1` to `:2`) says "trigram and word tables over
    search keys".
  - The two `SELECT` lists become one. Use the `MESSAGE_COLUMNS` constant
    (`message.rs:21`) with an `m.` prefix made once (a `const` written out
    in full, so the order matches `row_to_message`; a test checks it).
  - Plan check: with and without `ANALYZE`, `EXPLAIN QUERY PLAN` shows
    `messages` read by rowid (`SEARCH m USING INTEGER PRIMARY KEY`), never
    `SCAN m` (ML-29). If SQLite chooses a scan on a tiny table after
    `ANALYZE` (as B4a found, change doc deviation R3), use
    `CROSS JOIN` order: `FROM (SELECT rowid AS r FROM messages_tri WHERE …)
    t CROSS JOIN messages m ON m.rowid = t.r`, which fixes the order. Write
    which form you used into this plan.

- **D-B4b-16. The limits of chat search.** `host_impl.rs:411` checks, in
  this order, before `spawn_blocking`: more than `fts::MAX_SEARCH_BYTES`
  bytes → `ConversationError::InvalidArgument("search text is longer than
  1024 bytes")`, checked before any key is built, so a 10 MB query costs
  nothing; then `query_terms(query)`: no term →
  `InvalidArgument("search needs at least one word")`; more than
  `fts::MAX_SEARCH_TERMS` → `InvalidArgument("search has more than 16
  words")`. `from_host` maps `InvalidArgument` to `-32602`. The texts are
  the `data_db` texts without the `$`, because a chat client never writes
  `$search`. `roym_conversation`'s own empty-query check
  (`app/messages.rs:493`) stays. So the directory, `contacts.list` and chat
  answer a query with no word in the same way: `-32602` (P11).

- **D-B4b-17. Rebuild of the chat index.** `init_schema` (`schema.rs:46`)
  calls `text_index::ensure(conn)` after the table DDL. `ensure`:

  ```text
  create messages_tri, messages_word, text_index_version if missing
  have = SELECT key_version FROM text_index_version WHERE id = 1   // none on a new or old db
  old_fts = messages_fts exists
  if have == SEARCH_KEY_VERSION and not old_fts: return
  BEGIN
  DROP TABLE IF EXISTS messages_fts
  DELETE FROM messages_tri; DELETE FROM messages_word
  for (rowid, content_type, body) in
      SELECT rowid, content_type, body FROM messages
      WHERE system = 0 AND deleted_at IS NULL AND admission = 'accepted':
      if is_searchable_content_type(content_type) and body is UTF-8: sync(rowid, body)
  upsert text_index_version (1, SEARCH_KEY_VERSION)
  COMMIT
  ```

  - This is a rebuild of derived data, not a migration: it reads only
    `messages`, which does not change. It uses the live rule of D-B4b-14,
    so an `undecided` or `held` message is not indexed (P1).
  - The `messages_fts` line in `schema.rs:116` to `:121` is removed. The
    file doc (`schema.rs:1` to `:5`) lists the new tables.
  - The rebuild runs while the store opens, before any caller. A test opens
    a database made with the old schema (it creates `messages_fts` by hand)
    and checks that search finds its messages after open.

- **D-B4b-18. `contacts.list` text** (W3).
  - `ContactsListInput` (`crates/roym_core/src/method/profile.rs:28`) gets
    `pub text: Option<String>` with `#[serde(default)]`.
  - `roym_profile`: a `ensure_contacts(host)` helper next to `ensure_coll`
    (`crates/roym_profile/src/app.rs:35`) creates `contacts` with the
    `favourite` index and `text_fields: vec!["display_name".into()]`.
    `contacts::list` (`contacts.rs:67`) calls it (it calls `ensure_coll`
    with the index today, `:69` to `:73`). Every other `ensure_coll(host,
    CONTACTS, &[])` keeps the index (empty list).
  - When `text` is `Some` and not blank, `list` passes the filter
    `{"display_name": {"$search": text}}` to `paging::query_all`
    (`:78`); the favourites filter, offset and limit stay in memory.
  - A `$search` refusal (too many words, no word) is `-32602`: use the same
    typed page reader as D-B4b-12 (`paging::query_all_typed`, a thin
    wrapper over `next_page_typed`).
  - A contact with no `display_name` is never found by text (it is shown by
    its DID in the Hub).

- **D-B4b-19. Hub name sorting** (new file `crates/roym_web/ui/src/names.ts`,
  Q5 (a), Q6 (a)).

  ```ts
  /** The ISO 15924 code of the first letter of `name`, or "Zzzz". */
  export function scriptOf(name: string): string;
  /** A copy of `items`, sorted by name for `lang`, grouped by script. */
  export function sortByName<T>(items: readonly T[], nameOf: (item: T) => string, lang: string): T[];
  ```

  - `scriptOf` tests the first `\p{L}` of the name against a fixed list of
    `\p{Script=…}` patterns: `Latn`, `Deva`, `Taml`, `Beng`, `Gujr`,
    `Guru`, `Knda`, `Mlym`, `Orya`, `Telu`, `Arab`, `Cyrl`, `Grek`, `Hani`,
    `Hira`, `Kana`, `Hang`. Any other letter is `Zyyy`; no letter is `Zzzz`.
  - Group order: the script of the Hub language
    (`new Intl.Locale(lang).maximize().script`, in `try`/`catch`, `Latn` on
    a throw; change doc rule 1 for `Intl.Locale`), then `Latn`, `Deva`,
    `Taml`, then the rest by code, `Zyyy`, and `Zzzz` last.
  - Inside a group: one cached `new Intl.Collator(lang, { numeric: true })`;
    ties by the original index (stable).
  - `lang` is the Hub's format locale from `i18n.ts` (the one `format.ts`
    uses), not a raw navigator value.

- **D-B4b-20. Hub contact search and lists.**
  - `screens/contacts.ts`: above the list (`:33` to `:46`), an input
    `.contacts-search-input` (placeholder `contacts-search-placeholder`) and
    a button `.contacts-search-button` (`contacts-search-button`,
    `type="button"`). Pressing the button or Enter reloads the list with
    `contacts.list` `{ text }` when the input is not blank, else `{}`. An
    empty result shows `contacts-search-empty`. Rows are drawn through
    `sortByName(contacts, c => c.display_name ?? "", lang)`; a contact
    with no name sorts in `Zzzz` and shows its DID, as today.
  - Each row's name gets no `lang` attribute (a contact name has no tag,
    change doc deviation "ML-5 covers record text …").
  - `screens/groups.ts`: the member loop (`:527`) iterates
    `sortByName(info.members, m => <the label of :529-:530>, lang)`.
  - The search uses the existing `refiller` (`contacts.ts:36`), so an old
    reply cannot overwrite a newer one (the e2e test 2b, `roym-hub.spec.ts:122`).
  - **The add form gets class names.** `addContact` finds its fields as
    `page.locator('.contacts-screen input')` with `.nth(0)` for the DID and
    `.nth(1)` for the address (`hub-helpers.ts:58` to `:60`). The new search
    input above the list would become `nth(0)` and break every test that
    adds a contact (P3). So the DID and address inputs of the add form
    (`contacts.ts:50` to `:95`) get the classes `.contacts-did-input` and
    `.contacts-address-input` (the envelope field `.contacts-envelope-input`),
    and `addContact` uses them. `contacts.test.ts` and
    `hindi-screens.test.ts` are checked for index locators in the same way.

- **D-B4b-21. Hub messages** (English and Hindi, in the `contacts-` and
  `directory-` sections of `en.ftl` and `hi.ftl`):

  | Id | English |
  | --- | --- |
  | `contacts-search-placeholder` | `name to look for` |
  | `contacts-search-button` | `Search` |
  | `contacts-search-empty` | `No contact has this name.` |
  | `directory-search-help` (changed) | `Every word must appear in the title, summary, categories or an author translation, in any order. A word in Latin letters also finds the same word written in Devanagari or Tamil, and common spellings meet: "nal wala" finds "naal waala". There is no ranking and no paid placement -- results are shown newest first.` |

  The agent writes the Hindi text, reusing the words of the existing Hindi
  messages (`खोजें` for the button, as `directory-search-button`). The
  owner reviews the four Hindi messages before merge, as in B1b; the PR
  text lists them. The Hindi sweep (`hindi-screens.test.ts`) draws the
  contact screen with a search result and with the empty message.

- **D-B4b-22. Story 40 and the e2e helpers.**
  - `demo/20-roym-setup.sh`: `demo_set "${upper}_DIRECTORY_DID" "$(did_of
    "$dir" "member-roym#directory-0")"` next to `:35`.
  - `hub-helpers.ts`: `ensureOwnSynOrg(page, directoryDid, category, lang)`
    and `publishListing(page, title, category, directoryDid, lang)`, moved
    from
    `ensureSynOrgWithListing` (`roym-hub.spec.ts:514` to `:568`).
    `roym-hub.spec.ts` calls them with `'en'`; its tests do not change
    otherwise. Every check uses a message id, never typed English (B1b P8):

    | Today (`roym-hub.spec.ts`) | After |
    | --- | --- |
    | button `'SynOrg'` | `hubMessage(lang, 'tab-synorg')` |
    | status contains `'runs no SynOrg'` | contains `hubMessage(lang, 'synorg-status-none')` |
    | button `'Save settings'` | `hubMessage(lang, 'synorg-save-button')` |
    | `.synorg-save-status` is `'Saved.'` | `hubMessage(lang, 'common-saved')` (`screens/synorg.ts:106`) |
    | button `'Save limit'` | `hubMessage(lang, 'synorg-limit-save')` |
    | `.limit-status` is `'Saved.'` | `hubMessage(lang, 'common-saved')` (`synorg.ts:159`) |
    | button `'Listings'` | `hubMessage(lang, 'tab-listings')` |
    | `.listing-save-result` contains `'Saved lst_'` | contains `lst_` (the id is data; `listing-saved` has arguments, `listings.ts:550`) |
    | `.publish-status` contains `'Published'` | contains `hubMessage(lang, 'listing-published')` (no arguments, `listings.ts:211`) |
    | button `'Add directory'`, status contains `'Added'` | `hubMessage(lang, 'directory-add-button')`; the status matches `directory-added`, or the part before the first `{` of `directory-added-note` or `directory-added-silent` (`screens/directory.ts:176` to `:196`) |

    The membership credential is issued over `rpcCall` as today, with the
    category the caller passes.
  - Story 40, a new last step "Alice's screen: a search in Latin letters
    finds her Devanagari listing". A tab click draws its screen again
    (`show(render)`, `ui/src/main.ts:172`), and the chat step before this
    one leaves the Listings tab, so the draft of the B2 and B3a steps (the
    `gardening` chip and the `Garden work` translation) is gone when this
    step starts. The step does not depend on it: it calls `ensureOwnSynOrg`
    with `ALICE_DIRECTORY_DID` and `gardening`, then `publishListing` with
    the title `नल वाला ${runTag}` and `gardening` on a new draft (the text
    language select starts at `hi` in a Hindi Hub, as the B3a step checks).
    Then in the Directory tab it adds her own
    directory as a source, types `nal wala ${runTag}` and presses Search
    (`hubMessage('hi', 'directory-search-button')`). Expect exactly one
    `.search-hit` with the Devanagari title. The `runTag` (base-36 digits
    and letters) keeps the story repeatable on the same nodes; it folds the
    same on both sides.
  - Playwright, in `roym-hub.spec.ts` (English Hub):
    - "a Latin search finds a Devanagari listing, and the other way":
      publish `नल वाला <tag>` and `nal wala <tag2>` (the second tagged
      `hi-Latn` in the text language select); search `nal wala <tag>` →
      the Devanagari listing; search `नल वाला <tag2>` → the Latin one.
    - "contact search finds a name in another script". The Hub form sends
      no display name for a plain add (`upsertParams`, `screens/contacts.ts:5`),
      and the name comes from a verified profile envelope or from the API
      field `display_name` (`roym_profile/src/app/contacts.rs:155`, `:180`).
      So the test calls `contacts.upsert` with `display_name` through the
      existing `rpcCall` helper (`hub-helpers.ts:23`; it already calls
      local-only methods, `roym-hub.spec.ts:533`), for `राम` and `Ram`, each
      with its own `person_did` and address (with a run tag, so the test can
      run twice on one node). Then it opens Contacts, searches `ram` → both
      rows; searches `राम` → both rows; with an empty search the `Ram` row
      comes before the `राम` row (Latin first in an English Hub).
  - Before you add a row or a control: `git grep -n "\.first()\|nth(\|screen input" --
    crates/substrate/tests/e2e` and check each locator on the contact,
    group and directory screens (B3a P4). One is known: `addContact` counts
    the inputs of `.contacts-screen` (D-B4b-20 gives them class names).

## 6. Step-by-step changes

Each step builds and its tests pass. Rust: `cargo nextest run -p <crate>`
for the crates the step touches, output to a file (AGENTS.md "Context
Budget"). Paths are from the repository root.

### Step 1 — Commit this plan

Done in the planning session: this file and the B4b row of
[status.md](status.md).

### Step 2 — Owner check of the spelling data

1. The owner answers Q1 to Q3. Write each answer under its question here.
2. The owner (or the agent, with the owner's answers) edits only the
   header of `docs/planning/milestones/M08B-language-and-structure/spelling-pairs.tsv`:
   each line of the `# conflicts:` block gets `owner decision: <side>`
   from Q1 (a); the rows `hi/jharu distinct jaru`, `hi/saree distinct
   sadi` and `ta/ennai distinct enna` are listed under a new
   `# accepted merges:` line with the reason; line 4 becomes
   `# owner check: done <date>`. No data row changes.
3. Commit (`docs(planning): owner check of the spelling data`).
4. **Stop if the owner picks Q1 (b) or (c), or Q2 (b):** change D-B4b-5 or
   D-B4b-4 in this plan first.

### Step 3 — Tables and fold in `syneroym-text-lang`

1. `search_key/script.rs`, `devanagari.rs`, `tamil.rs`, `fold.rs`
   (D-B4b-2 to D-B4b-5); `search_key.rs`: the new `search_key`,
   `query_terms` (D-B4b-6) and `SEARCH_KEY_VERSION = 2` (D-B4b-7).
2. `fts.rs` (D-B4b-8) and `lib.rs` `pub mod fts;`.
3. The test data copy and `spelling-results.tsv` (D-B4b-9): run once with
   `UPDATE_SPELLING_RESULTS=1`, read the file, and check the floor.
4. Tests of section 7.2 ("text_lang").
5. `cargo nextest run -p syneroym-text-lang`.

At this point `data_db` still builds (same function names) but its tests
that assert key text may fail; Step 4 fixes them. Run Steps 3 and 4 before
the first commit that includes Step 3, or commit them together.

### Step 4 — `data_db` on the new key

1. `text_index.rs`: import `TRI_TABLE_OPTIONS`, `WORD_TABLE_OPTIONS` from
   `syneroym_text_lang::fts` (remove `TRI_TOKENIZE` and `WORD_TOKENIZE`,
   `:13` to `:15`, and change their users: `declare` and
   `text_index/tests/sqlite_build.rs:8`, `:34`, `:54`); `sync_row` (`:226`)
   uses `fts::index_keys(value)`: `joined` for `_tri`, `words` for
   `_word`. The two tables now get different values, so `params_vec`
   (`:263`) becomes two vectors.
2. `text_match.rs`: remove `MAX_SEARCH_TERMS` and `MAX_SEARCH_BYTES`
   (`:13`, `:14`) and use `fts::MAX_SEARCH_TERMS` and
   `fts::MAX_SEARCH_BYTES`; build the expressions with `fts::match_exprs`
   (D-B4b-8).
3. Read every test of `text_index/tests/` (2.2 point 11). Update only the
   assertions on key text and on inner punctuation; add the tests of
   section 7.2 ("data_db").
4. `test-components/dual-build-fixture/src/app/dispatch.rs:611`: add the
   fifth query `{"title": {"$search": "nal"}}`;
   `crates/app_host_native/tests/dual_build_parity/data_layer.rs:137`:
   the new expectation of 2.2 point 10.
5. `mise run build:test-components`, then
   `cargo nextest run -p syneroym-data-db -p syneroym-text-lang` and
   `cargo nextest run -p syneroym-app-host-native --test dual_build_parity`.

### Step 5 — The directory on `$search`

1. `roym_core/src/paging.rs`: `next_page_typed` and `query_all_typed`
   (D-B4b-12, D-B4b-18).
2. `roym_directory/src/app.rs`: `ensure_search_index` (D-B4b-10); the six
   call sites.
3. `search_ops.rs`: `index_text`, the filter, the validation and the typed
   candidates (D-B4b-11, D-B4b-12). `roym_core/src/directory.rs`: remove
   `normalize_text` and its tests.
4. New parity file `crates/roym_web/tests/dual_build_parity/search_scripts.rs`
   (declared in `crates/roym_web/tests/dual_build_parity.rs`), scenarios
   231 to 233 (section 7.2).
5. `mise run build:roym`, then `cargo nextest run -p syneroym-roym-core
   -p syneroym-roym-directory -p syneroym-roym-federated-query` and
   `cargo nextest run -p syneroym-roym-web --test dual_build_parity --test
   method_contract`, and `cargo nextest run -p syneroym-substrate --test
   roym_directory_e2e`.

### Step 6 — Chat search on search keys

1. `crates/conversation/src/store/text_index.rs` (D-B4b-14, D-B4b-17);
   `store.rs` declares it; `schema.rs` calls `ensure` and loses
   `messages_fts`.
2. The seven write sites and `scrub.rs:100` (2.2 point 6).
3. `search.rs` (D-B4b-15); `host_impl.rs:411` (D-B4b-16).
4. Tests of section 7.2 ("conversation"); update the four tests in
   `store/tests/search_and_delete.rs` and `scrub.rs:92` that read
   `messages_fts` by name (`git grep -n messages_fts crates/conversation`
   must find nothing after this step).
5. `cargo nextest run -p syneroym-conversation -p syneroym-roym-conversation`,
   then `mise run build:roym` and the parity scenario 235 in
   `search_scripts.rs`; `cargo nextest run -p syneroym-substrate --test
   roym_conversation_e2e`.

### Step 7 — `contacts.list` text

1. `ContactsListInput.text` (D-B4b-18).
2. `roym_profile`: `ensure_contacts` and the filter in `contacts::list`.
3. Parity scenario 234 and the contract test (section 7.2).
4. `mise run build:roym`, then `cargo nextest run -p syneroym-roym-core
   -p syneroym-roym-profile -p syneroym-roym-conversation` and
   `cargo nextest run -p syneroym-roym-web --test dual_build_parity --test
   method_contract`.

### Step 8 — Hub

1. `src/names.ts` and `names.test.ts` (D-B4b-19).
2. `screens/contacts.ts`, `screens/groups.ts` (D-B4b-20) and their tests.
3. `en.ftl`, `hi.ftl` (D-B4b-21); the Hindi sweep fixtures in
   `hindi-screens.test.ts` (contacts `:124`, stub `:126`).
4. `npm test` and `npm run build` in `crates/roym_web/ui` (the visible-text
   check runs in the build).

### Step 9 — e2e and story 40

1. `demo/20-roym-setup.sh`, `hub-helpers.ts`, `roym-hub.spec.ts` and story
   40 (D-B4b-22).
2. `mise run test:e2e` (redirected, AGENTS.md) and `mise run demo`.

### Step 10 — Sizes, docs and close-out

1. Component size (section 7.6) and the Hub size (section 7.5). Write them
   into task.md (a B4b result line, in the form of the B4a line).
2. ADR-0007: a dated amendment (D-B4b-23).
3. Change doc: decisions, deviations and test columns (section 3). In
   "Search", the bullet "The transliteration table" says "Our tables give
   the ISO form directly (`kzilā` …, `cheṉṉai` …, `ṁ` …)"; after B4b that is
   false. Change that sentence to "Our tables start from ISO 15919 and
   differ from it where search needs it (see Deviations)"; keep the survey
   part of the bullet as it is. `demo:` stays story 40; `status` stays `draft`; add any file of section
   9 that is not in `living-docs-touched`.
4. Living docs (section 9), deferred backlog (section 8), status.md.
5. The import clean-up pass over every Rust file you edited.
6. `cargo dupes report` over the changed Rust files (AGENTS.md recipe):
   one tokenizer pair, one match builder, one joined-key function.
7. `mise run verify` with no `--skip`, as a background job.
8. PR text: section 10's checklist.

- **D-B4b-23. ADR-0007 amendment.** A new dated entry in `## Amendments`
  (after `:244`): the search key of a text field now also transliterates
  Devanagari and Tamil and folds Latin spellings (`syneroym-text-lang`,
  version 2); the trigram table holds the key with spaces removed; a word
  break is any character that is not a letter, a digit or a mark. The
  2026-11-11 text stays as written (ADRs are not rewritten).

## 7. Tests

### 7.1 Hub unit tests

- `src/names.test.ts`: `scriptOf` for a Latin, a Devanagari, a Tamil, a
  Bengali and a Han name, a name that starts with a digit then a letter
  (`9 Ram` is `Latn`), and a name with no letter (`Zzzz`); `sortByName`
  in `en`: `["राम", "Zoe", "anil", "முருகன்", ""]` →
  `["anil", "Zoe", "राम", "முருகன்", ""]`; in `hi`: Devanagari first;
  `numeric: true` (`Shop 2` before `Shop 10`); an unknown `lang` (`xx-Zzzz`
  that `Intl.Locale` accepts, and one it rejects) does not throw; stable
  order of equal names.
- `screens/contacts.test.ts` (today it tests only `upsertParams`): the list
  is drawn sorted; the search box sends `contacts.list` with `{ text: "ram" }`
  (assert the exact params through `stub-rpc.ts`); a blank box sends `{}`;
  an empty answer shows `contacts-search-empty`; the button has
  `type="button"`.
- `screens/groups.test.ts`: the member list is drawn in `sortByName` order
  (one test with a Latin and a Devanagari member).
- `i18n/hindi-screens.test.ts`: the contact screen with a search result and
  with the empty message; no Latin word left.
- `catalog.test.ts` and the visible-text check run unchanged and must pass
  with the new ids.

### 7.2 Rust

**text_lang** (`crates/text_lang/src/search_key/tests*`)

- `devanagari.rs`: one test per row of the D-B4b-3 tables (a table-driven
  test over every letter, sign and nukta form, by code point, so no letter
  of U+0900 to U+097F that the table maps is untested); schwa deletion
  (`नल`, `न`, `कमरा`, `बिजली`, `राज मिस्त्री`); `ऩ`, `ऱ`, `ऴ`, avagraha, `ॐ`
  and one unmapped letter (`ॠ` kept as it is); the nukta forms after NFC
  (`क़िला` written as U+0958 and as `क` + U+093C give the same key); ZWJ and
  ZWNJ are removed; danda is a break; Devanagari digits.
- `tamil.rs`: every row of the stop table of D-B4b-4 (one word per cell);
  `ற்ற`, `ன்ற`, the `-ey` end, the loan clusters, sandhi
  (`கறிக் கடை` → `kari kadai`), `ஃப`; Tamil digits; the composed two-part
  vowel signs written decomposed give the same key.
- `fold.rs`: `fold_rules_table` (one input/output pair per rule 2 to 13 of
  D-B4b-5, plus one case where a rule must not fire: `ch` keeps its `h` in
  rule 9; `c` in `ch` is not rule 7); `fold_reaches_a_fixed_point`:
  `fold_word(fold_word(w)) == fold_word(w)` for every string of 1 to 5
  letters over `a e i y h c k s r` (about 66,000 strings) and every string
  of 1 to 4 letters over `a e i o u y h c k s r w z x` (about 41,000
  strings; no new dependency). This checks idempotence; the proof that the
  loop ends is in D-B4b-5. Fixed cases: `aeiy` → `ai` (3 changing
  passes), and `kaey` and `kai` give the same word (P6).
- `devanagari.rs` and `tamil.rs`: every value of the letter and sign
  tables has only the letters `a` to `z` (a test over the table entries,
  not over every output of `to_latin`, which keeps chars with no value and
  writes digits); a sign with no value (`ॄ`) is kept as it is.
- `search_key/tests.rs`: `search_key_is_idempotent` over every Latin and
  native value of the spelling data; `search_key_breaks_words_at_punctuation`
  (`plumber's` → `plumbar s`, `50%` → `50`, `a-b` → `a b`); mixed script
  words; a 1 KiB and a 64 KiB input of each script give a key with no
  panic (no wall-clock bound in a normal test; the time of a 64 KiB key
  is measured in an `#[ignore]` test, `measure_search_key_64_kib`, and the
  number goes into the PR text); a NUL inside the text is a break (B4a P5);
  one row for each rule of D-B4b-2 on script runs and `latin_base`: `é`
  written as `e` + U+0301 is one Latin run and gives `e`; `ø` stays `ø`; a
  digit after a Devanagari letter stays in its word;
  `query_terms` classification on the key (`नल` long, `एक` short, `ek`
  short); `constants_match_specification` with version 2.
- `spelling.rs`: the three tests of D-B4b-9.
- `fts.rs` tests (`crates/text_lang/src/fts/tests.rs`): `index_keys`
  (`nal wala` → words `nal vala`, joined `nalvala`); `match_exprs` with
  only long, only short, both, and a term with `"` (doubled).

**data_db** (`crates/data_db/src/sqlite/text_index/tests/`)

- `search.rs`: `search_finds_across_scripts` (a row `नल वाला` is found by
  `nal wala`, `naal waala`, `nalwala`, `नल`; a row `nal wala` is found by
  `नल वाला`); `search_joined_key_matches_across_word_breaks` (`nalwala`
  finds `nal wala`; `lwa` finds it too); `search_punctuation_breaks_words`
  (`plumber's` finds `plumber's shop`; a query of only `%%` is refused
  with "needs at least one word").
- `declare.rs`: `declare_key_version_mismatch_rebuilds` exists (`:214`);
  add an assertion that after a rebuild the stored keys are version-2 keys
  (a Devanagari row is found by a Latin query).
- The existing tests pass after the assertion updates of Step 4.3, and the
  count of tests in the crate does not go down.

**Directory** (new file `crates/roym_web/tests/dual_build_parity/search_scripts.rs`)

- `scenario_231_directory_finds_titles_in_every_script_parity`: publish
  three listings, `नल वाला` (`lang: hi`), `nal wala` (`lang: hi-Latn`)
  and `சென்னை பிளம்பர்` (`lang: ta`), with the existing fixtures
  (`ensure_synorg`, `enrol_signing`, `full_listing_params`,
  `publish_signed_listing`, `fixtures.rs:88`). `full_listing_params` sets
  `"lang": "en"`, the summary `Neat hedges, fortnightly.` and the
  categories `gardening, outdoor` for every listing (`fixtures.rs:88` to
  `:93`), so each listing overrides `lang`, and no query below uses a word
  of that shared text. Queries and the exact
  `listing_id` sets: `nal wala` → the first two; `नल वाला` → the first
  two; `naal waala` → the first two; `பிளம்பர்` → the third; `senai`
  (the key of `சென்னை`) → the third; `ek` (a short term) → none; a fifth
  listing `एक नल` (`lang: hi`) → `ek` finds only it. Both builds give the same ids (`assert_eq!` of the two
  answers, not only of the counts; B3b R7).
- `scenario_232_directory_accepts_a_query_in_any_script_parity`: a Kannada
  query (`ನಲ್ಲಿ`) answers `result.hits = []`, no error (before B4b it was
  `-32602`).
- `scenario_233_directory_refuses_more_than_16_words_parity`: 17 words
  that stay distinct after the key (`w1 w2 … w17`; `query_terms` drops
  duplicates, and words like `wala`/`vala` meet) → `-32602` with "more than
  16 words" on both builds; 16 such words are accepted; a text of only
  `!!!` → `-32602` "needs at least one word".
- A translation: in scenario 231, a fourth listing in `hi` with an `en`
  author translation titled `Lawn mowing`: `lawn` finds it and no other
  listing (Q8). Not `garden`: every fixture listing has the category
  `gardening`, so `garden` would find them all (P7).
- `roym_directory` unit tests: `index_text` joins title, summary,
  categories and translations; `build_search_filter` has a `$search` clause
  and no `$regex` on `text`.

**Chat** (`crates/conversation/src/store/tests/search_keys.rs`, new,
declared in `store/tests.rs`)

- `short_terms_use_the_word_table_and_never_scan`: messages `ek do teen`,
  `hello` and `एक बात`; `ek` finds the first and the third; `lo` finds
  nothing; `EXPLAIN QUERY PLAN` of the search SQL, with and without
  `ANALYZE` on 150 messages, has no `SCAN m` (ML-29).
- `latin_query_finds_devanagari_message` and the other way.
- `old_messages_fts_database_is_rebuilt_on_open`: create `messages` and
  `messages_fts` with the old DDL and two rows, close, open the store,
  search finds both; `messages_fts` no longer exists.
- `key_version_change_rebuilds`: set `text_index_version.key_version` to 1
  by hand, reopen, search still finds the rows (the index was rebuilt).
- `sync_twice_keeps_one_row`, `remove_without_row_is_ok`,
  `non_utf8_body_is_not_indexed`.
- `rebuild_indexes_only_visible_messages` (P1): an outgoing message, an
  accepted incoming one, a `held` and an `undecided` incoming one; set the
  key version to 1 and reopen; the index has rows for the first two only
  (`SELECT rowid FROM messages_tri`).
- `held_message_dropped_after_a_rebuild_leaves_no_key_on_disk` (in
  `store/tests/scrub.rs`, P1): a held message, a rebuild as above, a drop,
  a scrub; the files contain neither the body nor its key needles (next
  bullet).
- The scrub tests (`store/tests/scrub.rs`, `deleted_text_is_gone_from_the_files_after_a_scrub`,
  `dropped_text_is_gone_from_the_files_after_a_scrub`, P2): today they look
  for `zqxjvk` and `qxj` (`:12` to `:13`). After B4b the index holds
  `search_key("zqxjvk")` (`jksjvk`), so those needles are never in the
  index and the tests would pass even if the index kept the deleted key.
  Each test also looks for `fts::index_keys(WORD).joined` and one trigram
  of it (both computed in the test, so a later key change keeps it right),
  and asserts that they are on disk before the delete, as it already does
  for the body.
- `host_impl` tests (P11): a text of 1,025 bytes → `InvalidArgument`, and
  no key is built (a 10 MB text returns at once); `!!!` →
  `InvalidArgument("search needs at least one word")`.
- The existing tests of 2.2 point 6 (`search_and_delete.rs:6`, `:76`,
  `:115`; `scrub.rs:92`; `backup_roundtrip.rs:208`; `group/tests.rs:707`,
  `:738`) keep their names and pass, except one.
  `search_escapes_like_special_characters` (`:115`) is renamed
  `search_treats_like_characters_as_breaks`. It keeps its two messages
  (`m:pct` with `100% off`, `m:other` with `1000 off`). It asserts that
  `100%` finds both `m:pct` and `m:other` (compared as a set), and that
  `100_` finds the same two (`_` is a break too, so both queries are the
  long term `100`). This is the test of the number cost of W4 and
  behaviour change 1.
- `host_impl` test: 17 distinct words → `InvalidArgument`.
- Parity `scenario_235_chat_search_short_and_cross_script_parity` (in
  `search_scripts.rs`): send `ek do teen` and `एक बात`; `conversation.search`
  `{query: "ek"}` → both; `{query: "बात"}` → the second; the same on both
  builds.

**Contacts**

- Parity `scenario_234_contacts_list_text_finds_names_across_scripts_parity`:
  upsert `राम`, `Ram` and `Sita`; `{text: "ram"}` → `राम` and `Ram`;
  `{text: "राम"}` → the same two; `{text: "sita", favourites_only: true}`
  → none when Sita is not a favourite; `{text: " "}` → all three;
  `{text: "!!!"}` → `-32602`; `{text: 5}` → `-32602`. Stored as written
  (P13): `contacts.list` returns each `display_name` byte for byte as sent,
  also a name that NFC would change (`क़` written as the one code point
  U+0958, which NFC decomposes, and `é` written as `e` + U+0301, which NFC
  composes); `profile.set` with `display_name: "राम"` and with the U+0301
  name returns the same bytes from `profile.get`. No such test exists today
  (`git grep -nP 'display_name.*[^\x00-\x7F]' -- '*.rs'` finds nothing).
- `method_contract/contract.rs` (602 lines, not capped): one test that
  `contacts.list` accepts `text` and still refuses an unknown field with
  `data.field` (next to `:225` to `:252`).
- `method_contract/shapes.rs:185` and `:208` keep their golden shapes
  (the output does not change).

### 7.3 Playwright

- `roym-hub.spec.ts`: the two tests of D-B4b-22.
- Story 40: the new step.
- The existing tests 2b, 2c, 2d, 6 and 40 (`roym-hub.spec.ts:122`, `:144`,
  `:177`, `:284`, `:1278`) and the directory tests (`:599` to `:1403`) pass
  unchanged. Test 40 (`chat search … searchable<digits>`) still finds its
  message: `searchable` + digits folds the same on both sides.

### 7.4 Not tested

- Real phones' keyboards and transliteration input methods: the spelling
  data stands for them.
- Collation in every browser: the tests run in jsdom and Chromium only.
- Search speed on a large chat store: `ensure` reads every text message
  once on the first open after B4b; one measurement with 10,000 messages
  (an `#[ignore]` test, as B4a R4 did) gives the number for the PR text.

### 7.5 Size risks

| File or function | Now | Risk |
| --- | --- | --- |
| `crates/conversation/src/store/message.rs` | 743 lines | the five index sites become one call each; net fewer lines. Must stay under 800 |
| `crates/conversation/src/group.rs` | 742 lines | one site becomes one call |
| `crates/roym_directory/src/app/search_ops.rs` | 554 lines | `index_text` +15, the refusal −17 |
| `build_index_rows` (`search_ops.rs:78` to `:160`) | 83 lines | `text` becomes one call; no growth |
| `crates/text_lang/src/search_key/devanagari.rs`, `tamil.rs` | new | the tables are data; keep each file under 400 lines; split the letter table into a `const` array of `(char, &str)` |
| `crates/roym_web/tests/dual_build_parity/directory.rs`, `profile.rs` | at cap | not touched (new file `search_scripts.rs`) |
| `roym-hub.spec.ts` | 1465 lines (TypeScript is not under the file-length check) | +80; acceptable |
| Hub main script (`gzip -9`) | 52,297 bytes after B3b (B4a changed no Hub file) | `names.ts` and the contact box: expect under +1 KB; limit +5 KB (the B3b limit) |
| Hindi catalog (`gzip -9`) | 14,447 bytes | four messages: expect under +400 bytes; limit +1 KB |

### 7.6 Component size

Build the six Roym components (`mise run build:roym`) on `main` and on the
branch, in the same build folder, and write before and after into task.md.
Expected: no component grows by more than a few KB, because no guest calls
`search_key`, `query_terms` or `fts` (2.2 point 1). `directory` and
`profile` change their own code a little (`index_text`, the filter, the
typed page). Limits (task.md section 1): +40 KB for a component with no new
record code (all six here: B4b adds no record field). If one grows by more
than 40 KB, a guest links the tables: find the call and remove it, or stop
and tell the owner.

### 7.7 Hindi text

Four messages (D-B4b-21). The owner reviews them before merge. The Hindi
sweep and the catalog test cover them.

### 7.8 How B4b avoids the kinds of findings S0 to B4a had

- **Wrong lines and counts** (S0 P9, B4a P14, P17): every line above was
  read on `bc3dd8de`; each list of call sites has its `git grep` command
  (`messages_fts`, `normalize_text`, `ensure_coll(host, SEARCH_INDEX`,
  `ContactsListInput {`).
- **Missed callers** (B3b P3, B4a P2): the seven chat index sites, the six
  `search_index` sites, the `ContactsListInput::default()` call in
  `roym_conversation`, the parity fixture in `test-components`, and the
  developer guide block that story 30 runs.
- **Audit gaps** (B1a R1, B4a P9): section 4 lists every changed refusal
  and the English precision cost.
- **Odd input** (B3b P1, B4a P4, P5, P16): ZWJ/ZWNJ, NUL, nukta after NFC,
  two-part Tamil vowels, mixed scripts, 1 KiB inputs, punctuation-only
  queries, non-UTF-8 chat bodies.
- **Plan promises without proof** (B4a P1, P6): the chat no-scan claim has
  an `EXPLAIN QUERY PLAN` test with and without `ANALYZE`, and a fallback
  form is named (D-B4b-15).
- **Index identity and rebuild** (B4a P10, P12, P18): `SEARCH_KEY_VERSION`
  drives both rebuilds; D-B4b-17 says exactly when the chat rebuild runs.
- **Sizes** (B3a R13, B4a R8): section 7.6 measures before and after in one
  build folder, with the owner's two limits.
- **Steps that do not build** (B3b P9): Steps 3 and 4 land together
  (Step 3 note).
- **e2e traps** (B3a P4, P5): the `.first()`/`nth(` grep, `type="button"`,
  no second `<p>` in the contact box.
- **Hindi** (B1b P8): button names through `hubMessage`, not typed.
- **Tests that prove less than their name** (B3b R7, B4a R4): each parity
  test compares the full answers of both builds and the exact id sets. The
  scrub tests also look for the key on disk, not only the body (plan
  review P2); the translation query uses a word that no other fixture
  listing has (P7); the 17-word query has 17 distinct keys (P7).
- **Tests with a wall-clock bound** (plan review P15): none in a normal
  test; timing only in `#[ignore]` tests.
- **Change doc test column** (B3a R2, B3b R3): Step 10.3 checks every name
  with `git grep`.

## 8. Deferred backlog

Changes to [deferred-backlog.md](../../deferred-backlog.md) at close-out:

**Move to "Recently resolved":**

- §11 "The directory text index is ASCII-folded, so a non-Latin query has
  no searchable characters" (`:598`).
- §11 "Directory search does not index author translations" (`:569`), if
  Q8 (a).
- §11 "Chat search does not use the message languages" (`:570`): resolved
  by design. The script of each word picks the table, and one Latin fold
  serves every language, so the tags are not needed.
- §2 "A text field holds one string" (`:86`, target "M08B B4b (decide
  there)"): decided. The directory joins its fields into one string, and
  that is enough for v1. Moved with that reason.

**Change:**

- §2 "`$search` returns rows in `id` order" (`:82`, target "M08B B4b or
  M08F"): target becomes M08F. B4b adds no ranking.

**New rows:**

| Item | Why deferred | Target | Source |
| --- | --- | --- | --- |
| Transliteration tables for scripts other than Devanagari and Tamil | No test data (Q4) | TBD | `crates/text_lang/src/search_key.rs` |
| English loanwords and vowel-quality spellings do not meet | A rule would merge real words (Q1) | TBD, with new test data | `crates/text_lang/src/search_key/fold.rs` |
| Tamil `ச` typed `ch` and `க` typed `k` between vowels do not meet | Q2 (a) | TBD | `crates/text_lang/src/search_key/tamil.rs` |
| The Latin fold merges some English words (`sheet`, `seat`, `sit`) | A query has no language (Q1) | TBD | `crates/text_lang/src/search_key/fold.rs` |
| An existing directory index keeps ASCII-folded text until `directory.reindex` | Q7 (a), no migrations before release | Before the first release | `crates/roym_directory/src/app/search_ops.rs` |
| `contacts.list` reads every contact and pages in memory, also with `text` | The data layer has no offset; the list is small | TBD | `crates/roym_profile/src/app/contacts.rs` |
| The chat snippet starts at the beginning of a cross-script hit | The Hub cannot find the matched word in another script | TBD | `crates/roym_web/ui/src/screens/message_search.ts` |
| The chat text index is rebuilt on open, on the store thread | One pass on the first open after a key change | TBD (measure first) | `crates/conversation/src/store/text_index.rs` |

No new `TODO` or `FIXME` marker is written.

## 9. Living docs to update at close-out

- `docs/system-architecture.md`: `:568` (`syneroym-text-lang` holds the
  tables and the fold, and the `fts` module); `:1016` (chat search: two
  FTS5 tables over search keys, no `LIKE` scan); `:2768` (the search key of
  a text field: NFC, lower case, transliteration, fold; the joined key in
  the trigram table).
- `docs/system-requirements-spec.md:1045` (`$search` over search keys in
  every script).
- `docs/TERMINOLOGY.md:73` "Search key": the four steps; add "Latin fold"
  and "transliteration table".
- `docs/developer-guide.md` §6.6 (`:2512`): one paragraph on `text`: every
  word must match, in any script, and a Latin word finds the same word in
  Devanagari or Tamil; a query with more than 16 words, or with no word
  (only punctuation), is `-32602`; a number matches as a substring (`50%`
  finds `500`); after an upgrade the node owner runs
  `roym directory.reindex | jq .result` (D-B4b-13, Q7). The new sentence
  about `reindex` is prose with a plain `bash` block, not a
  `roym-client=` block, so story 30 does not run it. The `roym-client=search` block
  (`:2500` to `:2503`) does not change, so story 30 needs no change.
- `docs/roym-integrated-experience-spec.md` `### Search` (`:768`): text
  search in every script and across scripts; contact search.
- `docs/planning/traceability-matrix.md`: the `[APP-A11Y]` row (`:68`) and
  the `[ROY-FED]` row (`:95`) name the new tests.
- `docs/decisions/0007-data-layer-wit-interface.md`: D-B4b-23.
- `docs/demo/demo.md`: story 40's new step.
- `task.md`: the B4b result line.

## 10. Exit criteria

- `spelling-pairs.tsv` says `# owner check: done <date>`, with the owner's
  decisions in its header (Step 2).
- Every test of section 7 exists and passes; the spelling floor holds.
- `git grep -n 'normalize_text\|messages_fts' -- '*.rs'` finds nothing.
- `git grep -n 'LIKE' crates/conversation/src/store/search.rs` finds
  nothing.
- Component and Hub sizes within the limits, written in task.md.
- The change doc, living docs, backlog and status.md updated (sections 3,
  8, 9).
- `mise run verify` passes with no `--skip`.
- The PR text has: the CLA line; the four Hindi messages for owner review;
  "old nodes still work; an old directory still refuses a non-Latin query"
  (section 4); the exact call `roym directory.reindex | jq .result` for an
  existing node (Q7, D-B4b-13); the chat index rebuild on first open (no
  reset needed); the English precision cost (Q1) and the number cost
  (`50%` finds `500`, P5); index text of up to about 88 KB per listing
  (Q8); the 64 KiB key time from `measure_search_key_64_kib`; before and
  after sizes; test counts.

## 11. What B4b does not do

- No ranking of search results (backlog §2, M08F).
- No transliteration tables for scripts other than Devanagari and Tamil
  (Q4).
- No rule for English loanwords or vowel quality (Q1).
- No language-specific fold: the message tags and record `lang` do not
  pick a table or a rule.
- No search over category labels in a language (a Hindi word for a
  category does not find the category code); category search stays by code
  (layer B).
- No transliterated display of a name ("the reader's node may show a
  transliteration next to a name" in the change doc is a later display
  choice).
- No search in `roymctl` for contacts or chats (neither command exists).
- No translation hook point on the search query (B5).
- No automatic rebuild of an existing directory index (Q7).

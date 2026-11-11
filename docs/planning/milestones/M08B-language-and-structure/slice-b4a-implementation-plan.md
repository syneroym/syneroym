# M08B slice B4a — Full-text option in data-layer: implementation plan

*Reader: the developer (or AI) who implements slice B4a. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-11. Not reviewed. Owner questions open.**

**Size and PR.** M. One PR on `feat/m08b-b4a-full-text`, with the commits
of section 6 in that order. The branch starts from `main` at `ca9002f3`
(B3b is merged, [PR #356](https://github.com/syneroym/syneroym/pull/356))
and already holds the plan commit.

B4a lets a service ask the host for full-text search over chosen fields of
a collection. When a service creates a collection, it can name its **text
fields**. The host then keeps two SQLite FTS5 tables for that collection in
step with every write: a trigram table for words of 3 or more characters,
and a word table with a 1- and 2-character prefix index for short words.
The host indexes a **search key** of each field, not the raw text. In B4a
the key is Unicode NFC and lower case; B4b adds transliteration and
folding. A guest searches with a new filter operator, `$search`, in the
filter of `query` and `delete-many`. B4a is a substrate change. No Roym
service uses it yet: B4b moves the directory onto it.

**Base.** Every file path and line number below was checked on `main` at
`ca9002f3`. Links are relative to this folder.

**Dependencies.** [task.md](task.md) section 4: B4a depends on S0 only. S0
is "Merged" in [status.md](status.md). B4b waits for B4a.

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - `mise run verify` before you finish (background job, about 20 minutes,
    no `--skip`).
  - No planning ids in code or comments: no "M08B", "B4a", "ML-29" or
    "D-B4a-3" in `.rs`, `.wit`, `.json` files or test names. Do not copy
    review ids into code (B2 R5, B3a R7).
  - Comments never cite size rules.
  - **File size.** Production files at most 800 lines. Test files at most
    800 lines. A file in `xtask/oversized-test-files.txt` is capped at its
    recorded count. Four capped files get a schema literal (section 7.5).
  - **Function size.** `clippy::too_many_lines` at 100. `do_query` and
    `do_create_collection` grow; D-B4a-12 says what moves out.
  - The import clean-up pass, the deferred backlog, the living-docs
    close-out and the demo close-out.
  - The WIT and every `roym_*` crate change (one field), so run
    `mise run build:test-components` and `mise run build:roym` before you
    trust a parity run ([task.md](task.md) section 2, "Records").
- [task.md](task.md): section 1 (Y4, Y5 and "Where Y3 and Y4 live"),
  section 2 ("Search"), "Component size limit", "B4a", "B4b", section 4.
- [roym-multilingual change doc](../../changes/roym-multilingual/change.md):
  the term "Search key", "Rules for every layer" (rule 4 "No migrations"),
  the section "Search" (every bullet, and the decision of 2026-10-26),
  criteria ML-20 and ML-29, and `## Deviations` (the first entry is about
  `syneroym-text-lang`; `data_db` computes the key itself). Every B4a
  decision and deviation is written there, and only there (section 3).
- [ADR-0007](../../../decisions/0007-data-layer-wit-interface.md) (the
  filter language and its operator list) and
  [ADR-0017](../../../decisions/0017-fdae-policy-schema-and-compilation.md) section 4 (CLS:
  a masked field must not be filterable).
- [slice-b3b-implementation-plan.md](slice-b3b-implementation-plan.md) (the
  layout of this plan) and the six review files. Section 7.8 lists how B4a
  avoids their kinds of findings.

**Change docs.** The prompt lists one change doc, roym-multilingual.
[task.md](task.md) names no other change doc for B4a. B4a meets no
criterion alone; its tests are the storage half of ML-20 and ML-29, both in
roym-multilingual.

### Terms used in this plan

- **Data layer:** the host capability `syneroym:data-layer/store`
  ([data-layer.wit](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit)).
  A guest stores JSON records in **collections**. The host crate is
  `crates/data_db` (package `syneroym-data-db`). Each service has its own
  encrypted SQLite file (SQLCipher).
- **Collection table:** the SQLite table for one collection:
  `id TEXT PRIMARY KEY, payload JSON, creator_id, created_at, updated_at`
  ([schema.rs:43](../../../../crates/data_db/src/sqlite/schema.rs#L43) to
  `:51`). It is a rowid table: SQLite gives every row a hidden integer
  `rowid`.
- **Filter:** the MongoDB-style JSON document of `query` and `delete-many`
  (ADR-0007), compiled to SQL by
  [filter.rs](../../../../crates/data_db/src/filter.rs).
- **Text field:** a top-level field of the payload that a collection
  declares for text search. Its value is a JSON string.
- **Search key:** the form of a text that search compares (change doc,
  terms). In B4a: NFC, then lower case. Made by `syneroym-text-lang`.
- **Text index:** the two FTS5 tables of one collection, plus one row in
  the host table `_text_index` (D-B4a-4).
- **Long term / short term:** a word of the query, after the search key is
  applied. A long term has 3 or more Unicode scalar values (Rust `char`s)
  and goes to the trigram table. A short term has 1 or 2 and goes to the
  word table.
- **Both builds:** a Roym service runs as a `wasm32-wasip2` component, or
  linked into the substrate through `syneroym-app-host-native`. Both reach
  `data_db` through the same `HostState` code in `sandbox_wasm`
  ([host.rs:94](../../../../crates/app_host_native/src/host.rs#L94) calls
  `HostStore::create_collection` on the `HostState`). The dual-build parity
  tests check that both give the same result.

## 1. What B4a builds, in one paragraph

`syneroym-text-lang` gains `nfc`, `search_key`, `query_terms` and the
constant `SEARCH_KEY_VERSION`, using `icu_normalizer` 2.4 (D-S0-2). The WIT
record `collection-schema` gains `text-fields: list<string>` (Q1). Every
place that builds a `CollectionSchema` adds the field: the WASM host and
the native shim pass it through unchanged, the native dispatch DTO of
`control_plane` reads it, and every Roym service and test component sends
`[]`. In `data_db`, `create-collection` with text fields creates two FTS5
tables, `_text_<c>_tri` (trigram) and `_text_<c>_word` (`unicode61` with
`prefix='1 2'`), and fills them from the rows that exist. Every write path
(`put`, `patch`, `delete`, `delete-many`, `batch-mutate`, `create`) updates
them in the same SQLite transaction, keyed by the row's `rowid`. The filter
compiler gets the operator `{"<field>": {"$search": "<text>"}}`: the host
turns the text into search-key terms and matches long terms in the trigram
table and short terms as word prefixes in the word table, all terms
required. No query falls back to a scan. `drop-collection` removes the
text index. Collection names that start with `_text_` are refused, and the
text tables never show in `list_collections`.

## How this is shown to an end user

An end user sees nothing new from B4a. It adds a search mechanism to the
storage layer that no screen uses yet. Slice B4b finishes the visible
story: in story 40, a search typed in Latin letters finds a listing with a
Devanagari title.

How a developer can see it work:

- `cargo nextest run -p syneroym-data-db text_index` runs the new tests: a
  collection declares `title` as a text field; a `$search` for `नल`, for
  `வாலா`, for `ek` (a 2-letter word) and for `PLUMB` finds the right rows;
  an update, a delete and a rolled-back batch keep the index right.
- `cargo nextest run -p syneroym-app-host-native --test dual_build_parity
  text_search` runs the same search through the WASM build and the native
  build and gets the same ids.
- The `EXPLAIN QUERY PLAN` test in the same crate shows that a `$search`
  query reads the collection table only by `rowid`, never by a full scan.

**Demo:** none. B4a adds nothing a viewer can see (change doc `demo:`
gets no new story; the B4a close-out writes nothing there, and B4b extends
story 40).

## 2. Points for the owner

### 2.1 Questions

Each question names the step it blocks. The recommendation is the first
option.

**Q1. How does a collection declare its text fields? (blocks Step 4)**

Owner answer: —

`create-collection` takes `collection-schema { name, indexes }`
([data-layer.wit:15](../../../../crates/wit_interfaces/wit/data-layer/data-layer.wit#L15)
to `:18`). It is called often: Roym services call it lazily before most
operations (for example `ensure_coll`,
[roym_directory/src/app.rs:107](../../../../crates/roym_directory/src/app.rs#L107),
has 12 callers in the directory alone).

- (a) **A field `text-fields: list<string>` on `collection-schema`.** The
  schema says everything about a collection in one place. Each of the 45
  places that build a `CollectionSchema` (32 files: section 2.2 point 4)
  adds `text_fields: vec![]` once, a mechanical change. The WIT record
  changes shape, so every guest is rebuilt (CI does this already). The
  package stays `syneroym:data-layer@0.1.0`, as with the conversation WIT
  in B3b (change doc rule 4). No trait gains a method. *Recommended.*
- (b) **A new function `set-text-fields: func(collection: string, fields:
  list<string>)`.** No existing caller changes, and old guests still link.
  But a guest must make two calls, and `AppDataLayer`, `ServiceStore`,
  `DbCommand` and both hosts each gain a method.

**Q2. What does a `text-fields` list mean when the collection already has a
text index? (blocks Step 5)**

Owner answer: —

Roym calls `create-collection` from many places for one collection, and not
all of them will name the text fields (for example the directory's
`backup.rs:56` calls `ensure_coll(host, collection, &[])` for every
collection).

- (a) **An empty list leaves the text index as it is. A non-empty list that
  differs from the index replaces it** (drop, create, fill from the rows).
  This matches `indexes` today: `create-collection` never drops an index
  it created before
  ([schema.rs:52](../../../../crates/data_db/src/sqlite/schema.rs#L52) to
  `:62`). A text index is removed only with `drop-collection`.
  *Recommended.*
- (b) **The list is the exact state:** an empty list drops the text index.
  Clear, but one call site that forgets the fields drops and later
  rebuilds the whole index, again and again.

**Q3. How does a guest write a text search? (blocks Step 6)**

Owner answer: —

- (a) **A field operator `$search`: `{"title": {"$search": "nal wala"}}`.**
  It sits with the other field operators (`$gt`, `$in`, `$regex`), so it
  works inside `$and`, `$or` and `$not`, next to other operators, and in
  `delete-many`. A search over two fields is an `$or`. The CLS check
  already sees the field name, because `referenced_top_level_fields`
  collects every key that does not start with `$`
  ([filter.rs:86](../../../../crates/data_db/src/filter.rs#L86) to `:109`).
  *Recommended.*
- (b) **The same, named `$text`.** Shorter, but MongoDB's `$text` is a
  top-level operator with a `$search` key; a reader who knows MongoDB
  expects that shape.
- (c) **MongoDB's top-level shape, `{"$text": {"$search": "..."}}`, over
  every text field.** Familiar, but a search cannot name a field, and the
  CLS check must learn a new rule (a masked text field must refuse the
  whole search).

**Q4. When a query has several words, which rows match? (blocks Step 6)**

Owner answer: —

Today the directory matches the whole folded query as one substring
(`$regex` becomes `LIKE '%nal wala%'`,
[filter.rs:286](../../../../crates/data_db/src/filter.rs#L286) to `:299`).

- (a) **Every word must match (AND), in any order.** `nal wala` finds
  "wala nal" too. This is what a person expects from a search box, and it
  is what lets B4b find `nal wala` in a key with the spaces removed
  (task.md B4b, "Joined words"). *Recommended.*
- (b) **Any word (OR).** Finds more, ranks nothing, so the page fills with
  weak matches.
- (c) **The whole text as one phrase.** The same as today, but a short word
  inside a phrase cannot use the short-word index, and joined words break.

**Q5. What does the host do with a text field whose value is not a string?
(blocks Step 5)**

Owner answer: —

- (a) **A write refuses it with `schema-violation("text field 'x' must be
  a string")`. A rebuild skips it (that row is not found for that field)
  and logs one `warn!` per rebuild with the count.** A guest bug shows at
  write time. A rebuild runs inside `create-collection`, so it must not
  fail on rows written before the declaration. A missing field or `null`
  indexes as empty in both cases. *Recommended.*
- (b) **Skip it always, never refuse.** One rule, but a guest that writes a
  number into a text field never learns why search misses it.
- (c) **Refuse always**, also in a rebuild. Then one old row stops every
  `create-collection` of that collection.

**Q6. How does a text index row point at its record? (blocks Step 5)**

Owner answer: —

- (a) **The FTS row has the collection row's `rowid`.** No extra table. The
  chat store already does the same: `messages_fts` uses
  `content_rowid='rowid'` of a table with a text primary key
  ([store/schema.rs:115](../../../../crates/conversation/src/store/schema.rs#L115)
  to `:120`). SQLite keeps a row's `rowid` on `UPDATE` and on
  `ON CONFLICT DO UPDATE`. Only `VACUUM` may give new rowids to a table
  without an `INTEGER PRIMARY KEY`, and no code in `crates/` runs `VACUUM`
  (checked with `git grep -i vacuum`). A backlog row records that a future
  `VACUUM` must rebuild every text index. *Recommended.*
- (b) **A host table per collection, `(rid INTEGER PRIMARY KEY, id TEXT
  UNIQUE)`,** so the link survives `VACUUM`. One more table and one more
  lookup on every write, for a case that no code has.

**Q7. How is ADR-0007 updated? (blocks Step 8)**

Owner answer: —

ADR-0007 lists the filter operators and says "The host only implements the
operators listed above"
([0007:155](../../../decisions/0007-data-layer-wit-interface.md#L155) to
`:157`). B4a adds an operator and a schema field, so the ADR needs a
change.

- (a) **A dated entry in its `## Amendments` section** (`:167`), as M04A B4
  did for `aggregate`. It names `$search`, `text-fields`, the two tables
  and the rule that a text search never scans. The original text stays.
  *Recommended.*
- (b) **A new ADR** that ADR-0007 links to. Better if the full-text design
  is expected to grow a lot (ranking, more tokenizers), but it splits the
  filter language over two records.

### 2.2 Points found while reading

1. **The filter compiler knows nothing about a collection.**
   `compile_filter(filter_json)`
   ([filter.rs:30](../../../../crates/data_db/src/filter.rs#L30)) is pure
   and has four callers: `do_query`
   ([query.rs:114](../../../../crates/data_db/src/sqlite/query.rs#L114)),
   `do_delete_many`
   ([mutation.rs:161](../../../../crates/data_db/src/sqlite/mutation.rs#L161)),
   `merge_sieve` for policy caveats
   ([sieve.rs:110](../../../../crates/data_db/src/sqlite/sieve.rs#L110)) and
   `aggregate`'s `$match`
   ([aggregate.rs:64](../../../../crates/data_db/src/aggregate.rs#L64)).
   `$search` needs the collection name and its text fields, so only the
   first two can offer it (D-B4a-8).
2. **Filters never use an index today.** The path is bound
   (`json_extract(payload, ?)`), so SQLite cannot match the expression
   index (backlog §11, row "create-collection's declared indexes are never
   used by query", `deferred-backlog.md:565`). `$search` is the first
   operator that uses an index: the FTS tables. The query must not then
   fall back to a scan of the collection table through its `ORDER BY id`
   (D-B4a-9).
3. **Not every write runs in a transaction.** Without a policy,
   `do_authorized_put`, `do_authorized_patch` and `do_authorized_delete`
   call `do_put`, `do_patch` and `do_delete` directly
   ([mutation.rs:479](../../../../crates/data_db/src/sqlite/mutation.rs#L479),
   `:497`, `:514`), and `do_delete_many` runs one `DELETE`
   (`:183` to `:188`). `do_batch_mutate` and `do_create` already use a
   transaction (`:205`, `:263`). A write and its index update must commit
   together (D-B4a-6).
4. **Who builds a `CollectionSchema`** (45 literals in 32 files, from
   `git grep -n 'CollectionSchema {'`):
   - Host code: the native shim's
     [`collection_schema_in`](../../../../crates/app_host_native/src/convert/data_layer.rs#L17)
     and its test (`:115`); the native dispatch DTO of `control_plane`
     ([data.rs:248](../../../../crates/control_plane/src/synsvc_native/data.rs#L248)
     to `:263`). The WASM host passes the generated type through
     ([capabilities_store.rs:104](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs#L104)
     to `:115`): no change.
   - Roym: `roym_catalog/src/app.rs:47`, `roym_conversation/src/app.rs:57`,
     `roym_directory/src/app.rs:113`, `roym_profile/src/app.rs:42`,
     `roym_transaction/src/app.rs:324`, `roym_core/src/signing.rs:250`.
   - Test components (outside the workspace, rebuilt by
     `mise run build:test-components`): `abac-test` (2),
     `data-layer-test`, `dual-build-fixture/src/app.rs:319`,
     `http-guest-test`, `messaging-pubsub-test`, `miniapp-demo1-wasm` (2),
     `saga-test`, `scheduled-test`, `stream-test`.
   - Tests and benches: `data_db` (`sqlite/tests.rs` 3, `tests_create.rs`,
     `tests_crud.rs`, `tests_fdae.rs`, `tests_vault.rs`, two benches),
     `sandbox_wasm` (`tests_store.rs` 3, `abac_integration.rs`, one
     bench), `router/tests/native_dispatch_identity/helpers.rs` (2),
     `app_host_native` parity `host_services.rs:62`,
     `permitted_differences.rs:214`.
   - No `bindings.rs` for the data layer is committed (only
     `app_supervisor`'s, which has no `CollectionSchema`).
5. **`create-collection` needs no gate change.** It stays ungated by
   design (backlog §3, row "`create-collection` stays ungated", and the
   comment at
   [capabilities_store.rs:121](../../../../crates/sandbox_wasm/src/host_capabilities/capabilities_store.rs#L121)
   to `:133`). With B4a it can also rebuild a text index. A rebuild
   destroys no row, so the reason still holds. It is refused for a
   read-only caller, as today (`:105`).
6. **`list_collections` lists every table.**
   [`do_list_collections`](../../../../crates/data_db/src/sqlite/query.rs#L229)
   excludes only `sqlite_%` and `_vault` (`:239` to `:241`). An FTS5 table
   adds the virtual table and five shadow tables (`_data`, `_idx`,
   `_content`, `_docsize`, `_config`). The FDAE `strict:` warning would see
   them as collections (D-B4a-4).
7. **Guest names can start with `_`.** `IDENTIFIER_REGEX` is
   `^[a-zA-Z_][a-zA-Z0-9_]{0,63}$`
   ([schema.rs:17](../../../../crates/data_db/src/sqlite/schema.rs#L17));
   a test keeps `_audit` legal
   ([sqlite/tests.rs:409](../../../../crates/data_db/src/sqlite/tests.rs#L409)).
   Only `_vault` is refused (`:28`). No code uses a name that starts with
   `_text` (`git grep '"_text'` finds nothing).
8. **Unit tests use raw connections.** Several `sqlite/tests.rs` tests call
   `do_query` on `Connection::open_in_memory()` with no host table at all
   (for example `:514` to `:524`). So the write and read paths must work
   when the host table `_text_index` does not exist (D-B4a-5).
9. **The SQLite build.** `rusqlite` 0.38 with `bundled-sqlcipher` and
   `hooks` (`Cargo.toml:160`); `libsqlite3-sys` 0.36, SQLite 3.50.4, FTS5 on
   ([task.md](task.md) section 2). The `functions` feature (custom SQL
   functions) is off, so the host cannot compute a search key inside SQL or
   in a trigger. The key is computed in Rust (D-B4a-6). `RETURNING`
   (3.35), the `categories` option of `unicode61` (3.25) and
   `pragma_table_info` as a table-valued function are all in 3.50.4. The
   `unicode61` table with `prefix='1 2'` is not checked yet (S0); Step 2
   checks it first.
10. **`unicode61` splits Indic words by default.** Its default token
    characters are the categories `L* N* Co`. Devanagari and Tamil vowel
    signs and the virama are marks (`Mn`, `Mc`), so `वाला` would split into
    `व` and `ल`. In B4a the key is not yet Latin, so the word table adds
    `M*` to the categories and keeps the marks (`remove_diacritics 0`).
    Step 2 checks this.
11. **`icu_normalizer` 2.4.0 is in `Cargo.lock` already** (through `url`
    and `idna`), but not a workspace dependency. `syneroym-text-lang` is
    linked into all six Roym guests through `roym_core`. S0 measured that a
    guest that never calls `nfc()` does not grow (93,305 bytes with and
    without it, task.md S0). No guest calls the new functions in B4a.
12. **The dual-build parity scenario table is capped.**
    `app_host_native/tests/dual_build_parity/helpers.rs` is at its cap of
    842 lines (`xtask/oversized-test-files.txt`), and its `SCENARIOS` table
    is at `:760` to `:813`. A new row would break the cap. The B4a parity
    test goes into `data_layer.rs` (130 lines), like
    `both_builds_create_fence_round_trip` (`:120` to `:130`).
13. **Chat search.** The chat store's FTS (trigram over the raw body, with
    a `LIKE` scan for short queries,
    [search.rs:47](../../../../crates/conversation/src/store/search.rs#L47))
    is a separate store in a separate crate. B4a does not touch it. B4b
    moves it onto search keys and a short-word table; it can reuse
    `query_terms` from `syneroym-text-lang` (D-B4a-2).
14. **No Hub, `roymctl`, e2e or demo caller.** No Hub screen, `roymctl`
    command, Playwright test or demo story sends a filter with a new
    operator or a schema with text fields. The native dispatch JSON for
    `create-collection` gets `text_fields` with `#[serde(default)]`, so its
    callers (`router/tests/native_dispatch_identity/queries.rs`,
    `access_control.rs`) do not change.
15. **No developer-guide section on the data layer exists** (`git grep` of
    `create-collection`, `$gt` and `AppDataLayer` in
    `docs/developer-guide.md` finds nothing). The living docs that describe
    the filter language are the requirements and the architecture doc
    (section 9).

## 3. What B4a meets

| Criterion | What B4a does | Test |
| --- | --- | --- |
| ML-20 (storage half) | A `$search` for a Devanagari word and for a Tamil word finds the row whose text field holds it. NFC makes two encodings of the same word meet. | `text_index/tests` (`search_finds_devanagari_and_tamil_words`, `search_matches_nfc_equivalent_forms`); parity `data_layer.rs` |
| ML-29 (storage half) | A query term of 1 or 2 characters is matched as a word or a word prefix in the word table, never in the trigram table. A query of only such terms returns rows. No `$search` query scans the collection table. | `text_index/tests` (`short_terms_match_words_and_word_prefixes`, `search_plan_never_scans_the_collection`); the tokenizer checks of Step 2 |

B4b finishes both criteria (transliteration, and the directory and chat on
top). The exact test names go into the change doc's Test column of ML-20
and ML-29 at close-out, marked "storage half", after they exist (B3a R2,
B3b R3).

**Where decisions and deviations are written.** Both criteria are in the
roym-multilingual change doc. Every B4a decision and deviation goes there
and only there, at close-out (Step 8): decisions into the section "Search"
as a new list "Decided <date>, the full-text option in data-layer", after
the bullet "A guest cannot create an FTS5 table today" (`change.md:392`);
deviations under `## Deviations`. The deviations, if the owner takes the
recommendations: a third host object, `_text_index`, beside the two FTS5
tables (D-B4a-4); the operator is named `$search` (Q3); the word table
keeps marks as token characters (D-B4a-3); a text search returns rows in
`id` order, not by relevance (D-B4a-9); `$search` is refused in
`aggregate` and in policy caveats (D-B4a-8); the prefix `_text_` is
reserved (D-B4a-4).

## 4. Behaviour-change audit

### Wire changes

No WIT package version, JSON-RPC method, Roym record or Roym method changes.

| # | Where | Before | After |
| --- | --- | --- | --- |
| W1 | WIT `syneroym:data-layer@0.1.0`, `collection-schema` | `name, indexes` | also `text-fields: list<string>` (Q1). A guest built against the old WIT does not link to the new host, so all guests are rebuilt |
| W2 | `create-collection` | creates a table and indexes | also validates `text-fields` (D-B4a-7) and, when non-empty, creates or replaces the text index (Q2) |
| W3 | Filter language (ADR-0007) of `query` and `delete-many` | `$search` is `schema-violation("unsupported operator: $search")` | `{"<text field>": {"$search": "<text>"}}` matches rows (D-B4a-8). Refusals are `schema-violation` with the texts of D-B4a-8 |
| W4 | `put`, `patch`, `batch-mutate`, `create` on a collection with a text index | any JSON accepted | a non-string value in a text field is `schema-violation("text field '<f>' must be a string")` (Q5) |
| W5 | Collection names | `_vault` refused | also any name that starts with `_text_`, ignoring case: `schema-violation("invalid identifier: <name>")` (D-B4a-4) |
| W6 | `list_collections` (FDAE `strict:` warning) | every table but `sqlite_%` and `_vault` | also leaves out `_text_%` (D-B4a-4) |
| W7 | Native dispatch `create-collection` JSON (`control_plane`) | `name`, `indexes?` | also `text_fields?`, a list of strings, default `[]` |
| W8 | Service database | — | new host table `_text_index` and two FTS5 tables per collection with text fields (D-B4a-4) |

W1 means every component is built again; no old component is kept by any
test. W8 adds tables only when a collection declares text fields. No Roym
service does in B4a, so a Roym node's databases do not change.

### Behaviour changes

| # | Before | After |
| --- | --- | --- |
| 1 | `put`, `patch` and `delete` without a policy run outside a transaction | they run in one transaction with the index update (D-B4a-6). The result and the errors are the same |
| 2 | `delete-many` runs one `DELETE` | one transaction: `DELETE … RETURNING rowid`, then the index rows (D-B4a-6). The returned count is the same |
| 3 | `put` runs `INSERT … ON CONFLICT DO UPDATE` | the same statement with `RETURNING rowid` (D-B4a-6) |

### Stays the same

- Every other filter operator, its SQL and its errors (the existing
  `filter.rs` tests run unchanged).
- `get`, `check-access`, `aggregate`, `execute-ddl`, `query-raw`, the vault,
  FDAE sieves and their traces.
- Query order (`ORDER BY id`) and cursor paging.
- Every Roym service: each sends `text_fields: []`, so its tables and
  behaviour do not change. Directory search still uses `normalize_text`
  and `$regex` (B4b changes it).
- Chat search in `crates/conversation`.
- The Hub, `roymctl`, every e2e test and demo story.

## 5. Decisions this plan makes

These are written into the change doc at close-out (section 3). The ones
that depend on an owner answer say so.

- **D-B4a-1. The WIT** (Q1 (a)). In `collection-schema`, after `indexes`:

  ```wit
  /// Top-level payload fields to index for text search with the `$search`
  /// filter operator. Each value must be a string. The host indexes the
  /// search key of each value and keeps the index in step with every
  /// write. An empty list leaves any text index the collection has.
  /// At most 8 fields; each is an identifier like an index field name.
  text-fields: list<string>,
  ```

  The doc of `query` (`data-layer.wit:115`) gains one paragraph: the filter
  may use `{"<text field>": {"$search": "<text>"}}`; every word of the text
  must match; a word of 1 or 2 characters matches a word or the start of a
  word, a longer word matches anywhere inside the field; rows come in `id`
  order. `delete-many` gets one line that the same operator works there.

- **D-B4a-2. `syneroym-text-lang` additions** (file
  `crates/text_lang/src/search_key.rs`, declared `mod search_key;` and
  re-exported from `lib.rs`; tests in `crates/text_lang/src/search_key/tests.rs`).

  ```rust
  /// Changes whenever `search_key` gives a different output for some input.
  /// A stored text index records the version it was built with; a
  /// different version makes the next `create-collection` rebuild it.
  pub const SEARCH_KEY_VERSION: u32 = 1;

  /// A query term of at most this many `char`s is a short term.
  pub const SHORT_TERM_MAX_CHARS: usize = 2;

  /// Unicode NFC (canonical composition).
  pub fn nfc(text: &str) -> String;

  /// The form of a text that search compares: lower case, then NFC.
  /// Idempotent: `search_key(&search_key(s)) == search_key(s)`.
  pub fn search_key(text: &str) -> String;

  #[derive(Debug, Clone, PartialEq, Eq, Default)]
  pub struct QueryTerms {
      /// Terms of 3 or more `char`s, in query order, no duplicates.
      pub long: Vec<String>,
      /// Terms of 1 or 2 `char`s, in query order, no duplicates.
      pub short: Vec<String>,
  }

  /// `search_key(query)`, split on Unicode white space. A term with no
  /// alphanumeric `char` (for example `-` or `!!`) is dropped. Empty when
  /// nothing is left.
  pub fn query_terms(query: &str) -> QueryTerms;
  ```

  - Lower case first, then NFC, so the output is always NFC (`to_lowercase`
    can give a non-NFC sequence). `nfc` uses
    `icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(text)`
    (the `compiled_data` constructor of 2.4).
  - `Cargo.toml` (workspace): `icu_normalizer = { version = "2.4",
    default-features = false, features = ["compiled_data"] }` next to
    `icu_locale_core` (`:127`). `crates/text_lang/Cargo.toml`:
    `icu_normalizer.workspace = true`.
  - No cargo feature splits this from the tag type: S0 measured that a
    guest that does not call `nfc` does not grow (section 2.2 point 11),
    and Step 8 checks it on the six Roym components.
  - Terms are split only on white space. Punctuation stays inside a term:
    `ab"c` is one term. The key does not remove it in B4a; B4b's folding
    may. A word with a virama, such as `प्लंबर`, is one term. The drop rule
    keeps every Devanagari or Tamil word, because their letters are
    alphanumeric for `char::is_alphanumeric`.

- **D-B4a-3. The two FTS5 tables.** For collection `c` with text fields
  `f1, …, fn` (in declared order):

  ```sql
  CREATE VIRTUAL TABLE _text_c_tri USING fts5(
      f_f1, …, f_fn, tokenize = 'trigram case_sensitive 1');
  CREATE VIRTUAL TABLE _text_c_word USING fts5(
      f_f1, …, f_fn,
      tokenize = "unicode61 remove_diacritics 0 categories 'L* N* Co M*'",
      prefix = '1 2');
  ```

  - Ordinary FTS5 tables (they keep a copy of the key). An external-content
    table cannot work: the content is the key, not a column of the
    collection table. A contentless table would save space but cannot be
    read back in a test; the keys are small. Rejected for now.
  - Column `f_<field>`: the prefix keeps a field named `rank` or `rowid`
    (FTS5 hidden columns) legal, and the columns name the fields, so the
    table is its own declaration (D-B4a-5).
  - `trigram case_sensitive 1`: the key is already lower case, so the
    tokenizer must not change it again. `unicode61` always folds case; that
    is harmless on a lower-case key and the same on both sides.
  - `remove_diacritics 0` and `M*`: the key decides what a letter is, not
    the tokenizer (section 2.2 point 10). B4b's key is Latin; the same
    table serves it.
  - In B4a both tables index the same `search_key` of the value. B4b may
    give the trigram table the key with spaces removed (task.md B4b). That
    is a change in one line of `sync_row` (D-B4a-6), not of the tables.
  - The two tokenizer strings are the constants `TRI_TOKENIZE` and
    `WORD_TOKENIZE` in `crates/data_db/src/sqlite/text_index.rs`, used by
    the DDL and by the Step 2 tests.

- **D-B4a-4. Host names, and what they reserve.**
  - Per collection: `_text_<c>_tri` and `_text_<c>_word`. One host table
    for all collections:

    ```sql
    CREATE TABLE IF NOT EXISTS _text_index (
        collection  TEXT PRIMARY KEY,
        key_version INTEGER NOT NULL
    );
    ```

    It is made by the first `declare` or `drop_index` call (D-B4a-5), not
    at database open, so a service with no text index gets no new table.
  - The type comes last (`_tri`, `_word`), so no name of one collection can
    equal a shadow table of another: a shadow table name ends in `_data`,
    `_idx`, `_content`, `_docsize` or `_config`, never in `_tri` or `_word`
    (with type first, `_text_tri_foo_data` would be both the table of
    collection `foo_data` and a shadow table of `foo`).
  - `validate_identifier` also refuses a name that starts with `_text_`,
    ignoring ASCII case, with the same error as `_vault`. It also guards
    index and text field names, which is harmless.
  - `do_list_collections` adds `AND name NOT LIKE '\_text\_%' ESCAPE '\'`
    (`LIKE` ignores ASCII case, the same as the refusal).
  - The host table is a third object beside the two FTS5 tables that the
    change doc names. It holds only the key version (a deviation).

- **D-B4a-5. Where the text fields are read from.** The write and read
  paths read them from the word table itself:

  ```rust
  /// The text fields of `collection`, in declared order; empty when it
  /// has no text index. Reads `SELECT name FROM pragma_table_info(?1)` of
  /// `_text_<c>_word` and strips `f_`. A missing table gives no rows, not
  /// an error, so raw test connections need no host table.
  pub(super) fn fields(conn: &Connection, collection: &str) -> Result<Vec<String>, DataLayerError>;
  ```

  Use `prepare_cached`. This is one in-memory schema read per write, with
  no file access. The key version is read only by `declare`. If Step 2
  finds that `pragma_table_info` does not list FTS5 columns, keep the field
  list in `_text_index` instead (a `fields` column, joined by `,`), and
  write the change into the plan before you continue.

- **D-B4a-6. Keeping the index in step** (file
  `crates/data_db/src/sqlite/text_index.rs`; the pure parts are in
  D-B4a-8).

  ```rust
  /// Create, replace or keep the text index of `collection` (Q2 (a)).
  /// Called by `do_create_collection` inside its transaction.
  pub(super) fn declare(conn: &Connection, collection: &str, wanted: &[String]) -> Result<(), DataLayerError>;
  /// Drop both tables and the `_text_index` row. Called by `do_drop_collection`.
  pub(super) fn drop_index(conn: &Connection, collection: &str) -> Result<(), DataLayerError>;
  /// Check that every text field of `payload` is a string, null or absent (Q5 (a)).
  pub(super) fn check_values(fields: &[String], payload: &Value) -> Result<(), DataLayerError>;
  /// Replace the index rows of `rowid` with the keys of `payload`.
  pub(super) fn sync_row(conn: &Connection, collection: &str, fields: &[String], rowid: i64, payload: &Value) -> Result<(), DataLayerError>;
  /// Remove the index rows of these rowids.
  pub(super) fn remove_rows(conn: &Connection, collection: &str, rowids: &[i64]) -> Result<(), DataLayerError>;
  ```

  `declare(conn, c, wanted)`:

  ```text
  have = fields(conn, c)
  if wanted is empty: return Ok                      // Q2 (a); the common call
  ensure _text_index exists
  if have == wanted
     and _text_c_tri exists                           // sqlite_master lookup
     and key_version(c) == SEARCH_KEY_VERSION: return Ok
  drop both tables if they exist
  create both tables (D-B4a-3)
  for each (rowid, payload) in SELECT rowid, payload FROM c:
      parse payload; on a JSON error count it and skip
      a non-string text value is indexed as empty and counted (Q5 (a))
      sync_row(...)
  upsert _text_index (c, SEARCH_KEY_VERSION)
  if skipped > 0: warn!(collection = c, skipped, "text index rebuild skipped values that are not strings")
  ```

  A guest that calls `create-collection` with the same fields on every
  request pays one schema read and one primary-key read, and no write.

  The write paths (`mutation.rs`):

  - `payload_to_text` returns the parsed `Value` as well as the text, so the
    payload is parsed once.
  - `do_put`: `let fields = text_index::fields(conn, collection)?;`
    `check_values(&fields, &value)?` before any write; the upsert gains
    `RETURNING rowid` and runs with `query_row`; then, when `fields` is not
    empty, `sync_row`.
  - `do_patch`: the same after the merge: `check_values` on the merged
    value, `UPDATE … RETURNING rowid`, `sync_row`.
  - `do_delete`: `DELETE … WHERE id = ?1 RETURNING rowid` (zero or one row;
    zero rows is still `Ok`, the delete stays idempotent); then
    `remove_rows`.
  - `do_delete_many(conn: &mut Connection, …)`: a transaction;
    `DELETE FROM c {where} RETURNING rowid`, collected (an error still goes
    through `map_query_raw_step_error`, so a watchdog stop is still
    `quota-exceeded`); `remove_rows`; commit; the count is the number of
    rowids. The writer loop passes `&mut conn`
    ([service_store.rs:155](../../../../crates/data_db/src/sqlite/service_store.rs#L155)
    to `:160`).
  - `do_authorized_put`, `do_authorized_patch`, `do_authorized_delete`: the
    no-policy path opens a transaction too, so one rule holds for every
    write. The comment "the policy-absent hot path pays nothing new"
    (`mutation.rs:470` to `:471`) changes to say that every write and its
    index rows commit together. SQLite already wraps a single statement in
    its own transaction, so one `BEGIN`/`COMMIT` around 2 to 5 statements
    costs about the same.
  - `do_batch_mutate` and `do_create` already hold a transaction; a
    failing mutation rolls back the index rows with the rows.
  - `do_create_collection(conn: &mut Connection, …)` and
    `do_drop_collection(conn: &mut Connection, …)` each run in one
    transaction (writer loop `:124` and `:127` pass `&mut conn`).
  - `sync_row` runs `DELETE FROM _text_c_tri WHERE rowid = ?1`, the same for
    `_text_c_word`, then `INSERT INTO … (rowid, f_f1, …) VALUES (?, ?, …)`
    into each, with `search_key(value)` for a string and `NULL` otherwise.
    `remove_rows` runs the two `DELETE`s for each rowid.

- **D-B4a-7. Rules for `text-fields`** (checked in `do_create_collection`
  before any SQL, error `schema-violation`):
  - each name passes `validate_identifier`;
  - at most `MAX_TEXT_FIELDS = 8` (`"too many text fields (max 8)"`);
  - no name twice, ignoring ASCII case, because FTS5 column names ignore
    case (`"text field '<f>' is listed twice"`).
  - A text field may also be an index field.

- **D-B4a-8. The `$search` operator** (Q3 (a), Q4 (a)). Pure code in a new
  file `crates/data_db/src/text_match.rs` (module `text_match`, declared in
  `lib.rs`), called from `filter.rs`.

  ```rust
  /// The text index of one collection, as the filter compiler needs it.
  pub struct TextFields<'a> {
      pub collection: &'a str,
      pub fields: &'a [String],
  }

  pub const MAX_SEARCH_TERMS: usize = 16;
  pub const MAX_SEARCH_BYTES: usize = 1024;

  /// Compiles `{"<path>": {"$search": <value>}}` to a SQL condition on the
  /// collection's rowid, and pushes its bound parameters.
  pub(crate) fn compile_search(path: &str, value: &Json, text: Option<&TextFields<'_>>, params: &mut Vec<Value>) -> Result<String, DataLayerError>;
  ```

  - `filter.rs`: `compile_filter(json)` stays and calls
    `compile_filter_in(json, None)`. New `pub fn compile_filter_in(json,
    text: Option<&TextFields<'_>>)`. The five private functions pass `text`
    down; `compile_operator_map` gets the arm `"$search" =>
    { clauses.push(text_match::compile_search(path, opval, text, params)?);
    continue; }`. `CompiledFilter` gains `pub text_match: bool`, true when
    any `$search` was compiled (D-B4a-9).
  - Only `do_query` and `do_delete_many` call `compile_filter_in` with
    `Some`, after `text_index::fields`. `merge_sieve` (policy caveats) and
    `aggregate` keep `compile_filter`, so `$search` there is refused.
  - Checks, in this order, each `schema-violation`:
    - `text` is `None`: `"$search is not available in this filter"`;
    - `path` is not one of `fields`: `"'<path>' is not a text field of
      collection '<c>'"` (this also refuses dotted paths);
    - `value` is not a JSON string: `"$search requires a string for field
      '<path>'"`;
    - more than `MAX_SEARCH_BYTES` bytes: `"$search text is longer than
      1024 bytes"`;
    - `query_terms` is empty: `"$search needs at least one word"`;
    - more than `MAX_SEARCH_TERMS` terms: `"$search has more than 16
      words"`.
  - The SQL, with the column `f_<path>`. Long terms:
    `c.rowid IN (SELECT rowid FROM _text_c_tri WHERE _text_c_tri MATCH ?)`
    with the bound expression `f_<path> : "t1" AND f_<path> : "t2"`. Short
    terms: the same over `_text_c_word` with `f_<path> : "ab"* AND …`. When
    both kinds are there, the two conditions are joined with `AND`, in
    brackets.
  - Each term is written as an FTS5 string: `"` + the term with every `"`
    doubled + `"`. The expression is a bound parameter, never part of the
    SQL text. Table and column names are made from validated identifiers.
    So a query like `a" OR f_x : *` is only text.
  - The CLS check of `do_query` (`query.rs:105` to `:112`) needs no change:
    the field of a `$search` is a key without `$`, so
    `referenced_top_level_fields` reports it, and a masked field is
    `permission-denied`.

- **D-B4a-9. A text search never scans the collection table.** With
  `ORDER BY id` and a cursor `id > ?`, SQLite may choose to walk the `id`
  index of the collection table and test `rowid IN (…)` on every row. That
  is a full scan. When `CompiledFilter.text_match` is true, `do_query`
  writes `ORDER BY +c.id` and `+c.id > ?` (a unary `+` stops SQLite from
  using an index for that term). Then the only way in is the `rowid` list
  from FTS5, and SQLite sorts the matched rows. The order of the result and
  the cursor do not change. A text search returns rows in `id` order, not
  by relevance; ranking is not in B4a (backlog row, section 8).

- **D-B4a-10. Limits and cost.** The query is bounded by `MAX_SEARCH_TERMS`
  and `MAX_SEARCH_BYTES` per `$search`, by `MAX_FILTER_DEPTH` (10) and by
  `MAX_QUERY_PAGE_SIZE` (1000). A short term like `a` can match most rows;
  the host then reads and sorts all matched rows before it applies
  `LIMIT`. This is bounded by the collection size, not by the query, and
  it reads no row that does not match. A rebuild runs on the single writer
  and blocks that service's writes while it runs, like a `CREATE INDEX`
  today (architecture doc, "Schema & Indexing"). Step 6 measures a rebuild
  of 10,000 rows and a query on them, and the PR text gives the numbers
  (not a gate).

- **D-B4a-11. Shared test helpers in `data_db`.** A new `#[cfg(test)] mod
  test_support;` in `crates/data_db/src/lib.rs` (file
  `src/test_support.rs`) with
  `pub(crate) fn plain_schema(name: &str) -> CollectionSchema` and
  `pub(crate) fn schema_with(name: &str, indexes: Vec<IndexDefinition>, text_fields: Vec<String>) -> CollectionSchema`.
  `tests_create.rs`, `tests_fdae.rs`, `tests_crud.rs` and `sqlite/tests.rs`
  use them instead of their own literals (two copies of `plain_schema`
  exist today, `tests_create.rs:29` and `tests_fdae.rs:41`). This removes
  duplicates and keeps the capped files under their caps (section 7.5).

- **D-B4a-12. Function length.** `do_query` is 79 lines today
  (`query.rs:91` to `:170`). Its SQL building moves into
  `fn query_sql(collection: &str, sieve: Option<(String, Vec<SqlValue>)>, compiled: Option<&CompiledFilter>, cursor: Option<&str>, limit: u32) -> (String, Vec<SqlValue>)`,
  which the plan test (section 7.2) also calls. `do_create_collection`
  calls `text_index::declare`, so it grows by a few lines only.

- **D-B4a-13. Parity.** The dual-build fixture gains one request,
  `TextSearch`, in its `Request` enum
  ([app.rs:47](../../../../test-components/dual-build-fixture/src/app.rs#L47)),
  handled in `app/dispatch.rs`: it creates collection `text_scratch` with
  `text_fields: ["title"]`, puts four rows (`t1` `"नल वाला"`, `t2`
  `"Chennai plumber"`, `t3` `"ek do teen"`, `t4` `"சென்னை வாலா"`), runs
  three queries (`{"title": {"$search": "वाला"}}`, `"PLUMB"`, `"ek teen"`)
  and returns the three id lists. The test in `data_layer.rs` asserts that
  both builds give `[["t1","t4"],["t2"],["t3"]]`.

## 6. Step-by-step changes

Each step builds and its tests pass. Rust: `cargo nextest run -p <crate>`
for the crates the step touches, output to a file (AGENTS.md "Context
Budget"). Paths are from the repository root.

### Step 1 — Commit this plan

Done in the planning session: this file and the B4a row of
[status.md](status.md).

### Step 2 — Check the SQLite build first

1. `crates/data_db/src/sqlite/text_index.rs` with only `TRI_TOKENIZE`,
   `WORD_TOKENIZE` (D-B4a-3) and `#[cfg(test)] mod tests;`. Declare
   `mod text_index;` in `crates/data_db/src/sqlite.rs`.
2. `crates/data_db/src/sqlite/text_index/tests.rs` with the tokenizer tests
   of section 7.2 ("the SQLite build"). They use raw SQL on
   `Connection::open_in_memory()` and the two constants.
3. `cargo nextest run -p syneroym-data-db text_index`.
4. **Stop and tell the owner** if a `prefix='1 2'` query, a Devanagari or
   Tamil word in the word table, `RETURNING` on an upsert, or
   `pragma_table_info` of an FTS5 table does not work as section 7.2 says.
   Write what you found into this plan first.

### Step 3 — Search keys in `syneroym-text-lang`

1. Workspace `Cargo.toml` and `crates/text_lang/Cargo.toml`: `icu_normalizer`
   (D-B4a-2).
2. `crates/text_lang/src/search_key.rs` and its tests; `lib.rs`:
   `mod search_key; pub use search_key::{…};`.
3. `cargo nextest run -p syneroym-text-lang`; `cargo deny check licenses`
   (`icu_normalizer` is `Unicode-3.0`, already allowed through `idna`).

### Step 4 — The WIT field and every builder (no index yet)

1. `crates/data_db/src/test_support.rs` and the four test files use it
   (D-B4a-11), before the field exists. `cargo nextest run -p
   syneroym-data-db`: the same tests pass, the same number. Set the new
   line counts of `sqlite/tests.rs`, `tests_crud.rs` and `tests_fdae.rs` in
   `xtask/oversized-test-files.txt` (AGENTS.md "Mandatory Ratchet Update").
2. WIT (W1): `text-fields` in `collection-schema`, with the doc of D-B4a-1
   (the `query` paragraph waits for Step 6).
3. Every site of section 2.2 point 4: `text_fields: vec![]` (or
   `Vec::new()`); `collection_schema_in` passes `text_fields: v.text_fields`
   and its test checks it; the `control_plane` DTO gets
   `#[serde(default)] text_fields: Vec<String>` (W7); the two
   `test_support` helpers set it.
4. `data_db`: `do_create_collection` validates `text_fields` (D-B4a-7) and
   does nothing else with them yet. Tests of D-B4a-7.
5. `mise run build:test-components`, `mise run build:roym`, then
   `cargo nextest run -p syneroym-data-db -p syneroym-sandbox-wasm
   -p syneroym-app-host -p syneroym-app-host-native -p syneroym-control-plane
   -p syneroym-router -p syneroym-roym-core -p syneroym-roym-web`.

### Step 5 — The text index and the write paths

1. `schema.rs`: the `_text_` refusal in `validate_identifier` (D-B4a-4).
   `query.rs`: the `_text_%` exclusion in `do_list_collections`.
2. `text_index.rs`: `fields`, `declare`, `drop_index`, `check_values`,
   `sync_row`, `remove_rows` (D-B4a-5, D-B4a-6). It needs
   `syneroym-text-lang.workspace = true` in `crates/data_db/Cargo.toml`.
3. `schema.rs`: `do_create_collection` and `do_drop_collection` take
   `&mut Connection`, run in a transaction and call `declare` and
   `drop_index`. `service_store.rs` passes `&mut conn`.
4. `mutation.rs`: the write paths of D-B4a-6, in this order: `payload_to_text`,
   `do_put`, `do_patch`, `do_delete`, the three `do_authorized_*`,
   `do_delete_many` (and its writer-loop call).
5. Tests of section 7.2 ("declare" and "writes").
6. `cargo nextest run -p syneroym-data-db`.

### Step 6 — The `$search` operator

1. `crates/data_db/src/text_match.rs` (D-B4a-8) and `lib.rs`
   `pub mod text_match;`.
2. `filter.rs`: `compile_filter_in`, the `text` parameter, the `$search`
   arm, `CompiledFilter.text_match`. Its existing tests run unchanged; new
   ones of section 7.2 ("filter").
3. `query.rs`: `query_sql` (D-B4a-12); `do_query` reads
   `text_index::fields` when `opts.filter` is `Some` and calls
   `compile_filter_in`; the `+c.id` form when `text_match` (D-B4a-9).
   `mutation.rs`: `do_delete_many` the same, without the order hint (a
   delete has no `ORDER BY`).
4. WIT: the `query` and `delete-many` paragraphs of D-B4a-1.
5. Tests of section 7.2 ("search", "plan"); the timing of D-B4a-10.
6. `cargo nextest run -p syneroym-data-db`.

### Step 7 — Both builds

1. `test-components/dual-build-fixture`: `Request::TextSearch` and its
   handler (D-B4a-13).
2. `crates/app_host_native/tests/dual_build_parity/data_layer.rs`:
   `both_builds_text_search_round_trip`.
3. `mise run build:test-components`, then
   `cargo nextest run -p syneroym-app-host-native --test dual_build_parity`.

### Step 8 — Sizes, docs and close-out

1. Component size (section 7.6): the six Roym `.wasm` sizes from
   `mise run build:roym`, before (on `main`) and after, in the same build
   folder. Write them into task.md (B4a result line).
2. ADR-0007: the amendment of Q7 (a), dated the day you write it.
3. Change doc: decisions into "Search" as "Decided <date>, the full-text
   option in data-layer" (D-B4a-1 to -10, in short form, and every owner
   answer); the deviations of section 3; the Test column of ML-20 and
   ML-29 with the real test names, marked "storage half"; in "Search", the
   sentence "`unverified`: the second table, with the `unicode61` tokenizer
   and `prefix='1 2'`" becomes the result of Step 2. `demo:` stays story
   40; `status` stays `draft`; add any file of section 9 that is not in
   `living-docs-touched` yet.
4. Living docs (section 9), deferred backlog (section 8), task.md B4a
   result line, status.md.
5. The import clean-up pass over every Rust file you edited.
6. `cargo dupes report` over the changed Rust files (AGENTS.md recipe). One
   FTS5 string quoter (`text_match.rs`), one schema helper pair
   (`test_support.rs`), one key function (`search_key`).
7. `mise run verify` with no `--skip`, as a background job.

## 7. Tests

### 7.1 Hub unit tests

None. B4a does not change the Hub.

### 7.2 Rust

`crates/text_lang/src/search_key/tests.rs`

- `nfc`: the six S0 inputs give the S0 outputs: `U+0958` gives
  `U+0915 U+093C`; `U+0B95 U+0BC6 U+0BBE` gives `U+0B95 U+0BCA`;
  `U+0065 U+0301` gives `U+00E9`; the other three do not change (task.md
  S0, "NFC").
- `search_key`: `"Nal WALA"` gives `"nal wala"`; `"E\u{301}"` and `"É"`
  give the same key; the two forms of `क़िला` (with `U+0958`, and with
  `U+0915 U+093C`) give the same key; for each of 10 sample strings
  (Latin, Devanagari, Tamil, mixed), `search_key(search_key(s)) ==
  search_key(s)`.
- `query_terms`: `"  Nal   WALA "` gives long `["nal", "wala"]`, short
  `[]`; `"ek do teen"` gives long `["teen"]`, short `["ek", "do"]`;
  `"नल वाला"` gives short `["नल"]`, long `["वाला"]`; `"--- !!"` gives
  both empty; `"nal nal"` gives `["nal"]` once; `ab"c` is one long term.
- `SHORT_TERM_MAX_CHARS` is 2 and `SEARCH_KEY_VERSION` is 1.

`crates/data_db/src/sqlite/text_index/tests.rs` (split into
`text_index/tests/<topic>.rs` with `text_index/tests.rs` as the entry if it
passes 800 lines; module layout rule)

The SQLite build (Step 2), raw SQL, the two constants:

- A word table with rows `"ek"`, `"ekdam"`, `"nal wala"`, `"नल वाला"`,
  `"சென்னை வாலா"`: `MATCH '"e"*'` and `'"ek"*'` give the `ek` and `ekdam`
  rows; `'"नल"*'` gives the Devanagari row; `'"வா"*'` gives the Tamil row;
  `'"ல"*'` gives nothing (marks are kept, so `वाला` is one token, not `व`
  and `ल`).
- A trigram table with `case_sensitive 1`: `MATCH '"umb"'` finds
  `"plumber"`; a 2-character phrase finds nothing (the reason short terms
  go to the word table).
- Column filter: `'f_a : "abc"'` matches only a row with `abc` in `f_a`,
  not one with `abc` in `f_b`.
- `SELECT name FROM pragma_table_info('<fts table>')` lists `f_a, f_b`, and
  gives no rows for a table that does not exist.
- `INSERT … ON CONFLICT(id) DO UPDATE … RETURNING rowid` gives the same
  rowid for the insert and the later update of one id.

Declare (Step 5):

- `create-collection` with `text_fields: ["title"]` creates
  `_text_c_tri`, `_text_c_word` and one `_text_index` row with
  `key_version = 1`; `list_collections` shows only `c`.
- Rows written before the declaration are found after it (backfill).
- The same call again does not rebuild: a marker row written straight into
  `_text_c_word` by the test is still there.
- `text_fields: []` on a collection with a text index keeps it (Q2 (a)).
- `["title", "summary"]` after `["title"]` rebuilds: a search on `summary`
  works.
- A `_text_index` row set to `key_version = 0` by the test makes the next
  call rebuild (the marker row is gone).
- After the test drops `_text_c_tri` with raw SQL, the next call rebuilds.
- A stored row whose `title` is a number: the rebuild succeeds and that row
  is not found (Q5 (a)).
- Refusals: a collection named `_text_x` and `_TEXT_x`; 9 text fields;
  `["title", "Title"]`; a text field `"a.b"`. Each is `schema-violation`.
- `drop-collection` removes both tables and the `_text_index` row;
  creating the collection again with no text fields works.
- Two collections `foo` and `foo_data`, both with text fields, keep their
  own rows (D-B4a-4).

Writes (Step 5):

- `put` then search finds the row; a second `put` that changes `title`
  makes the old word miss and the new word match; a `patch` that sets
  `title` to `null` makes it miss; `delete` makes it miss and leaves no row
  in either FTS table (counted with raw SQL).
- `delete-many` with a filter removes exactly the matching rows from both
  FTS tables and returns the same count as before B4a.
- A `batch-mutate` whose last mutation fails rolls back the earlier rows'
  index rows; a `create` that hits an existing id leaves no index rows.
- `put` with `title: 5` is `schema-violation("text field 'title' must be a
  string")`, and no row is stored.
- A collection with no text fields stores and deletes the same as before
  (no FTS table is created, `_text_index` does not exist).

Filter (`filter.rs` tests, Step 6):

- `compile_filter(Some(r#"{"title": {"$search": "x"}}"#))` (no text index
  given) is `"$search is not available in this filter"`.
- With `TextFields { collection: "c", fields: &["title"] }`: `"plumber"`
  gives one trigram condition and the expression `f_title : "plumber"`;
  `"ek plumber"` gives both conditions joined by `AND`; `text_match` is
  true; a field `"summary"` is refused; a number is refused; 17 words are
  refused; 1025 bytes are refused; `"-- !"` is refused.
- `a" OR f_x : *` is bound as the string `f_title : "a"" OR …"` (quotes
  doubled), never in the SQL text.
- `referenced_top_level_fields` of a `$search` filter contains `title`.
- An `aggregate` with `$match: {"title": {"$search": "x"}}` is refused with
  the "not available" text.

Search (Step 6), through `SqliteServiceStore`:

- `search_finds_devanagari_and_tamil_words`: `नल` finds `"नल वाला"`;
  `வாலா` finds `"சென்னை வாலா"`.
- `search_matches_nfc_equivalent_forms`: a row stored with `U+0958` is
  found by a query with `U+0915 U+093C`, and the other way round.
- `PLUMB` finds `"Chennai plumber"` (case and a part of a word).
- `short_terms_match_words_and_word_prefixes`: `ek` finds `"ek do teen"`
  and `"ekdam"`; `lu` does not find `"plumber"` (a short term matches only
  the start of a word); `ek teen` needs both words.
- A search on `title` does not match text that is only in `summary`; an
  `$or` of the two does.
- `$search` with `$not`, and with an equality on another field.
- Paging: 25 matching rows, `limit: 10`: three pages, ids in order, no row
  twice, `next_cursor` `None` on the last.
- CLS: a `CompiledSieve` with `masked_fields = ["title"]` (built as
  `pathological_sieve` is in `sqlite/tests.rs`) makes a `$search` on
  `title` `permission-denied`.
- `$search` on a field that is not declared, and on a collection with no
  text index: `schema-violation` with the D-B4a-8 text.

Plan (Step 6):

- `search_plan_never_scans_the_collection`: `EXPLAIN QUERY PLAN` of the SQL
  that `query_sql` builds, for a long term, a short term, both, and both
  with a cursor, on a collection with 200 rows after `ANALYZE`. No plan line
  is `SCAN c` or `SCAN c USING …`; there is a line for each FTS table it
  uses. (The FTS lines start with `SCAN _text_c_… VIRTUAL TABLE`; the test
  checks the collection name exactly.)

`crates/app_host_native/src/convert/data_layer.rs` — the existing
`collection_schema_round_trips` also checks `text_fields`.

`crates/control_plane` — none new: the DTO change is one field with a
default, and `native_dispatch_identity` runs unchanged.

Parity (Step 7):

- `crates/app_host_native/tests/dual_build_parity/data_layer.rs`,
  `both_builds_text_search_round_trip` (D-B4a-13).
- `crates/roym_web/tests/dual_build_parity` runs unchanged after
  `mise run build:roym`.

### 7.3 Playwright

None. No spec or story changes; `mise run test:e2e` and `mise run demo`
run unchanged inside `mise run verify`.

### 7.4 Not tested

- A `VACUUM` of a service database (no code runs one; Q6 (a)).
- The time of a rebuild of a large collection beyond the one measurement of
  D-B4a-10.
- Transliteration and folding (B4b).

### 7.5 Size risks

| File | Today | Risk |
| --- | --- | --- |
| `crates/data_db/src/sqlite/tests.rs` | 899 (cap 899) | three 4-line literals become `&plain_schema("…")`: about −9. Lower the cap |
| `crates/data_db/src/tests_crud.rs` | 819 (cap 819) | its `schema` helper calls `schema_with`: about −2. Lower the cap |
| `crates/data_db/src/tests_fdae.rs` | 1791 (cap 1791) | its `plain_schema` moves to `test_support`: about −3. Lower the cap |
| `crates/sandbox_wasm/tests/abac_integration.rs` | 811 (cap 811) | the literal at `:753` passes 100 columns with the new field. Bind it to a `let` on one line so the call chain fits on one line too; do not let the file grow |
| `crates/data_db/src/sqlite/service_store.rs` | 749 | two `&conn` become `&mut conn`: 0 |
| `crates/data_db/src/sqlite/mutation.rs` | 521 | about +40 |
| `crates/data_db/src/filter.rs` | 476 | about +20 code, +70 tests: about 570 |
| `crates/control_plane/src/synsvc_native/data.rs` | 702 | +3 |
| `crates/sandbox_wasm/src/host_capabilities/tests/tests_store.rs` | 711 | +3 |
| `crates/data_db/src/sqlite/query.rs` | 299 | about +25 (`query_sql`) |
| `crates/data_db/src/sqlite/text_index.rs` | new | about 220 |
| `crates/data_db/src/sqlite/text_index/tests.rs` | new | about 700; split by topic if it passes 800 |
| `test-components/dual-build-fixture/src/app/dispatch.rs` | 608 | about +35 |

`app_host_native` parity `helpers.rs` (cap 842) does not change (section
2.2 point 12). If a capped file gets shorter, set its new count. New Rust
code adds to the workspace size, so the duplication percent may fall; if
`cargo xtask check-duplication` says so, lower
`MAX_EXACT_DUPLICATE_PERCENT` as its message says. Never raise it.

### 7.6 Component size

- Every Roym component gets one empty list in a struct literal and the
  rebuilt WIT record. No Roym component calls `search_key` or `nfc`, so
  the `icu_normalizer` code is not linked. Each of the six is a
  shared-code component ([task.md](task.md) "Component size limit"): stop
  and tell the owner if one grows by more than 40 KB. Expected: a few
  hundred bytes. Sizes after B3b: web 255,652, conversation 810,737,
  profile 767,699, catalog 927,744, transaction 1,507,245, directory
  1,463,187 bytes (task.md, B3b result). Measure before and after in the
  same build folder.
- The Hub bundle does not change.

### 7.7 Hindi text

None. B4a adds no Hub text.

### 7.8 How B4a avoids the kinds of findings S0 to B3b had

- **A check that nobody ran** (S0 P8): the `unicode61` `prefix='1 2'` table
  is checked first, in Step 2, as tests that stay, with a stop rule.
- **A function that crosses the clippy line limit** (B3a P3): `do_query`
  gives its SQL building to `query_sql` (D-B4a-12); `declare` lives in
  `text_index.rs`.
- **Two copies of one helper** (B1b R5; B2 R7; B3a R8, R14; B3b R8): one
  key function and one term splitter (`syneroym-text-lang`), one FTS5
  quoter (`text_match.rs`), one pair of schema test helpers that also
  replaces two existing copies (D-B4a-11).
- **A step that breaks a later step or a capped file** (B1b P4; B3a P4):
  the WIT field and all 45 builders change in one step (Step 4), and the
  capped files shrink first (Step 4.1, section 7.5).
- **A step that writes before the transaction is safe** (B3b P1, P3): every
  write path and its index rows share one transaction (D-B4a-6), and tests
  prove the rollback (section 7.2, "Writes").
- **Tests that prove less than their names** (B3a R5; B3b R7, R9): each
  test name says what it checks; the plan test reads the plan text, and
  the "no rebuild" test uses a marker row, not a timing.
- **The change doc's Test column names tests that do not exist** (B3a R2,
  B3b R3): names are written at close-out from the real files (section 3).
- **A claim not checked in the code** (S0 P1): every line here is from
  `ca9002f3`; the caller list is from `git grep`.
- **A planning id in a test name or comment** (B2 R5, B3a R7): none.
- **The change doc missing decisions or deviations** (B2 R8, B3a R10):
  Step 8.3 names what goes there.
- **`mise run verify` not run** (B1a R7): Step 8.7, no `--skip`.

## 8. Deferred backlog

In [deferred-backlog.md](../../deferred-backlog.md), §2 "Data layer"
(`:62`):

- **New row "A text index row is linked by the collection row's `rowid`"**
  (Q6 (a)): `VACUUM` may give new rowids to a table without an
  `INTEGER PRIMARY KEY`; no code runs `VACUUM` today; a change that adds it
  must rebuild every text index (set each `_text_index.key_version` to 0,
  then `create-collection`). Target `TBD`. Link
  `crates/data_db/src/sqlite/text_index.rs`.
- **New row "`$search` returns rows in `id` order, not by relevance"**
  (D-B4a-9): no `bm25` ranking; a short term can match many rows, which
  are all read and sorted. Target M08B B4b or M08F (exchange relevance,
  task.md Y4). Link `crates/data_db/src/text_match.rs`.
- **New row "`$search` is not available in `aggregate` or in policy
  caveats"** (D-B4a-8). Target `TBD`. Link `crates/data_db/src/filter.rs`.
- **New row "A text field holds one string"**: a list of strings (for
  example author translations) cannot be indexed as one field; a guest
  joins them into one string. Target M08B B4b (decide there). Link
  `crates/data_db/src/sqlite/text_index.rs`.
- **New row "A text index rebuild blocks the service's writer"**
  (D-B4a-10): it runs inside `create-collection` on the single writer.
  Target `TBD`. Link `crates/data_db/src/sqlite/text_index.rs`.

§2 row "Full MongoDB operator compatibility" stays (B4a adds one operator,
not MongoDB's `$text`). §11 row "create-collection's declared indexes are
never used by query" (`:565`) stays: B4a does not change how paths are
bound. §11 row "The directory text index is ASCII-folded" (`:591`) stays
for B4b. No new `TODO` or `FIXME` marker. If an owner answer is not the
recommendation, drop or change the matching row.

## 9. Living docs to update at close-out

- **`docs/system-requirements-spec.md:1045`** ([PLT-DAT], "Operations &
  Queries"): add `$search` to the operator list, and one sentence: a
  collection may declare text fields; the host keeps an FTS5 index of their
  search keys and `$search` matches every word of a query. **`:1046`**
  (the Envisioned note): remove "Native full-text search operators"; keep
  "aggregation pipelines over logical views".
- **`docs/system-architecture.md:2761`** (Resource Model, "Collection"):
  the schema is a name, a list of indexes and a list of text fields.
  **After `:2768`** (Query & Aggregation Model): a bullet "Text search":
  the two FTS5 tables, the search key from `syneroym-text-lang`, short and
  long terms, every word required, `id` order, no scan, the index kept in
  the same transaction as the write, and that `$search` is not in
  `aggregate`. **`:568`** (crate list): `syneroym-text-lang` holds the
  language tag type and the search key; `data_db` uses it for text fields.
- **`docs/planning/traceability-matrix.md:34`** (`[PLT-DAT]`, structured
  data): add "text search over declared fields (FTS5)" and its evidence
  (`text_index` tests, parity `both_builds_text_search_round_trip`).
- **`docs/TERMINOLOGY.md`**: a new row "Search key" after "Language tag"
  (`:72`): the form of a text that search compares, made by
  `syneroym-text-lang`; today NFC and lower case.
- **ADR-0007**: the amendment (Q7).
- **`docs/developer-guide.md`**: no change (section 2.2 point 15).
- **The change doc**, **task.md**, **status.md**: Step 8.

`docs/roym-integrated-experience-spec.md`: no change in B4a; B6 updates it
(task.md, B6).

## 10. Exit criteria

B4a is done when all of these are true:

1. The owner answered Q1 to Q7, and sections 3 to 9 match the answers.
2. The Step 2 checks pass on the `bundled-sqlcipher` build, or the owner
   decided what to do about the one that failed.
3. A collection with text fields keeps both FTS5 tables right through
   `put`, `patch`, `delete`, `delete-many`, `batch-mutate` and `create`,
   including rollback.
4. `$search` finds Devanagari, Tamil and Latin words, NFC-equivalent forms,
   short words by word prefix, and needs every word; it is refused where
   D-B4a-8 says.
5. No `$search` query plan scans the collection table.
6. Both builds give the same `$search` result (parity test).
7. Every `CollectionSchema` builder sends `text_fields`, every guest is
   rebuilt, and every Roym parity test passes.
8. The six component sizes are reported and within section 7.6.
9. ADR-0007, the change doc, the living docs, the backlog, task.md and
   status.md are updated as in sections 8 and 9.
10. The capped test files are at or under their caps, and the caps are
    lowered where a file got shorter.
11. `mise run verify` passes with no `--skip`.
12. One PR.

## 11. What B4a does not do

- No transliteration and no Latin folding: the key is NFC and lower case
  only (B4b).
- No Roym change beyond `text_fields: []`: the directory still uses
  `normalize_text` and `$regex`, and still refuses a non-Latin query
  (B4b).
- No change to chat search in `crates/conversation` (B4b).
- No contact search (B4b).
- No ranking: rows come in `id` order (backlog row).
- No `$search` in `aggregate`, in policy caveats or in `query-raw`.
- No text field that holds a list of strings.
- No way to remove a text index except `drop-collection` (Q2 (a)).
- No Hub, `roymctl`, e2e or demo change.
- No migration: the product is not released, and no existing database has a
  text index.

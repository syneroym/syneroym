---
status: draft
living-docs-touched:
  - docs/planning/deferred-backlog.md
  - docs/system-architecture.md
  - docs/system-requirements-spec.md
  - docs/planning/traceability-matrix.md
  - docs/developer-guide.md
  - docs/demo/demo.md
  - docs/TERMINOLOGY.md
demo:
  - crates/substrate/tests/e2e/demo/40-roym-languages.spec.ts
---
# Roym multilingual foundation

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change lets people use Roym in their own language. It also lets two people who speak different languages trade with each other. People can mix languages and scripts, for example Hindi typed in Latin letters, in chat, in contact names and in search. It is built as **M08B**, right after [Roym open foundation](../roym-open-foundation/change.md) (M08A) and before the [Value Exchange](../roym-value-exchange/change.md) work, so every screen and record that M8 adds is multilingual from the start.

Terms used here:

- **Roym:** the SynApp for local commerce and chat. See [roym-integrated-experience-spec.md](../../../roym-integrated-experience-spec.md).
- **SynApp, SynOrg, substrate:** see [TERMINOLOGY.md](../../../TERMINOLOGY.md).
- **Hub:** the Roym web UI in the browser (`crates/roym_web/ui`).
- **Signed record:** a Roym record (`profile`, `listing`, `request`, `quote`, and others) inside a signed envelope. The list is `RECORD_TYPES` in `crates/roym_core/src/record.rs:18`.
- **Language tag:** a code for a language, in [BCP 47](https://www.rfc-editor.org/info/bcp47) form. Examples: `hi` (Hindi), `ta` (Tamil), `mr` (Marathi), `en` (English), `hi-Latn` (Hindi written in Latin letters, often called Hinglish).
- **Taxonomy:** the shared list of category codes, with a label for each code in each language.
- **String catalog:** the file of all UI text for one language.
- **Machine translation (MT):** text translated by software, not by the author.
- **Script:** a writing system, such as Devanagari, Tamil or Latin. A BCP 47 tag can name it (`hi-Latn` is Hindi in Latin letters).
- **Transliteration:** writing the same words in another script, by sound (`राम` and `ram`). It is not translation.
- **Search key:** a form of a text that search compares instead of the text itself: Unicode NFC, lower case, and a Latin transliteration for non-Latin scripts.
- **Translation hook point:** a place in a Roym flow where the translation interface is called. With no engine, the call does nothing.

## Problem

Roym users in India speak many languages. Many people want to use Roym fully in their own language, with others who speak it. People also need to trade across languages. A Tamil-speaking buyer may need a Hindi-speaking plumber.

Today Roym is English only:

- The Hub has hard-coded English strings. There is no string catalog. The 41 Hub source files set visible text in about 420 places: literal `textContent` assignments, calls of the `text()` helper, `field()` labels, and `placeholder` and `title` values. `index.html` says `lang="en"`.
- Dates use `toLocaleString()` with the browser default (`screens/listings.ts:487`). Money shows as `35.50 INR` (`money.ts:66`, `formatMinor`), not as `₹35.50` with Indian digit grouping.
- `ProfilePayload.locale: Option<String>` exists (`crates/roym_core/src/person.rs:26`). `profile.set` stores it (`crates/roym_profile/src/app/profile_ops.rs:140`). Nothing reads it.
- Listing categories are free-form English tokens of `[a-z0-9-]` (`crates/roym_core/src/listing.rs:188`). Each group picks its own words. A Hindi speaker cannot search for "plumber" in Hindi.
- No signed record says which language its free text is in.
- Directory text search keeps only ASCII letters, digits, spaces and hyphens (`normalize_text`, `crates/roym_core/src/directory.rs:225`). A query in Devanagari or Tamil script has no searchable characters. `directory.search` refuses it with `-32602` (`crates/roym_directory/src/app/search_ops.rs:265`). A listing title in Hindi is never found by text search. The [deferred backlog](../../deferred-backlog.md) already has a row for this.

The requirement `[APP-A11Y]` in [system-requirements-spec.md](../../../system-requirements-spec.md) lists localisation as "Envisioned".

## The four layers

The design has four layers. Each layer removes a part of the problem.

| Layer | What it does | Needs translation? |
| --- | --- | --- |
| **A. UI localisation** | The Hub shows its own text in the user's language, with local date, number and `₹` formats and readable Indic fonts. | No. A human translates the string catalog once. |
| **B. Language-neutral structure** | Categories, units, areas and (in M8) commitment kinds are **codes**. Each code has a label per language. | No. Two people who search, filter and compare by code need no translation. |
| **C. Language-tagged free text** | Every record with free text says which language the text is in. The author may add their own signed translations. | Only by the author, by choice. |
| **D. Machine translation** | A pluggable engine translates free text for the reader, on the reader's own node. | Yes, but only for display. Not built in v1. |

Most cross-language trade needs only layers A and B. The buyer searches by category code and area. The listing's price, slot, unit and payment terms are structured fields. Each party reads them in their own language. Free text (title, summary, chat) is the only part that needs C or D.

## Scope and non-goals

- In scope:
  - A string catalog for the Hub, with English and Hindi. Hindi proves the design works for an Indic script.
  - Local formats in the Hub: dates, times, numbers with Indian grouping, and money with the currency sign (`₹`).
  - A UI language setting in the Hub. It is a local preference, not signed.
  - A category taxonomy: codes with labels in English and Hindi. The process to add a code (see [Open questions](#open-questions)).
  - Units of measure as codes, with labels.
  - A language tag on every signed record type that has free text today: `profile`, `listing`, `request`, `quote`. And on `opinion` when M8 adds it.
  - Optional translations signed by the author, on the same records.
  - `languages` on the profile: an ordered list of language tags the person speaks. It replaces `locale`.
  - Optional language tags on chat messages, up to 3 per message, for messages that mix languages.
  - Names in any script: contact names, display names and shop names, with search keys so a name typed in one script finds the same name in another.
  - Search with SQLite FTS5 over search keys, so text search works in every script and across scripts by transliteration.
  - Attribute definitions in the taxonomy (used by the [shop](../roym-shop/change.md) change).
  - Widget and extension labels from the string catalog (see [Roym extensions](../roym-extensions/change.md)).
  - A fixed list of translation hook points (see [Translation hook points](#translation-hook-points)). All of them do nothing in v1.
  - A new substrate host interface for machine translation, with one implementation that always answers "unavailable". The Hub hides the translate action when no engine is available.
  - The agreement records which language each party viewed it in.
- Not in scope:
  - A real translation engine. It comes with or after M9A (local inference).
  - Voice input, voice output, speech-to-text.
  - Right-to-left scripts. Urdu (`ur`) is right-to-left. The Hub layout is not checked for it in this change. Urdu is recorded as known future work.
  - String catalogs for languages other than English and Hindi. Adding one later is a data change only (see ML-2).
  - Translating the taxonomy into every Indian language. This change ships English and Hindi labels.
  - Localised error text from the substrate or from `roymctl`. Only the Hub is localised.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| **UI localisation** | | |
| ML-1 | Every user-visible string in the Hub comes from the string catalog. A check fails the UI build when a string literal is set as visible text outside the catalog. | `crates/roym_web/ui/src/i18n/visible-text.test.ts` ("sets no visible text outside the string catalog", and one case for each rule of the check; `npm run build` runs it); `hindi-screens.test.ts` (each screen drawn in Hindi has no English word left, and a raw code or an English screen makes the sweep fail); `catalog.test.ts` (every Hindi message has a Devanagari letter) |
| ML-2 | Adding a new language needs only a new catalog file and taxonomy labels. No code change. A missing message in that language falls back to English, never to a raw message id. | `crates/roym_web/ui/src/i18n/i18n.test.ts` ("falls back to English for a message the language lacks"); `catalog.test.ts` (file names are canonical tags; every used id is in `en.ftl`; `hi.ftl` has the ids of `en.ftl`; taxonomy label files match base.json and have Devanagari letters in Hindi); `taxonomy.test.ts` (falls back to English for missing taxonomy labels) |
| ML-3 | A user switches the Hub language between English and Hindi. The choice survives a reload. The first choice comes from the browser's preferred languages. | `i18n.test.ts` (`pickLanguage`, the stored choice); `picker.test.ts`; `roym-hub.spec.ts` ("the Hub opens in Hindi with no choice made", browser set to `hi-IN`); story 40 |
| ML-4 | Dates, times and numbers use the Hub language's format. ₹1,00,000.50 shows with Indian digit grouping and the `₹` sign in both English and Hindi. | `format.test.ts`; `money.test.ts` ("groups rupees the Indian way in English and in Hindi") |
| ML-5 | The Hub sets `lang` on the page, and on each block of user text to that text's language tag, so the browser picks a correct font and screen readers read it in the right language. | Page `lang` only so far: `i18n.test.ts` ("sets the page language and the format locale"); story 40. Record text follows with the language-tagged records. |
| ML-6 | The Hub shows service errors by error code from the catalog. It does not show English error text from a service to a Hindi user, except as an extra detail line. | `errors.test.ts` (including a `HubError`, a browser error, a thrown string and an `RpcError` with no message); `rpc.test.ts`; `login.test.ts` (a failed sign-in in Hindi); the editor error table in `editor.test.ts` |
| **Language-neutral structure** | | |
| ML-7 | A listing's categories are codes from the taxonomy. The Hub shows each code's label in the reader's language. A code with no label in that language shows its English label. A code not in the taxonomy shows as the code itself. | `taxonomy.test.ts` ("a category shows its label in each language", "falls back to the English label", "shows an unknown code as it came", the `constructor` cases); `category_picker.test.ts`; `hindi-screens.test.ts` (each picker and each shown category in Hindi); `directory.test.ts`, `templates.test.ts` and `synorg_members.test.ts` |
| ML-8 | A Hindi-language Hub user and an English-language Hub user search with the same category code and get the same results. | `directory.test.ts` ("the same category gives the same query in English and Hindi"); `roym-hub.spec.ts` ("a category search finds the same listings in English and in Hindi") |
| ML-9 | A product listing's unit is a code from a unit list. The Hub shows its label in the reader's language. | `editor.test.ts`, `listings.test.ts` (unit options in each language); `listing/tests.rs` (the unit code rule) |
| **Language-tagged free text** | | |
| ML-10 | `profile`, `listing`, `request` and `quote` each carry a `lang` field. The signer must set it. A record with a missing or malformed tag fails validation. | |
| ML-11 | The author can add up to a fixed number of translations of the free-text fields, inside the same signed record. Each translation has its own language tag and is shown as "translated by the author". | |
| ML-12 | The Hub shows the version of a record's text that best matches the reader's languages: the original, or an author translation. It always shows which one it shows, and the reader can always open the original. | |
| ML-13 | A profile carries `languages`, an ordered list of language tags. `locale` is removed. | |
| ML-14 | A chat message may carry up to 3 language tags, main language first. A message without one is shown as "language not stated". | |
| **Machine translation** | | |
| ML-15 | A WIT interface for translation exists. Roym services can call it through the host. | |
| ML-16 | The v1 host implementation answers "unavailable" to every call. With it, the Hub shows no translate action anywhere. | |
| ML-17 | No code path sends a chat message, a record or any user text to a remote host for translation unless the node owner has configured a remote engine by name. v1 has no such configuration. | |
| **Agreements** | | |
| ML-18 | Each party's signed acceptance of an agreement records the language tag of the text it viewed, and whether that text was a machine translation. | |
| ML-19 | Before a party accepts an agreement whose free text it viewed through machine translation, the Hub shows a warning that the original, in the named language, is what binds. | |
| **Search** | | |
| ML-20 | Text search finds a listing by a word of its title in Devanagari, and in Tamil script. | |
| **Mixed languages and scripts** | | |
| ML-21 | A search for `nal wala` (Latin letters) finds a listing titled `नल वाला`, and a search for `नल वाला` finds a listing tagged `hi-Latn` with the title `nal wala`. | |
| ML-22 | Common spelling variants in Latin letters match each other (for example `naal waala` and `nalwala` match `nal wala`). The folding rules are a fixed table, tested with real samples. | |
| ML-23 | A contact, display name or shop name in any script is stored as written. Contact search finds `राम` with `ram`, and `Ram` with `राम`. | |
| ML-24 | The Hub sorts names with the browser's collation for the UI language. Names in other scripts are grouped by script, not mixed by byte order. | |
| ML-25 | A message that mixes Hindi and English words keeps its text as written. Its tags list both languages. The Hub sets `lang` from the first tag. | |
| ML-29 | A query word of 1 or 2 characters (after folding) is matched as a whole word or a word prefix through a second index, not through the trigram index. A query made only of such words still returns results, and never runs a full table scan. | |
| **Translation hook points** | | |
| ML-26 | Each hook point in [Translation hook points](#translation-hook-points) calls the translation interface. With the v1 implementation, each call returns `unavailable`, and the flow continues with the original text. A test covers every hook point. | |
| ML-27 | A translate action appears in the Hub only at a hook point, and only when `supported-targets` is not empty. | |
| **Extension and taxonomy labels** | | |
| ML-28 | Taxonomy attribute names and choice values, widget labels and extension step names come from the string catalog and show in the reader's language, with the English label as fallback. | Taxonomy attributes and choices: `crates/roym_web/ui/src/taxonomy/taxonomy.test.ts` ("an attribute and its choices show their labels", fallback and unknown cases), `crates/roym_core/src/taxonomy/tests.rs` (the attribute checks); widget and extension labels follow in M08C/M08D |

## Design

### Rules for every layer

1. **The signed original always wins.** A record means what its author signed, in the language its author tagged. A translation by software is for display only. It is labelled. It is never signed as the author's words.
2. **Translation runs on the reader's own node.** Chat is end-to-end encrypted. Roym never sends user content to a server for translation by default. A remote engine is allowed only if the node owner chooses it by name, and its output is labelled with that name.
3. **Structure before text.** If a field can be a code, it is a code. Codes are never translated. Only their labels are.
4. **No migrations.** The product is not released. Records change in place. Every signed record already has an explicit `version`. This change keeps `version` at 1 for each record type it changes, as earlier unreleased changes did. The project-wide position is no migration before release ([ADR-0018](../../../decisions/0018-service-record-visibility.md) section 5).

### A. UI localisation

**String catalog.** One catalog file per language under `crates/roym_web/ui/src/i18n/`. Each message has a stable id. The Hub looks up every visible string by id. The recommended format is [Fluent](https://projectfluent.org/) (see [Which catalog format](#which-catalog-format)).

**Language choice.** The Hub keeps the user's UI language as a local preference. The first value comes from `navigator.languages`, matched against the catalogs the build ships. The UI language is a device setting. It is separate from the profile's `languages` list, which tells other people what the person speaks.

**Formats.** All formatting goes through the browser's `Intl` API with the Hub language:

- Dates and times: `Intl.DateTimeFormat`. Replace every `toLocaleString()` call with no locale.
- Numbers: `Intl.NumberFormat`. For `en-IN` and `hi-IN` this gives Indian grouping (`1,00,000`). `unverified` in every target browser.
- Money: keep `formatMinor` as the one place that turns minor units into text. Change its output to `Intl.NumberFormat` with `style: "currency"`. Amounts stay integer minor units on the wire.
- Digits: Latin digits (`0-9`) by default in every language. Native digits (for example Devanagari `०-९`) are a later user option.

**Fonts.** Today the Hub uses `font-family: system-ui, -apple-system, sans-serif` (`style.css:2`). Android, iOS, macOS and Windows ship fonts for the major Indic scripts (`unverified` for older Android versions). The first step uses system fonts and the correct `lang` attribute (ML-5). A bundled web font is added only if a test device shows broken shaping. See [Open questions](#open-questions).

**Errors.** Services return English error text. The Hub maps known error codes to catalog messages (ML-6). 22 places in the Hub read `.message` from an error today. Each one is checked.

**Decided 2026-11-06, how the Hub does it** (the mechanism):

- **Files and loading.** `src/i18n/en.ftl` is imported with `?raw` into the main script and is always there, as the fallback. Every other `src/i18n/<tag>.ftl` is found with `import.meta.glob` and is its own script file, loaded only when that language is chosen. A new file is a new language, with no code change (ML-2). The file name is the language tag in canonical form (`new Intl.Locale(name).toString()` equals the name, not `und`). A catalog test checks this.
- **Bundles.** One `FluentBundle` for English and one for the active language, both made with `useIsolating: false`. The Hub has no right-to-left language yet, and the isolation marks would sit in every copied amount. The backlog records that they must come back with Urdu.
- **`t(id, args)`.** The English bundle is built when the module loads, so `t()` works in unit tests and before `initLanguage()` ends. It reads the active bundle, then English. It never throws. For an id that English lacks, it returns the id and calls `console.error`. Codes, ids and years are passed as strings, because Fluent groups the digits of a number argument. Fluent removes a space at the start or end of a message, so a label is followed by its value through `labelled()` in `dom.ts`, and a separator that keeps its spaces is written `{ " - " }`. An id kept in a table is a `messageId("…")` in a `Map` and is read with `lookupText()`; every other `t()` call has a literal id, and a catalog test checks that by parsing the source.
- **Format locale.** The catalog tag with region `IN` added when the tag has no region, and Latin digits (`hi-IN-u-nu-latn`). A tag that names a region keeps it (`pt-BR-u-nu-latn`). This meets ML-4 in every browser, for example an `en-US` one.
- **Formats** (`src/i18n/format.ts`). `formatNumber`, `formatDateTime` (local time with the zone name), `formatTime`, `formatUtcDate`, `formatAge` (`Intl.RelativeTimeFormat`), and `timeElement`. `formatMinor` stays the one place for money: it builds the exact decimal string from the minor units and gives it to `Intl.NumberFormat` with `style: "currency"`. A code that `Intl` rejects is shown as `<amount> <code>`. On a card, a time that ends a row is a `<time datetime>` element, whose `datetime` the Hub computes from a number. A time inside a sentence is a `$date` variable of one whole message, so a translator can move it.
- **Language choice.** `pickLanguage(stored, preferred, available)`: the stored choice, else the first `navigator.languages` entry whose language and script match a catalog, else English. A failed catalog load keeps English and draws the Hub; `lang` changes only after the load works. The choice is the `localStorage` key `roym-hub-language`. The language list is in the session bar. A choice is saved and the page reloads. When the browser does not keep the choice, the Hub does not reload and shows a one-line note beside the list.
- **Errors.** `errText()` picks the catalog message by JSON-RPC code (a `Map` of ids). The service's own English text is the `$detail` variable: an English message is often just `{ $detail }`, and a Hindi message is a Hindi sentence that ends with `विवरण: { $detail }`. An HTTP failure keeps its status as the code. A `fetch` that fails, or a reply that is not JSON, becomes an `RpcError` with code `0` and its own message.
- **Hindi text.** An agent writes it. The owner reviews `hi.ftl` before the Hub screens slice merges; a comment at the top of the file says so.
- **Pinned English.** The 8 booking sentences and 10 group sentences that the Rust crate also holds are one line each in `en.ftl`. So are the data-use notice, the address-disclosure notice and the two membership notices (`request-data-use-notice`, `quote-address-disclosure-notice`, `membership-no-instant-removal`, `membership-withheld-revocation`). Four Rust tests read `en.ftl` and check the whole `id = text` line; a comment above each line says so. The two delete notes are not read by a Rust test, but a comment above them says they match the note that the conversation service returns.

**Decided 2026-11-06, every screen and the build check:**

- **What moves.** Every string a person can see comes from `t()`: text, labels, placeholders, `title` and `aria-label` values, option labels, help text, empty states, progress and status lines, errors that the Hub makes, and the words for codes. Data does not: a name, a listing title, a DID, a message body, an amount (it goes through `formatMinor`) or a date (it goes through `format.ts`). The English text of a message is the text the code showed before. A sentence that the code built from parts is one message with variables. Improving English wording is not part of this.
- **The check.** `findVisibleText(source, fileName)` in `src/i18n/scan.ts` parses a source file with the TypeScript compiler. Rule 1 reports a string literal (or a fixed part of a template, or either branch of `?:` and `+`, or the right side of `??`, `||` and `&&`) that reaches a place where text is shown: `textContent`, `innerText`, `placeholder`, `title`, `alt`, `ariaLabel` or `label` assigned; `setAttribute` of `aria-label`, `aria-description`, `aria-placeholder`, `title`, `placeholder`, `alt` or `label`; an argument of `append`, `prepend`, `replaceChildren`, `before`, `after`, `replaceWith`, `createTextNode`, `alert`, `confirm` or `prompt`; the text arguments of the Hub helpers `text`, `field`, `labelled`, `input` and `checkbox`; the first argument of `new Option` or of `new …Error`; and a `label`, `text`, `title`, `placeholder` or `hint` key of an object. Rule 2 reports any literal outside a type with two letters and white space between them, unless it is a `console.*` argument, the right side of an assignment to `className` or `rel`, the third argument of `text()` (a class list), or an argument of `classList.add`, `.remove`, `.toggle`, `.replace`, `querySelector`, `querySelectorAll`, `closest` or `matches`. The exemptions are by position, never by the look of the text. There is no comment that turns the check off. The files read are every `src/**/*.ts` except the tests, `src/testing/`, `src/i18n/i18n.ts` and `src/i18n/scan.ts`. `index.html` is not read: its only visible text is the page title, which `main.ts` replaces with `t("hub-title")` at start-up. The check cannot see a single word kept in a variable and drawn later (a backlog row records this).
- **Where the check runs.** `npm run build` is `eslint src/ && tsc && vitest run src/i18n/visible-text.test.ts && vite build`. The Hub unit tests (the `hub-ui` gate) run the same test.
- **Hub-made errors and browser errors.** `HubError` (`src/hub-error.ts`) is a failure whose message the Hub shows as it is, with no wrapper: Hub-made text in the language of the Hub, or a service text that the Hub shows as the detail of its own message. It is not a promise that the message is translated. `MoneyInputError` and `ListingInputError` extend it. `errText` shows a `HubError` as it is, an `RpcError` by its code, and any other error or value as the `$detail` of `error-unexpected` (English is just the detail, Hindi is a Hindi sentence and then `विवरण:` and the detail).
- **Text that is signed.** The data-use notice of a request and the three default quote terms come from the string catalog, and the person signs the text that was shown or left in the field. A Hindi Hub signs Hindi text. English `roymctl` and the English Hub are unchanged. The Rust constants stay the English text, and a test checks that `request-data-use-notice` and `quote-address-disclosure-notice` in `en.ftl` equal them (the membership notices the same way, with the whole `id = text` line). The records carry no language tag until the language-tagged records land.
- **Codes shown as words.** A code (a message state, a listing status, a booking mode, a pricing model, a condition, an address disclosure, a listing block, a report category, kind and status, a credential status, a revocation status) is a `Map` of message ids read with `codeWord` (see "Code words" below). The English word is the code. A code that the table lacks is shown as it came. The option `value`, the class name and `dataset` keep the code. A category code of a credential and a verifier `reason` from the node stay as they came: the category labels are the taxonomy work.
- **One copy of each helper.** The Hub has one `text()`, one `field()`, one `input()` and one `checkbox()`, all in `dom.ts`. A shared fake `/rpc` for screen tests is `src/testing/stub-rpc.ts`.
- **Counts and sentences.** One message per sentence. A value is a variable. A count is a number argument, so a translator can add plural variants and the digits are grouped (1,000 and more). A code, an id or a DID is a string argument. A text that ends in an optional reason or has parts that depend on a value uses a Fluent selector in the same message. A space at the start or end of a label is a separate text node.
- **Sample data is data.** The sample cards of the gallery are `src/cards/samples.json`, so the check needs no exception.
- **The Hindi sweep.** `src/i18n/hindi-screens.test.ts` draws every screen and dialog in Hindi with a fake `/rpc` and fails when a Latin word of two or more letters is left. Values that a person types or another node sends hold the marker `Zq` and are removed. Codes are never marked, so a screen that shows a code as it came fails. The Latin words that may stay are a fixed list in `src/testing/latin-words.ts`, each with a reason; an item matches as a whole word only. A catalog test checks that every Hindi message has a Devanagari letter, except a short named list.
- **Shared messages.** A message is shared between two places only when the English text and the meaning are the same and the sentence is generic: `common-load-failed`, `common-save-failed`, `common-saved`, `common-cancel` and `common-remove`. Every other message has its own id under its screen's prefix, even when the English text is the same as another.
- **Code words.** `codeWord(table, code)` in `src/i18n/i18n.ts` is the one place that reads a table of code words. The only other reader is `errText`, because an error code that the table lacks has a message of its own and is not shown as it came. A code that the table lacks is shown as it came. `select` and `codeOptions` in `dom.ts` build a list of codes.
- **Message ids.** Lower case, `-` between words, the screen's prefix first (`profile-`, `contacts-`, `messages-`, `request-`, `quote-`, `group-`, `listing-`, `directory-`, `membership-`, `synorg-`, `safety-`, `backup-`, `setup-`, `login-`, `common-`, `error-`). Each `.ftl` file is ordered in sections with a `##` heading.
- **Hindi review.** An agent wrote the Hindi text. The owner reads all of `hi.ftl` before the slice merges, with a side-by-side file that marks the messages with legal or safety weight; the first comment of `hi.ftl` then says `# owner review: done <date>`. The owner did this review on 2026-11-07 and asked for no changes.

### B. Language-neutral structure

**Categories.** A category stays a token of `[a-z0-9-]`, as today. The wire shape of `ListingPayload.categories` does not change. What changes is where the tokens come from:

- Roym ships a **base taxonomy**: a data file in `crates/roym_core` with each code and its parent code (for a simple tree). The codes and structure live in the data file. Labels per language live in the Hub's taxonomy label files (`src/i18n/taxonomy/<tag>.ftl`), one per language. M08D adds attribute labels to the same label files.
- The Hub category picker offers taxonomy codes, plus a "use your own code" choice for a well-formed code not in the taxonomy. It shows labels in the user's language, and it searches those labels in the UI language, English, and the code itself.
- `ListingPayload::validate` still accepts any well-formed token. A code outside the base taxonomy is allowed, so a group can use its own codes before they are added. The Hub shows such a code as the raw code (ML-7).
- The same codes are used in `request.categories` and, in M8, in wants.

Rejected: validating categories against the taxonomy inside `validate`. Then every taxonomy change would need a new build on every node before old nodes could read new listings.

**Decided 2026-11-07, the base taxonomy:**

- **Data file and parser.** The base taxonomy is `crates/roym_core/taxonomy/base.json`, parsed and validated by `roym_core::taxonomy`. It holds 105 categories (14 groups, 91 children), 17 units, 6 payment methods, and 6 attributes across 12 categories with 16 choices.
- **Hierarchy.** Two levels: groups (no parent) and leaf categories (parent). Only leaf categories can be picked; groups serve as headings. Search by group comes with M08D.
- **Labels and joined loader.** Kept in `src/i18n/taxonomy/<tag>.ftl` (English and Hindi), with IDs following `taxonomy-category-<code-word>`, `taxonomy-unit-<code-word>`, `taxonomy-method-<code-word>`, `taxonomy-attribute-<code-word>`, and `taxonomy-choice-<attribute-word>-<choice-word>`. The loader starts catalog and taxonomy loads together with `Promise.all` and joins them when present (D-B2-4). `taxonomyMessageIds()` ensures complete coverage between the data file and label catalogs. M08D adds attribute labels to the same label files.
- **Vite JSON import.** The Hub reads `base.json` through a Vite JSON import. There is no `web` RPC method for the base taxonomy (Q2, D-B2-5).
- **Code rules across services.** Listing `product.unit`, listing `payment.methods`, quote `terms.payment_methods`, payment acknowledgement `method`, and SynOrg settings `categories` all enforce lowercase `[a-z0-9-]`. The five checks return typed error variants mapped to JSON-RPC `-32602` Invalid Params (D-B2-10).
- **Hub pickers.** A shared category picker component is used across the listing editor, request form, directory search, and SynOrg settings. It offers taxonomy categories and an own-code option for well-formed custom codes. Filter searches the UI language, English, and the code itself.
- **Shown labels.** Cards, directory search hits, and SynOrg credential displays show labels for taxonomy codes, falling back to the raw code for unknown or own codes.
- **Owner review.** The initial Hindi taxonomy labels require owner review in `docs/planning/milestones/M08B-language-and-structure/taxonomy-review.md` before merge (D-B2-12).

**Units.** `ProductDetail.unit` is a free string today (`crates/roym_core/src/listing.rs:104`, up to 32 bytes). It becomes a code (any well-formed code; the Hub offers the base unit list from `crates/roym_core`, for example `kg`, `g`, `l`, `ml`, `piece`, `dozen`, `hour`). Each code has labels per language. Using [UN/CEFACT Recommendation 20](https://unece.org/trade/uncefact/cl-recommendations) codes was considered. They are hard to read in records (`KGM`, `H87`), and Roym needs only a few units. Roym keeps its own short codes.

**Areas.** Geometric areas (`Bbox`, `Circle`) are already language-neutral. `Area::Named` has a free `label` and an optional `code` (`crates/roym_core/src/area.rs:29`). The label is free text in the record's language. The `code` is how a named area becomes language-neutral (for example an Indian PIN code). Which code system to use is not decided here.

**Payment methods.** `PaymentTerms.methods` holds short free strings today (`listing.rs:88`). They become codes (for example `upi`, `cash`, `bank-transfer`) with labels, the same way as units.

**Commitment kinds and extension fields** (M8). The Value Exchange adds resource kinds and commitment kinds. Roym extensions add schema fields, step types and widgets. Each of these is a code with catalog labels, never free text. This change states the rule. The deal core (M08C) and the extensions follow it.

**Attributes.** The taxonomy also defines attributes per category, for the [shop](../roym-shop/change.md) change. A choice attribute's values are codes with labels, like categories.

### C. Language-tagged free text

**One tag per record.** Each record type with free text gets a required field:

```text
lang: string   // BCP 47 tag of all free text in this record, e.g. "hi", "ta", "hi-Latn"
```

One tag covers every free-text field in the record. A person writes one record in one language. A tag per field was rejected: it adds a field to every string, and mixed-language records are rare. A record that mixes languages is tagged with its main language.

Free-text fields, by record, today:

| Record | Free-text fields |
| --- | --- |
| `profile` | `display_name`, `about` |
| `listing` | `title`, `summary`, `service.includes`, `service.excludes`, `service.prerequisites`, `Area::Named.label`, `payment.payee` |
| `request` | `description`, `data_use_notice` |
| `quote` | `terms.scope`, `terms.cancellation_terms`, `terms.refund_terms`, `terms.dispute_path`, `location.address` |
| `opinion` (M8) | its text fields |

Names (`display_name`, `payee`) and addresses are not translated. They are covered by the tag so the Hub can mark the text with the right `lang` (ML-5).

**Author translations.** A record may carry `translations`: a short list. Each entry has its own `lang` and the same translatable fields as the record (for a listing: `title` and `summary`). The author signs them with the record. The Hub shows them as "translated by the author". The original `lang` text still binds. The maximum number of entries is set in the plan (a small number such as 4, to keep record size bounded, as other limits in `listing.rs` do).

**Tag validation.** The Rust side is the new crate `syneroym-text-lang` (see `## Deviations`). It checks that each tag is a well-formed BCP 47 tag and stores it in syntax form: fixed letter case and order, so `HI-latn` becomes `hi-Latn`. It uses `icu_locale_core` 2.4.0 from the ICU4X project, with `default-features = false, features = ["alloc"]`. The parse functions need `alloc`. No other feature is needed (no `serde`, `zerovec` or `databake`). Decided 2026-11-05: the node does not use the CLDR form, where old codes are replaced by new ones (`iw` becomes `he`). That form needs the larger `icu_locale` crate and its CLDR data, and the cost would land in every Roym guest.

Checked: `icu_locale_core` builds for `wasm32-wasip2` and runs in `wasmtime`. It has no C code. Its normal dependencies are `litemap`, `tinystr` and `writeable`, and the `displaydoc` proc-macro. It adds about 17 KB to a test component built with the workspace release profile. The underscore form `hi_IN` is refused. The Hub uses `Intl.Locale` to parse tags. The same 15 inputs went through both parsers:

| Input | `icu_locale_core` | `Intl.Locale` (Node 22) |
| --- | --- | --- |
| `hi`, `HI` | `hi` | `hi` |
| `hi-latn` | `hi-Latn` | `hi-Latn` |
| `hi-Latn-IN`, `ta-IN`, `und`, `hi-Deva`, `zh-hant-tw` (printed `zh-Hant-TW`), `en-u-nu-deva` | syntax form, same as `Intl.Locale` | syntax form |
| `en-us` | `en-US` | `en-US` |
| `iw` | `iw` | `he` |
| `hi_IN`, `x-private`, `toolongsubtag`, empty string | refused | refused |

The only input where the two differ is `iw`: Rust keeps it and `Intl.Locale` turns it into `he`. A record from another node can carry any tag, so the Hub follows two rules:

1. The Hub wraps every `new Intl.Locale(tag)` in `try`/`catch`. A throw means "unknown tag": the Hub shows the raw tag and does no best-match.
2. Language matching in the Hub compares both the stored tag and its `Intl.Locale` form. A stored `iw` then matches a reader whose browser asks for `he`.

The [deferred backlog](../../deferred-backlog.md) records that tags are not CLDR-canonicalised.

**Profile languages.** `ProfilePayload.locale: Option<String>` is replaced by:

```text
languages: Vec<String>   // BCP 47 tags, most preferred first, at most 8 (limit set in the plan)
```

The Hub shows it on the profile ("Speaks Hindi, Marathi"). The Hub may use it to rank author translations and, later, to offer MT.

**Chat messages.** A message is not a signed record. The conversation `message` WIT record has a `content-type` (`crates/wit_interfaces/wit/conversation/`). This change adds an optional list of up to 3 language tags next to it, main language first. People in India often mix languages in one message, so one tag is not enough. A signed record still has one tag: its main language. The Hub sets it from the UI language, and the sender can change it. Putting the tag inside `content-type` as a parameter was rejected: it is not a standard parameter, and code that compares content types would break.

### D. Machine translation

**Host interface.** A new WIT package under `crates/wit_interfaces/wit/translation/`, in the style of `signing`. A first draft:

```wit
package syneroym:translation@0.1.0;

interface translation {
    variant translation-error {
        /// No engine on this node. The v1 implementation always returns this.
        unavailable,
        /// The engine cannot translate between these two languages.
        unsupported-pair(tuple<string, string>),
        /// The text is longer than the engine accepts.
        too-long(u32),
        engine-failed(string),
    }

    record engine-info {
        /// Shown to the reader next to every translation.
        name: string,
        /// True when the text leaves this node.
        remote: bool,
    }

    record translated {
        text: string,
        /// The source language the engine used (stated or detected).
        source-lang: string,
        engine: engine-info,
    }

    record detected {
        lang: string,
        /// 0..=1000.
        confidence: u16,
    }

    /// Empty when no engine is configured. The Hub hides the translate
    /// action when this is empty.
    supported-targets: func() -> list<string>;
    detect-language: func(text: string) -> result<list<detected>, translation-error>;
    translate: func(text: string, source-lang: option<string>, target-lang: string)
        -> result<translated, translation-error>;
}
```

The exact names are set in the plan. The rules are fixed here:

- The host owns the engine. A guest cannot choose a remote engine.
- The v1 implementation returns `unavailable` from every call and an empty list from `supported-targets`.
- Every successful result names its engine and says whether it was remote. The Hub labels every translation with this.
- The output is a plain string for display. No API takes a translation as input to signing.

**How the Hub reaches it.** The Hub calls a local-only Roym method, and that service calls the host interface. Which Roym service holds the method is set in the plan. `web` is the likely home, because it already serves the Hub. The method is local-only (`roym_core::admit::require_internal`), like every Roym method except the four `directory` methods.

**Later engines.** Two kinds are expected:

1. A local model on the node. This fits the M9A local inference work ([meta-implementation-plan.md](../../meta-implementation-plan.md), Milestone 9A).
2. A remote service that the node owner chooses by name in the substrate config. Its results show a label such as "translated by `<engine name>`, sent off this device".

Neither is built in this change.

### Agreements across languages

Each party agrees to the original text, in the language the author tagged. Structured fields (price, currency, slot, unit, category, commitment kinds) need no translation. Each party sees them in its own UI language.

For free text, each party's signed acceptance records what that party viewed:

```text
viewed_lang: string          // the language tag of the text shown
viewed_via_mt: bool          // true if it was a machine translation
```

Today an `agreement-receipt` is made of halves, one signed by each party (`ReceiptHalf`, `crates/roym_core/src/transaction.rs:220`). The two fields go into the payload each party signs. The deal core (M08C) changes the canonical bytes of the quote and the receipt for selective disclosure (see the [Value Exchange](../roym-value-exchange/change.md) change, "Agreements made of commitments"). This change adds these fields first. M08C keeps them.

Before acceptance through MT, the Hub shows: the original binds, it is in the named language, and the shown text is a machine translation (ML-19). An author translation (layer C) is not MT. It is signed by the author, so `viewed_via_mt` is false. The original still binds.

### Search

Category search is language-neutral after layer B. Text search is not. Today:

- The directory index stores an ASCII-folded `text` field. A search builds a `$regex` filter. `data_db` compiles `$regex` to `json_extract(payload, ?) LIKE '%pattern%'` (`crates/data_db/src/filter.rs:286`). There is no SQLite FTS table. So the SQLite FTS tokenizer question does not apply today.
- `normalize_text` drops every non-ASCII character. So Devanagari and Tamil text is not indexed, and such a query is refused.

**Decision (2026-10-26): search moves to SQLite FTS5 over search keys, in this change.**

- **Search key.** Each indexed text is turned into a search key: Unicode NFC, lower case, then a Latin transliteration for every non-Latin script, then a loose folding of Latin spelling (for example long and short vowels fold together, doubled letters fold to one). Combining marks (Unicode `Mn` and `Mc`, such as Devanagari vowel signs and the virama) are kept until the transliteration step, so words do not break. The index stores both the NFC original and the key. NFC comes from `icu_normalizer` 2.4.0, with `default-features = false, features = ["compiled_data"]`. It is already in the host build (through `url` and `idna`) and comes from the same project as the tag crate. It builds for `wasm32-wasip2`. It gave the expected result on six test inputs, including the Devanagari nukta forms and the Tamil two-part vowel sign. `unicode-normalization` 0.1.26 gave the same results but was larger (a test component grew by 124 KB, against 69 KB for `icu_normalizer`). A guest that uses only the tag type does not pay for NFC code it never calls: with a library that has a public `nfc()` calling `icu_normalizer`, a test guest that calls only the tag function was the same size (93,305 bytes) with and without `nfc()`. A guest that also calls `nfc()` grew by 68,764 bytes.
- **The query gets the same treatment.** A query in Devanagari and a query in Latin letters both become the same kind of key. So `nal wala`, `naal waala` and `नल वाला` meet (ML-21, ML-22).
- **FTS5 with the trigram tokenizer.** It works for any script, with no word list per language, and finds a word inside a longer word. It replaces the `$regex` and `LIKE` path for text search. Category, area and attribute filters stay structured.
- **Short words.** The trigram index cannot match a query term shorter than 3 characters, and short transliterated words are common (`ek`, `do`, `na`). So each text field is also indexed in a second FTS5 table with the `unicode61` word tokenizer and a prefix index for 1 and 2 characters, over the same search keys. Search keys are in Latin letters, so word tokens work for every script. A query term of 1 or 2 characters goes to this index as a whole-word or prefix match. Longer terms go to the trigram index. A query with only short terms never falls back to a scan (ML-29).
- **The transliteration table** is a fixed table in `syneroym-text-lang`, one per script, starting with Devanagari and Tamil. It is lossy by design: it only has to make two spellings of the same word meet. ISO 15919 is the starting point. The folding rules are tested with a set of real spellings collected for the test data. After a survey of seven crates (2026-11-05), we decided to write our own tables. The rules were: pure Rust, builds for `wasm32-wasip2`, allowed license, covers Devanagari and Tamil, recent release, fixed output, stable output over time. One crate, `lipilekhika`, passed all seven (rule 7 from a summary of its changelog only; its 1.1.0 release broke the API). We still write our own tables. Its output is close to ISO 15919, but it differs where we want the standard form (`kzilā` for `क़िला`, `cheṉṉai` for `சென்னை`, and `ṁ` for Tamil `ம்`). Our tables give the ISO form directly. Every stored search key depends on the exact output, so we want it to change only in a reviewed change to our own table, not with a crate upgrade.
- The SQLite build used by `data_db` (`rusqlite` with `bundled-sqlcipher`) has FTS5 and the trigram tokenizer enabled. The chat store already creates a trigram table (`crates/conversation/src/store/schema.rs:115`). SQLite always runs in the host, so this one build serves both the native and the WASM build of Roym. `unverified`: the second table, with the `unicode61` tokenizer and `prefix='1 2'`. Nothing in `crates/` uses it today. The plan for the full-text option checks it first.
- **A guest cannot create an FTS5 table today.** Guests reach storage only through `data-layer` collections. `execute-ddl` is unreachable from a service method (deferred-backlog §11). So `data-layer` gains a full-text option on a collection (the fields to index, kept in sync by the host) and a text-match filter operator. This is a substrate change, and it is planned first.
- The index shape changes. The product is unreleased, so the index is rebuilt in place.

The same search keys serve the conversation search (`conversation.search`) and the contact list.

### Names in any script

A name (contact, display name, shop name) is stored exactly as written, in any script. Next to it, the node stores its search key (see [Search](#search)). Contact search and the contact picker compare keys, so `ram` finds `राम` (ML-23). The Hub sorts names with `Intl.Collator` for the UI language, and groups names by script (ML-24). A name is never translated. The reader's node may show a transliteration next to a name in a script the reader does not read. This is a display choice, not a signed field.

### Translation hook points

The translation interface is called at these points, and at no others. With the v1 engine, every call returns `unavailable`, and the flow uses the original text (ML-26).

| Hook point | What is translated | Where it runs |
| --- | --- | --- |
| Search query | the query text, into the languages of the indexed records | the searching participant's node, before the query is sent |
| Record display | free text of an offering, want, storefront, quote | the reader's node |
| Chat message | the message text | the reader's node |
| Widget answer | free-text fields of a widget answer or step record | the reader's node |
| Opinion | the opinion text | the reader's node |
| Agreement review | the free text of an agreement before acceptance, with ML-18 and ML-19 | the reader's node |

A translation is always labelled with its engine and is never signed (rule 1 above).

### Which catalog format

Recommended: **Fluent** (`@fluent/bundle`), from Mozilla.

- It lets a translator add plural and gender variants in one language without a code change. Hindi verbs agree with the gender of the subject. English does not. With Fluent the Hindi file can have the variant and the English file does not need it.
- The file format is plain text and easy for non-developers to read and edit.
- It is mature. Firefox uses it.
- It has no framework dependency. The Hub has none either.

Considered:

- **ICU MessageFormat 1** (for example `intl-messageformat`): widely used. Its syntax is harder for translators. A gender or plural variant needs the developer to put the selector in every language's message.
- **Unicode MessageFormat 2**: the new standard, and partly based on Fluent's ideas. Browser support (`Intl.MessageFormat`) is still a proposal (`unverified` as of this writing). A later move from Fluent to MF2 is a file conversion.
- **A plain JSON map of id to string**: simplest, but it has no plurals or gender. Every such case would become code in the Hub.

Checked: `@fluent/bundle` 0.19.1 (Apache-2.0, no runtime dependencies) adds 4,205 bytes after `gzip -9` to the Hub JavaScript (31,769 to 35,974 bytes; 11,443 bytes more before `gzip`). The limit was 25 KB after `gzip -9`, so Fluent stays. The built code has no `eval`, `Function(...)` or `new Function`, so it works with the Hub's Content Security Policy (`script-src 'self'`). The 25 KB limit covers the library only. The slice that adds the catalogs reports the total, library and catalogs together, because Hindi text is 3 bytes per character before `gzip`. With the English catalog in the main script and Hindi as its own script file, measured with `gzip -9`: the main script is 31,769 bytes before and 39,819 after (+8,050); the Hindi file is 4,763; together +12,813. The main script has no Hindi letter, and `bundle.tar.gz` holds both script files. After every screen moved, measured the same way: the main script is 45,160 bytes (+5,272 from 39,888 measured on the same base) and the Hindi file is 13,980.

## Rejected options

- **Translate messages on a server.** Chat is end-to-end encrypted. A server would see every message. It would also create a central service that a peer-to-peer product should not depend on.
- **Translate structured fields.** Price, slot, unit and category are codes or numbers. Translating them adds error and gains nothing. Each reader sees their labels.
- **Keep free-form categories.** Then cross-language search on category would need translation. Each group would also keep its own words for the same thing, so search across groups (M8 federation) would not match.
- **Sign machine translations.** A signature means "the signer said this". A machine translation is not what the author said. Signing it would make an agreement bind to words nobody wrote.
- **Validate categories against a fixed list in `ListingPayload::validate`.** Every taxonomy change would then need every node updated first. See [Categories](#b-language-neutral-structure).
- **A language tag per field.** See [One tag per record](#c-language-tagged-free-text).
- **Keep `locale` as one value.** Many people in India speak two or three languages. One value cannot say that.

## Open questions

- **Search over Indic scripts, and cross-script search.** Decided 2026-10-26: in scope, with FTS5 and search keys (see [Search](#search)).
- **Spelling test data.** The folding rules need real samples of Hindi and Tamil typed in Latin letters. Decided 2026-11-05: 200 word pairs per language. An agent drafts them, and the owner checks them before search is built. Drafted in `spelling-pairs.tsv` (in the M08B task folder); owner check pending.
- **Who maintains the taxonomy and its labels.** Decided 2026-11-05: the Roym project maintains the base taxonomy in the repository. A group can use its own codes at once (they show as raw codes). A group then proposes a code for the base taxonomy through the normal contribution process. Labels in a new language come with that language's string catalog. A later option: an exchange (M8) publishes labels for its own group codes.
- **The first taxonomy.** Decided 2026-11-05: a small hand-made list of local services and goods, 50 to 150 codes. No existing product taxonomy is adapted.
- **Named area codes.** Is a PIN code the right language-neutral code for `Area::Named.code`? Or a code from India's Local Government Directory? Not decided.
- **Font size and bundle cost.** If system fonts are not enough on some devices, a Noto font for each script must be bundled or loaded on demand. Size per script is `unverified`. A font file is loaded only if `index.html` also gets `font-src 'self'`; today `default-src 'none'` blocks every font.
- **Native digits.** Decided 2026-11-05: `123` by default in every language. Native digits (`१२३`) are a later user option. Users have not been asked yet.
- **Catalog format.** Decided 2026-11-05: Fluent.
- **Who checks the Hindi text.** Decided 2026-11-05: an agent writes the first Hindi catalog and taxonomy labels, and the owner reviews them before they merge.

## Tasks

- [ ] Write the M08B slice plans with exact field names, limits and WIT names.
- [ ] Hub: add the i18n module, the Fluent catalogs (`en`, `hi`), and the language setting. Move every visible string into the catalog.
- [ ] Hub: add a build check that fails on a visible string literal outside the catalog (ML-1).
- [ ] Hub: format dates, numbers and money with `Intl` and the Hub language. Set `lang` on the page and on user text.
- [ ] Hub: map service error codes to catalog messages.
- [ ] `roym_core`: add the base taxonomy, unit codes and payment-method codes, each with `en` and `hi` labels.
- [ ] Hub: category, unit and payment-method pickers use codes and show labels.
- [ ] `roym_core`: add `lang` and `translations` to `profile`, `listing`, `request`, `quote`. Validate tags.
- [ ] `roym_core`: replace `ProfilePayload.locale` with `languages`. Update `profile.set`.
- [ ] Conversation: add up to 3 optional language tags to the `message` WIT record and the stored row.
- [ ] `roym_core`: search keys (NFC, transliteration tables for Devanagari and Tamil, Latin folding) and their tests with real samples.
- [ ] FTS5 index over search keys for the directory (and later the exchange), conversation search and contacts.
- [ ] Names in any script: search keys on contacts, collation and grouping in the Hub.
- [ ] Taxonomy attribute definitions with `en` and `hi` labels.
- [ ] Call the translation interface at every hook point. A test for each.
- [ ] Add the `syneroym:translation` WIT package and the host implementation that answers `unavailable`. Link it for native and WASM builds.
- [ ] Add the local-only Roym method that calls it. The Hub hides translate when `supported-targets` is empty.
- [ ] Add `viewed_lang` and `viewed_via_mt` to each party's signed acceptance. Add the Hub warning (ML-19).
- [ ] Replace `normalize_text` and the `LIKE` search with the FTS5 path (ML-20 to ML-22). Close the matching deferred-backlog row.
- [ ] Integration and Hub tests for each ML row. Playwright e2e: a Hindi Hub user finds and books a listing written in English, by category.
- [ ] Update `[APP-A11Y]` in the requirements, the integrated spec and the architecture doc at close-out.

## Deviations

- **The language tag type and the search keys live in a new substrate crate, not in `roym_core`.** The chat store and chat search run in the host crate `crates/conversation`, which cannot depend on `roym_core`. Both chat tags and chat search need these parts in the host. So they go into `crates/text_lang` (package `syneroym-text-lang`), which `roym_core`, `data_db` and `crates/conversation` use. `data_db` computes the search key for each declared text field, so a guest sends raw text when it writes and when it queries. Owner decision, 2026-11-05.
- **The Hub shell moves with the cards.** The first plan moved only the word tables and the cards. The shell (`main.ts`: title, session bar, tab names, sign-in box and card gallery) moves too, so a Hindi Hub shows Hindi above the cards. Owner decision, 2026-11-06.
- **Card times change too, not only the four `toLocale*` calls.** Nine `toISOString()` calls in four card templates also follow the Hub format. A time that ends a row is a `<time>` element. A time inside a sentence is plain text in one whole message, because a translated sentence cannot hold an element. Owner decision, 2026-11-06.
- **Fluent isolation is off** (`useIsolating: false`). See the decisions in section A.
- **A network failure or a reply that is not JSON becomes an `RpcError` with code `0`**, so it has a catalog message. Before, the browser's own English text reached the person.
- **Different from the plan: `formatAge`.** The first plan gave `1 hour ago` for 3600 seconds. The thresholds are kept from the old `ageWords` (minutes until 90 minutes, hours until 48 hours), so 3600 seconds is `60 minutes ago`. The test uses 7200 seconds.
- **Different from the plan: dates in `membership.test.ts`.** Four expectations of this test also change, from `2027-01-14` to `14 Jan 2027`, because the membership dates use `formatUtcDate`.
- **Different from the plan: typing of `Intl.NumberFormat.format`.** TypeScript accepts a `string` only as a `` `${number}` `` template type, so `minorToDecimal` returns that type.
- **Different from the plan: more exports in `i18n.ts`.** `activateWith` and `initLanguageWith` take the loaders as a parameter, so the tests can add a language and a failing loader without a file. `cached` in `format.ts` is exported for `formatMinor`. `languagePicker` takes the reload function as a parameter, because jsdom cannot replace `location.reload`.
- **Different from the plan: one message is exempt from the "no unused message" check.** `group-add-unreachable` is a sentence the Rust crate holds. The service sends it as the error text of a `-32602` refusal, so no Hub screen shows it from the catalog. A Hindi user who adds a person who is offline reads the generic `error-invalid-input` sentence, with the English sentence after `विवरण:`. The backlog records this.
- **Different from the plan: a time that is not a date does not stop a screen.** `formatDateTime`, `formatTime` and `formatAge` give the text `Invalid Date` (`date-invalid`) for a value that is not a date, as `toLocaleString()` printed before, because one such time in a group message must not hide the rest of the thread. This also applies to the two card rows that put a date inside a sentence. `timeElement` and `formatUtcDate` still throw a `RangeError`, as `toISOString()` did.
- **Different from the plan: `common-` messages are added when first used.** The catalog test fails for a message that nothing uses, so `common-saved`, `common-save-failed`, `common-remove`, `common-load-failed` and `common-cancel` were added in the step that first draws them and not in the first step.
- **Different from the plan: a failed login with a service text has its own message.** The service text would have shown with no `विवरण:` label if it had been a plain `HubError`, because `login-failed` has no label. A new `login-error-service` (English `{ $detail }`, Hindi a sentence and then `विवरण:` and the detail) wraps it, so a Hindi person reads the label once and the English text last.
- **Different from the plan: a revocation status table.** The directory screen shows `revocation: good` or `unknown`, a code from the node. It is a table of message ids like the others (`directory-revocation-good`, `directory-revocation-unknown`), so the Hindi sweep passes. English is unchanged.
- **Different from the plan: the backup import line.** The services answer an import with a map of counts (conversation, catalog, directory), a flag (transaction) or other fields (profile), never a number, so the old code showed `[object Object]` or `true` and the first version of this change showed `{$count}`. The Hub now adds up a map of counts, and otherwise says `Imported: bundle record(s).`. This is a change of English text from `[object Object]`/`true` to a count or `bundle`.
- **Different from the plan: the Hindi sign-in messages name only the cause.** `login-error-http`, `login-error-challenge`, `login-error-service` and `login-error-methods` do not repeat "sign-in failed", because `login-failed` already says it. `login-error-http` has no Devanagari letter (`HTTP { $status }`).
- **Different from the plan: `NO_WORDS` in `catalog.test.ts`.** It holds `terms-range-separator`, `tab-synorg` and the messages that are only values and punctuation (`listing-version`, `listing-slot-line`, `safety-report-row`, `login-error-http`). There is no command message in the list, because every command sits inside a Hindi sentence.
- **Different from the plan: more entries in the list of allowed Latin words, and whole-word matching.** The sweep also needs `HTTP`, `JSON`, `ISO-4217`, `EUR` and `USD` (Hindi messages that name a protocol, a format or an example code), `did:key:...`, `item` (the unit the listing editor starts with), and `am` and `pm`, which `Intl` writes in Latin letters for Hindi times (a backlog row records this). An item matches as a whole word, so `am` does not hide the start of another word.
- **Labels in Fluent files and not in the base data file.** `crates/roym_core/taxonomy/base.json` defines codes and hierarchy without language strings. Labels live in `src/i18n/taxonomy/<tag>.ftl`, loaded per language with English fallback, keeping the data file compact and localization unified.
- **The code rule applies to five fields, not two.** Listing `product.unit`, listing `payment.methods`, quote `terms.payment_methods`, payment acknowledgement `method`, and SynOrg settings `categories` all enforce `listing::valid_token` (`[a-z0-9-]`, 1..=32 or 1..=64 bytes).
- **The category picker offers an own-code choice.** When the user types a valid code not present in the taxonomy, the picker shows an option to use that own code.
- **Category groups cannot be picked.** Only leaf categories can be selected; groups serve as visual headings. Searching by group is deferred to M08D.
- **The category picker searches the UI language, English, and the code itself.** It does not search across all installed languages, avoiding premature loading of untargeted catalogs.
- **A reader's view of another node's product unit is deferred to M08D.** In B2, the unit appears only in the listing editor; directory hits do not display product units until shop browse lands in M08D.
- **Different from the plan: `tEnglish` is exported from `i18n.ts`.** The category picker needs English fallback labels when searching and showing taxonomy items regardless of the active UI language, so `tEnglish` is exported.

## Close-out

Before merge, update the living docs (requirements, architecture, user guide) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

# M08C slice C7 — Message forwarding: implementation plan

*Reader: the developer (or AI) who implements slice C7, and the project
owner, who answered the questions in section 2.1. You do not have the
planning conversation. This plan is meant to stand alone.*

**Status: plan written, 2026-11-25, by Claude Opus 5.5. Plan reviewed,
2026-11-27, on `main` at `89d1dd23`, by Claude Sonnet 5.5 (section
"Review on main 89d1dd23" below). Second review, 2026-11-27, on `main` at
`89d1dd23`, by Claude Opus 5.5 (section "Second review" below). The owner
answered Q2 (two PRs, C7a and C7b) and Q4 (the host indexes the `text`
field) on 2026-11-27. On the same day the owner accepted the
recommendation of every other question (Q3, Q5, Q6, Q8 to Q11: (a); Q7:
(c)). C7a is merged (PR #399, 2026-11-28). The C7b section 6.2 was written
on 2026-11-28 on `main` at `79344661`: the three checks are answered, and
so is Q12 (the record check stays in the owning service, answer (a)).**

## Review on main 89d1dd23

*Reviewer: Claude Sonnet 5.5, 2026-11-27. The plan author was Claude
Opus 5.5.* The reviewer rebased the plan branch on `origin/main`
(`89d1dd23`, which holds C4a #383, C4b #384, C6a #396 and C6b #397), read
ADR-0030, the forwarding change doc, task.md, the C4 and C6 plans and
reviews, and the merged code, and checked every `file:line` in this plan.
The plan body below is already corrected. This section lists what changed
and why. Section 2.1 keeps its question numbers: Q1 is removed because the
merges answered it, and a new Q11 is added.

### Corrections of facts

| # | What was wrong or old | What it is now | Where fixed |
| --- | --- | --- | --- |
| K1 | "C4 is not merged." | C4a and C4b are merged. So are C6a and C6b. The C4 rule in task.md section 4 no longer blocks C7. | Q1 removed; Dependencies; 2.2 point 1; Step 0 |
| K2 | `send_body` was "planned" on the C4 branch. | `send_body(host, conversation: String, content_type: &str, languages: Vec<String>, body: Vec<u8>, check_reserved: F)` is on `main` at `messages.rs:299`. `widget.send` and `widget.answer` call it (`widget.rs:53`, `:97`). The name and the arguments fit D-C7-5. One change: `send_body` runs the group checks inside itself, so a signing call placed before it signs for a group the person left. D-C7-5 now splits it. | 2.2 point 1; D-C7-5 |
| K3 | Line numbers of `messages.rs`. | `send` `:263`, the reserved-type closure `:278` to `:294`, `send_body` `:299`, `history_item_to_view` `:344`, `history` `:395`, `changes` `:420`, `search` `:513`, `transcript_digest` `:552`. The `MessageView` literals are at `:349` and `:372`. | 2.2, section 4, 6 |
| K4 | Body of widgets: "C4 may have added a body envelope". | A widget body is `{widget_version, widget}` and an answer body is `{answer_version, widget_id, data}`, under their own content types `application/vnd.roym.widget+json` and `…widget-answer+json` (`roym_core/src/widget.rs:17`, `:161`, `:170`). Neither has a `forward_mode`. The chat body of D-C7-1 is a separate type. It does not reuse the widget body. It names its version field `chat_version`, as cards (`card_version`) and widgets do. D-C7-13 holds: a widget stays not forwardable (`forward` refuses at step 3). | D-C7-13 |
| K5 | `roym_transaction/src/app.rs:307` holds `resolve_principal_and_owner`. | It is at `app.rs:327`. 12 call sites in 10 files use it. C4 and C6 did not move or copy it. A sixth mapping of the same errors appeared in `as_listed_ops/answer.rs:728` (C3c). The other four copies are at `listing_ops.rs:172`, `credential_ops.rs:46`, `backup.rs:313` and `profile_ops.rs:268`. D-C7-11 now keeps the name and the signature and moves the body to `roym_core::signing`; `transaction` re-exports it, so the 12 call sites do not change. | 2.2 point 3; D-C7-11 |
| K6 | `SendInput {` literals only in tests. | `roym_transaction/src/app.rs:645` builds a `SendInput` in production code (it sends cards). It needs the new field too. Test literals: `method/tests.rs:504`, `:513`. | Step 1 |
| K7 | Highest parity scenario is below 260. | Highest is **277** (C6). C7 uses **278** to **288** for scenarios A to K. | 7.3 |
| K8 | `conversation` has 24 methods; C4 will make 27. | It has **27** (`method_contract/contract.rs:39`). C7 adds 2 (`forward`, `settings`) and the count becomes **29**. | D-C7-4; Step 2 |
| K9 | `RECORD_TYPES` ends at `step`. | It has 15 entries and ends at `commitment-failure` (`record.rs:18`). `message-origin` is the 16th. `record/tests/wire_words.rs` does not list record types; `record/tests.rs:5` loops over `RECORD_TYPES`, so no word list needs the new type. | D-C7-3 |
| K10 | `SCHEMA_VERSION` is at `app.rs:31`. | It is at `app.rs:32` and is still **4**. 4 to 5 stands. | D-C7-9 |
| K11 | Highest Hub Playwright case is 42 on `roym-hub.spec.ts`. | The highest number is 42, and it is in `roym-hub-remote.spec.ts:11`. `roym-hub.spec.ts` ends at 41. `roym-deal.spec.ts` (C6b) numbers its own cases 1 to 5. The next free number is **43**. | 6, 7.6 |
| K12 | Demo story 20 "has two nodes?" | Yes: Alice and Bob (`20-roym-setup.sh:51`, `:52`). C4 added poll steps to it. Stories are 10, 20, 30, 40, 50, 60. C7 extends story 20 and adds no story. | Q7 |
| K13 | `sha2` is not in `roym_core`; `bytes_to_hex` is at `backup.rs:232`. | `sha2` is still not in `roym_core/Cargo.toml` (workspace line `Cargo.toml:164`, `0.10`). `bytes_to_hex` is at `backup.rs:233`. | D-C7-2 |
| K14 | File sizes: `messages.rs` 537, `conversation.rs` 371, `messages.ts` 1,174, `groups.ts` 727. | `messages.rs` **559**, `roym_core/src/conversation.rs` 371, `roym_core/src/lib.rs` 47, `roym_transaction/src/app.rs` 762, `messages.ts` **1,113**, `groups.ts` **820**. Hub code in `ui/src/chat/` stays the rule. `app.rs` of `transaction` shrinks by about 30 lines. | 2.2 point 7; 6 |
| K15 | Hub anchors. | `OPEN_WORK` `messages.ts:38`; `MessageRow` `:51`; `renderThread` `:298`; the send handler `:334`; `messageElement` `:715`; `openDeleteDialog` `:1074`; `groups.ts` `doSend` `:500`, `renderThreadMessages` `:181`; tab table `main.ts:172`. Both screens now also have a poll composer (`pollComposer`) beside the text composer. | D-C7-10 |
| K16 | Living-doc anchors. | Integrated spec: "Messaging" `:693`, "What is encrypted" `:579`, "Records and what each one proves" `:471`. Developer guide: `conversation.send` block `:2571`, widgets paragraph just after it. Requirements `:291` is still right. `sync.rs` card filter `:264`. `signing.rs` tests `:257`. | 9 |
| K17 | Backlog: add a row "default forward mode per conversation". | That row already exists (section 5, from the C0 ADR). C7 must not add it again. Rows about key revocation exist ("Issuer-key revocation is not checked on any membership record", "A listing hit is never checked against a revocation list"). C7 extends them with `message-origin`. | 8 |
| K18 | Component sizes: `conversation` "813,818, so about 122 KB". | Measured on `main` `89d1dd23` (section "Sizes now" below): `conversation` is **985,469** bytes. C4a already used the +22% owner exception. A fresh +15% is 147,820 bytes (ceiling 1,133,289). See Q11. | 7.7; Q11 |

### Problems found in the design

| # | Problem | Fix | Where |
| --- | --- | --- | --- |
| D1 | **The search filter of D-C7-8 loses real hits.** The host search takes `(query, conversation, limit)` and has no offset (`conversation.wit:280`). It orders newest first and cuts at `limit` (at most 500, `store/search.rs:14`) *before* the filter runs. A Roym message body is JSON. The host indexes all of it (`is_searchable_content_type` accepts every `+json`). A query like "text", "signed" or "origin" matches every Hub message, and a short query can hit the random-looking origin string by chance. The host fills `limit` with false matches. The filter then drops them, and the real matches behind them never come back. The plan accepted "fewer rows than `limit`". The real result is "zero rows although a real hit exists". The index also grows: each `signed` message adds about 1 KB of origin text to the trigram index. A second copy of the host's search rule in `roym_core` must also be kept in step with the host. | New Q4 recommendation: the host indexes only the `text` string of a `+json` body that has one (6 call sites). Then the filter, `text_matches` and the ICU link into `conversation` are not needed. If the owner says no, Q4 (a) needs an over-fetch loop and states the loss. | Q4; D-C7-8; step 1 |
| D2 | **A short DID of 16 characters can be forged.** Every `did:key:h…` starts with the same 12 characters (the ed25519 prefix `0xed01` in z-base-32), so the first 16 characters hold only about 19 bits of the key. A person can grind a DID with the same 16 characters in seconds. Q9 (a) would show that string for an author who is not in the contacts. | Show the last 12 characters of the DID, with the whole DID in a tooltip. | Q9; D-C7-10 |
| D3 | The author name rule has no case for the reader's own DID. Bob forwards Alice's message, Alice reads it: "First written by did:key:… (not in your contacts)" is wrong. | The node tells the Hub (`origin.is_you`). The Hub shows "you". | D-C7-6; Q9 |
| D4 | Item 16 says every `text/plain` message shows a "Not forwardable" note, with "the author did not allow it". An old message, a `roymctl` message or a card has no author choice at all. The note would be false, and it would appear on every old message. A card, a widget and a system row must show nothing. | A small tag, only on a `text/plain` row and on a chat body of mode `not-forwardable`. Two texts: "Not forwardable (the author chose this)" and "Cannot be forwarded (no signed origin)". Never on cards, widgets or system rows. | 4 item 16; D-C7-10; 7.5 |
| D5 | `forward_count` as `u32`: a changed client can send `99999999999`. The whole body then fails to parse, and the Hub shows raw JSON. Also, a person can raise the count of their own message by forwarding their own forward again and again. | Read the count as `u64` and clamp it to 1,000. If the source is the forwarder's own forward, the new count is the old count (the forwarder is already counted). | D-C7-1; D-C7-7 |
| D6 | D-C7-5 signs before the group checks of `send_body`, and tells the implementer to "move that check into a small shared helper". | Split `send_body` into `check_target` (group state and the reserved type) and `deliver` (send and read back). `send_with_mode` calls `check_target`, then signs, then `deliver`. `widget.*` and `forward` keep calling `send_body`, which is now the two calls in a row. | D-C7-5 |
| D7 | Size. C4a spent +170,873 bytes on `conversation`. 85,474 of them are the `api::dispatch` state machine for three methods (task.md, C4a sizes). C7a adds 3 methods in this plan, plus the record check, plus `search_key` with its ICU data. That can pass +15% (147,820 bytes). (The second review found that the record check is already linked; see there.) | (1) One settings method instead of two. (2) The new dispatch arms use `Box::pin`. (3) Q4 host rule, so no ICU link. (4) Measure during the node steps, not only at the end. (5) New Q11 asks for the limit now. | D-C7-4; Steps 2, 3; Q11 |
| D8 | The forward line for a sender who is not a contact: in a group most senders are not contacts. A changed client that drops the count is then shown as the author's own message. | Accepted and written down (it is only a hint, like the count). The display of an honest forward is not affected, because it carries the count. | D-C7-6 |
| D9 | "An origin must be signed as a person" suggests that a delegation makes the author a known person. It does not. Anyone can make a master key and a certificate. The check proves "the holder of DID X wrote this text", not "a human you know". | The text of D-C7-3 says so. The real protection is the name rule (Q9): a name appears only from the reader's own contacts. | D-C7-3; Q9 |
| D10 | Deleting a message does not delete what others forwarded. Item 17 adds a sentence to the delete dialog but says only that "the origin stays". | The sentence also says that copies already forwarded stay. | 4 item 17 |
| D11 | Clock skew. The verifier refuses an origin dated more than 300 s in the future, and a certificate issued more than 300 s ahead (`verify.rs:185`; `delegation.rs:231`, not changeable by `VerifyOptions`). A reader whose clock is 6 minutes behind sees "origin could not be checked" until its clock catches up. Because the check runs at every read, it heals by itself. | Keep 300 s. Put the reason in `origin_error` ("dated in the future"). The Hub shows the generic note and the reason as a tooltip. A test pins the heal. | D-C7-3; 7.1 |
| D12 | No revocation list is passed to the verifier, so every origin has `revocation_status` "unknown". A stolen conversation signing key signs valid origins as the person, with no end. This is the same as for every other Roym record. | Write it into the existing backlog rows. The Hub never says "verified" about the person, only "origin checked". | 8 |
| D13 | The demo of Q7 (a) needs more than "Bob creates a group": Bob has Alice only as an incoming chat, and `group.add-member` needs a contact or an address. Alice's node must admit the group. No browser test in the repository forms a two-node group (`roym-groups.spec.ts` is one node). | New Q7 option (c): forward in the same chat. The three-node group case stays in the Rust e2e. | Q7; 7.6 |
| D14 | A `signed` message is about 1.2 KB bigger. A sealed mailbox message holds a body of about 13 KiB (`card.rs:40`); a bigger one waits for the receiver to be online. So a `signed` text over about 11.5 KiB no longer reaches an offline peer. The host limit (`max_body_bytes`, 262,144 by default) is for the whole body, and the host refuses an oversize body with `-32004`. | Record it. Step 4 measures it. No new error code is needed. | 4 item 3; D-C7-5 |
| D15 | C7a is big for one PR. C3c, C4 and C6 each split node and Hub into two PRs. C7a has five new Rust files, two methods, backup, a three-node e2e, four Hub files, 22 Hindi keys, a changed first-send path in every Playwright spec, and the demo. The reviewer proposed three PRs (node, Hub, records). **The owner answered Q2 (a) on 2026-11-27: two PRs.** The risk is review size, not safety. | Two PRs: C7a (text forward, node and Hub) and C7b (signed records). To keep C7a reviewable, push the node steps (0 to 4) as their own commits before the Hub work starts, and run the whole Playwright suite at the end. | Q2; 6 |
| D16 | `conversation.forward` did not say what happens to the signing certificate of the forwarder: it needs none, and a forward never signs. This is right and is now stated. A forward needs a checked origin, so it works even if the forwarder has no certificate. | Stated in D-C7-7. | D-C7-7 |

### Sizes now (measured on `main` `89d1dd23`)

Release `wasm32-wasip2`, one build folder (`target-c7/` in the worktree, not
committed), `cargo component build --release --target wasm32-wasip2`
for each of the seven crates, in bytes:

| Component | Bytes | Last recorded in task.md |
| --- | --- | --- |
| `conversation` | 985,469 | 985,469 (C6a) |
| `transaction` | 2,870,685 | 2,877,309 (C6a) |
| `catalog` | 960,590 | 960,590 |
| `directory` | 1,504,433 | 1,504,433 |
| `profile` | 772,748 | 772,748 |
| `scheduling` | 715,723 | 715,723 |
| `web` | 311,314 | 311,314 |

`transaction` is 6,624 bytes smaller than the C6a record. The reviewer did
not find the reason. The other six match the record. Step 0 measures again
and uses its own numbers.

### Owner questions changed

- **Q1 removed.** C4 is merged.
- **Q2** is answered by the owner (2026-11-27): (a), two PRs.
- **Q4** is answered by the owner (2026-11-27): (a), the host indexes the `text` field only.
- **Q5** now names one settings method.
- **Q6** keeps option (a). It now has a cost estimate (not a measurement) and a note that the Hub repeats the check.
- **Q7** gets a third option: forward in the same chat.
- **Q9** changes the short-name rule and adds "you".
- **Q11 is new:** the size limit of `conversation`.
- Q3, Q8 and Q10 keep their options. Their line numbers are updated.

### Order now (the rule that applies)

C4, C6a and C6b are merged, and so is C7a (PR #399, 2026-11-28). The rule
in task.md section 4, "C7 not beside C4", no longer blocks anything. No open
or planned slice in M08C edits the same files. The only other open work is
the docs worktrees of M08E (obligations), which change no code. So **C7b
can start from `main` now.** While a C7 PR is open, no other change may
edit these files, because every one of them is in a C7 PR:

- `crates/roym_conversation/src/api.rs` and `app/messages.rs`;
- `crates/roym_core/src/lib.rs`, `record.rs`, `conversation.rs`,
  `signing.rs` and `method/conversation.rs`;
- `crates/roym_web/ui/src/screens/messages.ts`, `groups.ts`, `profile.ts`,
  `i18n/en.ftl`, `i18n/hi.ftl`, and the Playwright files `hub-helpers.ts`,
  `tests/roym-groups.spec.ts` and `demo/20-roym-chat.spec.ts`;
- the host search rule (Q4 (a)): `crates/conversation/src/store/message.rs`,
  `store/backup.rs`, `store/text_index.rs`, `group.rs`, and
  `crates/wit_interfaces/wit/conversation/conversation.wit`;
- the parity scenario numbers: C7a used 278 to 288 and the Hub case 43;
  C7b takes 289 and up and case 44.

C7a is done; C7b runs next. C8 (close-out) waits for both.
M08D and later slices, and the M08E obligations module, start after C7a merges or rebase on it, because
`roym_transaction/src/app.rs` and `roym_core/src/lib.rs` are shared. A
Hub-only fix that touches none of the files above (as the Engage fix did)
may run beside C7.

**Size and PRs.** [task.md](task.md) sizes C7 as M. The owner chose two
PRs (Q2 (a), 2026-11-27). Each is one PR that passes `mise run verify`:

- **C7a — Forward mode, origin and forward of a text message (M, big).**
  The `forward_mode` in each message the Hub sends, the default-mode
  setting, the `message-origin` record, the check at read time,
  `conversation.forward`, and in the Hub the composer control, the
  first-message question, the forward action, the display rules and the
  group notice; the demo step. Criteria FW-1 to FW-6, FW-8 to FW-15. Inside
  the PR, steps 0 to 4 are the node and steps 5 and 6 are the Hub (section
  6.1). Push steps 0 to 4 as their own commits first, so that the
  reviewer can start on the node while the Hub is built.
- **C7b — Forward of signed records (S).** A `listing`, `profile` or
  `quote` travels as itself in a record-forward body. Criterion FW-7. Its
  section 6.2 was written on 2026-11-28 on `main` at `79344661`; it holds
  the three checks and the size measurement, and Q12 is answered (a).

Branch: `feat/m08a-c7-msg-forwarding` held C7a and its plan commits
(merged). C7b works on `feat/m08c-c7b-record-forward`, cut from `main`
after C7a merged. The two parts run one after the other.

**Base.** Every file path and line number below was checked on `main` at
`89d1dd23` (2026-11-27). The C7b section (6.2) was checked again on `main`
at `79344661` (2026-11-28), after C7a merged. Links are relative to this
folder.

**Dependencies.**

- C0 (the ADR) is done: [ADR-0030](../../../decisions/0030-signed-message-origins.md),
  accepted 2026-11-16 ([status.md](status.md), row C0).
- C2 is merged (C2a PR #370, C2b PR #371). C7b's forwarded quote needs it.
- C4 is merged (C4a PR #383, C4b PR #384). C4 moved the body of
  `conversation.send` into `send_body` (`messages.rs:299`), reserved the
  two widget content types, added `widget.send`, `widget.answer` and
  `widget.list`, and changed the Hub chat and story 20. C7a builds on
  these. Section 2.2, point 1 lists each place.
- C6a and C6b are merged (#396, #397). They changed `transaction`, the Hub
  deal screens and `messages.ts` (the quote form). No C6 code conflicts
  with C7.

## Second review

*Reviewer: Claude Opus 5.5, 2026-11-27, on `main` at `89d1dd23` (`main`
has not moved since the first review, so no rebase).* It checks the first
review's changes only: K2, K4 to K9, K14 and K18 in the code, every
`file:line` that the first review added, and the designs D1 to D16. The
plan body below is already corrected. Each row says what changed and why.

### Facts

| # | What was wrong | What it is now | Where fixed |
| --- | --- | --- | --- |
| S1 | K4: a widget body is `{v, widget_type, …}` at `widget.rs:15`, and the chat body's `v` "follows the same habit". | The widget body is `{widget_version, widget}`, the answer body `{answer_version, widget_id, data}` (`widget.rs:161`, `:170`), the card `{card_version, type, version, envelope, disclosures}` (`card.rs:46`). The chat body field is renamed `chat_version`, so the habit really holds. | K4; D-C7-1, D-C7-5, D-C7-7 |
| S2 | K7 says scenarios 278 to 286 (A to I). Section 7.3 has eleven, A to K. | 278 to 288. | K7 |
| S3 | D7, Q5, Q11: the record check (ed25519, delegation chain, JCS canonical form) is linked into `conversation` "for the first time", and each dispatch arm costs "about 28 KB". | It is already linked: `conversation.import` verifies the signed bundle (`roym_conversation/src/app/backup.rs:85` → `roym_core/src/backup.rs:206`, `record::verify_json`), and `conversation.install-signing-certificate` runs `verify_chain_at` (`signing.rs:111`). The 85,474 bytes of C4a were three large widget handlers inlined into `dispatch`, not a fixed cost per arm. So the new cost is the chat code and the two new handlers. The estimate drops from +100 to +180 KB to about +40 to +90 KB (not measured). The size probe of step 1 is removed (nothing new links there); measure after steps 2 and 3. One settings method stays, because one method is the simpler interface, not for its bytes. `Box::pin` must erase the type, and stays only if the measure shows a saving. | D7; Q5; Q11; D-C7-4; D-C7-5; Steps 1 to 3; 7.10 |
| S4 | D2, Q9: the last 12 characters of a DID hold 60 bits. | About 57 bits. A DID is 34 bytes in 55 z-base-32 characters, and the last character carries only 2 bits (`identity/src/substrate.rs:146`). Making a DID with the same last 12 characters takes about 2^57 key generations. That is out of reach for a casual attacker. The label "(not in your contacts)" and the full DID in the tooltip stay the real defence. 12 characters stay. | Q9 |
| S5 | D-C7-5 calls `signing::resolve_person_principal`. | `signing::resolve_principal_and_owner`, the name that D-C7-11 keeps. | D-C7-5 |
| S6 | Step 6: the compose box of `roym-deal.spec.ts` (`:291`, `:358`) meets the first-message dialog. | That box is a plain `<input>` that the test builds in `page.evaluate`. It never sends. Only `sendMessage` in `hub-helpers.ts:211` (7 calls) and the Send click at `roym-groups.spec.ts:102` go through the real composer. | Step 6 |
| S7 | 7.4: `boot_trio` nodes can send `signed`, and step 7 checks "whether it reaches an offline peer through the mailbox". | `boot_trio` (`roym_group.rs:39`) does not enrol signing; the test calls `RoymNode::enrol_signing` (`common/roym.rs:565`) on each node first. `boot_trio` has no mailbox role, so step 7 prints the body size and compares it with `MAILBOX_CARD_BODY_BYTES`; `mailbox_conversation_e2e.rs:341` already pins that a body of that size goes through the mailbox. | 7.4 |
| S8 | The status line says only Q2 is answered. | Q2 and Q4 are answered. | Status |

Every other `file:line` that the first review added was read on
`89d1dd23` and is right.

### Designs

| # | Design | Finding | Change |
| --- | --- | --- | --- |
| S9 | D1 / Q4 (a), host indexes `text` | The six call sites are right. No Roym body that exists today has a top-level string `text`: card, widget, widget answer, and the system types (membership event, group profile, deletion request) were checked. So cards, widgets, groups and the transcript digest behave as before. The digest reads the stored bodies, not the index (`store/history.rs:135`), and `catalog` does not use `conversation`. The index tables keep their schema, so there is no wire or schema change. No node holds a chat body before C7a, so no reindex is needed. Do not bump `SEARCH_KEY_VERSION` (`text_lang/src/search_key.rs:23`): it versions the key algorithm, not which text is indexed. The host crate is the right owner. One gap: the rule is general (any JSON body with a top-level `text`), so the WIT doc of `search` (`conversation.wit:277`) must say so. | D-C7-8 and step 2: the WIT doc comment, and `indexable_text` also takes over the UTF-8 test of the six sites. |
| S10 | D6, `check_target` + `deliver` | A window exists: a removal can arrive between the check and the send. It exists today in `send_body`, between `group_info` and `AppConversation::send`; signing adds one local host call to it. The host stays the last check (`send_group`, `group.rs:453`). An origin made for a send that then fails is never sent and proves nothing false. `widget.*` keep exactly their behaviour, because `send_body` is the two calls in the same order. | One sentence in D-C7-5. |
| S11 | D5, count clamp | (1) A `u64` field alone does not clamp: `serde_json` refuses a number above `u64::MAX` (it reads it as a float) and a negative number, so the whole body still becomes `Malformed`. A small `deserialize_with` is needed. (2) The count can still go up in two ways: a changed client sends any count up to 1,000 (the change doc accepts this), and two people who forward to each other add one per hop (a real hop). The own-forward rule only stops an honest Hub from raising it. | D-C7-1 rule and tests; D-C7-7 names `chat::next_forward_count`. |
| S12 | D-C7-1, lenient `ForwardMode` | "Every other string reads as `NotForwardable`" is right for a body from another node. But the same type is used in `SendInput` and `SettingsInput`, so a typo such as `"signd"` on a local call would silently send a not-forwardable message. | `ForwardMode` deserializes strictly. Only `ChatBody.forward_mode` reads leniently (`deserialize_with`). A parity case refuses a bad mode. |
| S13 | D3, `origin.is_you` | Right. `signing::owner_did` is the installation owner. `signing::install` refuses a certificate whose master is not the owner (`signing.rs:105`), so an origin's `issuer` equals `owner_did` exactly when this person signed it. A delegated key is never the author; the origin names the master DID. The same person on two nodes is "you" on both. | One sentence in D-C7-6. |
| S14 | D-C7-6, `message_views(rows: Vec<Message>)` | `history` gets `HistoryItem`s, not `Message`s, so the signature does not fit. `MessageView` already holds what the view needs (`direction`, `languages`, `content_type`, `body`, `deleted_at_secs`). | `add_forward_views(host, &mut [MessageView])`, called after the existing conversion in `history`, `changes` and `search`. `MessageView::tombstone` also clears `forward`. |
| S15 | D7, size plan | See S3. | See S3. |
| S16 | D11, D12, D14 | D11: the test and the `origin_error` reason are enough. D12: the two backlog rows are enough; the Hub says "origin checked", never "verified". D14: right, with the test change of S7. | Checked, no change. |

### Consistency

- S17. The plan still carried Q4 options (b) and (c) in section 4 (item 8
  and "Stays the same"), D-C7-8, steps 1 and 2, 7.1, 7.10, section 8,
  section 10 and section 11. Q4 is answered, so these are removed. Q4
  keeps one line for each option not chosen.
- S18. Step 2 named parity A to E and I, step 3 F to H. Search (H) and the
  group order (J) are built in step 2, and K is a forward. Now: step 2
  A to E, H, I, J; step 3 F, G, K. G also refuses a widget message, so
  D-C7-13 has a node test.
- S19. "How this is shown to an end user" said a changed app's forward of
  a not-forwardable message shows "origin could not be checked". The view
  table shows no note for a `not-forwardable` body. The text now says only
  what is true: the receiver sees the forwarder's own words, never the
  first author's name. The demo line (Q7 (c), 7.6) has the date: "First
  written by you on …".
- S20. "Order now" did not list the host files that the Q4 (a) change
  edits, nor `roym-groups.spec.ts`. Both are added. The C7b body field is
  renamed `record_forward_version`, for the same reason as S1.
- Searched and found nothing old: "two settings methods", `set-settings`
  (only as a record in D-C7-4), ICU in `conversation`, a three-PR split
  (only as Q2 (c), not chosen), the first 16 characters (only as the old
  rule in Q9), parity numbers below 278, the method counts 24 and 27 as a
  final count, Q1 as open.
- Every decision has a step and a test. Every step has a test.

### Owner questions

Q3, Q5, Q6, Q7, Q8, Q9 and Q10 are still needed, each has clear options
and one recommendation, and no recommendation changes. Q11 is still
needed as the stop rule, but S3 makes an overrun less likely; (a) stays.
The owner then accepted every recommendation (2026-11-27).

## 0. Read first, and the rules

- `AGENTS.md`. The rules this slice touches most:
  - functions at most 100 lines; match arms at most about 10 lines;
  - production `.rs` files at most 800 lines, so new code goes in new
    sibling files (`crates/roym_conversation/src/app/messages.rs` is 559
    lines; `crates/roym_core/src/conversation.rs` is 371;
    `crates/roym_transaction/src/app.rs` is 762 and gets shorter);
  - module layout: `foo.rs` beside `foo/`, never `mod.rs`;
  - comments say why, never which plan, slice or review finding;
  - import rules: types by `use`, functions through their parent module;
  - no hard-coded ports in tests (`common::alloc_ports`, `boot_trio`);
  - parity runs need a fresh `mise run build:roym`;
  - every new or changed Hindi message is listed for the owner's review.
- [The forwarding change doc](../../changes/roym-message-forwarding/change.md):
  criteria FW-1 to FW-15, and its `## Deviations`, which win over its main
  text.
- [ADR-0030](../../../decisions/0030-signed-message-origins.md). Its twelve
  decisions are fixed. This plan does not decide them again. It decides
  only what the ADR leaves to "the C7 plan": the body format, where the
  default is stored, and the display name of an author.
- [task.md](task.md) section 3 (C7) and section 1 (the component size rule).
- [The C4 plan](slice-c4-implementation-plan.md), point 14: a widget and a
  widget answer carry no forward mode, so they are not forwardable. "The C7
  plan decides whether that stays." It stays (D-C7-13). Read the C4 and C6
  review files too ([C4](slice-c4-review.md), [C6](slice-c6-review.md)): the
  findings there on stale line numbers, hard-coded counts and the parity
  harness apply here.

### Terms used in this plan

- **Chat body:** the new JSON body of a Roym text message, content type
  `application/vnd.roym.message+json` (`ROYM_MESSAGE_CONTENT_TYPE`). It
  holds the text, the forward mode, the origin (for `signed`) and the
  forward count (for a forward).
- **Inner content:** what the origin names: the content type `text/plain`,
  the message's language tags and the UTF-8 text. It is not the chat body
  itself, because the chat body holds the origin and the count.
- **Origin:** a `message-origin` record, version 1 (ADR-0030, decision 6).
- **Check:** verify the origin envelope, check that it is a
  `message-origin` version 1 signed as a person, and check that its
  `content_hash` matches the inner content. An origin that passes is
  **checked**.
- **Forward view:** the new `forward` field of `MessageView`: what the node
  found when it read a chat body.
- **Source message:** the message a person forwards.
- **Settings row:** the person's default forward mode, and whether the Hub
  has asked the first-message question. `conversation` keeps it.

## 1. What C7 builds, in one paragraph

Each message the Hub sends gets a forward mode, `signed` or
`not-forwardable`, inside a new chat body. For a `signed` message, the
`conversation` service signs a `message-origin` record as the person, with
its existing record-signing certificate, and puts it in the body. When any
node reads a chat body (`conversation.history`, `.changes`, `.search`), it
checks the origin and reports a forward view. `conversation.forward` sends
a checked, `signed` source message to another chat with the same text, the
same origin and the count plus one. It refuses anything else. The Hub shows
the mode in the composer, asks for a default before the person's first
message, offers "Forward" only on checked `signed` messages, and shows
"Forwarded. First written by … on …. Forwarded N times." A message sent
without a mode (another client, `roymctl`, the client guide) is sent as
`text/plain`, exactly as today, and counts as `not-forwardable` (ADR-0030,
decision 3). C7b then lets a person forward an offering, a profile or a
quote as the signed record itself.

## How this is shown to an end user

C7a:

- The first time a person sends a message, the Hub asks one question:
  "Can others forward your messages with your name?" It explains the two
  answers in plain words, and says that a signed message proves who wrote
  it and when, not to whom it was sent. The person picks one, or skips.
  Skip, or closing the question, means "not forwardable".
- Next to the send button, the person sees the mode for this message,
  "Signed" or "Not forwardable", and can change it for this one message.
  Their default is on the Profile tab.
- In a group, when the mode is "Signed", the composer also says: "Any
  member can show this message to anyone, with your name."
- On a signed message, the person sees "Forward". They pick a chat. The
  other person sees: "Forwarded. First written by Alice on 25 Nov 2026.
  Forwarded 1 time." From 5 forwards on it says "Forwarded many times".
- On a message that is not forwardable, there is no "Forward" button, and
  a small tag says why: "Not forwardable" when the author chose it, or
  "Cannot be forwarded (no signed origin)" for an older message or a
  message from another client. If a changed app forwards it anyway, the
  receiver sees it as the forwarder's own words. The first author's name
  is never shown.
- When the author reads a forward of their own message, it says "First
  written by you".

C7b:

- On a quote card, an offering in the directory, or a contact's profile,
  the person sees "Forward". The receiver sees the offering, profile or
  quote card, checked as today. For a quote, the Hub first warns that the
  forward shows both people of the deal. The receiver sees no prices.

How a developer can see it work:

- `cargo nextest run -p syneroym-roym-core chat` (C7a): the body format,
  the content hash, the origin check and every display rule, with no node.
- `cargo nextest run -p syneroym-substrate --test roym_forward_e2e` (C7a):
  three nodes. Alice sends a signed message to Bob. Bob forwards it into a
  group with Carol. Carol's node reports a checked origin that names Alice,
  and `forward_count` 1. `conversation.forward` refuses a not-forwardable
  message.
- `mise run demo` (C7a): story 20 shows Bob's forward on Alice's screen,
  with the line "Forwarded. First written by you …" (Q7).

## 2. Points for the owner

### 2.1 Questions

Q1 to Q11 are answered. The option the owner chose is marked **Chosen by
the owner.** The other options stay as the record of what was not chosen.
Q12, added with the C7b section, was answered (a) on 2026-11-28.

#### Q1. (Removed)

It asked when the C7a code starts. C4 is merged, so the answer is "now"
(section "Order now"). The other questions keep their numbers.

#### Q2. How many PRs? (Answered: (a), 2026-11-27)

- (a) **Chosen by the owner.** Two: C7a (everything for a text message,
  node and Hub) and C7b (FW-7, signed records). C7b shares no code with
  C7a except the conversation picker of the forward button. It touches the
  quote check (`roym_core::transaction`), the profile check
  (`roym_profile`) and the directory screen, which C7a does not. The
  reviewer judged the risk to be review size, not safety. The stop rules
  (size, origin size, history time) all fire in the node steps 1 to 4, so
  the owner is asked before any Hub work is done. Two habits keep it
  safe: push the node steps as their own commits first, and run the whole
  Playwright suite at the end.
- (b) One PR, size L.
- (c) Three: C7a (node), C7b (Hub and demo), C7c (signed records), as C3c,
  C4 and C6 were split. The reviewer first recommended it. Not chosen.

#### Q3. The chat body: which messages use it? (Answered: (a), 2026-11-27)

ADR-0030 leaves the body format to this plan. A forward mode and an origin
cannot ride in a `text/plain` body, so a new content type is needed.

- (a) **Chosen by the owner.** A message gets the chat body when the caller names
  a mode (`conversation.send` with `forward_mode`). The Hub always names
  one, so every Hub message has the mark (FW-1 and FW-13 say "marked"). A
  call with no mode is sent as `text/plain`, exactly as today. That keeps
  `roymctl roym group send`, the client guide (developer guide §6,
  `conversation.send` at line 2571), story 30 and every test that sends
  with no mode unchanged. Such a message counts as `not-forwardable`
  (ADR-0030, decision 3).
- (b) Only `signed` messages use the chat body. A `not-forwardable`
  message stays `text/plain`, with no mark. Fewer changes, but FW-1 says a
  `not-forwardable` message "has the mark", and the Hub could not tell the
  person's own choice from another client's message.
- (c) The service applies the person's default whenever the caller names
  no mode, so every message of every client gets the chat body. This
  changes the content type of every message that `roymctl` and the guide
  send.

#### Q4. Search over a chat body (Answered: (a), 2026-11-27)

The owner chose (a). The host change is part of C7a (step 2).

The host indexes the whole body of a `+json` content type
(`is_searchable_content_type`, `crates/conversation/src/store/message.rs:36`;
the index call at `:134`). For a chat body, it would index the JSON keys
and the origin envelope. A search for "text", "signed" or "message" would
then match every Hub message. A 3-letter query would also hit the random
origin string by chance. Each `signed` message adds about 1 KB to the
trigram index.

The host search is `search(query, conversation, limit)`
(`conversation.wit:280`, `host_impl.rs:411`). It has no offset. It orders
newest first and cuts at `limit`, at most 500 (`store/search.rs:14`),
before the Roym service sees a row. A filter in `roym_core` after
that call cannot give back a real hit that the host left out.

- (a) **Chosen by the owner.** A host change, small and general: the host indexes
  only the top-level string field `text` of a `+json` body that has one,
  and the whole body otherwise, as today. Widget bodies have no `text`
  field and do not change. A function `indexable_text(content_type, body)`
  replaces the `is_searchable_content_type` test at its six call sites
  (`store/message.rs:134`, `:187`, `:326`; `group.rs:424`;
  `store/backup.rs:439`; `store/text_index.rs:80`). The interface
  (`conversation.wit`) does not change, but the doc comment of `search`
  says the new rule. The change doc's sentence "the substrate
  conversation capability does not change" is corrected in Deviations.
  `conversation.search` then needs no filter, and `conversation` links no
  ICU data.
- (b) Not chosen: filter in the Roym service after the host search. It
  still loses hits behind the cap of 500 rows, and it links the ICU data
  of `syneroym_text_lang` into `conversation`.
- (c) Not chosen: accept the false matches and the lost hits.

#### Q5. Where is the default mode stored? (Answered: (a), 2026-11-27)

ADR-0030: "Not decided here, and left to the C7 plan: where the default
setting is stored."

- (a) **Chosen by the owner.** In `conversation`: a new collection
  `conversation_settings` with one row `forward`:
  `{ "default_mode": "signed" | "not-forwardable", "asked": bool }`. One new
  local method reads and writes it (D-C7-4). It is in the `conversation`
  backup bundle as a new section `settings`, so a restore keeps the choice
  and the Hub does not ask again. `SCHEMA_VERSION` 4 → 5
  (`crates/roym_conversation/src/app.rs:32`). The service that signs the
  origin keeps the setting, and every client of the node sees the same
  answer.
- (b) In the browser (`localStorage`). Lost on another device or browser,
  and another client never sees it.
- (c) In `profile`'s `settings` collection
  (`crates/roym_profile/src/app.rs:33`). `conversation` never needs it from
  there, so there is no reason for the extra service.

#### Q6. When does a node check an origin? (Answered: (a), 2026-11-27)

- (a) **Chosen by the owner.** At read time, in `history`, `changes` and `search`,
  with a pure function (D-C7-6). There is no new stored state, so nothing
  can go stale, and a restored message is checked the same way. Cost: two
  ed25519 checks (certificate and record) per chat body that carries an
  origin. The reviewer estimates 30 to 60 ms for a page of 200 on the WASM
  build, from the C6 timing of `transaction.sync` over 50 agreements
  (20 ms on WASM, each agreement with several signed records). This is an
  estimate, not a measurement. The Hub reads a full page (200 messages) at
  every thread redraw, so the cost repeats. **Stop rule:** if a
  `conversation.history` page of 200 `signed` messages takes more than
  500 ms in the WASM build (the epoch deadline is 5 s), stop and ask (then
  (b)). Step 4 prints the number. A first cheap saving, if it is needed:
  do not re-check the sender's own outgoing rows.
- (b) At arrival, in `on_message`
  (`crates/roym_conversation/src/app/inbox.rs:136`), with the result stored
  in a new collection. It needs a write per message, a row for the
  person's own sent messages too, and rules for backup and restore.

#### Q7. The demo step (Answered: (c), 2026-11-27)

Story 20 (`crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts`) has two
nodes, Alice and Bob (`demo/20-roym-setup.sh:51`, `:52`). A forward needs a
chat to go to. C4 added poll steps at the end of the story. Bob has Alice
only as an incoming chat today: Alice added Bob as a contact, Bob did not
add Alice. No browser test forms a two-node group (`roym-groups.spec.ts`
runs on one node).

- (a) Alice answers the question with "Signed" at her first message. Bob
  adds Alice as a contact, creates a group "Market", adds Alice, and
  forwards her message there. Alice's node must admit the group. Alice's
  screen shows it in the group. No third node. This adds about five steps
  and is the first browser test of a two-node group, so it is the likeliest
  step of the story to be flaky.
- (b) Add a third node (Carol) to the demo setup and forward to her. More
  natural, but the demo setup, its environment file and every story start
  get slower.
- (c) **Chosen by the owner.** Alice answers with "Signed" at her first message.
  Bob forwards her message into the same chat (FW-9 allows the same chat).
  Alice's screen shows: "Forwarded. First written by you on …. Forwarded
  1 time." It adds two steps and uses no group. It shows the display rule,
  the forward picker and the count, but not a third person. The group case
  stays in the three-node Rust test (section 7.4).

#### Q8. Attachments (FW-8) (Answered: (a), 2026-11-27)

Roym chat has no attachments today. `encode_body` says "Attachments are out
of scope for the first release, so in practice every stored body takes the
`Utf8` path" (`crates/roym_core/src/conversation.rs:121`). The WIT message
has no attachment field
(`crates/wit_interfaces/wit/conversation/conversation.wit:50`). The Hub has
no attach action.

- (a) **Chosen by the owner.** The content hash input has a list of blob hashes
  (ADR-0030, decision 6), and C7 always writes an empty list. A unit test
  proves that a different blob list gives a different hash. FW-8 is met in
  the format only. A Deviation row and a backlog row record it.
- (b) Build attachments first. Not in M08C.

#### Q9. The author's name (FW-3) (Answered: (a), 2026-11-27)

ADR-0030 leaves "the display name of an author" to this plan.

- (a) **Chosen by the owner.** "you" if the author's DID is the reader's own DID
  (the node says so: `origin.is_you`). Else the contact name, if the
  author's DID is in the reader's contacts. Else the last 12 characters of
  the DID with a leading "…" and "(not in your contacts)", and the whole
  DID in a tooltip. The name never comes from the forward itself, so a
  forwarder cannot choose what the reader sees. The first plan showed the
  first 16 characters. Every `did:key:h…` begins with the same 12
  characters (the ed25519 prefix `0xed01`, in z-base-32), so those 16
  characters hold only about 19 bits of the key, and someone can make a DID
  with the same start in seconds. The last 12 characters hold about 57
  bits (the last z-base-32 character carries only 2), so a DID with the
  same end takes about 2^57 key generations.
  Note for the Hub text: the origin proves that the holder of that DID
  wrote the text. It does not prove that the holder is a person the reader
  knows.
- (b) Always the whole DID.

#### Q10. Where is the default in the Hub? (Answered: (a), 2026-11-27)

The Hub has no Settings tab (the tab table, `crates/roym_web/ui/src/main.ts:172` to `:186`).

- (a) **Chosen by the owner.** A section "Forwarding of my messages" on the Profile
  tab (`crates/roym_web/ui/src/screens/profile.ts`, 109 lines). It is a
  setting of the person.
- (b) A new Settings tab.

#### Q11. The size limit of `conversation` (Answered: (a), 2026-11-27)

The rule (task.md section 1): a component with new record code may grow
by 15% of its size before the slice. `conversation` is 985,469 bytes on
`main` (measured, "Sizes now"), so +15% is 147,820 bytes (ceiling
1,133,289). C4a used the owner's one-time +22% exception, and the owner
said that it was "in force for C4a". C4a grew `conversation` by 170,873
bytes. 85,474 of them are the `api::dispatch` state machine, where the
three large widget handlers were inlined. C7a adds two handlers
(`forward`, `settings`) and the chat code. The record check is already
linked: `conversation.import` verifies a signed bundle
(`roym_core/src/backup.rs:206`) and the certificate install checks the
delegation chain (`signing.rs:111`). With the Q4 (a) host rule there is no
ICU data either. The second review estimates +40 to +90 KB (not
measured), so the limit is likely to hold.

- (a) **Chosen by the owner.** Keep +15%. Measure after step 2 and after step 3,
  not only at the end. If the build is over 147,820 bytes, first try a
  type-erased `Box::pin` on the two new dispatch arms (D-C7-5). If it is
  still over, stop and ask the owner.
- (b) Approve +22% now (a ceiling of 1,202,272 bytes). No early stop, but it
  invites more growth, and C4a took the same exception.

#### Q12. Where does a forward of a signed record get checked? (Answered: (a), 2026-11-28)

The C7b outline put the check in `conversation`, as a
`MessageView.record_forward` field. Measured on `main` at `79344661`
(section 6.2), the three checks add 229,778 bytes to `conversation`
before the view, the method body or any Hub code. The Q11 rule gives
`conversation` 158,505 bytes after C7a (15% of 1,056,675), so the outline
is over that limit by about 71 KB.

- (a) **Chosen by the owner, 2026-11-28.** The check stays in the service
  that owns the record, where it is today: `catalog.listing.verify`,
  `transaction.quote.verify`, and a new `profile.verify-envelope`. The
  Hub asks that service when it draws a forwarded record, and draws only
  from the verdict. `conversation` stores and returns the body only. No
  node-to-node wire change. The design is [D-C7-14](#d-c7-14-a-record-forward-is-checked-by-the-owning-service-q12-a).
- (b) Keep the check in `conversation` and accept a one-time exception,
  as C4a did: about +22% over the C7a size (a ceiling of about 1.30 MB),
  for this slice only.
- (c) Drop the quote from C7b and forward only a listing and a profile.
  The listing check alone is 118,318 bytes and the profile check 13,928,
  so this may fit. FW-7 names all three, so the criterion would need a
  change.

### 2.2 Points found while reading

1. **Where C7a builds on C4** (merged). The body of `conversation.send`
   is now `send_body(host, conversation: String, content_type: &str,
   languages: Vec<String>, body: Vec<u8>, check_reserved: F)`
   (`messages.rs:299`). It runs the group checks (`restored`, `is_member`)
   and the reserved-type check, calls `AppConversation::send`, and reads
   the message back. `widget.send` and `widget.answer` call it with
   `|_, _| None` (`widget.rs:53`, `:97`). `send` passes a closure that
   refuses six content types (`messages.rs:278` to `:294`): membership
   event, deletion request, group profile, widget, widget answer, and the
   card type inside a group. C7a adds `ROYM_MESSAGE_CONTENT_TYPE` to that
   closure. Parity scenario 207 pins the card refusal in a group.
   `group_lifecycle.rs:249` pins the notice for a send to a restored
   group. C4 raised the `conversation` method count to 27
   (`crates/roym_web/tests/method_contract/contract.rs:39`), added its
   verbs to scenario 73
   (`crates/roym_web/tests/dual_build_parity/wire_origin.rs:67`), used
   parity numbers 260 to 266, and C6 used 267 to 277. C7a takes 278 and up.
   One change to C4's code is needed: the group checks must run before the
   signing call, so `send_body` is split in two (D-C7-5).
2. **The requirements spec says Roym supports attachments.**
   `docs/system-requirements-spec.md:291`: "The system supports one-to-one
   text, attachments, private group chat …". The code has none (Q8). The
   C7a close-out corrects the sentence, because FW-8 depends on it.
3. **The signing refusal words have several copies.** The function
   `resolve_principal_and_owner` exists in `roym_transaction`
   (`crates/roym_transaction/src/app.rs:327`, with a `data.code` on each
   refusal; 12 call sites in 10 files) and, without `data.code`, in
   `roym_catalog` (`crates/roym_catalog/src/app/listing_ops.rs:159`).
   `roym_profile` (`profile_ops.rs:268`), `roym_directory`
   (`credential_ops.rs:46`), `roym_core::backup` (`backup.rs:313`) and
   `transaction`'s `as_listed_ops/answer.rs:728` map the same errors on
   their own. ADR-0030, decision 7, asks for "the same words as every
   other signing verb". C7a moves the body of the `transaction` copy (the
   one with codes) into `roym_core::signing`, keeps its name and
   signature, and re-exports it from `transaction`'s `app.rs`, so the 12
   call sites do not change. `conversation` calls it too. The other copies
   stay; a backlog row records them (section 8).
4. **`roym_conversation` has no unit tests**, and the `AppConversation` of
   `TestHost` is a stub (`crates/roym_core/src/testing/stubs.rs:169`). So
   C7a puts every rule it can into pure functions in `roym_core`
   (unit-tested), and tests the service through the parity harness (both
   builds) and the three-node test.
5. **The parity harness pins the signing clock ahead of wall-clock time**
   (`crates/roym_web/tests/dual_build_parity/fixtures.rs:68`,
   `helpers.rs:1034`: `RecordClock::Fixed(wall_now + 240)`). A check at
   read time uses `clock::now_secs()` with `VerifyOptions::new(now)`,
   which refuses a record dated more than 300 s in the future
   (`crates/signed_record/src/verify.rs:91`, `:185`). 240 s is inside 300 s
   and time only moves forward, so the pinned clock passes. Step 3
   confirms it with the first forward scenario. `forward_view` still takes
   `now_secs` as a parameter, so a unit test can set any clock.
   **A real node:** a reader whose clock is more than 300 s behind the
   author's sees "origin could not be checked" until its clock catches up.
   The certificate has its own 300 s rule (`delegation.rs:231`) that a
   caller cannot change. The check runs at every read and stores no
   verdict, so it heals by itself. A unit test pins this.
6. **An origin must still check after its certificate expires.** The
   verifier checks that the record was signed inside the certificate's
   window, not that the window is still open (`verify.rs:206`; test at
   `verify.rs:463`; `verify_chain_at` checks scope, master DID and
   `issued_at`, not `expires_at`). So a forward of an old message still
   checks. A unit test pins this for `message-origin` (section 7.1). No
   revocation list is passed to the verifier either, so `revocation_status`
   is "unknown" (as for every Roym record; backlog, section 8).
7. **`messages.ts` is 1,113 lines and `groups.ts` 820.** TS files are not
   checked by `check-file-lengths` (it reads only `*.rs`,
   `xtask/src/file_lengths.rs:398`), but an earlier review asked that
   `messages.ts` not grow (C1 R15). New Hub code goes into
   `crates/roym_web/ui/src/chat/`, which holds `message_text.ts` and
   `message_state.ts` today (`widgets/` shows the same habit).
8. **A forward needs no signing certificate.** `conversation.forward`
   copies the origin of the source. It signs nothing, so it works for a
   person who has not enrolled signing on `conversation`. Only a `signed`
   send needs the certificate.
9. **What a checked origin proves.** "Signed as a person" is a form check
   (a delegation is present and names the issuer). Anyone can make a master
   key and a certificate. A checked origin proves that the holder of the
   author DID wrote the text at the stated time. It does not prove that the
   holder is someone the reader knows. The reader's contacts decide the
   name (Q9).
10. **A signed message gives a smaller room for text.** The origin adds
    about 1.2 KB. A sealed mailbox message holds a body of about 13 KiB
    (`card.rs:40`), so a signed text over about 11.5 KiB waits for the
    receiver to be online. The host limit of 262,144 bytes (a config value,
    `store.rs:92`) is for the whole body, so a signed text near that limit
    is refused by the host with `-32004` after the service signed it. Step
    4 measures both sizes.

## 3. What C7 meets

All criteria are in the [forwarding change doc](../../changes/roym-message-forwarding/change.md).
Its Deviations (from ADR-0030) win over its main text.

| Criterion | Part | How | Main test |
| --- | --- | --- | --- |
| FW-1 | C7a | A mode in every message sent with a mode (the Hub always names one); an origin for `signed`; the payload as the Deviations say (no `author`, no `sent_at_secs`; `languages`) | `chat` unit tests; parity B, C |
| FW-2 | C7a | `conversation.forward` checks mode and origin itself (node); the Hub hides the action | parity G; `roym_forward_e2e` step 5; Hub unit test |
| FW-3 | C7a | Check at read (node); Hub line with name and date | `forward_view` unit tests; Hub unit test; story 20 |
| FW-4 | C7a | Failed check: no author in the view (node); the forwarder's own words plus a note (Hub) | `chat` unit tests; Hub unit test |
| FW-5 | C7a | The view carries `many_forwards` for a count of 5 or more (node); the Hub shows "Forwarded many times" | `chat` unit test; Hub unit test |
| FW-6 | C7a | Any change of text, languages or content type breaks the hash | `chat` unit tests |
| FW-7 | C7b | A record-forward body; `listing`, `profile`, `quote` only | C7b tests (section 6.2) |
| FW-8 | C7a | A blob-hash list in the hash input, always empty (Q8) | `chat` unit test; Deviation row |
| FW-9 | C7a | Forward to a one-to-one chat or a group, one method | `roym_forward_e2e` steps 3, 4 |
| FW-10 | C7a | First-message question; settings text | Hub unit test; Hub case; story 20 |
| FW-11 | C7a | Group composer notice; no group rule | Hub unit test |
| FW-12 | C7a | The default is stored and read (node); composer mode control, one change does not change the default (Hub) | parity A; Hub unit test |
| FW-13 | C7a | A forged forward has no checked origin (node); no forward action and a reason (Hub) | `chat` unit tests; Hub unit test |
| FW-14 | C7a | The mode is in the body, under the sender's message signature | parity B (raw body holds the mode); D-C7-1 |
| FW-15 | C7a | A forward copies the author's mode and origin; the forwarder's default is not read | parity F |

## 4. Behaviour-change audit

### Wire changes (between nodes)

1. **New content type** `application/vnd.roym.message+json` for every
   message the Hub sends (Q3 (a)). A node on an older build shows it as an
   unknown JSON message. No compatibility path: the product is not
   released.
2. **New record type** `message-origin` version 1, inside a chat body.
   Signed as the person by the `conversation` service key, with the
   certificate in the envelope's `delegation` field (ADR-0030, decision 11).
3. **Body size:** a `signed` chat body is about 1.2 KB larger than its text
   (ADR estimate: about 940 bytes for the origin, plus the JSON around it).
   C7a measures it (step 4). A short message still fits the mailbox
   estimate used for cards (`MAILBOX_CARD_BODY_BYTES` = 13 KiB,
   `crates/roym_core/src/card.rs:40`). A signed text over about 11.5 KiB
   does not (section 2.2, point 10).

Nothing else on the wire changes: `text/plain` messages, cards, group
system messages, deletion requests and the conversation envelope
(`crates/conversation/src/envelope.rs`) stay the same.

### Local method changes (one node)

4. `conversation.send` takes an optional `forward_mode` (`"signed"` or
   `"not-forwardable"`). With it, `content_type` must be absent or
   `"text/plain"`, else `-32602` on field `content_type`. With no
   `forward_mode`, nothing changes.
5. `conversation.send` refuses `content_type`
   `"application/vnd.roym.message+json"` as reserved, like the widget types
   (the closure at `messages.rs:278` to `:294`). A body over the host's
   limit (`max_body_bytes`, 262,144 by default, a config value) is refused
   by the host with `-32004`, as today. A `signed` text reaches it about
   1.2 KB sooner.
6. New methods `conversation.forward` and `conversation.settings`
   (D-C7-4, D-C7-7). The `conversation` method table grows from 27 to 29.
7. `MessageView` (`crates/roym_core/src/conversation.rs:141`) gains
   `forward: Option<ForwardView>`. It is set only for a chat body, and left
   out of the JSON when `None`. **For a chat body, `body` is the inner
   text**, not the raw JSON (D-C7-6). `content_type` stays the wire content
   type.
8. `conversation.search`: the host indexes only the `text` field of a chat
   body (Q4 (a)), so nothing changes in the method.
9. The `conversation` backup bundle gets a section `settings`;
   `SCHEMA_VERSION` 4 → 5. A bundle of version 4 is refused on import, as
   for every schema change.
10. `RECORD_TYPES` (`crates/roym_core/src/record.rs:18`) gains
    `("message-origin", 1)`.
11. A `signed` send with no usable certificate is refused with `-32602` and
    `data.code` `signing-not-enrolled`, `signing-certificate-expired`,
    `signing-certificate-stale` or `signing-no-owner`: the words
    `transaction` uses today (`crates/roym_transaction/src/app.rs:327` to
    `:357`). The refusals of `transaction` do not change: the function
    moves, its words stay.

### Visible behaviour changes (Hub)

12. Composer: a mode control in the one-to-one and group composers; a
    group notice when the mode is `signed`.
13. A first-message dialog, once per person (per node).
14. Profile tab: a section with the default mode.
15. On each chat-body message: a "Signed" tag, a "Forward" button (checked
    `signed` only) or a "Not forwardable" tag; the forwarded line; the
    "origin could not be checked" note.
16. A `text/plain` message (another client, or a message from before C7)
    shows a small tag "Cannot be forwarded (no signed origin)". It does
    not say that the author chose this, because the author chose nothing.
    The tag is drawn only on a `text/plain` row and on a chat body of
    mode `not-forwardable`. A card, a widget, a widget answer and a system
    row show no tag. Before C7 no row showed one.
17. The delete dialog of a `signed` message adds one sentence: the signed
    origin stays with everyone who has the message, and copies that others
    already forwarded stay too (ADR-0030, decision 12: "It cannot be taken
    back").
18. Search results show the inner text of a chat body (D-C7-6).

### Stays the same

- The interface of the substrate conversation capability. (Its indexing
  rule gets one case, Q4 (a), and the WIT doc comment of `search` says so.)
- `transaction` filing: it reads only card messages
  (`crates/roym_transaction/src/app/sync.rs:264`).
- Events: `conversation.message` events carry ids only
  (`crates/conversation/src/lib.rs:184`).
- `conversation.retry` resends the stored body unchanged. A retried
  `signed` message carries its first origin and is not signed again.
- Delete, admission, first-contact limits, blocks, group visibility.
- Signing enrolment: `conversation` is already in the Hub's
  `SIGNING_SERVICES` (`crates/roym_web/ui/src/session/enrolment.ts:7`) and
  in the list of `roymctl` (`apps/roymctl/src/commands/roym/signing.rs:13`).
- `roymctl`: no command changes (D-C7-12).
- Widgets and widget answers (C4) have no mode and are not forwardable
  (D-C7-13).

## 5. Decisions this plan makes

Every new input struct is `#[serde(deny_unknown_fields)]` (all new methods
are local-only). Every new output is a typed struct. Error codes go in
`data.code`, never only in the message text.

### D-C7-1. The chat body

New module `crates/roym_core/src/chat.rs`, with its tests in
`crates/roym_core/src/chat/tests.rs` (module layout rule). Callers write
`chat::…` after `use syneroym_roym_core::chat;`.

```rust
pub const ROYM_MESSAGE_CONTENT_TYPE: &str = "application/vnd.roym.message+json";
pub const CHAT_BODY_VERSION: u32 = 1;
/// The only inner content type in this version.
pub const INNER_TEXT_CONTENT_TYPE: &str = "text/plain";
/// An origin envelope longer than this is not read.
pub const MAX_ORIGIN_BYTES: usize = 8 * 1024;
/// "Forwarded many times" from this count on. The view carries it, so the
/// Hub does not keep its own copy of the number.
pub const MANY_FORWARDS: u32 = 5;
/// A larger count in a body is read as this.
pub const MAX_FORWARD_COUNT: u32 = 1_000;

/// Strict: a local method input with another string is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ForwardMode { Signed, NotForwardable }

/// As it travels. Unknown fields are ignored: another node wrote it, and a
/// later version may add fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatBody {
    pub chat_version: u32,
    pub content_type: String,
    pub text: String,
    /// "signed" -> Signed, every other string -> NotForwardable.
    #[serde(deserialize_with = "lenient_mode")]
    pub forward_mode: ForwardMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Any JSON number is read, then clamped (rules below).
    #[serde(default, deserialize_with = "clamped_count", skip_serializing_if = "Option::is_none")]
    pub forward_count: Option<u32>,
}

pub enum ChatBodyError { Malformed(String), Unsupported(String) }

impl ChatBody {
    pub fn parse(bytes: &[u8]) -> Result<ChatBody, ChatBodyError>;
    pub fn to_bytes(&self) -> Vec<u8>; // serde_json::to_vec, compact
}
```

Rules in `ChatBody::parse`:

- A body that is not JSON, or misses a field → `Malformed`.
- `chat_version != 1` → `Unsupported`.
- `content_type != "text/plain"` → `Unsupported`.
- An unknown `forward_mode` string reads as `NotForwardable`. A later mode
  never makes a message forwardable on this build. This lenient read is
  only for the body; `SendInput` and `SettingsInput` refuse an unknown
  mode with `-32602`.
- `clamped_count` reads the count as a `serde_json::Number`, because a
  plain integer field refuses a number above `u64::MAX` (`serde_json`
  reads it as a float) and a negative number, and the whole body would
  then be `Malformed`. A number above 1,000, or too big for any integer,
  reads as `MAX_FORWARD_COUNT`. Zero, a negative number or a fraction
  reads as `None`. A count that is not a number → `Malformed`. The count
  is only a hint (change doc), so a clamp loses nothing.

Rules applied later, in the check (D-C7-3) and the view (D-C7-6):

- On a `NotForwardable` body, any `origin` is ignored.
- An `origin` longer than `MAX_ORIGIN_BYTES` fails the check with the
  reason "origin too large". The length is tested before any parse.

The sender's message signature covers the chat body bytes (one-to-one: the
`DeliveryPayload` over the plain body, `crates/conversation/src/envelope.rs:92`;
group: the ciphertext, `crates/conversation/src/dag.rs:156`). So the mode
is covered (FW-14), and no new signature is needed for it.

### D-C7-2. The content hash

```rust
pub fn content_hash(
    content_type: &str,
    languages: &[String],
    body: &[u8],
    blob_hashes: &[[u8; 32]],
) -> String
```

```text
input =  b"roym-message-origin-v1"
      || u32be(len(content_type)) || content_type
      || u32be(count(languages))  || for each tag: u32be(len(tag)) || tag
      || u32be(len(body))         || body
      || u32be(count(blobs))      || for each blob: its 32 bytes
content_hash = "sha256:" + lowercase hex of SHA-256(input)
```

- `body` is the UTF-8 text. `languages` are the message's tags as stored
  (canonical form, 0 to 3, checked by `syneroym_text_lang::check_tag_list`
  in `send`, `messages.rs:263`). C7 always passes an empty blob list (Q8).
- Hex through the existing `roym_core::backup::bytes_to_hex`
  (`crates/roym_core/src/backup.rs:233`). SHA-256 through the `sha2`
  crate: `roym_core` gains `sha2.workspace = true` (workspace version
  `0.10`, root `Cargo.toml:164`). Every Roym component already links
  `sha2` through `syneroym-signed-record` (`envelope.rs:227`), so the size
  cost is the new code only.

### D-C7-3. The origin record

```rust
// crates/roym_core/src/record.rs
pub const RECORD_MESSAGE_ORIGIN: &str = "message-origin";

// crates/roym_core/src/chat.rs
pub const MESSAGE_ORIGIN_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageOriginPayload {
    pub content_type: String,
    pub languages: Vec<String>,
    pub content_hash: String,
}

pub fn origin_draft(payload: &MessageOriginPayload) -> RecordDraft;
pub fn verify_message_origin(envelope: &str, now_secs: u64)
    -> RecordVerdict<MessageOriginPayload>;
pub fn check_origin(body: &ChatBody, languages: &[String], now_secs: u64) -> OriginCheck;

pub enum OriginCheck {
    Checked { author_did: String, written_at_secs: u64, record_id: String },
    Failed { reason: String },
    Absent,
}
```

- `record.rs`: add the constant, and `("message-origin", 1)` to
  `RECORD_TYPES` (`record.rs:18`, 15 entries today, 16 after). The test
  `record/tests.rs:5` loops over the list, so it needs no change.
  `wire_words.rs` lists no record types.
- `origin_draft`: `version` 1, `record_type` `message-origin`, `subject`
  `""` (ADR-0030, decision 6; `signed_record/src/envelope.rs:61` checks
  only the maximum subject length), `expires_at_secs` `None`, `supersedes`
  `None`.
- `verify_message_origin`, in the shape of `verify_step`
  (`crates/roym_core/src/step.rs:235`) and `verdict::verify_party_record`
  (`crates/roym_core/src/verdict.rs:73`): `record::verify_json` with
  `VerifyOptions::new(now_secs)`, then refuse when: the type or version
  differs; the payload does not parse; `subject` is not `""`;
  `expires_at_secs` is set; the envelope has no `delegation` (an origin
  signed by a service key alone names no person, so "an origin must be
  signed as a person"). The verdict's `issuer` is then the person's master
  DID. This is a form check. Anyone can make a master key and a
  certificate, so a checked origin proves only that the holder of the
  author DID wrote the text (section 2.2, point 9). The name rule of Q9
  decides what the reader sees. `verify_message_origin` passes no
  revocation list (backlog, section 8).
- `check_origin`: `Absent` when the body is `NotForwardable` or has no
  origin; `Failed { "origin too large" }` over `MAX_ORIGIN_BYTES`; else
  `verify_message_origin`, then compare the payload's `content_type` with
  the body's and its `languages` with the message's, then recompute
  `content_hash` over the inner content and compare. Each failure has its
  own short reason.

### D-C7-4. The settings row and its methods

In `roym_conversation`, new file `src/app/settings.rs`. Types in
`crates/roym_core/src/method/conversation.rs`, beside `SendInput` (`:11`),
so the method shapes and parity see one type:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardSettings { pub default_mode: ForwardMode, pub asked: bool }
// The default, when there is no row: NotForwardable, asked = false
// (ADR-0030, decision 4).

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub default_mode: Option<ForwardMode>,
    pub asked: Option<bool>,
}
```

```rust
// crates/roym_conversation/src/app.rs
pub const SETTINGS: &str = "conversation_settings";
// crates/roym_conversation/src/app/settings.rs
const FORWARD_ROW: &str = "forward";
```

- `conversation.settings` (`SettingsInput => ForwardSettings`), one method
  for read and write. With no field set it only reads: no row reads as the
  default, and a storage error is `-32603`, never "the default". With a
  field set it merges the given fields into the row, writes it and returns
  it. `asked: false` is refused (`-32602`, field `asked`): the Hub asks
  only once (ADR-0030, decision 5). The first plan had a second method,
  `conversation.set-settings`. One method is the simpler interface: one
  contract row, one input type, one parity scenario.

The service never reads the setting when it sends (Q3 (a)). The Hub reads
it and passes the mode. The node keeps the setting for the Hub.

### D-C7-5. Sending with a mode

`SendInput` (`method/conversation.rs:11`) gains
`#[serde(default, skip_serializing_if = "Option::is_none")] pub forward_mode: Option<ForwardMode>`.

New file `crates/roym_conversation/src/app/chat_send.rs`:

```text
pub(crate) async fn send_with_mode(host, input, mode) -> Reply<SendOutput>
  if input.content_type is Some(ct) and ct != "text/plain":
      -32602, field content_type
  (languages are checked in send before this, messages.rs:264)
  messages::check_target(host, &conversation, ROYM_MESSAGE_CONTENT_TYPE, |_, _| None)?
      // group state only; no signing for a group the person left
  origin = None
  if mode == Signed:
      payload = MessageOriginPayload {
          content_type: "text/plain", languages,
          content_hash: chat::content_hash("text/plain", &languages, text, &[]) }
      (principal, _owner) = signing::resolve_principal_and_owner(host, now)?   // D-C7-11
      origin = Some(AppSigning::sign_record(host, chat::origin_draft(&payload), principal)
                    .await.map_err(internal_error)?)
  body = ChatBody { chat_version: 1, "text/plain", text, mode, origin, forward_count: None }
  messages::deliver(host, conversation, ROYM_MESSAGE_CONTENT_TYPE, languages, body.to_bytes())
```

- `messages::send` calls `send_with_mode` when `forward_mode` is `Some`,
  and keeps its path unchanged when it is `None`.
- The group checks must run **before** the signing call, so a send to a
  group the person left signs nothing. C4's `send_body` (`messages.rs:299`)
  runs them inside itself, after the body is made. So split it in two:
  `check_target(host, conversation, content_type, check_reserved) ->
  Result<bool /* is_group */, Response>` (the `group_info` match and the
  reserved check, lines `:310` to `:326` today) and `deliver(host,
  conversation, content_type, languages, body) -> Reply<SendOutput>` (the
  send and the read-back, lines `:328` to `:341`). `send_body` becomes the
  two calls in a row, so `send`, `widget.send`, `widget.answer` and
  `conversation.forward` keep their behaviour and their order of checks.
  The chat type is sent with `|_, _| None`, because `send_with_mode` is its
  one sanctioned sender (a raw `send` of it is refused by the closure of
  `send`).
- A removal can still arrive between `check_target` and `deliver`. That
  window exists today in `send_body` (between `group_info` and
  `AppConversation::send`); signing adds one local host call to it. The
  host's send stays the last check. An origin made for a send that then
  fails is never sent.
- Size (Q11): only if the measure after step 2 or 3 is over the limit,
  wrap the two new handlers in the `service_methods!` table with a small
  `async fn` that awaits a type-erased future
  (`Pin<Box<dyn Future<Output = Reply<_>> + '_>>`), so their code is not
  inlined into `dispatch`. A plain `Box::pin(fut).await` keeps the
  concrete type and may still inline. Keep the wrapper only if a second
  measure shows a saving.
- A signing failure sends nothing. There is no fallback to
  `not-forwardable` (ADR-0030, decision 7).

### D-C7-6. The forward view (read side)

Pure part, in `roym_core::chat`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardView {
    pub mode: ForwardMode,
    /// `signed`, and the origin is checked. The Hub offers "Forward"
    /// exactly when this is true.
    pub forwardable: bool,
    /// Only when the origin is checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginView>,
    /// Why a `signed` body's origin did not check, or why the body could
    /// not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_error: Option<String>,
    /// 0 for a message that is not a forward.
    pub forward_count: u32,
    pub many_forwards: bool,  // forward_count >= MANY_FORWARDS
    /// Draw the "Forwarded" line.
    pub forwarded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginView {
    pub author_did: String,
    pub written_at_secs: u64,
    pub record_id: String,
    /// The author is the reader. The Hub then says "you" (Q9).
    pub is_you: bool,
}

pub fn forward_view(
    body: &ChatBody,
    languages: &[String],
    sender_person_did: Option<&str>,
    reader_did: Option<&str>,
    now_secs: u64,
) -> ForwardView;

/// The view of a chat body that could not be read.
pub fn unreadable_view(reason: &str) -> ForwardView;
```

Rules of `forward_view` (each row is a unit test, section 7.1):

| Body and check | `forwardable` | `origin` | `origin_error` | `forwarded` |
| --- | --- | --- | --- | --- |
| `not-forwardable` | false | none | none | false |
| `signed`, checked, count 0, sender DID = author | true | yes | none | false |
| `signed`, checked, count ≥ 1 | true | yes | none | true |
| `signed`, checked, count 0, sender DID known and ≠ author | true | yes | none | true |
| `signed`, checked, count 0, sender DID unknown | true | yes | none | false |
| `signed`, failed or absent (any count) | false | none | yes | false |

The last row is FW-4: the Hub shows the message as the sender's own words,
with the note. `origin` is `None`, so no author name can be drawn. The
fourth row covers a forward that a changed client sent with no count: it
still names its author. The fifth row is a known gap: in a group, most
senders are not the reader's contacts, so a changed client that drops the
count from a forward is shown as if the sender wrote it (with a checked
origin that names the author). An honest forward always carries the count.
The count and this row are hints, as the change doc says; the origin is
the proof.

Service part, new file `crates/roym_conversation/src/app/view.rs`:

- `pub(crate) async fn add_forward_views<H: AppHost>(host: &H, views: &mut [MessageView])`.
  It runs on the views that `history`, `changes` and `search` already
  build, so it fits all three (`history` gets `HistoryItem`s, not
  `Message`s). For a view whose `content_type` is
  `ROYM_MESSAGE_CONTENT_TYPE` and that is not deleted: parse `view.body`
  (a `+json` body is stored as UTF-8, `conversation.rs:124`). On success,
  `view.body = Some(text)` and `view.forward = Some(forward_view(..))`.
  On an error, set `view.body = None` (the Hub then draws its "no body"
  text, never the raw JSON of a body that a changed client made) and
  `view.forward = Some(unreadable_view("message body could not be read"))`.
- `sender_person_did`: for an outgoing view, `signing::owner_did(host)`
  (`crates/roym_core/src/signing.rs:240`); for an incoming view, the
  `contacts_map` (`crates/roym_conversation/src/app.rs:144`) by `author`.
  Each is read **once per page**, and only if the page holds a chat body.
- `reader_did`: the same `signing::owner_did(host)`, from the same read.
  It is the installation owner, and `signing::install` refuses a
  certificate whose master is not the owner (`signing.rs:105`). So an
  origin's `issuer` equals it exactly when this person signed the origin,
  and `is_you` needs no other state. A delegated key is never the author.
- `now_secs`: `clock::now_secs()`.
- `history` (`messages.rs:395`), `changes` (`:420`) and `search` (`:513`)
  call it after their existing conversion. (The Hub calls only `history`
  and `search` today; `changes` is for other clients.) The two literal
  rows of `history_item_to_view` (`messages.rs:344`) get `forward: None`.
- `MessageView::tombstone` (`conversation.rs:193`) also clears `forward`.

### D-C7-7. `conversation.forward`

New file `crates/roym_conversation/src/app/forward.rs`. The input type is
in `method/conversation.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ForwardInput {
    pub message_id: String,     // the source message
    pub conversation: String,   // where the forward goes
}
// Output: SendOutput, the same as conversation.send.
```

Steps, in this order. Each refusal is `-32602` with the `data.code` named:

1. `AppConversation::get_message(host, message_id)`. `NotFound` →
   `message-not-found`. (It returns only accepted or own messages, so a
   held or dropped message is "not found".) Other host errors →
   `from_host` (`crates/roym_conversation/src/app.rs:131`).
2. Deleted (`deleted_at` set) → `message-deleted`.
3. `content_type != ROYM_MESSAGE_CONTENT_TYPE` → `not-forwardable` (a
   `text/plain` message, a card, a widget, a system message; ADR-0030,
   decision 3).
4. The body does not parse → `not-forwardable`.
5. `forward_mode != Signed` → `not-forwardable`.
6. `check_origin` is not `Checked` → `origin-not-checked`, with the reason
   in the message.
7. Build `ChatBody { chat_version: 1, content_type, text, Signed, origin:
   <the same string, byte for byte>, forward_count: Some(new_count) }`.
   `new_count = chat::next_forward_count(old, source_is_own_forward)`
   (pure, unit-tested): `old` is the source's count (0 if none). If the
   source is an outgoing message of this person and `old >= 1`, it is
   their own forward, and `new_count = old` (the forwarder is already
   counted; without this rule a person raises the count by forwarding
   their own forward again and again). Otherwise `new_count =
   old.saturating_add(1)`, clamped to `MAX_FORWARD_COUNT`. The count can
   still go up in two ways, both accepted: a changed client sends any
   count up to 1,000 (the change doc), and two people who forward to each
   other add one per hop.
8. `send_body(host, input.conversation, ROYM_MESSAGE_CONTENT_TYPE, source.languages, body, |_, _| None)`.
   The group checks of the target run there. (The reserved-type closure
   lets the chat type through, as in D-C7-5.)

- The forwarder's own default is never read (FW-15).
- The source may be in any chat, and the target may be the same chat
  (FW-9: "inside the same group").
- A restored source message may be forwarded: the check needs only the
  body and the languages.
- Nothing is signed with the forwarder's person key. The forwarder's own
  message signature covers the forward, as for any message (ADR-0030,
  decision 8). So a forward works for a person who has no signing
  certificate on `conversation`.
- A forward is a copy that lives on its own. Deleting the source message,
  here or on the author's node, does not delete forwards that were already
  sent (the delete dialog says so, section 4, item 17).
- There is no limit on how often or to how many chats a person forwards
  (change doc, non-goals).

### D-C7-8. Search (Q4 (a): the host indexes the text)

In `crates/conversation`, a function `indexable_text(content_type: &str,
body: &[u8]) -> Option<Cow<str>>` replaces the
`is_searchable_content_type(..) && str::from_utf8(..)` test at its six
call sites (`store/message.rs:134`, `:187`, `:326`; `group.rs:424`;
`store/backup.rs:439`; `store/text_index.rs:80`). Rule:

- a content type that is not text or JSON, or a body that is not UTF-8:
  `None`, as today;
- a `+json` type (or `application/json`) whose body is a JSON object with
  a top-level string field `text`: that string only;
- any other text or JSON body: the whole body, as today.

No Roym body that exists today has a top-level `text` (cards, widgets,
widget answers and the group system types were checked), so only chat
bodies change. No node holds a chat body before C7a, so no reindex is
needed. Do not bump `SEARCH_KEY_VERSION`: it versions the key algorithm
in `text_lang`, not which text is indexed. The rule is general, so the
doc comment of `search` in `conversation.wit` (`:277`) gains one sentence:
for a JSON body whose top level is an object with a string field `text`,
only that field is searched.

`conversation.search` is unchanged: a hit is a message whose text matches,
and `MessageView.body` is the inner text (D-C7-6). Tests, in the host
crate: a chat body is found by a word of its text, not by `signed`,
`forward_mode` or a fragment of the origin; a widget body is found as
before; a restored chat body is indexed by the same rule (the rebuild at
`text_index.rs:80` uses `indexable_text` too).

### D-C7-9. Backup

`crates/roym_conversation/src/app/backup.rs` (`:19`, `:20`, and the
`collect_sections` call in `export`): add `SECTION_SETTINGS = "settings"`
and `(SECTION_SETTINGS, SETTINGS)` to `collect_sections`. Import: the
section may hold zero rows, or one row with id `forward`. Any other id, or
a payload that does not parse as `ForwardSettings`, refuses the import
(`-32602`, the row id in the message). `import` is about 85 lines and
already has an arm for each section; the new arm goes into a helper
(`import_settings`), as the admissions arm does, so the function stays
under 100 lines. Chat bodies travel in `conversation_history` unchanged,
and are checked again at read after a restore.

`SCHEMA_VERSION` 4 → 5 (`crates/roym_conversation/src/app.rs:32`). Its doc
comment gains a "why" line: the settings section.

### D-C7-10. Hub

New files in `crates/roym_web/ui/src/chat/`:

- `forward_mode.ts`: `loadForwardSettings()`, `saveForwardSettings(..)`
  and `modeControl(defaultMode, inGroup)`. The control is a `<select>`
  ("Signed", "Not forwardable") set to the default, with a group notice
  element shown when `inGroup` is true and the value is `signed` (FW-11).
  `reset()` puts it back to the default after each send (FW-12).
- `first_message.ts`: `askOnce(anchor): Promise<ForwardMode>`. If
  `settings.asked` is true, it returns the default at once. Otherwise it
  draws a dialog (class `forward-mode-dialog`) with the two choices and
  "Skip", and the text of ADR-0030, decision 12, in plain words, with the
  sentence that a signed message proves who wrote it and when, not to whom
  it was sent. It saves `{ default_mode, asked: true }` (skip saves
  `asked: true` only) and returns the mode for this send. Closing the
  dialog counts as Skip. No choice is drawn as selected, so a quick click
  on "Send" cannot pick `signed`.
- `forward_view.ts`: `forwardLine(view, contacts)`, `forwardActions(m, refresh)`
  and the notes. The line is "Forwarded. First written by {name} on
  {date}. Forwarded {count} time(s)." or "… Forwarded many times." when
  `many_forwards` (FW-5). `{name}` by Q9 (a): "you", the contact name, or
  "…" and the last 12 characters of the DID with "(not in your contacts)"
  and the whole DID in a tooltip. `{date}` from `written_at_secs`
  (the author's own clock, so the line says "written", not "sent"),
  through the Hub's date format (`i18n/format.ts`). The tags of section 4,
  item 16 are drawn here too: only on a `text/plain` row and on a chat body
  of mode `not-forwardable`; never on a card, a widget, a widget answer or
  a system row.
- `forward_picker.ts`: a list of the person's chats from
  `conversation.list` (direct and groups, not hidden). "Forward" calls
  `conversation.forward`. The picker has the class `forward-picker`.

Changes to existing files:

- `screens/messages.ts`: add `.forward-mode-dialog, .forward-picker` to
  `OPEN_WORK` (`:38`), so a feed redraw waits for them. In `renderThread`
  (`:298`) and its send handler (`:334`): `askOnce` before the send, then
  `forward_mode` from the mode control. `MessageRow` (`:51`) gains
  `forward?: ForwardView`. `messageElement` (`:715`) calls `forwardLine`
  and `forwardActions` for a row with `forward`, and the tag for a
  `text/plain` row. The poll composer beside the text composer
  (`pollComposer`) sends no mode and needs no dialog.
- `screens/groups.ts`: the group composer (`doSend`, `:500`) does the same
  with `inGroup = true`. Its message rows (`renderThreadMessages`, `:181`;
  `renderMessageRow`, `:113`) use the same helpers.
- A refused `signed` send with `data.code` `signing-not-enrolled`,
  `signing-certificate-expired` or `signing-certificate-stale` shows two
  buttons: "Send as not forwardable" (sends again with that mode) and
  "Renew signing" (opens the setup screen, `screens/setup.ts`). It never
  changes the mode on its own (ADR-0030, decision 7).
- `screens/profile.ts`: the section "Forwarding of my messages", with the
  default and the explanation (FW-10, Q10 (a)).
- The delete dialog (`openDeleteDialog`, `messages.ts:1074`) adds the
  sentence of section 4, item 17, for a `signed` message.

### D-C7-11. One signing refusal helper

Move the body of `resolve_principal_and_owner` from
`crates/roym_transaction/src/app.rs:327` to `crates/roym_core/src/signing.rs`
as:

```rust
pub async fn resolve_principal_and_owner<H: AppHost>(host: &H, now: u64)
    -> Result<(Principal, String), Response>
```

The name and the signature stay, so `transaction` keeps its 12 call sites
(`git grep -n resolve_principal_and_owner crates/roym_transaction`) and
only re-exports the function in `app.rs`: `pub(crate) use
syneroym_roym_core::signing::resolve_principal_and_owner;`. Its words and
`data.code` values do not change. Remove the imports that the move leaves
unused in `app.rs` (`CertificateError`, `Principal`, `json!`, if no other
code there uses them). `roym_conversation` calls it in D-C7-5. The first
plan renamed it and switched every call site; that touches 10 files and
makes the PR conflict with any other `transaction` change.

### D-C7-12. No `roymctl` change

`roymctl roym group send` (`apps/roymctl/src/commands/roym/group.rs:197`)
keeps sending with no mode (Q3 (a)). No story or test needs a
`--forward-mode` flag or a `forward` command. The developer guide
documents the raw methods instead.

### D-C7-13. Widgets stay not forwardable

A `widget` or `widget-answer` message (C4) has its own content type and
body (`roym_core/src/widget.rs:15`) and no forward mode, so
`conversation.forward` refuses it at step 3 of D-C7-7. This was checked
against the merged widget code: the widget body has no field that a mode
or an origin could use, and a forward of a poll would make a second poll
with its own votes. The C4 decision on votes is final: an answer is an
ordinary chat message, authenticated by the chat's message signature, with
no master-DID signature (backlog, "Recently resolved"). So a vote is not
provable outside the chat. The Hub shows no forward action and no tag on a
widget.

### D-C7-14. A record forward is checked by the owning service (Q12 (a))

`conversation` does not verify a record-forward envelope. When the Hub
draws such a row, it asks the service that owns the record type:
`catalog.listing.verify`, `transaction.quote.verify`, or the new
`profile.verify-envelope`, and draws only from that verdict. Two reasons:
the three checks would add 229,778 bytes to `conversation` against a
158,505-byte limit (section 6.2), and the check stays in the service that
owns the record, where it is today.

The node stores and returns the body. The sender's Hub calls the same
verify method on its own node before it sends. A changed client can send a
broken envelope; the receiver's Hub shows "could not be checked" and
draws no record, never a name or a value.

## 6. Step-by-step changes

### 6.1 C7a — Forward of a text message

Steps 0 to 4 are the node. Steps 5 and 6 are the Hub and the demo. Step 7
is the close-out. Each step builds, and its tests pass, before the next one
starts. **Checkpoint:** after step 4, push the node commits and tell the
reviewer. The Hub steps follow on the same branch. If a stop rule fires in
steps 1 to 4 (size, origin size, history time), stop and ask the owner
before step 5.

#### Step 0 — Start from `main`

Cut the C7a branch from `main` (this branch holds only the plan). Measure
the component sizes on that commit (one `target/` folder,
`mise run build:roym`) as the "before" column. The reviewer's numbers on
`89d1dd23` are in "Sizes now"; use your own if `main` has moved. Re-check
any line number of this plan that a later merge moved.

#### Step 1 — Pure chat code in `roym_core`

- `crates/roym_core/Cargo.toml`: `sha2.workspace = true`.
- `src/chat.rs` and `src/chat/tests.rs`: D-C7-1, D-C7-2, D-C7-3, the
  pure part of D-C7-6 and `next_forward_count` (D-C7-7). `src/lib.rs`:
  `pub mod chat;`.
- `src/record.rs`: the record type.
- `src/conversation.rs`: `MessageView.forward` (and `tombstone` clears
  it), and the struct literals at `conversation.rs:248`,
  `conversation/group/tests.rs:7`,
  `crates/roym_conversation/src/app/messages.rs:349` and `:372`.
- `src/method/conversation.rs`: `SendInput.forward_mode`, `ForwardInput`,
  `ForwardSettings`, `SettingsInput`. Update every `SendInput` literal
  (`git grep -n "SendInput {"`): `method/tests.rs:504` and `:513`, and the
  production literal in `crates/roym_transaction/src/app.rs:645`, which
  sends cards.
- `src/signing.rs`: D-C7-11, and the re-export in `roym_transaction`.

Tests: section 7.1. `cargo nextest run -p syneroym-roym-core` and
`-p syneroym-roym-transaction`.

#### Step 2 — `conversation`: settings, send with a mode, the view

- `src/app.rs`: `SETTINGS`, `SCHEMA_VERSION` 5.
- `src/app/settings.rs`: D-C7-4.
- `src/app/chat_send.rs`: D-C7-5, with `check_target` and `deliver` split
  out of `send_body` in `messages.rs`.
- `src/app/view.rs`: the service part of D-C7-6.
- `src/app/messages.rs`: the reserved type; `send` calls `send_with_mode`;
  `history`, `changes` and `search` call `view::add_forward_views`.
- `src/app/backup.rs`: D-C7-9.
- `src/api.rs`: the row for `conversation.settings`.
- The host rule of D-C7-8, in `crates/conversation`, with its tests and
  the WIT doc comment.
- `crates/roym_web/tests/method_contract/contract.rs:39`: the count (27 → 28
  after this step; 29 after step 3).
- Scenario 73 (`dual_build_parity/wire_origin.rs:67`): add the verb.
- Method shapes: run with `UPDATE_METHOD_SHAPES=1` and read the diff.
- **Measure `conversation` again** (Q11). Over +15% (147,820 bytes over the
  "before" size): stop and ask.

Tests: parity A to E, H, I and J (section 7.3). Run `mise run build:roym`
first, then the **whole** parity binary.

#### Step 3 — `conversation.forward`

- `src/app/forward.rs`: D-C7-7. `src/api.rs`: its row. Contract count + 1;
  scenario 73 + 1; method shapes.
- Confirm section 2.2, point 5 (the pinned clock) with the first forward
  scenario.
- **Measure `conversation` again** (Q11), with the same rule as step 2.

Tests: parity F, G and K.

#### Step 4 — Three-node test and the measurements

- New `crates/substrate/tests/roym_forward_e2e.rs` on
  `common::roym_group::boot_trio` (`crates/substrate/tests/common/roym_group.rs:39`),
  `form_group` (`:72`) and `converge` (`:193`). Section 7.4.
- It prints, and asserts under 2,048 bytes, the length of the origin
  envelope of a short text message ("Market opens at 9", one tag `en`).
  It also prints the length of the whole chat body. **Over 2 KiB: stop and
  ask the owner** (ADR-0030, decision 11). Both numbers go into the change
  doc's Deviations.
- It measures one `conversation.history` page of 200 `signed` messages on
  the WASM build (Q6 stop rule: 500 ms) and prints the number.

#### Step 5 — Hub

D-C7-10, with the Hub unit tests of section 7.5. Run
`TZ=UTC npx vitest run`. New `en.ftl` and `hi.ftl` keys (section 7.8).

#### Step 6 — Playwright and the demo

Every send through the Hub composer now meets the first-message dialog.
Two places send through the real composer: the 7 `sendMessage(` calls
(story 20, story 40, `roym-hub.spec.ts`) and the Send click of the group
case at `tests/roym-groups.spec.ts:102`. (The compose box of
`roym-deal.spec.ts` is a plain input that the test builds; it never
sends.) Run the whole Playwright suite, not only the new case.

- `crates/substrate/tests/e2e/hub-helpers.ts`: `sendMessage` (`:211`)
  answers the first-message dialog with "Skip" when it is shown. Then every
  existing caller (`demo/20-roym-chat.spec.ts`,
  `demo/40-roym-languages.spec.ts`, `tests/roym-hub.spec.ts`) keeps
  working. A new helper `chooseForwardMode(page, mode)` answers it on
  purpose. `tests/roym-groups.spec.ts:102` answers it the same way.
- `tests/roym-hub.spec.ts`: a new case, number **43** (42 is the highest
  number on `main`, in `roym-hub-remote.spec.ts`), for the dialog, the
  composer control and the Profile setting, on one node.
- Story 20 (Q7): section 7.6.
- `mise run demo`, then `mise run test:e2e`, with the output redirected
  (AGENTS.md, Context Budget).

#### Step 7 — Sizes, docs and close-out of C7a

Component sizes (section 7.7) into [task.md](task.md) section 1; the
change doc's Deviations (section 9); backlog (section 8); living docs
(section 9); the `status.md` row with the Hindi keys for the owner
(section 7.8); the change doc's `demo:` list; `mise run verify` with no
`--skip` (in a worktree: `cargo xtask verify` with `CARGO_TARGET_DIR`).

### 6.2 C7b — Forward of signed records

*Checked on `main` at `79344661`, 2026-11-28, after C7a merged. No code
is written. This section replaces the outline and holds the three checks.
It had one stop, [Q12](#q12-where-does-a-forward-of-a-signed-record-get-checked-answered-a-2026-11-28):
the outline breaks the Q11 size rule, and the owner answered (a) on
2026-11-28 (the check stays in the owning service, D-C7-14).*

#### What the three checks found

**1. A quote with no disclosures passes its check, and its visible fields
are the ADR's list.** `verify_quote`
(`crates/roym_core/src/transaction.rs:458`) checks the envelope, parses
`QuotePayload`, and calls `validate()` (`:306`), which calls
`SignedTerms::validate_visible` (`crates/roym_core/src/deal/sd.rs:241`).
None of them reads a disclosure. `validate_visible` checks the commitment
count, the currency, the two `_sd` lists (5 terms digests, 10 per
commitment, all distinct) and the `after` graph. The fields that stay in
the clear are `TERMS_SET.visible` and `COMMITMENT_SET.visible`
(`crates/roym_core/src/disclosure.rs:59` to `:78`): the currency, the
expiry, the ids, the two parties, and each commitment's id, giver,
resource kind, kind, extension and `after` list. That is what ADR-0030,
decision 10, says the receiver sees. `verify_quote` uses
`allowing_expired`, so a lapsed quote still verifies with
`expired: true`. `listing::verify_envelope`
(`crates/roym_core/src/listing.rs:622`) does not allow an expired record.
`ProfilePayload::validate` is in `roym_core`
(`crates/roym_core/src/person.rs:63`), but the envelope and subject checks
are in `roym_profile` (`crates/roym_profile/src/app/contacts.rs:105` to
`:130`), so C7b moves them into `roym_core` first. A profile's `subject`
is its owner's DID (`profile_ops.rs:170`), so the generic check is
`subject == issuer`.

**The Hub quote card does not draw that view.** After C6b, `renderQuote`
(`crates/roym_web/ui/src/cards/templates/quote.ts:102`) draws a card from
the plain view that `disclosures::plain_view` makes, not from the
envelope. With zero disclosures it still draws a line for each commitment
("You give Payment" for a currency commitment, `:217` to `:235`) and the
totals block, where `dealTotals` sums absent values as 0 (`:263`). It
also calls the reader "you" for the counterparty's commitments when
`role` is absent (`:224`). A third party would see prices of zero and
"You give". That contradicts ADR-0030, decision 10. So a forwarded quote
gets its own visible-fields block, and `renderQuote` stays as it is.

**2. The third dispatch arm is cheap, but the three record checks are
not.** One `target/` folder, `cargo component build --release --target
wasm32-wasip2 -p syneroym-roym-conversation`. The baseline is 1,056,700
bytes (C7a's table says 1,056,675; a fresh build differs by 25 bytes).
Each probe added one arm and was then reverted:

| Probe (one arm added) | Size | Over baseline |
| --- | --- | --- |
| an arm that shares the existing `forward` future | 1,057,917 | +1,217 |
| an arm with a new wrapper handler that calls `forward` | 1,057,830 | +1,130 |
| an arm with `transaction::verify_quote` | 1,168,481 | +111,781 |
| an arm with `listing::verify_envelope` | 1,175,018 | +118,318 |
| an arm with the profile check and `ProfilePayload::validate` | 1,070,628 | +13,928 |
| an arm with all three checks | 1,286,478 | **+229,778** |

The arm is not the cost: the linker folds shared code. The cost is the
payload code that `conversation` does not link today. A quote check and a
listing check pull their payload, deal, extension and disclosure code in
(~112 KB and ~118 KB), and they share only about 11 KB. The Q11 rule
gives `conversation` 158,505 bytes after C7a (15% of 1,056,675, ceiling
1,215,180). The outline's "check the envelope with the matching pure
function" inside `conversation` is about 71 KB over that, before the
view, the method body or any Hub code. C7b stopped here for
[Q12](#q12-where-does-a-forward-of-a-signed-record-get-checked-answered-a-2026-11-28);
the owner answered (a) on 2026-11-28.

**3. The new content type is added to the reserved list.** `send`
(`crates/roym_conversation/src/app/messages.rs:264`) refuses six content
types in its closure at `:283` to `:300`: membership event, deletion
request, group profile, widget, widget answer, the chat type, plus the
card type in a group. The record-forward type goes into that list.
`conversation.forward-record` passes the same closure as `forward`
(`|_, _| None`), so a forward works in a group. `transaction.sync` files
only the card type (`crates/roym_transaction/src/app/sync.rs:264`), so
the new type is never filed as a deal, in a group or not.

#### The design (Q12 (a), D-C7-14)

The check stays in the service that owns the record, which is where it is
today:

- `catalog.listing.verify` exists
  (`crates/roym_catalog/src/app/listing_ops.rs:608`);
- `transaction.quote.verify` exists
  (`crates/roym_transaction/src/app/quote_ops.rs:601`); with no
  disclosures it returns the verdict and the visible terms;
- `profile.verify-envelope` is new and reuses the check moved into
  `roym_core`.

`conversation` stores and returns the body only:

- Content type `application/vnd.roym.record-forward+json`, body
  `{ "record_forward_version": 1, "record_type": "listing" | "profile" | "quote", "envelope": "<envelope JSON>" }`.
  It ignores the forward mode and carries no origin (ADR-0030,
  decision 10).
- `conversation.forward-record` `{ conversation, record_type, envelope }`:
  refuses another record type, checks the envelope against
  `MAX_CARD_BODY_BYTES` (160 KiB,
  `crates/roym_core/src/card.rs:75`) before it parses, refuses a body that
  does not parse, and sends with `send_body`. It does not verify the
  envelope and signs nothing.
- `history` and `changes` return the body unchanged. The host does not
  index it: `indexable_text`
  (`crates/conversation/src/store/message.rs:36`) gets one case for the new
  type and returns `None`. No reindex is needed, because no such body
  exists on a node before C7b.
- The Hub parses the body in a new `chat/record_forward.ts`, asks the
  matching local service for the verdict, and draws only from it. It keeps
  each verdict for the life of the screen, so a feed event does not verify
  the same record again.
- A listing draws with the directory hit card (moved into a shared
  module), a profile with a new small profile card, and a quote with a new
  visible-fields block: the ids, the two parties, the currency, the
  expiry, and each commitment's visible fields. No totals, no
  description, no value, no "you give". The picker warns before a quote
  forward that it shows both parties of the deal.
- No `MessageView.record_forward` field. The node keeps no verdict for
  these messages; the owning service gives it at draw time, as it does
  for a card today.

#### Steps

The numbering starts after C7a's step 7.

- Step 8 — `roym_core`. Move the profile envelope and subject check out of
  `roym_profile` into `person` (the storage stays in `roym_profile`). Add
  the record-forward body type with its parse, its three record names and
  its size rule. Unit tests.
- Step 9 — `profile.verify-envelope` (local only) and its tests. `catalog`
  and `transaction` keep their verify methods unchanged.
- Step 10 — `conversation`. The new arm, `forward-record`, the reserved
  type and the host index rule. Parity A to D below. **Measure
  `conversation` again**; the limit is 158,505 bytes.
- Step 11 — the Hub. `chat/record_forward.ts`, the three forward actions
  (quote card, directory hit, contact profile), the contact profile view,
  the quote warning, and the Hub unit tests.
- Step 12 — Playwright case **44**, the demo step, the sizes, the change
  doc, the living docs, the backlog and `mise run verify`.

#### Tests

- `roym_core` unit tests: the profile check after the move, the body parse
  and its refusals, and a quote with no disclosures that passes
  `verify_quote` with the ADR's visible fields.
- Parity: the highest number used is 288 (C7a), so C7b takes **289** and
  up. The new file is `dual_build_parity/forward_record.rs`.
  - A. `forward-record` of a quote, a listing and a profile: the target
    history row has the new content type and the same envelope, and the
    raw body parses. The source conversation is not changed.
  - B. `forward-record` refusals: another record type, an envelope over
    160 KiB, and a body that is not JSON. The card type in a group keeps
    its refusal (scenario 207).
  - C. `send` with the new content type is refused as reserved.
  - D. `search`: a record-forward body is not indexed.
- Hub unit tests: the parse, the verify call for each record type, a
  refused verdict that draws no record and names the reason, a quote block
  with no value and no "you give", and the warning before a quote
  forward.
- Playwright case **44** in `tests/roym-hub.spec.ts`: forward a quote card
  into the chat; the receiver sees the visible-fields block and no price.
- Demo: extend story 50, which already has a quote between Alice and Bob,
  with "Alice forwards the quote to the chat; Bob sees the two parties,
  the expiry and no amounts".

## 7. Tests

### 7.1 Unit tests (`crates/roym_core/src/chat/tests.rs`)

Test names say what they prove, with no plan ids.

Body:

- `a_chat_body_round_trips_through_bytes`
- `a_body_of_another_version_is_unsupported`
- `a_body_with_another_inner_content_type_is_unsupported`
- `an_unknown_forward_mode_reads_as_not_forwardable`
- `an_origin_on_a_not_forwardable_body_is_ignored`
- `an_origin_over_the_size_limit_fails_as_too_large`
- `a_forward_count_of_zero_or_a_negative_count_reads_as_none`
- `a_huge_forward_count_is_clamped_and_the_body_stays_readable` (2,000,
  and a number above `u64::MAX`)
- `a_strict_forward_mode_refuses_an_unknown_string` (the type used by
  `SendInput` and `SettingsInput`)
- `unknown_fields_in_a_chat_body_are_ignored`

Hash:

- `the_content_hash_is_stable` (a fixed vector)
- `the_hash_changes_with_the_text`, `the_hash_changes_with_one_language_tag`,
  `the_hash_changes_with_the_order_of_tags`,
  `the_hash_changes_with_the_content_type`,
  `the_hash_changes_with_the_blob_list` (FW-6, FW-8)
- `length_prefixes_keep_two_splits_apart` (`["ab","c"]` and `["a","bc"]`)

Origin. Signed with `syneroym_identity` keys and a real
`DelegationCertificate`, as the tests in `crates/signed_record/src/verify.rs`
do. Test clocks start at or after the real "now", because the certificate
lifetime is checked against it:

- `a_signed_origin_checks_and_names_the_master_did`
- `an_origin_still_checks_after_its_certificate_expired`
- `an_origin_signed_by_the_service_key_alone_is_refused`
- `an_origin_with_a_subject_is_refused`
- `an_origin_with_an_expiry_is_refused`
- `an_origin_of_another_record_type_is_refused`
- `an_origin_for_other_text_fails_the_hash` (FW-6)
- `an_origin_for_other_languages_fails`
- `a_tampered_origin_signature_fails` (FW-4)
- `an_origin_dated_far_in_the_future_fails_and_checks_once_the_clock_catches_up`
  (section 2.2, point 5)
- `an_origin_signed_for_one_master_but_naming_another_issuer_fails` (the
  certificate names a different master than the envelope's `issuer`)
- `an_origin_with_a_certificate_of_another_scope_is_refused`
- `an_origin_that_is_another_record_of_the_same_person_is_refused` (a
  `profile` envelope put in the origin field)
- `a_changed_text_with_the_old_origin_is_not_forwardable` (FW-13: the body
  a changed client would send)

View (`forward_view`): one test per row of the table in D-C7-6, and:

- `a_forward_of_a_forward_counts_up`
- `many_forwards_starts_at_five` (FW-5)
- `a_failed_origin_never_names_an_author` (FW-4)
- `an_unreadable_body_is_not_forwardable`
- `the_author_reading_a_forward_of_their_own_message_sees_is_you`
- `a_forward_view_row_for_an_unknown_sender_keeps_the_origin_but_not_the_forwarded_line`
  (the known gap of D-C7-6)

Forward count (a pure `next_forward_count(old, source_is_own_forward)` in
`chat`, used by `conversation.forward`):

- `forwarding_a_received_message_adds_one`
- `forwarding_your_own_forward_keeps_the_count`
- `the_count_never_passes_the_clamp`

Search. The tests are in the host crate
(`crates/conversation/src/store/tests.rs` and the host search tests):
`a_chat_body_is_found_by_a_word_of_its_text`,
`a_chat_body_is_not_found_by_signed_or_forward_mode_or_an_origin_fragment`,
`a_widget_body_is_still_found_by_its_whole_text`,
`a_restored_chat_body_is_indexed_by_the_same_rule`.

Signing helper (in the `signing::tests` module, `signing.rs:257`):

- `resolve_principal_and_owner_refuses_each_certificate_state_with_its_code`:
  `TestHost` with `set_signing_id` and no `CERTIFICATES` row, then an
  expired certificate, then a stale one, then no owner.

### 7.2 Service tests

`roym_conversation` has no unit tests (section 2.2, point 4). Its rules
are tested through 7.3 and 7.4.

### 7.3 Parity (both builds agree): new file `dual_build_parity/forward.rs`

Scenario numbers: 278 to 288 (the highest on `main` is 277, C6). Re-check
before you start; C4 used 260 to 266 and C6 267 to 277. Each scenario
calls `enrol_signing(&h, "conversation")`
(`dual_build_parity/fixtures.rs:59`) first, unless it tests the refusal.

- A. `conversation.settings` with no field on a fresh node returns the
  default; with fields it changes them and returns the row; `asked: false`
  is refused.
- B. `send` with `signed`: the history row has `forward.mode` `signed`,
  `forwardable: true`, `origin.author_did` = the owner DID, and `body` =
  the text. The raw stored body (read with the host's `get_message`) holds
  `"forward_mode":"signed"` (FW-1, FW-14).
- C. `send` with `not-forwardable`: no origin, `forwardable: false`.
- D. `send` with `signed` and no certificate: `data.code`
  `signing-not-enrolled`, and the outbox is empty.
- E. `send` with `forward_mode` and `content_type: "text/html"`: `-32602`,
  field `content_type`. A raw `send` of the chat content type: reserved.
  `send` with `forward_mode: "signd"`: `-32602` (the strict input type).
- F. `forward` of B's message to a second conversation, with the default
  set to `not-forwardable`: count 1, the same origin string, `forwarded:
  true`, mode `signed` (FW-2, FW-15).
- G. `forward` refusals: a `text/plain` message, C's message, a widget
  message (D-C7-13), a deleted message, an unknown id. Each has its
  `data.code`.
- H. Search: a word of B's text finds it; `forward_mode` and `signed` do
  not, and a word that matches many real messages returns the
  newest `limit` of the real ones, not fewer (the lost-hit case of D1).
- I. Backup: export after A, import into a wiped node: the settings row is
  back. A bundle with a second settings row id is refused.
- J. `send` with `signed` into a group that the person was removed from, and
  into a restored group: refused with the group notice, and the signing
  call was never made (the order of `check_target` and the signing).
- K. `forward` of a message that the same node sent as a forward: the count
  stays; `forward` into the same conversation works (FW-9).

The two builds share the owner identity and the pinned clock, so they
should sign the same bytes. Normalize the origin with `strip_volatile` only
if they do not, and say why in the normalizer.

### 7.4 Three-node test (`crates/substrate/tests/roym_forward_e2e.rs`)

On `boot_trio` (Alice, Bob, Carol), WASM build. `boot_trio` does not
enrol signing, so the test first calls `RoymNode::enrol_signing`
(`crates/substrate/tests/common/roym.rs:565`) on each node.

1. Alice and Bob open a one-to-one chat. Alice sends "Market opens at 9"
   with `signed`. Wait for `delivered`.
2. Bob's history: `forwardable: true`, author Alice's master DID.
3. Bob forms a group with Carol (`form_group`) and forwards Alice's
   message there. On Carol's node, after `converge`: `forwarded: true`,
   `forward_count: 1`, `origin.author_did` = Alice's master DID, `body` =
   the text (FW-3, FW-9 to a group).
4. Carol forwards it into the same group: count 2 (FW-9 inside the same
   group, FW-15).
5. Alice sends a `not-forwardable` message. Bob's `forward` of it is
   refused with `not-forwardable` (FW-2).
6. Bob forwards his own forward into the chat with Alice: the count stays 1,
   and Alice's view says `is_you` for the origin.
7. The size measurement and the history timing of step 4. Also one `signed`
   message of 11 KiB of text: print its body size and whether it is under
   `MAILBOX_CARD_BODY_BYTES` (section 2.2, point 10). `boot_trio` has no
   mailbox role; `mailbox_conversation_e2e.rs:341` already pins that a
   body of that size goes through the mailbox.

A forged forward (changed text with the old origin) cannot be sent through
`conversation.send`, because the chat content type is reserved. The view is
a pure function of the body and the languages, so the unit tests of 7.1
cover it on the receiving side.

### 7.5 Hub unit tests (`TZ=UTC npx vitest run`)

New files `chat/forward_view.test.ts`, `chat/forward_mode.test.ts`,
`chat/first_message.test.ts` and `chat/forward_picker.test.ts`:

- the forwarded line with a contact name, with "you" (`is_you`), with an
  unknown DID (the last 12 characters, "(not in your contacts)", and the
  whole DID in the tooltip; never the first 16), and with count 1, 4 and 5
  (FW-3, FW-5, Q9);
- a failed origin: no author name anywhere in the element, and the note
  (FW-4);
- no "Forward" button when `forwardable` is false, and the tag says why
  (FW-13); a `text/plain` row gets the "no signed origin" tag; a card, a
  widget, a widget answer and a system row get no tag; an unreadable body
  shows the "no body" text and never raw JSON;
- the mode control starts at the default, changes for one send, and is
  back at the default after it; the saved default is unchanged (FW-12);
- the group notice shows only for `signed` in a group (FW-11);
- the first-message dialog: shown when `asked` is false; each choice saves
  `asked: true`; skip and closing keep `not-forwardable`; nothing is
  pre-selected; not shown again (FW-10);
- a refused `signed` send shows the two buttons and does not resend on its
  own (ADR-0030, decision 7).

Existing tests: `screens/messages.test.ts`, `screens/groups.test.ts` and
`screens/messages_feed.test.ts` mock `conversation.send`. Update their
expected params, and add one assertion each that `forward_mode` is sent.

### 7.6 Playwright and the demo

- A Hub case, number 43, in `tests/roym-hub.spec.ts`: dialog, composer,
  Profile default.
- Story 20, after C4's poll steps (Q7 (c)):
  1. "Alice's screen: she chooses that her messages can be forwarded with
     her name": at her first send, `chooseForwardMode(page, 'signed')`.
     This is part of today's step "send Bob a message".
  2. "Bob's screen: he forwards Alice's message back to the chat".
  3. "Alice's screen: it shows 'Forwarded. First written by you on ….
     Forwarded 1 time.'". Wait `DELIVERY_WAIT_MS`. Match the line by its
     parts, not the whole string, because the date changes.
- `crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts` goes into the
  change doc's `demo:` list.

### 7.7 Component size

Release `wasm32-wasip2`, one `target/` folder, before (step 0) and after.
`conversation` gets new record code, so its limit is +15% of its "before"
size. On `main` `89d1dd23` that is 985,469 bytes, so **147,820 bytes**
(ceiling 1,133,289; Q11 (a)). `transaction`
changes only by the moved helper: +40 KB limit (2,870,685 bytes before).
The other components link `roym_core` but call none of the new code: +40 KB
limit each (`catalog` 960,590; `directory` 1,504,433; `profile` 772,748;
`scheduling` 715,723; `web` 311,314). Over a limit: stop and tell the
owner. Measure `conversation` at steps 2 and 3 as well as at the end.

C7b measures the same way. Its "before" for `conversation` is the C7a
result, 1,056,675 bytes, so the +15% limit is **158,505 bytes** (ceiling
1,215,180; Q11 and Q12). `profile` gains one method and no record code, so
its limit is +40 KB. `catalog` and `transaction` gain nothing. The size
probe of section 6.2 shows that a record check inside `conversation`
breaks this limit; step 10 measures again.

### 7.8 Hindi text

Every new key goes into `en.ftl` and `hi.ftl`, and the `status.md` row
lists them for the owner's review. Expected keys (names may change):
`forward-mode-label`, `forward-mode-signed`, `forward-mode-not-forwardable`,
`forward-mode-group-notice`, `forward-first-title`,
`forward-first-signed-explain`, `forward-first-not-forwardable-explain`,
`forward-first-to-whom-note`, `forward-first-skip`,
`forward-settings-heading`, `forward-settings-explain`,
`forward-button`, `forward-picker-title`, `forward-sent`, `forward-line`,
`forward-line-many`, `forward-author-you`, `forward-not-in-contacts`,
`forward-tag-not-forwardable`, `forward-tag-no-origin`,
`forward-origin-not-checked`, `forward-signed-tag`,
`forward-send-not-forwardable`, `forward-renew-signing`,
`messages-delete-signed-note`. The tests `i18n/hindi-screens.test.ts` and
`i18n/catalog.test.ts` check that every key has both languages.

### 7.9 Not tested

- A changed client that lowers the forward count: accepted by design (the
  change doc, "What the forward count can and cannot say").
- Attachments: none exist (Q8).

### 7.10 Size and time risks

- The size of `conversation` (Q11, steps 2 and 3): the chat code and two
  new handlers. The record check is already linked.
- The origin size (ADR-0030 stop rule, step 4).
- The read-time check (Q6 stop rule, step 4). The Hub repeats it at each
  redraw.
- `messages.ts` keeps its length: new code is in `chat/`.

### 7.11 How C7 avoids the kinds of findings in earlier reviews

- Every line number above was read on `89d1dd23`. Step 0 reads the ones
  that a later merge moved again (C4 P1, C3c P8).
- Callers of changed types: the `MessageView` literals (4), `SendInput`
  (`method/tests.rs`, parity, and the production literal in
  `roym_transaction/src/app.rs:645`; `roymctl` builds JSON, not the type)
  and `resolve_principal_and_owner` (12 call sites, kept by the
  re-export). `git grep` each before step 1 ends (C3c P3, C2 P2).
- Hard-coded counts: the `conversation` contract count, scenario 73, the
  method shapes (C4 P5, C3b R1).
- Every free field has a limit checked before it is parsed: origin 8 KiB,
  languages 3, text by the host limit `max_body_bytes`, 262,144 by default
  (`crates/conversation/src/store.rs:92`), forward count clamped to 1,000
  (C3a P4, C2 P5).
- The signer is compared with what is expected: the origin must be signed
  as a person (delegation present, a form check), and the display name
  comes only from the reader's contacts or the checked `issuer` (C2 R14,
  R25).
- The signing refusals are planned and tested (C3b P5, C2 R5).
- A storage error is never read as "no row" (C2 R22).
- New Hindi keys are listed (C1 R15).

## 8. Deferred backlog

Rows to add to [deferred-backlog.md](../../deferred-backlog.md) in C7a
(section 5, Messaging, unless noted):

- **Roym chat has no attachments, so the origin's blob-hash list is always
  empty.** FW-8 is met in the format only (Q8). Target: TBD.
- **Five places map the signing errors in their own words:**
  `roym_catalog` (`listing_ops.rs:159`, no `data.code`), `roym_profile`
  (`profile_ops.rs:268`), `roym_directory` (`credential_ops.rs:46`),
  `roym_core::backup` (`backup.rs:313`) and `transaction`'s
  `as_listed_ops/answer.rs:728`. The words of the `transaction` helper now
  live in `roym_core::signing` (section 2.2, point 3). Section 10 (Product
  surfaces). Target: TBD.
- **A `signed` text over about 11.5 KiB does not fit a sealed mailbox
  message, so it waits for the receiver to be online** (section 2.2,
  point 10). Target: TBD.
- Update the row "More demo stories" (§13): C7 extends story 20, and the
  forward is shown in the same chat (Q7 (c)). Add: a two-node group is
  not shown in any browser story or test.

Rows that exist already and must **not** be added again: "The forward mode
has one default per person: no default per conversation, and no group owner
limit" and "The forward action offers only three Roym record types" (both
in section 5, from the C0 ADR).

Rows to **extend** (add `message-origin` to the sentence, same target): the
two key-revocation rows, "Issuer-key revocation is not checked on any
membership record" and "A listing hit is never checked against a
revocation list". A checked origin has `revocation_status` "unknown" too,
so a stolen conversation signing key signs valid origins as the person
with no end, until a revocation source is wired (section 2.2, point 6).

## 9. Living docs and the change doc at close-out

Change doc [roym-message-forwarding](../../changes/roym-message-forwarding/change.md),
`## Deviations` (this change doc owns these, and they are written only
there):

- the chat body content type and format (Q3), and that a message sent with
  no mode stays `text/plain`;
- the sentence "the substrate conversation capability does not change":
  its indexing rule gets one case (Q4 (a): a JSON body with a top-level
  string `text` is indexed by that field only), and the WIT doc comment of
  `search` says so; its interface does not change;
- the forward count: read as a number up to 1,000, and a person's forward
  of their own forward does not raise it (D-C7-7);
- one `conversation.settings` method for the default mode and the
  first-message flag (D-C7-4);
- the inner content and its hash layout (D-C7-2), the `sha256:` hex form;
- FW-8 met in the format only (Q8);
- where the setting lives (Q5), and the first-message flag on the node;
- the check at read time (Q6);
- the display name rule (Q9): "you", the contact name, or the last 12
  characters of the DID; and that the delete dialog says forwards stay;
- the tags for a message that cannot be forwarded (section 4, item 16);
- the measured origin and body sizes (step 4);
- the split into C7a and C7b (Q2), until C7b merges.

Also in the change doc: tick the C7a lines of `## Tasks`. `status: done`
and `living-docs-touched` wait for C8 (task.md section 3, C8).

Living docs (C7a):

- `docs/roym-integrated-experience-spec.md`: "Messaging" (`:693`) — the
  chat body and the forward; "What is encrypted, and who can see what"
  (`:579`) — what a `signed` message gives up (ADR-0030, decision 12);
  "Records and what each one proves" (`:471`) — `message-origin`.
- `docs/TERMINOLOGY.md`: forward mode, origin (`message-origin`), forward,
  forward count.
- `docs/developer-guide.md` §6 "Build a Roym client": `forward_mode` on
  `conversation.send`, `conversation.forward`, `conversation.settings`,
  and the `forward` field of a message (the `conversation.send` block is at
  line 2571; the widgets paragraph follows it). The tagged code blocks run
  in story 30, so a new tagged block must run there too.
- `docs/system-requirements-spec.md:291`: remove "attachments"; add the
  forward with a signed origin. (Line 695 also says "attachments", for a
  request; it is about the request form, not chat. Leave it.)
- `docs/planning/traceability-matrix.md`: the messaging row gains the
  forward and its tests.
- [task.md](task.md): the C7a result line and the size table.

## 10. Exit criteria

C7a is done when all of these hold:

- [ ] Every C7a criterion in section 3 names a test that exists on the
      branch, and the test passes.
- [ ] The `roym_core` `chat` unit tests, the whole parity binary (after
      `mise run build:roym`), the host search tests, `roym_forward_e2e`,
      the Hub unit tests, the new Hub case, the whole Playwright suite and
      story 20 pass.
- [ ] The origin of a short text message is under 2 KiB, measured and
      recorded, or the owner answered the stop.
- [ ] A history page of 200 `signed` messages takes under 500 ms in the
      WASM build, or the owner answered the stop. The number is recorded.
- [ ] Component sizes are in task.md section 1, each within its limit (for
      `conversation`, +15% of the "before" size, Q11 (a)).
- [ ] Deviations, backlog rows, living docs and the `status.md` row (with
      the Hindi keys) are done.
- [ ] `mise run verify` with no `--skip` passes.

C7b is done when FW-7 has its tests, the Q12 (a) design is built (the
check in the owning service, D-C7-14), its sizes are in task.md
section 1, and its docs and `mise run verify` pass.

## 11. What C7 does not do

- No chain of forwarders, no forward limit, no copy-paste detection (the
  change doc's non-goals).
- No enforcement of `not-forwardable` against copy, a screenshot or a
  changed client.
- No mode for messages sent without one (`roymctl`, other clients). They
  stay `text/plain` and are not forwardable (Q3 (a)).
- No attachments (Q8).
- No default per conversation, and no group rule (ADR-0030).
- No forward of a quote's hidden fields, and no disclosures in a forward
  (ADR-0030, decision 10; agreements doc, open question 11).
- No forward of widgets, widget answers (D-C7-13), cards other than a
  quote, or system messages.
- No proof of which conversation or which receiver a signed message was
  written for (ADR-0030, decision 12), and no revocation check of origins
  (section 8).
- No new `roymctl` command (D-C7-12).
- No change to the interface of the substrate conversation capability.
  Its indexing rule gets one case (Q4 (a)).

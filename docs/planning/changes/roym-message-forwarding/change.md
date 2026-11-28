---
status: done
demo:
  - crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts
  - crates/substrate/tests/e2e/demo/50-roym-deal.spec.ts
living-docs-touched:
  - docs/roym-integrated-experience-spec.md
  - docs/TERMINOLOGY.md
  - docs/developer-guide.md
  - docs/system-requirements-spec.md
  - docs/system-architecture.md
  - docs/planning/traceability-matrix.md
---
# Roym message forwarding with a signed origin

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change lets a person forward a message to another chat. The forward keeps a **signed origin**: who first wrote the message and when, which the receiver can check. The receiver can see the difference between "my contact says this" and "my contact passes on what someone else said". The aim is to make people think before they start a harmful viral message, because their name stays on it.

The sender decides, for each message, between two **forward modes**:

- **Signed:** the message carries a signed origin, so it can be forwarded and the forward names its author.
- **Not forwardable:** the message carries a mark that says so. It has no origin. The Hub gives no forward action for it, and a forward cannot name the author.

A person sets a default mode in their settings. The sender can change the mode on any single message.

Terms used here:

- **Origin:** a small signed record made by the author of a message when it is sent. It names the author, the time and a hash of the content. It does not hold the content.
- **Forward:** a message that carries someone else's content plus that content's origin.
- **Forward count:** how many times the content was forwarded. Each forwarder adds one. It is not signed.
- **Forward mode:** the sender's choice for one message: `signed` or `not-forwardable`. The default mode is a setting of the person.
- **Hub:** the Roym web UI. **Client:** any program that uses Roym's methods.

This is a Roym feature. The substrate conversation capability's interface does not change; its search index gets one new case for a chat body (see [Deviations](#deviations)).

**This change needs an ADR before it is built.** A signed origin makes a message provable to strangers. That is a real loss of deniability (see [Privacy cost](#privacy-cost)). The owner chose the direction: the sender picks the forward mode, so only messages the sender chose to sign lose deniability. The ADR records this choice and the default mode. The choice is the same in a group as in a one-to-one chat. The ADR is [ADR-0030](../../../decisions/0030-signed-message-origins.md) (accepted 2026-11-16).

## Problem

Roym has no forward action. A person who wants to pass on a message copies the text. The receiver then sees it as the sender's own words. Nothing shows that it came from someone else, or from whom.

Two facts from the code shape the design:

- Each conversation entry is already signed by its author (`crates/conversation/src/dag.rs:148`). But the signature covers the ciphertext and the conversation context, and the author is a routing service id, not the person's identity. It cannot travel to another conversation.
- Roym's signed records (`profile`, `listing`, `quote`, …) are signed with the person's identity through the `signing` host interface. They can travel anywhere and still verify.

## Scope and non-goals

- In scope:
  - A forward mode on every message a person sends: `signed` or `not-forwardable`. A default mode in settings, and a change per message.
  - An origin on every `signed` message, signed with the person's identity.
  - A forward action in the Hub and as a Roym method.
  - Verification and display of the origin and the forward count.
  - Forwarding a signed Roym record (an offering, a quote, a profile card) as itself.
- Not in scope:
  - A **chain** of forwarders (A → B → C → you). Each receiver already knows who sent the message to them. A chain would show every middle person to everyone after them.
  - A limit on how many chats a forward can go to. People would copy and paste instead, which loses the origin.
  - A table of hashes of recent messages to detect copy-paste. Changing one character defeats it. A changed message is, correctly, the new sender's own words.
  - Enforcing `not-forwardable`. Nothing can stop a person from copying the text or from using a modified client. The mark makes honest clients refuse to forward. The real protection is that a forward of such a message cannot carry the author's name: it has no origin, so it counts as the forwarder's own words (FW-6, FW-13).

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| FW-1 | Every message a person sends through Roym has a forward mode, `signed` or `not-forwardable`. A `signed` message has an origin, signed with the person's identity: author master DID, send time, content type, language tag, and a hash of the content. A `not-forwardable` message has the mark and no origin. | `crates/roym_core/src/chat/tests.rs` (`a_signed_origin_checks_and_names_the_master_did`, `the_content_hash_is_stable`); parity scenarios 279 and 280 in `crates/roym_web/tests/dual_build_parity/forward.rs` |
| FW-2 | The forward action sends the original content, its origin, and the forward count plus one. It refuses a message that has no origin or is marked `not-forwardable`, in the Hub and in the Roym method. | Parity scenario 287 in `crates/roym_web/tests/dual_build_parity/forward.rs`; `crates/substrate/tests/roym_forward_e2e.rs`; `crates/roym_web/ui/src/chat/forward_view.test.ts` |
| FW-3 | The receiver's node checks that the origin's signature verifies and that the content hash matches. The Hub then shows "Forwarded. First written by <name> on <date>. Forwarded N times." | `crates/roym_core/src/chat/tests.rs` (`a_forwarded_checked_message_is_marked_forwarded`); parity scenario 286; `crates/roym_web/ui/src/chat/forward_view.test.ts`; demo story `20-roym-chat.spec.ts` |
| FW-4 | If the check fails, the Hub shows the message as the forwarder's own message, with the note "origin could not be checked". It never shows an unverified author name. | `crates/roym_core/src/chat/tests.rs` (`a_failed_origin_never_names_an_author`); `crates/roym_web/ui/src/chat/forward_view.test.ts` |
| FW-5 | A forward count of 5 or more shows as "Forwarded many times". | `crates/roym_core/src/chat/tests.rs` (`many_forwards_starts_at_five`); `crates/roym_web/ui/src/chat/forward_view.test.ts` |
| FW-6 | Editing the content before sending makes it a new message by the sender. The edited content cannot carry the old origin, because the hash no longer matches. | `crates/roym_core/src/chat/tests.rs` (the hash tests and `a_changed_text_with_the_old_origin_is_not_forwardable`) |
| FW-7 | A forwarded offering, quote or profile card travels as the signed record itself and verifies as it does today. | `crates/roym_core/src/record_forward/tests.rs`; `crates/roym_core/src/transaction/tests/verify.rs` (`a_quote_with_no_disclosures_shows_only_its_visible_fields`); parity scenario 289 in `crates/roym_web/tests/dual_build_parity/forward_record.rs`; `crates/roym_web/ui/src/chat/record_forward.test.ts`; Playwright case 44; demo story `50-roym-deal.spec.ts` |
| FW-8 | Attachments are covered: the content hash includes the hash of each attached blob. | `crates/roym_core/src/chat/tests.rs` (`the_hash_changes_with_the_blob_list`); met in the hash format only, as the "Attachments" Deviation says |
| FW-9 | Forward works the same in a one-to-one chat and in a group, inside the same group. | Parity scenario 288 (same conversation); `crates/substrate/tests/roym_forward_e2e.rs` (the three-node group); `crates/roym_web/ui/src/chat/forward_picker.test.ts` (a group row) |
| FW-10 | Hub settings, and the first message a person sends, say plainly that a `signed` message can be forwarded with the author's name and checked by others, and that a `not-forwardable` message cannot. The first message asks the person to choose the default mode. | `crates/roym_web/ui/src/chat/first_message.test.ts`, `crates/roym_web/ui/src/chat/forward_mode.test.ts`; demo story `20-roym-chat.spec.ts` |
| FW-11 | In a group, every member chooses the forward mode for each message they send, as in a one-to-one chat. The group owner cannot force or forbid a mode. The composer in a group says that a `signed` message can be shown by any member, to anyone, and forwarded with the author's name. | `crates/roym_web/ui/src/chat/forward_mode.test.ts` (the group notice) |
| FW-12 | A person has a default forward mode in settings. The composer shows the mode that applies to the message and lets the sender change it for that one message. Changing one message does not change the default. | Parity scenario 278; `crates/roym_web/ui/src/chat/forward_mode.test.ts` |
| FW-13 | On a message marked `not-forwardable`, the Hub shows no forward action and says why. If a modified client forwards it anyway, the forward has no origin, and the receiver shows it as the forwarder's own message (as in FW-4). | `crates/roym_core/src/chat/tests.rs` (`a_changed_text_with_the_old_origin_is_not_forwardable`, `a_failed_origin_never_names_an_author`); `crates/roym_web/ui/src/chat/forward_view.test.ts` |
| FW-14 | The mode is part of the message body, so the sender's conversation entry signature covers it. A forwarder cannot change a `not-forwardable` message to `signed`, or the reverse, without the change being visible as a different message. | Parity scenario 279 (the stored body holds `forward_mode`); `crates/roym_core/src/chat/tests.rs` (`a_chat_body_round_trips_through_bytes`) |
| FW-15 | A forwarded message stays forwardable as its author allowed. The forwarder's own default mode does not apply to content the forwarder did not write. | Parity scenario 286 (the forwarder's default is `not-forwardable`); `crates/roym_core/src/chat/tests.rs` (`forwarding_your_own_forward_keeps_the_count`) |

## Design

### Forward mode

Each message body carries a field `forward_mode`: `signed` or `not-forwardable`.

- `signed`: the message also carries the `message-origin` record below.
- `not-forwardable`: the message carries no origin. The receiving node and the Hub treat the message as not forwardable.

The field sits in the message body, so the sender's conversation entry signature covers it (FW-14). The receiving node does not trust the Hub to hide the forward action. The forward method checks the mode itself (FW-2).

The sender's default mode is a person-level Roym setting, kept on the person's own node. It is not sent to anyone. The composer shows the mode in use. The sender can change it on one message and the default stays as it was (FW-12). The plan sets where the setting is stored.

What `not-forwardable` does and does not do:

- It stops the forward action in honest clients, and it stops a forward from carrying the author's name.
- It does not stop copy and paste, a screenshot, or a modified client. Those pass on the words, not the author's signature, so the receiver sees them as the new sender's own words. This is the same as today.

### The origin record

A new Roym record type, `message-origin`, version 1:

```text
author        master DID of the person who wrote the message
sent_at_secs  send time
content_type  the message content type
lang          language tag of the text, or "und" when not stated
content_hash  SHA-256 over content_type, lang, body and attached blob hashes
```

It exists only for a `signed` message. It is signed like other Roym records, through the `signing` host interface, so it carries the delegation certificate and verifies on any node. It travels inside the message body with the content. The exact body format (a Roym content type) is set in the plan.

### Forward

The forward action takes a message the person received or sent. It sends a new message with:

- the original content, unchanged (only a `signed` message can be forwarded);
- the original `message-origin` record;
- `forward_count`: the received count plus one, or 1 for a first forward.

The forwarder's own conversation entry signature still covers the whole message, as for any message. So the receiver knows who forwarded it to them.

The forward keeps the author's `signed` mode. The next receiver can forward it again, with the count plus one (FW-15).

### What the forward count can and cannot say

The count is written by each forwarder and is not signed. A modified client can lower it or reset it. That is accepted: the count is a hint for readers, as in other chat apps. The **origin** is the part that cannot be faked: no one can make a valid origin naming someone else.

### Privacy cost

The privacy cost is real, not small.

- **Today:** each conversation entry is signed, but with the sender's routing service id, over the ciphertext and the conversation context. To prove a message to an outsider, a member would have to hand over the ciphertext, the key material that decrypts it, and the link from the routing service id to the person. That is possible, but hard, and not something a client does. `unverified`: how strong this proof is in practice. The ADR checks it.
- **With a signed origin:** the origin is signed with the person's master DID, over the plain content hash. Anyone, anywhere, can check it with no key material. A person can no longer say "I never wrote that" about a `signed` message. A `not-forwardable` message has no origin, so it stays as deniable as it is today.

**What decides deniability is whether the signed origin exists, not whether it is shown.** If a message carries a signed origin in its body, every member holds it. A hostile member can copy the record and show it to anyone. Hiding the origin when forwarding only stops honest clients. So any option that signs every message gives up deniability against a hostile member, in every conversation. When the sender picks the mode (option E), a `not-forwardable` message carries no origin, and keeps deniability.

That loss is the point of the feature where messages spread: it makes the author answerable. But in a private chat it is a cost with little gain. So the owner decides, in an ADR, among these options:

| Option | What it means | Deterrence | Deniability against a hostile member |
| --- | --- | --- | --- |
| A. Sign every message | Every message carries a signed origin. | Strong: every forward names its author. | None, in any conversation. |
| B. The author turns origins on, per conversation | Only messages in conversations where the author turned it on carry a signed origin. A forward of any other message shows "origin not available". | Weak: a person who starts a harmful message leaves it off. | Kept wherever the author left it off. |
| C. Sign every message, show the origin only inside the conversation by default | Every message carries a signed origin. A forward outside the conversation hides it unless the group allows it. | Medium for honest clients. | **None.** The signed origin is in every message, so a hostile member can still show it. Hiding it stops only honest clients. |
| D. No master-DID origin unless the group allows outside forwarding | Inside a conversation, members see the author from today's per-sender conversation entry signature, as now. Nothing new is added for them. A message carries a signed origin (master DID) only in a group that allows forwarding outside, and in a one-to-one chat only when the author turned it on. | Strong where messages spread (groups that allow outside forwarding, broadcasts). None in private chats, which is the intent. | Unchanged from today in every conversation that does not allow outside forwarding. Lost, as stated, in those that do. |
| E. The sender picks per message: `signed` or `not-forwardable`, with a default setting | A `signed` message carries a signed origin. A `not-forwardable` message carries none, and honest clients cannot forward it. The same in a one-to-one chat and in a group. | Strong where it counts: the forward action spreads only messages whose author signed them. A harmful message that is not signed cannot be passed on with the forward action. | Kept for every `not-forwardable` message. Lost for a `signed` one. |

- **Why not a shared-key code inside the conversation.** An earlier draft of D used a code made with one key that all members share. In a group of three or more, any member could then make a message that checks as "written by A" for the others, so it would prove less than today's per-sender entry signature. A separate key for each pair of members would fix that, but costs much more. D therefore keeps today's entry signature for authorship inside the conversation.
- **Option D's cost:** the author must know at send time whether the group allows outside forwarding, because the signature cannot be added later. A group that turns the setting on applies it only to messages sent after the change. In those groups every message carries a signed origin, and members are told so when they join.
- **What D keeps and what it does not.** D adds no new proof in private conversations. It does not make them more deniable than today either: today's entry signature already lets a hostile member try to prove a message, as described above.

**Direction: E**, chosen by the owner. It fixes the weak point of option B: with B, a person who starts a harmful message leaves the origin off and the message still spreads. With E, leaving it off means the forward action is not available. It also needs no group rule: the author decides for each message, in a group too (see [Group messages](#group-messages)). The difference from D is that D lets the group owner decide whether messages carry an origin, and E lets each author decide. The ADR records E and the default mode (see [Open questions](#open-questions)). C is not recommended: its privacy claim was wrong. The ADR may choose otherwise.


### Group messages

A group has no rule about forwarding. Every member chooses the forward mode for each message they send, with their own default, as in a one-to-one chat (FW-11). A group owner cannot force a member to sign, and cannot stop a member from signing.

- **A forward inside the group** works as in a one-to-one chat (FW-9).
- **A forward outside the group** carries the origin of a `signed` message. The author chose that, so the group does not limit it.
- **What the sender must know.** A `signed` message in a group is held, with its signed origin, by every member. Any member can show it to anyone, even if the member's client does not offer forward. So the composer in a group says this next to the mode (FW-11). A person who wants no proof of a message sends it as `not-forwardable`.
- **Members and the default.** A person's default mode applies in every group and chat. Per-group defaults are an open question.

### Why the author signs at send time

The author must sign when sending. A forwarder cannot get a signature later. The author may be offline, or may refuse. Signing costs one signature per `signed` message and the record's size. The size is measured in the plan (see [Open questions](#open-questions)).

### Legal note

India's IT (Intermediary Guidelines and Digital Media Ethics Code) Rules, 2021, Rule 4(2), require a significant social media intermediary that mainly provides messaging (more than 5 million registered users in India) to help identify the first originator of a message, when a court or an authority under Section 69 of the IT Act orders it. WhatsApp has challenged this rule in the Delhi High Court. Sources: [SFLC.in](https://sflc.in/what-originator-or-traceability-provision-information-technology-intermediary-guidelines-and/), [Stanford WILMAP](https://wilmap.stanford.edu/entries/information-technology-intermediary-guidelines-and-digital-media-ethics-code-rules-2021). Whether the rule applies to Roym, which has no central messaging service, is not known. This design does not depend on it. A signed origin is visible only to the people who receive the forward. No server sees it.

## Rejected options

- **A signed chain of forwarders.** See [Scope and non-goals](#scope-and-non-goals).
- **A forward limit.** See [Scope and non-goals](#scope-and-non-goals).
- **Reusing the conversation entry signature as the origin.** It covers ciphertext and conversation context, and names a routing service id, not the person. It cannot be checked in another conversation.
- **An origin only when the author opts in per conversation** is option B in [Privacy cost](#privacy-cost). It is weak as a deterrent, because harmful content would simply be sent without one and still spread. Option E, which the owner chose, closes that gap: a message without an origin has no forward action.
- **A "do not forward" mark alone, with no signed origin.** It cannot be enforced, and it gives the forwarder no author to name. It works here only together with the signed option.

## Open questions

- **The shipped default mode.** Recommendation: `not-forwardable`. It keeps today's deniability for every message the person does not think about, and choosing `signed` is a clear act of the author. The first message asks the person to choose (FW-10), so the shipped default matters only for people who skip the question. The owner confirms in the ADR.
- **A default per conversation.** One default for the person is the first version. A default per conversation or per group may be wanted later. It is not planned now.
- **A group owner's limit.** There is none now: members choose in every group. A group owner may want to forbid `signed` messages in a private group, as in option D. That would be a later addition, and it needs its own decision.
- **Size.** An origin record with its delegation certificate may be 1 KB or more. Measure it. If it is too large, carry the certificate once per conversation and a reference in each origin.
- **Display name.** Show the author's name from the reader's contacts, from the author's signed profile if the forward carries it, or the DID only. Recommendation: contact name if known, otherwise a short DID with "not in your contacts".

## Tasks

- [x] ADR: signed message origins and the sender's forward mode (option E), the default mode and deniability. [ADR-0030](../../../decisions/0030-signed-message-origins.md).
- [x] Write the slice plan (in M08C).
- [x] `forward_mode` field in the message body, the default-mode setting, and the composer control.
- [x] `message-origin` record type, signing at send for `signed` messages, verification at receive.
- [x] Forward method and Hub action, refusing `not-forwardable` messages. Display rules FW-3 to FW-5, FW-13.
- [x] Forward of signed records as themselves.
- [x] The group composer notice (FW-11). Settings and first-message text (FW-10). Update the integrated spec's privacy section at close-out.

## Deviations

From [ADR-0030](../../../decisions/0030-signed-message-origins.md) (2026-11-16). Where this list and the text above differ, this list holds.

- **The signature today, in a one-to-one chat, covers the plain message.** "Problem" and "Privacy cost" say the conversation entry signature covers the ciphertext. That holds for groups only (`crates/conversation/src/dag.rs`, `canonical_entry_bytes`). In a one-to-one chat the sender signs the plain message (`crates/conversation/src/envelope.rs`, `canonical_bytes`). What keeps today's messages deniable is the key: it is a random conversation key, and nothing signed by the person's master key says it is theirs (`crates/conversation/src/crypto.rs`, `PrekeyBundle`). This resolves the `unverified` note in "Privacy cost".
- **The origin payload is smaller.** It has no `author` and no `sent_at_secs`: the envelope's `issuer` (the master DID) and `issued_at_secs` are the author and the send time. It has `languages` (0 to 3 tags, as in the message) in place of one `lang` tag. `content_hash` covers `languages`. The `subject` is the empty string.
- **A message with no forward mode counts as `not-forwardable`.** This covers other clients, older content types and system messages.
- **No certificate, no `signed` message.** If the `conversation` service has no valid record-signing certificate, a `signed` send is refused. The node never changes the mode without asking. The Hub offers `not-forwardable` or a certificate renewal.
- **Forwarded records (FW-7).** A record travels as itself, in a forward body of its own content type, never as a deal card. It ignores the forward mode. In M08C the forward action offers only `listing`, `profile` and `quote`. A forwarded quote shows only its visible fields (agreements doc, open question 11), and the Hub warns that it shows both parties of the deal.
- **Size.** The certificate travels inside each origin, as in every Roym record. The ADR estimates about 940 bytes for a short text message. C7 measures a real one and stops to ask the owner if it is over 2 KiB.
- **The first message.** The Hub asks once. The person may skip, and the shipped default (`not-forwardable`) applies.
- **The chat body and the mode (Q3).** A message sent with a forward mode is `application/vnd.roym.message+json`: `chat_version` 1, `content_type` `text/plain`, `text`, `forward_mode`, optional `origin`, and optional `forward_count`. A message sent with no mode stays `text/plain` and is not forwardable.
- **The conversation capability's indexing rule (Q4 (a)).** Its interface does not change. A JSON body with a top-level string `text` is indexed by that field only; every other body is indexed as before. The `search` WIT doc comment says so.
- **The forward count.** A stored count is read as a number up to 1,000: 0, a negative number and a fraction read as no count, and a larger number reads as 1,000. A person's forward of their own forward does not raise it.
- **One `conversation.settings` method.** It holds the default mode and the first-message flag for the person, not a table per conversation.
- **The inner content and its hash.** The origin names `text/plain`, the 0 to 3 language tags, and a `sha256:` lowercase hex hash. The hash layout is the domain string `roym-message-origin-v1`; a `u32be` length prefix and the content type; a `u32be` tag count, then a `u32be` length prefix and the tag for each tag; a `u32be` length prefix and the body; a `u32be` blob count, then each 32-byte blob hash with no length prefix. The blob list is empty today.
- **Attachments (FW-8) are met in the format only.** The hash layout has a blob list, empty today. No message carries an attachment in this change.
- **Where the setting lives (Q5).** The default mode and the first-message flag are one row on the person's own node, in the `conversation_settings` collection of the `conversation` service's database (schema version 5). `settings` is the name of its section in a backup bundle, and the row travels there.
- **The check at read time (Q6).** `conversation.history`, `conversation.changes` and `conversation.search` check each chat body's origin again and return the view. The send path does not check its own origin.
- **The author's name and the delete note (Q9).** The Hub shows "you", the reader's contact name when it has one, the contact's short DID when the contact has no name, or the last 12 characters of an unknown DID with "not in your contacts". It never takes a name from the message. The delete dialog of a `signed` message says the origin and the forwards stay.
- **Tags for a message that cannot be forwarded (section 4, item 16).** A `not-forwardable` body shows "Not forwardable", a plain message shows no origin, and a `signed` body whose origin fails shows "origin could not be checked".
- **The measured origin and body sizes (step 4).** They are recorded under "Different from the plan": an 848-byte origin, a 1,037-byte body, and 100 and 164 ms for a 200-message history page.
- **The record check for a forward runs in the owning service (Q12 (a)).** `conversation` does not check a record-forward envelope; doing all three checks there would add 229,778 bytes against its 158,505-byte limit. The client asks `catalog.listing.verify`, `transaction.quote.verify` or `profile.verify-envelope` when it draws the row and draws only from that verdict. The receiver's own node still checks the record, so nothing trusts the sender.
- **A record-forward body is not indexed.** The host's `indexable_text` returns nothing for `application/vnd.roym.record-forward+json`, so `conversation.search` never matches an envelope or a record's own words. The Q4 bullet above covers chat bodies only.
- **The record-forward body.** It holds `record_forward_version`, `record_type` (one of `listing`, `profile`, `quote`) and `envelope`. The envelope is at most `MAX_CARD_BODY_BYTES` (160 KiB), checked before it parses. Another type, a bigger envelope, a body that is not JSON, and an envelope that is not a JSON object are refused with a `data.code`.
- **A forwarded profile is checked as its own subject.** `profile.verify-envelope` accepts a profile only when its `subject` equals its `issuer`. Adding a contact still requires the DID the person typed, as before.
- **The split into C7a and C7b (Q2).** C7a, the text message, and C7b, the forward of signed records, were built in two PRs: #399 and #400.

### Different from the plan

- **`origin_draft` returns the host signing draft.** The signing host takes its own `RecordDraft` type on both builds, with the payload as a JSON string, not the signed-record type with a JSON value. The envelope payload and the hash layout are unchanged; the chat tests convert the draft before building a test envelope.
- **Export creates the settings collection first.** Without that call a fresh node's first export had no `settings` key at all, and the bundle shape then depended on whether the person had ever opened the setting. The section is now present and empty on a fresh node.
- **The parity refusal for a group uses a restored group, not a removed one.** A group owner is always a member and `group.remove-member` refuses to remove the owner, so the parity harness cannot make its own node a removed non-owner. A restored group reaches the same group check before the signing call, so it proves the order the scenario is for. The three-node test keeps the group case.
- **The 11 KiB measurement does not assert the mailbox limit.** It prints the body size and whether it is under `MAILBOX_CARD_BODY_BYTES`, as the plan asks; the test does not make an offline mailbox part of this slice.
- **`check-roym-deps` allows `sha2` in a Roym service crate.** The content hash needs it in `roym_core`, and the hygiene check keeps a fixed allowlist, so that list gained the one entry.
- **The first-message question reports whether it asked.** `askOnce` resolves to `null` when the question was already answered, so the composer keeps the mode the person chose for this message; when it asks, it resolves to the answer, which also becomes the default. The plan gave it a plain `ForwardMode` return, which cannot tell "already asked" from "answered not-forwardable".
- **"Renew signing" opens the existing setup screen.** There is no route for it, so the button re-renders the screen the person is on after `screens/setup.ts` finishes. It never changes the mode on its own.
- **The chat picker labels a chat by name, peer DID, or address.** A direct chat whose peer is not a contact has no name, so it shows the conversation address the person already uses elsewhere.
- **The browser case checks the composer control and the Profile default, not the first-message question.** The Playwright Hub runs on one shared node, and other cases send on it before this one, so the question is already answered there. The question is covered by its Hub unit test (`chat/first_message.test.ts`) and by story 20 on fresh nodes, which answers it with "Signed" at Alice's first send.
- **Measured sizes and read time.** On the three-node test: a short signed text ("Market opens at 9", one `en` tag) has an 848-byte origin and a 1,037-byte chat body; an 11 KiB signed text has a 12,278-byte body, under the 13,312-byte mailbox estimate; a `conversation.history` page of 200 signed messages took 100 ms and 164 ms on two runs on the WASM build. The owner's limits are 2 KiB for an origin and 500 ms for the page.
- **The single-node Playwright case forwards a listing, not a quote.** A quote needs a second party, and the main Hub suite runs on one node. Case 44 publishes a listing, finds it in the directory and forwards it into a chat; the row draws the listing and no raw JSON. The two-node demo forwards a quote (story 50), so the quote block and its warning are shown on a real second node.
- **The forwarded quote block is new, not the C6b quote card.** That card draws a totals block and "You give" lines from a plain view; with no disclosures it would show zero prices. The Hub draws a forwarded quote from the verdict: both parties, the currency, the expiry, each commitment's id, giver and resource kind, and a note that amounts and details are hidden. There is no totals line, no description and no value.
- **A forwarded listing has its own small view, not the directory hit card.** The card shows the title, summary, categories and status. The directory hit card was not moved into a shared module.
- **A forwarded profile shows its name and about.** The Hub had no view for another person's profile. The new small view on the Contacts screen shows those two fields and the Forward action. Languages and translations are not shown (backlog).
- **A received record-forward row does not offer Forward again.** The three source screens do. A person who receives a forwarded record can open its source screen only if they hold the record; re-forwarding the row itself is in the backlog.

## Close-out

Before merge, update the living docs (integrated spec, requirements, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

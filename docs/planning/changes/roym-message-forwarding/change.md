---
status: draft
living-docs-touched:
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

This is a Roym feature. The substrate conversation capability does not change.

**This change needs an ADR before it is built.** A signed origin makes a message provable to strangers. That is a real loss of deniability (see [Privacy cost](#privacy-cost)). The owner chose the direction: the sender picks the forward mode, so only messages the sender chose to sign lose deniability. The ADR records this choice, the default mode, and what happens to group messages.

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
| FW-1 | Every message a person sends through Roym has a forward mode, `signed` or `not-forwardable`. A `signed` message has an origin, signed with the person's identity: author master DID, send time, content type, language tag, and a hash of the content. A `not-forwardable` message has the mark and no origin. | |
| FW-2 | The forward action sends the original content, its origin, and the forward count plus one. It refuses a message that has no origin or is marked `not-forwardable`, in the Hub and in the Roym method. | |
| FW-3 | The receiver's node checks that the origin's signature verifies and that the content hash matches. The Hub then shows "Forwarded. First written by <name> on <date>. Forwarded N times." | |
| FW-4 | If the check fails, the Hub shows the message as the forwarder's own message, with the note "origin could not be checked". It never shows an unverified author name. | |
| FW-5 | A forward count of 5 or more shows as "Forwarded many times". | |
| FW-6 | Editing the content before sending makes it a new message by the sender. The edited content cannot carry the old origin, because the hash no longer matches. | |
| FW-7 | A forwarded offering, quote or profile card travels as the signed record itself and verifies as it does today. | |
| FW-8 | Attachments are covered: the content hash includes the hash of each attached blob. | |
| FW-9 | Forward works the same in a one-to-one chat and in a group, inside the same group. | |
| FW-10 | Hub settings, and the first message a person sends, say plainly that a `signed` message can be forwarded with the author's name and checked by others, and that a `not-forwardable` message cannot. The first message asks the person to choose the default mode. | |
| FW-11 | In a group that does not allow outside forwarding, a member can send only `not-forwardable` messages, and the composer says so. Such a message has no signed origin. In a group that allows it, the sender's mode applies to each message. | |
| FW-12 | A person has a default forward mode in settings. The composer shows the mode that applies to the message and lets the sender change it for that one message. Changing one message does not change the default. | |
| FW-13 | On a message marked `not-forwardable`, the Hub shows no forward action and says why. If a modified client forwards it anyway, the forward has no origin, and the receiver shows it as the forwarder's own message (as in FW-4). | |
| FW-14 | The mode is part of the message body, so the sender's conversation entry signature covers it. A forwarder cannot change a `not-forwardable` message to `signed`, or the reverse, without the change being visible as a different message. | |
| FW-15 | A forwarded message stays forwardable as its author allowed. The forwarder's own default mode does not apply to content the forwarder did not write. | |

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
| E. The sender picks per message: `signed` or `not-forwardable`, with a default setting | A `signed` message carries a signed origin. A `not-forwardable` message carries none, and honest clients cannot forward it. In a group that does not allow outside forwarding, only `not-forwardable` can be sent. | Strong where it counts: the forward action spreads only messages whose author signed them. A harmful message that is not signed cannot be passed on with the forward action. | Kept for every `not-forwardable` message. Lost for a `signed` one. |

- **Why not a shared-key code inside the conversation.** An earlier draft of D used a code made with one key that all members share. In a group of three or more, any member could then make a message that checks as "written by A" for the others, so it would prove less than today's per-sender entry signature. A separate key for each pair of members would fix that, but costs much more. D therefore keeps today's entry signature for authorship inside the conversation.
- **Option D's cost:** the author must know at send time whether the group allows outside forwarding, because the signature cannot be added later. A group that turns the setting on applies it only to messages sent after the change. In those groups every message carries a signed origin, and members are told so when they join.
- **What D keeps and what it does not.** D adds no new proof in private conversations. It does not make them more deniable than today either: today's entry signature already lets a hostile member try to prove a message, as described above.

**Direction: E**, chosen by the owner. It fixes the weak point of option B: with B, a person who starts a harmful message leaves the origin off and the message still spreads. With E, leaving it off means the forward action is not available. It also takes from D the rule that a group which does not allow outside forwarding never carries a signed origin, so the group rule protects against a hostile member too (see [Group messages](#group-messages)). The difference from D is that the author chooses for each message where outside forwarding is allowed, and in one-to-one chats. The ADR records E, the default mode (see [Open questions](#open-questions)), and the group rule. C is not recommended: its privacy claim was wrong. The ADR may choose otherwise.


### Group messages

A group sets one rule: **"members may forward messages outside the group with their origin"**, on or off. The group owner sets it. The default is off for a private group.

- **Off:** members can send only `not-forwardable` messages in the group, and the composer says so. No signed origin exists in those messages, so a hostile member has none to show. This is what protects the group (FW-11).
- **On:** members see the rule when they join, and the sender's mode applies to each message: the person's default, or the choice made for that message. A `signed` message can then be forwarded outside the group with its origin.
- **A change applies only to later messages.** The author must know at send time whether the group allows it, because a signature cannot be added later.
- **Why the rule must stop signing, not only hide the origin.** A signed origin in the body is held by every member. Hiding it on forward stops only honest clients (see [Privacy cost](#privacy-cost)).

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
- **Group rule off and a one-to-one chat.** In a group with the rule off, members can only send `not-forwardable` messages. A one-to-one chat has no such rule, so the sender's mode is free there. The ADR confirms that a person may sign in a one-to-one chat.
- **Size.** An origin record with its delegation certificate may be 1 KB or more. Measure it. If it is too large, carry the certificate once per conversation and a reference in each origin.
- **Display name.** Show the author's name from the reader's contacts, from the author's signed profile if the forward carries it, or the DID only. Recommendation: contact name if known, otherwise a short DID with "not in your contacts".

## Tasks

- [ ] ADR: signed message origins and the sender's forward mode (option E), the default mode, deniability, and the group rule.
- [ ] Write the slice plan (in M08C).
- [ ] `forward_mode` field in the message body, the default-mode setting, and the composer control.
- [ ] `message-origin` record type, signing at send for `signed` messages, verification at receive.
- [ ] Forward method and Hub action, refusing `not-forwardable` messages. Display rules FW-3 to FW-5, FW-13.
- [ ] Forward of signed records as themselves.
- [ ] The group setting and FW-11. Settings and first-message text (FW-10). Update the integrated spec's privacy section at close-out.

## Deviations

## Close-out

Before merge, update the living docs (integrated spec, requirements, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

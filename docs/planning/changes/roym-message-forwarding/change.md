---
status: draft
living-docs-touched:
---
# Roym message forwarding with a signed origin

*Reader: developers who plan and build Milestone 8, and the project owner.*

This change lets a person forward a message to another chat. The forward keeps a **signed origin**: who first wrote the message and when, which the receiver can check. The receiver can see the difference between "my contact says this" and "my contact passes on what someone else said". The aim is to make people think before they start a harmful viral message, because their name stays on it.

Terms used here:

- **Origin:** a small signed record made by the author of a message when it is sent. It names the author, the time and a hash of the content. It does not hold the content.
- **Forward:** a message that carries someone else's content plus that content's origin.
- **Forward count:** how many times the content was forwarded. Each forwarder adds one. It is not signed.
- **Hub:** the Roym web UI. **Client:** any program that uses Roym's methods.

This is a Roym feature. The substrate conversation capability does not change.

## Problem

Roym has no forward action. A person who wants to pass on a message copies the text. The receiver then sees it as the sender's own words. Nothing shows that it came from someone else, or from whom.

Two facts from the code shape the design:

- Each conversation entry is already signed by its author (`crates/conversation/src/dag.rs:148`). But the signature covers the ciphertext and the conversation context, and the author is a routing service id, not the person's identity. It cannot travel to another conversation.
- Roym's signed records (`profile`, `listing`, `quote`, …) are signed with the person's identity through the `signing` host interface. They can travel anywhere and still verify.

## Scope and non-goals

- In scope:
  - An origin on every message a person sends, signed with the person's identity.
  - A forward action in the Hub and as a Roym method.
  - Verification and display of the origin and the forward count.
  - Forwarding a signed Roym record (an offering, a quote, a profile card) as itself.
- Not in scope:
  - A **chain** of forwarders (A → B → C → you). Each receiver already knows who sent the message to them. A chain would show every middle person to everyone after them.
  - A limit on how many chats a forward can go to. People would copy and paste instead, which loses the origin.
  - A table of hashes of recent messages to detect copy-paste. Changing one character defeats it. A changed message is, correctly, the new sender's own words.
  - An author's "do not forward" mark. It cannot be enforced (anyone can copy the text), and it would weaken the deterrence this change is for.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| FW-1 | Every message a person sends through Roym has an origin, signed with the person's identity: author master DID, send time, content type, language tag, and a hash of the content. | |
| FW-2 | The forward action sends the original content, its origin, and the forward count plus one. | |
| FW-3 | The receiver's node checks that the origin's signature verifies and that the content hash matches. The Hub then shows "Forwarded. First written by <name> on <date>. Forwarded N times." | |
| FW-4 | If the check fails, the Hub shows the message as the forwarder's own message, with the note "origin could not be checked". It never shows an unverified author name. | |
| FW-5 | A forward count of 5 or more shows as "Forwarded many times". | |
| FW-6 | Editing the content before sending makes it a new message by the sender. The edited content cannot carry the old origin, because the hash no longer matches. | |
| FW-7 | A forwarded offering, quote or profile card travels as the signed record itself and verifies as it does today. | |
| FW-8 | Attachments are covered: the content hash includes the hash of each attached blob. | |
| FW-9 | Forward works the same in a one-to-one chat and in a group. | |
| FW-10 | Hub settings say plainly that what a person sends can be forwarded with their name and checked by others. | |

## Design

### The origin record

A new Roym record type, `message-origin`, version 1:

```text
author        master DID of the person who wrote the message
sent_at_secs  send time
content_type  the message content type
lang          language tag of the text, or "und" when not stated
content_hash  SHA-256 over content_type, lang, body and attached blob hashes
```

It is signed like other Roym records, through the `signing` host interface, so it carries the delegation certificate and verifies on any node. It travels inside the message body with the content. The exact body format (a Roym content type) is set in the plan.

### Forward

The forward action takes a message the person received or sent. It sends a new message with:

- the original content, unchanged;
- the original `message-origin` record;
- `forward_count`: the received count plus one, or 1 for a first forward.

The forwarder's own conversation entry signature still covers the whole message, as for any message. So the receiver knows who forwarded it to them.

### What the forward count can and cannot say

The count is written by each forwarder and is not signed. A modified client can lower it or reset it. That is accepted: the count is a hint for readers, as in other chat apps. The **origin** is the part that cannot be faked: no one can make a valid origin naming someone else.

### Why sign every message

The author must sign when sending. A forwarder cannot get a signature later. The author may be offline, or may refuse. Signing every message costs one signature per message and the record's size. The size is measured in the plan (see [Open questions](#open-questions)). The privacy cost is small: each conversation entry is already signed by its author, so the receiver can already show that the author sent it.

### Legal note

India's IT Rules 2021 ask large messaging services to be able to identify the "first originator" of a message (`unverified`: the exact rule and whom it covers). A signed origin gives this information to the people who receive the message. It changes nothing in end-to-end encryption, and no server sees it.

## Rejected options

- **A signed chain of forwarders.** See [Scope and non-goals](#scope-and-non-goals).
- **A forward limit.** See [Scope and non-goals](#scope-and-non-goals).
- **Reusing the conversation entry signature as the origin.** It covers ciphertext and conversation context, and names a routing service id, not the person. It cannot be checked in another conversation.
- **An origin only when the author opts in.** Then harmful content would simply be sent without one. A forward would then look the same as the forwarder's own message, and the deterrence is lost.

## Open questions

- **Size.** An origin record with its delegation certificate may be 1 KB or more. Measure it. If it is too large, carry the certificate once per conversation and a reference in each origin.
- **Display name.** Show the author's name from the reader's contacts, from the author's signed profile if the forward carries it, or the DID only. Recommendation: contact name if known, otherwise a short DID with "not in your contacts".

## Tasks

- [ ] Write the slice plan (in M08C).
- [ ] `message-origin` record type, signing at send, verification at receive.
- [ ] Forward method and Hub action. Display rules FW-3 to FW-5.
- [ ] Forward of signed records as themselves.
- [ ] Settings text (FW-10). Update the integrated spec's privacy section at close-out.

## Deviations

## Close-out

Before merge, update the living docs (integrated spec, requirements, terminology) so they describe the result. Then set `status: done` and fill in `living-docs-touched`. Record anything you postponed in [deferred-backlog.md](../../deferred-backlog.md).

# ADR 0030: Signed message origins and the sender's forward mode

Reader: developers who build Roym chat, and the project owner.

## Status
Accepted

## Related
- [Roym message forwarding](../planning/changes/roym-message-forwarding/change.md): the change this ADR decides for (option E).
- [Roym agreements](../planning/changes/roym-agreements/change.md), open question 11: a forwarded quote carries no disclosures in M08C. This ADR follows it.
- ADR-0025 (the conversation capability owns history)
- ADR-0013 (P2P messaging and identity architecture)

## Context
A Roym person wants to pass on a message to another chat. Today they copy the text, and the receiver sees it as the sender's own words. The forwarding change adds a **forward** that keeps a **signed origin**: who first wrote the message and when, which any receiver can check. A forward makes the author answerable. That is the point of it where messages spread. It is also a real loss of deniability for the author.

Terms used here:

- **Origin:** a `message-origin` record. The author's node signs it at send time, with the person's identity. It names the content by a hash. It does not hold the content.
- **Forward mode:** the sender's choice for one message, `signed` or `not-forwardable`.
- **Delegation certificate:** a JSON record, signed by a person's master key, that lets one service key sign records as that person ([`DelegationCertificate`](../../crates/identity/src/delegation.rs#L62)).
- **Deniability:** whether a person can credibly say "I never wrote that" to someone outside the conversation.

What the code does today (checked on `main` at `48ea989c`):

- **One-to-one messages.** The sender signs a `DeliveryPayload` over the **plain** message: message id, conversation id, author, time, content type, languages and body ([`canonical_bytes`](../../crates/conversation/src/envelope.rs#L92), [`sign`](../../crates/conversation/src/envelope.rs#L132)). The session encryption is separate.
- **Group messages.** The author signs a conversation entry over the header and the **ciphertext** ([`canonical_entry_bytes`](../../crates/conversation/src/dag.rs#L156), [`sign_entry`](../../crates/conversation/src/dag.rs#L260)). The epoch key opens the ciphertext, and the header is the AEAD's associated data ([`seal`](../../crates/conversation/src/dag.rs#L270)).
- **The key that signs both.** It is a random ed25519 key that the conversation store makes once ([`generate_identity_bytes`](../../crates/conversation/src/crypto.rs#L161)). It is not the person's master key. Its only link to a person is a self-signed prekey bundle and trust on first use ([`PrekeyBundle`](../../crates/conversation/src/crypto.rs#L45)): "this bundle alone proves internal consistency, never who actually holds it". In a group, the peer assertion is self-signed too ([`sign_peer_assertion`](../../crates/conversation/src/dag.rs#L353)).
- **The `message` WIT record** has `content-type: string` and `body: list<u8>` ([conversation.wit:50](../../crates/wit_interfaces/wit/conversation/conversation.wit#L50), `content-type` at line 67). A forward mode and an origin fit in a Roym message body with no WIT change.
- **Roym signed records** (`listing`, `profile`, `quote`, …) go through the `signing` host interface ([`sign-record`](../../crates/wit_interfaces/wit/signing/signing.wit#L74)). With `principal::delegated` ([signing.wit:27](../../crates/wit_interfaces/wit/signing/signing.wit#L27)), the host signs with the service's own key, sets `issuer` to the person's master DID, and puts the certificate JSON in the envelope's `delegation` field ([`Envelope`](../../crates/signed_record/src/envelope.rs#L145)). The `delegation` field is inside the signed bytes ([`signing_bytes`](../../crates/signed_record/src/envelope.rs#L190)).
- **Each Roym service holds its own certificate**, one row, for its own key ([`person_principal`](../../crates/roym_core/src/signing.rs#L192)). The `conversation` service already has `conversation.install-signing-certificate` ([api.rs:47](../../crates/roym_conversation/src/api.rs#L47)).
- **A record verifies after its certificate expires.** The verifier checks that the record was signed inside the certificate's window, not that the window is still open ([verify.rs:206](../../crates/signed_record/src/verify.rs#L206)).
- **Deal cards** go only into a one-to-one chat ([messages.rs:290](../../crates/roym_conversation/src/app/messages.rs#L290)). The `transaction` service files every message whose content type is the card type ([sync.rs:207](../../crates/roym_transaction/src/app/sync.rs#L207)), and refuses a quote card that names another conversation ([sync.rs:408](../../crates/roym_transaction/src/app/sync.rs#L408)).

**Size of an origin (estimate).** No test fixture prints a signed envelope with a certificate, so this is computed from the formats in the code, not measured. A `did:key` is 64 characters (`did:key:h` plus z-base-32 of 34 bytes, [`derive_did_key`](../../crates/identity/src/substrate.rs#L142)). A signature is 103 characters (z-base-32 of 64 bytes). A certificate JSON is about 364 bytes. A `message-origin` envelope for a short text message is about 940 bytes with the certificate, and about 540 bytes without it. The certificate is stored as an escaped JSON string inside the envelope, so it costs about 400 bytes there. A message body may be up to 256 KiB ([store.rs:92](../../crates/conversation/src/store.rs#L92)). C7 measures a real one.

## Decision

1. **Option E: the sender picks the forward mode for each message.** Every message a person writes in Roym has a `forward_mode` in its body: `signed` or `not-forwardable`. A `signed` message also carries a `message-origin`. A `not-forwardable` message carries no origin, and no honest client offers to forward it. The rule is the same in a one-to-one chat and in a group. A group owner cannot force a mode and cannot forbid one. There is no group rule about forwarding.

2. **The mode is in the signed message body.** It sits in the body, so the sender's own message signature covers it (one-to-one: the plain body; group: the ciphertext, bound by the AEAD). A forwarder cannot change the mode of a message without the change showing as a different message.

3. **A message with no forward mode counts as `not-forwardable`.** This covers a message from another client, a message of an older content type, and every system message. Nothing can be forwarded without an origin.

4. **The shipped default is `not-forwardable`.** The person sets their own default in Hub settings. The composer shows the mode for the message and lets the sender change it for that one message. The default does not change.

5. **The first message.** Before a person's first message is sent, the Hub asks them to choose their default. It says in plain words that a `signed` message can be forwarded with their name and checked by anyone, and that a `not-forwardable` message cannot. The person may skip the question. Then the shipped default applies. The Hub asks only once.

6. **The origin record.** A new Roym record type, `message-origin`, version 1, signed through the `signing` host interface as the person (`principal::delegated`), by the `conversation` service. The envelope's `issuer` is the author's master DID, and its `issued_at_secs` is the send time. So the payload does not repeat them. The payload holds:

   ```text
   content_type   the message content type
   languages      the message's language tags, as in the message (0 to 3)
   content_hash   SHA-256 over a domain tag, content_type, languages,
                  the body, and the hash of each attached blob,
                  each field length-prefixed
   ```

   The `subject` is the empty string. The origin names no conversation and no receiver, so it can travel to any chat. The exact body format of a Roym message (a Roym content type) is set in the C7 plan.

7. **No certificate, no `signed` message.** If the `conversation` service has no certificate, or its certificate is stale or expired, a `signed` send is refused with the same words as every other signing verb. The node never changes a `signed` message to `not-forwardable` without asking. The Hub then offers to send the message as `not-forwardable`, or to renew the certificate.

8. **A forward.** A forward is a new message that carries the original content unchanged, the original `message-origin`, and `forward_count` (the received count plus one, or 1). The forward keeps the author's `signed` mode, so the next receiver can forward it again. The forwarder's own default does not apply to content the forwarder did not write. The forwarder's own message signature covers the whole forward, so each receiver knows who passed it on. The forward method on the node checks the mode itself and refuses a message with no origin. It does not rely on the Hub to hide the action. A forward may go to any chat the forwarder is in, one-to-one or group.

9. **The receiver checks the origin.** It verifies the envelope and checks that `content_hash` matches the content. Only then does the Hub show the author ("Forwarded. First written by …"). If either check fails, the Hub shows the message as the forwarder's own words, with the note "origin could not be checked". The `forward_count` is not signed and is only a hint.

10. **Forwarded Roym records travel as themselves.** A signed Roym record travels **as itself**, in a forward body of its own content type. It does not follow the message's forward mode and carries no `message-origin`, because the record is already signed by its issuer. It is never sent as a deal card, so the receiver's `transaction` service never files it as a deal. In M08C, the forward action offers this for three record types only: an offering (`listing`), a `profile` and a `quote`. A forwarded quote carries no disclosures (agreements doc, open question 11). So the receiver sees only its visible fields: the ids, the two parties, the currency, the expiry, and for each commitment its id, giver, resource kind, commitment kind, extension and `after` list. The receiver sees no scope, no description and no value. Before a person forwards a quote, the Hub says that the forward shows both parties of the deal.

11. **The certificate travels inside each origin.** An origin carries its delegation certificate in the envelope's `delegation` field, as every Roym record does today. The verifier does not change. C7 measures a real origin for a short text message. If it is over **2 KiB**, C7 stops and asks the owner. The fallback design is then: an envelope version 2 with a `delegation_ref` (the hash of the certificate) in place of `delegation`; the sender sends its certificate once per conversation, as a system message; and a forward always carries the full certificate, because the next chat has never seen it. The fallback changes the signed-record format for every record type, so it needs its own decision.

12. **What `signed` gives up, and what `not-forwardable` keeps.** This is stated to the person in settings, at the first message and in the group composer.
    - **Today, for every message:** a member who holds a message can show a third party the content and a signature from the author's conversation key. In a one-to-one chat this needs no key material, because the signature covers the plain message. In a group the member must also hand over the epoch key, which opens every other message of that epoch. But nothing signed by the person's master key says that the conversation key is theirs. The member can only claim it. So the person can still credibly say "that key is not mine".
    - **A `signed` message:** the origin is signed by the person's master DID, through a certificate the master key signed. Anyone, anywhere, can check it with no key material and with nothing but the content. It still verifies after the certificate expires. It cannot be taken back. Every member of the chat holds it, so any member can show it to anyone, even with a client that offers no forward. The origin proves who wrote the content and when. It does not prove to whom it was sent.
    - **A `not-forwardable` message:** it carries no origin. It is exactly as deniable as a message today. The mark stops honest clients from forwarding. It does not stop copy and paste, a screenshot or a modified client. Those pass on the words without the author's signature, so the receiver sees them as the forwarder's own words.

## Consequences
- The deniability loss of a `signed` message is permanent and has no time limit. The Hub must say so before a person first chooses `signed`.
- Each `signed` message costs one signing call and about 1 KB (estimate). A `not-forwardable` message costs nothing new.
- Signing an origin needs the `conversation` service's certificate. A person who has not enrolled signing on `conversation` can send only `not-forwardable` messages until they do.
- An origin made for one message can be attached to the same content anywhere. That is the purpose: the content is the author's, wherever it goes.
- A forwarded quote shows both parties' DIDs to the receiver. Only the two parties hold the quote at first, so the first forward always comes from a party.
- `widget-answer` records (roym-extensions, EX-12) are signed by the person, so a poll vote is as provable as a `signed` message, whatever the person's forward mode. This ADR does not decide that record. The C4 plan decides it with the owner. A vote may not need the master-DID signature at all: a poll counts votes inside one chat, where the message signature already says which member sent it.
- Not decided here, and left to the C7 plan: where the default setting is stored, the exact body format, and the display name of an author (the forwarding change recommends the contact name, else a short DID with "not in your contacts").
- Not in M08C: a default per conversation, a group owner's limit on `signed` messages, and a forward that shows a quote's hidden fields. A group owner's limit needs its own decision. Showing a quote's amounts is for the M08G disclosure ADR.

## Alternatives considered
- **A. Sign every message.** Rejected: no message is deniable, in any chat.
- **B. The author turns origins on per conversation.** Rejected: a person who starts a harmful message leaves it off, and the message still spreads by the forward action.
- **C. Sign every message, and show the origin only inside the conversation.** Rejected: every member holds the signed origin, so a hostile member can show it anyway. Hiding it stops only honest clients.
- **D. The group owner decides whether messages carry an origin.** Rejected: the author, not the group owner, pays the deniability cost, so the author decides.
- **Reuse the conversation message signature as the origin.** Rejected: it is made by a conversation key with no signed link to the person, and in a group it covers the ciphertext and the conversation context. It cannot be checked in another chat.
- **A forwarded record follows the message's forward mode.** Rejected: the record is already signed by its issuer, and both parties hold it. A `not-forwardable` mark on it would stop only honest clients and would give the issuer a false sense of privacy.
- **Carry the certificate once per conversation from the start.** Not chosen now: it changes the envelope format of every record type to save about 400 bytes per `signed` message. It is the fallback if C7's measurement is too large.

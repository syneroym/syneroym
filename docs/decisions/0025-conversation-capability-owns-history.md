# ADR 0025: Inbound admission, deletion, search, history export and group names belong to the conversation capability

## Status
Accepted

## Related
- ADR-0013 (§5 ordering rule, §6 layers)
- ADR-0023 (durable async primitives: the outbox the re-ask loop runs in)

## Context
`syneroym:conversation` originally stored and delivered messages but offered no way to refuse, delete, search, export or import them. Roym worked around this with a second full copy of every message in app storage. Keeping two stores in step caused lost messages, stale delivery states and a transcript check that could miss membership changes. It also doubled disk use, left "deleted" messages readable in the host store, and left a rate-limited sender with no sign of the refusal. Every future chat SynApp would face the same gaps.

## Decision
The conversation capability provides:
- A durable app answer for each incoming message (`accept`, `hold`, `drop`), asked once per message and again only until answered, with readmit and a time limit on held bodies; an app with no inbox declares it, and only then are its messages accepted without asking.
- A refusal report to the sender when the app asks for one (never for a block).
- Local delete with wiped free pages, and an author-only "please delete" request.
- Group events and owner-signed group names inside history, and a transcript digest over the whole synced log.
- A feed of newly visible messages in local order (`changes`).
- Full-text search with substring matching via SQLite FTS5.
- Paged, versioned export and import of history; imported rows are marked restored, are not verified, and are never sent, synced or relayed.

Apps keep only their policy (who to accept) and their product data.

## Consequences
- One store per service, and no app-side copy.
- Deleting or dropping removes the readable body from the host store and its free pages. For a group, the encrypted log entry and the group key remain on the node.
- Restore of history is a capability feature, not an app feature. A restored group is read-only.
- Every imported message is marked restored, also in a chat that stays live after a same-address restore. `retry` refuses it, so a message that was still pending at backup time can never be sent again.
- A dropped row is kept so a repeat delivery is recognised. Each direct chat keeps the newest `max_dropped_per_conversation` of them; group chats keep all, because every member must hold the same rows for the transcript code to match. The transcript code is therefore meant for groups; a direct chat's code changes on the receiving side when its dropped rows are pruned.
- Deleting a 1:1 message that is still pending cancels further delivery attempts and still sends a deletion request, because a send in flight or a lost receipt means the peer may already hold it. A peer that never got the message ignores the request. A group message is already in the group log, which members read on their own, so delete removes the local text and sends a deletion request. A member that receives the request before the message remembers it, and stores the message without its text when it arrives. A deleted message cannot be retried.
- Rewriting the search index after a delete or drop is spread over `scrub_min_interval_secs`, so a flood of drops cannot hold the store busy. The first scrub after a quiet time runs at once. Whether a scrub is still needed is stored in the database, in the same transaction as the delete or drop, so a restart neither loses a needed scrub nor runs one that is not needed.
- The WIT interface changes four records, adds six types and eight functions, removes `membership-history`, and changes the `on-message` result.
- Continuing a conversation after a move to a new machine is still open and tracked in deferred backlog.

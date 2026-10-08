# Fix coverage, part D1 (Layer 2 audit)

Reader: the developer who closes out the architecture fix.

Scope: every DIVERGES, NOT BUILT, STALE and UNCLEAR row of
[audit-architecture-layer2.md](audit-architecture-layer2.md) (41 rows), its
questions Q1 to Q8, its structural problems (section 3, items 1 to 9) and its
proposed split (section 5). All checks were made against the current
`docs/system-architecture.md` in the worktree (HEAD `4e667af3`) and against the
code. Layer 2 moved a lot, so "Doc place" names headings, not lines.

## 1. Summary

| Verdict | Count |
| --- | --- |
| APPLIED | 54 |
| PARTLY | 0 |
| NOT APPLIED | 0 |
| OVERTAKEN (doc follows the later decision) | 3 |
| NOT APPLICABLE | 2 |
| **Total** | **59** |

Rows: 41 claim rows (39 APPLIED, 2 OVERTAKEN), Q1 to Q8 (7 APPLIED, 1 OVERTAKEN),
section 3 items 1 to 9 (8 APPLIED, 1 NOT APPLICABLE), section 5 (1 NOT APPLICABLE).

L2-39 / Q3 (the write-rules table) is now done by commit `4e667af3`
("move Roym write rules to layer 4"). No other item was missed. There is no
PARTLY or NOT APPLIED row, so section 3 below is empty.

## 2. Items

### 2.1 Claim rows

| Item | Verdict | Doc place | Note |
| --- | --- | --- | --- |
| L2-02 | APPLIED | Layer 2 > Internal Architecture > "Ingress." and the WebSocket paragraph | "HTTP/1.1 requests travel inside these streams ... JSON-RPC 2.0"; WebSocket is "an option for one app", frames are app-defined, "not JSON-RPC". The diagram has no WebSocket node. |
| L2-03 | APPLIED | Same diagram (node `PROXY` "Universal Proxy: service-to-service calls"); top Implementation Note | The wRPC node is gone. Top warning says wRPC is not implemented. |
| L2-07 | APPLIED | "Access control." three numbered layers | Stream identity, per-service admission, row-level policy (FDAE). No "Access Control Engine" node. |
| L2-08 | APPLIED | Diagram subgraph `ROUTING` "Connection Router" | Renamed. |
| L2-11 | APPLIED | "Sandboxes." Podman bullet | Q7 default: "calls the host's `podman` command"; prefers rootless, does not check. Matches `docs/developer-guide.md:561`. See problem P1 (repeated sentence). |
| L2-13 | APPLIED | Diagram node `SQLITE` | `CRSQL` is gone. No `crsql` in the doc. |
| L2-14 | APPLIED | Diagram `QUEUE` "Durable outbox SQLite"; "Durable outbox." paragraph | Says worker retries, file is next to the service database, on the node. Checked `crates/router/src/proxy_outbox.rs:1-15` (one queue per calling service, sibling file). |
| L2-16 | OVERTAKEN | Packaging > Envisioned "Replicated backups"; Storage > Envisioned; PLT-RED | Q-1 says keep Litestream as an option. Diagram node is gone. Text says "Litestream is another option" and no text says PLT-RED rejects it. Doc follows Q-1. |
| L2-17 | APPLIED | Storage paragraph on the blob store | Only the optional `aws` S3 blob backend is stated as built. S3 backups are under Envisioned. |
| L2-18 | APPLIED | "Not substrate components." (end of Internal Architecture) | Q2: Roym features. Names the `directory` service. |
| L2-19 | APPLIED | Same paragraph | Reputation is not a substrate part. Layer 3 and Phase 5 state that no reputation record exists. |
| L2-20 | APPLIED | Same paragraph | Names payment records and signed receipts of `transaction`. |
| L2-21 | APPLIED | Diagram edges `IDENT --> PIPE`, `PIPE -->|...| ...`, `WASM -->|host capabilities ...| STORAGE`; "Routing." | Redrawn as stream, identity, pipeline, service. Guest goes to storage by host capabilities. |
| L2-22 | APPLIED | "**Roles.**" table | Nine roles. I compared with `crates/core/src/config/roles.rs:9-21` (`app_sandbox`, `podman_sandbox`, `community_registry`, `coordinator`, `client_gateway`, `auth`, `observability`, `supervisor`, `roym`). MQTT broker and conversation host are named below the table. |
| L2-28 | APPLIED | Packaging > "Backup and Restore" + Envisioned block | Roym backup commands as built. "There is no `syneroym` binary and no generic app export." |
| L2-29 | APPLIED | Same section, bullet list | Master identity (encrypted) and data of five services. The SQLite snapshot, blob store and App Spec are under Envisioned. `EXPORT_SERVICES` in `apps/roymctl/src/commands/roym/backup.rs:33` has the same five. |
| L2-30 | APPLIED | Same section, paragraph "The command encrypts ..." | "Encrypted ... header authenticated ... Each service bundle has a manifest that the person signs. Restore checks the signature." Checked `crates/roym_core/src/backup.rs:75-79,192-197,250-260`. |
| L2-31 | APPLIED | Same section, "Restore accepts only archive version 1." | Checked `apps/roymctl/src/commands/roym/backup.rs:25,229,258`. |
| L2-32 | APPLIED | Same section, "Restore has two commands." | `restore-identity` and `restore-data` through the gateway. "Safe to run again": the five `import` handlers use `Put` mutations (profile, catalog, directory) or skip stored rows (conversation `backup.rs:105`). |
| L2-33 | OVERTAKEN | Envisioned "Replicated backups" | Q-1 and the later redundancy decision: replication design is open, Litestream and Iroh WAL shipping are both options. "A live copy of a service database ... no final design." |
| L2-34 | APPLIED | Not in the doc | Mutual backup pool dropped (Q1 default). Recorded in change.md Deviations (the "Q1 against keep the vision" entry and the redundancy decision entry). See problem P2 (requirements spec still names peer backup pools). |
| L2-35 | APPLIED | Envisioned "Replicated backups"; PLT-RED "Failure Philosophy" | Active failover is gone. One rule remains: "promoting a secondary by hand"; PLT-RED says "no automatic failover". |
| L2-38 | APPLIED | Storage > Envisioned; Multi-Device B > `replicas = N` paragraph | "Today there is one database per service and no replica role." `replicas` is explained as N independent members, each with its own database. |
| L2-39 | APPLIED | Storage > "Write rules." and Layer 4 > Booking State Machine ("Listing and message rules.") | Done by `4e667af3`. Layer 2 now names only the substrate primitives (one writer, `put`, `create` fence, FDAE policy rule) and links to Layer 4 for the Roym rules. The table is gone from Layer 2. |
| L2-40 | APPLIED | Layer 4 > Booking State Machine ("Slot claiming." and the Envisioned block after it) | Real rule: one decision per agreement, first claim wins, written on the provider's node only. "Provider beats consumer" sits under Envisioned and says the nearest rule today is the agreement decision. Code: `crates/roym_transaction/src/app/ledger.rs:62,124-126,152`. |
| L2-41 | APPLIED | Layer 4 > "Listing and message rules." | "saves the whole listing with `put`, so the last write wins for the listing. Each version is also kept in a history collection." Code: `crates/roym_catalog/src/app/listing_ops.rs:232-245` (`LISTINGS` and `LISTING_HISTORY`). |
| L2-44 | APPLIED | Not in Layer 2 | Reputation row dropped (change.md says so). Layer 3 and Phase 5 say there is no reputation record. |
| L2-45 | APPLIED | Storage > "Write rules." last sentences | Says only: one policy document per service, a new policy replaces the old one at once (ADR-0017). No "operator cannot override" claim anywhere in the doc (searched). Code: `crates/data_db/src/traits.rs:112-116`. |
| L2-46 | APPLIED | "Durable outbox." paragraph; Multi-Device A (Envisioned) | Node-side outbox as built. "The SDK client does not set an idempotency key"; device queue is Envisioned. |
| L2-47 | APPLIED | Multi-Device A, first bullet | Kept as a design rule under an Envisioned marker (Q4). |
| L2-48 | APPLIED | Multi-Device A, second bullet | Under Envisioned. |
| L2-49 | APPLIED | Multi-Device A, third bullet | Under Envisioned. |
| L2-51 | APPLIED | Multi-Device B, first paragraph | "A manifest names a substrate with `[placement]` ... default, and each service can override it." Code: `models/manifest.rs:24`, `models/service.rs:306`. |
| L2-52 | APPLIED | Multi-Device B > Envisioned "Resource-class scheduling" | "Today the only attribute in the inventory is the list of service types." |
| L2-53 | APPLIED | Multi-Device B, "Calls between services on different substrates use Iroh QUIC with JSON-RPC." | Sharded mode "never produced by the compiler" is under Envisioned. |
| L2-54 | APPLIED | Same paragraph; Envisioned "Queued dependents" | "A call to a service that is down fails after the retry policy. A guest can queue a call in the durable outbox." |
| L2-55 | APPLIED | "Example placement: the Roym services ..." | Real six names. |
| L2-56 | APPLIED | Substrate API Surfaces, first paragraph | "The substrate has one API surface: JSON-RPC 2.0." |
| L2-57 | APPLIED | Same section, Envisioned "wRPC surface" | Moved under the marker. |
| L2-58 | APPLIED | Same section, Envisioned "wRPC surface" | "WIT types are kept end to end, with no JSON conversion" is now a wRPC goal, and the note says today every call converts. |
| L2-61 | APPLIED | Same section, Envisioned "OpenRPC schema" | Under the marker. |

### 2.2 Questions

| Item | Verdict | Doc place | Note |
| --- | --- | --- | --- |
| Q1 Replication target | OVERTAKEN | Envisioned blocks in Packaging, Storage, PLT-RED, Security R2, Technology Stack, Toolchain | The owner decision (Litestream stays an option) wins over the default (drop Litestream). Both options are named everywhere and none is called "the design". I searched all seven `Litestream` hits; each is under an Envisioned marker. |
| Q2 Shared utilities | APPLIED | "Not substrate components."; System Layers Overview diagram and the paragraph after it | Matching, reputation and payment are not in any substrate diagram. Layer 3 holds only Identity and Messaging in the layers diagram. |
| Q3 Arbitration table | APPLIED | Storage "Write rules." and Layer 4 Booking State Machine | Real rules (agreement decision, seat claim, listing replace, message log) are in Layer 4. Order and reputation rows are dropped or Envisioned. The Roym section of this file is the "Roym docs" place until the file split. Resolved TBD rows 3 and 4 point to both places. |
| Q4 Multi-device | APPLIED | Multi-Device A | Kept, Envisioned. |
| Q5 Scheduling | APPLIED | Multi-Device B | Operator-named placement as built. Scheduling Envisioned. |
| Q6 App export | APPLIED | Packaging > Backup and Restore | Roym backup as built. Generic export Envisioned. |
| Q7 Rootless Podman | APPLIED | Sandboxes > Podman bullet; Technology Stack row; developer guide line 561 | Advice, not a check. |
| Q8 Operator cannot override | APPLIED | Storage > Write rules | Not claimed. |

### 2.3 Structural problems (section 3) and the split (section 5)

| Item | Verdict | Doc place | Note |
| --- | --- | --- | --- |
| S3-1 Two documents in one file | APPLIED | Top Migration Note; "Target Designs (Addendum)" | One `#` title only. The note says Layers 1 to 4 are canonical and the addendum adds detail. The Litestream conflict is gone (see Q1). |
| S3-2 Duplicate "Layer 3" heading | APPLIED | Headings | I extracted all headings with a script: no duplicates. |
| S3-3 Broken table of contents | APPLIED | Table of Contents | Script check with GitHub slug rules: 0 broken `](#...)` links in the file. The Appendix and the addendum are in the table. The "MVP Phase 1" entry is gone. |
| S3-4 Plans written as working systems | APPLIED | Status legend, line 3 | 93 `> **Envisioned.**` markers. Every unbuilt Layer 2 item I checked is under one. |
| S3-5 Milestone and slice IDs | APPLIED | Whole file | Searched `M0x`, `Slice`, `D-xx`, `milestone`, `Implementation Status`, `2026-07`: no hits (the `M1`..`M4` hits are box labels in message diagrams). |
| S3-6 Commit hash | APPLIED | Migration Note; Addendum intro | The visible hash is gone. `dd864a1` is left only inside the link anchor `#post-dd864a1-target-specifications-addendum`. The anchor exists in `docs/system-requirements-spec.md` (line 774), and change.md Deviations records this. |
| S3-7 Other places repeat Layer 2 claims | APPLIED | Simulation Testing (write-rule bullet); Resolved TBD rows 1 to 4 | Both now match the fixed text. |
| S3-8 Mixed readers | NOT APPLICABLE | n/a | The fix is the file split (change.md task "Split the architecture doc", still open). Today only Layer 1 and Layer 2 have a `*Reader:*` line. See P4. |
| S3-9 Stale diagram names | APPLIED | Layer 2 diagram | `CRSQL` removed. Other diagrams belong to other audit parts. See P5 for one similar label. |
| S5 Proposed split | NOT APPLICABLE | n/a | A later task (change.md task list, `docs/README.md` says the split is in progress). Not part of this fix. |

## 3. Misses

None. Every item is applied, overtaken by a recorded decision that the doc
follows, or outside the architecture doc (the split).

## 4. Other problems noticed

- **P1. Repeated sentence.** Layer 2 > Sandboxes > Podman bullet says "The substrate calls the host's `podman` command" twice in the same bullet. Delete the second one: keep "So a container is rootless only when Podman on the host is set up that way."
- **P2. Peer backup pools.** `docs/system-requirements-spec.md:280-282` says "peer backup pools are optional and must not be required for portability". The architecture doc no longer mentions a backup pool at all (L2-34 dropped). The requirements round is planned for later, so no edit now. Check that the requirements round either keeps the sentence and adds one Envisioned line to the architecture, or drops it from the requirements.
- **P3. Same text twice.** The Storage > Envisioned block repeats the disconnected-client paragraph that Multi-Device A already says (queue locally, replay, idempotency keys). Two copies can drift apart. Keep one, and link to it from the other.
- **P4. Reader lines.** Only Layer 1 (`*Reader: a developer or operator ...*`) and Layer 2 have one. AGENTS.md asks for a named reader at the top of each doc. This is fixed by the split, but until then Layer 3 and the later sections have none.
- **P5. "Offline Outbox Queue" label.** The Layer 3 > Messaging diagram (`STORAGE_MSG`, node `Q2`) still says "Offline Outbox Queue". This is the same wording the audit flagged in L2-14. The text below the diagram says the message stays in the outbox of the sender. Rename the node "Durable outbox" so it does not look like a device queue.
- **P6. Backlog.** `docs/planning/deferred-backlog.md` has no row for the Layer 2 Envisioned items that are now stated plainly (generic app export, resource-class scheduling, secondary-device sync, OpenRPC schema). They are old unbuilt designs, not new deferrals from this change, so this is optional. The wRPC item is already there (row about wRPC).

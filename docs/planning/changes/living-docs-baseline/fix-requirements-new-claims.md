# New statements and code evidence

Reader: the coordinator, any sub-agent adding statements, and any independent checker.

Every new or changed statement that describes built functionality in `docs/system-requirements-spec.md` or `docs/planning/traceability-matrix.md` must be recorded here with its verified code evidence (`file:line`).

| Commit | Heading | Statement | Evidence |
| --- | --- | --- | --- |
| 1 | `### [APP-A11Y] Accessibility and Localisation` | ProfilePayload supports an optional locale field (Option<String>), with default en-US. | `crates/roym_core/src/person.rs:26`, `crates/roym_profile/src/app/profile_ops.rs:99` |
| 1 | `### [APP-ESC] Escrow, System Coins, and Mutual Credit` | Payments are recorded through signed out-of-band payment requests and acknowledgements. | `crates/roym_core/src/payment.rs:16` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Substrate Core establishes secure P2P connections across NATs and resolves cryptographic node IDs. | `crates/router/src/net_iroh.rs:94`, `crates/coordinator_iroh/src/coordinator.rs:40` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym Conversation exercises outbox message queuing and causal DAG syncing upon reconnection. | `crates/conversation/src/outbox.rs:20`, `crates/substrate/tests/roym_group_offline_e2e.rs:21` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym Conversation exercises end-to-end encrypted messaging with Double Ratchet and causal DAG ordering via syneroym:conversation. | `crates/conversation/src/dag.rs:15`, `crates/wit_interfaces/wit/conversation/conversation.wit:1` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Substrate event notification bridges exercise event delivery through an embedded MQTT broker. | `crates/mqtt_broker/src/lib.rs:20`, `crates/data_db/src/pubsub.rs:40` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym Catalog imports blob-store WIT capability to store and retrieve content-addressed blobs. | `crates/roym_catalog/wit/world.wit:10`, `crates/data_blob/src/lib.rs:25` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym Hub and roymctl generate root keypairs and enforce authorization via ControllerAgreement, App Supervisors, and UCAN or FDAE policies. | `crates/identity/src/substrate.rs:48`, `crates/fdae/src/lib.rs:20` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym services retrieve configuration secrets from the encrypted vault via syneroym:vault/reveal. | `crates/sandbox_wasm/src/host_capabilities/capabilities_services.rs:20`, `crates/data_db/src/sqlite/service_store.rs:80` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | roymctl compiles and deploys WASM SynApp components into the sandboxed Wasmtime runtime. | `apps/roymctl/src/commands/app/deploy.rs:20`, `crates/substrate/tests/roymctl_app_e2e.rs:15` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Substrate runtime executes stateful init() and migrate() SQL DDL lifecycle hooks. | `crates/sandbox_wasm/src/engine/lifecycle.rs:110` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym Directory publishes signed listing records and resolves service providers via directory query and deterministic client-side merging. | `crates/roym_directory/src/app/client_query.rs:20`, `crates/roym_directory/src/app/client_merge.rs:15` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Roym Transaction generates bilateral agreement and fulfilment receipts and Roym Hub displays credential trust summaries. | `crates/roym_core/src/transaction.rs:192`, `crates/roym_web/ui/src/cards/templates/agreement_receipt.ts:15` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Substrate Core records operational metrics in an in-memory recorder exposed over HTTP /metrics. | `crates/observability/src/recorder.rs:20` |
| 1 | `## Appendix: Substrate Feature Coverage Matrix` | Control plane executes deploy-time rollback of configuration generations, asset bundles, and FDAE policies upon deployment failure. | `crates/control_plane/src/service/orchestration/backends.rs:151` |

# Deferred Backlog

**Status:** Living document. **Last consolidated:** 2026-07-24.

This is the single running list of everything we have consciously **postponed, shortcut, or scoped out** during implementation — pulled together from the places that previously tracked deferrals in isolation:

- `system-requirements-spec.md` → [Appendix: Later-Phase Additions](../system-requirements-spec.md#appendix-later-phase-additions)
- `meta-implementation-plan.md` → [Later-Phase Additions](./meta-implementation-plan.md#later-phase-additions)
- Architecture Decision Records under [`docs/decisions/`](../decisions/)
- Per-slice "Out of scope" / "Deferred, recorded not dropped" sections in [`docs/planning/milestones/`](./milestones/)
- The [traceability matrix](./traceability-matrix.md) deferral rows
- In-code `TODO` / `FIXME` markers

Those sections remain the **authoritative source of record** for each item's rationale — this doc is the index that lets you see the whole backlog at once and find the source. When an item here disagrees with its source doc, the source doc wins; fix this doc.

## How to maintain this doc

**This is a running doc.** Whenever, in the course of implementing something, we **drop a feature, take a shortcut, gate something coarsely as a stand-in, or decide "later,"** add it here — as a final sanity step of the change, the same way we clean up imports and confirm the workspace builds. See the check in [`AGENTS.md`](../../AGENTS.md#ai-agent-guidelines).

- **Adding:** put the item under the right theme, state what's deferred and *why*, name a target milestone/phase if one exists (else `TBD`), and link the source of record (ADR, plan doc, or `file.rs:line`).
- **Landing (un-deferring):** when a deferred item ships, strike it or move it to [§ Recently resolved](#recently-resolved) with the commit/PR, and remove the corresponding in-code `TODO` marker.
- **Code markers:** every open `TODO(...)`/`FIXME` that represents a real deferral should have a home in [§ Open in-code markers](#open-in-code-markers). Markers that get resolved must be deleted from the code *and* removed here.

Target notation: `M05`, `M07`, `M10+`, `Phase 6` = a sequenced milestone/phase; `TBD` = deferred with no committed target; `blocks-prod` = a gap that blocks a production/multi-tenant posture and should not be forgotten.

---

## 1. Protocol & transport

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| wRPC binary wire protocol | Native `wrpc://` / `http-wrpc://` / `binary-wrpc://` schemes are reserved but not implemented; JSON-RPC 2.0 is the actual wire everywhere. | M05+ | [ADR-0014](../decisions/0014-quic-stream-protocol-routing.md); [system-architecture.md](../system-architecture.md) §wRPC; `router/src/preamble.rs`, `router/src/lib.rs` |
| Protocol-version negotiation `[LFC-VER]` | Handshake protocol-version negotiation deferred together with the wRPC wire. | M05+ | [traceability-matrix.md](./traceability-matrix.md); [M04A A1 plan](./milestones/M04A-proxy-and-auth-foundation/plans/A1.md) |
| Data Pipeline Streams `[PLT-DAP-05]` (QUIC point-to-point) | Moved wholesale to M5; WASI async streams not yet stable across runtimes. | M05 | [traceability-matrix.md](./traceability-matrix.md); [ADR-0010](../decisions/0010-mqtt-broker-rumqttd.md) |

## 2. Data layer

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| DuckDB / OLAP engine | DuckDB via the SQLite-scanner extension in the `olap` profile explicitly deferred. | Future phase | [system-architecture.md](../system-architecture.md); [system-requirements-spec.md](../system-requirements-spec.md); [M03 task](./milestones/M03-sss/task.md) |
| Init-defined logical views (F1/D3) | The `AggregationPipeline` shipped (B4); init-defined logical views on top of it did not. | TBD | [ADR-0007](../decisions/0007-data-layer-wit-interface.md); [M04A B4 plan](./milestones/M04A-proxy-and-auth-foundation/plans/B4.md) |
| Full MongoDB operator compatibility | Explicitly out of scope — only the supported operator subset is implemented. | Out of scope | [ADR-0007](../decisions/0007-data-layer-wit-interface.md) |
| Regex support in query layer | Deferred. | TBD | [ADR-0007](../decisions/0007-data-layer-wit-interface.md) |
| Logical data-service routing / replication | Routing a logical data service across nodes deferred to M5; log-replication overlays to M7. | M05 / M07 | [traceability-matrix.md](./traceability-matrix.md); [M03 task](./milestones/M03-sss/task.md) |
| WASM `migrate()` snapshot/rollback safety net | Migration path exists without a full snapshot/rollback safety net. | M05 | [ADR-0007](../decisions/0007-data-layer-wit-interface.md); `sandbox_wasm/src/engine.rs` |

## 3. Access control, identity & security

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Per-instance KEK "Model B" (IAM-gated provisioning) | Per-SynApp-instance/per-service KEK provisioning deferred, **not dropped**. Blocks multi-tenant production. | blocks-prod | [ADR-0006](../decisions/0006-sqlite-encryption-sqlcipher.md); [traceability-matrix.md](./traceability-matrix.md); [M04A B6 plan](./milestones/M04A-proxy-and-auth-foundation/plans/B6.md) |
| FDAE stage-4 WASM ABAC | Field redaction, budget overruns, widen/reject at the WASM ABAC stage not yet implemented. | M04B follow-on | [M04B task](./milestones/M04B-fdae-policy/task.md); [ADR-0017](../decisions/0017-fdae-policy-schema-and-compilation.md) |
| FDAE write-side Mode-A authorization | Write-path authorization not yet delivered. | M04B follow-on | [traceability-matrix.md](./traceability-matrix.md) |
| Federated cross-service parameter fetch | Cross-service relation resolution / stage-2 fetch deferred; seams kept open. | M04B follow-on | [M04B slice-b2 plan](./milestones/M04B-fdae-policy/slice-b2-implementation-plan.md) |
| Fine-grained caller authorization on routes | Current gate only proves *an* identity is present, not *which* callers may reach a route; coarse fail-closed stand-in in the proxy. | M04B follow-on | `router/src/route_handler/io.rs`, `router/src/proxy.rs` |
| `anchor_did` path-list binding | No near-term consumer; recorded and deferred beyond B3. | TBD | [access-control-design.md](./access-control-design.md); [ADR-0015 §A8](../decisions/0015-ucan-capability-model.md) |
| Pairwise DIDs | Costs FDAE dearly with no M4 consumer. | TBD | [access-control-design.md](./access-control-design.md); [ADR-0015 §A8](../decisions/0015-ucan-capability-model.md) |
| Stale-relationship-data handling; SCP-style node ceilings | Explicitly deferred ("do not build" for node ceilings). | TBD | [access-control-design.md](./access-control-design.md) |
| Multiple substrate owners (F12) + `ControllerAgreement` creation tool | Multi-owner representation and the agreement-creation tool spun out. | M05 | [M04A B7 plan](./milestones/M04A-proxy-and-auth-foundation/plans/B7.md); [traceability-matrix.md](./traceability-matrix.md) |
| Hardware attestation; supply-chain binary signing | Deferred to M7. | M07 | [M02 task](./milestones/M02-reliable-node/task.md) |
| Master key export/recovery; Tier-1 fallback; ZK-proof plugin `[FND-IDT]` | Deferred to the later-phase additions. | M10+ | [meta-implementation-plan.md](./meta-implementation-plan.md#later-phase-additions) |
| W3C Verifiable Credential compatibility | Delegation-cert format does not target VC data model yet. | TBD | [ADR-0001](../decisions/0001-delegation-certificate-format.md) |
| Auto-unseal (e.g. AWS KMS) | Out of scope for the substrate. | Out of scope | [ADR-0006](../decisions/0006-sqlite-encryption-sqlcipher.md) |

## 4. Storage / blob

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Public (unsigned) blob HTTP serving | Only signed-URL HMAC serving exists; public unsigned serving deferred. | TBD | [ADR-0009](../decisions/0009-blob-storage-object-store.md); `router/src/route_handler/http.rs` |
| `blob(list<u8>)` WIT arm | A possible future WIT extension, out of scope. | Out of scope | [M04A B5 plan](./milestones/M04A-proxy-and-auth-foundation/plans/B5.md) |

## 5. Messaging

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| MQTT shared subscriptions (`$share/<group>/<filter>`) | Small `namespace_topic` fix in `crates/mqtt_broker`; `rumqttd 0.20` already supports it. Doesn't block later milestones. | post-M3B, low pri | [meta-implementation-plan.md](./meta-implementation-plan.md#later-phase-additions); [M03B task](./milestones/M03B-messaging/task.md) |
| WebSocket upgrade for messaging | Out of scope for the messaging slice; revisit as follow-up. | TBD | [M03B task](./milestones/M03B-messaging/task.md) |
| Decentralized messaging overlay | Pushed to M7. | M07 | [M03B task](./milestones/M03B-messaging/task.md); commit `c619b71` |
| Raw-timestamp plausibility bounds | Left open for a follow-up decision. | TBD | [ADR-0013](../decisions/0013-p2p-messaging-architecture.md) |

## 6. Payments & economics

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Escrow / dispute-mediated custody | Not part of MVP payment surface; needs separate spec + legal review. | M11+ / Phase 6 | [system-requirements-spec.md](../system-requirements-spec.md#appendix-later-phase-additions); [m0-status](./milestones/m0-status.md) |
| System coins & mutual credit (DLN) | Native ledger token + bilateral IOU layered on the Payment Abstraction Layer; needs legal review + DLN scoping. | M11+ / Phase 6 | [system-requirements-spec.md](../system-requirements-spec.md#appendix-later-phase-additions); [meta-implementation-plan.md](./meta-implementation-plan.md#later-phase-additions) |

## 7. Gateway & networking

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Client-gateway remote-access security | Currently local-machine-only as a stopgap; proper remote-access auth not designed. | TBD | `client_gateway/src/gateway.rs` |
| Gateway caller = substrate-owner DID threading | Present the controller DID as caller identity through the gateway. | post-B0 | `client_gateway/src/gateway.rs` |
| Non-IP mesh transport (Zigbee / Thread / BLE) | Integrating non-IP mesh into the IP topology. | Later phase | [system-requirements-spec.md](../system-requirements-spec.md#appendix-later-phase-additions) |

## 8. Node lifecycle & ops

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Structured retry policy on `SubstrateConfig` | Config does not yet expose a structured retry policy. | TBD | [ADR-0003](../decisions/0003-retry-policy-ownership.md); [M02 task](./milestones/M02-reliable-node/task.md) |
| Resource quotas on `ServiceManifest` | Manifest types have no resource-quota fields yet. | TBD | [M02 task](./milestones/M02-reliable-node/task.md); [ADR-0019](../decisions/0019-deploy-time-artifact-delivery.md) |
| WASM fuel metering | Not yet configured. | TBD | [ADR-0005](../decisions/0005-wasm-fuel-quota-schema.md); [M02 task](./milestones/M02-reliable-node/task.md) |
| Full admin surface | Deferred across M5/M7. | M05 / M07 | [M02 task](./milestones/M02-reliable-node/task.md) |
| Raspberry Pi 4 DB-open perf budget | Perf figure outstanding, tracked with Model B. | blocks-prod | [ADR-0006](../decisions/0006-sqlite-encryption-sqlcipher.md); [M04A B6 plan](./milestones/M04A-proxy-and-auth-foundation/plans/B6.md) |

## 9. Observability

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Observability engine build-out | Current engine is a placeholder / basic shell. | TBD | `observability/src/engine.rs` |
| Process-global tracing init | Handle process-global tracing initialization more cleanly. | TBD | `observability/src/engine.rs` |
| Resource metering `[FND-OBS]` | Exact per-service utilization metering (for billing) not built. | TBD | [system-requirements-spec.md](../system-requirements-spec.md) feature matrix |

## 10. Product surfaces & UX

| Item | What's deferred / why | Target | Source of record |
|---|---|---|---|
| Consent / moderation UX `[PRD-SAF]` | Retargeted to a consumer-facing surface (Chat/Hub) that doesn't exist yet. | M06+ | [traceability-matrix.md](./traceability-matrix.md) |
| Syneroym Hub UI, marketplace/aggregator/facilitator `SynSvcs`, Producer-Distributor mesh | Phase 6 product expansion. | Phase 6 | [meta-implementation-plan.md](./meta-implementation-plan.md#later-phase-additions) |
| Accessibility (WCAG 2.1 AA) & localisation/i18n | Architecture to stay i18n-ready; full support later. | Later phase | [system-requirements-spec.md](../system-requirements-spec.md#appendix-later-phase-additions) |
| Mobile secure-hardware bindings (StrongBox / Secure Enclave), mobile lifecycle | Part of the mobile milestone. | Mobile milestone | [meta-implementation-plan.md](./meta-implementation-plan.md) |

## 11. Open in-code markers

Live `TODO`/`FIXME` markers that encode a real deferral. Remove both the code marker **and** its row here when resolved. (Excludes markers already resolved in-tree, e.g. the former `TODO(M4)` init-context gates removed in B0 and the `session.rs` `TODO(B7)` resolved in B7b.)

| Location | Marker | Maps to |
|---|---|---|
| `crates/app_orchestration/src/compiler.rs:163` | Temporary M1 hack force-prepending in the deployment-plan compiler | §8 |
| `crates/client_gateway/src/gateway.rs:34` | Present substrate-owner (controller) DID as caller | §7 |
| `crates/client_gateway/src/gateway.rs:108` | Local-machine-only security stopgap | §7 |
| `crates/observability/src/engine.rs:6` | Engine is a placeholder/basic shell | §9 |
| `crates/observability/src/engine.rs:52,70` | Cleaner process-global tracing init | §9 |
| `crates/sdk/src/lib.rs:60` | Self-asserted pubkey is an assertion, not FDAE-backed proof | §3 |
| `crates/control_plane/src/service.rs:171` | Security ops (KEK/secret) authorization pending grant layer | §3 |
| `crates/ucan/src/capability.rs:392` | Capability passthrough semantics documented but not implemented | §3 |
| `crates/router/src/proxy.rs:192` | Coarse interim fail-closed dispatch gate (stands in for FDAE) | §3 |
| `crates/router/src/route_handler/io.rs:147`, `route_handler/dispatch.rs:84` | Fine-grained caller authorization | §3 |
| `crates/router/src/route_handler/http.rs:930` | Blob GET authorization is HMAC-only pending FDAE | §3 |
| `crates/router/src/lib.rs:4`, `preamble.rs:19,28,29,140`, `route_handler.rs:4` | wRPC wire formats not implemented | §1 |
| `crates/sandbox_wasm/src/engine.rs:533` | `migrate()` snapshot/rollback safety net | §2 |
| `crates/sandbox_wasm/src/engine.rs:599` | Cache function-parameter details (perf) | §2 |
| `apps/roymctl/tests/cli_args.rs:10` | Expand CLI argument-parsing tests | (test coverage) |
| `conversions.rs` (data layer) | Positional→named param binding | §2 |

---

## Recently resolved

Move items here (with commit/PR) when they land, then prune periodically.

| Item | Resolved by |
|---|---|
| `anchor_did` anchor stamp (ADR-0015 A5) | `5a6f047`, remote fetch `279d284` (#89) |
| `TODO(M4)` init-context DDL gates → `data-layer/admin` capability gate | Slice B0 |
| `session.rs TODO(B7)` facts gate | Slice B7b (ADR-0015 A6) |

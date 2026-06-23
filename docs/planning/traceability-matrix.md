# Traceability Matrix

This document maps the requirements from the System Requirements Specification (`system-requirements-spec.md`) and related documents to their target milestones, current implementation status, and acceptance evidence.

## Core Requirements

| Requirement ID | Description | Target Milestone | Status | Acceptance Evidence |
| --- | --- | --- | --- | --- |
| `[PRD-AUT]` | Provider identity, data, policy, and operator choice remain under provider control. | TBD | Pending | Delegation-revocation and operator-migration journeys. |
| `[PRD-CUX]` | Consumers complete the reference journey without understanding hosting or federation. | M6 | Pending | Moderated task-success test and accessibility audit. |
| `[PRD-FED]` | Independent implementations interoperate without a mandatory central data-plane or authority. | M8 | Pending | Two-node federation and bootstrap-outage tests. |
| `[PRD-OFF]` | Safe workflows remain intelligible and converge after disconnection; unsafe retries fail explicitly. | M5 | Pending | Fault-injection, idempotency, and state-model tests. |
| `[PRD-POR]` | Participants can export, verify, and restore in-scope identity-linked data through versioned open formats. | TBD | Pending | Clean-node export/import drill and cross-version fixtures. |
| `[PRD-TRU]` | Trust evidence is sourced, scoped, fresh, explainable, and correctable; uncertainty remains visible. | M8 | Pending | Trust-display, revocation, omission, and abuse cases. |
| `[PRD-OPS]` | A non-specialist can install or join, understand health, recover, update, and exit within the declared operating profile. | M2 | Pending | Timed onboarding and incident-recovery exercises. |
| `[PRD-EXT]` | Third-party SynApps can declare capabilities and pass public compatibility tests. | M5 | Pending | Package inspection and protocol conformance suite. |
| `[PRD-SAF]` | Consent, data lifecycle, moderation boundaries, and responsible parties are explicit throughout a transaction. | M4 | Pending | Policy-version, grant, report, dispute, and deletion scenarios. |

## Substrate Capabilities

| Requirement ID | Description | Target Milestone | Status | Acceptance Evidence |
| --- | --- | --- | --- | --- |
| `[TOP-PRM]` | Core Primitives & Overlay | M1 | Pending | TBD |
| `[TOP-ADR]` | Service Addressing | M1 | Pending | TBD |
| `[TOP-REG]` | Registries (App & Endpoint) | M1 | Pending | TBD |
| `[TOP-DSC]` | Discovery Mechanisms | M1 | Pending | TBD |

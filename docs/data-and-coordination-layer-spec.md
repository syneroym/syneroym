# Distributed Application Framework – Data & Coordination Layer

## 1. Core Services Overview

The Data & Coordination layer provides a complete foundation for distributed applications, focusing on simplicity, correctness, and a manageable operational surface area. It is composed of four core services:

1. **REST Data Service**: Structured data and metadata.
2. **Object Service**: S3-compatible object storage with built-in HTTP serving.
3. **MQTT Event Service**: Asynchronous communication and state propagation.
4. **Registry Service**: Authoritative control plane for cluster coordination.

---

## 2. Service Definitions

### 2.1. REST Data Service
Manages structured data and metadata.

* **Capabilities:**
  * CRUD (Create, Read, Update, Delete)
  * Filtering and querying
  * Pagination and sorting
  * Batch operations
  * Transactions (optional)
  * File/object metadata
* **Examples:**
  * `GET /users/123`
  * `POST /orders`
  * `POST /users:batchUpsert`
  * `GET /files?tag=invoice`

### 2.2. Object Service
S3-compatible object storage with built-in HTTP serving. Note that object metadata remains in the REST service.

* **Capabilities:**
  * S3-compatible API
  * File/blob storage
  * HTTP file serving
  * Static website hosting
  * Public/private objects
  * Signed URLs
  * Range requests
  * CDN-friendly URLs
* **Examples:**
  * `s3://assets/logo.png` is automatically available as `https://files.example.com/assets/logo.png`
* **Use Cases:**
  * Images, Videos, Documents
  * Software artifacts
  * Static websites
  * Backups

### 2.3. MQTT Event Service
Handles asynchronous communication and state propagation.

* **Capabilities:**
  * Pub/Sub
  * Wildcard topics
  * Retained messages
  * Change notifications
  * Workflow triggers
* **Examples:**
  * `users.created`
  * `orders.updated`
  * `files.uploaded`
* **Use Cases:**
  * Realtime updates
  * Device communication
  * Event-driven workflows
  * Cache invalidation

### 2.4. Registry Service
The authoritative control plane for cluster coordination.

* **Responsibilities:**
  * Membership
  * Ownership
  * Service discovery
  * Node status
  * Promotion workflows

---

## 3. Registry & Coordination Model

### 3.1. Node States
* **ACTIVE**: Normal operation.
* **SUSPECT**: Communication problems observed.
* **QUARANTINED**: No new work assigned. No ownership claims accepted. No application traffic routed. Management traffic allowed.
* **RETIRED**: Permanently removed.

### 3.2. Ownership Model
Every owned resource has an assignment in the registry. This provides fencing without leases.
```json
{
  "resource": "orders-shard-1",
  "owner": "nodeA",
  "epoch": 42
}
```
* **Rules:**
  * Exactly one owner.
  * Ownership changes occur **only** through the registry.
  * Epoch increments on every ownership transfer.
  * Epoch accompanies ownership-sensitive operations.
  * Stale epochs are rejected.

### 3.3. Failure Philosophy
The framework does not attempt to answer: *Is nodeA dead?*
Instead, it answers: *Is nodeA authorized?*
**Authority is determined solely by registry state.**

#### Quarantine Workflow
`ACTIVE` → `SUSPECT` → `QUARANTINED`
Quarantine is a registry decision, not a local decision. All nodes converge on the same view through registry updates.

#### Promotion Workflow
When an owner becomes unavailable:
1. Mark owner `QUARANTINED`.
2. Propagate registry update.
3. Wait for acknowledgement from all remaining `ACTIVE` nodes.
4. Promote replacement owner.
5. Increment epoch.

*Policy: Consistency > Availability.* The system may temporarily stall rather than risk split-brain.

### 3.4. Registry Failure Model
The Registry is the authoritative control plane. If the registry becomes unavailable, the system splits its behavior:

* **Control Plane = Frozen**
  * Unavailable operations: Ownership transfers, promotions, membership changes, quarantines, topology changes.
* **Data Plane = Continues**
  * Existing shard ownership, routing, MQTT flows, and HTTP/object access continue functioning using cached registry state.

#### Registry Recovery
There is no automatic registry failover. The recovery workflow relies on out-of-band verification:
1. Registry unreachable.
2. Freeze control plane.
3. Out-of-band verification (Physical access, Hypervisor access, Cloud control plane, Console access, etc.).
4. Confirm old registry unavailable.
5. Quarantine old registry.
6. Assign logical registry address.
7. Promote replacement registry.
8. Increment registry epoch.

---

## 4. Architectural Principles

* **Simplicity**: No application-visible leader election, lease management, or distributed lock service.
* **Safety**: Use **Ownership + Registry State + Epoch Fencing** instead of liveness assumptions.
* **Consistency First**: When uncertainty exists: Freeze → Quarantine → Achieve agreement → Promote → Resume.

## 5. Platform Surface Area Summary

* **REST Data Service** → Structured data & metadata
* **Object Service** → S3 + HTTP file serving
* **MQTT Event Service** → Pub/Sub & state propagation
* **Registry Service** → Membership, ownership, coordination


# ADR 0031: Node-local wake of a service

Reader: developers building services on Syneroym, and operators running substrates.

## Status
Accepted

## Related
- ADR-0023 (durable async primitives, supervisor schedules and long tasks)
- ADR-0027 (host event feed and the event window)
- ADR-0016 (caller identity on native dispatch)

## Context
A service often needs to do work on a timer or when new data arrives. For example, a deal service needs to advance agreements, expire old quotes, and process incoming cards without a user opening an app screen.

Before this decision, a guest service ran code only when an external caller made a request, or through the supervisor's cluster schedule (ADR-0023 §6). However, the supervisor schedule model does not fit node-local maintenance:
1. Many deployments and embedded apps run with no cluster supervisor.
2. A cluster schedule picks one node in the cluster to run a tick. But node-local data (such as local conversations and encrypted SQLite rows) exists on every member's own node. Each node must maintain its own data.
3. The supervisor schedule interval has a coarse 30-second pass. Deals and messaging need reactions within a second when new cards arrive.
4. Guest components do not hold timers and cannot sleep across calls. Guest execution is bounded by an epoch timeout (typically 5 seconds).

We need a way for the hosting substrate to wake a local service safely, on events and on a regular timer.

## Decision

1. **Manifest declaration.** An app service may declare at most one `wake` block in its manifest:

   ```toml
   [services.<name>.wake]
   interface = "syneroym-roym:transaction/api@0.1.0"
   method = "invoke"
   params = '[ "{\"method\":\"transaction.drive\",\"params\":{}}" ]'
   on_events = ["conversation.message"]
   every_secs = 30
   timeout_ms = 10000
   ```

   Validation limits:
   - `interface` must be one of the service's declared interfaces.
   - `method` must not be empty.
   - `params`, if set, must be valid JSON.
   - `on_events` holds at most 8 event kinds. Each kind follows the event kind grammar (lowercase letters, digits, `.` and `-`, at most 64 bytes).
   - `every_secs` is an integer between 5 and 86 400 seconds.
   - `timeout_ms` is an integer between 1 and 30 000 milliseconds (default 10 000 ms).

2. **Node-local scope.** A wake is strictly node-local. Every member of a service runs the wake on its own hosting node for its own local data. It is never a cluster-wide task.

3. **Strictly local dispatch.** The substrate checks its local endpoint registry before invoking a wake target. If the local endpoint is missing (for example, while a service is stopping or redeploying), the substrate logs a warning and skips the call. A wake never resolves through the remote community registry and never makes an outbound network call.

4. **Background worker and polling tick.** The substrate hosts a background worker (`WakeWorker`) with a 1-second loop tick. The worker checks targets every second:
   - If a target is already running from an earlier wake, it is skipped.
   - If matching events arrived on the app's event bus since the last cursor, the wake triggers.
   - If the event bus signals `reload` (events were lost or the node restarted), the wake triggers.
   - If the time since the last run reaches `every_secs`, the wake triggers.
   If the substrate runs in coordinator mode with no proxy router, worker spawning is skipped. If a standalone service outside any app declares `on_events`, those events are ignored and a warning is logged once.

5. **Concurrency and safety.**
   - At most one wake call runs at a time for any given service.
   - Calls are never queued. If an event arrives while a call is running, the next loop tick after completion handles it.
   - The running state is protected by an RAII guard. If a call panics or cancels, the running flag resets cleanly to false.

6. **The wake reply contract.** A wake method returns a standard JSON-RPC result. The substrate worker reads two optional fields from this result:
   - `again: bool`: when true, the worker wakes the service again on the next 1-second loop tick instead of waiting for `every_secs`. A service sets this only when it processed a full batch and still has work pending.
   - `warnings: list<string>`: human-readable warning strings that the substrate logs with `tracing::warn!`. This gives the guest a safe way to report issues to node operators.

7. **Bounded ordinary call.** A wake call is an ordinary guest invocation. It is bounded by the host epoch timer and `timeout_ms`. It is not a long-running background task (ADR-0023 §7).

8. **No automatic retry on failure.** If a wake call returns an error, the substrate logs the error. It does not retry immediately. The next scheduled interval or the next incoming event serves as the retry.

## Consequences
- A service can advance deals, retry releases, and clear expired records in the background with no client session open.
- Background execution does not overload the node: execution is bounded to at most one short call per second per service.
- The mechanism works identically in both native substrate builds and WebAssembly sandboxes.
- A service with a backlog of work can drain it quickly (one batch per second) by returning `again: true`, and returns to low-frequency polling when idle.

## Alternatives considered
- **Cluster schedules in the app supervisor.** Rejected: the supervisor is absent in standalone and test deployments. Its 30-second interval is too slow for interactive chat, and cluster scheduling runs on only one node instead of every member.
- **Hard-coded substrate thread per service.** Rejected: coupling specific service method calls directly into the substrate binary prevents generic app deployment and breaks WebAssembly sandbox isolation.
- **Client-only drivers.** Rejected: agreements would only advance when a user views the screen. Unattended nodes and closed mobile apps would leave deals stalled.
- **Push events into guest components.** Postponed: requires bidirectional callback interfaces and complex lifecycle handling in guest WebAssembly components. Pull-triggered wakes achieve the same result with simpler code.

# Performance Summary

This file is automatically updated by `cargo xtask perf-summary`.

## Run: 2026-07-26 09:03:04 (70f00c9) -- targeted, `cargo bench --bench abac_bench` only

Manually recorded (not a full `cargo xtask perf-summary` sweep) to close
Slice B4-fdae review finding B4-15: `task.md`'s Performance Budgets row 3
and Slice B4-fdae's own plan (§4) ask for the stage-4 ABAC after-step
measurement here. Row-count sweep at a fixed ~28-byte payload; payload-size
sweep at a fixed 100-row batch, added per review finding B4-02 (payload
bytes, not row count, drive the `Val::List(Val::U8...)` marshalling cost).

| Benchmark | Mean Time |
|-----------|-----------|
| abac_after_step_0_rows | 34.5 µs |
| abac_after_step_1_rows | 34.9 µs |
| abac_after_step_10_rows | 45.0 µs |
| abac_after_step_100_rows (~28 B/row) | 142.2 µs |
| abac_after_step_1000_rows (~28 B/row, `MAX_ABAC_ROWS`) | 1.102 ms |
| abac_after_step_100_rows_at_1kb | 1.494 ms |
| abac_after_step_100_rows_at_16kb | 22.14 ms |

The instantiation floor (0 rows) dominates at realistic page sizes, matching
Slice B3 Phase 5's federated-fetch finding -- but payload size, not row
count, is what actually moves the number: 100 rows at 16 KB/row (1.6 MB
total) is ~156x the ~28-byte-row figure at the same row count, confirming
B4-02's concern and motivating `MAX_ABAC_PAYLOAD_BYTES` (16 MiB/batch).

## Run: 2026-06-04 08:47:03 (285d29d)

### Environment
| Commit | Timestamp | OS | CPU | Memory |
|--------|-----------|----|-----|--------|
| 285d29d | 2026-06-04 08:47:03 | MacOS 26.5.1  | Apple M3 | 24.0 GB |

### Criterion Micro-Benchmarks
| Benchmark | Mean Time (ms) |
|-----------|----------------|
| encrypt | 0.01 ms |
| decrypt | 0.01 ms |
| write_frame_10kb | 0.00 ms |
| write_frame_100b | 0.00 ms |
| read_frame_100b | 0.00 ms |
| read_frame_10kb | 0.00 ms |
| binary_json_rpc | 0.00 ms |
| composable | 0.00 ms |
| http_json_rpc | 0.00 ms |
| encrypted_query_params | 0.00 ms |
| json_to_wasm_params | 0.00 ms |
| ecdh_p256_server_handshake | 0.22 ms |
| wasm_cached_instantiation | 0.02 ms |
| encrypt | 0.41 ms |
| decrypt | 0.42 ms |
| wasm_store_creation | 0.00 ms |

### Syneroym Perf: Latency
| Scenario | p50 | p95 |
|----------|-----|-----|
| TCP Proxy (HTTP GET /) | 0.22 ms | 0.35 ms |
| WASM Component (Execution) | 0.19 ms | 0.25 ms |

### Syneroym Perf: Soak
| Duration | Throughput (rps) | Peak RSS | Result |
|----------|------------------|----------|--------|
| 1800s | 10.0 | 84.1 MB | ✅ PASS |

# Performance Summary

This file is automatically updated by `cargo xtask perf-summary`.

## Run: 2026-07-26 -- targeted, `cargo bench -p syneroym-data-db --bench fdae_bench -- fdae_authorized_write`

Manually recorded (not a full `cargo xtask perf-summary` sweep) for Slice
B5-fdae's write-side Mode-A authorization: `task.md`'s Performance Budgets
row for authorized single-row writes. Compares an authorized `patch`/
`batch_mutate` (paying the `USING`/`WITH CHECK` `EXISTS` checks) against the
unauthorized (`auth: None`) baseline, single-hop ReBAC, real SQLite via
`SqliteServiceStore`.

| Benchmark | Mean Time |
|-----------|-----------|
| patch_unauthorized_baseline | 20.585 µs |
| patch_authorized | 39.925 µs |
| batch_mutate_50_unauthorized_baseline | 340.46 µs |
| batch_mutate_50_authorized | 964.41 µs |

A single authorized `patch` costs ~19 µs more than the unauthorized baseline
(one pre-image + one post-image `EXISTS`, ~2x) and a 50-mutation authorized
`batch_mutate` (all of one caller's own rows) costs ~625 µs more than its
baseline (~2.8x) -- both well under 1 ms and negligible against the 25 ms
p99 Mode-B pushdown-query budget above (0.16% for the single `patch`).

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
B4-02's concern and motivating `MAX_ABAC_PAYLOAD_BYTES` (1 MiB/batch as of
review residual R4 -- an initial 16 MiB was too generous against these same
numbers: 100 rows @ 16 KB is a tenth of that cap yet already costs ~18-22
ms, implying ~180 ms/~640 MB for a batch actually at the old cap).

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

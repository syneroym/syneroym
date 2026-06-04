# Performance Summary

This file is automatically updated by `cargo xtask perf-summary`.

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

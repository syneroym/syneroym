# New statements added by the architecture fix

Reader: the checker of stage 2, who verifies each row against the code.

Every new or changed statement that says what the system does gets a row here, with `file:line` evidence.

| Commit | Heading | Statement | Evidence |
| --- | --- | --- | --- |
| 1 | Glossary > wRPC | The router reserves the `wrpc://` scheme and answers it with a typed unsupported protocol error; JSON-RPC 2.0 is the only wire protocol today. | `crates/router/src/preamble.rs:227` (`"wrpc"` parses to `RouteProtocol::Wrpc`); `crates/router/src/route_handler/dispatch.rs:288-293` (unsupported protocol error, "this node speaks json-rpc/v1") |
| 1 | Glossary > ABAC | ABAC is stage 4 of the data-access pipeline: a guest-exported `authorize-rows` function that checks candidate rows (ADR-0017 §7). | `crates/wit_interfaces/wit/data-layer/authorizer.wit:47`; `docs/decisions/0017-fdae-policy-schema-and-compilation.md` §7 heading; `crates/fdae/src/trace.rs:153` |
| 1 | Glossary > LWW | The most recent write to a record persists; `put` replaces the whole payload. | `crates/wit_interfaces/wit/data-layer/data-layer.wit:96-101` ("`put` replaces the whole payload"); `crates/roym_catalog/src/app/listing_ops.rs:232-239` (listing saved with `put`) |
| 1 | Glossary > SynApp | A SynApp is a composed set of services. | `crates/app_orchestration/src/models/manifest.rs:14-16` (`SynAppManifest`); `crates/roym_core/app/roym.toml` |
| 1 | Open Questions > OQ-1 | The DHT records are signed `pkarr` packets published to the mainline DHT; there is no libp2p dependency. | `Cargo.toml:142` (`pkarr = "6.0"`); `crates/core/src/dht_registry/client.rs:4,32-35` (`dht.publish`, "published ... to Mainline DHT"); `crates/core/src/dht_registry/master_anchor.rs:4` |
| 1 | Open Questions > OQ-2 | The master key never enters the browser; `roymctl session delegate` makes a temporary key pair and a delegation certificate; the Hub imports them once and keeps the private key in IndexedDB as a non-extractable WebCrypto key. | `crates/roym_web/ui/src/session/login.ts:1-10` (header comment), `:200-216` (signs the challenge with the stored key); `apps/roymctl/src/commands/session.rs:29,220` (`Delegate`) |
| 1 | Open Questions > OQ-5 | A SynOrg issues signed membership credentials and withdraws them with signed revocation and moderation records; these are Roym records, not W3C Verifiable Credentials. | `crates/roym_core/src/membership.rs:1-20` (credential, revocation, moderation decision versions); `crates/roym_directory/src/app/credential_ops.rs:1-10`; `docs/roym-integrated-experience-spec.md:98` (Directory is the SynOrg's service: `credential.*`, `revocation.*`); owner decision Q-A4 in `change.md` (an aggregator is a SynOrg `directory` service) |
| 1 | Open Questions > OQ-6 | A substrate republishes its endpoint records every hour; this is not an uptime proof. | `crates/core/src/dht_registry/types.rs:18-20` (`HEARTBEAT_INTERVAL_SECS = 3600`); `crates/substrate/src/runtime/publish.rs:12` |
| 1 | Open Questions > OQ-7 | The Hub, a web UI, is the one reference client. | `crates/roym_web/ui/` (web UI); no `Cargo.toml` under `crates` or `apps` mentions Tauri |

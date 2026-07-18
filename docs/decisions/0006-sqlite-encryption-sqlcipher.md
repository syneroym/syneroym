# ADR-0006: SQLite Encryption via SQLCipher (Envelope Encryption — D-03-01)

## Status

Accepted

## Context

`[FND-SEC]` requires per-service encrypted SQLite databases with envelope
encryption: Data Encryption Keys (DEKs) encrypted by a Key Encryption Key (KEK)
injected into RAM at startup. The workspace currently uses
`rusqlite = { version = "0.40", features = ["bundled"] }` (vanilla SQLite).

Three options were evaluated:

- **Option A (SQLCipher):** Mature, transparent page-level AES-256-CBC
  encryption via the `rusqlite-cipher` crate (a drop-in fork of `rusqlite` that
  bundles SQLCipher instead of vanilla SQLite). WAL mode fully supported.
- **Option B (File-level AES-GCM):** Encrypt the entire `.db` file at open/close
  time with a DEK. Requires flushing the WAL on every close; incompatible with
  WAL streaming planned for M7.
- **Option C (Custom WAL-frame VFS):** Encrypt individual WAL frames via a thin
  Rust custom VFS. Maximum control but highest engineering cost in M3.

A fourth concern is the KEK scope: substrate-global (one KEK per node) vs.
per-SynApp-Instance (one KEK per app instance).

Auto-unseal mechanisms (e.g., AWS KMS) are explicitly out of scope for the
Syneroym substrate itself. Cloud deployers can implement auto-unseal externally
via deployer scripts that call `roymctl kek inject` on startup.

## Decision

### Encryption Mechanism

**Use SQLCipher via `rusqlite-cipher`** (Option A).

- Replace `rusqlite = { ..., features = ["bundled"] }` in `Cargo.toml` with
  `rusqlite = { package = "rusqlite-cipher", version = "...", features =
  ["bundled-sqlcipher"] }` (or the equivalent crate providing SQLCipher as a
  drop-in).
- SQLCipher encrypts at the page level (AES-256-CBC by default) transparently.
  WAL mode is fully supported and will not conflict with M7 WAL frame shipping,
  which operates above the SQLCipher encryption layer.
- Option B is rejected because WAL flush-on-close is incompatible with M7.
  Option C is rejected because the engineering cost is unjustified when SQLCipher
  already solves the problem with a production-proven implementation.

### Key Derivation

- Each service's DEK is a 32-byte random key generated with `rand::random::<[u8; 32]>()`.
- The DEK is the SQLCipher key material passed to `PRAGMA key = "x'<hex>'";`.
  No password hashing (PBKDF2) is applied to the DEK itself — PBKDF2 is only
  relevant when SQLCipher is opened with a passphrase string, not a raw key.
  Using a raw hex key bypasses PBKDF2 entirely, giving fast opens with no
  security regression (the randomness of the DEK provides the required entropy).
- The DEK is encrypted at rest using AES-256-GCM with a random 12-byte nonce
  and the KEK as the encryption key. The resulting ciphertext and nonce are
  stored as a row in `substrate.db`'s `dek_store` table.

### KEK Scope

- **M3: Substrate-global KEK.** One KEK per substrate node. A single
  `roymctl kek inject` call at startup unlocks all service DEKs on that node.
- **M4: Per-SynApp-Instance KEK** (deferred). Requires M4's UCAN/IAM layer to
  enforce which authenticated caller is authorised to inject which app's KEK.
  Attempting per-app KEK in M3 without IAM enforcement would be security theatre.
  The `KeyStore` API must be designed so that the scope can be narrowed in M4
  without breaking the interface (i.e., the `inject_kek` / `load_dek` contract
  accepts a `scope` parameter or equivalent extensibility point).

> [!IMPORTANT]
> **M4 Dependency:** Per-SynApp-Instance KEK is a hard requirement that must be
> tracked as an explicit gate item in the M4 milestone plan. The M3 implementation
> must not make architectural choices that prevent this narrowing.

### Memory Protection

- The KEK in RAM is protected with `mlock` (prevent swap to disk) and
  `madvise(MADV_DONTDUMP)` (exclude from core dumps), reusing the `lock_memory`
  helper introduced in M2 (`crates/identity`).
- All key material structs derive `ZeroizeOnDrop`.

### DB Open Time Budget

- With a raw hex key (no PBKDF2), SQLCipher DB open time is not dominated by
  key derivation. The budget is therefore **not** set in M3 for this reason.
- A DB open time budget will be established in the M4 ADR for per-SynApp-Instance
  KEK, at which point the full key derivation and injection path will be measured
  on Tier 1 hardware (Raspberry Pi 4).

## Consequences

- `rusqlite` in `Cargo.toml` is replaced with `rusqlite-cipher` (bundled
  SQLCipher). This is a compile-time change only; the `rusqlite` API surface is
  identical.
- The workspace `unsafe_code = "deny"` lint applies. SQLCipher's C code is
  behind the FFI boundary inside the bundled build; no unsafe Rust is introduced
  in the wrapper layer.
- All test fixtures that open SQLite databases must pass the DEK as a raw hex
  key or use an unencrypted in-memory DB for tests that do not exercise
  encryption specifically.
- M4 must introduce per-SynApp-Instance KEK before any production multi-tenant
  deployment is considered secure.

## Amendments

**2026-07-18 (M04A Slice B6).** Per-SynApp-Instance KEK narrowing shipped as
the **derived-KEK model ("Model A")**, not the provisioning model this ADR's
"KEK Scope" section originally described — see
`docs/planning/milestones/M04A-proxy-and-auth-foundation/plans/B6.md` §2 for
the full comparison. This ADR is amended in place rather than superseded,
per the same convention ADR-0007/ADR-0011 used for their M04A amendments.

- **What shipped:** the substrate-global master KEK is still injected exactly
  once (`inject_kek`'s vestigial `_scope: Option<&str>` param, dead since M3,
  is removed rather than wired up). Each service's DEK is now wrapped by a
  **per-instance KEK derived via HKDF-SHA256(master, info =
  "syneroym:kek:v1:{service_id}")** (`derive_instance_kek`,
  `crates/data_keystore/src/key_store.rs`) instead of by the raw master
  directly. `service_id` is the derivation scope; it is already the
  app-instance id (`crates/router/src/route_handler/io.rs` records
  `app_instance_id == service_id`), so this is genuine per-app-instance
  narrowing without a new identifier or a WIT/dispatch-signature change.
- **What this buys, and what it does NOT buy:** a leaked derived key does not
  reveal the master or any sibling instance's key (cryptographic isolation,
  `cross_instance_kek_isolation` test). It does **not** buy at-rest isolation
  against a party who holds the master or reads substrate RAM — one injected
  master still derives every instance's key. That stronger property is this
  ADR's actual original M4 ask (below).
- **ADR-0006's real M4 requirement — NOT satisfied, deferred.** This ADR's
  original "KEK Scope" section asked for M4 to let the `KeyStore` API accept
  "a `scope` parameter or equivalent extensibility point" so a *distinct,
  externally-provisioned* KEK could be injected **per instance** ("Model B"),
  IAM-gated on who may inject which app's KEK. That provisioning channel and
  its authorization model do not exist yet — there is no consumer or
  management-channel surface for it in M04A, the same "no consumer, defer"
  reasoning this milestone applied elsewhere (data-pipeline streams,
  credit-based backpressure). Model B is deferred, not dropped; tracked
  durably in `docs/planning/traceability-matrix.md`'s `[FND-SEC]` row and in
  the M04A `task.md`/`status.md` B6 sections.
- **The multi-tenant caveat above stays in force, retargeted at Model B.**
  Model A is a forward-compatible down-payment (defense-in-depth), **not** a
  substitute for the provisioning requirement. Do not read this amendment, or
  B6's completion, as "M4 done / multi-tenant-at-rest safe" — that gate
  remains shut until Model B ships.
- **DB Open Time Budget outcome:** measured on the dev host (not Tier-1
  Raspberry Pi 4 hardware — that figure remains outstanding, consistent with
  how M03 recorded its own deferred Pi-4 item), via `criterion`
  (`crates/data_db/benches/security_config_bench.rs`,
  `service_db_open_per_instance_kek` group): `open_service_db` end-to-end
  (HKDF derive + AES-GCM DEK unwrap/wrap + SQLCipher `PRAGMA key` open) is
  ~705 µs on a first open (DEK generated) and ~96 µs on a warm re-open (DEK
  loaded); the HKDF derivation itself, measured in isolation
  (`hkdf_derive_instance_kek`), is ~2.3 µs — a small fraction of either
  figure, confirming this section's original prediction that raw-key
  SQLCipher open time is not dominated by key derivation. Full numbers in
  `status.md`'s B6 section.

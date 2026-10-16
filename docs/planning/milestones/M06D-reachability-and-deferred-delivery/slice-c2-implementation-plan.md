# M06D slice C2 — Sealed message and receipt formats: implementation plan

*Reader: the developer (or AI) who implements slice C2. You do not have the
planning conversation. This plan is meant to stand alone.*

## 0. Read first, and the rules

Read these before you change anything:

- `AGENTS.md`. All rules apply. The ones this slice touches most:
  `mise run verify` before you finish, the import clean-up pass, the
  deferred backlog, no planning references in code (no "M06D", "C2", "C5",
  "R-4", "M-13", "D-C2-n" in code, comments or test names; ADR ids are fine),
  simple English in comments, the 800-line file cap, the 100-line function
  cap, and the module layout rule (`foo.rs` beside `foo/`, never `mod.rs`
  under `src/`).
  **This plan names its decisions `C2-1` to `C2-14`. Those ids are for this
  plan only. When a step says to write a comment "for the reason in C2-n",
  write the reason itself in the comment, never the id.**
- `task.md` in this folder: §2 item 5 (the key algorithm), §3 slice C2, and
  slices C3 to C6 (the users of what C2 builds).
- `docs/planning/changes/service-mailbox/change.md`: criteria M-2, M-4, M-11,
  M-13, M-14, K-1, K-2, S-1, S-5, R-3, R-4, R-5, R-7; Design sections
  "Sealed message", "Mailbox key", "Mailbox methods", "Receiver flow";
  `## Deviations`.
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) (all),
  [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1
  and §3 (member master, instance key, instance certificate).
- The code this slice builds on (read, do not change):
  - `crates/identity/src/delegation.rs`: `DelegationCertificate`,
    `verify_at`, `SCOPE_SERVICE_INSTANCE`.
  - `crates/identity/src/keys.rs`: `Identity::sign_json`.
  - `crates/identity/src/substrate.rs`: `canonicalize_json_value`,
    `verify_json_signature`, `resolve_did_key`, `derive_did_key`.
  - `crates/passport/src/proof.rs`: the pattern this slice copies for a
    signed statement with a domain tag (`challenge_message`,
    `sign_challenge`).
  - `crates/router/src/proxy/router.rs:345-392`: how a hosted service's
    instance key and certificate are found today
    (`node_identity.derive_service_identity(owner, sid)` and
    `registry.instance_cert(sid)`). C5 and C6 use the same pair with C2's
    functions.

**Branch.** Work on `feat/m06d-c2-implementation`. It exists locally and was
cut from `main` at `cec9f26b` (B3 merged, #309). It is not on `origin` yet.

C2 needs only the S0 key decision. It changes no existing crate's code. It
adds one new crate and two workspace dependencies.

## 1. What C2 builds, in one paragraph

A new crate `syneroym-mailbox-format` (`crates/mailbox_format`) holds the
three formats that the mailbox, the sender and the receiver share, and the
functions that make and check them: (1) the **mailbox key pair** of a service
member (X25519, as text for the service record and the deploy call); (2) the
**deposit**: the outer part the mailbox reads (destination, random deposit
id, time-to-live, sealed bytes) and the inner part only the receiver can
open (interface, method, params, idempotency key, the sender's instance
certificate, the send time, and the sender's signature), sealed with HPKE
base mode; (3) the **receipt** a sender reads back from the mailbox, where a
`delivered` receipt carries the receiver's signature and instance
certificate. Every function is synchronous and pure: it takes `now_secs`
from the caller and does no I/O. No outbox, router, store or RPC method uses
the crate yet; C3 to C6 do.

## 2. Decisions this plan makes (check them before you start)

Each fills a gap in task.md or the change doc. Items marked **owner** need a
yes from the owner before the step that uses them; the plan gives the default
to use if the owner does not answer.

| # | Decision | Why |
| --- | --- | --- |
| C2-1 | New crate `crates/mailbox_format`, package `syneroym-mailbox-format`. Depends on `syneroym-identity` only (no `core`, no `rpc`, no SQLite). | Four slices use it from four crates: `core` (C3 record field), `router` (C5 sender), `substrate` (C6 receiver), the mailbox service (C4). A crate with only identity below it can be used by all four with no cycle. It must not go in `signed_record`: that crate compiles for `wasm32-wasip2` guests and promises to expose nothing that needs a private key. It must not go in the C4 service crate: the router would then link the mailbox store. |
| C2-2 | HPKE base mode, `DHKEM(X25519, HKDF-SHA256)`, `HKDF-SHA256`, `AES-256-GCM`, `info = b"syneroym-mailbox-v1"`, **empty `aad`**, through the `hpke` crate. All HPKE calls live in one private module (`src/crypto.rs`), so the fallback in Step 1 changes one file. | Fixed by task.md §2.5 and the change doc. The empty `aad` is explained in C2-4. |
| C2-3 | The sender's signature covers this JSON object (canonical JSON, signed with `Identity::sign_json`): `{"domain": "syneroym-mailbox-message-v1", "destination", "deposit_id", "idempotency_key", "payload_hash", "sent_at_secs"}`. **`deposit_id` is added** to the list in the change doc. | Without it, a dishonest mailbox can relabel: give the receiver message A (from sender 1) under the deposit id of message B (from sender 2), receive the receiver's signed `delivered` for B, drop B, and show sender 2 a valid `delivered` receipt for a message that never arrived. With `deposit_id` in the signed object, the receiver rebuilds the object from the deposit id the mailbox claims and the signature fails. Record as a deviation (Step 9). |
| C2-4 | The receiver never reads `destination` or `deposit_id` from inside the sealed part. It rebuilds the signed object from its **own** service id (the member master DID it pulls for) and the **deposit id the mailbox gave at claim**. A wrong value shows as `bad_signature`. So the HPKE `aad` adds nothing and stays empty. | One binding mechanism, not two. |
| C2-5 | `payload_hash` = lowercase hex of SHA-256 over `serde_json::to_vec(&canonicalize_json_value(&json!({"interface": i, "method": m, "params": p})))`. | Binds interface and method as well as params. The receiver recomputes it from the parsed inner part, so the form must survive a JSON round trip; a test proves it (Step 6, test `payload_hash_is_stable_across_a_json_round_trip`). |
| C2-6 | The outer part has **no `size` field** (the change doc lists one). Size is the length of the decoded sealed bytes. | A field that repeats a derived value is one more thing to check and to get wrong. Record as a deviation. |
| C2-7 | Text forms on the wire: mailbox public and secret keys as z-base-32 (52 characters, like DIDs and signatures in this repo); deposit id as 32 lowercase hex characters (16 random bytes); sealed bytes as standard base64 with padding (new direct dependency `base64` 0.22, already in `Cargo.lock` through other crates). | Keys go into the 1000-byte BEP44 record (K-1), so the short z32 form matters. Sealed bodies are up to 64 KiB and a phone downloads them; base64 costs 33 % where hex costs 100 %. |
| C2-8 | `MAX_SEALED_BYTES = 64 * 1024`, counted on the raw sealed bytes (32-byte HPKE `enc` + ciphertext + 16-byte tag), before base64. The same constant is S-1's "sealed size" and M-4's "message size". | One number, one place. |
| C2-9 | The mailbox secret key is the 32-byte seed of RFC 9180 `DeriveKeyPair`. The private and public X25519 keys are derived from it when needed. | Generation needs no random-number type from the `hpke` crate (only `getrandom`), and the stored form is one fixed-size value. C3 delivers this seed in the deploy call. |
| C2-10 | `open` checks (in this order): sealed length, HPKE open, inner JSON parse (`deny_unknown_fields`), sender signature, sender certificate valid **at `sent_at_secs`** (`verify_at` with scope `service-instance`), then the time window: `sent_at_secs + max_ttl_secs + max_clock_skew_secs >= now_secs` and `sent_at_secs <= now_secs + max_clock_skew_secs`. | R-4. The future bound is not in R-4; it is added because Conversation has the same rule (`crates/conversation/src/transport.rs:320`) and an unbounded future time would make the age check useless. Record as a deviation. |
| C2-11 | `open` does **not** check revocation. It returns the sender's instance DID and master DID; C6 checks the instance DID against the master's anchor (`MasterAnchorResolver`, as `crates/router/src/handshake.rs:66-75` does) and acks `rejected` with code `revoked`. | Revocation needs an async network lookup. Every function in this crate is pure. |
| C2-12 | The age bound takes `max_ttl_secs` as a parameter (`OpenPolicy`), default `OPEN_MAX_TTL_SECS = 7 days`; `max_clock_skew_secs` default `MAX_CLOCK_SKEW_SECS = 86_400`. | A dedicated mailbox may raise the time-to-live with the passport fact `max_ttl_secs` (ADR-0015 A10). The receiver learns its queue's limit at `register` (C4, C8) and passes it in. See §4 item 3. |
| C2-13 | A `delivered` receipt signature covers `{"domain": "syneroym-mailbox-receipt-v1", "deposit_id", "state": "delivered", "at_secs"}`, signed with the receiver's instance key. The sender checks: the certificate's `master_did` equals the receiver service id it deposited to, the certificate has scope `service-instance` and was valid at `at_secs`, the signature verifies with the certificate's `temporary_did`, and `at_secs <= now_secs + max_clock_skew_secs`. | M-13 as written, plus a domain tag and the same future bound as C2-10. |
| C2-14 | **owner.** A `rejected` receipt made by the receiver is signed too, with the same object plus `"code"`. A `rejected` receipt made by the mailbox (5 claims, M-14) has no signature. The format carries the signature as optional. How the sender treats an unsigned `rejected` is decided in C5. **Default if no answer: build it as described.** | S-5 says `rejected` fails the message. `rejected` is then the one answer by which a dishonest mailbox can make a sender drop a message for good (`expired` only sends it back to direct attempts). With an optional signature in the format now, C5 can choose "fail only on a signed rejection; treat an unsigned one like `expired`" without a format change. If the owner says no, drop `proof` from `Receipt::Rejected` and the `AckOutcome::Rejected` signing path; nothing else changes. See §4 item 1. |

## 3. Step-by-step changes

### Step 1 — dependency check, with a stop gate

The S0 check that `hpke` fits the workspace pins was never recorded (see §4
item 6). Do it first.

1. Find the latest stable `hpke` on crates.io (0.13.x is expected; take the
   newest non-pre-release). Read its `Cargo.toml` in
   `~/.cargo/registry/src/*/hpke-<ver>/` after the first `cargo fetch`
   (network needs the sandbox off or `index.crates.io`,
   `static.crates.io` allowed). Record: its feature names, and which
   `rand_core`, `x25519-dalek`, `aes-gcm`, `hkdf`, `sha2` versions it uses.
2. Add to the root `Cargo.toml` `[workspace.dependencies]`, in the
   alphabetical place of each:

   ```toml
   base64 = "0.22"
   hpke = { version = "<latest 0.13.x>", default-features = false, features = ["alloc", "x25519"] }
   ```

   Use the feature names you read in 1. The goal: X25519 KEM and the
   `single_shot_*` functions, no P-256/P-384/P-521, no `std` unless the
   `single_shot` functions need it.
3. Create the crate skeleton (Step 2), then run:

   ```bash
   cargo tree -p syneroym-mailbox-format -e normal > target/c2-tree.txt
   cargo tree --workspace -d -e normal > target/c2-dupes-after.txt
   ```

   Compare `-d` output with the same command on `main`. **Pass** if the only
   new crates in the whole graph are `hpke` and `base64` 0.22 is not new
   (it is already in `Cargo.lock`), and no new *version* of
   `curve25519-dalek`, `x25519-dalek`, `rand_core`, `aes-gcm`, `hkdf`,
   `sha2`, `digest` or `generic-array` appears. Today `Cargo.lock` has
   `curve25519-dalek` 4.1.3 (and 5.0.0-pre.1 for iroh), `x25519-dalek`
   2.0.1, `rand_core` 0.6.4 / 0.9.5 / 0.10.1, `aes-gcm` 0.10.3, `hkdf`
   0.12.4, `sha2` 0.10.9, so any `hpke` built on the stable 0.10/0.12/0.14
   RustCrypto line passes.
4. Run `cargo deny check licenses` and `cargo audit` (sandbox off for audit).
5. **Random-number type for `single_shot_seal`.** It takes `&mut R` where
   `R: rand_core::CryptoRng + rand_core::RngCore` of the `rand_core` version
   `hpke` depends on. Use that version's OS generator:
   - `rand_core` 0.6: `rand_core::OsRng` with feature `getrandom` (add
     `rand_core = { version = "0.6", features = ["getrandom"] }` to the
     crate, not the workspace, unless a workspace entry exists).
   - `rand_core` 0.9: `rand_core::OsRng` with feature `os_rng`, or
     `rand::rngs::OsRng` from the workspace `rand = "0.9"`, whichever
     compiles without a new crate. Prefer the workspace `rand`.
   If `hpke` re-exports `rand_core` (check its `lib.rs`), use the re-export
   and add no dependency.

**Stop gate.** If 3 or 4 fails (a new curve or RNG version, an advisory, or a
license problem), do not add `hpke`. Use the fallback below and record it as
a deviation. Do not pick another sealing algorithm.

**Fallback: RFC 9180 base mode written by hand in `src/crypto.rs`**, on
`x25519-dalek` 2 (feature `static_secrets`), `hkdf` 0.12, `sha2` 0.10,
`aes-gcm` 0.10 (all already in the lock). Suite ids: KEM `0x0020`, KDF
`0x0001`, AEAD `0x0002`.

```text
I2OSP(n, w)            = n as w big-endian bytes
kem_suite              = "KEM"  || I2OSP(0x0020, 2)
hpke_suite             = "HPKE" || I2OSP(0x0020, 2) || I2OSP(0x0001, 2) || I2OSP(0x0002, 2)
LabeledExtract(suite, salt, label, ikm)      = HKDF-Extract(salt, "HPKE-v1" || suite || label || ikm)
LabeledExpand(suite, prk, label, info, L)    = HKDF-Expand(prk, I2OSP(L, 2) || "HPKE-v1" || suite || label || info, L)

DeriveKeyPair(seed):   dkp_prk = LabeledExtract(kem_suite, "", "dkp_prk", seed)
                       sk = LabeledExpand(kem_suite, dkp_prk, "sk", "", 32); pk = X25519(sk, 9)
Encap(pkR):            skE = 32 bytes from getrandom; pkE = X25519(skE, 9); dh = X25519(skE, pkR)
                       if dh is all zero -> error
                       kem_context = pkE || pkR
                       eae_prk = LabeledExtract(kem_suite, "", "eae_prk", dh)
                       shared = LabeledExpand(kem_suite, eae_prk, "shared_secret", kem_context, 32)
                       return (shared, enc = pkE)
Decap(enc, skR):       dh = X25519(skR, enc); same steps with kem_context = enc || pk(skR)
KeySchedule(shared, info), mode 0x00, no psk:
                       psk_id_hash = LabeledExtract(hpke_suite, "", "psk_id_hash", "")
                       info_hash   = LabeledExtract(hpke_suite, "", "info_hash", info)
                       ctx         = 0x00 || psk_id_hash || info_hash
                       secret      = LabeledExtract(hpke_suite, shared, "secret", "")
                       key         = LabeledExpand(hpke_suite, secret, "key", ctx, Nk)   (Nk = 32)
                       base_nonce  = LabeledExpand(hpke_suite, secret, "base_nonce", ctx, 12)
Seal (one message, seq 0): ct = AES-256-GCM(key, nonce = base_nonce, aad = "", pt)
```

Make `Nk` and the AEAD a private generic parameter so a test can run the
same code with `AES-128-GCM` (`Nk = 16`, AEAD id `0x0001`) against the
RFC 9180 Appendix A.1.1 vector (DHKEM(X25519, HKDF-SHA256), HKDF-SHA256,
AES-128-GCM, base mode). Copy that vector from the RFC text; do not
invent values. The public API of `src/crypto.rs` stays the same as in the
`hpke` version.

### Step 2 — the crate skeleton

Files (all new):

```text
crates/mailbox_format/Cargo.toml
crates/mailbox_format/src/lib.rs
crates/mailbox_format/src/crypto.rs        crates/mailbox_format/src/crypto/tests.rs
crates/mailbox_format/src/keys.rs          crates/mailbox_format/src/keys/tests.rs
crates/mailbox_format/src/deposit.rs       crates/mailbox_format/src/deposit/tests.rs
crates/mailbox_format/src/message.rs       crates/mailbox_format/src/message/tests.rs
crates/mailbox_format/src/receipt.rs       crates/mailbox_format/src/receipt/tests.rs
crates/mailbox_format/src/error.rs         crates/mailbox_format/src/error/tests.rs
crates/mailbox_format/src/test_support.rs  (cfg(test) only)
```

If `message/tests.rs` passes 800 lines, split it into
`message/tests.rs` (declares `mod open; mod seal;`) plus
`message/tests/open.rs` and `message/tests/seal.rs`.

`Cargo.toml`:

```toml
[package]
name = "syneroym-mailbox-format"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
syneroym-identity.workspace = true
base64.workspace = true
getrandom.workspace = true
hex.workspace = true
hpke.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
z32.workspace = true
zeroize.workspace = true
# plus the rand_core line from Step 1.5, only if needed
```

Every listed dependency must be used (`cargo shear` gate). Do not add
`anyhow`: map `anyhow::Error` from `syneroym-identity` with
`.map_err(|e| ...(e.to_string()))`.

Root `Cargo.toml`: add `syneroym-mailbox-format = { path =
"crates/mailbox_format" }` to `[workspace.dependencies]` beside
`syneroym-passport`. If `cargo shear` reports the workspace entry as unused
(no crate depends on it until C3), remove the entry and say so in the PR
text; C3 adds it back.

`src/lib.rs`:

```rust
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The formats a service mailbox, a sender and a receiver share (ADR-0026):
//! the mailbox key pair of a service member, the deposit (an outer part the
//! mailbox reads and an inner part sealed for the receiver), and the receipt
//! the sender reads back.
//!
//! Every function here is pure: the caller passes the time and does any
//! network lookup. Revocation of a sender's instance key is not checked
//! here; the receiver checks it against the master's anchor after `open`.

mod crypto;
mod deposit;
mod error;
mod keys;
mod message;
mod receipt;

pub use deposit::{Deposit, DepositId, SealedBytes};
pub use error::{DepositError, OpenError, ReceiptError, SealError};
pub use keys::{MailboxPublicKey, MailboxSecretKey};
pub use message::{MessageContent, OpenPolicy, OpenedMessage, Recipient, Sender, open, seal};
pub use receipt::{AckOutcome, Receipt, ReceiverProof, RejectCode, sign_receipt};

/// The HPKE `info` input: domain separation for every sealed message.
pub const HPKE_INFO: &[u8] = b"syneroym-mailbox-v1";
/// Domain tag of the sender's signature over a message.
pub const MESSAGE_DOMAIN: &str = "syneroym-mailbox-message-v1";
/// Domain tag of the receiver's signature over a receipt.
pub const RECEIPT_DOMAIN: &str = "syneroym-mailbox-receipt-v1";
/// Largest sealed message, in raw bytes before base64.
pub const MAX_SEALED_BYTES: usize = 64 * 1024;
/// Longest time-to-live of an open mailbox. A dedicated mailbox may allow
/// more; the receiver then passes its own limit in `OpenPolicy`.
pub const OPEN_MAX_TTL_SECS: u64 = 7 * 24 * 3600;
/// Clock difference allowed between two nodes, the same as Conversation.
pub const MAX_CLOCK_SKEW_SECS: u64 = 24 * 3600;

#[cfg(test)]
mod test_support;
```

### Step 3 — `src/crypto.rs` (the only file that names `hpke`)

```rust
/// Bytes HPKE adds to a plaintext: the 32-byte encapsulated key and the
/// 16-byte AES-GCM tag.
pub(crate) const SEAL_OVERHEAD: usize = 32 + 16;
const ENC_LEN: usize = 32;

type Kem = hpke::kem::X25519HkdfSha256;
type Kdf = hpke::kdf::HkdfSha256;
type Aead = hpke::aead::AesGcm256;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum CryptoError {
    #[error("sealed bytes are shorter than the HPKE overhead")]
    TooShort,
    #[error("the sealed bytes do not open with this key")]
    Undecryptable,
    #[error("the public key is not a valid X25519 key")]
    BadPublicKey,
    #[error("sealing failed: {0}")]
    Seal(String),
}

/// X25519 key pair from the 32-byte seed (RFC 9180 DeriveKeyPair).
pub(crate) fn public_key_from_seed(seed: &[u8; 32]) -> [u8; 32];

/// `enc || ciphertext`. Fresh ephemeral key for each call.
pub(crate) fn seal_to(public_key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError>;

/// Opens `enc || ciphertext` made by `seal_to`.
pub(crate) fn open_with(seed: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, CryptoError>;
```

Pseudo-code (check every `hpke` name against the fetched source; the names
below are from `hpke` 0.12/0.13 and may differ slightly):

```text
public_key_from_seed(seed):
    (_sk, pk) = <Kem as hpke::Kem>::derive_keypair(seed)
    return pk.to_bytes() as [u8; 32]

seal_to(pk_bytes, plaintext):
    pk = <Kem as hpke::Kem>::PublicKey::from_bytes(pk_bytes)  -> map_err BadPublicKey
    rng = OS generator from Step 1.5
    (enc, ct) = hpke::single_shot_seal::<Aead, Kdf, Kem, _>(&OpModeS::Base, &pk, HPKE_INFO, plaintext, b"", &mut rng)
                -> map_err Seal(e.to_string())
    return enc.to_bytes() ++ ct

open_with(seed, sealed):
    if sealed.len() < SEAL_OVERHEAD: return TooShort
    (enc_bytes, ct) = sealed.split_at(ENC_LEN)
    enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(enc_bytes) -> map_err Undecryptable
    (sk, _pk) = <Kem as hpke::Kem>::derive_keypair(seed)
    return hpke::single_shot_open::<Aead, Kdf, Kem>(&OpModeR::Base, &sk, &enc, HPKE_INFO, ct, b"")
           -> map_err Undecryptable
```

Every decryption failure maps to one `Undecryptable`: wrong key and changed
bytes must look the same to the caller.

### Step 4 — `src/keys.rs`

```rust
/// The public half of a service member's mailbox key. It goes into the
/// signed service record. Text form: z-base-32 of the 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MailboxPublicKey([u8; 32]);

/// The private half: the 32-byte seed the X25519 key pair is derived from.
/// The deployer makes it with the member master and gives it to each
/// instance; it is never logged.
#[derive(Clone, zeroize::ZeroizeOnDrop)]
pub struct MailboxSecretKey { seed: [u8; 32] }
```

Methods:

| Signature | Behaviour |
| --- | --- |
| `MailboxSecretKey::generate() -> Result<Self, getrandom::Error>` | 32 bytes from `getrandom::fill`. |
| `MailboxSecretKey::from_seed(seed: [u8; 32]) -> Self` | For C3's store. |
| `MailboxSecretKey::seed(&self) -> &[u8; 32]` | `pub(crate)`. |
| `MailboxSecretKey::public_key(&self) -> MailboxPublicKey` | `crypto::public_key_from_seed`. |
| `MailboxSecretKey::to_z32(&self) -> zeroize::Zeroizing<String>` and `from_z32(s: &str) -> Result<Self, KeyParseError>` | Text form for the deploy call (C3). |
| `impl fmt::Debug for MailboxSecretKey` | Prints `MailboxSecretKey(<public key z32>)`, never the seed. |
| `MailboxPublicKey::from_bytes([u8; 32]) -> Self`, `as_bytes(&self) -> &[u8; 32]` | |
| `impl Display` (z32), `impl FromStr` (`Err = KeyParseError`) | Exactly 32 decoded bytes, else error. |
| `impl Serialize/Deserialize` for `MailboxPublicKey` | As the z32 string (`#[serde(into = "String", try_from = "String")]` with the `From`/`TryFrom` impls). |

`KeyParseError` is a small `thiserror` enum in `keys.rs`
(`NotZ32`, `WrongLength(usize)`), re-exported from `lib.rs`. Do not give
`MailboxSecretKey` serde impls: C3 decides how the seed travels and is
stored, and a derive would make it too easy to log.

Whether `from_bytes` must reject the all-zero or low-order X25519 points:
`hpke` rejects an all-zero shared secret at seal time, which is enough;
`seal` maps it to `SealError::Crypto`. The fallback (Step 1) must do the
same check.

### Step 5 — `src/deposit.rs` (the outer part)

```rust
/// A random id the sender picks for one deposit. It is not the idempotency
/// key: Conversation keys come from the content, and a mailbox could then
/// confirm a guessed message by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct DepositId([u8; 16]);

/// The sealed inner part, base64 on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct SealedBytes(Vec<u8>);

/// What a sender gives the mailbox. The sender is not a field: the mailbox
/// takes it from the verified transport identity (ADR-0016).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deposit {
    /// Member master DID of the receiving service.
    pub destination: String,
    pub deposit_id: DepositId,
    pub ttl_secs: u64,
    pub sealed: SealedBytes,
}
```

Methods:

| Signature | Behaviour |
| --- | --- |
| `DepositId::random() -> Result<Self, getrandom::Error>` | 16 bytes from `getrandom::fill`. |
| `impl Display/FromStr for DepositId` | 32 lowercase hex characters. `FromStr` accepts only that (`hex::decode_to_slice` into `[u8; 16]`, then refuse upper case so one id has one text form). |
| `SealedBytes::as_slice(&self) -> &[u8]`, `len(&self) -> usize`, `pub(crate) fn new(Vec<u8>) -> Self` | |
| `TryFrom<String> for SealedBytes` | `base64::engine::general_purpose::STANDARD.decode`. Refuse a decoded length over `MAX_SEALED_BYTES` here too, so a huge body is refused while parsing. |
| `Deposit::check(&self, max_ttl_secs: u64) -> Result<(), DepositError>` | For the mailbox (C4, M-4). `destination` must pass `syneroym_identity::substrate::resolve_did_key`; `1 <= ttl_secs <= max_ttl_secs`; `sealed.len() <= MAX_SEALED_BYTES` and `>= crypto::SEAL_OVERHEAD`. Each failure has its own variant. |

### Step 6 — `src/message.rs` (seal and open)

Public types:

```rust
/// The call a sender seals. Only the receiver reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageContent {
    pub interface: String,
    pub method: String,
    pub params: serde_json::Value,
    pub idempotency_key: String,
}

/// The receiving service member: its id and its published mailbox key.
#[derive(Debug, Clone, Copy)]
pub struct Recipient<'a> {
    pub service_id: &'a str,
    pub mailbox_key: &'a MailboxPublicKey,
}

/// The sending service instance: its instance key and the certificate by
/// which its member master allows that key (ADR-0020).
#[derive(Debug, Clone, Copy)]
pub struct Sender<'a> {
    pub instance: &'a Identity,
    pub certificate: &'a DelegationCertificate,
}

/// Limits the receiver applies to the send time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPolicy {
    /// The longest time-to-live of the receiver's mailbox queue.
    pub max_ttl_secs: u64,
    pub max_clock_skew_secs: u64,
}
impl Default for OpenPolicy { /* OPEN_MAX_TTL_SECS, MAX_CLOCK_SKEW_SECS */ }

/// A message that opened and passed every check of this crate. Revocation
/// is not checked yet.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenedMessage {
    pub content: MessageContent,
    pub sender_certificate: DelegationCertificate,
    pub sent_at_secs: u64,
}
impl OpenedMessage {
    /// The caller identity of the delivered call (the original sender,
    /// never the mailbox).
    pub fn sender_master_did(&self) -> &str { &self.sender_certificate.master_did }
    /// The key to check against the master's revocation list.
    pub fn sender_instance_did(&self) -> &str { &self.sender_certificate.temporary_did }
}
```

Private inner part (the plaintext before sealing):

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedContent {
    interface: String,
    method: String,
    params: serde_json::Value,
    idempotency_key: String,
    sent_at_secs: u64,
    sender_certificate: DelegationCertificate,
    /// z-base-32 Ed25519 signature over `signed_statement(..)`.
    signature: String,
}
```

Private helpers:

```rust
fn payload_hash(interface: &str, method: &str, params: &Value) -> Result<String, serde_json::Error>
    // C2-5: hex(sha256(to_vec(canonicalize_json_value(json!({...})))))

fn signed_statement(destination: &str, deposit_id: &DepositId, idempotency_key: &str,
                    payload_hash: &str, sent_at_secs: u64) -> Value
    // json!({"domain": MESSAGE_DOMAIN, "destination": .., "deposit_id": deposit_id.to_string(),
    //        "idempotency_key": .., "payload_hash": .., "sent_at_secs": ..})

/// Builds and seals without the sender-side checks, so tests can make
/// messages a real sender would refuse to make.
pub(crate) fn seal_unchecked(content: &MessageContent, recipient: Recipient<'_>,
                             signer: &Identity, certificate: &DelegationCertificate,
                             deposit_id: DepositId, ttl_secs: u64, sent_at_secs: u64)
                             -> Result<Deposit, SealError>
```

`seal_unchecked` has 7 parameters; `clippy::too_many_arguments` warns
above 7 (the workspace `clippy.toml` does not change it), so it passes. If
clippy still complains, group `deposit_id`, `ttl_secs` and `sent_at_secs`
into a private `Envelope` struct; do not add an `allow`.

**`pub fn seal(content: &MessageContent, recipient: Recipient<'_>, sender: Sender<'_>, ttl_secs: u64, now_secs: u64) -> Result<Deposit, SealError>`**

```text
if ttl_secs == 0: return ZeroTtl
resolve_did_key(recipient.service_id) else NotADidKey
if derive_did_key(&sender.instance.public_key()) != sender.certificate.temporary_did:
    return SignerMismatch
sender.certificate.verify_at(&sender.certificate.master_did, &[SCOPE_SERVICE_INSTANCE], now_secs)
    else BadCertificate(e.to_string())        // never make a message the receiver will reject
deposit_id = DepositId::random() else Random
return seal_unchecked(content, recipient, sender.instance, sender.certificate,
                      deposit_id, ttl_secs, now_secs)
```

**`seal_unchecked`**

```text
hash = payload_hash(&content.interface, &content.method, &content.params) else Json
statement = signed_statement(recipient.service_id, &deposit_id, &content.idempotency_key, &hash, sent_at_secs)
signature = signer.sign_json(&statement) else Json
inner = SealedContent { content fields cloned, sent_at_secs, sender_certificate: certificate.clone(), signature }
plaintext = serde_json::to_vec(&inner) else Json
if plaintext.len() + crypto::SEAL_OVERHEAD > MAX_SEALED_BYTES:
    return TooLarge { bytes: plaintext.len() + SEAL_OVERHEAD, max: MAX_SEALED_BYTES }
sealed = crypto::seal_to(recipient.mailbox_key.as_bytes(), &plaintext) else Crypto
return Deposit { destination: recipient.service_id.to_string(), deposit_id, ttl_secs, sealed: SealedBytes::new(sealed) }
```

`TooLarge` is the signal C5 uses for S-1 ("sealed size at most 64 KiB"):
the call stays on direct delivery.

**`pub fn open(key: &MailboxSecretKey, own_service_id: &str, deposit_id: &DepositId, sealed: &SealedBytes, policy: OpenPolicy, now_secs: u64) -> Result<OpenedMessage, OpenError>`**

```text
if sealed.len() > MAX_SEALED_BYTES: return TooLarge
plaintext = crypto::open_with(key.seed(), sealed.as_slice())
            TooShort -> Malformed("too short"); anything else -> Undecryptable
inner: SealedContent = serde_json::from_slice(&plaintext) else Malformed(e.to_string())
hash = payload_hash(&inner.interface, &inner.method, &inner.params) else Malformed
statement = signed_statement(own_service_id, deposit_id, &inner.idempotency_key, &hash, inner.sent_at_secs)
verify_json_signature(&inner.sender_certificate.temporary_did, &statement, &inner.signature)
    else BadSignature
inner.sender_certificate.verify_at(&inner.sender_certificate.master_did,
                                   &[SCOPE_SERVICE_INSTANCE], inner.sent_at_secs)
    else BadCertificate(e.to_string())
oldest = now_secs.saturating_sub(policy.max_ttl_secs.saturating_add(policy.max_clock_skew_secs))
if inner.sent_at_secs < oldest: return TooOld { sent_at_secs, oldest_secs: oldest }
latest = now_secs.saturating_add(policy.max_clock_skew_secs)
if inner.sent_at_secs > latest: return FromFuture { sent_at_secs, latest_secs: latest }
return OpenedMessage { content: MessageContent { .. }, sender_certificate, sent_at_secs }
```

Notes for the code comments (write the reason, not the id):

- The signature is checked against the certificate's own `temporary_did`,
  and the certificate against its own `master_did`. The result is "this
  master's instance signed this". The receiver then uses
  `sender_master_did()` as the caller identity; it never reads a DID from
  the mailbox.
- `verify_at(.., sent_at_secs)` checks the certificate at the send time,
  not now: a message that waits 6 hours at the mailbox would otherwise
  carry an expired certificate.
- An empty idempotency key is not checked here, in `seal` or in `open`.
  Whether a call has a usable key is part of S-1 eligibility, which C5
  checks before it calls `seal`.

Split `open` into small private functions so it stays readable and under
100 lines: `decrypt_inner`, `check_signature`, `check_send_time`.

### Step 7 — `src/receipt.rs`

```rust
/// Why a message was closed without delivery. Stable wire codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    // the receiver could not accept the sealed message (OpenError::reject_code)
    Malformed, Undecryptable, BadSignature, BadCertificate, TooOld, FromFuture, TooLarge,
    /// The sender's instance key is revoked (checked by the receiver after `open`).
    Revoked,
    /// The method did not opt in to deferred delivery.
    NotEligible,
    /// The mailbox closed it after too many claims without an ack.
    TooManyClaims,
    /// A code this version does not know.
    #[serde(other)]
    Unknown,
}

/// The receiver's signature over a receipt, with the certificate that ties
/// its instance key to the receiving service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverProof {
    pub receiver_certificate: DelegationCertificate,
    /// z-base-32 Ed25519 signature over the receipt statement.
    pub signature: String,
}

/// What the mailbox answers to `status` for one deposit id. The mailbox
/// stores a receiver-signed receipt as it got it; it cannot make one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Receipt {
    Stored { at_secs: u64 },
    Delivered { at_secs: u64, proof: ReceiverProof },
    Rejected {
        at_secs: u64,
        code: RejectCode,
        /// Present when the receiver rejected it; absent when the mailbox did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proof: Option<ReceiverProof>,
    },
    Expired { at_secs: u64 },
    /// The mailbox has no receipt for this id (never stored, or its receipt time ended).
    Unknown,
}

/// What the receiver acknowledges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckOutcome { Delivered, Rejected(RejectCode) }
```

Do **not** add `deny_unknown_fields` to `Receipt` or `ReceiverProof`: a
sender reads them from mailboxes of other operators, which may be newer
(same rule as `crates/passport/src/wire.rs`).

Private helper:

```rust
fn receipt_statement(deposit_id: &DepositId, outcome: AckOutcome, at_secs: u64) -> Value
    // Delivered:        {"domain": RECEIPT_DOMAIN, "deposit_id", "state": "delivered", "at_secs"}
    // Rejected(code):   {"domain": RECEIPT_DOMAIN, "deposit_id", "state": "rejected", "code": <snake_case code>, "at_secs"}
```

Get the code string with `serde_json::to_value(code)` so the signed text
and the wire text never differ.

**`pub fn sign_receipt(instance: &Identity, certificate: &DelegationCertificate, deposit_id: &DepositId, outcome: AckOutcome, at_secs: u64) -> Result<Receipt, ReceiptError>`**

```text
if derive_did_key(&instance.public_key()) != certificate.temporary_did: SignerMismatch
certificate.verify_at(&certificate.master_did, &[SCOPE_SERVICE_INSTANCE], at_secs) else BadCertificate
signature = instance.sign_json(&receipt_statement(deposit_id, outcome, at_secs)) else Json
proof = ReceiverProof { receiver_certificate: certificate.clone(), signature }
match outcome:
    Delivered     -> Receipt::Delivered { at_secs, proof }
    Rejected(code)-> Receipt::Rejected { at_secs, code, proof: Some(proof) }
```

**`impl Receipt { pub fn verify_signed(&self, deposit_id: &DepositId, receiver_service_id: &str, max_clock_skew_secs: u64, now_secs: u64) -> Result<(), ReceiptError> }`**

```text
(outcome, at_secs, proof) = match self:
    Delivered { at_secs, proof }               -> (Delivered, at_secs, proof)
    Rejected { at_secs, code, proof: Some(p) } -> (Rejected(code), at_secs, p)
    _                                          -> return Unsigned
cert = &proof.receiver_certificate
if cert.master_did != receiver_service_id: WrongReceiver
cert.verify_at(receiver_service_id, &[SCOPE_SERVICE_INSTANCE], at_secs) else BadCertificate
verify_json_signature(&cert.temporary_did, &receipt_statement(deposit_id, outcome, at_secs), &proof.signature)
    else BadSignature
if at_secs > now_secs.saturating_add(max_clock_skew_secs): FromFuture
Ok(())
```

`verify_signed` is for the sender (C5) and, if C4 chooses, for the mailbox
before it stores an ack. What the sender does with each receipt state is
C5's decision (S-5 and §4 item 1), not this crate's.

### Step 8 — `src/error.rs`

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("time-to-live is zero")] ZeroTtl,
    #[error("destination is not a did:key: {0}")] NotADidKey(String),
    #[error("the instance key does not match the certificate")] SignerMismatch,
    #[error("the sender certificate is not valid now: {0}")] BadCertificate(String),
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")] TooLarge { bytes: usize, max: usize },
    #[error("no random bytes: {0}")] Random(String),
    #[error("json: {0}")] Json(String),
    #[error("sealing failed: {0}")] Crypto(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OpenError {
    TooLarge { bytes: usize, max: usize },
    Undecryptable,
    Malformed(String),
    BadSignature,
    BadCertificate(String),
    TooOld { sent_at_secs: u64, oldest_secs: u64 },
    FromFuture { sent_at_secs: u64, latest_secs: u64 },
}
impl OpenError {
    /// The code the receiver puts in its `rejected` ack. Every open error
    /// is permanent: the same bytes fail the same way next time.
    pub fn reject_code(&self) -> RejectCode;   // one arm per variant, same names
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReceiptError {
    Unsigned, WrongReceiver, BadSignature, BadCertificate(String), FromFuture,
    SignerMismatch, Json(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DepositError {
    NotADidKey(String),
    ZeroTtl,
    TtlTooLong { ttl_secs: u64, max_secs: u64 },
    TooLarge { bytes: usize, max: usize },
    TooSmall { bytes: usize, min: usize },
}
```

Give every variant an `#[error(...)]` message in simple English.
`OpenError` needs no `Display` of secrets: none of its fields hold content.

### Step 9 — tests

`src/test_support.rs` (`#[cfg(test)]`, `pub(crate)`):

```rust
pub(crate) struct Member { pub master: Identity, pub instance: Identity, pub cert: DelegationCertificate, pub mailbox: MailboxSecretKey }
impl Member {
    /// A member master, one instance key with a `service-instance`
    /// certificate valid for `lifetime_secs` from the real now, and a
    /// mailbox key.
    pub(crate) fn new(lifetime_secs: u64) -> Self;
    pub(crate) fn service_id(&self) -> String;             // derive_did_key(master.public_key())
    pub(crate) fn sender(&self) -> Sender<'_>;
    pub(crate) fn recipient<'a>(&'a self, id: &'a str) -> Recipient<'a>;
}
pub(crate) fn content() -> MessageContent;                 // interface "conversation", method "deliver", params with nested object, a float and a non-ASCII key, key "msg-1"
```

`DelegationCertificate::issue` uses the real clock, so tests use times
relative to `cert.issued_at_secs` and `cert.expires_at_secs`, not fixed
numbers. Before you finish, run the duplication recipe in `AGENTS.md`
(Context Budget) over the new files; if `Member::new` matches
`crates/passport/src/test_support.rs`, that is acceptable test-only
duplication across crates only if the gate passes.

Test names below are the exact names to use. "Wrong key", "tamper",
"replay" and "old send time" are the four kinds task.md asks for; each is
marked.

`crypto/tests.rs`:

1. `sealed_bytes_open_with_the_matching_key`
2. `sealed_bytes_do_not_open_with_another_key` (wrong key)
3. `a_changed_byte_in_the_key_part_the_body_or_the_tag_fails` (tamper: flip one bit at index 0, at index 40, and at the last index; each gives `Undecryptable`)
4. `two_seals_of_the_same_plaintext_differ`
5. `input_shorter_than_the_overhead_is_refused`
6. `a_stored_sealed_vector_still_opens` — a fixed seed (32 bytes of `0x42`), a fixed plaintext, and a sealed hex string you generate **once** with the final code and paste in. It fails if anyone changes the suite, the `info` tag or the key derivation.
7. Fallback only: `rfc9180_a_1_1_vector_matches` (Step 1).

`keys/tests.rs`:

8. `public_key_round_trips_through_its_text_form`
9. `secret_key_round_trips_and_derives_the_same_public_key`
10. `a_public_key_of_the_wrong_length_is_refused`
11. `secret_key_debug_does_not_show_the_seed` (the seed's z32 text is not in `format!("{key:?}")`)
12. `public_key_serializes_as_a_z32_string`

`deposit/tests.rs`:

13. `deposit_id_is_32_lowercase_hex_and_round_trips`
14. `deposit_id_refuses_upper_case_and_wrong_length`
15. `sealed_bytes_serialize_as_base64`
16. `sealed_bytes_over_the_limit_are_refused_while_parsing`
17. `deposit_check_refuses_zero_and_too_long_ttl`
18. `deposit_check_refuses_a_destination_that_is_not_a_did_key`
19. `deposit_check_refuses_sealed_bytes_shorter_than_the_overhead`
20. `a_deposit_with_an_unknown_field_is_refused`

`message/tests.rs` (split as Step 2 says if it grows):

21. `a_sealed_message_opens_with_its_content_and_sender`
22. `the_mailbox_sees_no_interface_method_or_params` — serialize the `Deposit` to JSON; assert the method name, the idempotency key and a param value are not in it (M-11).
23. `opening_with_another_services_key_fails` (wrong key → `Undecryptable`)
24. `a_tampered_sealed_body_fails` (tamper → `Undecryptable`)
25. `a_message_opened_under_another_deposit_id_fails` (replay/relabel → `BadSignature`)
26. `a_message_opened_by_another_destination_fails` — same mailbox key, other `own_service_id` (replay to another service → `BadSignature`)
27. `the_same_message_opened_twice_gives_the_same_idempotency_key` (replay of one deposit: the format accepts it; the receiver's idempotency fence removes the duplicate)
28. `a_signature_by_a_key_other_than_the_certificates_fails` (`seal_unchecked` with another `Identity` → `BadSignature`)
29. `a_certificate_with_another_scope_fails` (`routing` scope → `BadCertificate`)
30. `a_certificate_expired_at_send_time_fails` (`seal_unchecked` with `sent_at = cert.expires_at_secs` → `BadCertificate`)
31. `a_certificate_expired_now_but_valid_at_send_time_is_accepted` (open with `now = cert.expires_at_secs + 3600`)
32. `a_send_time_older_than_ttl_plus_skew_is_rejected` (old send time: `now = sent_at + 7d + 24h + 1` → `TooOld`; `now = sent_at + 7d + 24h` opens)
33. `a_larger_max_ttl_accepts_an_older_message` (`OpenPolicy { max_ttl_secs: 30d, .. }`)
34. `a_send_time_too_far_in_the_future_is_rejected` (old send time, other side: `now = sent_at - 24h - 1` → `FromFuture`)
35. `seal_refuses_content_over_the_size_limit` (params with a 70 000-byte string → `SealError::TooLarge`)
36. `seal_refuses_an_instance_key_that_does_not_match_the_certificate`
37. `seal_refuses_an_expired_certificate` (`now = cert.expires_at_secs`)
38. `seal_refuses_a_zero_ttl_and_a_destination_that_is_not_a_did_key`
39. `inner_content_with_an_unknown_field_is_refused` (seal by hand with `crypto::seal_to` a JSON that has an extra field → `Malformed`)
40. `payload_hash_is_stable_across_a_json_round_trip` (params with a float `0.1`, nested objects in reverse key order, a non-ASCII key; hash, serialize, parse, hash again: equal)

`receipt/tests.rs`:

41. `a_delivered_receipt_verifies_for_its_deposit_and_receiver`
42. `a_delivered_receipt_for_another_deposit_fails` (replay → `BadSignature`)
43. `a_delivered_receipt_from_another_service_fails` (other member's cert → `WrongReceiver`)
44. `a_delivered_receipt_with_a_changed_time_fails` (tamper `at_secs` → `BadSignature`)
45. `a_delivered_receipt_signed_after_the_certificate_expired_fails` (`sign_receipt` refuses it; and a hand-built one fails `verify_signed` with `BadCertificate`)
46. `a_receiver_rejection_is_signed_and_verifies`
47. `a_rejection_with_a_changed_code_fails` (tamper `code` → `BadSignature`)
48. `stored_expired_unknown_and_a_mailbox_rejection_are_unsigned` (`verify_signed` → `Unsigned`)
49. `receipts_serialize_with_a_state_tag` — assert the exact JSON of each variant, for example `{"state":"stored","at_secs":5}` and `{"state":"unknown"}`. This test is the wire contract C4 and C5 build on.
50. `an_unknown_reject_code_reads_as_unknown`
51. `sign_receipt_refuses_an_instance_key_that_does_not_match_the_certificate`

`error/tests.rs`:

52. `every_open_error_has_its_own_reject_code` (each variant maps to the code of the same name; no two variants share a code).

If the owner says no to C2-14, drop tests 46, 47 and the "mailbox rejection" half of 48.

### Step 10 — docs in the same PR

No living doc changes in this slice: no user, operator or developer can
use the crate yet, and the mailbox change doc closes out after C7/C8.
(Check once: `grep -rn "syneroym-passport" docs/developer-guide.md
docs/system-architecture.md` returns nothing today, so there is no crate
list to extend.)

1. `docs/planning/changes/service-mailbox/change.md`, `## Deviations`, add:
   - `D-C2-1`: the sender signature also covers `deposit_id` (the reason in
     C2-3).
   - `D-C2-2`: the outer part has no `size` field (C2-6).
   - `D-C2-3`: crate home `crates/mailbox_format` (`syneroym-mailbox-format`)
     and the text forms (C2-1, C2-7).
   - `D-C2-4`: the receiver also refuses a send time more than 24 hours in
     the future; the age limit uses the queue's own time-to-live (C2-10,
     C2-12).
   - `D-C2-5`: revocation is checked by the receiver after `open`, with
     the code `revoked` (C2-11).
   - `D-C2-6`: the mailbox secret key is the 32-byte `DeriveKeyPair` seed
     (C2-9).
   - `D-C2-7`: receiver-made `rejected` receipts are signed; mailbox-made
     ones are not (C2-14, or the owner's answer).
   - `D-C2-8`: the result of the Step 1 check (the `hpke` version, the
     `rand_core` version it uses, and "no new crypto versions"), or that
     the fallback was used and why. Replace nothing in the Design section;
     its "*Unverified:*" sentence is answered here.
   - Tick `- [x] Sealed message format with tests ...` in `## Tasks`.
2. `task.md`: header status line ("C2 is done"), and under slice C2 add
   "Done <date>. Decisions `D-C2-1` to `D-C2-n` are in the
   [mailbox change doc](../../changes/service-mailbox/change.md)."
3. `status.md`: replace the `C2 to C8` row with a `C2` row (with evidence:
   the test files and counts, and the verify result) and a `C3 to C8 | Not
   started` row.
4. `docs/planning/deferred-backlog.md`: add a row only for what this slice
   defers. Expected: none, unless the owner answers "no" to C2-14 — then add
   "A mailbox can fail a message for good with an unsigned `rejected`"
   under the mailbox theme, target `TBD`.

## 4. Things that are ambiguous or stale in task.md and the change docs

1. **`rejected` can be forged by the mailbox (owner decision, C2-14).**
   S-5 makes `rejected` final. M-14 lets the mailbox produce `rejected`
   itself, so any mailbox can end a message for good without the receiver.
   ADR-0026 says the mailbox "cannot invent a delivery"; it does not say
   whether it may invent a rejection. The plan adds an optional receiver
   signature to the format and leaves the sender rule to C5.
2. **Relabel attack (decided here, C2-3).** The change doc's list of signed
   fields lets a dishonest mailbox turn the receiver's `delivered` for one
   message into a receipt for another. Fixed by signing `deposit_id`.
3. **"Longest mailbox time-to-live" in R-4 and R-7 is not one number.** The
   open limit is 7 days, but the passport fact `max_ttl_secs` (ADR-0015 A10)
   lets a dedicated mailbox allow more. C2 makes it a parameter (C2-12).
   C6 must pass the queue's limit from `register`, and R-7 (C6) must size
   the idempotency memory from the same value, not from a 7-day constant.
   C8 should cap `max_ttl_secs` so that memory stays bounded; nothing caps
   it today.
4. **A sender without an instance certificate cannot seal.** The inner part
   carries "the sender's instance certificate". A service deployed without
   a member master has none (`EndpointRegistry::instance_cert` returns
   `None`, `crates/core/src/local_registry.rs:337`), and a node-level call
   (`CallOrigin::Native { service_id: None }`) has none either. S-1 does not
   list this. C5 should add "the sender has a valid instance certificate"
   to the eligibility check.
5. **"Replay" in the C2 test list was not defined.** This plan reads it as:
   the same sealed bytes under another deposit id or another destination
   (refused), a receipt for another deposit (refused), and the same deposit
   opened twice (accepted by the format; the receiver's idempotency fence,
   R-2/R-7 in C6, removes the duplicate).
6. **S0 is marked done, but the `hpke` version check was never recorded.**
   task.md §2.5 says "Check in S0", status.md says S0 is done, and the
   change doc still says "*Unverified:*". Nothing in the repo mentions
   `hpke` outside these two docs, and `hpke` is not in `Cargo.lock`. Step 1
   does the check. (When this plan was written the network was not
   available, so the check could not be done here.)
7. **"Outer: ... size" (change doc, Sealed message).** Dropped (C2-6).
8. **No future bound on the send time in R-4.** Added (C2-10).
9. **The change doc says the receiver checks "not revoked now".** That
   needs the master anchor lookup, so it lives in C6, not in the format
   (C2-11).
10. **task.md §1 "Facts" are still true for C2:** `x25519-dalek` 2.0.1 is in
    `Cargo.lock` through `vodozemac`; the router handshake is P-256 ECDH and
    not reusable; no sealed box exists in the workspace.
11. **C4 detail this plan exposes, not decides:** the mailbox should key
    receipts on `(sender, deposit_id)` or refuse a duplicate `deposit_id`,
    because the sender picks the id and a second sender could reuse it.
    Write this in the C4 plan.

## 5. How later slices use this crate (no change in C2)

C2 has no existing call sites to update: it adds a crate nothing uses yet.
These are the future users, so the API above is shaped for them. Do not
build any of this in C2.

| Slice | Crate and place | Uses |
| --- | --- | --- |
| C3 | `crates/core/src/dht_registry/types.rs` (`EndpointInfo.mailbox`) | `MailboxPublicKey` (serde as z32). |
| C3 | the deploy call (control plane, `roymctl`) | `MailboxSecretKey::generate`, `to_z32`, `from_z32`, `public_key`. |
| C4 | the mailbox native service | `Deposit` (the `deposit` params), `Deposit::check`, `DepositId`, `Receipt` (the `status` answer), `RejectCode::TooManyClaims`, optionally `Receipt::verify_signed` before storing an ack. |
| C5 | `crates/router/src/proxy/` (sender hook) | `seal` with the instance key and certificate found as in `router.rs:361-368`; `SealError::TooLarge` keeps a call on direct delivery; `Receipt::verify_signed` on `status` answers. |
| C6 | `crates/substrate` pull worker | `open` with `OpenPolicy` from the queue's limits; `OpenedMessage::sender_master_did` as the caller identity; `sender_instance_did` for the revocation check; `OpenError::reject_code`; `sign_receipt` for the ack. |

## 6. Completion steps

1. Import clean-up pass over every new file (types by `use`, functions by
   their parent module, no inline `a::b::c` paths).
2. `cargo +nightly-2026-04-06 fmt --all`.
3. `cargo clippy -q --workspace --all-targets --all-features -- -D warnings`.
4. `cargo nextest run -p syneroym-mailbox-format`.
5. `cargo shear`, `cargo deny check licenses`, `cargo audit` (sandbox off).
6. The duplication recipe from `AGENTS.md`; lower
   `MAX_EXACT_DUPLICATE_PERCENT` if the gate says the number improved.
7. Docs from Step 10.
8. `mise run verify` as a background job, no `--skip`. All gates must pass.
9. Commit on `feat/m06d-c2-implementation`, for example
   `feat(mailbox-format): add sealed message format`, body of one or two
   sentences. Open the PR with the CLA line from `AGENTS.md`.

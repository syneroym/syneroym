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
  **This plan names its decisions `C2-1` to `C2-17`. Those ids are for this
  plan only. When a step says to write a comment "for the reason in C2-n",
  write the reason itself in the comment, never the id.**
- `task.md` in this folder: §2 item 5 (the key algorithm), §3 slice C2, and
  slices C3 to C8 (the users of what C2 builds).
- `docs/planning/changes/service-mailbox/change.md`: criteria M-2, M-4, M-11,
  M-13, M-14, K-1, K-2, S-1, S-4, S-5, R-3, R-4, R-5, R-6, R-7; Design
  sections "Sealed message", "Mailbox key", "Mailbox methods", "Sender
  flow", "Receiver flow"; `## Deviations` (`D-C1-3`, `D-C1-4`).
- [ADR-0026](../../../decisions/0026-receiver-chosen-mailbox.md) (all),
  [ADR-0020](../../../decisions/0020-stable-logical-service-identity.md) §1
  and §3 (member master, instance key, instance certificate), and the
  `## Amendments` section of
  [ADR-0015](../../../decisions/0015-ucan-capability-model.md) (the format
  for the ADR-0026 amendment in Step 10).
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

**Branch.** Work on `feat/m06d-c2-implementation`. It was cut from `main` at
`cec9f26b` (B3 merged, #309) and holds only this plan. It is not on `origin`
yet.

C2 needs only the S0 key decision. It changes no existing crate's code. It
adds one new crate and two workspace dependencies, and one amendment to
ADR-0026.

## 1. What C2 builds, in one paragraph

A new crate `syneroym-mailbox-format` (`crates/mailbox_format`) holds the
three formats that the mailbox, the sender and the receiver share, and the
functions that make and check them: (1) the **mailbox key pair** of a service
member (X25519, as text for the service record and the deploy call); (2) the
**deposit**: the outer part the mailbox reads (destination, random deposit
id, time-to-live, sealed bytes) and the inner part only the receiver can
open (interface, method, params, idempotency key, the sender's instance
certificate, the send time, and the sender's signature), sealed with HPKE
base mode; (3) the **receipt** a sender reads back from the mailbox. A
`delivered` receipt, and a `rejected` receipt for a fault of the sender,
carry the receiver's signature over the deposit id, the sender and a hash of
the exact message, so the mailbox cannot make one and cannot reuse one for
another message. Every function is synchronous and pure: it takes `now_secs`
from the caller and does no I/O. No outbox, router, store or RPC method uses
the crate yet; C3 to C6 do.

## 2. Decisions this plan makes (check them before you start)

Each fills a gap in task.md or the change doc. All of them are decided; the
implementer does not need to ask anyone. C2-14 was confirmed by the owner on
2026-10-16. C2-13 to C2-17 come from a review of the first version of this
plan.

| # | Decision | Why |
| --- | --- | --- |
| C2-1 | New crate `crates/mailbox_format`, package `syneroym-mailbox-format`. Depends on `syneroym-identity` only (no `core`, no `rpc`, no SQLite). | Four slices use it from four crates: `core` (C3 record field), `router` (C5 sender), `substrate` (C6 receiver), the mailbox service (C4). A crate with only identity below it can be used by all four with no cycle. It must not go in `signed_record`: that crate compiles for `wasm32-wasip2` guests and promises to expose nothing that needs a private key. It must not go in the C4 service crate: the router would then link the mailbox store. |
| C2-2 | HPKE base mode, `DHKEM(X25519, HKDF-SHA256)`, `HKDF-SHA256`, `AES-256-GCM`, `info = b"syneroym-mailbox-v1"`, **empty `aad`**, through the `hpke` crate, version 0.13 (Step 1 says why not 0.14). All HPKE calls live in one private module (`src/crypto.rs`), so a later move to `hpke` 0.14 changes one file. | Fixed by task.md §2.5 and the change doc. The empty `aad` is explained in C2-4. |
| C2-3 | The sender's signature covers this JSON object, the **message statement** (canonical JSON, signed with `Identity::sign_json`): `{"domain": "syneroym-mailbox-message-v1", "destination", "deposit_id", "idempotency_key", "payload_hash", "sent_at_secs"}`. **`deposit_id` is added** to the list in the change doc. | It stops relabelling: the mailbox cannot give the receiver an existing message under another deposit id, because the receiver rebuilds the statement with the claimed id and the signature fails. On its own this does **not** stop a fake delivery; C2-13 does. Record as a deviation (Step 10). |
| C2-4 | The receiver never reads `destination` or `deposit_id` from inside the sealed part. It rebuilds the message statement from its **own** service id (the member master DID it pulls for) and the **deposit id the mailbox gave at claim**. A wrong value shows as `bad_signature`. So the HPKE `aad` adds nothing and stays empty. | One binding mechanism, not two. |
| C2-5 | `payload_hash` = lowercase hex of SHA-256 over `serde_json::to_vec(&canonicalize_json_value(&json!({"interface": i, "method": m, "params": p})))`. | Binds interface and method as well as params. The receiver recomputes it from the parsed inner part, so the form must survive a JSON round trip; a test proves it (test 40). |
| C2-6 | The outer part has **no `size` field** (the change doc lists one). Size is the length of the decoded sealed bytes. | A field that repeats a derived value is one more thing to check and to get wrong. Record as a deviation. |
| C2-7 | Text forms on the wire: mailbox public and secret keys as z-base-32 (52 characters, like DIDs and signatures in this repo); deposit id as 32 lowercase hex characters (16 random bytes); message hash as 64 lowercase hex characters; sealed bytes as standard base64 with padding (new direct dependency `base64` 0.23, the newest stable, already in `Cargo.lock` as 0.23.1 through other crates; its `Engine` and `general_purpose::STANDARD` API is the same as 0.22). | Keys go into the 1000-byte BEP44 record (K-1), so the short z32 form matters. Sealed bodies are up to 64 KiB and a phone downloads them; base64 costs 33 % where hex costs 100 %. |
| C2-8 | `MAX_SEALED_BYTES = 64 * 1024`, counted on the raw sealed bytes (32-byte HPKE `enc` + ciphertext + 16-byte tag), before base64. The same constant is S-1's "sealed size" and M-4's "message size". | One number, one place. |
| C2-9 | The mailbox secret key is the 32-byte seed of RFC 9180 `DeriveKeyPair`. The private and public X25519 keys are derived from it when needed. | Generation needs no random-number type from the `hpke` crate (only `getrandom`), and the stored form is one fixed-size value. C3 delivers this seed in the deploy call. |
| C2-10 | `open` runs its checks in two groups. **Group 1, before the message is known to be authentic:** sealed length, HPKE open, inner JSON parse, sender signature. A failure here returns `TooLarge`, `Undecryptable`, `Malformed` or `BadSignature`. **Group 2, on an authentic message:** sender certificate valid **at `sent_at_secs`** (`verify_at` with scope `service-instance`), then the time window `sent_at_secs + max_ttl_secs + max_clock_skew_secs >= now_secs` and `sent_at_secs <= now_secs + max_clock_skew_secs`. A failure here returns `OpenError::Refused`, which carries the code and the authentic message. | R-4. The future bound is not in R-4; it is added because Conversation has the same rule (`crates/conversation/src/transport.rs:320`) and an unbounded future time would make the age check useless. The two groups are what C2-14 needs: only an authentic message can get a signed rejection. Record the future bound as a deviation. |
| C2-11 | `open` does **not** check revocation. It returns the sender's instance DID and master DID; C6 checks the instance DID against the master's anchor (`MasterAnchorResolver`, as `crates/router/src/handshake.rs:66-75` does) and acks with a signed `rejected` with code `revoked`. | Revocation needs an async network lookup. Every function in this crate is pure. |
| C2-12 | The age bound takes `max_ttl_secs` as a parameter (`OpenPolicy`), default `OPEN_MAX_TTL_SECS = 7 days`; `max_clock_skew_secs` default `MAX_CLOCK_SKEW_SECS = 86_400`. | A dedicated mailbox may raise the time-to-live with the passport fact `max_ttl_secs` (ADR-0015 A10). The receiver learns its queue's limit at `register` (C4, C8) and passes it in. See §4 item 3. |
| C2-13 | **A signed receipt names the exact message.** `message_hash` = SHA-256 over the canonical JSON bytes of the message statement (C2-3). `seal` returns it to the sender inside a `SentDeposit` (deposit id, receiver service id, sender master DID, message hash), and `open` returns it to the receiver. The receipt statement, signed with the receiver's instance key, is `{"domain": "syneroym-mailbox-receipt-v1", "deposit_id", "sender", "message_hash", "state", "at_secs"}`, plus `"code"` (the raw wire string) when `state` is `"rejected"`. `sender` is the sender's member master DID. The sender checks a receipt against its own `SentDeposit` only. | Without it a dishonest mailbox can fake a delivery: drop deposit `X` of sender S2, seal **its own** new message (signed by any instance it controls) with `deposit_id = X`, let the receiver open it and sign `delivered(X)`, and show that receipt to S2. With `sender` and `message_hash` in the receipt, S2 rebuilds the statement from its own values and the signature fails. The mailbox cannot make a message with S2's hash: the hash covers the idempotency key and the payload hash, which are sealed. |
| C2-14 | **Owner-confirmed (2026-10-16), narrowed by review.** The receiver signs a `rejected` receipt only (a) for an authentic message (it has the message hash, C2-13) and (b) for a code the **sender caused**: `bad_certificate`, `from_future`, `revoked`, `not_eligible`. For every other failure (`malformed`, `undecryptable`, `bad_signature`, `too_large`, `too_old`) the receiver acks with an **unsigned** `rejected`. The mailbox's own `rejected` after 5 claims (M-14, code `too_many_claims`) is unsigned too. `sign_*` functions refuse to sign any other code, and `verify_signed` refuses a signed rejection with any other code. | S-5 says `rejected` fails the message, and the mailbox can cause most failures itself: change one byte (`undecryptable`), relabel (`bad_signature`), or hold the message back and release it late (`too_old`). If the receiver signed those, a dishonest mailbox could still make a sender drop a message for good. The four sender-caused codes come from checks on content the sender signed, or on the receiver's own rules, so the mailbox cannot cause them. The receiver acks the others (unsigned) instead of leaving them unacked, so the mailbox closes them at once instead of after 5 claims and 25 minutes. |
| C2-15 | **The sender rule for each receipt** (built in C5; written here because the format must support it): `delivered` that passes `verify_signed` → done. `rejected` that passes `verify_signed` → failed. `stored` → keep polling. **Everything else** → handled like `expired`, that is, back to direct attempts until the outbox's own attempt budget or age limit ends the message: `expired`, `unknown`, a state this version does not know, an unsigned `rejected`, and **any** `delivered` or `rejected` for which `verify_signed` returns an error (`Unsigned`, `NotSignable`, `WrongReceiver`, `BadCertificate`, `BadSignature`, `FromFuture`). | A receipt with a bad proof is either a bug or a dishonest mailbox. It must never count as delivered or as a final rejection. Treating it like `expired` is safe: the idempotency fence at the receiver removes a second copy. |
| C2-16 | **A `Deposit` is used once.** Every deposit, including a second deposit after an `expired` receipt (S-4, S-5), is a fresh `seal` call with a new deposit id and a new `sent_at_secs`. C5 keeps the plaintext call (the queued call payload already stays unchanged while parked, `D-C1-4`) and stores only the `SentDeposit` and the mailbox id in the park context (`D-C1-3`), never the sealed bytes. | A stored deposit sent again later would carry an old send time. Inside Conversation's 30-day window it would fail `too_old` at the receiver. |
| C2-17 | **Forward compatibility.** (a) Reject codes travel as plain strings; the receipt signature covers the raw string. `RejectCode` is a helper enum with `as_str`, `parse` (returns `None` for a string it does not know) and `is_sender_fault`. An unknown code is kept as text and is never sender-caused, so it is never final. (b) `Receipt` reads a `state` it does not know as `Unknown` (`#[serde(other)]`). (c) `Deposit` and the sealed inner part **ignore** unknown fields. | All three are read by nodes of other operators that may run a newer version. Ignoring an unknown field is safe in (c): every field that carries meaning is either signed (inner part) or checked by `Deposit::check` (outer part), and a change that breaks the meaning changes the `-v1` domain tags. This replaces the first version's `deny_unknown_fields` on `Deposit` and on the inner part. |

## 3. Step-by-step changes

### Step 1 — the `hpke` dependency (checked, use exactly this)

The S0 check that `hpke` fits the workspace pins was never recorded (see §4
item 6). It was done on 2026-10-16, on this branch at `6351a1d1`, by adding
`hpke` to the workspace for a moment and reverting. Results:

- **`hpke` 0.14.1 (the newest) does not resolve.** It is built on the new
  RustCrypto line (`sha2` 0.11, `hkdf` 0.13, `aes-gcm` 0.11,
  `x25519-dalek` 3, `rand_core` 0.10). iroh 0.97 pins
  `sha2 = "=0.11.0-rc.2"` exactly (through `iroh-base`), and `hpke` 0.14
  needs a stable `sha2` 0.11. Cargo stops with "failed to select a version
  for `sha2`".
- **`hpke` 0.13.0 fits.** It uses `x25519-dalek` 2, `aes-gcm` 0.10,
  `hkdf` 0.12, `sha2` 0.10 and `rand_core` 0.9, all already in
  `Cargo.lock`. With it, the only new entry in `Cargo.lock` was
  `hpke 0.13.0`. `cargo deny check licenses` passed (MIT/Apache-2.0) and
  `cargo audit` reported nothing.
- **The API names in Step 3 were compiled and run** in a throwaway test:
  `derive_keypair` from a 32-byte seed, `single_shot_seal` and
  `single_shot_open` with `AesGcm256` / `HkdfSha256` / `X25519HkdfSha256`,
  `EncappedKey::from_bytes` on 32 bytes, a ciphertext of plaintext + 16
  bytes, and a failed open with another key.
- **Random-number type:** `hpke` 0.13 re-exports `rand_core` 0.9, the same
  version as the workspace `rand = "0.9"`.
  `rand::rand_core::UnwrapErr(rand::rngs::OsRng)` compiles and works. It
  panics only if the OS random source fails. `Identity::generate` depends on
  the same source, so the node cannot run without it anyway; write that
  reason in the comment.

What to do:

1. Add to the root `Cargo.toml` `[workspace.dependencies]`, each in its
   alphabetical place:

   ```toml
   base64 = "0.23"
   # 0.14 needs stable sha2 0.11, which conflicts with iroh's exact
   # pre-release pin; move up when iroh moves to the stable crypto line.
   hpke = { version = "0.13", default-features = false, features = ["alloc", "x25519"] }
   ```

   `alloc` gives `single_shot_seal` and `single_shot_open`. `x25519` gives
   the KEM. The default `p256` feature is not wanted. AES-GCM is always
   built in 0.13 (no feature). `rand` is already a workspace dependency.
2. After Step 2, compare `Cargo.lock` with `main`:

   ```bash
   git diff main -- Cargo.lock | grep -E '^\+name = '
   ```

   Expected: only `+name = "hpke"`. If anything else appears (for example
   after a `cargo update`), stop and ask the owner. Do not change the
   sealing algorithm, and do not write HPKE by hand.

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

If `message/tests.rs` or `receipt/tests.rs` passes 800 lines, split it the
same way: `message/tests.rs` declares `mod open; mod seal;` and the tests go
into `message/tests/open.rs` and `message/tests/seal.rs`.

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
rand.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
z32.workspace = true
zeroize.workspace = true
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
pub use keys::{KeyParseError, MailboxPublicKey, MailboxSecretKey};
pub use message::{
    MessageContent, MessageHash, OpenPolicy, OpenedMessage, Recipient, RefusedMessage, Sealed,
    Sender, SentDeposit, open, seal,
};
pub use receipt::{Receipt, ReceiverProof, RejectCode, SignedOutcome};

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

type MailboxKem = hpke::kem::X25519HkdfSha256;
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

Pseudo-code. These `hpke` 0.13 names were compiled and run (Step 1). Import
the traits from the crate root: `use hpke::{Deserializable, Kem as KemTrait,
OpModeR, OpModeS, Serializable};`.

```text
public_key_from_seed(seed):
    (_sk, pk) = <MailboxKem as KemTrait>::derive_keypair(seed)
    return pk.to_bytes() as [u8; 32]

seal_to(pk_bytes, plaintext):
    pk = <MailboxKem as KemTrait>::PublicKey::from_bytes(pk_bytes)  -> map_err BadPublicKey
    rng = rand::rand_core::UnwrapErr(rand::rngs::OsRng)
          // comment: panics only if the OS random source fails; the node's
          // own key generation needs the same source
    (enc, ct) = hpke::single_shot_seal::<Aead, Kdf, MailboxKem, _>(&OpModeS::Base, &pk, HPKE_INFO, plaintext, b"", &mut rng)
                -> map_err Seal(e.to_string())
    return enc.to_bytes() ++ ct

open_with(seed, sealed):
    if sealed.len() < SEAL_OVERHEAD: return TooShort
    (enc_bytes, ct) = sealed.split_at(ENC_LEN)
    enc = <MailboxKem as KemTrait>::EncappedKey::from_bytes(enc_bytes) -> map_err Undecryptable
    (sk, _pk) = <MailboxKem as KemTrait>::derive_keypair(seed)
    return hpke::single_shot_open::<Aead, Kdf, MailboxKem>(&OpModeR::Base, &sk, &enc, HPKE_INFO, ct, b"")
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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyParseError {
    #[error("a mailbox key is z-base-32 text")]
    NotZ32,
    #[error("a mailbox key is 32 bytes, not {0}")]
    WrongLength(usize),
}
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

Do not give `MailboxSecretKey` serde impls: C3 decides how the seed travels
and is stored, and a derive would make it too easy to log.

Whether `from_bytes` must reject the all-zero or low-order X25519 points:
`hpke` 0.13 rejects an all-zero shared secret at seal time
(`src/dhkex/x25519.rs`, checked in the source), which is enough; `seal`
maps it to `SealError::Crypto`.

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
/// takes it from the verified transport identity (ADR-0016). Used once:
/// a second deposit of the same call is a new `seal`.
///
/// Unknown fields are ignored, not refused: a mailbox reads deposits from
/// senders that may run a newer version, and every field it acts on is
/// checked by `check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContent {
    pub interface: String,
    pub method: String,
    pub params: serde_json::Value,
    pub idempotency_key: String,
}

/// SHA-256 of the canonical message statement: the exact message a
/// receipt is about. 64 lowercase hex characters on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct MessageHash([u8; 32]);

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

/// What the sender keeps after a deposit, to check the receipt later. It
/// never holds the sealed bytes: a later deposit is a new `seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentDeposit {
    pub deposit_id: DepositId,
    pub receiver_service_id: String,
    pub sender_master_did: String,
    pub message_hash: MessageHash,
}

/// The result of `seal`: what goes to the mailbox, and what the sender keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub deposit: Deposit,
    pub sent: SentDeposit,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedMessage {
    pub content: MessageContent,
    pub sender_certificate: DelegationCertificate,
    pub sent_at_secs: u64,
    pub deposit_id: DepositId,
    pub message_hash: MessageHash,
}

/// An authentic message (its sender signature is good) that failed a check
/// on the sender's side: its certificate, or its send time. It must not be
/// delivered. `receipt` makes the ack for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedMessage {
    pub code: RejectCode,
    pub detail: String,
    message: OpenedMessage,
}
```

Methods on these types:

| Signature | Behaviour |
| --- | --- |
| `OpenedMessage::sender_master_did(&self) -> &str` | `&self.sender_certificate.master_did`. The caller identity of the delivered call (the original sender, never the mailbox). |
| `OpenedMessage::sender_instance_did(&self) -> &str` | `&self.sender_certificate.temporary_did`. The key to check against the master's revocation list. |
| `OpenedMessage::sign_delivered(&self, instance: &Identity, certificate: &DelegationCertificate, at_secs: u64) -> Result<Receipt, ReceiptError>` | `receipt::sign(.., SignedOutcome::Delivered, ..)` over this message. |
| `OpenedMessage::sign_rejection(&self, instance: &Identity, certificate: &DelegationCertificate, code: RejectCode, at_secs: u64) -> Result<Receipt, ReceiptError>` | For the receiver's own checks after `open` (`Revoked`, `NotEligible`). Refuses a code that is not sender-caused (`NotSignable`). |
| `RefusedMessage::sender_master_did(&self) -> &str` | For the log. |
| `RefusedMessage::receipt(&self, instance: &Identity, certificate: &DelegationCertificate, at_secs: u64) -> Result<Receipt, ReceiptError>` | Signed rejection if `self.code.is_sender_fault()` (`BadCertificate`, `FromFuture`); otherwise `Receipt::unsigned_rejection(self.code, at_secs)` (`TooOld`). |
| `MessageHash`: `Display`/`FromStr` (64 lowercase hex), `as_bytes(&self) -> &[u8; 32]` | |

Private inner part (the plaintext before sealing):

```rust
/// Unknown fields are ignored: the sender may run a newer version, and
/// every field that carries meaning is under the sender signature.
#[derive(Serialize, Deserialize)]
struct SealedContent {
    interface: String,
    method: String,
    params: serde_json::Value,
    idempotency_key: String,
    sent_at_secs: u64,
    sender_certificate: DelegationCertificate,
    /// z-base-32 Ed25519 signature over the message statement.
    signature: String,
}
```

Private helpers:

```rust
fn payload_hash(interface: &str, method: &str, params: &Value) -> Result<String, serde_json::Error>
    // C2-5: hex(sha256(to_vec(canonicalize_json_value(json!({...})))))

fn message_statement(destination: &str, deposit_id: &DepositId, idempotency_key: &str,
                     payload_hash: &str, sent_at_secs: u64) -> Value
    // json!({"domain": MESSAGE_DOMAIN, "destination": .., "deposit_id": deposit_id.to_string(),
    //        "idempotency_key": .., "payload_hash": .., "sent_at_secs": ..})

fn message_hash(statement: &Value) -> Result<MessageHash, serde_json::Error>
    // sha256(serde_json::to_vec(&canonicalize_json_value(statement)))
    // These are the same bytes `Identity::sign_json` signs.

/// Builds and seals without the sender-side checks, so tests can make
/// messages a real sender would refuse to make.
pub(crate) fn seal_unchecked(content: &MessageContent, recipient: Recipient<'_>,
                             signer: &Identity, certificate: &DelegationCertificate,
                             deposit_id: DepositId, ttl_secs: u64, sent_at_secs: u64)
                             -> Result<Sealed, SealError>
```

`seal_unchecked` has 7 parameters; `clippy::too_many_arguments` warns
above 7 (the workspace `clippy.toml` does not change it), so it passes. If
clippy still complains, group `deposit_id`, `ttl_secs` and `sent_at_secs`
into a private struct; do not add an `allow`.

**`pub fn seal(content: &MessageContent, recipient: Recipient<'_>, sender: Sender<'_>, ttl_secs: u64, now_secs: u64) -> Result<Sealed, SealError>`**

Doc comment: each call makes a new deposit id and uses `now_secs` as the
send time; to deposit the same call again, call `seal` again.

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
p_hash = payload_hash(&content.interface, &content.method, &content.params) else Json
statement = message_statement(recipient.service_id, &deposit_id, &content.idempotency_key, &p_hash, sent_at_secs)
m_hash = message_hash(&statement) else Json
signature = signer.sign_json(&statement) else Json
inner = SealedContent { content fields cloned, sent_at_secs, sender_certificate: certificate.clone(), signature }
plaintext = serde_json::to_vec(&inner) else Json
if plaintext.len() + crypto::SEAL_OVERHEAD > MAX_SEALED_BYTES:
    return TooLarge { bytes: plaintext.len() + SEAL_OVERHEAD, max: MAX_SEALED_BYTES }
sealed = crypto::seal_to(recipient.mailbox_key.as_bytes(), &plaintext) else Crypto
return Sealed {
    deposit: Deposit { destination: recipient.service_id.to_string(), deposit_id, ttl_secs, sealed: SealedBytes::new(sealed) },
    sent: SentDeposit { deposit_id, receiver_service_id: recipient.service_id.to_string(),
                        sender_master_did: certificate.master_did.clone(), message_hash: m_hash },
}
```

`TooLarge` is the signal C5 uses for S-1 ("sealed size at most 64 KiB"):
the call stays on direct delivery. `BadCertificate` from `seal` means the
sender has no valid instance certificate now; the call also stays on direct
delivery (§4 item 4).

**`pub fn open(key: &MailboxSecretKey, own_service_id: &str, deposit_id: &DepositId, sealed: &SealedBytes, policy: OpenPolicy, now_secs: u64) -> Result<OpenedMessage, OpenError>`**

```text
// group 1: before the message is known to be authentic
if sealed.len() > MAX_SEALED_BYTES: return TooLarge
plaintext = crypto::open_with(key.seed(), sealed.as_slice())
            TooShort -> Malformed("too short"); anything else -> Undecryptable
inner: SealedContent = serde_json::from_slice(&plaintext) else Malformed(e.to_string())
p_hash = payload_hash(&inner.interface, &inner.method, &inner.params) else Malformed
statement = message_statement(own_service_id, deposit_id, &inner.idempotency_key, &p_hash, inner.sent_at_secs)
verify_json_signature(&inner.sender_certificate.temporary_did, &statement, &inner.signature)
    else BadSignature
m_hash = message_hash(&statement) else Malformed
message = OpenedMessage { content: MessageContent { .. }, sender_certificate, sent_at_secs,
                          deposit_id: *deposit_id, message_hash: m_hash }

// group 2: checks on the sender's side of an authentic message
refuse(code, detail) = Err(OpenError::Refused(Box::new(RefusedMessage { code, detail, message })))
message.sender_certificate.verify_at(&message.sender_certificate.master_did,
                                     &[SCOPE_SERVICE_INSTANCE], message.sent_at_secs)
    else refuse(BadCertificate, e.to_string())
oldest = now_secs.saturating_sub(policy.max_ttl_secs.saturating_add(policy.max_clock_skew_secs))
if message.sent_at_secs < oldest: refuse(TooOld, "sent at .., oldest accepted ..")
latest = now_secs.saturating_add(policy.max_clock_skew_secs)
if message.sent_at_secs > latest: refuse(FromFuture, "sent at .., latest accepted ..")
return Ok(message)
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
- `too_old` is refused but not sender-caused: a mailbox can hold a message
  back and release it late.
- An empty idempotency key is not checked here, in `seal` or in `open`.
  Whether a call has a usable key is part of S-1 eligibility, which C5
  checks before it calls `seal`.

Split `open` into small private functions so it stays readable and under
100 lines: `decrypt_inner`, `check_signature`, `check_sender`.

### Step 7 — `src/receipt.rs`

```rust
/// Why a message was closed without delivery. On the wire a code is a
/// plain string, so a code from a newer version survives a round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectCode {
    // caused by the sender: the receiver may sign these
    BadCertificate, FromFuture, Revoked, NotEligible,
    // could be caused by the mailbox: never signed
    Malformed, Undecryptable, BadSignature, TooLarge, TooOld, TooManyClaims,
}
impl RejectCode {
    pub const ALL: [RejectCode; 10] = [/* every variant */];
    /// `"bad_certificate"`, `"from_future"`, `"revoked"`, `"not_eligible"`,
    /// `"malformed"`, `"undecryptable"`, `"bad_signature"`, `"too_large"`,
    /// `"too_old"`, `"too_many_claims"`.
    pub fn as_str(self) -> &'static str;
    /// The code for exactly one of the strings above, else `None`.
    pub fn parse(s: &str) -> Option<Self>;
    /// True only for a failure the mailbox cannot cause: a check on content
    /// the sender signed, or on the receiver's own rules. Only these may be
    /// signed, and only a signed one ends a message.
    pub fn is_sender_fault(self) -> bool;  // BadCertificate | FromFuture | Revoked | NotEligible
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
        /// A `RejectCode` string, or a code from a newer version.
        code: String,
        /// Present only for a sender-caused code the receiver signed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proof: Option<ReceiverProof>,
    },
    Expired { at_secs: u64 },
    /// The mailbox has no receipt for this id (never stored, or its receipt
    /// time ended), or it sent a state this version does not know.
    #[serde(other)]
    Unknown,
}

/// The two outcomes a receiver signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedOutcome { Delivered, Rejected(RejectCode) }
```

Do **not** add `deny_unknown_fields` to `Receipt` or `ReceiverProof`: a
sender reads them from mailboxes of other operators, which may be newer
(same rule as `crates/passport/src/wire.rs`). `#[serde(other)]` on a unit
variant is allowed in an internally tagged enum; test 55 proves it works.

Private helpers:

```rust
/// state is "delivered" or "rejected"; code only for "rejected", as the raw wire string.
fn receipt_statement(deposit_id: &DepositId, sender: &str, message_hash: &MessageHash,
                     state: &str, code: Option<&str>, at_secs: u64) -> Value
    // {"domain": RECEIPT_DOMAIN, "deposit_id", "sender", "message_hash", "state", "at_secs"}
    // plus "code": code when Some

/// Shared by `OpenedMessage::sign_delivered`, `sign_rejection` and `RefusedMessage::receipt`.
pub(crate) fn sign(instance: &Identity, certificate: &DelegationCertificate,
                   message: &OpenedMessage, outcome: SignedOutcome, at_secs: u64)
                   -> Result<Receipt, ReceiptError>
```

**`receipt::sign`**

```text
if let Rejected(code) = outcome and !code.is_sender_fault(): return NotSignable
if derive_did_key(&instance.public_key()) != certificate.temporary_did: SignerMismatch
certificate.verify_at(&certificate.master_did, &[SCOPE_SERVICE_INSTANCE], at_secs) else BadCertificate
(state, code) = match outcome { Delivered => ("delivered", None), Rejected(c) => ("rejected", Some(c.as_str())) }
statement = receipt_statement(&message.deposit_id, message.sender_master_did(), &message.message_hash, state, code, at_secs)
signature = instance.sign_json(&statement) else Json
proof = ReceiverProof { receiver_certificate: certificate.clone(), signature }
match outcome:
    Delivered      -> Receipt::Delivered { at_secs, proof }
    Rejected(code) -> Receipt::Rejected { at_secs, code: code.as_str().to_string(), proof: Some(proof) }
```

**`impl Receipt`**

| Signature | Behaviour |
| --- | --- |
| `pub fn unsigned_rejection(code: RejectCode, at_secs: u64) -> Receipt` | `Rejected { at_secs, code: code.as_str().into(), proof: None }`. For the receiver's ack of a failure the mailbox could have caused, and for the mailbox's own `too_many_claims`. |
| `pub fn verify_signed(&self, sent: &SentDeposit, max_clock_skew_secs: u64, now_secs: u64) -> Result<SignedOutcome, ReceiptError>` | See below. Call it only for `Delivered` and `Rejected`; any `Err` means "handle like `expired`" (C2-15). |

```text
verify_signed(sent, skew, now):
    (outcome, state, code_str, at_secs, proof) = match self:
        Delivered { at_secs, proof }            -> (Delivered, "delivered", None, at_secs, proof)
        Rejected { at_secs, code, proof: Some(p) } ->
            c = RejectCode::parse(code) filtered by is_sender_fault() else return NotSignable
            (Rejected(c), "rejected", Some(code.as_str()), at_secs, p)
        _ -> return Unsigned
    cert = &proof.receiver_certificate
    if cert.master_did != sent.receiver_service_id: WrongReceiver
    cert.verify_at(&sent.receiver_service_id, &[SCOPE_SERVICE_INSTANCE], at_secs) else BadCertificate
    statement = receipt_statement(&sent.deposit_id, &sent.sender_master_did, &sent.message_hash, state, code_str, at_secs)
    verify_json_signature(&cert.temporary_did, &statement, &proof.signature) else BadSignature
    if at_secs > now.saturating_add(skew): FromFuture
    Ok(outcome)
```

Every value in the rebuilt statement comes from the sender's own
`SentDeposit`, never from the receipt. That is what makes a receipt for
another message fail.

The mailbox (C4) cannot check a proof: it does not know the message hash.
It stores the receipt as it got it (M-13).

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
    // the message is not known to be authentic: the ack is unsigned
    TooLarge { bytes: usize, max: usize },
    Undecryptable,
    Malformed(String),
    BadSignature,
    // an authentic message that failed a check on the sender's side
    #[error("refused ({}): {}", .0.code.as_str(), .0.detail)]
    Refused(Box<RefusedMessage>),
}
impl OpenError {
    /// The code for the ack. Use `Receipt::unsigned_rejection` with it for
    /// every variant except `Refused`, whose `receipt` method decides.
    pub fn reject_code(&self) -> RejectCode;   // TooLarge, Undecryptable, Malformed, BadSignature, or the refused code
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReceiptError {
    Unsigned,
    /// A signed rejection with a code that is not sender-caused, or unknown.
    NotSignable,
    WrongReceiver, BadSignature, BadCertificate(String), FromFuture,
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

Give every variant an `#[error(...)]` message in simple English. No error
message may contain the message content.

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
(Context Budget) over the new files. If `Member::new` matches
`crates/passport/src/test_support.rs`, that test-only duplication is
acceptable only if the duplication gate still passes.

Test names below are the exact names to use. "Wrong key", "tamper",
"replay" and "old send time" are the four kinds task.md asks for; each is
marked. "Receiver" below is `R`, the honest sender is `S2`, and a member
the mailbox controls is `M`.

`crypto/tests.rs`:

1. `sealed_bytes_open_with_the_matching_key`
2. `sealed_bytes_do_not_open_with_another_key` (wrong key)
3. `a_changed_byte_in_the_key_part_the_body_or_the_tag_fails` (tamper: flip one bit at index 0, at index 40, and at the last index; each gives `Undecryptable`)
4. `two_seals_of_the_same_plaintext_differ`
5. `input_shorter_than_the_overhead_is_refused`
6. `a_stored_sealed_vector_still_opens` — a fixed seed (32 bytes of `0x42`), a fixed plaintext, and a sealed hex string you generate **once** with the final code and paste in. It fails if anyone changes the suite, the `info` tag or the key derivation.
7. `a_fixed_seed_gives_a_fixed_public_key` — `public_key_from_seed(&[0x42; 32])` must equal hex `ae3bf1cd87c2d2ed25af4a1a239eed04a990f00e7403e4c8065927de010fd17a`. This value was computed with `hpke` 0.13.0 `derive_keypair` during the Step 1 check. It fails if the key derivation changes, which would make every published mailbox key wrong.

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
20. `a_deposit_with_an_unknown_field_is_accepted`

`message/tests.rs` (split as Step 2 says if it grows):

21. `a_sealed_message_opens_with_its_content_sender_and_hash` (the opened `message_hash` and `deposit_id` equal the ones in `Sealed::sent`)
22. `the_mailbox_sees_no_interface_method_or_params` — serialize the `Deposit` to JSON; assert the method name, the idempotency key and a param value are not in it (M-11).
23. `opening_with_another_services_key_fails` (wrong key → `Undecryptable`)
24. `a_tampered_sealed_body_fails` (tamper → `Undecryptable`)
25. `a_message_opened_under_another_deposit_id_fails` (replay/relabel → `BadSignature`)
26. `a_message_opened_by_another_destination_fails` — same mailbox key, other `own_service_id` (replay to another service → `BadSignature`)
27. `the_same_message_opened_twice_gives_the_same_idempotency_key` (replay of one deposit: the format accepts it; the receiver's idempotency fence removes the duplicate)
28. `a_signature_by_a_key_other_than_the_certificates_fails` (`seal_unchecked` with another `Identity` → `BadSignature`)
29. `a_certificate_with_another_scope_is_refused` (`routing` scope → `Refused` with `BadCertificate`)
30. `a_certificate_expired_at_send_time_is_refused` (`seal_unchecked` with `sent_at = cert.expires_at_secs` → `Refused` with `BadCertificate`)
31. `a_certificate_expired_now_but_valid_at_send_time_is_accepted` (open with `now = cert.expires_at_secs + 3600`)
32. `a_send_time_older_than_ttl_plus_skew_is_refused` (old send time: `now = sent_at + 7d + 24h + 1` → `Refused` with `TooOld`; `now = sent_at + 7d + 24h` opens)
33. `a_larger_max_ttl_accepts_an_older_message` (`OpenPolicy { max_ttl_secs: 30d, .. }`)
34. `a_send_time_too_far_in_the_future_is_refused` (old send time, other side: `now = sent_at - 24h - 1` → `Refused` with `FromFuture`)
35. `seal_refuses_content_over_the_size_limit` (params with a 70 000-byte string → `SealError::TooLarge`)
36. `seal_refuses_an_instance_key_that_does_not_match_the_certificate`
37. `seal_refuses_an_expired_certificate` (`now = cert.expires_at_secs`)
38. `seal_refuses_a_zero_ttl_and_a_destination_that_is_not_a_did_key`
39. `inner_content_with_an_unknown_field_still_opens` (seal by hand with `crypto::seal_to` a correctly signed inner JSON with one extra field → opens)
40. `payload_hash_is_stable_across_a_json_round_trip` (params with a float `0.1`, nested objects in reverse key order, a non-ASCII key; hash, serialize, parse, hash again: equal)
41. `sealing_the_same_call_twice_gives_new_ids_and_hashes` (two `seal` calls a second apart: different `deposit_id`, different `message_hash`, both open)

`receipt/tests.rs`:

42. `a_delivered_receipt_verifies_against_the_sent_deposit` (returns `SignedOutcome::Delivered`)
43. `a_delivered_receipt_for_another_deposit_fails` (replay: verify with the `SentDeposit` of another deposit to the same receiver → `BadSignature`)
44. `a_delivered_receipt_from_another_service_fails` (other member's cert → `WrongReceiver`)
45. `a_delivered_receipt_with_a_changed_time_fails` (tamper `at_secs` → `BadSignature`)
46. `a_delivered_receipt_signed_after_the_certificate_expired_fails` (`sign_delivered` refuses it; and a hand-built one fails `verify_signed` with `BadCertificate`)
47. `a_delivery_of_a_mailbox_made_message_under_the_same_deposit_id_fails` — the attack in C2-13: `S2` seals to `R` and keeps its `SentDeposit`. `M` uses `seal_unchecked` to seal its own content to `R` with `S2`'s `deposit_id`. `R` opens `M`'s message (it opens) and signs `delivered`. `verify_signed` with `S2`'s `SentDeposit` → `BadSignature`.
48. `a_signed_rejection_for_a_sender_caused_code_verifies` (`sign_rejection(Revoked)` → `verify_signed` returns `Rejected(Revoked)`; same for `NotEligible`)
49. `a_refused_message_gets_a_signed_or_an_unsigned_receipt_by_its_code` (`RefusedMessage::receipt`: `BadCertificate` and `FromFuture` give a signed receipt that verifies; `TooOld` gives an unsigned one)
50. `no_code_the_mailbox_could_cause_can_be_signed` (`sign_rejection` with each of `Malformed`, `Undecryptable`, `BadSignature`, `TooLarge`, `TooOld`, `TooManyClaims` → `NotSignable`)
51. `a_signed_rejection_with_a_code_the_mailbox_could_cause_is_not_final` (a hand-signed `too_old` rejection with a valid proof → `verify_signed` gives `NotSignable`)
52. `a_rejection_with_a_changed_code_fails` (tamper `revoked` to `not_eligible` → `BadSignature`)
53. `an_unknown_reject_code_is_kept_and_is_never_final` (a receipt with `"code":"rate_limited"` round-trips with the same text; `verify_signed` → `NotSignable`)
54. `stored_expired_unknown_and_unsigned_rejections_are_unsigned` (`verify_signed` → `Unsigned`; the unsigned rejection made by `Receipt::unsigned_rejection`)
55. `an_unknown_receipt_state_reads_as_unknown` (`{"state":"held","at_secs":5}` → `Receipt::Unknown`)
56. `receipts_serialize_with_a_state_tag` — assert the exact JSON of each variant, for example `{"state":"stored","at_secs":5}`, `{"state":"rejected","at_secs":5,"code":"too_old"}` and `{"state":"unknown"}`. This test is the wire contract C4 and C5 build on.
57. `signing_refuses_an_instance_key_that_does_not_match_the_certificate`

`error/tests.rs`:

58. `every_reject_code_round_trips_through_its_string` (`parse(as_str(c)) == Some(c)` for every `RejectCode::ALL`; no two share a string; `parse("unknown_code") == None`)
59. `only_the_four_sender_caused_codes_are_sender_faults`
60. `every_open_error_has_its_own_reject_code` (each non-`Refused` variant maps to the code of the same name; `Refused` gives its own code)

### Step 10 — docs in the same PR

No living doc changes in this slice: no user, operator or developer can
use the crate yet, and the mailbox change doc closes out after C7/C8.
(Check once: `grep -rn "syneroym-passport" docs/developer-guide.md
docs/system-architecture.md` returns nothing today, so there is no crate
list to extend.)

1. **ADR-0026 amendment.** ADR-0026 Decision 6 says "`rejected` fails the
   message". C2-14 and C2-15 change that, so add an amendment, in the form
   ADR-0015 uses (amended in place, not superseded). Do not change the
   original Decision text. Append to
   `docs/decisions/0026-receiver-chosen-mailbox.md`:

   ```markdown
   ## Amendments

   ### A1. Signed receipts name the message; only a sender-caused rejection is final

   **Added 2026-10-16.** Decision 6 and Decision 7 change as follows.

   1. A receipt the receiver signs names the deposit id, the sender's member
      master DID and a hash of the exact message. The sender checks it
      against what it sent. So a mailbox cannot reuse a receipt for another
      message, and cannot make a delivery receipt for a message it put in
      the queue itself.
   2. The receiver signs a rejection only for a failure the mailbox cannot
      cause: the sender's certificate was not valid at the send time, the
      send time is in the future, the sender's key is revoked, or the method
      does not accept deferred delivery. For every other failure (a message
      that does not open or does not verify, or one that arrives too late)
      the receiver's rejection is unsigned. The mailbox's own rejection after
      repeated claims is unsigned too.
   3. The sender fails a message only on a signed rejection that it can
      verify. An unsigned rejection, or a receipt whose signature does not
      verify, is handled like `expired`: the sender returns to direct
      attempts until its own age limit, and seals the message again for the
      next deposit.

   Why: a mailbox can damage, delay or relabel a message, and the receiver
   would then reject it. If such a rejection were final, a mailbox run by a
   stranger could make a sender drop any message. With this rule the worst
   a mailbox can do is delay delivery.
   ```

2. `docs/planning/changes/service-mailbox/change.md`:
   - Criteria rows (the change doc is approved, not done, so its criteria
     describe what will be built and tested):
     - **M-13**: replace "the receiver's signature over `(deposit_id, state,
       time)`" with "the receiver's signature over the deposit id, the
       sender's member master DID, the message hash, the state and the
       time". Leave the rest of the row.
     - **M-14**: add at the end: "This `rejected` receipt is unsigned."
     - **S-5**: replace "A `rejected` receipt fails the message. An
       `expired` or unknown receipt (also after the receipt time) sends the
       message back to direct attempts until that age limit." with "A
       `rejected` receipt with a valid receiver signature fails the message
       (ADR-0026 A1). An `expired` or unknown receipt (also after the
       receipt time), an unsigned `rejected` receipt, or a receipt whose
       signature does not verify sends the message back to direct attempts
       until that age limit. Each new deposit is sealed again, with a new
       deposit id and send time."
     - **R-4**: add at the end: "A rejection for a send time that is too old
       is unsigned, because a mailbox can delay a message."
   - "Sender flow" diagram: keep the layout; replace the text
     `|--rejected--> failed` with `|--rejected (signed)--> failed`, and
     `'--expired / unknown--> pending` with
     `'--expired / unknown / unsigned rejected / bad proof--> pending`.

   - `## Deviations`, add:
     - `D-C2-1`: the sender signature also covers `deposit_id` (C2-3).
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
     - `D-C2-7`: receipts and rejections, ADR-0026 A1: the receipt names
       the sender and the message hash; only sender-caused rejections are
       signed and final; any other receipt is handled like `expired`
       (C2-13, C2-14, C2-15). The rows M-13, M-14, S-5, R-4 and the Sender
       flow diagram were changed to match.
     - `D-C2-8`: the `hpke` check. `hpke` 0.13 is used, not 0.14: 0.14 needs
       stable `sha2` 0.11, which conflicts with iroh 0.97's exact pin
       `sha2 = "=0.11.0-rc.2"`. 0.13 adds no crate besides itself (it uses
       `x25519-dalek` 2, `aes-gcm` 0.10, `hkdf` 0.12, `sha2` 0.10 and
       `rand_core` 0.9, already in the build). Replace nothing in the Design
       section; its "*Unverified:*" sentence is answered here.
     - `D-C2-9`: a deposit is sealed once and used once; a second deposit
       is a new `seal` (C2-16).
     - `D-C2-10`: reject codes are plain strings; unknown states read as
       `unknown`; deposits and the inner part ignore unknown fields (C2-17).
   - Tick `- [x] Sealed message format with tests ...` in `## Tasks`.
3. `task.md`:
   - Header status line: add C2 to the done list.
   - Under slice C2 add: "Done <date>. Decisions `D-C2-1` to `D-C2-10` are
     in the [mailbox change doc](../../changes/service-mailbox/change.md),
     with [ADR-0026 A1](../../../decisions/0026-receiver-chosen-mailbox.md#amendments)."
   - Add these bullets to the later slices (they are work for those slices,
     not deferrals, so they go here and not in the backlog; B1 and B2 did
     the same for C8):
     - **C4:** "Key receipts on `(sender, deposit_id)`, or refuse a
       `deposit_id` that already exists: the sender picks the id, so a
       second sender could reuse it. Store a receiver's ack as it came: the
       mailbox cannot check the proof, because it does not know the message
       hash. Its own rejection after 5 claims is
       `Receipt::unsigned_rejection(TooManyClaims, ..)`."
     - **C5:** "Eligibility (`S-1`) also needs a valid instance certificate:
       a `SealError::BadCertificate` or `SealError::TooLarge` from `seal`
       keeps the call on direct delivery. Each deposit is a new `seal` call;
       keep the plaintext call and store only the `SentDeposit` and the
       mailbox id in the park context (`D-C2-9`). Handle receipts by the
       rule in ADR-0026 A1: only a `verify_signed` result fails or completes
       a message; everything else goes back to direct attempts."
     - **C6:** "Check revocation after `open` and ack with
       `OpenedMessage::sign_rejection(Revoked, ..)` (`D-C2-5`). Ack every
       `open` failure: `RefusedMessage::receipt` for `OpenError::Refused`,
       `Receipt::unsigned_rejection(e.reject_code(), ..)` for the rest. Pass
       the queue's own `max_ttl_secs` from `register` in `OpenPolicy`, and
       size the idempotency memory (`R-7`) from the same value, not from a
       7-day constant."
     - **C8:** "Cap the passport fact `max_ttl_secs`: the receiver's
       idempotency memory (`R-7`) grows with it."
4. `status.md`: replace the `C2 to C8` row with a `C2` row (with evidence:
   the test files and counts, and the verify result) and a `C3 to C8 | Not
   started` row.
5. `docs/planning/deferred-backlog.md`: add one row under the mailbox
   theme: "`hpke` is held at 0.13" — 0.14 conflicts with iroh's exact
   pre-release `sha2` pin; move to 0.14 when iroh uses the stable crypto
   line (the same time the `deps:update` pins in `mise.toml` go away).
   Target `TBD`. Source: the root `Cargo.toml` `hpke` line. Add any other
   row only for something this slice really defers.
6. `mise.toml` `deps:update` description: add one sentence that `hpke`
   stays at 0.13 for the same reason, so the person who removes the iroh
   pins also moves `hpke`.

## 4. Things that are ambiguous or stale in task.md and the change docs

1. **`rejected` can be caused or forged by the mailbox (decided, C2-14,
   C2-15).** S-5 makes `rejected` final. M-14 lets the mailbox produce
   `rejected` itself, and the mailbox can also make the receiver reject a
   message by damaging, relabelling or delaying it. ADR-0026 says the
   mailbox "cannot invent a delivery"; it does not say whether it may cause
   a rejection. Decided: only a signed rejection for a sender-caused code is
   final. This changes ADR-0026 Decision 6, so Step 10 adds amendment A1.
2. **Fake delivery and relabelling (decided, C2-3 and C2-13).** The change
   doc's list of signed fields lets a dishonest mailbox turn the receiver's
   `delivered` for one message into a receipt for another. Signing
   `deposit_id` in the message stops relabelling of an existing message.
   It does not stop a mailbox that seals its own message under another
   sender's deposit id; naming the sender and the message hash in the
   receipt does.
3. **"Longest mailbox time-to-live" in R-4 and R-7 is not one number.** The
   open limit is 7 days, but the passport fact `max_ttl_secs` (ADR-0015 A10)
   lets a dedicated mailbox allow more. C2 makes it a parameter (C2-12).
   C6 must pass the queue's limit from `register`, and R-7 (C6) must size
   the idempotency memory from the same value. C8 should cap
   `max_ttl_secs` so that memory stays bounded; nothing caps it today.
   Step 10 writes both into task.md.
4. **A sender without an instance certificate cannot seal.** The inner part
   carries "the sender's instance certificate". A service deployed without
   a member master has none (`EndpointRegistry::instance_cert` returns
   `None`, `crates/core/src/local_registry.rs:337`), and a node-level call
   (`CallOrigin::Native { service_id: None }`) has none either. S-1 does not
   list this. Step 10 adds it to C5 in task.md.
5. **"Replay" in the C2 test list was not defined.** This plan reads it as:
   the same sealed bytes under another deposit id or another destination
   (refused), a receipt for another deposit (refused), and the same deposit
   opened twice (accepted by the format; the receiver's idempotency fence,
   R-2/R-7 in C6, removes the duplicate).
6. **S0 is marked done, but the `hpke` version check was never recorded.**
   task.md §2.5 says "Check in S0", status.md says S0 is done, and the
   change doc still says "*Unverified:*". Nothing in the repo mentions
   `hpke` outside these two docs, and `hpke` is not in `Cargo.lock`. The
   check was done for this plan (Step 1). Result: task.md's expectation
   ("`hpke` fits") holds only for `hpke` 0.13; the newest, 0.14, does not
   resolve against iroh's pins.
7. **"Outer: ... size" (change doc, Sealed message).** Dropped (C2-6).
8. **No future bound on the send time in R-4.** Added (C2-10).
9. **The change doc says the receiver checks "not revoked now".** That
   needs the master anchor lookup, so it lives in C6, not in the format
   (C2-11).
10. **task.md §1 "Facts" are still true for C2:** `x25519-dalek` 2.0.1 is in
    `Cargo.lock` through `vodozemac`; the router handshake is P-256 ECDH and
    not reusable; no sealed box exists in the workspace.
11. **Deposit ids are picked by the sender**, so two senders can use the
    same one. C4 must key receipts on `(sender, deposit_id)` or refuse a
    repeated id. Step 10 adds this to C4 in task.md.
12. **The re-deposit rule was not written anywhere (decided, C2-16).**
    S-4 and S-5 send a message back to direct attempts and deposit it
    again later, but nothing said whether the second deposit reuses the
    first. Reusing it would fail `too_old` inside Conversation's 30-day
    window.
13. **What a receipt with a bad proof means was not written (decided,
    C2-15).** It is handled like `expired`.

## 5. How later slices use this crate (no change in C2)

C2 has no existing call sites to update: it adds a crate nothing uses yet.
These are the future users, so the API above is shaped for them. Do not
build any of this in C2.

| Slice | Crate and place | Uses |
| --- | --- | --- |
| C3 | `crates/core/src/dht_registry/types.rs` (`EndpointInfo.mailbox`) | `MailboxPublicKey` (serde as z32). |
| C3 | the deploy call (control plane, `roymctl`) | `MailboxSecretKey::generate`, `to_z32`, `from_z32`, `public_key`. |
| C4 | the mailbox native service | `Deposit` (the `deposit` params), `Deposit::check`, `DepositId`, `Receipt` (the `status` answer and the stored ack), `Receipt::unsigned_rejection(RejectCode::TooManyClaims, ..)`. It cannot check proofs. |
| C5 | `crates/router/src/proxy/` (sender hook) | `seal` with the instance key and certificate found as in `router.rs:361-368`; `SealError::TooLarge` and `SealError::BadCertificate` keep a call on direct delivery; `SentDeposit` in the park context; `Receipt::verify_signed` on `status` answers, with the rule in C2-15. |
| C6 | `crates/substrate` pull worker | The flow below. |

The receiver flow C6 builds on this API:

```text
match open(key, own_id, deposit_id, sealed, policy_from_register, now):
    Ok(m):
        if revoked(m.sender_instance_did()):  ack m.sign_rejection(Revoked)
        elif method not eligible:            ack m.sign_rejection(NotEligible)
        else dispatch as m.sender_master_did():
            ok                 -> ack m.sign_delivered(..)
            temporary failure  -> no ack (the mailbox closes it after 5 claims, unsigned)
    Err(OpenError::Refused(r)): ack r.receipt(..)          // signed or unsigned by its code
    Err(e):                     ack Receipt::unsigned_rejection(e.reject_code(), now)
```

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

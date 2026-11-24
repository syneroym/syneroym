# End-to-End P2P Identity Binding & Stateless TEE Verification (UIDAI Offline e-KYC XML)

This walkthrough covers the complete flow—from downloading the UIDAI Paperless Offline e-KYC archive, binding it to a local P2P keypair, verifying inside a stateless Trusted Execution Environment (TEE) with liveness nonces, and returning a signed attestation to a verifier with **Name** plus **non-PII / coarse attributes** (such as `verified: true/false`, `yob` or age bracket, `state` or coarse region, and liveness match) while keeping sensitive PII (full address, photo, mobile/email hashes, and raw XML) out of the verifier's hands.

---

## Phase 1: Download & Unpack the UIDAI Offline e-KYC Archive (Prover Device)

1. **Download from UIDAI:** The user downloads the **Aadhaar Paperless Offline e-KYC** ZIP package from UIDAI using their OTP/share code workflow and sets a 4-character **share code / passphrase**.
2. **Local Extraction:** On the prover's device, extract the ZIP using the share code to obtain the UIDAI-signed XML file (`OfflinePaperlessKyc`).
3. **Compute Canonical Digest:** Hash the raw signed XML bytes locally:
```text
xml_sha256 = sha256(raw_xml_bytes)

```


4. **Extract Reference Metadata:** Parse `referenceId` (which embeds the last four digits of the Aadhaar number followed by a `YYYYMMDDHHMMSSmmm` timestamp) so local policies can check snapshot freshness if desired.

---

## Phase 2: Generate P2P Identity Keypair & Self-Signed Binding Cert (Prover Device)

1. **Generate P2P Keypair:** Generate a fresh P2P identity keypair (`sk_p2p`, `pk_p2p`) locally (e.g., Ed25519). `sk_p2p` never leaves the device.
2. **Construct Binding Statement:** Create a canonical binding record tying `pk_p2p` to `xml_sha256`:
```json
{
  "binding_version": "1.0",
  "p2p_pubkey_b64": "<base64 pk_p2p>",
  "aadhaar_xml_sha256": "<hex sha256(raw_xml_bytes)>",
  "bound_at_unix": 1760000000
}

```


3. **Sign Binding Statement:** Sign the canonicalized binding record with `sk_p2p` to produce `binding_sig_b64`.
4. **Generate Self-Signed Certificate:** Generate a self-signed X.509 certificate (`self_signed_cert_b64`) containing `pk_p2p` as the subject public key and embedding/referencing the binding hash or `binding_sig_b64` in an extension/metadata field, signed by `sk_p2p`.

---

## Phase 3: Remote Attestation & Liveness Nonce Handshake

To keep the prover, stateless TEE enclave, and verifier synchronized without trusting a central operator:

1. **Verifier Attests the TEE:** The verifier fetches the TEE's hardware attestation quote (e.g., AWS Nitro PCRs, Intel TDX MRTD/RTMRs, or AMD SEV-SNP measurement) and confirms:
* Hardware root of trust is valid.
* Debug/test modes are disabled.
* The measured enclave image matches an approved **reproducible build** from the published source repository.
* The attestation binds an enclave signing key (`pk_tee` / `sk_tee`).


2. **Nonce Exchange:**
* The **Verifier** generates a high-entropy `verifier_nonce` (and optional `expires_at_unix`).
* The **TEE** issues a short-lived `tee_nonce` for the active encrypted session (optionally bound to `verifier_nonce`).


3. **Prover Live Signature:** The prover signs a domain-separated liveness payload using `sk_p2p`:
```text
domain   = "syneroym-tee-liveness-v1"
msg      = domain || utf8(verifier_nonce) || utf8(tee_nonce) || sha256(raw_xml_bytes) || pk_p2p
sig_live = sign(sk_p2p, msg)

```



---

## Phase 4: Submit to the Stateless TEE Enclave

Over an encrypted channel (TLS or Noise) directly to the attested enclave endpoint, the prover sends:

```json
{
  "protocol_version": "1.0",
  "aadhaar_xml_b64": "<base64 raw_xml_bytes>",
  "binding": {
    "p2p_pubkey_b64": "<base64 pk_p2p>",
    "self_signed_cert_b64": "<base64 self-signed cert>",
    "binding_payload": {
      "binding_version": "1.0",
      "p2p_pubkey_b64": "<base64 pk_p2p>",
      "aadhaar_xml_sha256": "<hex sha256(raw_xml_bytes)>",
      "bound_at_unix": 1760000000
    },
    "binding_sig_b64": "<base64 binding signature>"
  },
  "liveness": {
    "verifier_nonce": "<nonce from verifier>",
    "tee_nonce": "<nonce from TEE>",
    "liveness_sig_b64": "<base64 sig_live>"
  },
  "requested_claims": {
    "include_name": true,
    "include_yob_or_adult_flag": true,
    "include_state_only": true
  }
}

```

---

## Phase 5: In-Enclave Verification (Zero Persistence, In-Memory Only)

The TEE processes the payload strictly in volatile memory, avoids disk/swap/logging, and zeroizes buffers on completion:

1. **Nonce Validation:** Verify `tee_nonce` is valid and unexpired for the session, and `verifier_nonce` meets format/length expectations.
2. **Self-Signed Cert Check:** Verify `self_signed_cert_b64` is validly self-signed, within its validity dates, and matches `p2p_pubkey_b64`.
3. **UIDAI Signature Check:** Parse `OfflinePaperlessKyc`, verify UIDAI's digital signature against UIDAI's pinned public certificate, and compute `actual_xml_sha256 = sha256(raw_xml_bytes)`.
4. **Binding Verification:** Confirm `binding_payload.aadhaar_xml_sha256 == actual_xml_sha256` and verify `binding_sig_b64` under `pk_p2p`.
5. **Liveness Proof-of-Possession:** Verify `liveness_sig_b64` under `pk_p2p` over `(domain || verifier_nonce || tee_nonce || actual_xml_sha256 || pk_p2p)`.
6. **Claim Projection:** Extract only the requested minimal fields—such as `name`, coarse age/YOB or `is_adult` boolean, and `state`—while dropping street address, PIN code, photo, mobile/email hashes, and the raw XML.

---

## Phase 6: Signed Verdict Token & Verifier Verification

The TEE signs a compact verdict with `sk_tee` and returns it (via the prover or directly to the verifier):

```json
{
  "verdict_version": "1.0",
  "verified": true,
  "reason_code": "OK",
  "p2p_pubkey_b64": "<base64 pk_p2p>",
  "p2p_pubkey_fingerprint": "<sha256 of pk_p2p>",
  "verifier_nonce": "<echoed verifier_nonce>",
  "tee_nonce": "<echoed tee_nonce>",
  "issued_at_unix": 1760000005,
  "expires_at_unix": 1760000125,
  "tee_measurement": "<expected enclave measurement digest>",
  "tee_pubkey_b64": "<base64 pk_tee>",
  "claims": {
    "name": "<Name from verified XML>",
    "state": "<State from verified XML>",
    "is_adult": true
  },
  "verdict_sig_b64": "<signature over canonical verdict body using sk_tee>"
}

```

### How the Verifier Convinces Itself

1. **Trust the TEE Measurement:** Confirm `tee_measurement` matches the verifier's allowlisted reproducible build hash and that the attestation quote shows hardware root-of-trust validity with debug mode disabled.
2. **Verify the Verdict Signature:** Validate `verdict_sig_b64` using `tee_pubkey_b64` (which is bound to the attested enclave).
3. **Check Freshness & Nonce:** Confirm `verified == true`, `verifier_nonce` matches the challenge the verifier issued, and the current time is before `expires_at_unix`.
4. **Bind to the P2P Peer:** Match `p2p_pubkey_b64` / `p2p_pubkey_fingerprint` (and the prover's self-signed cert) to the P2P identity making the request, and inspect the attested `claims` (such as `name`, `state`, and `is_adult`) without ever needing to see the underlying XML or full address.
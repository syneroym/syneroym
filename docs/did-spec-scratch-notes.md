Below is a **clean, implementation-ready spec** for your node identity + controller model, including CLI, data structures, validation, and boot flow.

---

# 📜 Node Identity & Controller Agreement Spec (v0.1)

---

# 1. 🧠 Overview

This spec defines:

* Node identity (DID derived from key)
* Controller relationship (owner of node)
* ControllerAgreement (mutual, signed)
* Boot-time configuration & validation

---

# 2. 🔑 Definitions

### 2.1 Node DID

* MUST be a `did:key`
* MUST be derived from node public key

---

### 2.2 Controller DID

* MUST be a valid DID (initially `did:key`)
* Represents owner identity

---

### 2.3 Controller Agreement

* A signed object binding:

  * controller DID
  * node DID
* MUST be signed by:

  * controller key
  * node key

---

# 3. 📦 Data Structures

## 3.1 ControllerAgreement

```json
{
  "type": "ControllerAgreement",
  "controlled": "<node DID>",
  "controller": "<controller DID>",
  "issuedAt": "<RFC3339 timestamp>",
  "expiresAt": "<RFC3339 timestamp | optional>",
  "proof": [
    {
      "type": "Ed25519Signature2020",
      "verificationMethod": "<controller DID>#<key-id>",
      "proofPurpose": "assertionMethod",
      "proofValue": "<signature>"
    },
    {
      "type": "Ed25519Signature2020",
      "verificationMethod": "<node DID>#<key-id>",
      "proofPurpose": "assertionMethod",
      "proofValue": "<signature>"
    }
  ]
}
```

---

## 3.2 Node State

```text
NodeIdentityState {
  did: DID,
  controller: Option<DID>,
  status: VERIFIED | UNVERIFIED | NONE
}
```

---

# 4. ⚙️ CLI Specification

## 4.1 Required Inputs

### `--key <path>`

* Path to node private key
* MUST exist or be creatable
* Used to derive node DID

---

## 4.2 Optional Inputs

### `--agreement <path>`

* Path to ControllerAgreement JSON
* If present → MUST be validated at boot

---

### `--controller <did>`

* Candidate controller DID
* Used only if agreement is NOT provided
* MUST be treated as UNVERIFIED

---

### `--require-agreement`

* If set:

  * Node MUST NOT start without valid agreement


## 4.3 Equivalent config.toml keys
Config keys equivalent to argument are added to the config.sample.toml file. Add comments for those
---

---

# 5. 🚀 Boot Flow Specification

## Step 1 — Load / Generate Key

```text
IF key exists:
  load key
ELSE:
  generate key
```

---

## Step 2 — Derive Node DID

```text
node_did = did:key(public_key)
```

---

## Step 3 — Agreement Handling

### Case A — Agreement provided

```text
load agreement
```

#### Validate:

1. ✔ Controlled DID matches node

```text
agreement.controlled == node_did
```

2. ✔ Controller signature valid

```text
resolve(controller DID) → pubkey
verify(signature)
```

3. ✔ Node signature valid

```text
verify using node key
```

4. ✔ (Optional) expiry check

```text
now < expiresAt
```

---

### If ALL checks pass:

```text
state.controller = agreement.controller
state.status = VERIFIED
```

---

### If ANY check fails:

```text
IF --require-agreement:
  FAIL boot
ELSE:
  continue as UNVERIFIED
```

---

## Case B — No agreement, controller provided

```text
state.controller = controller DID
state.status = UNVERIFIED
```

---

## Case C — Neither provided

```text
state.controller = None
state.status = NONE
```

---

# 6. 🔐 Verification Rules

## 6.1 DID Resolution

For `did:key`:

```text
decode DID → extract public key
```

No network calls allowed.

---

## 6.2 Signature Verification

* MUST verify against:

  * canonical serialized payload
* MUST reject invalid or malformed proofs

---

## 6.3 Agreement Precedence

```text
agreement > controller flag
```

Controller from agreement MUST override CLI input.

---

# 7. 🔄 Runtime Behavior

## 7.1 VERIFIED

* Controller has full authority
* Node may:

  * accept commands
  * sign on behalf of ownership

---

## 7.2 UNVERIFIED

* Controller is only a hint
* Node MUST NOT:

  * grant privileged access
  * assume ownership

---

## 7.3 NONE

* Node operates standalone

---

# 8. 🔁 Controller Update (Runtime API)

### Endpoint:

```text
POST /controller/agreement
```

### Behavior:

1. Validate agreement (same rules as boot)
2. If valid:

```text
state.controller = agreement.controller
state.status = VERIFIED
```

---

# 9. ⚠️ Security Requirements

* MUST NOT trust controller DID without signature
* MUST verify both signatures in agreement
* MUST ensure node DID matches agreement
* MUST NOT allow key mismatch
* MUST reject stale/expired agreements (if expiry used)

---

# 10. 💡 Implementation Notes

* DID Document is NOT required for `did:key`
* Public key MUST be derived from DID
* Agreement JSON MUST be canonicalized before signing
* Node private key MUST never be shared


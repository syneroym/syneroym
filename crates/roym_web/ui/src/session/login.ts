// The `delegated-key` browser login (ADR-0024 §4a).
//
// The person's master key never enters the browser. `roymctl session
// delegate` mints a short-lived `DelegationCertificate` for a fresh
// temporary key pair and writes `session-key.json`. The Hub imports that
// file once, keeps the temporary private key in IndexedDB as a
// non-extractable WebCrypto `CryptoKey` (page JS can sign with it, never
// read it), signs the auth service's challenge, and posts the login.

const SESSION_ENDPOINT = "/_syneroym/session";

const DB_NAME = "roym-hub-session";
const DB_STORE = "delegated-key";
const DB_RECORD = "current";

/** One `session-key.json` produced by `roymctl session delegate`. */
export interface SessionKeyBundle {
  temp_secret_key_hex: string;
  temp_did: string;
  certificate: DelegationCertificate;
}

export interface DelegationCertificate {
  master_did: string;
  temporary_did: string;
  expires_at_secs: number;
  [k: string]: unknown;
}

interface StoredKey {
  key: CryptoKey; // non-extractable Ed25519 private key
  temp_did: string;
  certificate: DelegationCertificate;
}

// z-base-32, matching the Rust `z32` crate: a big-endian 5-bit bitstream
// over this alphabet, the trailing partial group left-aligned.
const Z32_ALPHABET = "ybndrfg8ejkmcpqxot1uwisza345h769";

export function z32Encode(data: Uint8Array): string {
  let value = 0;
  let bits = 0;
  let out = "";
  for (const byte of data) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      out += Z32_ALPHABET[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) {
    out += Z32_ALPHABET[(value << (5 - bits)) & 31];
  }
  return out;
}

// Wrap a raw 32-byte Ed25519 seed in the fixed PKCS#8 framing WebCrypto's
// `importKey("pkcs8", ...)` requires (RFC 8410); the prefix is constant for
// every Ed25519 private key.
const PKCS8_ED25519_PREFIX = new Uint8Array([
  0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04,
  0x22, 0x04, 0x20,
]);

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.trim();
  if (clean.length % 2 !== 0) throw new Error("odd-length hex string");
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

async function importTempKey(secretKeyHex: string): Promise<CryptoKey> {
  const seed = hexToBytes(secretKeyHex);
  if (seed.length !== 32) throw new Error("temporary secret key must be 32 bytes");
  const pkcs8 = new Uint8Array(PKCS8_ED25519_PREFIX.length + 32);
  pkcs8.set(PKCS8_ED25519_PREFIX, 0);
  pkcs8.set(seed, PKCS8_ED25519_PREFIX.length);
  return crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, false, [
    "sign",
  ]);
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(DB_STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function putStoredKey(record: StoredKey): Promise<void> {
  const db = await openDb();
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(DB_STORE, "readwrite");
    tx.objectStore(DB_STORE).put(record, DB_RECORD);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
  db.close();
}

async function getStoredKey(): Promise<StoredKey | null> {
  let db: IDBDatabase;
  try {
    db = await openDb();
  } catch {
    return null;
  }
  const record = await new Promise<StoredKey | null>((resolve) => {
    const tx = db.transaction(DB_STORE, "readonly");
    const req = tx.objectStore(DB_STORE).get(DB_RECORD);
    req.onsuccess = () => resolve((req.result as StoredKey) ?? null);
    req.onerror = () => resolve(null);
  });
  db.close();
  return record;
}

export async function clearStoredKey(): Promise<void> {
  let db: IDBDatabase;
  try {
    db = await openDb();
  } catch {
    return;
  }
  await new Promise<void>((resolve) => {
    const tx = db.transaction(DB_STORE, "readwrite");
    tx.objectStore(DB_STORE).delete(DB_RECORD);
    tx.oncomplete = () => resolve();
    tx.onerror = () => resolve();
  });
  db.close();
}

/** Whether a delegated key is already held in this browser. */
export async function hasStoredKey(): Promise<boolean> {
  return (await getStoredKey()) !== null;
}

async function completeLogin(stored: StoredKey): Promise<void> {
  const masterDid = stored.certificate.master_did;

  const challengeRes = await fetch(`${SESSION_ENDPOINT}/challenge`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ master_did: masterDid }),
  });
  if (!challengeRes.ok) {
    throw new Error(`challenge failed: HTTP ${challengeRes.status}`);
  }
  const challenge: { nonce: string; assertion?: string } = await challengeRes.json();
  if (!challenge.assertion) {
    throw new Error("auth service did not return an assertion to sign");
  }

  const signature = new Uint8Array(
    await crypto.subtle.sign(
      { name: "Ed25519" },
      stored.key,
      new TextEncoder().encode(challenge.assertion),
    ),
  );

  const loginRes = await fetch(`${SESSION_ENDPOINT}/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      method: "delegated-key",
      temp_did: stored.temp_did,
      delegation: stored.certificate,
      nonce: challenge.nonce,
      signature: z32Encode(signature),
    }),
  });
  if (!loginRes.ok) {
    let message = `HTTP ${loginRes.status}`;
    try {
      const body = await loginRes.json();
      if (body?.error) message = String(body.error);
    } catch {
      // keep the status-code message
    }
    throw new Error(message);
  }
}

/** Import a `session-key.json` bundle, persist its key, and log in. */
export async function loginWithBundle(bundle: SessionKeyBundle): Promise<void> {
  const key = await importTempKey(bundle.temp_secret_key_hex);
  const stored: StoredKey = {
    key,
    temp_did: bundle.temp_did,
    certificate: bundle.certificate,
  };
  await completeLogin(stored);
  await putStoredKey(stored);
}

/** Log in again with the delegated key already held in this browser. */
export async function loginWithStoredKey(): Promise<boolean> {
  const stored = await getStoredKey();
  if (!stored) return false;
  await completeLogin(stored);
  return true;
}

export interface SessionMethods {
  methods: string[];
}

export async function fetchMethods(): Promise<SessionMethods> {
  const res = await fetch(`${SESSION_ENDPOINT}/methods`);
  if (!res.ok) throw new Error(`methods failed: HTTP ${res.status}`);
  return res.json();
}

export async function logout(): Promise<void> {
  try {
    await fetch(`${SESSION_ENDPOINT}/logout`, { method: "POST" });
  } finally {
    await clearStoredKey();
  }
}

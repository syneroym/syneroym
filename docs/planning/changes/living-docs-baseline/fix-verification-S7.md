# Fix verification, batch S7

Reader: the author who applies the stage 2 fixes to `docs/system-architecture.md`.

Batch S7 covers fix commit 18 (Layer 3 discovery, messaging, reputation, payments; 27 rows) and fix commit 19 (identity, master anchor, signed records; 25 rows). All work was static reading. Doc line numbers are those of `docs/system-architecture.md` on branch `docs/architecture-fix`.

## Summary

| Verdict | Commit 18 | Commit 19 | Total |
| --- | --- | --- | --- |
| CONFIRMED | 24 | 22 | 46 |
| CITE-OFF | 0 | 0 | 0 |
| PARTLY | 3 | 3 | 6 |
| WRONG | 0 | 0 | 0 |
| UNVERIFIABLE | 0 | 0 | 0 |
| Rows | 27 | 25 | 52 |

Several cites are off by a few lines (for example `client.rs:266-270` is really `:266-269`). I counted those as CONFIRMED.

## Rows

### Commit 18

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 18.1 | CONFIRMED | Registry first, DHT fallback, backfill only if a registry URL is set. A bad registry answer stops the lookup. | `crates/core/src/dht_registry/client.rs:203-270` |
| 18.2 | CONFIRMED | Listing is the only type in `directory.publish`; filters are text, categories, area, open_to, booking_mode. | `crates/roym_directory/src/app/publication_ops.rs:123-146`, `crates/roym_core/src/directory.rs:169-187`, `record.rs:18-31` |
| 18.3 | PARTLY | Checks are real, but the revocation status is always `unknown`: no revocation list is ever passed. | `crates/roym_core/src/listing.rs:505-545`, `crates/signed_record/src/verify.rs:86-95,237-244` |
| 18.4 | CONFIRMED | Sort by `issued_at_secs` desc, then `listing_id`; round-robin merge; no score field. | `crates/roym_directory/src/app/search_ops.rs:210-214`, `client_merge.rs:96-141` |
| 18.5 | CONFIRMED | Host store holds log, outbox, search, export; inbox returns Accept, Hold or Drop. | `crates/rpc/src/conversation.rs:98-110`, `crates/roym_conversation/src/app/inbox.rs:159-260` |
| 18.6 | PARTLY | vodozemac does Olm (triple DH + Double Ratchet), not X3DH. AES-256-GCM per epoch and no openmls/libsignal are right. | vodozemac 0.10.0 `src/olm/shared_secret.rs:15-26`; `crates/conversation/src/dag.rs:265-290`; `Cargo.toml:162` |
| 18.7 | CONFIRMED | `prekey-bundle` call to the peer; default 20 per caller per clock hour. | `crates/conversation/src/lib.rs:247-262`, `store/session.rs:158-174`, `host_impl.rs:482`, `core/src/config/sandbox.rs:344` |
| 18.8 | PARTLY | Three states are right. A pending message becomes `failed` after 30 days or on a terminal refusal. | `crates/conversation/src/outbox.rs:195-205`, `core/src/config/sandbox.rs:210-214,334` |
| 18.9 | CONFIRMED | Owner-only; cap 256 on add; new random key per change, sent as `group-key` via direct session; removed member excluded. | `crates/conversation/src/group.rs:218-361`, `dag.rs:13` |
| 18.10 | CONFIRMED | `membership` entry is signed and carries `new_epoch` and `member_list_hash`. | `crates/conversation/src/dag.rs:15-37,60-90` |
| 18.11 | CONFIRMED | Scheduled rekey runs from the outbox tick; default 604800 s. | `crates/conversation/src/group.rs:560-580`, `outbox.rs:46`, `core/src/config/sandbox.rs:229,350` |
| 18.12 | CONFIRMED | Inbox calls `contacts.admit-first-contact`; defaults 3 per 24 h; bounds 60 s to 30 d and 0 to 1000. | `crates/roym_conversation/src/app/inbox.rs:186-203`, `crates/roym_core/src/safety.rs:19-45` |
| 18.13 | CONFIRMED | Rate-limited, blocked: Drop. Hidden group: Hold `group-hidden`. | `crates/roym_conversation/src/app/inbox.rs:176-224,252-258` |
| 18.14 | CONFIRMED | Blocks and reports live in `profile`; directory limits default 20 per 24 h. | `crates/roym_profile/src/app.rs:28,78-85`, `crates/roym_core/src/safety.rs:56-90`, `directory.rs:284` |
| 18.15 | CONFIRMED | Body zeroed, row kept with `deleted_at`, FTS entry removed, `secure_delete = ON`, scrub merges FTS and truncates WAL. | `crates/conversation/src/store/message.rs:362-436`, `schema.rs:253`, `scrub.rs:80-115` |
| 18.16 | CONFIRMED | Author and conversation must match, in direct and group paths. Group log tables untouched. | `crates/conversation/src/store/message.rs:439-462`, `group/entry.rs:218-240`, `dag.rs:14` |
| 18.17 | CONFIRMED | FTS5 trigram; accepted or outgoing, non-deleted. Under 3 characters it uses LIKE. | `crates/conversation/src/store/search.rs:1-47`, `roym_conversation/src/app/backup.rs:35` |
| 18.18 | CONFIRMED | No thread, reply-to or collaborative-edit code in either crate or the WIT. | rg over `crates/conversation/src`, `crates/roym_conversation/src`, `wit/conversation` |
| 18.19 | CONFIRMED | Each half is its own envelope, issuer matches role; pair state is computed from two halves. | `crates/roym_core/src/transaction.rs:204-237`, `fulfilment.rs:25-40` |
| 18.20 | CONFIRMED | Three record types; credential lifetime capped at two years at issue; suspend and lift. | `crates/roym_core/src/membership.rs:18-90`, `roym_directory/src/app/credential_ops.rs:156` |
| 18.21 | CONFIRMED | `listed()` needs a valid membership; pin kept on re-add; a reply never re-pins. | `crates/roym_directory/src/app/search_ops.rs:444-468`, `client_sources.rs:27-31,141-146`, `held.rs:136-153` |
| 18.22 | CONFIRMED | Notice text in core and Hub; wire table has four open or verified methods only. | `crates/roym_core/src/membership.rs:29-34`, `crates/roym_directory/src/app.rs:75-78,184-186` |
| 18.23 | CONFIRMED | Local block verbs in `profile`; manifest verified with signature check. | `crates/roym_profile/src/app.rs:78-81`, `crates/roym_core/src/backup.rs:192-200` |
| 18.24 | CONFIRMED | No vouch, `ReputationRecord`, rating or score anywhere in `crates/`, `apps/`. | rg `vouch\|ReputationRecord\|\brating\b\|\bscore\b` (only a hash score in `app_orchestration/src/resolver/select.rs:49`) |
| 18.25 | CONFIRMED | Records exist as stated; `PAYMENT_NOTICE` says Roym does not see the money move. | `crates/roym_core/src/payment.rs:46-130,215-235`, `booking.rs:26-27`, `roym_web/ui/src/cards/wording.ts:10` |
| 18.26 | CONFIRMED | Only `http:` and `https:` become links; others become text nodes. | `crates/roym_web/ui/src/cards/link.ts:1-17` |
| 18.27 | CONFIRMED | No gateway, intent, adapter, mutual credit or coin code. | rg `PaymentIntent\|stripe\|mutual.credit\|coin\|payment.gateway\|razorpay\|upi` over `crates/`, `apps/` |

### Commit 19

| Row | Verdict | Note | Checked at |
| --- | --- | --- | --- |
| 19.1 | CONFIRMED | Six Roym services in manifest; identity and auth are substrate crates. | `crates/roym_core/app/roym.toml:9-87`, `crates/identity/src/lib.rs:1-15`, `crates/auth/src/lib.rs:1-20` |
| 19.2 | CONFIRMED | Key file mode 0600 (unix); supervisor vault; backup under a random recovery key. | `crates/identity/src/keys.rs:151-175`, `crates/app_supervisor/src/keys.rs:1-9`, `identity/src/backup.rs:1-10` |
| 19.3 | CONFIRMED | Master signs certificates, anchors, member endpoint records, UCANs. | `crates/identity/src/delegation.rs:99-125`, `core/src/dht_registry/master_anchor.rs:45-66`, `types.rs:122-131`, `roymctl/src/commands/identity.rs:55-80` |
| 19.4 | CONFIRMED | 24 h for both roymctl commands; 4 h supervisor default. | `apps/roymctl/src/commands/session.rs:30-33`, `identity.rs:113`, `crates/core/src/config/roles.rs:82-84,164-180` |
| 19.5 | CONFIRMED | Node key signs the substrate record and is passed to Iroh; member record needs the master signature. | `crates/substrate/src/runtime/publish.rs:162-195`, `router.rs:64,95,164`, `connection_router.rs:68-106`, `types.rs:122-131` |
| 19.6 | CONFIRMED | No enclave, keychain, TPM, ZK or government-id code found. | rg `aadhaar\|zero.knowledge\|enclave\|keychain\|keyring\|tpm\|webauthn` over `crates/`, `apps/`, all `*.toml` |
| 19.7 | CONFIRMED | Five signed fields over canonical JSON. | `crates/identity/src/delegation.rs:59-125` |
| 19.8 | CONFIRMED | Four scopes; router accepts two; auth accepts `session-auth`; signer and verifier use `record-signing`. | `delegation.rs:11-30`, `router/src/handshake.rs:10,57`, `auth/src/service.rs:262`, `core/src/record_signer.rs:141`, `signed_record/src/verify.rs:53` |
| 19.9 | CONFIRMED | Two steps as stated; 5 s timeout; any resolve error also refuses; no certificate means key is master. | `crates/router/src/handshake.rs:44-82` |
| 19.10 | CONFIRMED | `enc` stage never reads the certificate; verifier has no `enc` input. | `crates/router/src/route_handler/encryption.rs:284-330`, `handshake.rs:25-83` |
| 19.11 | CONFIRMED | No proof-of-possession in router; Iroh remote id is only logged; login checks nonce signature. | `crates/router/src/handshake.rs:29-83`, `route_handler.rs:525`, `sdk/src/client.rs:77-85`, `auth/src/service.rs:296-313` |
| 19.12 | CONFIRMED | No Method B code; requirement says plugin is not release-blocking. | `docs/system-requirements-spec.md:945-949`, rg as in 19.6 |
| 19.13 | CONFIRMED | All envelope fields, id prefix and digest, `supersedes` format only. | `crates/signed_record/src/envelope.rs:11-15,109-119,131-143,199-202,225` |
| 19.14 | CONFIRMED | 64 KiB, depth 32, integers, 1-64 type bytes, 256 subject; twelve types in `RECORD_TYPES`. | `crates/signed_record/src/envelope.rs:6-10,47-95`, `roym_core/src/record.rs:18-31` |
| 19.15 | CONFIRMED | Host builds and signs; read-only gets `permission-denied`; all certificate checks run per call. | `crates/wit_interfaces/wit/signing/signing.wit:1-77`, `core/src/record_signer.rs:100-165`, `sandbox_wasm/.../capabilities_services.rs:49-56` |
| 19.16 | CONFIRMED | All listed checks exist (300 s skew, window, scope, issuer). Status is `Unknown` when source has no answer. | `crates/signed_record/src/lib.rs:1-9`, `verify.rs:53-130,158-250` |
| 19.17 | CONFIRMED | Schema, micro-second timestamp, carry-forward of `revoke_list_registry`; no reader. | `crates/core/src/dht_registry/master_anchor.rs:17-31,45-66`, `client.rs:501-515,546-554` |
| 19.18 | CONFIRMED | Newer timestamp wins; older (and equal with different packet) is refused with 409. | `crates/community_registry/src/registry.rs:316-350` |
| 19.19 | PARTLY | Registry-then-DHT is right. An alias cannot be resolved through the DHT. | `crates/core/src/dht_registry/client.rs:203-258` |
| 19.20 | CONFIRMED | Registry first, then DHT; registry anchor needs master signature and under 24 h. | `crates/core/src/dht_registry/client.rs:283-362`, `master_anchor.rs:152-165` |
| 19.21 | PARTLY | Code does as stated for instance keys. No shipped tool revokes a person's key; `publish-anchor` writes an empty list. | `client.rs:540-554`, `roymctl/src/commands/identity.rs:265-273`, `app_supervisor/src/anchors.rs:73-75` |
| 19.22 | PARTLY | 24 h and 12 h are right. Refresh needs a configured registry and an unlocked vault. | `crates/app_supervisor/src/service/renewal.rs:422-480`, `anchors.rs:55-62`, `core/src/config/roles.rs:155-162` |
| 19.23 | CONFIRMED | Chain verified per edge; unresolvable anchor leaves chain valid. | `crates/router/src/route_handler/io.rs:69-85,226-235`, `crates/ucan/src/token.rs:221` |
| 19.24 | CONFIRMED | Backup restore only; registry compares timestamps only; no replace-master flow. | `crates/identity/src/backup.rs:1-20`, `community_registry/src/registry.rs:329-341`, rg `rotat\|successor\|continuity` |
| 19.25 | CONFIRMED | Roots are admin root and a service's recorded owner; session token minted after delegated login. | `crates/router/src/route_handler/io.rs:206-232`, `auth/src/service.rs:243-335`, `ucan/src/session_token.rs:1-8` |

## Findings

### 18.3 PARTLY (doc line 861)

Doc words: "It then checks every hit itself: the signature, the issue time, the expiry, the delegation window and the revocation status."

Code: `listing::verify_envelope` calls `verify_json` with `VerifyOptions::new(now_secs)` (`crates/roym_core/src/listing.rs:505-510`). That default holds an empty revocation source (`crates/signed_record/src/verify.rs:86-95`), which answers `Unknown` for every key and record (`verify.rs:19-27`). `with_revocations` is called only in tests (`verify.rs:564-634`); no other file builds a revocation source. So every verified hit carries `revocation_status: "unknown"` (`listing.rs:534`, `client_query.rs:361`). The node does not check revocation here. It only reports a status.

Proposed doc text: "It then checks every hit itself: the signature, the issue time, the expiry and the delegation window. Each hit also carries a revocation status. Today this status is always `unknown`, because the check is given no revocation list. A withdrawn membership is handled apart, by the standing check (see Trust & Reputation)."

### 18.6 PARTLY (doc lines 906, 915, 931, 1550, 1566, 1850)

Doc words: "`vodozemac` for X3DH + Double Ratchet in 1-to-1 chat."

Code: vodozemac 0.10.0 implements Olm. Its key agreement is "A 3DH implementation following the Olm spec": three Diffie-Hellman results from identity keys and one-time keys (`~/.cargo/registry/src/index.crates.io-*/vodozemac-0.10.0/src/olm/shared_secret.rs:15-26`). X3DH uses a signed prekey and four results. The repo code uses vodozemac's `Account` and `Session` only (`crates/conversation/src/crypto.rs:5-24`). The code comment at `crypto.rs:51` calls the signing key an "X3DH signed prekey role", but the key agreement itself is Olm's.

Proposed doc text (line 931): "**Libraries:** `vodozemac` for 1-to-1 chat. It implements Olm: a triple Diffie-Hellman key agreement and a Double Ratchet." Replace "X3DH + Double Ratchet" with "Olm (triple Diffie-Hellman + Double Ratchet)" at lines 906, 915-916 (diagram labels), 1550, 1566 and 1850. Do not edit ADR-0013.

### 18.8 PARTLY (doc line 933)

Doc words: "A message that cannot be delivered stays in the outbox of the sender and shows `pending` until the peer is reachable."

Code: a pending message moves to `failed` after `conversation_max_pending_age_secs` (default 2 592 000 s, 30 days): `crates/conversation/src/outbox.rs:195-205`, `crates/core/src/config/sandbox.rs:210-214,334`. A settled refusal (for example no certificate or malformed envelope) is also not retried (`crates/conversation/src/transport.rs:50-58`).

Proposed doc text: "A message that cannot be delivered stays in the outbox of the sender and shows `pending` while the peer is not reachable. It becomes `failed` if the peer refuses it, or after 30 days (`conversation_max_pending_age_secs`). Delivery has three states: `pending`, `delivered` and `failed`."

### 19.19 PARTLY (doc line 807)

Doc words: "A client asks the community registry first, and the DHT second, for a signed endpoint record by DID or alias".

Code: the registry resolves an alias (`crates/community_registry/src/registry.rs:307`). The DHT needs a full DID: `client.rs:242-258` ("shorthash aliases won't work purely on DHT") and a `warn` for non-DID ids.

Proposed doc text: "A client asks the community registry first, and the DHT second, for a signed endpoint record by DID. An alias works only at the registry, because the DHT needs the full DID."

### 19.21 PARTLY (doc lines 812-814 and the diagram, lines 842-844)

Doc words: "If a Temporary Key is compromised (e.g., a stolen laptop), the Master Key adds the DID of that key to `revoked_keys` and publishes a new anchor."

Code: the only code that adds to `revoked_keys` is `RegistryClient::revoke_instance_key` (`crates/core/src/dht_registry/client.rs:540-554`). Its only caller is the App Supervisor verb `revoke-instance` (`crates/app_supervisor/src/anchors.rs:73-75`, `service/verbs/revoke.rs:195`), reached by `roymctl supervisor revoke-instance` (`apps/roymctl/src/commands/supervisor.rs:96-104,402`). No `roymctl identity` or `roymctl session` command revokes a person's delegated key (rg `revoke` in `apps/roymctl/src`). `roymctl identity publish-anchor` calls `publish_master_anchor(..., vec![], None, ...)` (`apps/roymctl/src/commands/identity.rs:272`). That writes an empty `revoked_keys` list and does not read the current anchor, so it drops existing revocations. The supervisor path reads the old list first and carries it forward (`client.rs:499-515`).

Proposed doc text for the first paragraph of Passive Revocation: "The master adds the DID of a temporary key to `revoked_keys` and publishes a new anchor. Today the App Supervisor does this for the instance key of a service it manages (`roymctl supervisor revoke-instance`). No `roymctl` command revokes a person's delegated key. `roymctl identity publish-anchor` writes an anchor with an empty list, so it removes any earlier revocation. Entries stay in the list, and revoking a key twice does not add a second entry." Mark the stolen-laptop case as `Envisioned` for a person's key, or remove the example.

### 19.22 PARTLY (doc line 816)

Doc words: "The App Supervisor republishes the anchor of each master it manages every 12 hours by default."

Code: the refresh runs only if the supervisor has an anchor writer, which exists only when `substrate.registry_url` is set (`crates/app_supervisor/src/anchors.rs:55-65`, `service/renewal.rs:423`), and only while the vault key is loaded (`renewal.rs:430-437`). It also skips a master whose last refresh is newer than the interval (`renewal.rs:441-452`). It covers the masters of the instance plan.

Proposed doc text: "When a registry URL is configured and the vault is unlocked, the App Supervisor republishes the anchor of each master it manages. The default interval is 12 hours."

## Doc text not covered by any row

1. Line 787 (Verifying): "It then checks the revocation source that the caller passes". True for the API. No production caller passes a source (rg `with_revocations` finds only `crates/signed_record/src/verify.rs:564-634`, tests). Every production verify therefore returns status `Unknown` (see 18.3). Add: "No Roym service passes a revocation source today, so the status is `Unknown`."
2. Line 857 (Relay Discovery): "BEP 0044 Mainline DHT (via `pkarr`) resolves node/relay endpoints only". The DHT also carries master anchors: published at `crates/core/src/dht_registry/client.rs:402-413` and read at `client.rs:333-358`. Suggested wording: "...resolves endpoint records and master anchors, not catalog search."
3. Line 760 (Method A): the `enc=ecdh-p256` handshake reads the `pubkey` field as a P-256 point (`crates/router/src/route_handler/encryption.rs:292-303`). The certificate check reads the same field as a 32-byte Ed25519 key (`crates/router/src/handshake.rs:29-37`; `crates/router/src/preamble.rs:38-39`). The doc says they are separate. I could not confirm whether one stream can carry both an encrypted handshake and a certificate. If not, the doc should say so. A test that sends both fields on one stream would settle it.
4. Lines 804 and 816 (anchor duty): the community registry keeps anchors only in memory (`crates/community_registry/src/registry.rs:59,70,98`; the sweep at `:143-163` covers endpoints only). After a registry restart, every master with a certificate is refused on a stream until its anchor is published again (`handshake.rs:62-66`). The doc says nothing about this.
5. Line 749 (Method A): `roymctl identity delegate` has no default lifetime: `--expires-days` is required (`apps/roymctl/src/commands/identity.rs:77`). The doc line 724 lists defaults only for two commands, so it is correct. `roymctl identity certify-signing` also defaults to 24 h (`identity.rs:130`) and is not named. Optional addition.
6. Line 787 and `crates/signed_record/src/lib.rs:1-9` ("can never sign"): the crate gives a guest no signing function. A guest could still hold its own key and sign. The doc is true only for keys that the node holds. Suggested wording: "A guest can verify a record. It cannot sign with a key that the node holds."
7. Line 861/859: "each client, SynOrg, directory and aggregator chooses what it queries" has no row. I found `directory.add-source` (`crates/roym_directory/src/app.rs:223`) for the client side. I found nothing for an aggregator that queries. Unproven for "aggregator".

## Cost notes

- Rows per hour: about 52 rows in roughly 1.5 hours of effort, so about 35 rows per hour. Many rows share evidence (messaging, anchor).
- Hard rows: 18.3 (needed a trace from the listing check down to `VerifyOptions::new` and a search for every caller of `with_revocations`), 18.6 (needed the vodozemac source in `~/.cargo/registry`; docs and code comments do not show the mismatch), 19.21 (needed a search of every `roymctl` command to prove a negative), 19.5 (the negative "no publish call for a delegated key" needed a search of every `.sign(` call on an endpoint record).
- Easy rows: the constant and default rows (18.12, 18.14, 19.4, 19.14), which are one grep each.
- Evidence column was mostly accurate. Wrong or loose items were claim wording, not cites.

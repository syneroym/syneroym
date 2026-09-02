//! Profile service application logic, target-independent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, Mutation, QueryOptions, RecordWriteValue,
        },
        signing::RecordDraft,
    },
};
use syneroym_roym_core::{
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_BLOCKS, SECTION_CONTACTS, SECTION_PROFILE,
        SECTION_REPORTS,
    },
    clock,
    envelope::{Request, Response},
    person::{ProfilePayload, is_did_key},
    record::{Envelope, RECORD_PROFILE, VerifyOptions, content_digest, verify_json},
    safety::{self, Admission, ContactLimits},
    services,
    signing::{self, CertificateError},
};

/// This service's own schema version. Bumped by whichever slice changes
/// what this service stores; read by `status` and by nothing else.
pub const SCHEMA_VERSION: u32 = 2;

pub const PROFILES: &str = "profiles";
pub const PROFILE_HISTORY: &str = "profile_history";
pub const CONTACTS: &str = "contacts";
pub const BLOCKS: &str = "blocks";
pub const REPORTS: &str = "reports";
pub const CONTACT_ATTEMPTS: &str = "contact_attempts";
pub const SETTINGS: &str = "settings";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContactRow {
    pub person_did: String,
    pub display_name: Option<String>,
    pub conversation_address: String,
    pub favourite: bool,
    pub added_at_secs: u64,
    pub from_profile_record: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockRow {
    pub key: String,
    pub person_did: Option<String>,
    pub address: Option<String>,
    pub reason: Option<String>,
    pub at_secs: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReportRow {
    pub report_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub category: String,
    pub details: Option<String>,
    pub status: String,
    pub at_secs: u64,
}

async fn ensure_coll<H: AppHost>(
    host: &H,
    name: &str,
    indexes: &[IndexDefinition],
) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec() },
    )
    .await
    .map_err(|e| e.to_string())
}

fn block_keys(person_did: Option<&str>, address: Option<&str>) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(d) = person_did {
        keys.push(format!("did:{d}"));
    }
    if let Some(a) = address {
        keys.push(format!("addr:{a}"));
    }
    keys
}

async fn current_owner_profile_address<H: AppHost>(
    host: &H,
    owner: &str,
) -> Result<Option<String>, String> {
    ensure_coll(host, PROFILES, &[]).await?;
    let Some(row) = AppDataLayer::get(host, PROFILES.to_string(), owner.to_string())
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let val: Value = serde_json::from_slice(&row.payload).map_err(|e| e.to_string())?;
    let env_str = val.get("envelope").and_then(|v| v.as_str()).ok_or("missing envelope field")?;
    let env = Envelope::from_json(env_str).map_err(|e| e.to_string())?;
    let payload: ProfilePayload = serde_json::from_value(env.payload).map_err(|e| e.to_string())?;
    Ok(Some(payload.conversation_address))
}

async fn current_owner_profile_record_id<H: AppHost>(
    host: &H,
    owner: &str,
) -> Result<Option<String>, String> {
    ensure_coll(host, PROFILES, &[]).await?;
    let Some(row) = AppDataLayer::get(host, PROFILES.to_string(), owner.to_string())
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let val: Value = serde_json::from_slice(&row.payload).map_err(|e| e.to_string())?;
    Ok(val.get("record_id").and_then(|v| v.as_str()).map(String::from))
}

async fn load_contact_limits<H: AppHost>(host: &H) -> Result<ContactLimits, String> {
    ensure_coll(host, SETTINGS, &[]).await?;
    let row = AppDataLayer::get(host, SETTINGS.to_string(), "contact_limits".to_string())
        .await
        .map_err(|e| e.to_string())?;
    if let Some(r) = row {
        serde_json::from_slice(&r.payload).map_err(|e| e.to_string())
    } else {
        Ok(ContactLimits::default())
    }
}

async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    ensure_coll(host, collection, &[]).await?;
    let mut results = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;

        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                results.push(json!({ "id": r.id, "payload": parsed }));
            }
        }

        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(results)
}

async fn reverify_profiles<H: AppHost>(host: &H, now: u64) -> Result<u64, String> {
    ensure_coll(host, PROFILES, &[]).await?;
    let records = collect(host, PROFILES).await?;
    let mut verified = 0;
    for rec in records {
        if let Some(payload) = rec.get("payload")
            && let Some(env_str) = payload.get("envelope").and_then(|v| v.as_str())
        {
            let did = rec.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            if verify_json(env_str, &VerifyOptions::new(now).expecting(did)).is_ok() {
                verified += 1;
            }
        }
    }
    Ok(verified)
}

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::PROFILE.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = signing::handle_certificate_verb(host, "profile.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "profile.ping" => Response::ok(json!({ "service": services::PROFILE.name })),
        "profile.policy" => Response::ok(json!({
            "statement": "A blocked sender's messages are refused at this node's inbox. They are never shown in any conversation, never fire a notification, and are never counted. Block is enforced locally by this installation's own Conversation service.",
            "one_person_per_installation": true,
            "retention": "app-data stored until explicitly deleted or restored",
        })),
        "profile.get" => {
            let owner_res = signing::owner_did(host).await;
            let did = req
                .params
                .get("person_did")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| owner_res.ok());

            let Some(did) = did else {
                return Response::invalid_params("person_did or recorded owner required");
            };

            if let Err(e) = ensure_coll(host, PROFILES, &[]).await {
                return Response::internal_error(e);
            }

            match AppDataLayer::get(host, PROFILES.to_string(), did).await {
                Ok(Some(row)) => match serde_json::from_slice::<Value>(&row.payload) {
                    Ok(val) => Response::ok(val),
                    Err(e) => Response::internal_error(e.to_string()),
                },
                Ok(None) => Response::ok(Value::Null),
                Err(e) => Response::internal_error(e.to_string()),
            }
        }
        "profile.set" => {
            let now = clock::now_secs();
            let owner = match signing::owner_did(host).await {
                Ok(o) => o,
                Err(CertificateError::NoOwner) => {
                    return Response::invalid_params("this installation has no recorded owner");
                }
                Err(e) => return Response::internal_error(e.to_string()),
            };

            let (principal, _master) = match signing::person_principal(host, now).await {
                Ok(res) => res,
                Err(CertificateError::NotEnrolled) => {
                    return Response::invalid_params("signing-not-enrolled");
                }
                Err(CertificateError::Expired(t)) => {
                    return Response::invalid_params(format!("signing-certificate-expired at {t}"));
                }
                Err(CertificateError::Stale { installed_for, current }) => {
                    return Response::invalid_params(format!(
                        "signing-certificate-stale: {installed_for} vs {current}"
                    ));
                }
                Err(e) => return Response::internal_error(e.to_string()),
            };

            let display_name = match req.params.get("display_name").and_then(|v| v.as_str()) {
                Some(n) => n.to_string(),
                None => return Response::invalid_params("display_name is required"),
            };
            let about = req.params.get("about").and_then(|v| v.as_str()).map(String::from);
            let locale = req.params.get("locale").and_then(|v| v.as_str()).map(String::from);

            let address = match req.params.get("conversation_address").and_then(|v| v.as_str()) {
                Some(a) => a.to_string(),
                None => match current_owner_profile_address(host, &owner).await {
                    Ok(Some(existing)) => existing,
                    _ => {
                        return Response::invalid_params(
                            "conversation_address is required for the first profile",
                        );
                    }
                },
            };

            let payload =
                ProfilePayload { display_name, about, conversation_address: address, locale };
            if let Err(e) = payload.validate() {
                return Response::invalid_params(e.to_string());
            }

            let supersedes = match current_owner_profile_record_id(host, &owner).await {
                Ok(s) => s,
                Err(e) => return Response::internal_error(e),
            };

            let payload_json_str = match serde_json::to_string(&payload) {
                Ok(s) => s,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            let wit_draft = RecordDraft {
                version: 1,
                record_type: RECORD_PROFILE.to_string(),
                subject: owner.clone(),
                payload: payload_json_str,
                expires_at_secs: None,
                supersedes,
            };

            let envelope_json = match AppSigning::sign_record(host, wit_draft, principal).await {
                Ok(j) => j,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            let envelope = match Envelope::from_json(&envelope_json) {
                Ok(e) => e,
                Err(e) => {
                    return Response::internal_error(format!(
                        "the host returned an envelope this build cannot parse: {e}"
                    ));
                }
            };

            if envelope.issuer != owner {
                return Response::internal_error(
                    "the host signed under an issuer this service did not ask for",
                );
            }

            let record_id = match envelope.record_id() {
                Ok(id) => id,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            if let Err(e) = ensure_coll(host, PROFILE_HISTORY, &[]).await {
                return Response::internal_error(e);
            }
            if let Err(e) = ensure_coll(host, PROFILES, &[]).await {
                return Response::internal_error(e);
            }

            let profile_row = json!({
                "envelope": envelope_json,
                "record_id": record_id,
                "verified_at_secs": now,
            });

            let profile_payload = match serde_json::to_vec(&profile_row) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            // Write the pointer first. If the process crashes between these two writes,
            // the pointer still points to the previous valid record — the supersedes chain
            // stays intact. An orphaned history record (written second, never pointed to)
            // is harmless.
            if let Err(e) = AppDataLayer::put(
                host,
                PROFILES.to_string(),
                RecordWriteValue { id: owner, payload: profile_payload },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }

            if let Err(e) = AppDataLayer::put(
                host,
                PROFILE_HISTORY.to_string(),
                RecordWriteValue {
                    id: record_id.clone(),
                    payload: envelope_json.as_bytes().to_vec(),
                },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }

            Response::ok(json!({ "record_id": record_id, "envelope": envelope_json }))
        }
        "profile.export" => {
            let owner = match signing::owner_did(host).await {
                Ok(o) => o,
                Err(e) => return Response::internal_error(e.to_string()),
            };
            let now = clock::now_secs();

            let p_sec = match collect(host, PROFILES).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };
            let c_sec = match collect(host, CONTACTS).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };
            let b_sec = match collect(host, BLOCKS).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };
            let r_sec = match collect(host, REPORTS).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };

            let sections = BTreeMap::from([
                (SECTION_PROFILE.to_string(), p_sec),
                (SECTION_CONTACTS.to_string(), c_sec),
                (SECTION_BLOCKS.to_string(), b_sec),
                (SECTION_REPORTS.to_string(), r_sec),
            ]);

            let mut manifest_sections = BTreeMap::new();
            for (k, v) in &sections {
                match Bundle::digest(SCHEMA_VERSION, v) {
                    Ok(d) => {
                        manifest_sections.insert(k.clone(), d);
                    }
                    Err(e) => return Response::internal_error(e.to_string()),
                }
            }

            let manifest = BundleManifest {
                bundle_version: BUNDLE_VERSION,
                produced_at_secs: now,
                subject_did: owner,
                sections: manifest_sections,
            };

            let bundle = Bundle { manifest, sections };
            match serde_json::to_value(&bundle) {
                Ok(v) => Response::ok(v),
                Err(e) => Response::internal_error(e.to_string()),
            }
        }
        "profile.import" => {
            let bundle_val =
                match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
                    Some(v) => v,
                    None => return Response::invalid_params("bundle is required"),
                };
            let bundle_str = bundle_val.to_string();
            let bundle = match Bundle::from_json(&bundle_str) {
                Ok(b) => b,
                Err(e) => return Response::invalid_params(format!("invalid bundle: {e}")),
            };

            if let Err(e) = bundle.check_integrity() {
                return Response::invalid_params(e.to_string());
            }

            let owner = match signing::owner_did(host).await {
                Ok(o) => o,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            if bundle.manifest.subject_did != owner {
                return Response::invalid_params(format!(
                    "bundle belongs to '{}', this node holds '{}'",
                    bundle.manifest.subject_did, owner
                ));
            }

            for (name, declared) in &bundle.manifest.sections {
                if declared.schema_version != SCHEMA_VERSION {
                    return Response::invalid_params(format!(
                        "section '{name}' has schema version {}, this node requires \
                         {SCHEMA_VERSION}",
                        declared.schema_version
                    ));
                }
            }

            let now = clock::now_secs();
            let mut prepared_writes: Vec<(&'static str, Vec<Mutation>)> = Vec::new();

            for (name, records) in &bundle.sections {
                let collection = match name.as_str() {
                    SECTION_PROFILE => PROFILES,
                    SECTION_CONTACTS => CONTACTS,
                    SECTION_BLOCKS => BLOCKS,
                    SECTION_REPORTS => REPORTS,
                    other => return Response::invalid_params(format!("unknown section '{other}'")),
                };

                let mut section_muts = Vec::new();
                for rec in records {
                    let id = match rec.get("id").and_then(|v| v.as_str()) {
                        Some(i) => i.to_string(),
                        None => return Response::invalid_params("record missing id"),
                    };
                    let mut payload_val = match rec.get("payload") {
                        Some(p) => p.clone(),
                        None => return Response::invalid_params("record missing payload"),
                    };
                    if name == SECTION_PROFILE {
                        let env_str = match payload_val.get("envelope").and_then(|v| v.as_str()) {
                            Some(s) => s,
                            None => {
                                return Response::invalid_params("profile record missing envelope");
                            }
                        };
                        let verified_rec =
                            match verify_json(env_str, &VerifyOptions::new(now).expecting(&id)) {
                                Ok(vr) => vr,
                                Err(e) => {
                                    return Response::invalid_params(format!(
                                        "profile record '{id}' failed verification: {e}"
                                    ));
                                }
                            };
                        if verified_rec.record_type != RECORD_PROFILE {
                            return Response::invalid_params("record is not a profile");
                        }
                        if verified_rec.version != 1 {
                            return Response::invalid_params("unsupported profile record version");
                        }
                        if verified_rec.subject != id {
                            return Response::invalid_params(format!(
                                "profile record subject '{}' does not match id '{id}'",
                                verified_rec.subject
                            ));
                        }
                        if let Some(obj) = payload_val.as_object_mut() {
                            obj.insert("verified_at_secs".to_string(), json!(now));
                        }
                    }
                    let payload_bytes = match serde_json::to_vec(&payload_val) {
                        Ok(b) => b,
                        Err(e) => return Response::internal_error(e.to_string()),
                    };
                    section_muts
                        .push(Mutation::Put(RecordWriteValue { id, payload: payload_bytes }));
                }
                prepared_writes.push((collection, section_muts));
            }

            // Phase 2: All records and sections verified clean -- apply mutations
            for (collection, muts) in prepared_writes {
                if let Err(e) = ensure_coll(host, collection, &[]).await {
                    return Response::internal_error(e);
                }
                for chunk in muts.chunks(100) {
                    if let Err(e) =
                        AppDataLayer::batch_mutate(host, collection.to_string(), chunk.to_vec())
                            .await
                    {
                        return Response::internal_error(e.to_string());
                    }
                }
            }

            let verified = match reverify_profiles(host, clock::now_secs()).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };

            Response::ok(
                json!({ "sections": bundle.sections.len(), "profiles_verified": verified }),
            )
        }
        "contacts.list" => {
            let favourites_only =
                req.params.get("favourites_only").and_then(|v| v.as_bool()).unwrap_or(false);
            if let Err(e) = ensure_coll(
                host,
                CONTACTS,
                &[IndexDefinition {
                    field_name: "favourite".to_string(),
                    type_: IndexType::Boolean,
                }],
            )
            .await
            {
                return Response::internal_error(e);
            }
            let records = match collect(host, CONTACTS).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };

            let mut list = Vec::new();
            for item in records {
                if let Some(p) = item.get("payload")
                    && let Ok(row) = serde_json::from_value::<ContactRow>(p.clone())
                    && (!favourites_only || row.favourite)
                {
                    list.push(row);
                }
            }
            let offset = req.params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
            let paged: Vec<_> = match limit {
                Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
                None => list.into_iter().skip(offset).collect(),
            };
            Response::ok(json!(paged))
        }
        "contacts.get" => {
            let person_did = match req.params.get("person_did").and_then(|v| v.as_str()) {
                Some(d) => d,
                None => return Response::invalid_params("person_did is required"),
            };
            if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
                return Response::internal_error(e);
            }
            match AppDataLayer::get(host, CONTACTS.to_string(), person_did.to_string()).await {
                Ok(Some(row)) => match serde_json::from_slice::<ContactRow>(&row.payload) {
                    Ok(r) => Response::ok(json!(r)),
                    Err(e) => Response::internal_error(e.to_string()),
                },
                Ok(None) => Response::ok(Value::Null),
                Err(e) => Response::internal_error(e.to_string()),
            }
        }
        "contacts.upsert" => {
            let person_did = match req.params.get("person_did").and_then(|v| v.as_str()) {
                Some(d) => d.to_string(),
                None => return Response::invalid_params("person_did is required"),
            };
            if !is_did_key(&person_did) {
                return Response::invalid_params(format!("'{person_did}' is not a did:key"));
            }

            let now = clock::now_secs();
            let profile_env_option = req.params.get("profile_envelope").and_then(|v| v.as_str());

            let (display_name, address, from_record) = match profile_env_option {
                Some(json_str) => {
                    let v = match verify_json(
                        json_str,
                        &VerifyOptions::new(now).expecting(&person_did),
                    ) {
                        Ok(v) => v,
                        Err(e) => {
                            return Response::invalid_params(format!(
                                "profile did not verify: {e}"
                            ));
                        }
                    };
                    if v.record_type != RECORD_PROFILE || v.version != 1 {
                        return Response::invalid_params(
                            "not a profile record this build understands",
                        );
                    }
                    if v.subject != person_did {
                        return Response::invalid_params(format!(
                            "profile subject '{}' does not match contact DID '{person_did}'",
                            v.subject
                        ));
                    }
                    let p: ProfilePayload = match serde_json::from_value(v.payload.clone()) {
                        Ok(p) => p,
                        Err(e) => return Response::invalid_params(format!("profile payload: {e}")),
                    };
                    if let Err(e) = p.validate() {
                        return Response::invalid_params(e.to_string());
                    }
                    if let Err(e) = ensure_coll(host, PROFILES, &[]).await {
                        return Response::internal_error(e);
                    }
                    let profile_row = json!({
                        "envelope": json_str,
                        "record_id": v.record_id,
                        "verified_at_secs": now,
                    });
                    let profile_payload = match serde_json::to_vec(&profile_row) {
                        Ok(b) => b,
                        Err(e) => return Response::internal_error(e.to_string()),
                    };
                    if let Err(e) = AppDataLayer::put(
                        host,
                        PROFILES.to_string(),
                        RecordWriteValue { id: person_did.clone(), payload: profile_payload },
                    )
                    .await
                    {
                        return Response::internal_error(e.to_string());
                    }
                    (Some(p.display_name), p.conversation_address, Some(v.record_id))
                }
                None => {
                    let disp =
                        req.params.get("display_name").and_then(|v| v.as_str()).map(String::from);
                    let addr = match req.params.get("conversation_address").and_then(|v| v.as_str())
                    {
                        Some(a) => a.to_string(),
                        None => {
                            return Response::invalid_params(
                                "conversation_address is required without a profile",
                            );
                        }
                    };
                    (disp, addr, None)
                }
            };

            if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
                return Response::internal_error(e);
            }

            let existing =
                match AppDataLayer::get(host, CONTACTS.to_string(), person_did.clone()).await {
                    Ok(Some(row)) => serde_json::from_slice::<ContactRow>(&row.payload).ok(),
                    _ => None,
                };

            let existing_added_at = existing.as_ref().map(|r| r.added_at_secs);
            // When the caller omits `favourite`, preserve the stored value.
            // To un-star a contact the caller must send `"favourite": false` explicitly.
            let favourite = req
                .params
                .get("favourite")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| existing.as_ref().map(|r| r.favourite).unwrap_or(false));

            let row = ContactRow {
                person_did: person_did.clone(),
                display_name,
                conversation_address: address,
                favourite,
                added_at_secs: existing_added_at.unwrap_or(now),
                from_profile_record: from_record,
            };

            let payload_bytes = match serde_json::to_vec(&row) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            if let Err(e) = AppDataLayer::put(
                host,
                CONTACTS.to_string(),
                RecordWriteValue { id: person_did.clone(), payload: payload_bytes },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }

            Response::ok(json!({ "person_did": person_did }))
        }
        "contacts.remove" => {
            let person_did = match req.params.get("person_did").and_then(|v| v.as_str()) {
                Some(d) => d.to_string(),
                None => return Response::invalid_params("person_did is required"),
            };
            if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
                return Response::internal_error(e);
            }
            if let Err(e) =
                AppDataLayer::delete(host, CONTACTS.to_string(), person_did.clone()).await
            {
                return Response::internal_error(e.to_string());
            }
            Response::ok(json!({ "removed": person_did }))
        }
        "contacts.resolve-address" => {
            let person_did = match req.params.get("person_did").and_then(|v| v.as_str()) {
                Some(d) => d,
                None => return Response::invalid_params("person_did is required"),
            };
            if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
                return Response::internal_error(e);
            }
            match AppDataLayer::get(host, CONTACTS.to_string(), person_did.to_string()).await {
                Ok(Some(row)) => match serde_json::from_slice::<ContactRow>(&row.payload) {
                    Ok(r) => {
                        Response::ok(json!({ "conversation_address": r.conversation_address }))
                    }
                    Err(e) => Response::internal_error(e.to_string()),
                },
                Ok(None) => Response::invalid_params(format!("contact '{person_did}' not found")),
                Err(e) => Response::internal_error(e.to_string()),
            }
        }
        "contacts.admit-first-contact" => {
            let sender_person_did = req.params.get("sender_person_did").and_then(|v| v.as_str());
            let sender_address = match req.params.get("sender_address").and_then(|v| v.as_str()) {
                Some(a) => a,
                None => return Response::invalid_params("sender_address is required"),
            };

            let now = clock::now_secs();
            let limits = match load_contact_limits(host).await {
                Ok(l) => l,
                Err(e) => return Response::internal_error(e),
            };

            let key = sender_person_did
                .map(|d| format!("did:{d}"))
                .unwrap_or_else(|| format!("addr:{sender_address}"));

            if let Err(e) = ensure_coll(
                host,
                BLOCKS,
                &[IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric }],
            )
            .await
            {
                return Response::internal_error(e);
            }
            if let Err(e) = ensure_coll(
                host,
                CONTACT_ATTEMPTS,
                &[
                    IndexDefinition {
                        field_name: "sender_key".to_string(),
                        type_: IndexType::String,
                    },
                    IndexDefinition {
                        field_name: "at_secs".to_string(),
                        type_: IndexType::Numeric,
                    },
                ],
            )
            .await
            {
                return Response::internal_error(e);
            }

            let blocked_by_key =
                match AppDataLayer::get(host, BLOCKS.to_string(), key.clone()).await {
                    Ok(row) => row.is_some(),
                    Err(e) => return Response::internal_error(e.to_string()),
                };
            let blocked_by_addr = if sender_person_did.is_some() {
                match AppDataLayer::get(host, BLOCKS.to_string(), format!("addr:{sender_address}"))
                    .await
                {
                    Ok(row) => row.is_some(),
                    Err(e) => return Response::internal_error(e.to_string()),
                }
            } else {
                false
            };
            let blocked = blocked_by_key || blocked_by_addr;

            let floor = now.saturating_sub(limits.window_secs);
            let filter_json =
                json!({ "sender_key": key, "at_secs": { "$gte": floor } }).to_string();
            // No limit: we need all attempts in the window to reliably identify the
            // oldest one for an accurate retry_after_secs hint.
            let attempts: Vec<u64> = match AppDataLayer::query(
                host,
                CONTACT_ATTEMPTS.to_string(),
                QueryOptions { filter: Some(filter_json), limit: None, cursor: None },
            )
            .await
            {
                Ok(res) => res
                    .records
                    .iter()
                    .filter_map(|r| {
                        serde_json::from_slice::<Value>(&r.payload)
                            .ok()
                            .and_then(|v| v.get("at_secs").and_then(|t| t.as_u64()))
                    })
                    .collect(),
                Err(e) => return Response::internal_error(e.to_string()),
            };

            match safety::admit_first_contact(blocked, &attempts, &limits, now) {
                Admission::Allow => {
                    let attempt_val = json!({ "sender_key": key, "at_secs": now });
                    let payload = match serde_json::to_vec(&attempt_val) {
                        Ok(b) => b,
                        Err(e) => return Response::internal_error(e.to_string()),
                    };
                    if let Err(e) = AppDataLayer::put(
                        host,
                        CONTACT_ATTEMPTS.to_string(),
                        RecordWriteValue { id: format!("{key}:{now}:{}", attempts.len()), payload },
                    )
                    .await
                    {
                        return Response::internal_error(e.to_string());
                    }
                    Response::ok(json!({ "admission": "allow" }))
                }
                Admission::Blocked => Response::ok(json!({ "admission": "blocked" })),
                Admission::RateLimited { retry_after_secs } => Response::ok(json!({
                    "admission": "rate-limited",
                    "retry_after_secs": retry_after_secs
                })),
            }
        }
        "contacts.limits" => match load_contact_limits(host).await {
            Ok(l) => Response::ok(json!(l)),
            Err(e) => Response::internal_error(e),
        },
        "contacts.set-limits" => {
            let window_secs = match req.params.get("window_secs").and_then(|v| v.as_u64()) {
                Some(w) => w,
                None => return Response::invalid_params("window_secs is required"),
            };
            let max_per_window = match req.params.get("max_per_window").and_then(|v| v.as_u64()) {
                Some(m) => m as u32,
                None => return Response::invalid_params("max_per_window is required"),
            };

            let limits = ContactLimits { window_secs, max_per_window };
            if let Err(e) = limits.validate() {
                return Response::invalid_params(e.to_string());
            }

            if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
                return Response::internal_error(e);
            }

            let payload = match serde_json::to_vec(&limits) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            if let Err(e) = AppDataLayer::put(
                host,
                SETTINGS.to_string(),
                RecordWriteValue { id: "contact_limits".to_string(), payload },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }

            Response::ok(json!(limits))
        }
        "block.add" => {
            let person_did =
                req.params.get("person_did").and_then(|v| v.as_str()).map(String::from);
            let address = req.params.get("address").and_then(|v| v.as_str()).map(String::from);
            let reason = req.params.get("reason").and_then(|v| v.as_str()).map(String::from);

            let keys = block_keys(person_did.as_deref(), address.as_deref());
            if keys.is_empty() {
                return Response::invalid_params(
                    "at least one of person_did or address is required",
                );
            }

            let primary_key = keys[0].clone();
            let now = clock::now_secs();

            if let Err(e) = ensure_coll(
                host,
                BLOCKS,
                &[IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric }],
            )
            .await
            {
                return Response::internal_error(e);
            }

            for key in keys {
                let row = BlockRow {
                    key: key.clone(),
                    person_did: person_did.clone(),
                    address: address.clone(),
                    reason: reason.clone(),
                    at_secs: now,
                };
                let payload = match serde_json::to_vec(&row) {
                    Ok(b) => b,
                    Err(e) => return Response::internal_error(e.to_string()),
                };
                if let Err(e) = AppDataLayer::put(
                    host,
                    BLOCKS.to_string(),
                    RecordWriteValue { id: key, payload },
                )
                .await
                {
                    return Response::internal_error(e.to_string());
                }
            }

            Response::ok(json!({ "key": primary_key }))
        }
        "block.remove" => {
            let person_did = req.params.get("person_did").and_then(|v| v.as_str());
            let address = req.params.get("address").and_then(|v| v.as_str());

            let mut keys_to_delete = block_keys(person_did, address);
            if keys_to_delete.is_empty() {
                return Response::invalid_params(
                    "at least one of person_did or address is required",
                );
            }

            if let Err(e) = ensure_coll(host, BLOCKS, &[]).await {
                return Response::internal_error(e);
            }

            let primary_key = keys_to_delete[0].clone();

            // Look up existing rows so complementary keys (e.g. addr: when given did:) are
            // also removed
            for key in &keys_to_delete.clone() {
                if let Ok(Some(row_val)) =
                    AppDataLayer::get(host, BLOCKS.to_string(), key.clone()).await
                    && let Ok(row) = serde_json::from_slice::<BlockRow>(&row_val.payload)
                {
                    for extra_key in block_keys(row.person_did.as_deref(), row.address.as_deref()) {
                        if !keys_to_delete.contains(&extra_key) {
                            keys_to_delete.push(extra_key);
                        }
                    }
                }
            }

            for key in keys_to_delete {
                if let Err(e) = AppDataLayer::delete(host, BLOCKS.to_string(), key).await {
                    return Response::internal_error(e.to_string());
                }
            }

            Response::ok(json!({ "removed": primary_key }))
        }
        "block.list" => {
            if let Err(e) = ensure_coll(
                host,
                BLOCKS,
                &[IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric }],
            )
            .await
            {
                return Response::internal_error(e);
            }
            let records = match collect(host, BLOCKS).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };

            let mut list = Vec::new();
            for item in records {
                if let Some(p) = item.get("payload")
                    && let Ok(row) = serde_json::from_value::<BlockRow>(p.clone())
                {
                    // Skip secondary addr: alias rows for blocks that have a primary did: row
                    if row.key.starts_with("addr:") && row.person_did.is_some() {
                        continue;
                    }
                    list.push(row);
                }
            }
            let offset = req.params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
            let paged: Vec<_> = match limit {
                Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
                None => list.into_iter().skip(offset).collect(),
            };
            Response::ok(json!(paged))
        }
        "block.check" => {
            let person_did = req.params.get("person_did").and_then(|v| v.as_str());
            let address = req.params.get("address").and_then(|v| v.as_str());

            let keys = block_keys(person_did, address);
            if keys.is_empty() {
                return Response::invalid_params(
                    "at least one of person_did or address is required",
                );
            }

            if let Err(e) = ensure_coll(host, BLOCKS, &[]).await {
                return Response::internal_error(e);
            }

            for key in keys {
                match AppDataLayer::get(host, BLOCKS.to_string(), key).await {
                    Ok(Some(row)) => {
                        let parsed = serde_json::from_slice::<BlockRow>(&row.payload).ok();
                        return Response::ok(json!({
                            "blocked": true,
                            "reason": parsed.as_ref().and_then(|b| b.reason.clone()),
                            "since_secs": parsed.as_ref().map(|b| b.at_secs),
                        }));
                    }
                    Ok(None) => {}
                    Err(e) => return Response::internal_error(e.to_string()),
                }
            }

            Response::ok(json!({ "blocked": false }))
        }
        "report.create" => {
            let subject_kind = match req.params.get("subject_kind").and_then(|v| v.as_str()) {
                Some(k) if matches!(k, "person" | "listing" | "message") => k.to_string(),
                _ => {
                    return Response::invalid_params(
                        "subject_kind must be 'person', 'listing', or 'message'",
                    );
                }
            };
            let subject_id = match req.params.get("subject_id").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => return Response::invalid_params("subject_id is required"),
            };
            let category = match req.params.get("category").and_then(|v| v.as_str()) {
                Some(c)
                    if matches!(
                        c,
                        "impersonation"
                            | "fraud"
                            | "harassment"
                            | "unsafe-service"
                            | "illegal-content"
                    ) =>
                {
                    c.to_string()
                }
                _ => {
                    return Response::invalid_params(
                        "category must be one of the five valid categories",
                    );
                }
            };
            let details = req.params.get("details").and_then(|v| v.as_str()).map(String::from);

            let content_val = json!({
                "subject_kind": subject_kind,
                "subject_id": subject_id,
                "category": category,
                "details": details,
            });

            let report_id = match content_digest("rep_", &content_val) {
                Ok(id) => id,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            let now = clock::now_secs();

            if let Err(e) = ensure_coll(
                host,
                REPORTS,
                &[
                    IndexDefinition { field_name: "status".to_string(), type_: IndexType::String },
                    IndexDefinition {
                        field_name: "at_secs".to_string(),
                        type_: IndexType::Numeric,
                    },
                ],
            )
            .await
            {
                return Response::internal_error(e);
            }

            // Check whether this content was already reported or withdrawn.
            // `report_id` is content-derived, so the same content hits the same row.
            // Re-filing a withdrawn report is not permitted: the original decision and
            // timestamp must be preserved.
            if let Ok(Some(existing_row)) =
                AppDataLayer::get(host, REPORTS.to_string(), report_id.clone()).await
                && let Ok(existing) = serde_json::from_slice::<ReportRow>(&existing_row.payload)
            {
                if existing.status == "withdrawn" {
                    return Response::invalid_params(
                        "this report was withdrawn; re-filing the same content is not permitted",
                    );
                }
                // Already recorded — return idempotently, preserving the original timestamp.
                return Response::ok(json!({
                    "report_id": existing.report_id,
                    "status": existing.status,
                }));
            }

            let row = ReportRow {
                report_id: report_id.clone(),
                subject_kind,
                subject_id,
                category,
                details,
                status: "recorded".to_string(),
                at_secs: now,
            };

            let payload = match serde_json::to_vec(&row) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            if let Err(e) = AppDataLayer::put(
                host,
                REPORTS.to_string(),
                RecordWriteValue { id: report_id.clone(), payload },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }

            Response::ok(json!({ "report_id": report_id, "status": "recorded" }))
        }
        "report.list" => {
            let status_filter = req.params.get("status").and_then(|v| v.as_str());
            if let Err(e) = ensure_coll(
                host,
                REPORTS,
                &[
                    IndexDefinition { field_name: "status".to_string(), type_: IndexType::String },
                    IndexDefinition {
                        field_name: "at_secs".to_string(),
                        type_: IndexType::Numeric,
                    },
                ],
            )
            .await
            {
                return Response::internal_error(e);
            }
            let records = match collect(host, REPORTS).await {
                Ok(v) => v,
                Err(e) => return Response::internal_error(e),
            };

            let mut list = Vec::new();
            for item in records {
                if let Some(p) = item.get("payload")
                    && let Ok(row) = serde_json::from_value::<ReportRow>(p.clone())
                    && status_filter.is_none_or(|s| row.status == s)
                {
                    list.push(row);
                }
            }
            let offset = req.params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
            let paged: Vec<_> = match limit {
                Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
                None => list.into_iter().skip(offset).collect(),
            };
            Response::ok(json!(paged))
        }
        "report.get" => {
            let report_id = match req.params.get("report_id").and_then(|v| v.as_str()) {
                Some(r) => r,
                None => return Response::invalid_params("report_id is required"),
            };
            if let Err(e) = ensure_coll(host, REPORTS, &[]).await {
                return Response::internal_error(e);
            }
            match AppDataLayer::get(host, REPORTS.to_string(), report_id.to_string()).await {
                Ok(Some(row)) => match serde_json::from_slice::<ReportRow>(&row.payload) {
                    Ok(r) => Response::ok(json!(r)),
                    Err(e) => Response::internal_error(e.to_string()),
                },
                Ok(None) => Response::ok(Value::Null),
                Err(e) => Response::internal_error(e.to_string()),
            }
        }
        "report.withdraw" => {
            let report_id = match req.params.get("report_id").and_then(|v| v.as_str()) {
                Some(r) => r.to_string(),
                None => return Response::invalid_params("report_id is required"),
            };
            if let Err(e) = ensure_coll(host, REPORTS, &[]).await {
                return Response::internal_error(e);
            }
            let row_opt =
                match AppDataLayer::get(host, REPORTS.to_string(), report_id.clone()).await {
                    Ok(r) => r,
                    Err(e) => return Response::internal_error(e.to_string()),
                };

            let Some(row) = row_opt else {
                return Response::invalid_params(format!("report '{report_id}' not found"));
            };

            let mut parsed: ReportRow = match serde_json::from_slice(&row.payload) {
                Ok(p) => p,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            parsed.status = "withdrawn".to_string();
            let payload = match serde_json::to_vec(&parsed) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };

            if let Err(e) = AppDataLayer::put(
                host,
                REPORTS.to_string(),
                RecordWriteValue { id: report_id.clone(), payload },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }

            Response::ok(json!({ "report_id": report_id, "status": "withdrawn" }))
        }
        other => Response::method_not_found(other),
    }
}

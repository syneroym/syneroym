//! Contacts management and first-contact safety admission.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{IndexDefinition, IndexType, QueryOptions, RecordWriteValue},
};
use syneroym_roym_core::{
    clock,
    envelope::{Request, Response},
    person::{ProfilePayload, is_did_key},
    record::{RECORD_PROFILE, VerifyOptions, verify_json},
    safety::{self, Admission, ContactLimits},
};

use super::{BLOCKS, CONTACT_ATTEMPTS, CONTACTS, PROFILES, SETTINGS, backup::collect, ensure_coll};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContactRow {
    pub person_did: String,
    pub display_name: Option<String>,
    pub conversation_address: String,
    pub favourite: bool,
    pub added_at_secs: u64,
    pub from_profile_record: Option<String>,
}

pub(crate) async fn load_contact_limits<H: AppHost>(host: &H) -> Result<ContactLimits, String> {
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

pub(crate) async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
    let favourites_only =
        req.params.get("favourites_only").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Err(e) = ensure_coll(
        host,
        CONTACTS,
        &[IndexDefinition { field_name: "favourite".to_string(), type_: IndexType::Boolean }],
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

pub(crate) async fn get<H: AppHost>(host: &H, req: &Request) -> Response {
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

async fn process_profile_envelope<H: AppHost>(
    host: &H,
    json_str: &str,
    person_did: &str,
    now: u64,
) -> Result<(Option<String>, String, Option<String>), Response> {
    let v = match verify_json(json_str, &VerifyOptions::new(now).expecting(person_did)) {
        Ok(v) => v,
        Err(e) => {
            return Err(Response::invalid_params(format!("profile did not verify: {e}")));
        }
    };
    if v.record_type != RECORD_PROFILE || v.version != 1 {
        return Err(Response::invalid_params("not a profile record this build understands"));
    }
    if v.subject != person_did {
        return Err(Response::invalid_params(format!(
            "profile subject '{}' does not match contact DID '{person_did}'",
            v.subject
        )));
    }
    let p: ProfilePayload = match serde_json::from_value(v.payload.clone()) {
        Ok(p) => p,
        Err(e) => return Err(Response::invalid_params(format!("profile payload: {e}"))),
    };
    if let Err(e) = p.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    if let Err(e) = ensure_coll(host, PROFILES, &[]).await {
        return Err(Response::internal_error(e));
    }
    let profile_row = json!({
        "envelope": json_str,
        "record_id": v.record_id,
        "verified_at_secs": now,
    });
    let profile_payload = match serde_json::to_vec(&profile_row) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    if let Err(e) = AppDataLayer::put(
        host,
        PROFILES.to_string(),
        RecordWriteValue { id: person_did.to_string(), payload: profile_payload },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }
    Ok((Some(p.display_name), p.conversation_address, Some(v.record_id)))
}

pub(crate) async fn upsert<H: AppHost>(host: &H, req: &Request) -> Response {
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
        Some(json_str) => match process_profile_envelope(host, json_str, &person_did, now).await {
            Ok(res) => res,
            Err(resp) => return resp,
        },
        None => {
            let disp = req.params.get("display_name").and_then(|v| v.as_str()).map(String::from);
            let addr = match req.params.get("conversation_address").and_then(|v| v.as_str()) {
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

    let existing = match AppDataLayer::get(host, CONTACTS.to_string(), person_did.clone()).await {
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

pub(crate) async fn remove<H: AppHost>(host: &H, req: &Request) -> Response {
    let person_did = match req.params.get("person_did").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => return Response::invalid_params("person_did is required"),
    };
    if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = AppDataLayer::delete(host, CONTACTS.to_string(), person_did.clone()).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": person_did }))
}

pub(crate) async fn resolve_address<H: AppHost>(host: &H, req: &Request) -> Response {
    let person_did = match req.params.get("person_did").and_then(|v| v.as_str()) {
        Some(d) => d,
        None => return Response::invalid_params("person_did is required"),
    };
    if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
        return Response::internal_error(e);
    }
    match AppDataLayer::get(host, CONTACTS.to_string(), person_did.to_string()).await {
        Ok(Some(row)) => match serde_json::from_slice::<ContactRow>(&row.payload) {
            Ok(r) => Response::ok(json!({ "conversation_address": r.conversation_address })),
            Err(e) => Response::internal_error(e.to_string()),
        },
        Ok(None) => Response::invalid_params(format!("contact '{person_did}' not found")),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

pub(crate) async fn admit_first_contact<H: AppHost>(host: &H, req: &Request) -> Response {
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
            IndexDefinition { field_name: "sender_key".to_string(), type_: IndexType::String },
            IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric },
        ],
    )
    .await
    {
        return Response::internal_error(e);
    }

    let blocked_by_key = match AppDataLayer::get(host, BLOCKS.to_string(), key.clone()).await {
        Ok(row) => row.is_some(),
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let blocked_by_addr = if sender_person_did.is_some() {
        match AppDataLayer::get(host, BLOCKS.to_string(), format!("addr:{sender_address}")).await {
            Ok(row) => row.is_some(),
            Err(e) => return Response::internal_error(e.to_string()),
        }
    } else {
        false
    };
    let blocked = blocked_by_key || blocked_by_addr;

    let floor = now.saturating_sub(limits.window_secs);
    let filter_json = json!({ "sender_key": key, "at_secs": { "$gte": floor } }).to_string();
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
        Admission::RateLimited { retry_after_secs } => Response::ok(
            json!({ "admission": "rate-limited", "retry_after_secs": retry_after_secs }),
        ),
    }
}

pub(crate) async fn limits<H: AppHost>(host: &H) -> Response {
    match load_contact_limits(host).await {
        Ok(l) => Response::ok(json!(l)),
        Err(e) => Response::internal_error(e),
    }
}

pub(crate) async fn set_limits<H: AppHost>(host: &H, req: &Request) -> Response {
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

//! Contacts management and first-contact safety admission.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{IndexDefinition, IndexType, RecordWriteValue},
};
pub use syneroym_roym_core::method::profile::ContactRow;
use syneroym_roym_core::{
    clock,
    envelope::Response,
    method::{
        NoParams, Reply,
        profile::{
            AdmissionKind, AdmitFirstContactInput, AdmitFirstContactOutput, ContactKeyInput,
            ContactsListInput, ResolveAddressOutput,
        },
    },
    paging,
    person::{ProfilePayload, is_did_key},
    record::{RECORD_PROFILE, VerifyOptions, verify_json},
    safety::{self, Admission, ContactLimits},
};

use super::{BLOCKS, CONTACT_ATTEMPTS, CONTACTS, PROFILES, SETTINGS, ensure_coll};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactUpsertInput {
    pub person_did: String,
    pub profile_envelope: Option<String>,
    pub display_name: Option<String>,
    pub conversation_address: Option<String>,
    pub favourite: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ContactUpsertOutput {
    pub person_did: String,
}

#[derive(Debug, Serialize)]
pub struct ContactRemoveOutput {
    pub removed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactLimitsInput {
    pub window_secs: u64,
    pub max_per_window: u32,
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

pub(crate) async fn list<H: AppHost>(host: &H, input: ContactsListInput) -> Reply<Vec<ContactRow>> {
    let favourites_only = input.favourites_only.unwrap_or(false);
    if let Err(e) = ensure_coll(
        host,
        CONTACTS,
        &[IndexDefinition { field_name: "favourite".to_string(), type_: IndexType::Boolean }],
    )
    .await
    {
        return Err(Response::internal_error(e));
    }
    let mut list: Vec<ContactRow> = match paging::query_all(host, CONTACTS, None).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    list.retain(|row| !favourites_only || row.favourite);
    let offset = input.offset.unwrap_or(0) as usize;
    let limit = input.limit.map(|v| v as usize);
    let paged: Vec<_> = match limit {
        Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
        None => list.into_iter().skip(offset).collect(),
    };
    Ok(paged)
}

pub(crate) async fn get<H: AppHost>(host: &H, input: ContactKeyInput) -> Reply<Option<ContactRow>> {
    if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
        return Err(Response::internal_error(e));
    }
    match AppDataLayer::get(host, CONTACTS.to_string(), input.person_did).await {
        Ok(Some(row)) => match serde_json::from_slice::<ContactRow>(&row.payload) {
            Ok(r) => Ok(Some(r)),
            Err(e) => Err(Response::internal_error(e.to_string())),
        },
        Ok(None) => Ok(None),
        Err(e) => Err(Response::internal_error(e.to_string())),
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

pub(crate) async fn upsert<H: AppHost>(
    host: &H,
    input: ContactUpsertInput,
) -> Reply<ContactUpsertOutput> {
    let person_did = input.person_did;
    if !is_did_key(&person_did) {
        return Err(Response::invalid_params(format!("'{person_did}' is not a did:key")));
    }

    let now = clock::now_secs();

    let (display_name, address, from_record) = match input.profile_envelope.as_deref() {
        Some(json_str) => process_profile_envelope(host, json_str, &person_did, now).await?,
        None => {
            let addr = match input.conversation_address {
                Some(a) => a,
                None => {
                    return Err(Response::invalid_params(
                        "conversation_address is required without a profile",
                    ));
                }
            };
            (input.display_name, addr, None)
        }
    };

    if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
        return Err(Response::internal_error(e));
    }

    let existing = match AppDataLayer::get(host, CONTACTS.to_string(), person_did.clone()).await {
        Ok(Some(row)) => serde_json::from_slice::<ContactRow>(&row.payload).ok(),
        _ => None,
    };

    let existing_added_at = existing.as_ref().map(|r| r.added_at_secs);
    let favourite =
        input.favourite.unwrap_or_else(|| existing.as_ref().map(|r| r.favourite).unwrap_or(false));

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
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if let Err(e) = AppDataLayer::put(
        host,
        CONTACTS.to_string(),
        RecordWriteValue { id: person_did.clone(), payload: payload_bytes },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    Ok(ContactUpsertOutput { person_did })
}

pub(crate) async fn remove<H: AppHost>(
    host: &H,
    input: ContactKeyInput,
) -> Reply<ContactRemoveOutput> {
    if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = AppDataLayer::delete(host, CONTACTS.to_string(), input.person_did.clone()).await
    {
        return Err(Response::internal_error(e.to_string()));
    }
    Ok(ContactRemoveOutput { removed: input.person_did })
}

pub(crate) async fn resolve_address<H: AppHost>(
    host: &H,
    input: ContactKeyInput,
) -> Reply<ResolveAddressOutput> {
    let person_did = input.person_did;
    if let Err(e) = ensure_coll(host, CONTACTS, &[]).await {
        return Err(Response::internal_error(e));
    }
    match AppDataLayer::get(host, CONTACTS.to_string(), person_did.clone()).await {
        Ok(Some(row)) => match serde_json::from_slice::<ContactRow>(&row.payload) {
            Ok(r) => Ok(ResolveAddressOutput { conversation_address: r.conversation_address }),
            Err(e) => Err(Response::internal_error(e.to_string())),
        },
        Ok(None) => Err(Response::invalid_params(format!("contact '{person_did}' not found"))),
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

pub(crate) async fn admit_first_contact<H: AppHost>(
    host: &H,
    input: AdmitFirstContactInput,
) -> Reply<AdmitFirstContactOutput> {
    let sender_person_did = input.sender_person_did.as_deref();
    let sender_address = &input.sender_address;

    let now = clock::now_secs();
    let limits = match load_contact_limits(host).await {
        Ok(l) => l,
        Err(e) => return Err(Response::internal_error(e)),
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
        return Err(Response::internal_error(e));
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
        return Err(Response::internal_error(e));
    }

    let blocked_by_key = match AppDataLayer::get(host, BLOCKS.to_string(), key.clone()).await {
        Ok(row) => row.is_some(),
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let blocked_by_addr = if sender_person_did.is_some() {
        match AppDataLayer::get(host, BLOCKS.to_string(), format!("addr:{sender_address}")).await {
            Ok(row) => row.is_some(),
            Err(e) => return Err(Response::internal_error(e.to_string())),
        }
    } else {
        false
    };
    let blocked = blocked_by_key || blocked_by_addr;

    let floor = now.saturating_sub(limits.window_secs);
    let filter_json = json!({ "sender_key": key, "at_secs": { "$gte": floor } }).to_string();

    let attempts = paging::filter_map(host, CONTACT_ATTEMPTS, Some(filter_json), |row| {
        let v: Value = serde_json::from_slice(&row.payload).ok()?;
        v.get("at_secs").and_then(Value::as_u64)
    })
    .await;
    let attempts = match attempts {
        Ok(a) => a,
        Err(e) => return Err(Response::internal_error(e)),
    };

    let res = safety::admit_first_contact(blocked, &attempts, &limits, now);
    match res.admission {
        Admission::Allow => {
            let attempt_val = json!({ "sender_key": key, "at_secs": now });
            let payload = match serde_json::to_vec(&attempt_val) {
                Ok(b) => b,
                Err(e) => return Err(Response::internal_error(e.to_string())),
            };
            if let Err(e) = AppDataLayer::put(
                host,
                CONTACT_ATTEMPTS.to_string(),
                RecordWriteValue { id: format!("{key}:{now}:{}", attempts.len()), payload },
            )
            .await
            {
                return Err(Response::internal_error(e.to_string()));
            }
            Ok(AdmitFirstContactOutput {
                admission: AdmissionKind::Allow,
                blocked: res.blocked,
                retry_after_secs: None,
            })
        }
        Admission::RateLimited { retry_after_secs } => Ok(AdmitFirstContactOutput {
            admission: AdmissionKind::RateLimited,
            blocked: res.blocked,
            retry_after_secs: Some(retry_after_secs),
        }),
        Admission::Blocked => Ok(AdmitFirstContactOutput {
            admission: AdmissionKind::Blocked,
            blocked: true,
            retry_after_secs: None,
        }),
    }
}

pub(crate) async fn limits<H: AppHost>(host: &H, _: NoParams) -> Reply<ContactLimits> {
    match load_contact_limits(host).await {
        Ok(l) => Ok(l),
        Err(e) => Err(Response::internal_error(e)),
    }
}

pub(crate) async fn set_limits<H: AppHost>(
    host: &H,
    input: ContactLimitsInput,
) -> Reply<ContactLimits> {
    let limits =
        ContactLimits { window_secs: input.window_secs, max_per_window: input.max_per_window };
    if let Err(e) = limits.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Err(Response::internal_error(e));
    }

    let payload = match serde_json::to_vec(&limits) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if let Err(e) = AppDataLayer::put(
        host,
        SETTINGS.to_string(),
        RecordWriteValue { id: "contact_limits".to_string(), payload },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    Ok(limits)
}

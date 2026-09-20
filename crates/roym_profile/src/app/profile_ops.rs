//! Profile operations: policy, get, and set.

use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::RecordWriteValue,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    clock,
    envelope::{Request, Response},
    person::ProfilePayload,
    record::{Envelope, RECORD_PROFILE},
    signing::{self, CertificateError},
};

use super::{PROFILE_HISTORY, PROFILES, ensure_coll};

pub(crate) fn policy() -> Response {
    Response::ok(json!({
        "statement": "A blocked sender's messages are refused at this node's inbox. They are never shown in any conversation, never fire a notification, and are never counted. Block is enforced locally by this installation's own Conversation service.",
        "one_person_per_installation": true,
        "retention": "app-data stored until explicitly deleted or restored. This installation keeps its own copy of every message it sends and receives, separate from the copy the substrate keeps for delivery. That is what an export, a search, and a delete act on, and it means each message is stored twice on this machine.",
    }))
}

pub(crate) async fn current_owner_profile_address<H: AppHost>(
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

pub(crate) async fn current_owner_profile_record_id<H: AppHost>(
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

pub(crate) async fn get<H: AppHost>(host: &H, req: &Request) -> Response {
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

async fn parse_and_validate_payload<H: AppHost>(
    host: &H,
    req: &Request,
    owner: &str,
) -> Result<ProfilePayload, Response> {
    let display_name = match req.params.get("display_name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return Err(Response::invalid_params("display_name is required")),
    };
    let about = req.params.get("about").and_then(|v| v.as_str()).map(String::from);
    let locale = req.params.get("locale").and_then(|v| v.as_str()).map(String::from);

    let address = match req.params.get("conversation_address").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => match current_owner_profile_address(host, owner).await {
            Ok(Some(existing)) => existing,
            _ => {
                return Err(Response::invalid_params(
                    "conversation_address is required for the first profile",
                ));
            }
        },
    };

    let payload = ProfilePayload { display_name, about, conversation_address: address, locale };
    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }
    Ok(payload)
}

async fn sign_profile_draft<H: AppHost>(
    host: &H,
    payload: &ProfilePayload,
    owner: &str,
    supersedes: Option<String>,
    principal: Principal,
) -> Result<(String, String), Response> {
    let payload_json_str = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let wit_draft = RecordDraft {
        version: 1,
        record_type: RECORD_PROFILE.to_string(),
        subject: owner.to_string(),
        payload: payload_json_str,
        expires_at_secs: None,
        supersedes,
    };

    let envelope_json = match AppSigning::sign_record(host, wit_draft, principal).await {
        Ok(j) => j,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(e) => e,
        Err(e) => {
            return Err(Response::internal_error(format!(
                "the host returned an envelope this build cannot parse: {e}"
            )));
        }
    };

    if envelope.issuer != owner {
        return Err(Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        ));
    }

    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    Ok((envelope_json, record_id))
}

async fn persist_profile_records<H: AppHost>(
    host: &H,
    owner: String,
    record_id: &str,
    envelope_json: &str,
    now: u64,
) -> Result<(), Response> {
    if let Err(e) = ensure_coll(host, PROFILE_HISTORY, &[]).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = ensure_coll(host, PROFILES, &[]).await {
        return Err(Response::internal_error(e));
    }

    let profile_row = json!({
        "envelope": envelope_json,
        "record_id": record_id,
        "verified_at_secs": now,
    });

    let profile_payload = match serde_json::to_vec(&profile_row) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
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
        return Err(Response::internal_error(e.to_string()));
    }

    if let Err(e) = AppDataLayer::put(
        host,
        PROFILE_HISTORY.to_string(),
        RecordWriteValue { id: record_id.to_string(), payload: envelope_json.as_bytes().to_vec() },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    Ok(())
}

pub(crate) async fn set<H: AppHost>(host: &H, req: &Request) -> Response {
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

    let payload = match parse_and_validate_payload(host, req, &owner).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };

    let supersedes = match current_owner_profile_record_id(host, &owner).await {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e),
    };

    let (envelope_json, record_id) =
        match sign_profile_draft(host, &payload, &owner, supersedes, principal).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };

    if let Err(resp) = persist_profile_records(host, owner, &record_id, &envelope_json, now).await {
        return resp;
    }

    Response::ok(json!({ "record_id": record_id, "envelope": envelope_json }))
}

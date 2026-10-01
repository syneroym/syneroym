//! Server half: settings, roster.

use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, AppSigning};
use syneroym_roym_core::{
    clock,
    directory::{Member, SynOrgSettings},
    envelope::{Request, Response},
    membership::MembershipVerdict,
    paging,
};

use super::{
    MEMBERS, SETTINGS, SETTINGS_KEY, ensure_coll, get_json, issuer_did, publication_ops, put_json,
    standing,
};

pub(in crate::app) async fn load_settings<H: AppHost>(
    host: &H,
) -> Result<Option<SynOrgSettings>, String> {
    ensure_coll(host, SETTINGS, &[]).await?;
    get_json(host, SETTINGS, SETTINGS_KEY).await
}

pub(in crate::app) async fn get_settings<H: AppHost>(host: &H) -> Response {
    match load_settings(host).await {
        Ok(Some(s)) => Response::ok(json!(s)),
        Ok(None) => Response::ok(Value::Null),
        Err(e) => Response::internal_error(e),
    }
}

pub(in crate::app) async fn set_settings<H: AppHost>(host: &H, req: &Request) -> Response {
    let settings: SynOrgSettings = match serde_json::from_value(req.params.clone()) {
        Ok(s) => s,
        Err(e) => return Response::invalid_params(format!("invalid settings: {e}")),
    };
    if let Err(e) = settings.validate() {
        return Response::invalid_params(e.to_string());
    }
    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = put_json(host, SETTINGS, SETTINGS_KEY, &settings).await {
        return Response::internal_error(e);
    }
    Response::ok(json!(settings))
}

async fn member_count<H: AppHost>(host: &H) -> Result<u64, String> {
    ensure_coll(host, MEMBERS, &[]).await?;
    Ok(paging::query_all::<_, Value>(host, MEMBERS, None).await?.len() as u64)
}

pub(in crate::app) async fn info<H: AppHost>(host: &H) -> Response {
    let settings = match load_settings(host).await {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e),
    };
    let Some(settings) = settings else {
        // Refusing would be indistinguishable from a network fault, which
        // is worse for the case this is actually about: someone adding an
        // address a friend gave them.
        return Response::ok(Value::Null);
    };
    // Best-effort: a retention prune that fails must not fail the probe a
    // stranger makes while deciding whether to trust this group.
    let _ = publication_ops::prune_expired_publications(host, settings.retention_secs).await;
    let count = match member_count(host).await {
        Ok(c) => c,
        Err(e) => return Response::internal_error(e),
    };
    let issuer = issuer_did(host).await;
    let signing_did = AppSigning::signing_identity(host).await.ok().map(|id| id.signing_did);
    Response::ok(json!({
        "name": settings.name,
        "rules": settings.rules,
        "area": settings.area,
        "categories": settings.categories,
        "support_contact": settings.support_contact,
        "dispute_path": settings.dispute_path,
        "retention_secs": settings.retention_secs,
        "member_count": count,
        "issuer_did": issuer,
        "signing_did": signing_did,
    }))
}

pub(in crate::app) async fn member_add<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("did is required"),
    };
    let note = req.params.get("note").and_then(Value::as_str).unwrap_or_default().to_string();
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let member = Member { did: did.clone(), note, added_at_secs: clock::now_secs() };
    if let Err(e) = put_json(host, MEMBERS, &did, &member).await {
        return Response::internal_error(e);
    }
    Response::ok(json!(member))
}

pub(in crate::app) async fn member_remove<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) => d.to_string(),
        None => return Response::invalid_params("did is required"),
    };
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let now = clock::now_secs();
    let verdict = match standing::own_verdict(host, &did, None, now).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    if matches!(verdict, MembershipVerdict::Valid { .. } | MembershipVerdict::Suspended { .. }) {
        return Response::invalid_params("revoke this member's credential first");
    }
    let existed = AppDataLayer::get(host, MEMBERS.to_string(), did.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, MEMBERS.to_string(), did).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": existed }))
}

pub(in crate::app) async fn member_list<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let members: Vec<Value> = match paging::query_all(host, MEMBERS, None).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    Response::ok(json!({ "members": members }))
}

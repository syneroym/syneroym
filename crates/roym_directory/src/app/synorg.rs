//! Server half: settings, roster.

use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost};
use syneroym_roym_core::{
    clock,
    directory::{Member, SynOrgSettings},
    envelope::{Request, Response},
};

use super::{
    MEMBERS, SETTINGS, SETTINGS_KEY, collect_raw, ensure_coll, get_json, publication_ops, put_json,
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
    Ok(collect_raw(host, MEMBERS).await?.len() as u64)
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
    Response::ok(json!({
        "name": settings.name,
        "rules": settings.rules,
        "area": settings.area,
        "categories": settings.categories,
        "support_contact": settings.support_contact,
        "dispute_path": settings.dispute_path,
        "retention_secs": settings.retention_secs,
        "member_count": count,
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
    let rows = match collect_raw(host, MEMBERS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let members: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
    Response::ok(json!({ "members": members }))
}

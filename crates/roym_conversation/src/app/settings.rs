//! The person's default forward mode and the first-message flag.
//!
//! One row, in this service's own storage, so every client of this node
//! sees the same answer and a restore keeps it.

use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    envelope::Response,
    method::{
        ParamError, Reply,
        conversation::{ForwardSettings, SettingsInput},
    },
};

use super::{SETTINGS, ensure_settings};

pub(crate) const FORWARD_ROW: &str = "forward";

/// A storage error is never read as "no row": the default is returned only
/// when the row really is absent.
async fn load<H: AppHost>(host: &H) -> Result<ForwardSettings, Response> {
    let row = AppDataLayer::get(host, SETTINGS.to_string(), FORWARD_ROW.to_string())
        .await
        .map_err(|e| Response::internal_error(e.to_string()))?;
    let Some(row) = row else {
        return Ok(ForwardSettings::default());
    };
    serde_json::from_slice(&row.payload).map_err(|e| Response::internal_error(e.to_string()))
}

pub(crate) async fn settings<H: AppHost>(host: &H, input: SettingsInput) -> Reply<ForwardSettings> {
    ensure_settings(host).await.map_err(Response::internal_error)?;
    let current = load(host).await?;

    if input.default_mode.is_none() && input.asked.is_none() {
        return Ok(current);
    }
    if input.asked == Some(false) {
        return Err(ParamError {
            field: Some("asked".into()),
            message: "the question is asked only once".into(),
        }
        .into_response());
    }

    let next = ForwardSettings {
        default_mode: input.default_mode.unwrap_or(current.default_mode),
        asked: input.asked.unwrap_or(current.asked),
    };
    let payload = serde_json::to_vec(&next).map_err(|e| Response::internal_error(e.to_string()))?;
    AppDataLayer::put(
        host,
        SETTINGS.to_string(),
        RecordWriteValue { id: FORWARD_ROW.to_string(), payload },
    )
    .await
    .map_err(|e| Response::internal_error(e.to_string()))?;
    Ok(next)
}

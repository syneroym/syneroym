//! Backup import for saved deal defaults.

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::envelope::Response;

use crate::app::{
    DEAL_DEFAULTS,
    defaults_ops::{self, DEFAULT_ROW_ID, DealDefaults},
    put_row,
    refusal::refusal,
};

pub(crate) fn validate_defaults_section(
    defaults_rows: &[Value],
) -> Result<Option<DealDefaults>, Response> {
    let mut defaults = None;
    for r in defaults_rows {
        let payload = r.get("payload").ok_or_else(|| refusal("bad-deal-defaults"))?;
        let parsed: DealDefaults =
            serde_json::from_value(payload.clone()).map_err(|_| refusal("bad-deal-defaults"))?;
        defaults_ops::validate_defaults(&parsed)?;
        defaults = Some(parsed);
    }
    Ok(defaults)
}

pub(crate) async fn write_defaults_section<H: AppHost>(
    host: &H,
    defaults: Option<&DealDefaults>,
) -> Result<(), Response> {
    if let Some(d) = defaults {
        put_row(host, DEAL_DEFAULTS, DEFAULT_ROW_ID, d).await.map_err(Response::internal_error)?;
    }
    Ok(())
}

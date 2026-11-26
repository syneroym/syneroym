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

pub(crate) async fn import_defaults_section<H: AppHost>(
    host: &H,
    defaults_rows: &[Value],
) -> Result<(), Response> {
    for r in defaults_rows {
        let payload = r.get("payload").cloned().unwrap_or_else(|| r.clone());
        let defaults: DealDefaults =
            serde_json::from_value(payload).map_err(|_| refusal("bad-deal-defaults"))?;
        defaults_ops::validate_defaults(&defaults)?;
        put_row(host, DEAL_DEFAULTS, DEFAULT_ROW_ID, &defaults)
            .await
            .map_err(Response::internal_error)?;
    }
    Ok(())
}

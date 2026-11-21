//! `scheduling.export` and `scheduling.import`.
//!
//! The bundle holds the slots and the reservations. It cannot check a
//! reservation against an agreement: agreements are in the transaction
//! service's bundle. A restore keeps the two in step on the transaction side.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{Mutation, RecordWriteValue},
};
use syneroym_roym_core::{
    backup::{self, Bundle, ImportInput, SECTION_AVAILABILITY, SECTION_RESERVATIONS},
    clock,
    envelope::Response,
    method::{NoParams, Reply},
    signing,
};

use super::{
    AVAILABILITY, RESERVATIONS, SCHEMA_VERSION, ensure_availability, ensure_coll,
    ensure_reservations,
};

mod check;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulingImportOutput {
    pub imported: BTreeMap<String, usize>,
}

pub(crate) async fn export<H: AppHost>(host: &H, _: NoParams) -> Reply<Bundle> {
    let owner =
        signing::owner_did(host).await.map_err(|e| Response::internal_error(e.to_string()))?;
    let now = clock::now_secs();
    ensure_availability(host).await.map_err(Response::internal_error)?;
    ensure_reservations(host).await.map_err(Response::internal_error)?;
    let sections = backup::collect_sections(
        host,
        &[(SECTION_AVAILABILITY, AVAILABILITY), (SECTION_RESERVATIONS, RESERVATIONS)],
    )
    .await
    .map_err(Response::internal_error)?;
    backup::export_signed_bundle(host, owner, SCHEMA_VERSION, sections, now).await
}

pub(crate) async fn import<H: AppHost>(
    host: &H,
    input: ImportInput,
) -> Reply<SchedulingImportOutput> {
    let bundle = input.bundle;
    let now = clock::now_secs();
    let owner =
        signing::owner_did(host).await.map_err(|e| Response::internal_error(e.to_string()))?;
    backup::check_signed_bundle(&bundle, &owner, now)
        .map_err(|e| Response::invalid_params(e.to_string()))?;
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Err(Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            )));
        }
    }
    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        match name.as_str() {
            SECTION_AVAILABILITY => {
                prepared.push((AVAILABILITY, puts(check::availability_rows(records)?)?));
            }
            SECTION_RESERVATIONS => {
                let checked = check::reservation_rows(records)?;
                ensure_reservations(host).await.map_err(Response::internal_error)?;
                check::refuse_changed_rows(host, &checked).await?;
                prepared.push((RESERVATIONS, puts(checked)?));
            }
            other => return Err(Response::invalid_params(format!("unknown section '{other}'"))),
        }
    }
    let mut counts = BTreeMap::new();
    for (collection, muts) in prepared {
        ensure_coll(host, collection, &[]).await.map_err(Response::internal_error)?;
        counts.insert(collection.to_string(), muts.len());
        for chunk in muts.chunks(100) {
            AppDataLayer::batch_mutate(host, collection.to_string(), chunk.to_vec())
                .await
                .map_err(|e| Response::internal_error(e.to_string()))?;
        }
    }
    Ok(SchedulingImportOutput { imported: counts })
}

fn puts(rows: Vec<(String, Value)>) -> Result<Vec<Mutation>, Response> {
    let mut muts = Vec::with_capacity(rows.len());
    for (id, payload) in rows {
        let payload =
            serde_json::to_vec(&payload).map_err(|e| Response::internal_error(e.to_string()))?;
        muts.push(Mutation::Put(RecordWriteValue { id, payload }));
    }
    Ok(muts)
}

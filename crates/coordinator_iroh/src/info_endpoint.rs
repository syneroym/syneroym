use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorInfo {
    pub endpoint_addr_bytes: Vec<u8>, // serde_json-encoded iroh::EndpointAddr
    pub substrate_id: String,
    pub relay_url: Option<String>, // Local relay URL hosted by this coordinator
    pub parent_coordinator_url: Option<String>, // Parent coordinator URL
}

#[derive(Debug)]
pub struct InfoState {
    pub info: CoordinatorInfo,
}

pub async fn get_info(State(state): State<Arc<InfoState>>) -> Json<CoordinatorInfo> {
    Json(state.info.clone())
}

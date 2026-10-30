use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppHost, AppSigning};

use crate::{
    clock,
    envelope::Response,
    method::{NoParams, Reply},
    signing::{CertificateError, CertificateStatus, install, status},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SigningStatusOutput {
    pub signing_did: String,
    pub pubkey_hex: String,
    pub owner_did: Option<String>,
    pub certificate: CertificateStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallCertificateInput {
    pub certificate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallCertificateOutput {
    pub master_did: String,
    pub expires_at_secs: u64,
}

pub async fn signing_status<H: AppHost>(host: &H, _: NoParams) -> Reply<SigningStatusOutput> {
    let id = match AppSigning::signing_identity(host).await {
        Ok(i) => i,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    match status(host, clock::now_secs()).await {
        Ok(s) => Ok(SigningStatusOutput {
            signing_did: id.signing_did,
            pubkey_hex: id.pubkey_hex,
            owner_did: id.owner_did,
            certificate: s,
        }),
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

pub async fn install_signing_certificate<H: AppHost>(
    host: &H,
    input: InstallCertificateInput,
) -> Reply<InstallCertificateOutput> {
    match install(host, &input.certificate, clock::now_secs()).await {
        Ok(s) => Ok(InstallCertificateOutput {
            master_did: s.master_did,
            expires_at_secs: s.expires_at_secs,
        }),
        Err(CertificateError::Rejected(m)) => Err(Response::invalid_params(m)),
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

use std::mem;

use anyhow::{Result, anyhow};
use syneroym_rpc::CallerContext;
use tokio::{io::BufReader, time};
use tracing::debug;

use crate::{
    e2e_channel::OwnedStream,
    handshake::{HandshakeVerifier, now_secs},
    preamble::{IdentityFields, RoutePreamble},
    route_handler::{
        RouteHandler,
        io::{PRE_AUTH_READ_TIMEOUT, build_caller, read_line_bounded},
    },
};

pub(super) enum CallerCheck {
    Anonymous,
    Verified(Box<CallerContext>),
    Refused,
    /// The identity comes after the handshake, in the encrypted line.
    InChannel,
}

impl CallerCheck {
    /// The caller of an `Anonymous` or `Verified` check.
    pub(super) fn into_caller(self) -> Option<CallerContext> {
        match self {
            Self::Verified(c) => Some(*c),
            _ => None,
        }
    }
}

impl RouteHandler {
    /// The caller check before the encryption stage.
    pub(super) async fn check_plain_caller(&self, preamble: &RoutePreamble) -> CallerCheck {
        if !preamble.wants_e2e() {
            return self.verify_caller(preamble).await;
        }
        if preamble.has_identity_fields() {
            // A sender must keep its identity out of the plain-text line on
            // an encrypted stream; refusing at once shows such a sender its
            // mistake instead of working while it leaks.
            debug!(
                service_id = %preamble.service_id,
                "identity in plain text on an encrypted stream; refused"
            );
            return CallerCheck::Refused;
        }
        CallerCheck::InChannel
    }

    /// Reads the encrypted identity line, sets its fields on `preamble`,
    /// and checks them against the interface as the caller sent it.
    pub(super) async fn check_channel_caller(
        &self,
        stream: &mut OwnedStream,
        preamble: &mut RoutePreamble,
        sent_interface: &str,
    ) -> CallerCheck {
        let mut reader =
            BufReader::new(mem::replace(&mut stream.reader, Box::new(tokio::io::empty())));
        let line = time::timeout(PRE_AUTH_READ_TIMEOUT, read_line_bounded(&mut reader)).await;
        stream.reader = Box::new(reader);
        let fields = match line {
            Ok(Ok(raw)) => IdentityFields::parse_line(&raw),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("timed out reading the identity line")),
        };
        let fields = match fields {
            Ok(f) => f,
            Err(e) => {
                debug!(service_id = %preamble.service_id, "identity line refused: {e}");
                return CallerCheck::Refused;
            }
        };
        preamble.set_identity(fields);
        let mut signed_view = preamble.clone();
        signed_view.interface = sent_interface.to_string();
        self.verify_caller(&signed_view).await
    }

    pub(super) async fn verify_caller(&self, preamble: &RoutePreamble) -> CallerCheck {
        if !preamble.has_identity_fields() {
            return CallerCheck::Anonymous;
        }
        match self.check_caller(preamble).await {
            Ok(caller) => CallerCheck::Verified(Box::new(caller)),
            Err(e) => {
                // Log once at debug: forged streams must not flood the logs.
                debug!(service_id = %preamble.service_id, "caller refused: {e}");
                CallerCheck::Refused
            }
        }
    }

    pub(super) async fn check_caller(&self, preamble: &RoutePreamble) -> Result<CallerContext> {
        // `resolve-relation` answers for one principal: the `on_behalf`
        // person or the UCAN anchor, never a choice between two.
        if preamble.on_behalf.is_some() && preamble.ucan.is_some() {
            return Err(anyhow!("a preamble carries on_behalf or ucan, not both"));
        }
        let now = now_secs();
        let resolver = self.inner.registry_client.as_ref();
        let id = HandshakeVerifier::verify_preamble_at(preamble, resolver, now).await?;
        let mut caller = build_caller(
            preamble,
            &id,
            self.inner.admin_ucan_root.as_deref(),
            &self.inner.node_did,
            &self.inner.registry,
            resolver,
            self.inner.grant_resolve_to_node_did,
        )
        .await;
        if let Some(proof) = &preamble.on_behalf {
            let original = HandshakeVerifier::verify_on_behalf_at(proof, resolver, now).await?;
            caller.on_behalf_did = Some(original.master_did);
        }
        Ok(caller)
    }
}

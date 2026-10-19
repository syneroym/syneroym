//! The mailbox native service: handles JSON-RPC requests for interface
//! `mailbox`.

use std::{
    collections::HashMap,
    fmt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use metrics::counter;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use syneroym_core::{
    config::MailboxRole,
    dht_registry::{MasterAnchorResolver, RegistryClient},
    mailbox::{
        AckAnswer, AckParams, ClaimParams, InfoAnswer, MAX_CLAIM_BATCH, MAX_CLAIMS, METHOD_ACK,
        METHOD_CLAIM, METHOD_DEPOSIT, METHOD_INFO, METHOD_REGISTER, METHOD_STATUS,
        METHOD_UNREGISTER, PROTOCOL_VERSION, RegisterParams, StatusAnswer, StatusParams,
        UnregisterAnswer,
    },
};
use syneroym_mailbox_format::{
    Deposit, DepositId, MAX_SEALED_BYTES, SealedBytes, SealedBytesParseError,
};
use syneroym_passport::RequestLimiter;
use syneroym_rpc::{
    CallerContext, NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult,
};
use tokio::sync::Notify;

use crate::{
    caller::{Instance, check_not_revoked, instance_of, sender_of},
    error::MailboxError,
    limits::Limits,
    record::CheckedRecord,
    store::{MailboxStore, blocking},
    worker::MailboxWorker,
};

#[cfg(test)]
mod tests;

/// The native service handling mailbox JSON-RPC requests.
pub struct MailboxService {
    node_did: String,
    store: Arc<MailboxStore>,
    anchors: Arc<dyn MasterAnchorResolver>,
    per_sender: RequestLimiter<String>,
    total: RequestLimiter<()>,
    per_master_register: RequestLimiter<String>,
    total_register: RequestLimiter<()>,
    wake: Arc<Notify>,
    sweep_every: Duration,
    republish_every_secs: u64,
    revocation_cache: Mutex<HashMap<(String, String), u64>>,
}

impl fmt::Debug for MailboxService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailboxService").field("node_did", &self.node_did).finish()
    }
}

impl MailboxService {
    /// Opens the mailbox storage under `db_dir` and constructs the service.
    pub fn open(
        role: &MailboxRole,
        db_dir: &Path,
        node_did: &str,
        anchors: Arc<dyn MasterAnchorResolver>,
    ) -> anyhow::Result<Self> {
        let limits = Limits::from_role(role)?;
        let db_path = db_dir.join(&role.db_name);
        let store = Arc::new(MailboxStore::open(&db_path, limits)?);
        let per_sender = RequestLimiter::new(role.deposits_per_minute_per_sender, 10_000);
        let total = RequestLimiter::new(role.deposits_per_minute_total, 1);
        let per_master_register = RequestLimiter::new(role.register_per_minute_per_master, 10_000);
        let total_register = RequestLimiter::new(role.register_per_minute_total, 1);
        let wake = Arc::new(Notify::new());
        let sweep_every = Duration::from_secs(role.sweep_interval_secs);
        let republish_every_secs = role.republish_interval_secs;

        Ok(Self {
            node_did: node_did.to_string(),
            store,
            anchors,
            per_sender,
            total,
            per_master_register,
            total_register,
            wake,
            sweep_every,
            republish_every_secs,
            revocation_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Creates the background worker loop for this mailbox service.
    #[must_use]
    pub fn worker(&self, registry: Arc<RegistryClient>) -> MailboxWorker {
        MailboxWorker::new(
            self.store.clone(),
            registry,
            self.wake.clone(),
            self.sweep_every,
            self.republish_every_secs,
        )
    }

    #[cfg(test)]
    pub(crate) fn clear_revocation_cache(&self) {
        if let Ok(mut cache) = self.revocation_cache.lock() {
            cache.clear();
        }
    }

    async fn check_revocation(&self, who: &Instance, now: u64) -> Result<(), MailboxError> {
        let key = (who.master_did.clone(), who.instance_did.clone());
        if let Ok(cache) = self.revocation_cache.lock()
            && let Some(&expires) = cache.get(&key)
            && now < expires
        {
            return Ok(());
        }
        check_not_revoked(self.anchors.as_ref(), who).await?;
        if let Ok(mut cache) = self.revocation_cache.lock() {
            if cache.len() > 10_000 {
                cache.retain(|_, &mut exp| exp > now);
            }
            cache.insert(key, now.saturating_add(15));
        }
        Ok(())
    }

    async fn instance(&self, caller: &CallerContext, now: u64) -> Result<Instance, MailboxError> {
        let who = instance_of(caller, now)?;
        self.check_revocation(&who, now).await?;
        Ok(who)
    }

    fn sender(&self, caller: &CallerContext, now: u64) -> Result<String, MailboxError> {
        let who = sender_of(caller, now)?;
        Ok(who.master_did)
    }

    fn info(&self) -> Result<Value, MailboxError> {
        let limits = self.store.limits();
        answer(&InfoAnswer {
            version: PROTOCOL_VERSION,
            mailbox: self.node_did.clone(),
            limits: limits.queue,
            max_sealed_bytes: MAX_SEALED_BYTES,
            receipt_keep_secs: limits.receipt_keep_secs,
            visibility_secs: limits.visibility_secs,
            queue_lifetime_secs: limits.queue_lifetime_secs,
            max_claims: MAX_CLAIMS,
        })
    }

    async fn register(
        &self,
        caller: &CallerContext,
        params: Value,
        now: u64,
    ) -> Result<Value, MailboxError> {
        let who = instance_of(caller, now)?;
        if !self.per_master_register.allow_key(who.master_did.clone(), now)
            || !self.total_register.allow_key((), now)
        {
            counter!("substrate.mailbox.register_refused", "kind" => "rate_limited").increment(1);
            return Err(MailboxError::RateLimited);
        }
        self.check_revocation(&who, now).await?;
        let now = syneroym_passport::now_secs();
        let p: RegisterParams = parse(params)?;
        let record = CheckedRecord::check(&p.record, &who, &self.node_did, now)?;
        let r = blocking(&self.store, move |s| s.register(&who, &record, now)).await?;
        if r.publish_now {
            self.wake.notify_one();
        }
        answer(&r.answer)
    }

    async fn unregister(&self, caller: &CallerContext, now: u64) -> Result<Value, MailboxError> {
        let who = self.instance(caller, now).await?;
        let now = syneroym_passport::now_secs();
        let removed = blocking(&self.store, move |s| s.unregister(&who, now)).await?;
        answer(&UnregisterAnswer { removed })
    }

    async fn deposit(
        &self,
        caller: &CallerContext,
        params: Value,
        now: u64,
    ) -> Result<Value, MailboxError> {
        let sender = self.sender(caller, now)?;
        let deposit = parse_deposit(params)?;
        if !self.per_sender.allow_key(sender.clone(), now) || !self.total.allow_key((), now) {
            counter!("substrate.mailbox.deposits_refused", "kind" => "rate_limited").increment(1);
            return Err(MailboxError::RateLimited);
        }

        let res = blocking(&self.store, move |s| s.deposit(&sender, &deposit, now)).await;
        match res {
            Ok(out) => {
                counter!("substrate.mailbox.deposits").increment(1);
                if out.evicted > 0 {
                    counter!("substrate.mailbox.evictions").increment(u64::from(out.evicted));
                }
                answer(&out.receipt)
            }
            Err(e) => {
                counter!("substrate.mailbox.deposits_refused", "kind" => e.kind()).increment(1);
                Err(e)
            }
        }
    }

    async fn claim(
        &self,
        caller: &CallerContext,
        params: Value,
        now: u64,
    ) -> Result<Value, MailboxError> {
        let who = self.instance(caller, now).await?;
        let now = syneroym_passport::now_secs();
        let p: ClaimParams = parse(params)?;
        let max = p.max.unwrap_or(MAX_CLAIM_BATCH);
        let ans = blocking(&self.store, move |s| s.claim(&who, max, now)).await?;
        answer(&ans)
    }

    async fn ack(
        &self,
        caller: &CallerContext,
        params: Value,
        now: u64,
    ) -> Result<Value, MailboxError> {
        let who = self.instance(caller, now).await?;
        let now = syneroym_passport::now_secs();
        let p: AckParams = parse(params)?;
        let results = blocking(&self.store, move |s| s.ack(&who, &p.acks, now)).await?;
        answer(&AckAnswer { results })
    }

    async fn status(
        &self,
        caller: &CallerContext,
        params: Value,
        now: u64,
    ) -> Result<Value, MailboxError> {
        let sender = self.sender(caller, now)?;
        let p: StatusParams = parse(params)?;
        let receipts =
            blocking(&self.store, move |s| s.status(&sender, &p.deposit_ids, now)).await?;
        answer(&StatusAnswer { receipts })
    }
}

#[async_trait::async_trait]
impl NativeService for MailboxService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let NativeInvocation { method, params, caller, .. } = invocation;
        let now = syneroym_passport::now_secs();
        let payload = match method.as_str() {
            METHOD_INFO => self.info(),
            METHOD_REGISTER => self.register(&caller, params, now).await,
            METHOD_UNREGISTER => self.unregister(&caller, now).await,
            METHOD_DEPOSIT => self.deposit(&caller, params, now).await,
            METHOD_CLAIM => self.claim(&caller, params, now).await,
            METHOD_ACK => self.ack(&caller, params, now).await,
            METHOD_STATUS => self.status(&caller, params, now).await,
            other => return Err(RpcError::MethodNotFound(other.to_string())),
        }
        .map_err(RpcError::from)?;

        Ok(NativeResponse { payload })
    }
}

/// `DepositWire` exists because a `Deposit` whose `sealed` is too large fails
/// inside serde, and serde does not keep the error type; a separate error is
/// needed for size limit reporting.
#[derive(serde::Deserialize)]
struct DepositWire {
    destination: String,
    deposit_id: DepositId,
    ttl_secs: u64,
    sealed: String,
}

fn parse_deposit(params: Value) -> Result<Deposit, MailboxError> {
    let w: DepositWire = parse(params)?;
    let sealed = SealedBytes::try_from(w.sealed).map_err(|e| match e {
        SealedBytesParseError::TooLarge { bytes, max } => MailboxError::TooLarge { bytes, max },
        SealedBytesParseError::NotBase64 => {
            MailboxError::InvalidParams("sealed bytes are not valid base64".to_string())
        }
    })?;
    Ok(Deposit {
        destination: w.destination,
        deposit_id: w.deposit_id,
        ttl_secs: w.ttl_secs,
        sealed,
    })
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, MailboxError> {
    serde_json::from_value(params)
        .map_err(|e| MailboxError::InvalidParams(format!("failed to parse parameters: {e}")))
}

fn answer<T: Serialize>(value: &T) -> Result<Value, MailboxError> {
    serde_json::to_value(value)
        .map_err(|e| MailboxError::Internal(format!("failed to serialize answer: {e}")))
}

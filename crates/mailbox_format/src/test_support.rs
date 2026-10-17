//! Shared fixtures for the tests of this crate.

use serde_json::json;
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate::derive_did_key,
};

use crate::{MailboxPublicKey, MailboxSecretKey, MessageContent, Recipient, Sealed, Sender, seal};

/// The life of an instance certificate in these tests.
pub(crate) const CERT_LIFE_SECS: u64 = 3_600;
pub(crate) const TTL_SECS: u64 = 3_600;

/// A service member: a master, one instance key with a `service-instance`
/// certificate, and a mailbox key.
///
/// A certificate takes its time from the wall clock when it is issued, so
/// tests use times relative to `cert.issued_at_secs` and
/// `cert.expires_at_secs`, not fixed numbers.
pub(crate) struct Member {
    pub master: Identity,
    pub instance: Identity,
    pub cert: DelegationCertificate,
    pub mailbox: MailboxSecretKey,
    pub mailbox_public: MailboxPublicKey,
}

impl Member {
    pub(crate) fn new(lifetime_secs: u64) -> Self {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            lifetime_secs,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mailbox = MailboxSecretKey::generate().unwrap();
        let mailbox_public = mailbox.public_key();
        Self { master, instance, cert, mailbox, mailbox_public }
    }

    pub(crate) fn service_id(&self) -> String {
        derive_did_key(&self.master.public_key())
    }

    pub(crate) fn sender(&self) -> Sender<'_> {
        Sender { instance: &self.instance, certificate: &self.cert }
    }

    pub(crate) fn recipient<'a>(&'a self, id: &'a str) -> Recipient<'a> {
        Recipient { service_id: id, mailbox_key: &self.mailbox_public }
    }

    /// A time inside this member's certificate window.
    pub(crate) fn at(&self) -> u64 {
        self.cert.issued_at_secs + 10
    }
}

/// A time inside the certificate window of every member given.
pub(crate) fn common_now(members: &[&Member]) -> u64 {
    members.iter().map(|m| m.at()).max().unwrap()
}

pub(crate) fn content() -> MessageContent {
    MessageContent {
        interface: "conversation".to_string(),
        method: "deliver".to_string(),
        params: json!({
            "to": {"room": "r-1", "ünï": [1, 2, 3]},
            "ratio": 0.1,
            "text": "hello world",
        }),
        idempotency_key: "msg-1".to_string(),
    }
}

/// `from` seals the standard content to `to`, at a time both certificates
/// cover. Returns the deposit and that time.
pub(crate) fn seal_between(from: &Member, to: &Member) -> (Sealed, u64) {
    let now = common_now(&[from, to]);
    let id = to.service_id();
    let sealed = seal(&content(), to.recipient(&id), from.sender(), TTL_SECS, now).unwrap();
    (sealed, now)
}

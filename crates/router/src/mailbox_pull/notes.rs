//! Reasons a service is not pulled, logged and counted once an hour.

use std::collections::HashMap;

use tokio::time::{Duration, Instant};
use tracing::{debug, warn};

/// How often the same note for the same service is a warning.
const NOTE_INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Note {
    NoCertificate,
    InstanceRefused,
    StaleInstance,
    NoKey,
    KeyUnreadable,
    NoRecord,
    RecordAndFactDiffer,
    RecordExpired,
    MailboxFull,
    ServedRecordDiffers,
}

impl Note {
    /// The metric label.
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::NoCertificate => "no_certificate",
            Self::InstanceRefused => "instance_refused",
            Self::StaleInstance => "stale_instance",
            Self::NoKey => "no_key",
            Self::KeyUnreadable => "key_unreadable",
            Self::NoRecord => "no_record",
            Self::RecordAndFactDiffer => "record_and_fact_differ",
            Self::RecordExpired => "record_expired",
            Self::MailboxFull => "mailbox_full",
            Self::ServedRecordDiffers => "served_record_differs",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::NoCertificate => {
                "the service has no valid instance certificate, so its mailbox is not pulled"
            }
            Self::InstanceRefused => {
                "the mailbox refused the instance certificate (revoked, or the clocks disagree); \
                 waiting for a new certificate"
            }
            Self::StaleInstance => {
                "a newer instance of this service holds the mailbox queue; waiting for a new \
                 certificate"
            }
            Self::NoKey => "the service has no mailbox key, so its mailbox is not pulled",
            Self::KeyUnreadable => "the mailbox key cannot be read now (vault locked?)",
            Self::NoRecord => "the service has no usable stored deploy record that names a mailbox",
            Self::RecordAndFactDiffer => {
                "the stored deploy record and the recorded mailbox differ; the stored record is \
                 used"
            }
            Self::RecordExpired => {
                "the mailbox refused the stored record as expired; the owner must deploy again to \
                 renew the record"
            }
            Self::MailboxFull => "the mailbox is full and takes no new queue",
            Self::ServedRecordDiffers => {
                "the record the registry serves names another mailbox than the stored record; the \
                 stored record is used"
            }
        }
    }
}

/// The last time each `(service, note)` was a warning.
#[derive(Default)]
pub(super) struct Notes {
    last: HashMap<(String, Note), Instant>,
}

impl Notes {
    /// Warns and counts the first time and then once an hour for each
    /// service and note; logs at debug in between. `true` when it warned.
    pub(super) fn note(&mut self, service_id: &str, note: Note, detail: &str) -> bool {
        let now = Instant::now();
        let key = (service_id.to_string(), note);
        let due = self.last.get(&key).is_none_or(|at| now.duration_since(*at) >= NOTE_INTERVAL);
        if !due {
            debug!(service_id, reason = note.reason(), detail, "{}", note.text());
            return false;
        }
        self.last.insert(key, now);
        metrics::counter!("substrate.mailbox_receiver.blocked", "reason" => note.reason())
            .increment(1);
        warn!(service_id, reason = note.reason(), detail, "{}", note.text());
        true
    }
}

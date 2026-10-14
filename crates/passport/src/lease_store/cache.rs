//! The in-memory index of the lease table and the deny list. Admission
//! checks read only this.

use std::collections::{HashMap, HashSet};

use super::Lease;
use crate::issue::PassportId;

#[derive(Debug, Default)]
pub(super) struct Cache {
    /// Passport -> subject -> lease.
    by_passport: HashMap<PassportId, HashMap<String, Lease>>,
    /// Subject -> the passports it has leases under.
    by_subject: HashMap<String, HashSet<PassportId>>,
    denied: HashSet<PassportId>,
}

impl Cache {
    pub(super) fn insert(&mut self, lease: Lease) {
        self.by_subject.entry(lease.subject.clone()).or_default().insert(lease.passport_id.clone());
        self.by_passport
            .entry(lease.passport_id.clone())
            .or_default()
            .insert(lease.subject.clone(), lease);
    }

    /// Remove every lease of `id`. Returns how many were removed.
    pub(super) fn remove_passport(&mut self, id: &PassportId) -> usize {
        let Some(leases) = self.by_passport.remove(id) else {
            return 0;
        };
        for subject in leases.keys() {
            self.unlink(subject, id);
        }
        leases.len()
    }

    /// Remove every lease that has ended at `now_secs`.
    pub(super) fn prune(&mut self, now_secs: u64) {
        let mut ended = Vec::new();
        self.by_passport.retain(|id, leases| {
            leases.retain(|subject, lease| {
                let running = now_secs < lease.lease_ends_at;
                if !running {
                    ended.push((subject.clone(), id.clone()));
                }
                running
            });
            !leases.is_empty()
        });
        for (subject, id) in ended {
            self.unlink(&subject, &id);
        }
    }

    fn unlink(&mut self, subject: &str, id: &PassportId) {
        if let Some(ids) = self.by_subject.get_mut(subject) {
            ids.remove(id);
            if ids.is_empty() {
                self.by_subject.remove(subject);
            }
        }
    }

    fn running_of<'a>(&'a self, subject: &str, now_secs: u64) -> impl Iterator<Item = &'a Lease> {
        self.by_subject
            .get(subject)
            .into_iter()
            .flatten()
            .filter_map(move |id| self.by_passport.get(id)?.get(subject))
            .filter(move |lease| now_secs < lease.lease_ends_at)
    }

    pub(super) fn is_admitted(&self, subject: &str, now_secs: u64) -> bool {
        self.running_of(subject, now_secs).next().is_some()
    }

    /// The running lease of `subject` with the newest registration; ties go
    /// to the later lease end, then to the smaller passport id, so the
    /// choice is the same every time.
    pub(super) fn lease_for(&self, subject: &str, now_secs: u64) -> Option<Lease> {
        self.running_of(subject, now_secs)
            .max_by(|a, b| {
                a.registered_at
                    .cmp(&b.registered_at)
                    .then(a.lease_ends_at.cmp(&b.lease_ends_at))
                    .then(b.passport_id.cmp(&a.passport_id))
            })
            .cloned()
    }

    /// How many subjects other than `subject` hold a running lease under
    /// `id`.
    pub(super) fn other_running_subjects(
        &self,
        id: &PassportId,
        subject: &str,
        now_secs: u64,
    ) -> u64 {
        let Some(leases) = self.by_passport.get(id) else {
            return 0;
        };
        let count = leases
            .values()
            .filter(|lease| lease.subject != subject && now_secs < lease.lease_ends_at)
            .count();
        u64::try_from(count).unwrap_or(u64::MAX)
    }

    /// Every running lease, sorted by passport id, then subject.
    pub(super) fn running(&self, now_secs: u64) -> Vec<Lease> {
        let mut all: Vec<Lease> = self
            .by_passport
            .values()
            .flat_map(HashMap::values)
            .filter(|lease| now_secs < lease.lease_ends_at)
            .cloned()
            .collect();
        all.sort_by(|a, b| a.passport_id.cmp(&b.passport_id).then(a.subject.cmp(&b.subject)));
        all
    }

    pub(super) fn deny(&mut self, id: PassportId) {
        self.denied.insert(id);
    }

    pub(super) fn is_denied(&self, id: &PassportId) -> bool {
        self.denied.contains(id)
    }
}

//! Rendezvous ranking. Each chooser gets its own stable order of a list.
//! When an entry leaves the list, only the choosers that picked it move.

use sha2::{Digest, Sha256};

use crate::curated_list::ListEntry;

/// `SHA-256(len(chooser) || chooser || len(id) || id)`, with each `len` a
/// 4-byte big-endian byte count. Not Rust's default hasher: its output may
/// change between compiler versions.
///
/// `chooser` is part of the result, so every caller must pass the same form:
/// for the coordinator list, the substrate DID exactly as
/// `substrate::derive_did_key` returns it (`did:key:h...`), never the Iroh
/// endpoint id; for the mailbox list, the picking service's `service_id` as
/// it appears in its endpoint record.
pub fn rank_key(chooser: &str, id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in [chooser, id] {
        // A string of 4 GiB or more cannot be an id; saturating keeps this total.
        let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
        hasher.update(len.to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().into()
}

/// Checks and normalises each entry, drops the bad ones and the duplicates,
/// and sorts the rest by `rank_key(chooser, id)`, smallest first. The first
/// entry is the choice; the rest is the failover order.
pub fn rank<E: ListEntry>(chooser: &str, entries: impl IntoIterator<Item = E>) -> Vec<E> {
    let mut keyed: Vec<([u8; 32], String, E)> = entries
        .into_iter()
        .filter_map(|mut e| match e.check() {
            Ok(id) => Some((rank_key(chooser, &id), id, e)),
            Err(err) => {
                tracing::debug!(error = %err, "skipping a bad list entry");
                None
            }
        })
        .collect();
    keyed.sort_by_key(|a| a.0);
    // Equal ids give equal keys, so duplicates are next to each other.
    keyed.dedup_by(|a, b| a.1 == b.1);
    keyed.into_iter().map(|(_, _, e)| e).collect()
}

use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow};

use super::types::{
    RangeRoutingTable, ResolvedTopology, ServiceId, ShardingStrategy, TopologyKey, TopologyMode,
};

// ─────────────────────────────────────────────────────────────
// Rendezvous hashing
// ─────────────────────────────────────────────────────────────

/// Select one member from `members` via deterministic BLAKE3 rendezvous
/// hashing.
///
/// `app_instance_id` and `service_name` form the domain separator context.
/// `routing_key` is the caller-supplied bytes.
///
/// Returns `None` if `members` is empty.
///
/// Tie-breaking (hash collision): lexicographic comparison of the canonical
/// `ServiceId` byte representation (highest wins).
pub fn rendezvous_select<'a>(
    members: &'a [ServiceId],
    app_instance_id: &[u8],
    service_name: &[u8],
    routing_key: &[u8],
) -> Option<&'a ServiceId> {
    use blake3::Hasher;

    let mut prefix_hasher = Hasher::new();

    prefix_hasher.update(&(app_instance_id.len() as u64).to_be_bytes());
    prefix_hasher.update(app_instance_id);

    prefix_hasher.update(&(service_name.len() as u64).to_be_bytes());
    prefix_hasher.update(service_name);

    prefix_hasher.update(&(routing_key.len() as u64).to_be_bytes());
    prefix_hasher.update(routing_key);

    members
        .iter()
        .map(|m| {
            let mut hasher = prefix_hasher.clone();
            let service_id_bytes = m.as_str().as_bytes();
            hasher.update(&(service_id_bytes.len() as u64).to_be_bytes());
            hasher.update(service_id_bytes);
            let score = *hasher.finalize().as_bytes();
            (score, m)
        })
        .max_by(|a, b| {
            // Primary: unsigned lexicographic comparison of 32-byte digests (highest wins).
            // Tie-break: lexicographic comparison of ServiceId bytes (highest wins).
            a.0.cmp(&b.0).then_with(|| a.1.as_str().as_bytes().cmp(b.1.as_str().as_bytes()))
        })
        .map(|(_, m)| m)
}

/// Helper to select a ServiceId using Range Sharding.
///
/// Validates the routing table dynamically to ensure the boundaries are valid
/// before routing.
pub fn range_select(table: &RangeRoutingTable, key: &[u8]) -> Result<ServiceId> {
    table.validate()?;

    let chunk = table
        .chunks
        .iter()
        .find(|c| {
            let after_start = c.start_key.as_deref().is_none_or(|start| key >= start);
            let before_end = c.end_key.as_deref().is_none_or(|end| key < end);
            after_start && before_end
        })
        .ok_or_else(|| anyhow!("No routing chunk found for key"))?;

    Ok(chunk.target.clone())
}

pub(crate) fn select_member(
    topology: &ResolvedTopology,
    routing_key: Option<&[u8]>,
    key: &TopologyKey,
) -> Result<ServiceId> {
    if topology.members.is_empty() {
        return Err(anyhow!("Topology has no eligible members"));
    }

    match topology.mode {
        TopologyMode::Singleton => {
            // Must have exactly one member by design; defensive guard.
            topology
                .members
                .first()
                .cloned()
                .ok_or_else(|| anyhow!("Singleton topology has no members"))
        }

        TopologyMode::Redundant => {
            if let Some(routing_key) = routing_key {
                // Keyed call: rendezvous hashing.
                rendezvous_select(
                    &topology.members,
                    key.app.as_str().as_bytes(),
                    key.service_name.as_str().as_bytes(),
                    routing_key,
                )
                .cloned()
                .ok_or_else(|| anyhow!("Redundant topology member selection failed"))
            } else {
                // Unkeyed call: round-robin.
                let idx = topology.rr_counter.fetch_add(1, Ordering::Relaxed) as usize
                    % topology.members.len();
                Ok(topology.members[idx].clone())
            }
        }

        TopologyMode::Sharded => {
            let routing_key = routing_key
                .ok_or_else(|| anyhow!("Sharded topology requires a routing_key for selection"))?;

            match &topology.sharding_strategy {
                Some(ShardingStrategy::RangeSharding(table)) => range_select(table, routing_key),
                Some(ShardingStrategy::HashSharding)
                | None
                | Some(ShardingStrategy::EntityTagSharding) => {
                    let effective_key = match &topology.sharding_strategy {
                        Some(ShardingStrategy::EntityTagSharding) => {
                            routing_key.split(|&b| b == 0).next().unwrap_or(routing_key)
                        }
                        _ => routing_key,
                    };

                    rendezvous_select(
                        &topology.members,
                        key.app.as_str().as_bytes(),
                        key.service_name.as_str().as_bytes(),
                        effective_key,
                    )
                    .cloned()
                    .ok_or_else(|| anyhow!("Sharded topology member selection failed"))
                }
            }
        }
    }
}

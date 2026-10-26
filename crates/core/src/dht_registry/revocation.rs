//! Revocation check for the edges of a delegation chain.

use std::collections::HashSet;

use super::MasterAnchorResolver;

/// Whether none of the `(issuer, audience)` edges is revoked: no issuer's
/// master anchor lists its audience as a revoked key.
///
/// This generalizes the delegation-certificate revocation check to a UCAN
/// chain. An unresolvable anchor counts as not revoked, the same as on the
/// delegation path, which only fails hard on a revoked key. Identical edges
/// (a chain that reuses the same proof) are resolved once: the chain size
/// is already bounded, and one network round trip per edge is enough.
pub async fn chain_edges_not_revoked(
    edges: &[(&str, &str)],
    resolver: &dyn MasterAnchorResolver,
) -> bool {
    let mut checked = HashSet::new();
    for &edge @ (issuer, audience) in edges {
        if !checked.insert(edge) {
            continue;
        }
        if let Ok(anchor) = resolver.resolve_master_anchor(issuer).await
            && anchor.revoked_keys.iter().any(|k| k == audience)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests;

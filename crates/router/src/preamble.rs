use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreamble {
    pub protocol: String,
    pub interface: String,
    pub service_id: String,
}

impl RoutePreamble {
    pub fn parse(raw: &str) -> Result<Self> {
        let (protocol, target) = raw
            .trim()
            .split_once("://")
            .ok_or_else(|| anyhow!("Invalid preamble format: {raw}"))?;
        let (interface, service_id) = target
            .split_once('.')
            .ok_or_else(|| anyhow!("Invalid preamble target format: {target}"))?;

        if protocol.is_empty() || interface.is_empty() || service_id.is_empty() {
            return Err(anyhow!("Incomplete preamble: {raw}"));
        }

        Ok(Self {
            protocol: protocol.to_string(),
            interface: interface.to_string(),
            service_id: service_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_route_preamble() {
        let parsed = RoutePreamble::parse("json-rpc://health.substrate-123\n").unwrap();
        assert_eq!(parsed.protocol, "json-rpc");
        assert_eq!(parsed.interface, "health");
        assert_eq!(parsed.service_id, "substrate-123");
    }
}

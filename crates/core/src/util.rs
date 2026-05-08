//! Common utilities and helpers.

/// Parses a string representing a size into a number of bytes.
///
/// Supports common suffixes like `Ki`, `Mi`, `Gi`, `K`, `M`, `G`.
/// If the string cannot be parsed as a number, it returns `default_if_unparseable`
/// multiplied by the parsed suffix multiplier.
pub fn parse_size_string(s: &str, default_if_unparseable: u64) -> u64 {
    let s = s.trim();
    let mut multiplier = 1;
    let num_str = if let Some(stripped) = s.strip_suffix("Gi") {
        multiplier = 1024 * 1024 * 1024;
        stripped
    } else if let Some(stripped) = s.strip_suffix("Mi") {
        multiplier = 1024 * 1024;
        stripped
    } else if let Some(stripped) = s.strip_suffix("Ki") {
        multiplier = 1024;
        stripped
    } else if let Some(stripped) = s.strip_suffix("G") {
        multiplier = 1000 * 1000 * 1000;
        stripped
    } else if let Some(stripped) = s.strip_suffix("M") {
        multiplier = 1000 * 1000;
        stripped
    } else if let Some(stripped) = s.strip_suffix("K") {
        multiplier = 1000;
        stripped
    } else {
        s
    };

    num_str.trim().parse::<u64>().unwrap_or(default_if_unparseable) * multiplier
}

/// Generates a short z32-encoded hash from the given data.
/// It uses SHA256 and takes the first 5 bytes, resulting in an 8-character string.
pub fn short_hash(data: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    z32::encode(&result[..5])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_string() {
        assert_eq!(parse_size_string("1024", 128), 1024);
        assert_eq!(parse_size_string("1K", 128), 1000);
        assert_eq!(parse_size_string("1Ki", 128), 1024);
        assert_eq!(parse_size_string("2 M", 128), 2000000);
        assert_eq!(parse_size_string("500Mi", 128), 500 * 1024 * 1024);
        assert_eq!(parse_size_string("invalidGi", 128), 128 * 1024 * 1024 * 1024);
    }
}

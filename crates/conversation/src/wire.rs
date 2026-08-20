//! `serde`'s array support tops out at 32 elements; a 64-byte ed25519
//! signature does not fit. This is the `#[serde(with = "wire::fixed_bytes")]`
//! helper for any fixed-size byte array field on a wire type.

pub mod fixed_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        d: D,
    ) -> Result<[u8; N], D::Error> {
        let v: Vec<u8> = Vec::deserialize(d)?;
        let len = v.len();
        v.try_into().map_err(|_| serde::de::Error::custom(format!("expected {N} bytes, got {len}")))
    }
}

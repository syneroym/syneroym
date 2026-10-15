//! Who may use the Iroh relay of a coordinator: `[roles.coordinator] access`.

use std::fmt;

use serde::{
    Deserialize, Serialize, Serializer,
    de::{self, Deserializer, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
};

/// Who may use the Iroh relay of a coordinator: `[roles.coordinator] access`.
///
/// Four forms parse. Parsing never widens a value to "everyone". The passport
/// forms need `[tls]` (`check_tls`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CoordinatorAccess {
    /// `access = "everyone"`: any Iroh endpoint. The default.
    #[default]
    Everyone,
    /// `access = "passport"`: only endpoints with a running passport lease.
    Passport,
    /// `access = ["<endpoint id>", ...]`: only the listed endpoints.
    List(Vec<String>),
    /// `access = { list = ["<endpoint id>", ...], passport = true }`: a listed
    /// endpoint, or one with a running passport lease.
    ListOrPassport(Vec<String>),
}

impl CoordinatorAccess {
    /// Whether a node with a running passport lease is admitted.
    #[must_use]
    pub const fn accepts_passports(&self) -> bool {
        matches!(self, Self::Passport | Self::ListOrPassport(_))
    }

    /// The listed endpoint ids. Empty for `Everyone` and `Passport`.
    #[must_use]
    pub fn list(&self) -> &[String] {
        match self {
            Self::List(ids) | Self::ListOrPassport(ids) => ids,
            Self::Everyone | Self::Passport => &[],
        }
    }

    /// The passport forms need `[tls]`: the passport endpoints carry the
    /// passport and are served only over HTTPS.
    ///
    /// # Errors
    /// For a passport form when `has_tls` is false.
    pub fn check_tls(&self, has_tls: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            has_tls || !self.accepts_passports(),
            "[roles.coordinator] access accepts passports, so [tls] must be set: the passport \
             endpoints are served only over HTTPS"
        );
        Ok(())
    }
}

const EXPECTED: &str =
    "\"everyone\", \"passport\", a list of endpoint ids, or { list = [...], passport = true }";

impl<'de> Deserialize<'de> for CoordinatorAccess {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(AccessVisitor)
    }
}

struct AccessVisitor;

impl<'de> Visitor<'de> for AccessVisitor {
    type Value = CoordinatorAccess;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(EXPECTED)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        match value {
            "everyone" => Ok(CoordinatorAccess::Everyone),
            "passport" => Ok(CoordinatorAccess::Passport),
            other => Err(E::custom(format!(
                "unknown [roles.coordinator] access value {other:?}; expected {EXPECTED}"
            ))),
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut ids = Vec::new();
        while let Some(id) = seq.next_element::<String>()? {
            ids.push(id);
        }
        Ok(CoordinatorAccess::List(ids))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let table = AccessTable::deserialize(MapAccessDeserializer::new(map))?;
        if !table.passport {
            return Err(de::Error::custom(
                "[roles.coordinator] access table with `passport = false`: write the plain list \
                 form `access = [...]` instead",
            ));
        }
        Ok(CoordinatorAccess::ListOrPassport(table.list))
    }
}

/// The table form, `{ list = [...], passport = true }`. Both keys are required.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessTable {
    list: Vec<String>,
    passport: bool,
}

#[derive(Serialize)]
struct AccessTableRef<'a> {
    list: &'a [String],
    passport: bool,
}

impl Serialize for CoordinatorAccess {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Everyone => serializer.serialize_str("everyone"),
            Self::Passport => serializer.serialize_str("passport"),
            Self::List(ids) => ids.serialize(serializer),
            Self::ListOrPassport(ids) => {
                let table = AccessTableRef { list: ids.as_slice(), passport: true };
                table.serialize(serializer)
            }
        }
    }
}

#[cfg(test)]
mod tests;

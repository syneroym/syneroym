//! Who may use the Iroh relay of a coordinator: `[roles.coordinator] access`.

use std::fmt;

use serde::{
    Deserialize, Serialize, Serializer,
    de::{self, Deserializer, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
};

/// Who may use the Iroh relay of a coordinator: `[roles.coordinator] access`.
///
/// Four forms parse. A coordinator may refuse to start with a form it cannot
/// enforce yet; parsing never widens a value to "everyone".
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

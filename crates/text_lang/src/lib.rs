//! Language tag parsing and validation in canonical syntax form.

use std::{fmt, str::FromStr};

use icu_locale_core::Locale;

#[cfg(test)]
mod tests;

/// Longest tag a record may carry. RFC 5646 section 4.4.1 asks a reader to
/// keep at least 35 characters.
pub const MAX_TAG_LEN: usize = 35;

/// A BCP 47 language tag in canonical syntax form. The CLDR form (`iw` to
/// `he`) is not used: it needs CLDR data in every guest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LangTag(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LangTagError {
    #[error("language tag is longer than {MAX_TAG_LEN} bytes")]
    TooLong,
    #[error("language tag '{0}' is not a well-formed BCP 47 tag")]
    Malformed(String),
    #[error("language tag '{given}' is not in canonical form; send '{canonical}'")]
    NotCanonical { given: String, canonical: String },
}

impl LangTag {
    /// Any well-formed tag, put in canonical form (`HI-latn` -> `hi-Latn`).
    pub fn parse(s: &str) -> Result<Self, LangTagError> {
        if s.len() > MAX_TAG_LEN {
            return Err(LangTagError::TooLong);
        }
        let loc = Locale::try_from_str(s).map_err(|_| LangTagError::Malformed(s.to_string()))?;
        let canonical = loc.to_string();
        if canonical.len() > MAX_TAG_LEN {
            return Err(LangTagError::TooLong);
        }
        Ok(Self(canonical))
    }

    /// Only a tag already in canonical form. This is the wire rule.
    pub fn parse_canonical(s: &str) -> Result<Self, LangTagError> {
        let parsed = Self::parse(s)?;
        if parsed.0 != s {
            return Err(LangTagError::NotCanonical { given: s.to_string(), canonical: parsed.0 });
        }
        Ok(parsed)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LangTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for LangTag {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for LangTag {
    type Err = LangTagError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl serde::Serialize for LangTag {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for LangTag {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_canonical(&s).map_err(serde::de::Error::custom)
    }
}

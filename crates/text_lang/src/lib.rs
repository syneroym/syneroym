//! Language tag parsing and validation in canonical syntax form.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::{collections::HashSet, fmt, str::FromStr};

use icu_locale_core::Locale;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[cfg(test)]
mod tests;

/// Longest tag a record may carry. RFC 5646 section 4.4.1 asks a reader to
/// keep at least 35 characters.
pub const MAX_TAG_LEN: usize = 35;

/// Most language tags a chat message carries, main language first.
pub const MAX_MESSAGE_LANGUAGES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TagListError {
    #[error("more than {0} languages")]
    TooMany(usize),
    #[error("the language '{0}' is listed twice")]
    Duplicate(String),
}

/// An ordered list of tags: at most `max`, no tag twice. Tags are
/// compared as canonical strings, so `hi` and `hi-IN` are two tags.
pub fn check_tag_list(tags: &[LangTag], max: usize) -> Result<(), TagListError> {
    if tags.len() > max {
        return Err(TagListError::TooMany(max));
    }
    let mut seen = HashSet::with_capacity(tags.len());
    for tag in tags {
        if !seen.insert(tag) {
            return Err(TagListError::Duplicate(tag.to_string()));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MessageLanguagesError {
    #[error(transparent)]
    Tag(#[from] LangTagError),
    #[error(transparent)]
    List(#[from] TagListError),
}

/// Parses each string with `LangTag::parse_canonical` and checks the list
/// against MAX_MESSAGE_LANGUAGES. The one check every holder of message
/// languages calls.
pub fn parse_message_languages(tags: &[String]) -> Result<Vec<LangTag>, MessageLanguagesError> {
    let mut parsed = Vec::with_capacity(tags.len());
    for tag in tags {
        parsed.push(LangTag::parse_canonical(tag)?);
    }
    check_tag_list(&parsed, MAX_MESSAGE_LANGUAGES)?;
    Ok(parsed)
}

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

impl Serialize for LangTag {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LangTag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_canonical(&s).map_err(de::Error::custom)
    }
}

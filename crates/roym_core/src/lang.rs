//! Language tags and translation rules for records.

use std::collections::HashSet;

pub use syneroym_text_lang::{LangTag, LangTagError};

pub const MAX_TRANSLATIONS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranslationError {
    #[error("more than {MAX_TRANSLATIONS} translations")]
    TooMany,
    #[error("a translation has the record's own language '{0}'")]
    SameAsRecord(String),
    #[error("two translations have the language '{0}'")]
    Duplicate(String),
    #[error("translation '{lang}': {field} is empty")]
    Empty { lang: String, field: &'static str },
    #[error("translation '{lang}': {field} is over {max} bytes")]
    TooLong { lang: String, field: &'static str, max: usize },
}

/// Checks that translations do not exceed the count limit, that none has
/// the record's own language, and that no two translations have the same
/// language.
pub fn check_translation_langs<'a>(
    record: &LangTag,
    langs: impl ExactSizeIterator<Item = &'a LangTag>,
) -> Result<(), TranslationError> {
    if langs.len() > MAX_TRANSLATIONS {
        return Err(TranslationError::TooMany);
    }
    let mut seen = HashSet::with_capacity(langs.len());
    for lang in langs {
        if lang == record {
            return Err(TranslationError::SameAsRecord(lang.to_string()));
        }
        if !seen.insert(lang) {
            return Err(TranslationError::Duplicate(lang.to_string()));
        }
    }
    Ok(())
}

/// Checks one translated field against length and optional requirement rules.
pub fn check_text(
    lang: &LangTag,
    field: &'static str,
    value: &str,
    max: usize,
    required: bool,
) -> Result<(), TranslationError> {
    if required && value.trim().is_empty() {
        return Err(TranslationError::Empty { lang: lang.to_string(), field });
    }
    if value.len() > max {
        return Err(TranslationError::TooLong { lang: lang.to_string(), field, max });
    }
    Ok(())
}

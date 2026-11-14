//! The only translation host in v1: it has no engine, so every call says
//! `unavailable` and the list of target languages is empty. The three
//! functions read no state, so the native build calls them directly and both
//! builds give the same answer.

use syneroym_wit_interfaces::translation_host::syneroym::translation::translation::{
    Detected, Host as TranslationHost, Translated, TranslationError,
};

use crate::host_capabilities::HostState;

impl HostState {
    /// Languages the engine can translate into. Empty: there is no engine.
    #[must_use]
    pub fn translation_targets() -> Vec<String> {
        Vec::new()
    }

    /// Detect the language of a text. There is no engine to ask.
    pub fn detect_language(_text: &str) -> Result<Vec<Detected>, TranslationError> {
        Err(TranslationError::Unavailable)
    }

    /// Translate a text. There is no engine to ask.
    pub fn translate(
        _text: &str,
        _source_lang: Option<&str>,
        _target_lang: &str,
    ) -> Result<Translated, TranslationError> {
        Err(TranslationError::Unavailable)
    }
}

impl TranslationHost for HostState {
    async fn supported_targets(&mut self) -> Vec<String> {
        Self::translation_targets()
    }

    async fn detect_language(&mut self, text: String) -> Result<Vec<Detected>, TranslationError> {
        Self::detect_language(&text)
    }

    async fn translate(
        &mut self,
        text: String,
        source_lang: Option<String>,
        target_lang: String,
    ) -> Result<Translated, TranslationError> {
        Self::translate(&text, source_lang.as_deref(), &target_lang)
    }
}

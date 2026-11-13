//! The native translation host. It calls the same no-engine functions as the
//! WASM build, and needs no lock because they read no state.

use syneroym_app_host::{
    AppTranslation,
    types::translation::{Detected, Translated, TranslationError},
};
use syneroym_sandbox_wasm::HostState;

use super::NativeAppHost;
use crate::convert;

impl AppTranslation for NativeAppHost {
    async fn supported_targets(&self) -> Vec<String> {
        HostState::translation_targets()
    }

    async fn detect_language(&self, text: String) -> Result<Vec<Detected>, TranslationError> {
        HostState::detect_language(&text)
            .map(|found| found.into_iter().map(convert::detected_guest).collect())
            .map_err(convert::translation_error_guest)
    }

    async fn translate(
        &self,
        text: String,
        source_lang: Option<String>,
        target_lang: String,
    ) -> Result<Translated, TranslationError> {
        HostState::translate(&text, source_lang.as_deref(), &target_lang)
            .map(convert::translated_guest)
            .map_err(convert::translation_error_guest)
    }
}

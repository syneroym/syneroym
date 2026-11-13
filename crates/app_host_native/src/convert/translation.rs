use syneroym_app_host::types::translation::{
    Detected as GuestDetected, EngineInfo as GuestEngineInfo, Translated as GuestTranslated,
    TranslationError as GuestTranslationError,
};
use syneroym_wit_interfaces::translation_host::syneroym::translation::translation::{
    Detected as HostDetected, Translated as HostTranslated,
    TranslationError as HostTranslationError,
};

pub(crate) fn translation_error_guest(e: HostTranslationError) -> GuestTranslationError {
    match e {
        HostTranslationError::Unavailable => GuestTranslationError::Unavailable,
        HostTranslationError::UnsupportedPair(pair) => GuestTranslationError::UnsupportedPair(pair),
        HostTranslationError::TooLong(limit) => GuestTranslationError::TooLong(limit),
        HostTranslationError::EngineFailed(msg) => GuestTranslationError::EngineFailed(msg),
    }
}

pub(crate) fn detected_guest(d: HostDetected) -> GuestDetected {
    GuestDetected { lang: d.lang, confidence: d.confidence }
}

pub(crate) fn translated_guest(t: HostTranslated) -> GuestTranslated {
    GuestTranslated {
        text: t.text,
        source_lang: t.source_lang,
        engine: GuestEngineInfo { name: t.engine.name, remote: t.engine.remote },
    }
}

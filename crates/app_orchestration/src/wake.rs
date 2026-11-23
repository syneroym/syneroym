//! Node-local wake manifest surface (ADR-0031). A wake names an interface and
//! a method on a logical service, an optional parameter string, an optional
//! list of event kinds, an interval, and a timeout. The hosting substrate
//! invokes the method on events and on a timer.

use serde::{Deserialize, Serialize};

use crate::models::InterfaceName;

pub const MIN_WAKE_EVERY_SECS: u32 = 5;
pub const MAX_WAKE_EVERY_SECS: u32 = 86_400;
pub const MAX_WAKE_EVENT_KINDS: usize = 8;
pub const DEFAULT_WAKE_TIMEOUT_MS: u32 = 10_000;
pub const MAX_WAKE_TIMEOUT_MS: u32 = 30_000;

const fn default_wake_timeout_ms() -> u32 {
    DEFAULT_WAKE_TIMEOUT_MS
}

/// A node-local call the hosting substrate makes on events and on a timer.
/// See ADR-0031.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WakeSpec {
    pub interface: InterfaceName,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    /// Event kinds of this app that trigger a call. May be empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_events: Vec<String>,
    /// The call also runs when this much time passed since the last one.
    pub every_secs: u32,
    #[serde(default = "default_wake_timeout_ms")]
    pub timeout_ms: u32,
}

impl WakeSpec {
    /// Validates the wake specification against the service's declared
    /// interfaces.
    pub fn validate(&self, interfaces: &[InterfaceName]) -> Result<(), String> {
        if !interfaces.contains(&self.interface) {
            return Err(format!(
                "wake interface '{}' is not one of the service's declared interfaces",
                self.interface
            ));
        }
        if self.method.trim().is_empty() {
            return Err("wake method must not be empty".to_string());
        }
        if let Some(params) = &self.params
            && let Err(err) = serde_json::from_str::<serde_json::Value>(params)
        {
            return Err(format!("wake params must be valid JSON: {err}"));
        }
        if self.on_events.len() > MAX_WAKE_EVENT_KINDS {
            return Err(format!(
                "wake declares {} event kinds, exceeding the limit of {MAX_WAKE_EVENT_KINDS}",
                self.on_events.len()
            ));
        }
        for event in &self.on_events {
            if event.is_empty()
                || event.len() > 64
                || !event
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
            {
                return Err(format!(
                    "wake event kind '{event}' is invalid: must be 1 to 64 chars of lowercase \
                     letters, digits, '.' or '-'"
                ));
            }
        }
        if !(MIN_WAKE_EVERY_SECS..=MAX_WAKE_EVERY_SECS).contains(&self.every_secs) {
            return Err(format!(
                "wake every_secs must be between {MIN_WAKE_EVERY_SECS} and {MAX_WAKE_EVERY_SECS}, \
                 got {}",
                self.every_secs
            ));
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_WAKE_TIMEOUT_MS {
            return Err(format!(
                "wake timeout_ms must be between 1 and {MAX_WAKE_TIMEOUT_MS}, got {}",
                self.timeout_ms
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> WakeSpec {
        WakeSpec {
            interface: InterfaceName::new("syneroym-roym:transaction/api@0.1.0"),
            method: "invoke".to_string(),
            params: Some("{\"method\":\"transaction.drive\",\"params\":{}}".to_string()),
            on_events: vec!["conversation.message".to_string()],
            every_secs: 30,
            timeout_ms: 10_000,
        }
    }

    fn interfaces() -> Vec<InterfaceName> {
        vec![InterfaceName::new("syneroym-roym:transaction/api@0.1.0")]
    }

    #[test]
    fn wake_validation_accepts_valid_spec() {
        assert!(valid_spec().validate(&interfaces()).is_ok());
    }

    #[test]
    fn wake_validation_refuses_undeclared_interface() {
        let mut spec = valid_spec();
        spec.interface = InterfaceName::new("syneroym-roym:other/api@0.1.0");
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("not one of the service's declared interfaces"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_empty_method() {
        let mut spec = valid_spec();
        spec.method = "   ".to_string();
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("method must not be empty"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_invalid_params_json() {
        let mut spec = valid_spec();
        spec.params = Some("invalid json".to_string());
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("params must be valid JSON"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_too_many_event_kinds() {
        let mut spec = valid_spec();
        spec.on_events = (0..=MAX_WAKE_EVENT_KINDS).map(|i| format!("event-{i}")).collect();
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("exceeding the limit"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_empty_event_kind() {
        let mut spec = valid_spec();
        spec.on_events = vec!["".to_string()];
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("event kind '' is invalid"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_invalid_event_kind_characters() {
        let mut spec = valid_spec();
        spec.on_events = vec!["Invalid_Event".to_string()];
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("event kind 'Invalid_Event' is invalid"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_too_long_event_kind() {
        let mut spec = valid_spec();
        spec.on_events = vec!["a".repeat(65)];
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("is invalid: must be 1 to 64 chars"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_too_small_every_secs() {
        let mut spec = valid_spec();
        spec.every_secs = MIN_WAKE_EVERY_SECS - 1;
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("every_secs must be between"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_too_large_every_secs() {
        let mut spec = valid_spec();
        spec.every_secs = MAX_WAKE_EVERY_SECS + 1;
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("every_secs must be between"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_zero_timeout_ms() {
        let mut spec = valid_spec();
        spec.timeout_ms = 0;
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("timeout_ms must be between 1 and"), "{err}");
    }

    #[test]
    fn wake_validation_refuses_too_large_timeout_ms() {
        let mut spec = valid_spec();
        spec.timeout_ms = MAX_WAKE_TIMEOUT_MS + 1;
        let err = spec.validate(&interfaces()).unwrap_err();
        assert!(err.contains("timeout_ms must be between 1 and"), "{err}");
    }
}

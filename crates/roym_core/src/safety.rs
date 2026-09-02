//! The safety rules, as arithmetic. No host, no clock, no storage:
//! everything takes its limits and its `now_secs` as parameters, so every
//! rule is a unit test away from proven and both builds run the identical
//! function.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitsError {
    #[error("window_secs must be between {min} and {max}")]
    WindowOutOfBounds { min: u64, max: u64 },
    #[error("max_per_window must be at most {max}")]
    MaxPerWindowCeiling { max: u32 },
}

/// The recipient's own ceiling on unsolicited first contact. Settable,
/// because the requirement is "rate-limited per sender **and controllable
/// by the recipient**" -- a constant would meet half of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactLimits {
    pub window_secs: u64,
    pub max_per_window: u32,
}

impl Default for ContactLimits {
    fn default() -> Self {
        Self { window_secs: 24 * 60 * 60, max_per_window: 3 }
    }
}

impl ContactLimits {
    pub const MIN_WINDOW_SECS: u64 = 60;
    pub const MAX_WINDOW_SECS: u64 = 30 * 24 * 60 * 60;
    pub const MAX_PER_WINDOW_CEILING: u32 = 1000;
    /// A recipient may loosen or tighten, within bounds a mistyped value
    /// cannot escape. `max_per_window == 0` is allowed and means "no
    /// unsolicited first contact at all".
    pub fn validate(&self) -> Result<(), LimitsError> {
        if self.window_secs < Self::MIN_WINDOW_SECS || self.window_secs > Self::MAX_WINDOW_SECS {
            return Err(LimitsError::WindowOutOfBounds {
                min: Self::MIN_WINDOW_SECS,
                max: Self::MAX_WINDOW_SECS,
            });
        }
        if self.max_per_window > Self::MAX_PER_WINDOW_CEILING {
            return Err(LimitsError::MaxPerWindowCeiling { max: Self::MAX_PER_WINDOW_CEILING });
        }
        Ok(())
    }
}

/// Same shape for listing publication. No writer in this slice: the
/// producer is the catalog's publish path and the directory's admission,
/// neither of which exists yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationLimits {
    pub window_secs: u64,
    pub max_per_window: u32,
}

impl Default for PublicationLimits {
    fn default() -> Self {
        Self { window_secs: 24 * 60 * 60, max_per_window: 20 }
    }
}

impl PublicationLimits {
    pub const MIN_WINDOW_SECS: u64 = 60;
    pub const MAX_WINDOW_SECS: u64 = 30 * 24 * 60 * 60;
    pub const MAX_PER_WINDOW_CEILING: u32 = 1000;

    pub fn validate(&self) -> Result<(), LimitsError> {
        if self.window_secs < Self::MIN_WINDOW_SECS || self.window_secs > Self::MAX_WINDOW_SECS {
            return Err(LimitsError::WindowOutOfBounds {
                min: Self::MIN_WINDOW_SECS,
                max: Self::MAX_WINDOW_SECS,
            });
        }
        if self.max_per_window > Self::MAX_PER_WINDOW_CEILING {
            return Err(LimitsError::MaxPerWindowCeiling { max: Self::MAX_PER_WINDOW_CEILING });
        }
        Ok(())
    }
}

/// Never a bare `bool`: a refusal must carry why, because the refusal has
/// to be visible to the sender, and "blocked" and "too many, try later"
/// are different things to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "admission", rename_all = "kebab-case")]
pub enum Admission {
    Allow,
    Blocked,
    RateLimited { retry_after_secs: u64 },
}

fn admit_windowed(
    prior_secs: &[u64],
    window_secs: u64,
    max_per_window: u32,
    now_secs: u64,
) -> Admission {
    if max_per_window == 0 {
        return Admission::RateLimited { retry_after_secs: window_secs };
    }
    let floor = now_secs.saturating_sub(window_secs);
    let mut inside: Vec<u64> = prior_secs.iter().copied().filter(|t| *t > floor).collect();
    if inside.len() < max_per_window as usize {
        return Admission::Allow;
    }
    inside.sort_unstable();
    // The oldest attempt still inside the window is the one that must age
    // out before another is admitted.
    let oldest = inside[inside.len() - max_per_window as usize];
    Admission::RateLimited {
        retry_after_secs: (oldest + window_secs).saturating_sub(now_secs).max(1),
    }
}

/// `attempts_secs` is every prior first-contact attempt by this sender,
/// in any order. Only those inside the window count.
pub fn admit_first_contact(
    blocked: bool,
    attempts_secs: &[u64],
    limits: &ContactLimits,
    now_secs: u64,
) -> Admission {
    if blocked {
        return Admission::Blocked;
    }
    admit_windowed(attempts_secs, limits.window_secs, limits.max_per_window, now_secs)
}

pub fn admit_publication(
    prior_secs: &[u64],
    limits: &PublicationLimits,
    now_secs: u64,
) -> Admission {
    admit_windowed(prior_secs, limits.window_secs, limits.max_per_window, now_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history_allows() {
        let limits = ContactLimits::default();
        assert_eq!(admit_first_contact(false, &[], &limits, 1000), Admission::Allow);
    }

    #[test]
    fn blocked_beats_clean_history() {
        let limits = ContactLimits::default();
        assert_eq!(admit_first_contact(true, &[], &limits, 1000), Admission::Blocked);
    }

    #[test]
    fn max_per_window_zero_refuses_everything() {
        let limits = ContactLimits { window_secs: 3600, max_per_window: 0 };
        assert_eq!(
            admit_first_contact(false, &[], &limits, 1000),
            Admission::RateLimited { retry_after_secs: 3600 }
        );
    }

    #[test]
    fn exactly_at_ceiling_refuses() {
        let limits = ContactLimits { window_secs: 100, max_per_window: 2 };
        let history = vec![950, 980];
        let now = 1000;
        let res = admit_first_contact(false, &history, &limits, now);
        // oldest inside window (floor = 900) is 950. 950 + 100 - 1000 = 50.
        assert_eq!(res, Admission::RateLimited { retry_after_secs: 50 });
    }

    #[test]
    fn attempt_one_second_outside_window_does_not_count() {
        let limits = ContactLimits { window_secs: 100, max_per_window: 2 };
        // floor is 1000 - 100 = 900. 900 is outside floor (*t > floor).
        let history = vec![900, 950];
        let now = 1000;
        assert_eq!(admit_first_contact(false, &history, &limits, now), Admission::Allow);
    }

    #[test]
    fn retry_after_secs_is_never_zero() {
        let limits = ContactLimits { window_secs: 100, max_per_window: 1 };
        let history = vec![1000];
        let now = 1000;
        let res = admit_first_contact(false, &history, &limits, now);
        assert_eq!(res, Admission::RateLimited { retry_after_secs: 100 });

        let history_future = vec![1050];
        let res_fut = admit_first_contact(false, &history_future, &limits, now);
        if let Admission::RateLimited { retry_after_secs } = res_fut {
            assert!(retry_after_secs >= 1);
        } else {
            panic!("expected RateLimited");
        }
    }

    #[test]
    fn limits_validation_rejects_out_of_bounds() {
        let too_small = ContactLimits { window_secs: 10, max_per_window: 1 };
        assert!(matches!(too_small.validate(), Err(LimitsError::WindowOutOfBounds { .. })));

        let too_large = ContactLimits { window_secs: 40 * 24 * 3600, max_per_window: 1 };
        assert!(matches!(too_large.validate(), Err(LimitsError::WindowOutOfBounds { .. })));

        let max_exceeded = ContactLimits { window_secs: 3600, max_per_window: 2000 };
        assert!(matches!(max_exceeded.validate(), Err(LimitsError::MaxPerWindowCeiling { .. })));

        let valid = ContactLimits::default();
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn publication_limits_and_admission_work() {
        let limits = PublicationLimits::default();
        assert!(limits.validate().is_ok());

        let invalid = PublicationLimits { window_secs: 10, max_per_window: 1 };
        assert!(matches!(invalid.validate(), Err(LimitsError::WindowOutOfBounds { .. })));

        let empty: Vec<u64> = vec![];
        assert_eq!(admit_publication(&empty, &limits, 1000), Admission::Allow);

        let zero_limit = PublicationLimits { window_secs: 600, max_per_window: 0 };
        assert_eq!(
            admit_publication(&empty, &zero_limit, 1000),
            Admission::RateLimited { retry_after_secs: 600 }
        );

        let capped_limit = PublicationLimits { window_secs: 100, max_per_window: 2 };
        let history = vec![950, 980];
        assert_eq!(
            admit_publication(&history, &capped_limit, 1000),
            Admission::RateLimited { retry_after_secs: 50 }
        );
    }
}

//! Scheduled-task manifest surface (ADR-0023 §6). A schedule names an
//! interface and method on a logical service and a cron expression; the
//! supervisor evaluates it on its own reconcile pass and fires the method on
//! exactly one member.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::InterfaceName;

/// A manifest's or plan's count of services carrying a `schedule` above this
/// is refused, both at `SynAppManifest::validate()` and again at the
/// interface that accepts an already-compiled plan. Follows `MAX_REPLICAS`'s
/// precedent: a bound set before the first measurement can never fail.
pub const MAX_SCHEDULED_SERVICES: usize = 16;

/// A scheduled run's default budget. Deliberately *not*
/// `DEFAULT_PROBE_TIMEOUT_MS` (2 s): a probe is a liveness ping, and a
/// schedule is not a probe. 2 s is below `dispatch_epoch_timeout_secs`
/// (5 s), so inheriting it would have the supervisor kill a run at 2 s that
/// the guest's own budget permits to run for 5 -- work the platform allows,
/// stopped by the component that asked for it. 10 s covers that budget plus
/// a round trip.
pub const DEFAULT_SCHEDULE_TIMEOUT_MS: u32 = 10_000;

/// The largest budget a schedule may ask for. A scheduled run is awaited
/// inline inside a reconcile pass, and every other app instance this
/// supervisor manages waits behind it, so the bound belongs to the
/// supervisor -- but it is enforced at manifest validation, where the
/// author can still read the refusal. The supervisor clamps to the same
/// value as a second line of defence for a hand-edited plan that never
/// passed through `validate`.
pub const MAX_SCHEDULE_TIMEOUT_MS: u32 = 30_000;

const fn default_schedule_timeout_ms() -> u32 {
    DEFAULT_SCHEDULE_TIMEOUT_MS
}

/// When a service's method runs, and which method. Declared per logical
/// service; exactly one member runs a given tick.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleSpec {
    /// Standard five-field cron, evaluated in UTC. UTC rather than a
    /// configurable zone because a zone makes every occurrence a DST
    /// question, and there is no operator-facing zone setting anywhere else
    /// in a manifest to be consistent with.
    pub cron: String,
    /// The interface the method is exported on. Must be one of the
    /// service's own declared `interfaces`.
    pub interface: InterfaceName,
    pub method: String,
    /// JSON text passed verbatim as the call's params. Absent sends an
    /// empty positional array -- the shape the one existing in-tree caller
    /// of a no-argument guest method sends (the `rpc` readiness probe), not
    /// `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    /// How long one run may take. Same *field* as `RpcProbe.timeout_ms` and
    /// a different default: a probe's 2 s is a liveness ping and is below
    /// the guest's own 5 s execution budget. Clamped at
    /// `SCHEDULED_RUN_CEILING` by the supervisor, since this call is
    /// awaited inside a reconcile pass other app instances are queued
    /// behind.
    #[serde(default = "default_schedule_timeout_ms")]
    pub timeout_ms: u32,
}

impl ScheduleSpec {
    /// Parses `cron`. Fails with the parser's own message, which names the
    /// offending field.
    pub fn parsed(&self) -> Result<croner::Cron> {
        self.cron
            .parse::<croner::Cron>()
            .map_err(|e| anyhow!("schedule cron '{}' does not parse: {e}", self.cron))
    }
}

/// Whether `cron` has at least one occurrence in the half-open interval
/// `(after, until]`, both Unix seconds, UTC.
pub fn has_occurrence_in(cron: &croner::Cron, after: u64, until: u64) -> Result<bool> {
    if after >= until {
        // The clock went backwards, or the window is empty.
        return Ok(false);
    }
    let start = DateTime::<Utc>::from_timestamp(i64::try_from(after)?, 0)
        .ok_or_else(|| anyhow!("timestamp {after} is out of range"))?;
    let next = cron
        .find_next_occurrence(&start, false)
        .map_err(|e| anyhow!("cron evaluation failed: {e}"))?;
    Ok(u64::try_from(next.timestamp())? <= until)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn spec(cron: &str) -> ScheduleSpec {
        ScheduleSpec {
            cron: cron.to_string(),
            interface: InterfaceName::new("scheduled-driver"),
            method: "tick".to_string(),
            params: None,
            timeout_ms: default_schedule_timeout_ms(),
        }
    }

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
        u64::try_from(Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap().timestamp()).unwrap()
    }

    #[test]
    fn has_occurrence_in_is_true_for_a_daily_cron_across_its_own_hour() {
        let cron = spec("0 3 * * *").parsed().unwrap();
        let after = ts(2026, 1, 1, 0, 0, 0);
        let until = ts(2026, 1, 1, 6, 0, 0);
        assert!(has_occurrence_in(&cron, after, until).unwrap());
    }

    #[test]
    fn has_occurrence_in_is_false_for_a_window_that_misses_the_occurrence() {
        let cron = spec("0 3 * * *").parsed().unwrap();
        let after = ts(2026, 1, 1, 4, 0, 0);
        let until = ts(2026, 1, 1, 5, 0, 0);
        assert!(!has_occurrence_in(&cron, after, until).unwrap());
    }

    #[test]
    fn has_occurrence_in_is_false_when_the_window_is_empty_or_inverted() {
        let cron = spec("* * * * *").parsed().unwrap();
        let now = ts(2026, 1, 1, 0, 0, 0);
        assert!(!has_occurrence_in(&cron, now, now).unwrap());
        assert!(!has_occurrence_in(&cron, now, now - 1).unwrap());
    }

    #[test]
    fn a_five_field_crontab_line_parses_as_the_hour_an_operator_meant() {
        // "0 3 * * *" is minute=0, hour=3 -- 03:00, not 00:03 and not
        // 03:00's second field misread.
        let cron = spec("0 3 * * *").parsed().unwrap();
        let just_before =
            DateTime::<Utc>::from_timestamp(ts(2026, 1, 1, 0, 0, 0) as i64, 0).unwrap();
        let next = cron.find_next_occurrence(&just_before, false).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 1, 1, 3, 0, 0).unwrap());
    }

    #[test]
    fn a_six_field_expression_is_read_as_seconds_first() {
        // A six-field expression's first field is seconds, not minutes --
        // "30 0 3 * * *" is 03:00:30, not 03:30.
        let cron: croner::Cron = "30 0 3 * * *".parse().unwrap();
        let just_before =
            DateTime::<Utc>::from_timestamp(ts(2026, 1, 1, 0, 0, 0) as i64, 0).unwrap();
        let next = cron.find_next_occurrence(&just_before, false).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 1, 1, 3, 0, 30).unwrap());
    }

    #[test]
    fn a_schedule_timeout_defaults_above_the_guests_own_execution_budget() {
        // Pinned as a relationship against the substrate's dispatch epoch
        // timeout (5s), not as a literal, so the two cannot drift into the
        // supervisor killing runs the guest is allowed to finish.
        const DISPATCH_EPOCH_TIMEOUT_SECS: u64 = 5;
        assert!(
            u64::from(DEFAULT_SCHEDULE_TIMEOUT_MS) > DISPATCH_EPOCH_TIMEOUT_SECS * 1_000,
            "DEFAULT_SCHEDULE_TIMEOUT_MS must exceed the guest's own dispatch epoch timeout"
        );
    }
}

//! Shared vocabulary for the SynOrg / Directory service's two halves: the
//! server half (a SynOrg's own settings, roster, publications and search)
//! and the client half (this person's list of directories, the fan-out,
//! and the merge). Every number here is an integer, because `settings` is
//! unsigned today and might not stay that way.

use serde::{Deserialize, Serialize};

use crate::{area::Area, safety::PublicationLimits};

pub const DIRECTORY_SCHEMA_VERSION: u32 = 2;
pub const MAX_SYNORG_NAME_LEN: usize = 128;
pub const MAX_RULES_LEN: usize = 8192;
pub const MAX_CONTACT_LEN: usize = 256;
pub const MAX_DISPUTE_PATH_LEN: usize = 2048;
pub const MAX_CATEGORIES: usize = 32;
pub const MAX_CATEGORY_LEN: usize = 64;
pub const MAX_AREAS: usize = 8;
pub const MIN_RETENTION_SECS: u64 = 3600;
pub const MAX_RETENTION_SECS: u64 = 5 * 365 * 24 * 3600;

/// The **merged** page cap. Distinct from `MAX_HITS_PER_SOURCE` on
/// purpose: one constant serving as both was the ambiguity that let a
/// single source fill a page.
pub const MAX_SEARCH_RESULTS: u32 = 50;
/// What any one directory may contribute to a merged page, however many
/// validly signed recent listings it holds.
pub const MAX_HITS_PER_SOURCE: u32 = 10;
/// What one directory may return for one query, before merging.
pub const MAX_HITS_PER_QUERY: u32 = 50;
/// Refused hits are carried and capped separately, so a directory serving
/// forgeries crowds nothing out.
pub const MAX_REFUSED_RESULTS: u32 = 20;
/// Directories one person may add, and therefore the number of
/// `query-source` calls one run makes. It bounds no single dispatch.
pub const MAX_SOURCES: usize = 8;
/// How many of those calls may be in flight at once. Derived from
/// `max_concurrent_guest_http_per_service` (default 4), not chosen: one
/// below it, so a search cannot consume every admission permit this
/// service has and stall the rest of the Hub. `start-run` returns this
/// value so the client does not carry its own copy.
pub const MAX_CLIENT_CONCURRENCY: usize = 3;
/// Verified hits one `query-source` call stores. Twice the per-source
/// share, because the round-robin skips a listing another source already
/// contributed, so a source can be asked for more than its share's worth
/// of rows before it has contributed its share.
pub const MAX_STORED_PER_SOURCE: u32 = 2 * MAX_HITS_PER_SOURCE;
/// Per-source deadline for the one proxy call `directory.query-source`
/// makes. Derived from the guest dispatch epoch, not chosen: the dispatch
/// traps after `dispatch_epoch_timeout_secs` of wall clock -- 5s by
/// default -- and that budget is spent while waiting on the call.
pub const DEFAULT_SOURCE_TIMEOUT_MS: u32 = 2_000;
/// Headroom the assertion reserves for verifying a full page of envelopes
/// and writing the run rows.
pub const DISPATCH_HEADROOM_MS: u32 = 1_500;
/// A search run's rows are pruned once older than this. Runs are working
/// state, not a cache.
pub const RUN_RETENTION_SECS: u64 = 3_600;

/// A SynOrg's own statement about itself. Unsigned app state: the spec's
/// Records table has no settings row and no roster row, and `directory`
/// mounts no signing certificate in this slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynOrgSettings {
    pub name: String,
    pub rules: String,
    #[serde(default)]
    pub area: Vec<Area>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub support_contact: String,
    pub dispute_path: String,
    pub retention_secs: u64,
    #[serde(default)]
    pub publication_limits: PublicationLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SynOrgSettingsError {
    #[error("name is empty or over {MAX_SYNORG_NAME_LEN} bytes")]
    NameShape,
    #[error("rules text is over {MAX_RULES_LEN} bytes")]
    RulesTooLong,
    #[error("support_contact is over {MAX_CONTACT_LEN} bytes")]
    ContactTooLong,
    #[error("dispute_path is over {MAX_DISPUTE_PATH_LEN} bytes")]
    DisputePathTooLong,
    #[error("more than {MAX_CATEGORIES} categories")]
    TooManyCategories,
    #[error("a category is empty or over {MAX_CATEGORY_LEN} bytes")]
    CategoryShape,
    #[error("more than {MAX_AREAS} areas")]
    TooManyAreas,
    #[error("area: {0}")]
    Area(#[from] crate::area::AreaError),
    #[error("retention_secs must be between {MIN_RETENTION_SECS} and {MAX_RETENTION_SECS}")]
    RetentionOutOfBounds,
    #[error("publication_limits: {0}")]
    PublicationLimits(#[from] crate::safety::LimitsError),
}

impl SynOrgSettings {
    pub fn validate(&self) -> Result<(), SynOrgSettingsError> {
        if self.name.trim().is_empty() || self.name.len() > MAX_SYNORG_NAME_LEN {
            return Err(SynOrgSettingsError::NameShape);
        }
        if self.rules.len() > MAX_RULES_LEN {
            return Err(SynOrgSettingsError::RulesTooLong);
        }
        if self.support_contact.len() > MAX_CONTACT_LEN {
            return Err(SynOrgSettingsError::ContactTooLong);
        }
        if self.dispute_path.len() > MAX_DISPUTE_PATH_LEN {
            return Err(SynOrgSettingsError::DisputePathTooLong);
        }
        if self.categories.len() > MAX_CATEGORIES {
            return Err(SynOrgSettingsError::TooManyCategories);
        }
        for c in &self.categories {
            if c.trim().is_empty() || c.len() > MAX_CATEGORY_LEN {
                return Err(SynOrgSettingsError::CategoryShape);
            }
        }
        if self.area.len() > MAX_AREAS {
            return Err(SynOrgSettingsError::TooManyAreas);
        }
        for a in &self.area {
            a.validate()?;
        }
        if self.retention_secs < MIN_RETENTION_SECS || self.retention_secs > MAX_RETENTION_SECS {
            return Err(SynOrgSettingsError::RetentionOutOfBounds);
        }
        self.publication_limits.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub did: String,
    #[serde(default)]
    pub note: String,
    pub added_at_secs: u64,
}

/// One query. Every field optional; an empty query returns the newest
/// active publications, which is what a person landing on a directory
/// expects to see.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub area: Option<Area>,
    #[serde(default)]
    pub open_to: Option<String>,
    #[serde(default)]
    pub booking_mode: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One result, as a *directory* answers it. Carries no verification
/// verdict at all -- a directory's own answer is never a verified answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub listing_id: String,
    pub record_id: String,
    /// The bytes the provider signed. Never a projection.
    pub envelope: String,
    pub issued_at_secs: u64,
    /// This directory's own clock, its claim, never used for age.
    pub received_at_secs: u64,
    pub area_match: AreaMatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AreaMatch {
    NotQueried,
    Geometric { area_index: u32 },
    Named { label: String },
    NoAreaStated,
}

/// Why one source contributed nothing. `NotStarted` is the node's own
/// refusal to begin the call -- a 503 from guest-HTTP admission -- and is
/// deliberately not folded into `TimedOut`: one says this installation was
/// busy, the other says that directory did not answer, and only the
/// second is the directory's fault.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceError {
    NotFound,
    TimedOut,
    NotStarted,
    Refused { code: i64, message: String },
    Unreadable { reason: String },
}

/// Normalizes a free-text query the identical way at write time and at
/// query time, so the two sides cannot drift: lowercased, every character
/// outside `[a-z0-9 -]` replaced with a space, runs of spaces collapsed.
/// `compile_regex` emits `LIKE` with no `ESCAPE` clause, so a wildcard can
/// only be removed, never escaped -- this is the removal.
#[must_use]
pub fn normalize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_space = false;
    for ch in input.to_lowercase().chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == ' ' || ch == '-' { ch } else { ' ' };
        if mapped == ' ' {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(mapped);
            last_was_space = false;
        }
    }
    out.trim_end().to_string()
}

/// The delimited token string `$regex: "|token|"` matches exactly.
/// Categories are already constrained to `[a-z0-9-]` by `ListingPayload`,
/// so nothing here needs escaping.
#[must_use]
pub fn category_tokens(categories: &[String]) -> String {
    if categories.is_empty() {
        return String::new();
    }
    format!("|{}|", categories.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::area::{self, AreaError};

    fn settings() -> SynOrgSettings {
        SynOrgSettings {
            name: "Bengaluru Trades Guild".to_string(),
            rules: "Be kind. Do good work.".to_string(),
            area: vec![Area::Named { label: "Bengaluru".to_string(), code: None }],
            categories: vec!["plumbing".to_string()],
            support_contact: "support@example.org".to_string(),
            dispute_path: "Contact the owner.".to_string(),
            retention_secs: 30 * 24 * 3600,
            publication_limits: PublicationLimits::default(),
        }
    }

    #[test]
    fn a_full_settings_validates() {
        assert!(settings().validate().is_ok());
    }

    #[test]
    fn name_bounds() {
        let mut s = settings();
        s.name = String::new();
        assert_eq!(s.validate(), Err(SynOrgSettingsError::NameShape));
        s.name = "x".repeat(MAX_SYNORG_NAME_LEN + 1);
        assert_eq!(s.validate(), Err(SynOrgSettingsError::NameShape));
    }

    #[test]
    fn retention_bounds() {
        let mut s = settings();
        s.retention_secs = MIN_RETENTION_SECS - 1;
        assert_eq!(s.validate(), Err(SynOrgSettingsError::RetentionOutOfBounds));
        s.retention_secs = MAX_RETENTION_SECS + 1;
        assert_eq!(s.validate(), Err(SynOrgSettingsError::RetentionOutOfBounds));
    }

    #[test]
    fn category_and_area_caps() {
        let mut s = settings();
        s.categories = (0..MAX_CATEGORIES + 1).map(|i| format!("c{i}")).collect();
        assert_eq!(s.validate(), Err(SynOrgSettingsError::TooManyCategories));
        let mut s = settings();
        s.area = (0..MAX_AREAS + 1)
            .map(|_| Area::Named { label: "x".to_string(), code: None })
            .collect();
        assert_eq!(s.validate(), Err(SynOrgSettingsError::TooManyAreas));
        let mut s = settings();
        s.area = vec![Area::Circle { lat_e6: 0, lon_e6: 0, radius_m: area::MAX_RADIUS_M + 1 }];
        assert!(matches!(
            s.validate(),
            Err(SynOrgSettingsError::Area(AreaError::RadiusTooLarge(_)))
        ));
    }

    #[test]
    fn normalize_text_lowercases_and_collapses() {
        assert_eq!(normalize_text("Emergency  Plumber! 24h"), "emergency plumber 24h");
        assert_eq!(normalize_text("50% off"), "50 off");
        assert_eq!(normalize_text("  leading"), "leading");
    }

    #[test]
    fn normalize_text_is_idempotent() {
        let once = normalize_text("Hedge-Trimming & More!!");
        let twice = normalize_text(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn category_tokens_shape() {
        assert_eq!(category_tokens(&[]), "");
        assert_eq!(
            category_tokens(&["plumbing".to_string(), "emergency".to_string()]),
            "|plumbing|emergency|"
        );
    }

    #[test]
    fn source_timeout_fits_inside_the_dispatch_epoch() {
        // D-C6-19 / F6b: the assertion this row exists to make, checked at
        // build time rather than discovered as an intermittent trap.
        let epoch_ms = 5_000u32;
        assert!(
            DEFAULT_SOURCE_TIMEOUT_MS + DISPATCH_HEADROOM_MS < epoch_ms,
            "source timeout plus headroom must fit inside the dispatch epoch"
        );
    }

    #[test]
    fn client_concurrency_stays_below_guest_http_admission() {
        // D-C6-26 / F6d.
        let max_concurrent_guest_http_per_service = 4usize;
        assert!(MAX_CLIENT_CONCURRENCY < max_concurrent_guest_http_per_service);
    }
}

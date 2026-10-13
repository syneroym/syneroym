//! `coordinators.json`: the last coordinator list from the registry and the
//! relay chosen from it. It lets a restart use the old choice without asking
//! the registry, and use an old list when the registry is down.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use syneroym_core::curated_list::{self, CoordinatorEntry, CuratedList, ListEntry};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CoordinatorCache {
    /// The registry the list came from, normalised. A cache from another
    /// registry is not used.
    pub registry_url: String,
    /// Unix seconds when the list was fetched.
    pub fetched_at_secs: u64,
    /// The list as the registry sent it (entries not normalised).
    pub list: CuratedList<CoordinatorEntry>,
    /// The chosen entry, normalised (it comes out of `curated_list::rank`).
    /// `None` until an entry has worked.
    pub chosen: Option<CoordinatorEntry>,
}

impl CoordinatorCache {
    pub(super) fn expires_at_secs(&self) -> u64 {
        self.fetched_at_secs.saturating_add(self.list.max_age_secs)
    }

    pub(super) fn is_fresh(&self, now_secs: u64) -> bool {
        now_secs < self.expires_at_secs()
    }

    /// Replaces the chosen entry with the entry of the list that has the same
    /// URL, so a changed `info_url` or `operator` is used. Does nothing when
    /// the list no longer has the entry.
    pub(super) fn refresh_chosen(&mut self) {
        let Some(url) = self.chosen.as_ref().map(|c| c.url.clone()) else { return };
        let fresh = self.list.entries.iter().find_map(|e| {
            let mut e = e.clone();
            e.check().is_ok_and(|id| id == url).then_some(e)
        });
        if fresh.is_some() {
            self.chosen = fresh;
        }
    }

    /// Whether the list has an entry whose normalised `url` is `url`.
    pub(super) fn lists(&self, url: &str) -> bool {
        self.list
            .entries
            .iter()
            .any(|e| curated_list::normalize_url(&e.url).is_ok_and(|u| u == url))
    }
}

/// Reads the cache. A missing file is `None` with no log. A file that does
/// not parse is `None` with a warning (it is replaced at the next save).
pub(super) fn load(path: &Path) -> Option<CoordinatorCache> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "cannot read the coordinator cache; ignoring it");
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(cache) => Some(cache),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "coordinator cache is not valid; ignoring it");
            None
        }
    }
}

/// Writes the cache: to `<path>.tmp` first, then renames it over `path`, so a
/// crash never leaves half a file. Creates the parent directory.
///
/// # Errors
/// When the directory or the file cannot be written.
pub(super) fn save(path: &Path, cache: &CoordinatorCache) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let bytes = serde_json::to_vec_pretty(cache).context("encoding the coordinator cache")?;
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests;

//! Curated lists: lists of entries that a registry operator writes into the
//! registry config, and that the registry serves over HTTP. A client ranks a
//! list with `rank` to pick one entry.

use std::{collections::HashMap, fmt, marker::PhantomData};

use anyhow::{Context, ensure};
use reqwest::Url;
use serde::{
    Deserialize, Serialize, Serializer,
    de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor},
    ser::SerializeMap,
};

mod coordinator;
mod rank;
#[cfg(test)]
mod tests;

pub use coordinator::{CoordinatorEntry, CoordinatorFilter, CoordinatorTransport};
pub use rank::{rank, rank_key};

/// The query parameter that counts forwards between registries.
pub const HOPS_PARAM: &str = "hops";

/// A registry forwards a list request to its parent only while the request
/// has made fewer forwards than this. It stops loops between registries.
pub const MAX_LIST_HOPS: u8 = 3;

/// One kind of entry in a curated list.
pub trait ListEntry: Clone + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The route (`GET /<LIST_KEY>`), the JSON key of the entry array, and the
    /// config key under `[roles.community_registry]`.
    const LIST_KEY: &'static str;
    /// The query parameters a client sends to choose entries.
    type Filter: DeserializeOwned + Send + Sync + 'static;

    /// Whether this entry answers `filter`.
    fn matches(&self, filter: &Self::Filter) -> bool;
    /// `filter` as query pairs, to forward a request to a parent registry.
    fn filter_pairs(filter: &Self::Filter) -> Vec<(&'static str, String)>;
    /// Checks the entry, normalises its fields in place, and returns its id.
    ///
    /// # Errors
    /// When a field is not valid.
    fn check(&mut self) -> anyhow::Result<String>;
}

/// The body of `GET /<LIST_KEY>`:
/// `{ "max_age_secs": <u64>, "<LIST_KEY>": [ ... ] }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuratedList<E> {
    /// How long a client may cache this answer.
    pub max_age_secs: u64,
    pub entries: Vec<E>,
}

impl<E: ListEntry> Serialize for CuratedList<E> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("max_age_secs", &self.max_age_secs)?;
        map.serialize_entry(E::LIST_KEY, &self.entries)?;
        map.end()
    }
}

impl<'de, E: ListEntry> Deserialize<'de> for CuratedList<E> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct CuratedListVisitor<E>(PhantomData<E>);

        impl<'de, E: ListEntry> Visitor<'de> for CuratedListVisitor<E> {
            type Value = CuratedList<E>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a curated list map")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut max_age_secs: Option<u64> = None;
                let mut entries: Option<Vec<E>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    if key == "max_age_secs" {
                        if max_age_secs.is_some() {
                            return Err(de::Error::duplicate_field("max_age_secs"));
                        }
                        max_age_secs = Some(map.next_value()?);
                    } else if key == E::LIST_KEY {
                        if entries.is_some() {
                            return Err(de::Error::duplicate_field(E::LIST_KEY));
                        }
                        entries = Some(map.next_value()?);
                    } else {
                        map.next_value::<de::IgnoredAny>()?;
                    }
                }

                let max_age_secs =
                    max_age_secs.ok_or_else(|| de::Error::missing_field("max_age_secs"))?;
                let entries = entries.ok_or_else(|| de::Error::missing_field(E::LIST_KEY))?;

                Ok(CuratedList { max_age_secs, entries })
            }
        }

        d.deserialize_map(CuratedListVisitor(PhantomData))
    }
}

/// Checks and normalises every entry, and refuses duplicate ids.
/// The error names the config key and the entry index, for example
/// `roles.community_registry.coordinators[2]`.
///
/// # Errors
/// On the first bad entry or duplicate.
pub fn check_entries<E: ListEntry>(entries: &mut [E]) -> anyhow::Result<()> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, entry) in entries.iter_mut().enumerate() {
        let id = entry
            .check()
            .with_context(|| format!("roles.community_registry.{}[{i}]", E::LIST_KEY))?;
        if let Some(first) = seen.insert(id.clone(), i) {
            anyhow::bail!(
                "roles.community_registry.{}[{i}] is a duplicate of entry [{first}] ({id})",
                E::LIST_KEY
            );
        }
    }
    Ok(())
}

/// Asks the registry at `registry_url` for its list.
/// `hops` is the number of forwards already made; a client sends 0.
///
/// # Errors
/// When the registry cannot be reached, answers with a non-2xx status, or
/// sends a body that does not parse.
pub async fn fetch<E: ListEntry>(
    client: &reqwest::Client,
    registry_url: &str,
    filter: &E::Filter,
    hops: u8,
) -> anyhow::Result<CuratedList<E>> {
    let base = registry_url.trim_end_matches('/');
    let mut url = Url::parse(&format!("{base}/{}", E::LIST_KEY))
        .with_context(|| format!("registry URL {registry_url:?}"))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in E::filter_pairs(filter) {
            q.append_pair(k, &v);
        }
        q.append_pair(HOPS_PARAM, &hops.to_string());
    }
    let list = client.get(url).send().await?.error_for_status()?.json::<CuratedList<E>>().await?;
    Ok(list)
}

/// The normal form of a relay or info URL: lower-case scheme and host, no
/// default port, no trailing slash. Ranking and the duplicate check use this
/// form, because `x` and `x/` must be the same entry.
///
/// # Errors
/// When `raw` is not an `http` or `https` URL with a host, or has a user,
/// password, query or fragment.
pub fn normalize_url(raw: &str) -> anyhow::Result<String> {
    let url = Url::parse(raw).with_context(|| format!("{raw:?} is not a valid URL"))?;
    ensure!(matches!(url.scheme(), "http" | "https"), "{raw:?}: scheme must be http or https");
    ensure!(url.host_str().is_some(), "{raw:?}: URL has no host");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "{raw:?}: URL has a user or password"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "{raw:?}: URL has a query or fragment"
    );
    let s = url.as_str();
    Ok(s.strip_suffix('/').unwrap_or(s).to_owned())
}

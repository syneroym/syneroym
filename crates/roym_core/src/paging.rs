//! Reading every row of a data-layer query, one page at a time.
//!
//! The data layer answers a query with at most one page of rows and a
//! cursor for the next page. Every Roym service that needs all matching
//! rows walks those pages the same way, so the walk is written once here.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{QueryOptions, RecordReadValue},
};

use crate::record::Envelope;

/// Rows asked for in each page.
pub const PAGE_SIZE: u32 = 500;

/// The pages of one query, read in order.
///
/// The walk ends when the host returns no next cursor, or returns the same
/// cursor it was just given: a host that cannot move forward would
/// otherwise keep the caller in an endless loop.
#[derive(Debug)]
pub struct Pages<'a, H> {
    host: &'a H,
    collection: &'a str,
    filter: Option<String>,
    cursor: Option<String>,
    done: bool,
}

impl<'a, H: AppHost> Pages<'a, H> {
    pub fn new(host: &'a H, collection: &'a str, filter: Option<String>) -> Self {
        Self { host, collection, filter, cursor: None, done: false }
    }

    /// The next page's rows, or `None` after the last page.
    pub async fn next_page(&mut self) -> Result<Option<Vec<RecordReadValue>>, String> {
        if self.done {
            return Ok(None);
        }
        let page = AppDataLayer::query(
            self.host,
            self.collection.to_string(),
            QueryOptions {
                filter: self.filter.clone(),
                limit: Some(PAGE_SIZE),
                cursor: self.cursor.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        if page.next_cursor.is_none() || page.next_cursor == self.cursor {
            self.done = true;
        } else {
            self.cursor = page.next_cursor;
        }
        Ok(Some(page.records))
    }
}

/// Every matching row that `keep` maps to `Some`, in the order the host
/// returns them.
pub async fn filter_map<H: AppHost, U>(
    host: &H,
    collection: &str,
    filter: Option<String>,
    mut keep: impl FnMut(RecordReadValue) -> Option<U>,
) -> Result<Vec<U>, String> {
    let mut out = Vec::new();
    let mut pages = Pages::new(host, collection, filter);
    while let Some(rows) = pages.next_page().await? {
        out.extend(rows.into_iter().filter_map(&mut keep));
    }
    Ok(out)
}

/// Every matching row's payload, decoded as `T`. A row that does not
/// decode is skipped, not reported as an error.
pub async fn query_all<H: AppHost, T: DeserializeOwned>(
    host: &H,
    collection: &str,
    filter: Option<String>,
) -> Result<Vec<T>, String> {
    filter_map(host, collection, filter, |row| serde_json::from_slice(&row.payload).ok()).await
}

/// The same rows as [`query_all`], each beside its row id.
pub async fn query_all_with_ids<H: AppHost, T: DeserializeOwned>(
    host: &H,
    collection: &str,
    filter: Option<String>,
) -> Result<Vec<(String, T)>, String> {
    filter_map(host, collection, filter, |row| {
        serde_json::from_slice(&row.payload).ok().map(|value| (row.id, value))
    })
    .await
}

/// How many rows match `filter`, whether their payloads decode or not.
pub async fn count<H: AppHost>(
    host: &H,
    collection: &str,
    filter: Option<String>,
) -> Result<usize, String> {
    let mut total = 0;
    let mut pages = Pages::new(host, collection, filter);
    while let Some(rows) = pages.next_page().await? {
        total += rows.len();
    }
    Ok(total)
}

/// Every signed envelope stored in `collection` whose payload has `field`
/// equal to `value`, oldest first by `issued_at_secs`. Two envelopes
/// issued in the same second keep store order.
///
/// The host filters on the payload field first, so only candidate rows
/// cross the boundary. It is still a scan (there is no expression index on
/// a JSON path), and each candidate is parsed and checked again here.
pub async fn envelope_history<H: AppHost>(
    host: &H,
    collection: &str,
    field: &str,
    value: &str,
) -> Result<Vec<String>, String> {
    let mut filter = Map::new();
    filter.insert(format!("payload.{field}"), Value::from(value));
    let mut found = filter_map(host, collection, Some(Value::Object(filter).to_string()), |row| {
        let text = String::from_utf8_lossy(&row.payload).into_owned();
        let envelope = Envelope::from_json(&text).ok()?;
        let matches = envelope.payload.get(field).and_then(Value::as_str) == Some(value);
        matches.then_some((envelope.issued_at_secs, text))
    })
    .await?;
    found.sort_by_key(|(issued_at_secs, _)| *issued_at_secs);
    Ok(found.into_iter().map(|(_, text)| text).collect())
}

#[cfg(test)]
pub(crate) mod tests;

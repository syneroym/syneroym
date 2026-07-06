//! Conversions from internal storage errors into the WIT-facing
//! `data-layer-error` shape used by `ServiceStore`.

use crate::host_store::DataLayerError;

/// Maps a `rusqlite` error into the WIT `data-layer-error` variant a guest
/// should see. Missing-table errors surface as `CollectionNotFound` (the
/// caller operated on a collection that was never created via
/// `create-collection`); everything else is an opaque `Internal` error.
pub fn map_rusqlite_error(err: rusqlite::Error) -> DataLayerError {
    if is_no_such_table(&err) {
        DataLayerError::CollectionNotFound
    } else {
        DataLayerError::Internal(err.to_string())
    }
}

fn is_no_such_table(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(_, Some(msg)) => msg.contains("no such table"),
        other => other.to_string().contains("no such table"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_no_such_table_maps_to_collection_not_found() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let err = conn.execute("SELECT * FROM does_not_exist", []).unwrap_err();
        assert!(matches!(map_rusqlite_error(err), DataLayerError::CollectionNotFound));
    }

    #[test]
    fn test_other_error_maps_to_internal() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id TEXT PRIMARY KEY)", []).unwrap();
        conn.execute("INSERT INTO t (id) VALUES ('a')", []).unwrap();
        let err = conn.execute("INSERT INTO t (id) VALUES ('a')", []).unwrap_err();
        assert!(matches!(map_rusqlite_error(err), DataLayerError::Internal(_)));
    }
}

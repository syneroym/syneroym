//! `AppDataLayer` implementation for `TestHost` with memory storage and fault
//! injection.

use syneroym_app_host::{
    AppDataLayer,
    types::data_layer::{
        CollectionSchema, DataLayerError, Mutation, QueryOptions, QueryResult, RawQueryResult,
        RecordReadValue, RecordWriteValue, SqlValue,
    },
};

use crate::testing::TestHost;

impl AppDataLayer for TestHost {
    async fn create_collection(&self, schema: CollectionSchema) -> Result<(), DataLayerError> {
        let mut store = self.storage.lock().unwrap();
        store.entry(schema.name).or_default();
        Ok(())
    }

    async fn drop_collection(&self, name: String) -> Result<(), DataLayerError> {
        let mut store = self.storage.lock().unwrap();
        store.remove(&name);
        Ok(())
    }

    async fn put(&self, collection: String, value: RecordWriteValue) -> Result<(), DataLayerError> {
        self.check_write_fault(&collection, &value.id)?;
        let mut store = self.storage.lock().unwrap();
        store.entry(collection).or_default().insert(value.id, value.payload);
        Ok(())
    }

    async fn patch(
        &self,
        _col: String,
        _id: String,
        _patch: Vec<u8>,
    ) -> Result<(), DataLayerError> {
        unimplemented!()
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        self.reads.lock().unwrap().push((collection.clone(), id.clone()));
        self.check_read_fault(&collection, &id)?;
        let store = self.storage.lock().unwrap();
        if let Some(col) = store.get(&collection)
            && let Some(val) = col.get(&id)
        {
            return Ok(Some(RecordReadValue {
                id,
                payload: val.clone(),
                creator_id: String::new(),
                created_at: 0,
                updated_at: 0,
            }));
        }
        Ok(None)
    }

    async fn query(
        &self,
        collection: String,
        opts: QueryOptions,
    ) -> Result<QueryResult, DataLayerError> {
        self.reads
            .lock()
            .unwrap()
            .push((collection.clone(), opts.filter.clone().unwrap_or_default()));
        self.queries.lock().unwrap().push((collection.clone(), opts.clone()));
        if let Some(page) = self.query_pages.lock().unwrap().pop_front() {
            return Ok(page);
        }
        let store = self.storage.lock().unwrap();
        let Some(col) = store.get(&collection) else {
            return Ok(QueryResult { records: Vec::new(), next_cursor: None });
        };

        let filter_obj =
            opts.filter.as_deref().and_then(|f| serde_json::from_str::<serde_json::Value>(f).ok());

        let mut matches: Vec<RecordReadValue> = Vec::new();
        for (id, payload) in col {
            if let Some(ref f) = filter_obj
                && !matches_filter(payload, f)
            {
                continue;
            }
            matches.push(RecordReadValue {
                id: id.clone(),
                payload: payload.clone(),
                creator_id: String::new(),
                created_at: 0,
                updated_at: 0,
            });
        }

        matches.sort_by(|a, b| a.id.cmp(&b.id));

        let start_idx = if let Some(ref cur) = opts.cursor {
            matches.iter().position(|r| &r.id > cur).unwrap_or(matches.len())
        } else {
            0
        };

        let limit = opts.limit.map(|l| l as usize).unwrap_or(matches.len());
        let page: Vec<RecordReadValue> = matches.into_iter().skip(start_idx).take(limit).collect();
        let next_cursor = if page.len() == limit && !page.is_empty() {
            page.last().map(|r| r.id.clone())
        } else {
            None
        };

        Ok(QueryResult { records: page, next_cursor })
    }

    async fn aggregate(
        &self,
        _col: String,
        _pipe: String,
    ) -> Result<RawQueryResult, DataLayerError> {
        unimplemented!()
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DataLayerError> {
        self.check_write_fault(&collection, &id)?;
        let mut store = self.storage.lock().unwrap();
        if let Some(col) = store.get_mut(&collection) {
            col.remove(&id);
        }
        Ok(())
    }

    async fn delete_many(&self, collection: String, filter: String) -> Result<u64, DataLayerError> {
        self.check_write_fault(&collection, "*")?;
        let filter_obj = serde_json::from_str::<serde_json::Value>(&filter).ok();
        let mut store = self.storage.lock().unwrap();
        let Some(col) = store.get_mut(&collection) else {
            return Ok(0);
        };
        let to_remove: Vec<String> =
            col.iter()
                .filter(|(_id, payload)| {
                    if let Some(ref f) = filter_obj { matches_filter(payload, f) } else { true }
                })
                .map(|(id, _)| id.clone())
                .collect();
        let count = to_remove.len() as u64;
        for id in to_remove {
            col.remove(&id);
        }
        Ok(count)
    }

    async fn batch_mutate(
        &self,
        collection: String,
        muts: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        for m in &muts {
            match m {
                Mutation::Put(v) => self.check_write_fault(&collection, &v.id)?,
                Mutation::Patch(p) => self.check_write_fault(&collection, &p.id)?,
                Mutation::Delete(id) => self.check_write_fault(&collection, id)?,
            }
        }
        let mut store = self.storage.lock().unwrap();
        let col = store.entry(collection).or_default();
        for m in muts {
            match m {
                Mutation::Put(v) => {
                    col.insert(v.id, v.payload);
                }
                Mutation::Patch(_p) => {}
                Mutation::Delete(id) => {
                    col.remove(&id);
                }
            }
        }
        Ok(())
    }

    async fn create(
        &self,
        collection: String,
        values: Vec<RecordWriteValue>,
    ) -> Result<Option<String>, DataLayerError> {
        for v in &values {
            self.check_write_fault(&collection, &v.id)?;
        }
        let mut store = self.storage.lock().unwrap();
        let col = store.entry(collection).or_default();
        for v in &values {
            if col.contains_key(&v.id) {
                return Ok(Some(v.id.clone()));
            }
        }
        for v in values {
            col.insert(v.id, v.payload);
        }
        Ok(None)
    }

    async fn execute_ddl(&self, _sql: String) -> Result<(), DataLayerError> {
        unimplemented!()
    }

    async fn query_raw(
        &self,
        _sql: String,
        _params: Vec<SqlValue>,
    ) -> Result<RawQueryResult, DataLayerError> {
        unimplemented!()
    }

    async fn check_access(
        &self,
        _col: String,
        _id: String,
        _op: String,
    ) -> Result<bool, DataLayerError> {
        unimplemented!()
    }
}

pub(crate) fn matches_filter(payload: &[u8], filter: &serde_json::Value) -> bool {
    let Ok(val) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return false;
    };
    let Some(filter_obj) = filter.as_object() else {
        return true;
    };
    for (k, expected) in filter_obj {
        let actual = val.get(k);
        if let Some(exp_obj) = expected.as_object() {
            if let Some(gt_val) = exp_obj.get("$gt").and_then(|v| v.as_f64()) {
                let actual_num = actual.and_then(|v| v.as_f64());
                if actual_num.is_none_or(|n| n <= gt_val) {
                    return false;
                }
            }
            if let Some(gte_val) = exp_obj.get("$gte").and_then(|v| v.as_f64()) {
                let actual_num = actual.and_then(|v| v.as_f64());
                if actual_num.is_none_or(|n| n < gte_val) {
                    return false;
                }
            }
            if let Some(lt_val) = exp_obj.get("$lt").and_then(|v| v.as_f64()) {
                let actual_num = actual.and_then(|v| v.as_f64());
                if actual_num.is_none_or(|n| n >= lt_val) {
                    return false;
                }
            }
            if let Some(lte_val) = exp_obj.get("$lte").and_then(|v| v.as_f64()) {
                let actual_num = actual.and_then(|v| v.as_f64());
                if actual_num.is_none_or(|n| n > lte_val) {
                    return false;
                }
            }
        } else if actual != Some(expected) {
            return false;
        }
    }
    true
}

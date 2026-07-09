use std::collections::BTreeMap;

use serde_json::Value;

pub fn flatten_json_config(json: &Value, prefix: &str, map: &mut BTreeMap<String, String>) {
    match json {
        Value::Object(obj) => {
            for (k, v) in obj {
                let new_prefix =
                    if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                flatten_json_config(v, &new_prefix, map);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let new_prefix = format!("{}[{}]", prefix, i);
                flatten_json_config(v, &new_prefix, map);
            }
        }
        Value::Null => {
            map.insert(prefix.to_string(), "null".to_string());
        }
        Value::Bool(b) => {
            map.insert(prefix.to_string(), b.to_string());
        }
        Value::Number(n) => {
            map.insert(prefix.to_string(), n.to_string());
        }
        Value::String(s) => {
            map.insert(prefix.to_string(), s.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn test_flatten_json_config() {
        let json = serde_json::json!({
            "db": {
                "host": "localhost",
                "port": 5432
            },
            "features": ["a", "b"]
        });
        let mut map = BTreeMap::new();
        flatten_json_config(&json, "", &mut map);
        assert_eq!(map.get("db.host"), Some(&"localhost".to_string()));
        assert_eq!(map.get("db.port"), Some(&"5432".to_string()));
        assert_eq!(map.get("features[0]"), Some(&"a".to_string()));
        assert_eq!(map.get("features[1]"), Some(&"b".to_string()));
    }
}

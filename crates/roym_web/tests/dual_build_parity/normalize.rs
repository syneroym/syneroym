use std::collections::HashMap;

use serde_json::Value;

/// Replaces every host message id with `<msg:N>` -- N being the row's
/// position once `messages` / `matches` rows are in their own sort order,
/// which the service already returns them in. Host message ids fold in a
/// random nonce, so they differ between the two stacks; a positional
/// rewrite keeps "two messages merged into one row" detectable where a
/// blanket strip would hide it. Returns the count of distinct ids mapped.
pub(crate) fn normalize_message_ids(val: &mut Value) -> usize {
    let mut order: Vec<String> = Vec::new();
    collect_ordered_ids(val, &mut order);
    let map: HashMap<String, String> =
        order.iter().enumerate().map(|(i, id)| (id.clone(), format!("<msg:{i}>"))).collect();
    rewrite_ids(val, &map);
    map.len()
}

/// Replaces every `booking-progress` card's `record_id` with
/// `<progress:N>`, N being the card's position once `cards` rows are in
/// their own sort order. A snapshot the provider's own node signs embeds
/// `track_window_ends_at_secs` (computed from `clock::now_secs()`, not the
/// pinned signing clock) in its payload, so the two builds' independent
/// wall-clock reads can each produce a different content hash for the same
/// logical snapshot. Returns the count of distinct ids mapped.
pub(crate) fn normalize_progress_record_ids(val: &mut Value) -> usize {
    let mut order: Vec<String> = Vec::new();
    collect_ordered_progress_ids(val, &mut order);
    let map: HashMap<String, String> =
        order.iter().enumerate().map(|(i, id)| (id.clone(), format!("<progress:{i}>"))).collect();
    rewrite_ids(val, &map);
    map.len()
}

fn collect_ordered_progress_ids(val: &Value, out: &mut Vec<String>) {
    match val {
        Value::Object(map) => {
            if map.get("card_type").and_then(Value::as_str) == Some("booking-progress")
                && let Some(id) = map.get("record_id").and_then(Value::as_str)
                && !out.iter().any(|e| e == id)
            {
                out.push(id.to_string());
            }
            for v in map.values() {
                collect_ordered_progress_ids(v, out);
            }
        }
        Value::Array(arr) => arr.iter().for_each(|v| collect_ordered_progress_ids(v, out)),
        _ => {}
    }
}

pub(crate) fn collect_ordered_ids(val: &Value, out: &mut Vec<String>) {
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                if (k == "messages" || k == "matches" || k == "cards")
                    && let Value::Array(rows) = v
                {
                    for row in rows {
                        let maybe_id =
                            row.get("id").or_else(|| row.get("message_id")).and_then(Value::as_str);
                        if let Some(id) = maybe_id
                            && !out.iter().any(|e| e == id)
                        {
                            out.push(id.to_string());
                        }
                    }
                }
                collect_ordered_ids(v, out);
            }
        }
        Value::Array(arr) => arr.iter().for_each(|v| collect_ordered_ids(v, out)),
        _ => {}
    }
}

pub(crate) fn rewrite_ids(val: &mut Value, map: &HashMap<String, String>) {
    match val {
        Value::Object(m) => m.values_mut().for_each(|v| rewrite_ids(v, map)),
        Value::Array(a) => a.iter_mut().for_each(|v| rewrite_ids(v, map)),
        Value::String(s) => {
            if let Some(replacement) = map.get(s) {
                *s = replacement.clone();
            }
        }
        _ => {}
    }
}

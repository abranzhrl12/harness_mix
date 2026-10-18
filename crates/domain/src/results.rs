use crate::Failure;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

struct Entry {
    id: String,
    text: String,
    created: Instant,
}

pub struct Results {
    entries: VecDeque<Entry>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
    ttl: Duration,
}

impl Default for Results {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            max_bytes: 8 * 1024 * 1024,
            max_entries: 64,
            ttl: Duration::from_secs(600),
        }
    }
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Return a byte boundary at a UTF-16 offset, rejecting half a surrogate pair.
fn byte_offset(text: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}

pub fn prefix(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    let mut end = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > max_units {
            break;
        }
        units += ch.len_utf16();
        end = byte + ch.len_utf8();
    }
    &text[..end]
}

impl Results {
    pub fn value(&mut self, id: &str) -> Result<Value, Failure> {
        self.prune();
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(("DETAIL", "detalle vencido, descartado o inexistente"))?;
        serde_json::from_str(&entry.text).map_err(|_| ("DETAIL", "stored result is not valid JSON"))
    }
    fn remove_oldest(&mut self) {
        if let Some(entry) = self.entries.pop_front() {
            self.bytes -= entry.text.len();
        }
    }
    fn prune(&mut self) {
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.created.elapsed() >= self.ttl)
        {
            self.remove_oldest();
        }
    }
    pub fn save(&mut self, value: &Value, complete: bool) -> Option<Value> {
        self.prune();
        let text = value.to_string();
        if text.len() > self.max_bytes {
            return None;
        }
        while self.entries.len() >= self.max_entries || self.bytes + text.len() > self.max_bytes {
            self.remove_oldest();
        }
        let id = uuid::Uuid::new_v4().to_string();
        let detail = json!({"ref":id,"characters":utf16_len(&text),"complete":complete});
        self.bytes += text.len();
        self.entries.push_back(Entry {
            id,
            text,
            created: Instant::now(),
        });
        Some(detail)
    }
    pub fn get(&mut self, id: &str, offset: usize, max_chars: usize) -> Result<Value, Failure> {
        self.prune();
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(("DETAIL", "detalle vencido, descartado o inexistente"))?;
        if !(1..=16000).contains(&max_chars) {
            return Err(("DETAIL", "rango de detalle inválido"));
        }
        let total = utf16_len(&entry.text);
        let start = byte_offset(&entry.text, offset).ok_or((
            "DETAIL",
            "offset inválido o divide un carácter Unicode; usar nextOffset",
        ))?;
        let mut end_units = offset.saturating_add(max_chars).min(total);
        let end = if let Some(end) = byte_offset(&entry.text, end_units) {
            end
        } else {
            end_units -= 1;
            byte_offset(&entry.text, end_units).expect("preceding UTF-16 boundary")
        };
        let (end, end_units) = if end == start && start < entry.text.len() {
            let ch = entry.text[start..].chars().next().expect("nonempty suffix");
            (start + ch.len_utf8(), offset + ch.len_utf16())
        } else {
            (end, end_units)
        };
        Ok(
            json!({"ref":id,"offset":offset,"text":entry.text[start..end],"nextOffset":if end_units < total { Some(end_units) } else { None },"totalCharacters":total,"format":"json-text"}),
        )
    }
    pub fn envelope(&mut self, tool: &str, args: &Value, result: Result<Value, Failure>) -> Value {
        let mut scope = json!({"tool":tool});
        if let Some(path) = args
            .get("path")
            .filter(|v| v.is_string())
            .or_else(|| args.get("scope").filter(|v| v.is_string()))
        {
            scope["path"] = path.clone();
        }
        if let Some(language) = args.get("language").filter(|v| v.is_string()) {
            scope["language"] = language.clone();
        }
        let (state, mut data, mut diagnostics, omitted, detail) = match result {
            Err((code, message)) => (
                match code {
                    "CANCELLED" => "cancelled",
                    "TIMEOUT" => "timeout",
                    _ => "error",
                },
                json!({}),
                json!([{"code":code,"message":message}]),
                json!({}),
                Value::Null,
            ),
            Ok(mut data) => {
                let map = data
                    .as_object_mut()
                    .expect("internal tool results are objects");
                let state = if map.remove("cancelled") == Some(json!(true)) {
                    "cancelled"
                } else if map.remove("timedOut") == Some(json!(true)) {
                    "timeout"
                } else if map.remove("ok") == Some(json!(false)) {
                    "error"
                } else {
                    "ok"
                };
                // Remove all control fields even when a preceding state took priority.
                map.remove("cancelled");
                map.remove("timedOut");
                map.remove("ok");
                let complete = map
                    .remove("_complete")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let mut omitted = json!({});
                if let Some(count) = map
                    .remove("omitted")
                    .filter(|v| v.as_u64().is_some_and(|n| n > 0))
                {
                    omitted["matches"] = count;
                }
                if map.remove("truncated") == Some(json!(true)) {
                    omitted["truncated"] = json!(true);
                }
                let detail = map
                    .remove("_detail")
                    .map(|value| {
                        self.save(&value, complete).unwrap_or_else(|| {
                            omitted["detailUnavailable"] = json!(true);
                            Value::Null
                        })
                    })
                    .unwrap_or(Value::Null);
                let diagnostics = map.remove("diagnostics").unwrap_or_else(|| json!([]));
                (state, data, diagnostics, omitted, detail)
            }
        };
        if let Some(items) = diagnostics.as_array_mut() {
            for item in items {
                let code = item["code"].as_str().unwrap_or("UNKNOWN");
                let (retryable, action) = match code {
                    "TIMEOUT" | "BUSY" => (true, "check_status_then_reduce_scope"),
                    "STALE" | "HASH" | "HASH_MISMATCH" => (false, "reread_source_and_hash"),
                    "SYMBOL"
                    | "AMBIGUOUS"
                    | "AMBIGUOUS_SYMBOL"
                    | "STALE_SYMBOL"
                    | "TARGET_NOT_RESOLVED" => {
                        (false, "select_exact_target_from_candidates_or_outline")
                    }
                    "DETAIL" => (
                        false,
                        "obtain_new_result_reference_without_repeating_mutations",
                    ),
                    "ARGUMENT" | "RANGE" | "QUERY_SYNTAX" => (false, "correct_arguments"),
                    "PATH" => (false, "select_authorized_workspace_relative_path"),
                    "QUERY_LIMIT" | "GRAPH_LIMIT" | "SYNC_LIMIT" => (false, "reduce_scope"),
                    "INDEX" | "INDEX_STALE" | "INDEX_UNCHECKED" => {
                        (false, "inspect_index_status_and_freshness")
                    }
                    "SQL_CONFIG" | "SQL_CONNECTION" => (false, "inspect_sql_status"),
                    "CANCELLED" => (false, "resume_only_if_requested"),
                    _ => (false, "inspect_diagnostics_before_retry"),
                };
                if let Some(map) = item.as_object_mut() {
                    map.entry("retryable").or_insert(json!(retryable));
                    map.entry("requiredAction").or_insert(json!(action));
                }
            }
        }
        if data.get("language") == args.get("language") {
            data.as_object_mut().expect("object").remove("language");
        }
        json!({"state":state,"scope":scope,"diagnostics":diagnostics,"omitted":omitted,"detail":detail,"data":data})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_match_javascript_utf16_offsets_without_splitting_emoji() {
        let mut store = Results::default();
        let detail = store.save(&json!({"text":"a😀b"}), true).unwrap();
        let id = detail["ref"].as_str().unwrap();
        let mut offset = 0;
        let mut joined = String::new();
        loop {
            let page = store.get(id, offset, 1).unwrap();
            joined.push_str(page["text"].as_str().unwrap());
            if page["nextOffset"].is_null() {
                break;
            }
            offset = page["nextOffset"].as_u64().unwrap() as usize;
        }
        assert_eq!(joined, json!({"text":"a😀b"}).to_string());
        let emoji_byte = joined.find('😀').unwrap();
        let split = utf16_len(&joined[..emoji_byte]) + 1;
        assert_eq!(store.get(id, split, 20).unwrap_err().0, "DETAIL");
        assert!(store.get(id, 1000, 1).is_err());
        assert!(store.get(id, 0, 0).is_err());
    }
    #[test]
    fn eviction_ttl_and_byte_limits() {
        let mut store = Results {
            max_entries: 1,
            max_bytes: 100,
            ..Results::default()
        };
        let old = store.save(&json!({"a":1}), true).unwrap();
        let new = store.save(&json!({"a":2}), false).unwrap();
        assert!(store.get(old["ref"].as_str().unwrap(), 0, 10).is_err());
        assert_eq!(new["complete"], false);
        assert!(store.save(&json!("x".repeat(101)), true).is_none());
        store.ttl = Duration::ZERO;
        assert!(store.get(new["ref"].as_str().unwrap(), 0, 10).is_err());
        assert_eq!(store.bytes, 0);
    }
}

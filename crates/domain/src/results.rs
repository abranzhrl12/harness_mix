use crate::Failure;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Write},
    mem::size_of,
    ops::Range,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Entry {
    id: String,
    text: String,
    created: Instant,
    session: Option<String>,
    expires_at: Option<u64>,
    complete: bool,
    search_collection: Option<String>,
    search_page_size: usize,
    search_row_ranges: Option<Vec<Range<usize>>>,
    search_metadata_range: Option<Range<usize>>,
    memory_bytes: usize,
}

/// Verifies serializer output against the retained JSON while recording its
/// byte span. It indexes one known search array without parsing/copying it.
struct RangeWriter<'a> {
    source: &'a [u8],
    offset: usize,
}

impl Write for RangeWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(end) = self.offset.checked_add(bytes.len()) else {
            return Err(io::Error::other("JSON range overflow"));
        };
        if self.source.get(self.offset..end) != Some(bytes) {
            return Err(io::Error::other(
                "JSON value differs from retained snapshot",
            ));
        }
        self.offset = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialized_range(text: &str, start: usize, value: &Value) -> Option<Range<usize>> {
    let mut writer = RangeWriter {
        source: text.as_bytes(),
        offset: start,
    };
    serde_json::to_writer(&mut writer, value).ok()?;
    (writer.offset > start).then_some(start..writer.offset)
}

fn top_level_value_range(text: &str, value: &Value, target: &str) -> Option<Range<usize>> {
    let object = value.as_object()?;
    let mut writer = RangeWriter {
        source: text.as_bytes(),
        offset: 0,
    };
    writer.write_all(b"{").ok()?;
    let mut found = None;
    for (index, (key, field)) in object.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",").ok()?;
        }
        serde_json::to_writer(&mut writer, key).ok()?;
        writer.write_all(b":").ok()?;
        let start = writer.offset;
        serde_json::to_writer(&mut writer, field).ok()?;
        if key == target {
            found = Some(start..writer.offset);
        }
    }
    writer.write_all(b"}").ok()?;
    if writer.offset == text.len() {
        found
    } else {
        None
    }
}

fn index_array(text: &str, value: &Value, key: &str) -> Option<Vec<Range<usize>>> {
    let rows = value.get(key)?.as_array()?;
    let mut cursor = top_level_value_range(text, value, key)?.start;
    if text.as_bytes().get(cursor) != Some(&b'[') {
        return None;
    }
    cursor += 1;
    let mut ranges = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            if text.as_bytes().get(cursor) != Some(&b',') {
                return None;
            }
            cursor += 1;
        }
        let range = serialized_range(text, cursor, row)?;
        cursor = range.end;
        ranges.push(range);
    }
    (text.as_bytes().get(cursor) == Some(&b']')).then_some(ranges)
}

fn index_metadata(text: &str, value: &Value) -> Option<Range<usize>> {
    top_level_value_range(text, value, "_searchSnapshot")
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
            max_bytes: 64 * 1024 * 1024,
            max_entries: 1024,
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
    fn remove_oldest(&mut self) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.session.is_none())
        else {
            return false;
        };
        let entry = self.entries.remove(index).expect("existing entry");
        self.bytes -= entry.memory_bytes;
        true
    }
    fn prune(&mut self) {
        self.entries.retain(|entry| {
            let keep = entry.session.is_some() || entry.created.elapsed() < self.ttl;
            if !keep {
                self.bytes -= entry.memory_bytes;
            }
            keep
        });
    }
    pub fn save(&mut self, value: &Value, complete: bool) -> Option<Value> {
        self.save_for_session(value, complete, None)
    }
    pub fn save_for_session(
        &mut self,
        value: &Value,
        complete: bool,
        session: Option<&str>,
    ) -> Option<Value> {
        self.save_internal(value, complete, session, None)
    }

    pub fn save_search_for_session(
        &mut self,
        value: &Value,
        complete: bool,
        session: Option<&str>,
        collection: &str,
        page_size: usize,
    ) -> Option<Value> {
        self.save_internal(
            value,
            complete,
            session,
            Some((collection, page_size.max(1))),
        )
    }

    fn save_internal(
        &mut self,
        value: &Value,
        complete: bool,
        session: Option<&str>,
        search: Option<(&str, usize)>,
    ) -> Option<Value> {
        self.prune();
        let text = value.to_string();
        if text.len() > self.max_bytes {
            return None;
        }
        let indexed = search.and_then(|(collection, page_size)| {
            Some((
                collection.to_owned(),
                page_size,
                index_array(&text, value, collection)?,
                index_metadata(&text, value)?,
            ))
        });
        let (
            mut search_collection,
            mut search_page_size,
            mut search_row_ranges,
            mut search_metadata_range,
        ) = match indexed {
            Some((collection, page_size, ranges, metadata)) => {
                (Some(collection), page_size, Some(ranges), Some(metadata))
            }
            None => (None, 0, None, None),
        };
        let index_bytes = search_row_ranges.as_ref().map_or(0, |ranges| {
            ranges.capacity().saturating_mul(size_of::<Range<usize>>())
        });
        let id = uuid::Uuid::new_v4().to_string();
        let base_memory_bytes = size_of::<Entry>()
            .saturating_add(text.capacity())
            .saturating_add(id.capacity())
            .saturating_add(session.map_or(0, str::len));
        let mut memory_bytes = base_memory_bytes
            .saturating_add(search_collection.as_ref().map_or(0, String::capacity))
            .saturating_add(index_bytes);
        // Keep the detail payload available even when its optional row index
        // would exceed capacity; pagination then reports unavailable.
        if memory_bytes > self.max_bytes && search_row_ranges.is_some() {
            search_collection = None;
            search_page_size = 0;
            search_row_ranges = None;
            search_metadata_range = None;
            memory_bytes = base_memory_bytes;
        }
        if memory_bytes > self.max_bytes {
            return None;
        }
        while self.entries.len() >= self.max_entries
            || self.bytes.saturating_add(memory_bytes) > self.max_bytes
        {
            if !self.remove_oldest() {
                return None;
            }
        }
        let expires_at = if session.is_some() {
            None
        } else {
            Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .saturating_add(self.ttl.as_secs()),
            )
        };
        let detail = json!({"ref":id,"characters":utf16_len(&text),"complete":complete,
            "expiresAt":expires_at,"expirationFormat":"unix-seconds","retention":if session.is_some() {"server-session"} else {"ttl-or-capacity"},
            "nextCall":{"tool":"harness_detail","arguments":{"ref":id,"offset":0,"maxChars":16000,"session":session}}});
        self.bytes += memory_bytes;
        self.entries.push_back(Entry {
            id,
            text,
            created: Instant::now(),
            session: session.map(str::to_owned),
            expires_at,
            complete,
            search_collection,
            search_page_size,
            search_row_ranges,
            search_metadata_range,
            memory_bytes,
        });
        Some(detail)
    }

    fn search_reference(&mut self, id: &str) -> Option<Value> {
        self.prune();
        let entry = self.entries.iter().find(|entry| entry.id == id)?;
        Some(json!({
            "ref":entry.id,
            "characters":utf16_len(&entry.text),
            "complete":entry.complete,
            "expiresAt":entry.expires_at,
            "expirationFormat":"unix-seconds",
            "retention":if entry.session.is_some() { "server-session" } else { "ttl-or-capacity" },
            "nextCall":{"tool":"harness_detail","arguments":{"ref":entry.id,"offset":0,"maxChars":16000,"session":entry.session}}
        }))
    }

    fn has_search_index(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry.id == id && entry.search_collection.is_some() && entry.search_row_ranges.is_some()
        })
    }

    /// Return one page from a previously saved search detail snapshot.
    pub fn search_page(
        &mut self,
        id: &str,
        offset: usize,
        session: Option<&str>,
    ) -> Result<Value, Failure> {
        self.prune();
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(("DETAIL", "cursor vencido, descartado o inexistente"))?;
        let collection = entry.search_collection.as_deref().ok_or((
            "DETAIL",
            "la referencia no contiene un snapshot de harness_search",
        ))?;
        let complete = entry.complete;
        let page_size = entry.search_page_size;
        let stored_session = entry.session.clone();
        if stored_session
            .as_deref()
            .is_some_and(|owner| Some(owner) != session)
        {
            return Err(("DETAIL", "cursor pertenece a otra sesión"));
        }
        let expires_at = entry.expires_at;
        let retention = if stored_session.is_some() {
            "server-session"
        } else {
            "ttl-or-capacity"
        };
        let ranges = entry.search_row_ranges.as_ref().ok_or((
            "DETAIL",
            "el snapshot de búsqueda no contiene un índice de filas",
        ))?;
        if offset > ranges.len() {
            return Err(("RANGE", "offset fuera del snapshot de búsqueda"));
        }
        let end = offset.saturating_add(page_size).min(ranges.len());
        let mut page: Vec<Value> = ranges[offset..end]
            .iter()
            .map(|range| serde_json::from_str(&entry.text[range.clone()]))
            .collect::<Result<_, _>>()
            .map_err(|_| ("DETAIL", "fila de búsqueda dañada en snapshot"))?;
        let metadata: Value = entry
            .search_metadata_range
            .as_ref()
            .map(|range| serde_json::from_str(&entry.text[range.clone()]))
            .transpose()
            .map_err(|_| ("DETAIL", "metadatos de búsqueda dañados en snapshot"))?
            .unwrap_or_else(|| json!({}));
        if offset == ranges.len() {
            if let Some(continuation) = metadata.get("continuation").filter(|value| value.is_object()) {
                return Ok(json!({"_resume":continuation}));
            }
        }
        let mut content_truncated = false;
        if collection == "matches" {
            for row in &mut page {
                if let Some(text) = row.get("text").and_then(Value::as_str) {
                    let row_truncated = utf16_len(text) > 500;
                    content_truncated |= row_truncated;
                    if row_truncated {
                        row["text"] = json!(prefix(text, 500).trim_end());
                    }
                }
            }
        }
        let has_more = end < ranges.len() || metadata.get("continuation").is_some_and(Value::is_object);
        let next_offset = has_more.then_some(end);
        let cursor = json!({"tool":"harness_search","arguments":{
            "cursor":id,"offset":end,"session":session
        }});
        let mut result = json!({
            "offset":offset,
            "nextOffset":next_offset,
            "hasMore":has_more,
            "complete":complete,
            "scanComplete":complete,
            "snapshotOnly":!complete && metadata.get("continuation").is_none(),
            "continuationMode":if metadata.get("continuation").is_some() { json!("live") } else { Value::Null },
            "_complete":complete,
            "contentTruncated":content_truncated,
            "presentationTruncated":end < ranges.len(),
            "truncated":has_more || content_truncated || !complete,
            "omitted":ranges.len().saturating_sub(page.len()),
            "scanIncompleteReasons":metadata.get("scanIncompleteReasons").cloned().unwrap_or_else(|| json!([])),
            "cursorExpiresAt":expires_at,
            "cursorRetention":retention,
            "detailRef":id,
            "nextCall":if has_more { cursor } else { Value::Null }
        });
        result[collection] = json!(page);
        if let Some(metadata) = metadata.as_object() {
            for key in [
                "kind",
                "coverage",
                "discovery",
                "language",
                "syntaxOnly",
                "indexFreshness",
                "limits",
                "selector",
                "contextLines",
                "onlyMatching",
                "extracted",
                "retrieval",
                "ast",
                "limited",
                "timedOut",
            ] {
                if let Some(value) = metadata.get(key) {
                    result[key] = value.clone();
                }
            }
        }
        Ok(result)
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
            json!({"ref":id,"offset":offset,"text":entry.text[start..end],"nextOffset":if end_units < total { Some(end_units) } else { None },"totalCharacters":total,"format":"json-text","expiresAt":entry.expires_at,"expirationFormat":"unix-seconds","retention":if entry.session.is_some() {"server-session"} else {"ttl-or-capacity"},"nextCall":if end_units < total {json!({"tool":"harness_detail","arguments":{"ref":id,"offset":end_units,"maxChars":max_chars,"session":entry.session}})} else {Value::Null}}),
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
                let search_collection = if tool == "harness_search" {
                    ["matches", "paths", "files"]
                        .into_iter()
                        .find(|key| map.get(*key).is_some_and(Value::is_array))
                        .map(str::to_owned)
                } else {
                    None
                };
                let search_page_count = search_collection
                    .as_deref()
                    .and_then(|key| map.get(key))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                let search_total = search_collection
                    .as_deref()
                    .and_then(|key| map.get("_detail").and_then(|detail| detail.get(key)))
                    .and_then(Value::as_array)
                    .map_or(search_page_count, Vec::len);
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
                    .map(|mut value| {
                        if let Some(collection) = search_collection.as_deref() {
                            let mut metadata = json!({"collection":collection});
                            for key in ["kind", "coverage", "discovery", "language", "syntaxOnly", "indexFreshness", "limits", "selector", "contextLines", "onlyMatching", "extracted", "retrieval", "ast", "limited", "timedOut", "scanIncompleteReasons"] {
                                if let Some(item) = map.get(key) {
                                    metadata[key] = item.clone();
                                }
                            }
                            if let Some(continuation) = value.get("_continuation") {
                                metadata["continuation"] = continuation.clone();
                            }
                            value["_searchSnapshot"] = metadata;
                        }
                        let session = args
                            .get("session")
                            .and_then(Value::as_str)
                            .or_else(|| map.get("session").and_then(Value::as_str));
                        let default_limit = if map.get("kind") == Some(&json!("file_size")) {
                            30
                        } else {
                            40
                        };
                        let page_size = args
                            .get("limit")
                            .and_then(Value::as_u64)
                            .map_or(default_limit, |limit| limit as usize);
                        let saved = if let Some(collection) = search_collection.as_deref() {
                            self.save_search_for_session(
                                &value,
                                complete,
                                session,
                                collection,
                                page_size,
                            )
                        } else {
                            self.save_for_session(&value, complete, session)
                        };
                        saved
                        .map(|detail| {
                            if let (Some(_), Some(id)) = (
                                search_collection.as_deref(),
                                detail.get("ref").and_then(Value::as_str),
                            ) {
                                if self.has_search_index(id) {
                                    let has_more = search_total > search_page_count || value.get("_continuation").is_some_and(Value::is_object);
                                    map.insert("offset".into(), json!(0));
                                    map.insert("nextOffset".into(), if has_more { json!(search_page_count) } else { Value::Null });
                                    map.insert("hasMore".into(), json!(has_more));
                                    map.insert("scanComplete".into(), json!(complete));
                                    map.insert("snapshotOnly".into(), json!(!complete && value.get("_continuation").is_none()));
                                    map.insert("nextCall".into(), if has_more { json!({
                                        "tool":"harness_search",
                                        "arguments":{"cursor":id,"offset":search_page_count,"session":args.get("session")}
                                    }) } else { Value::Null });
                                } else {
                                    map.insert("paginationUnavailable".into(), json!(true));
                                }
                            }
                            detail
                        })
                        .unwrap_or_else(|| {
                            omitted["detailUnavailable"] = json!(true);
                            Value::Null
                        })
                    })
                    .or_else(|| {
                        args.get("cursor")
                            .and_then(Value::as_str)
                            .and_then(|id| self.search_reference(id))
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
                    "MOVE_EXISTS" => (false, "choose_unused_destination"),
                    "MOVE_IO" | "MOVE_POSTCHECK" => (false, "inspect_both_paths_before_retry"),
                    "MOVE_INDEX" => (false, "move_completed_refresh_index_only"),
                    "DELETE_IO" | "DELETE_POSTCHECK" => (false, "inspect_path_before_retry"),
                    "DELETE_INDEX" => (false, "delete_completed_refresh_index_only"),
                    "TIMEOUT" | "BUSY" => (true, "check_status_then_reduce_scope"),
                    "STALE" | "HASH" | "HASH_MISMATCH" => (false, "reread_source_and_hash"),
                    "MATCH" => (false, "align_oldText_with_the_hint_then_retry"),
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
                    "TOOL_PROFILE" => (false, "request_explicit_authorization_for_profile_restart"),
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
    fn search_snapshot_pages_without_rescanning_and_preserves_detail_reference() {
        let mut store = Results::default();
        let rows = json!([
            {"file":"a.rs","text":"row 1"},
            {"file":"b.rs","text":"row 2"},
            {"file":"c.rs","text":"row 3"},
            {"file":"d.rs","text":"row 4"}
        ]);
        let first = store.envelope(
            "harness_search",
            &json!({"limit":2}),
            Ok(json!({
                "matches":[rows[0].clone(), rows[1].clone()],
                "omitted":2,
                "retrieval":{"mode":"terms","ranking":"bm25","exactMatches":0},
                "_complete":true,
                "_detail":{"matches":rows}
            })),
        );
        let cursor = first["data"]["nextCall"]["arguments"]["cursor"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(first["data"]["hasMore"], true);
        assert_eq!(first["data"]["nextOffset"], 2);

        let page = store.search_page(&cursor, 2, None).unwrap();
        assert_eq!(page["matches"].as_array().unwrap().len(), 2);
        assert_eq!(page["matches"][0]["file"], "c.rs");
        assert_eq!(page["retrieval"]["ranking"], "bm25");
        assert_eq!(page["scanComplete"], true);
        assert_eq!(page["hasMore"], false);
        assert!(page["nextCall"].is_null());

        let second = store.envelope(
            "harness_search",
            &json!({"cursor":cursor,"offset":2}),
            Ok(page),
        );
        assert_eq!(second["detail"]["ref"], cursor);
        assert_eq!(second["data"]["matches"][0]["file"], "c.rs");
    }

    #[test]
    fn partial_search_snapshot_is_pageable_but_reports_scan_incomplete() {
        let mut store = Results::default();
        let rows = json!([
            {"file":"a.rs","text":"row 1"},
            {"file":"b.rs","text":"row 2"},
            {"file":"c.rs","text":"row 3"}
        ]);
        let first = store.envelope(
            "harness_search",
            &json!({"limit":1}),
            Ok(json!({
                "matches":[rows[0].clone()],
                "_complete":false,
                "scanIncompleteReasons":["time_budget"],
                "_detail":{"matches":rows,"scanIncompleteReasons":["time_budget"]}
            })),
        );
        let cursor = first["data"]["nextCall"]["arguments"]["cursor"]
            .as_str()
            .unwrap();
        assert_eq!(first["data"]["snapshotOnly"], true);
        let next = store.search_page(cursor, 1, None).unwrap();
        assert_eq!(next["scanComplete"], false);
        assert_eq!(next["snapshotOnly"], true);
        assert_eq!(next["scanIncompleteReasons"][0], "time_budget");
        assert_eq!(next["nextOffset"], 2);
    }

    #[test]
    fn file_size_snapshot_indexes_top_level_rows_with_nested_keys_and_escaped_text() {
        let mut store = Results::default();
        let rows = json!([
            {"path":"a[1].rs","text":"literal \\\"files\\\":[x] 😀","nested":{"files":[{"x":[1,2]}]}},
            {"path":"b.rs","nested":{"files":[{"files":"same key"}]}}
        ]);
        let first = store.envelope(
            "harness_search",
            &json!({"kind":"file_size","limit":1}),
            Ok(json!({
                "kind":"file_size",
                "files":[rows[0].clone()],
                "omitted":1,
                "_complete":true,
                "limits":{"files":10000},
                "_detail":{"files":rows,"limits":{"files":10000}}
            })),
        );
        let cursor = first["data"]["nextCall"]["arguments"]["cursor"]
            .as_str()
            .unwrap();
        let second = store.search_page(cursor, 1, None).unwrap();
        assert_eq!(second["files"][0]["path"], "b.rs");
        assert_eq!(
            second["files"][0]["nested"]["files"][0]["files"],
            "same key"
        );
        assert_eq!(second["limits"]["files"], 10000);
    }

    #[test]
    fn pagination_index_memory_is_counted_and_detail_survives_when_it_does_not_fit() {
        let mut store = Results::default();
        let snapshot = json!({"matches":[{"file":"a.rs","text":"needle"}],"_searchSnapshot":{}});
        let serialized = snapshot.to_string();
        store.max_bytes = size_of::<Entry>() + serialized.capacity() + 36 + 8;
        let detail = store
            .save_search_for_session(&snapshot, true, None, "matches", 1)
            .expect("the unindexed detail should still fit");
        let id = detail["ref"].as_str().unwrap();
        assert!(store.bytes <= store.max_bytes);
        assert!(store.value(id).is_ok());
        assert_eq!(store.search_page(id, 0, None).unwrap_err().0, "DETAIL");
    }

    #[test]
    fn search_snapshot_cursor_is_bound_to_its_session() {
        let mut store = Results::default();
        let detail = store
            .save_search_for_session(
                &json!({"matches":[{"file":"a.rs"}],"_searchSnapshot":{}}),
                false,
                Some("session-a"),
                "matches",
                1,
            )
            .unwrap();
        let id = detail["ref"].as_str().unwrap();
        assert_eq!(
            store.search_page(id, 0, Some("session-b")).unwrap_err().0,
            "DETAIL"
        );
        assert_eq!(
            store.search_page(id, 0, Some("session-a")).unwrap()["matches"][0]["file"],
            "a.rs"
        );
    }
    #[test]
    fn session_references_survive_ttl_capacity_and_repeated_paging() {
        let mut store = Results {
            max_entries: 2,
            max_bytes: 1024,
            ..Results::default()
        };
        let first = store
            .save_for_session(&json!({"text":"a".repeat(80)}), true, Some("s"))
            .unwrap();
        let id = first["ref"].as_str().unwrap();
        assert!(first["expiresAt"].is_null());
        assert_eq!(first["retention"], "server-session");
        store.ttl = Duration::ZERO;
        let page = store.get(id, 0, 16).unwrap();
        assert_eq!(page["nextCall"]["arguments"]["offset"], 16);
        assert_eq!(page["nextCall"]["arguments"]["session"], "s");
        assert!(
            store
                .save_for_session(&json!({"b":2}), true, Some("s"))
                .is_some()
        );
        assert!(
            store
                .save_for_session(&json!({"c":3}), true, Some("s"))
                .is_none()
        );
        assert!(store.value(id).is_ok());
        assert_eq!(store.get(id, 0, 16).unwrap()["text"], page["text"]);
        assert!(store.get(id, 16, 16).is_ok());
    }

    #[test]
    fn envelope_pins_session_details_including_bootstrap() {
        let mut store = Results::default();
        for (args, data) in [
            (json!({"session":"s"}), json!({"_detail":{"a":1}})),
            (json!({}), json!({"session":"s","_detail":{"a":1}})),
        ] {
            let result = store.envelope("test", &args, Ok(data));
            assert_eq!(result["detail"]["retention"], "server-session");
            assert_eq!(result["detail"]["nextCall"]["arguments"]["session"], "s");
        }
    }

    #[test]
    fn eviction_ttl_and_byte_limits() {
        let mut store = Results {
            max_entries: 1,
            max_bytes: size_of::<Entry>() + 36 + 100,
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

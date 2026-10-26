use super::*;
use toml_edit::{DocumentMut, Item, Table, value};

const CODEX: &str = ".codex/config.toml";
const ANTIGRAVITY: &str = ".agents/mcp_config.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientRecord {
    command: String,
    args: Vec<String>,
    created_file: bool,
    created_parent: bool,
}

pub(super) struct Plan {
    pub key: String,
    pub record: Option<ClientRecord>,
    bytes: Vec<u8>,
    expected: Option<String>,
}
impl Plan {
    pub fn apply(self, root: &Path) -> Result<(), String> {
        let current = storage::read_safe_file(root, Path::new(&self.key))?;
        if current.as_deref() == Some(self.bytes.as_slice()) {
            return Ok(());
        }
        storage::write_safe_file(
            root,
            Path::new(&self.key),
            &self.bytes,
            self.expected.as_deref(),
        )
    }
}

pub(super) fn validate_key(key: &str) -> Result<(), String> {
    if !matches!(key, CODEX | ANTIGRAVITY) {
        return Err("unknown installation client path".into());
    }
    Ok(())
}

fn json_document(bytes: Option<&[u8]>) -> Result<Value, String> {
    let document: Value = bytes
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|_| "invalid Antigravity MCP JSON")?
        .unwrap_or_else(|| json!({}));
    if !document.is_object() {
        return Err("Antigravity MCP config must be an object".into());
    }
    if document.get("mcpServers").is_some_and(|v| !v.is_object()) {
        return Err("mcpServers must be an object".into());
    }
    Ok(document)
}

fn toml_document(bytes: Option<&[u8]>) -> Result<DocumentMut, String> {
    let text =
        std::str::from_utf8(bytes.unwrap_or_default()).map_err(|_| "Codex config is not UTF-8")?;
    let document = text
        .parse::<DocumentMut>()
        .map_err(|_| "invalid Codex TOML")?;
    if document.get("mcp_servers").is_some_and(|v| !v.is_table()) {
        return Err("mcp_servers must be a TOML table".into());
    }
    Ok(document)
}

fn toml_entry(document: &DocumentMut) -> Option<&Item> {
    document.get("mcp_servers")?.get("harness")
}
fn matches_toml(entry: &Item, record: &ClientRecord) -> bool {
    entry.as_table().is_some_and(|table| table.len() == 2)
        && entry.get("command").and_then(Item::as_str) == Some(&record.command)
        && entry
            .get("args")
            .and_then(Item::as_array)
            .is_some_and(|array| {
                array.len() == record.args.len()
                    && array
                        .iter()
                        .zip(&record.args)
                        .all(|(a, b)| a.as_str() == Some(b))
            })
}

pub(super) fn plan(
    root: &Path,
    exe: &Path,
    owned: &BTreeMap<String, ClientRecord>,
) -> Result<Vec<Plan>, String> {
    let command = exe
        .to_str()
        .ok_or("executable path is not UTF-8")?
        .to_owned();
    let args = vec![
        "--root".into(),
        root.to_str().ok_or("root is not UTF-8")?.to_owned(),
    ];
    let mut result = Vec::new();
    for key in [CODEX, ANTIGRAVITY] {
        let previous = storage::read_safe_file(root, Path::new(key))?;
        let mut record = ClientRecord {
            command: command.clone(),
            args: args.clone(),
            created_file: previous.is_none(),
            created_parent: false,
        };
        let (bytes, existing, already_equal) = if key == CODEX {
            let mut doc = toml_document(previous.as_deref())?;
            record.created_parent = doc.get("mcp_servers").is_none();
            let existing = toml_entry(&doc).is_some();
            let equal = toml_entry(&doc).is_some_and(|e| matches_toml(e, &record));
            if existing
                && !equal
                && !owned
                    .get(key)
                    .is_some_and(|r| toml_entry(&doc).is_some_and(|e| matches_toml(e, r)))
            {
                return Err(
                    "existing Codex harness entry differs; preserved without overwriting".into(),
                );
            }
            if !equal {
                if doc.get("mcp_servers").is_none() {
                    doc["mcp_servers"] = Item::Table(Table::new());
                }
                let mut table = Table::new();
                table["command"] = value(&command);
                let mut array = toml_edit::Array::new();
                for arg in &args {
                    array.push(arg.as_str());
                }
                table["args"] = value(array);
                doc["mcp_servers"]["harness"] = Item::Table(table);
            }
            (doc.to_string().into_bytes(), existing, equal)
        } else {
            let mut doc = json_document(previous.as_deref())?;
            record.created_parent = doc.get("mcpServers").is_none();
            let desired = json!({"command":command,"args":args});
            let entry = doc.get("mcpServers").and_then(|v| v.get("harness"));
            let existing = entry.is_some();
            let equal = entry == Some(&desired);
            if existing
                && !equal
                && !owned
                    .get(key)
                    .is_some_and(|r| entry == Some(&json!({"command":r.command,"args":r.args})))
            {
                return Err(
                    "existing Antigravity harness entry differs; preserved without overwriting"
                        .into(),
                );
            }
            if !equal {
                if doc.get("mcpServers").is_none() {
                    doc["mcpServers"] = json!({});
                }
                doc["mcpServers"]["harness"] = desired;
            }
            (
                serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?,
                existing,
                equal,
            )
        };
        // An identical pre-existing entry is usable, but is not ours to remove.
        let managed = if let Some(old) = owned.get(key) {
            record.created_file = old.created_file;
            record.created_parent = old.created_parent;
            Some(record)
        } else if !existing {
            Some(record)
        } else {
            None
        };
        let bytes = if already_equal {
            previous.clone().unwrap_or(bytes)
        } else {
            bytes
        };
        result.push(Plan {
            key: key.into(),
            record: managed,
            bytes,
            expected: previous.as_deref().map(hash),
        });
    }
    Ok(result)
}

pub(super) fn remove(root: &Path, key: &str, record: &ClientRecord) -> Result<(), String> {
    validate_key(key)?;
    let Some(bytes) = storage::read_safe_file(root, Path::new(key))? else {
        return Ok(());
    };
    let expected = hash(&bytes);
    let (next, empty) = if key == CODEX {
        let mut doc = toml_document(Some(&bytes))?;
        if let Some(entry) = toml_entry(&doc) {
            if !matches_toml(entry, record) {
                return Err("modified Codex harness entry preserved".into());
            }
            doc["mcp_servers"]
                .as_table_mut()
                .ok_or("invalid mcp_servers")?
                .remove("harness");
        }
        if record.created_parent
            && doc
                .get("mcp_servers")
                .and_then(Item::as_table)
                .is_some_and(Table::is_empty)
        {
            doc.remove("mcp_servers");
        }
        let empty = doc.is_empty() && doc.to_string().trim().is_empty();
        (doc.to_string().into_bytes(), empty)
    } else {
        let mut doc = json_document(Some(&bytes))?;
        if let Some(entry) = doc.get("mcpServers").and_then(|v| v.get("harness")) {
            if entry != &json!({"command":record.command,"args":record.args}) {
                return Err("modified Antigravity harness entry preserved".into());
            }
            doc["mcpServers"]
                .as_object_mut()
                .ok_or("invalid mcpServers")?
                .remove("harness");
        }
        if record.created_parent
            && doc
                .get("mcpServers")
                .and_then(Value::as_object)
                .is_some_and(|o| o.is_empty())
        {
            doc.as_object_mut().unwrap().remove("mcpServers");
        }
        let empty = doc.as_object().unwrap().is_empty();
        (
            serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?,
            empty,
        )
    };
    if record.created_file && empty {
        storage::remove_safe_file(root, Path::new(key), &expected)
    } else {
        storage::write_safe_file(root, Path::new(key), &next, Some(&expected))
    }
}

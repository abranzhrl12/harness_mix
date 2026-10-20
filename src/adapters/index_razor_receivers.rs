//! Typed Razor receiver facts and negative/positive targets cached per publication.
use super::{Failure, INDEX, OperationContext, sql};
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Facts {
    calls: BTreeMap<(String, i64, i64), (String, String)>,
    namespaces: Vec<String>,
    namespace_key: String,
    aliases: BTreeMap<String, String>,
}
impl Facts {
    pub(super) fn reserved(&self) -> usize {
        self.calls
            .iter()
            .map(|((name, _, _), (ty, method))| name.len() + ty.len() + method.len() + 128)
            .sum::<usize>()
            + self.namespaces.iter().map(String::len).sum::<usize>()
    }
}
#[derive(Default)]
pub(super) struct Targets {
    entries: BTreeMap<(String, String, String), Option<(String, String)>>,
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
fn type_name(value: &str) -> bool {
    value.len() <= 512
        && value
            .strip_prefix("global::")
            .unwrap_or(value)
            .split('.')
            .all(identifier)
}
pub(super) fn analyze(source: &str, context: &OperationContext) -> Result<Facts, Failure> {
    let mut blocks = Vec::new();
    let symbols = super::super::csharp_syntax::collect_cshtml_blocks(
        source,
        "",
        context,
        &mut blocks,
        false,
    )?;
    let mut header = String::new();
    for symbol in symbols.iter().filter(|s| s.kind == "razor_model") {
        if type_name(&symbol.name) {
            header.push_str(&format!("public {} Model; ", symbol.name));
        }
    }
    let mut facts = Facts::default();
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut scan_block = 0;
    while at < bytes.len() {
        context.check()?;
        while blocks
            .get(scan_block)
            .is_some_and(|(range, _)| range.end < at)
        {
            scan_block += 1;
        }
        if let Some((range, _)) = blocks.get(scan_block) {
            if at >= range.start.saturating_sub(1) && at <= range.end {
                at = range.end + 1;
                continue;
            }
        }
        if let Some(end) = super::super::csharp_syntax::razor_markup_end(bytes, at) {
            at = end;
            continue;
        }
        if bytes[at..].starts_with(b"@*") {
            at = source[at + 2..]
                .find("*@")
                .map_or(bytes.len(), |n| at + n + 4);
            continue;
        }
        if bytes[at..].starts_with(b"@@") {
            at += 2;
            continue;
        }
        if bytes[at..].starts_with(b"@{") {
            if let Some(end) = super::super::razor_syntax::block_end(bytes, at + 1) {
                at = end;
                continue;
            }
        }
        if bytes[at..].starts_with(b"@(") {
            if let Some(end) = super::super::razor_syntax::delimited_end(bytes, at + 1, b'(', b')')
            {
                at = end;
                continue;
            }
        }
        let tail = &bytes[at..];
        if tail.starts_with(b"@inject ") || tail.starts_with(b"@using ") {
            let end = source[at..].find('\n').map_or(source.len(), |n| at + n);
            let parts = source[at..end].split_whitespace().collect::<Vec<_>>();
            if parts.len() == 3
                && parts[0] == "@inject"
                && type_name(parts[1])
                && identifier(parts[2])
            {
                header.push_str(&format!("public {} {}; ", parts[1], parts[2]));
            } else if parts.len() == 2
                && parts[0] == "@using"
                && type_name(parts[1].trim_end_matches(';'))
            {
                facts
                    .namespaces
                    .push(parts[1].trim_end_matches(';').to_owned());
            } else if parts.len() == 4
                && parts[0] == "@using"
                && identifier(parts[1])
                && parts[2] == "="
                && type_name(parts[3].trim_end_matches(';'))
            {
                if facts
                    .aliases
                    .insert(
                        parts[1].to_owned(),
                        parts[3].trim_end_matches(';').to_owned(),
                    )
                    .is_some()
                {
                    return Ok(Facts::default());
                }
            } else if parts.first() == Some(&"@using") && source[at..end].contains('=') {
                return Ok(Facts::default());
            }
            at = end;
            continue;
        }
        at += 1;
    }
    facts.namespaces.sort();
    facts.namespaces.dedup();
    if facts.namespaces.len() > 64 {
        return Ok(Facts::default());
    }
    facts.namespace_key = harness_domain::hash(facts.namespaces.join("\0").as_bytes());
    let projection = super::super::razor_render::project(source, &header, context)?;
    context.check()?;
    if let Some(calls) = codegraph_native::csharp_receivers::analyze(&projection.text) {
        if calls.len() > 20_000 {
            return Ok(Facts::default());
        }
        for call in calls {
            let location = if call.line > projection.line_offset {
                projection.location(call.line, call.column)
            } else if call.line > 1 {
                Some((call.line - 1, call.column))
            } else {
                None
            };
            if let Some((line, column)) = location {
                facts.calls.insert(
                    (call.name, line as i64, column as i64),
                    (call.type_name, call.method),
                );
            }
        }
    }
    Ok(facts)
}
impl Targets {
    pub(super) fn resolve(
        &mut self,
        db: &Connection,
        facts: &Facts,
        name: &str,
        line: i64,
        col: i64,
        context: &OperationContext,
    ) -> Result<Option<(String, String)>, Failure> {
        let Some((ty, method)) = facts.calls.get(&(name.to_owned(), line, col)) else {
            return Ok(None);
        };
        let alias_type = ty.split_once('.').map_or_else(
            || facts.aliases.get(ty).cloned(),
            |(head, tail)| {
                facts
                    .aliases
                    .get(head)
                    .map(|alias| format!("{alias}.{tail}"))
            },
        );
        let ty = alias_type.as_ref().unwrap_or(ty);
        let key = (ty.clone(), method.clone(), facts.namespace_key.clone());
        if let Some(target) = self.entries.get(&key) {
            return Ok(target.clone());
        }
        context.check()?;
        let bare = ty.strip_prefix("global::").unwrap_or(ty);
        let names = if ty.starts_with("global::") || bare.contains('.') {
            vec![bare.to_owned()]
        } else {
            std::iter::once(bare.to_owned())
                .chain(facts.namespaces.iter().map(|n| format!("{n}.{bare}")))
                .collect()
        };
        let json = serde_json::to_string(&names).map_err(|_| INDEX)?;
        let mut statement = sql(db.prepare_cached(
            "WITH types AS MATERIALIZED (SELECT id,qualified_name FROM nodes WHERE name=?3 COLLATE BINARY AND language='csharp' AND kind IN ('class','struct','record') AND replace(qualified_name,'::','.') IN (SELECT value FROM json_each(?1)) LIMIT 2) SELECT m.id,replace(t.qualified_name,'::','.') FROM types t JOIN edges e ON e.source=t.id AND e.kind='contains' JOIN nodes m ON m.id=e.target WHERE (SELECT count(*) FROM types)=1 AND m.name=?2 COLLATE BINARY AND m.kind='method' AND m.visibility='public' AND m.is_static=0 LIMIT 2"))?;
        let ids = sql(sql(statement.query_map(
            [json.as_str(), method, bare.rsplit('.').next().unwrap_or("")],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ))?
        .collect::<Result<Vec<_>, _>>())?;
        let target = if ids.len() == 1 {
            ids.into_iter().next()
        } else {
            None
        };
        self.entries.insert(key, target.clone());
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_thousand_calls_share_one_positive_and_one_negative_target_lookup() {
        let source = format!(
            "@functions {{ Demo.Controller receiver; void Run() {{\n{}\n{}\n}} }}",
            "receiver.Execute();\n".repeat(1000),
            "receiver.Missing();\n".repeat(1000)
        );
        let context = OperationContext::default();
        let facts = analyze(&source, &context).unwrap();
        assert_eq!(facts.calls.len(), 2000);
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES('type','class','Controller','Demo::Controller','Controller.cs','csharp',1,1,0,1,0); INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at,visibility,is_static) VALUES('method','method','Execute','Demo::Controller::Execute','Controller.cs','csharp',1,1,0,1,0,'public',0); INSERT INTO edges(source,target,kind) VALUES('type','method','contains');").unwrap();
        let mut targets = Targets::default();
        for (name, line, col) in facts.calls.keys() {
            let result = targets
                .resolve(&db, &facts, name, *line, *col, &context)
                .unwrap();
            assert_eq!(result.is_some(), name == "receiver.Execute");
        }
        assert_eq!(targets.entries.len(), 2);
    }
}

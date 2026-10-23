//! Bounded, operation-local SQL DDL evidence for literal C# SqlCommand calls.
//! No database connection is made. Dynamic SQL, temporary tables and ambiguous
//! definitions remain unresolved. SQL files retain raw hashes for Explore.
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{Connection, params};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(in crate::adapters) struct Snapshot {
    objects: Option<Vec<Object>>,
    files: Files,
    complete: bool,
    published: bool,
}

pub(in crate::adapters) struct Target {
    pub id: String,
    pub hash: String,
}

struct Object {
    id: String,
    kind: &'static str,
    name: String,
    path: String,
    hash: String,
    line: usize,
    end: usize,
    dependencies: Vec<(String, usize)>,
}

#[derive(Clone)]
struct Token {
    text: String,
    line: usize,
    identifier: bool,
    quoted: bool,
}

fn sql<T>(value: rusqlite::Result<T>) -> Result<T, Failure> {
    value.map_err(|_| ("SYNC_INDEX", "Local SQL graph publication failed"))
}

/// Re-parse the current SQL bytes instead of trusting a persisted source range.
/// This reader is deliberately scoped to nodes created by the local DDL subset.
pub(in crate::adapters) fn read_object(
    project: &Project,
    path: &str,
    id: &str,
    context: &OperationContext,
) -> Result<serde_json::Value, Failure> {
    let bytes = project.bytes_with_limit(path, context, 1024 * 1024)?;
    let digest = hash(&bytes);
    let source = harness_domain::encoding::decode_source(&bytes)?;
    let objects = parse(path, &source, &digest);
    let object = objects
        .iter()
        .find(|object| object.id == id)
        .ok_or(("SYMBOL_NOT_FOUND", "Local SQL declaration no longer exists"))?;
    let text = source
        .split_inclusive('\n')
        .skip(object.line.saturating_sub(1))
        .take(object.end.saturating_sub(object.line) + 1)
        .collect::<String>();
    Ok(json!({"path":path,"hash":digest,"syntaxOnly":true,
        "symbol":{"name":object.name.rsplit('.').next().unwrap_or(&object.name),
            "kind":object.kind,"start":object.line,"end":object.end,
            "symbolId":object.id,"signature":format!("{} {}", object.kind, object.name)},
        "text":text,"coverage":"local SQL literal DDL subset; no SQL compilation"}))
}

impl Snapshot {
    pub(in crate::adapters::index_store) fn resolve(
        &mut self,
        db: &Connection,
        project: &Project,
        name: &str,
        context: &OperationContext,
        freshness: &mut super::repository::framework::Snapshot,
    ) -> Result<Option<Target>, Failure> {
        if self.objects.is_none() {
            let (objects, complete, files) = inventory(project, context)?;
            self.complete = complete;
            self.files = files;
            self.objects = Some(objects);
        }
        if !self.complete {
            return Ok(None);
        }
        let tokens = tokenize(name);
        let Some((name, used)) = qualified(&tokens, 0) else {
            return Ok(None);
        };
        if used != tokens.len() {
            return Ok(None);
        }
        let objects = self.objects.as_deref().unwrap_or_default();
        for (path, (digest, _)) in &self.files {
            freshness.remember(path, digest)?;
        }
        // A snapshot may outlive a rolled-back file publication. Check the DB,
        // rather than treating an in-memory cache as proof of committed nodes.
        let target = unique(objects, "stored_procedure", &name);
        let exists = if let Some(target) = target {
            sql(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1)",
                [&target.id],
                |row| row.get::<_, bool>(0),
            ))?
        } else {
            true
        };
        if !self.published || !exists {
            publish(db, objects, &self.files, context)?;
            self.published = true;
        }
        Ok(target.map(|object| Target {
            id: object.id.clone(),
            hash: object.hash.clone(),
        }))
    }
}

type Files = BTreeMap<String, (String, usize)>;

fn inventory(
    project: &Project,
    context: &OperationContext,
) -> Result<(Vec<Object>, bool, Files), Failure> {
    let mut result = Vec::new();
    let mut files = Files::new();
    let mut bytes_read = 0usize;
    // Match normal repository ignore rules, never follow links or descend into
    // index/build output and protected secret paths.
    let mut builder = ignore::WalkBuilder::new(&project.root);
    builder.follow_links(false).filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        !matches!(
            name.as_str(),
            ".git" | ".codegraph" | "target" | "node_modules" | "bin" | "obj"
        ) && !name.starts_with(".env")
            && !name.starts_with("secret")
            && !name.starts_with("credential")
    });
    let ignore_path = project.root.join(".harnessignore");
    if ignore_path.exists() && builder.add_ignore(ignore_path).is_some() {
        return Ok((result, false, files));
    }
    let walk = builder.build();
    let mut visited = 0usize;
    for entry in walk {
        context.check()?;
        visited += 1;
        if visited > 100_000 {
            return Ok((result, false, files));
        }
        let Ok(entry) = entry else {
            return Ok((result, false, files));
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file())
            || !entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
        {
            continue;
        }
        if files.len() >= 512 {
            return Ok((result, false, files));
        }
        let Ok(relative) = entry.path().strip_prefix(&project.root) else {
            return Ok((result, false, files));
        };
        let path = relative.to_string_lossy().replace('\\', "/");
        if crate::adapters::index_inventory::restricted(&path) {
            continue;
        }
        let raw = match project.bytes_with_limit(&path, context, 1024 * 1024) {
            Ok(raw) => raw,
            Err(_) => {
                context.check()?;
                return Ok((result, false, files));
            }
        };
        bytes_read += raw.len();
        if bytes_read > 16 * 1024 * 1024 {
            return Ok((result, false, files));
        }
        let source = match harness_domain::encoding::decode_source(&raw) {
            Ok(source) => source,
            Err(_) => return Ok((result, false, files)),
        };
        let digest = hash(&raw);
        let parsed = parse(&path, &source, &digest);
        if result.len() + parsed.len() > 10_000 {
            return Ok((result, false, files));
        }
        files.insert(path, (digest, raw.len()));
        result.extend(parsed);
    }
    Ok((result, true, files))
}

fn unique<'a>(objects: &'a [Object], kind: &str, name: &str) -> Option<&'a Object> {
    let mut matches = objects.iter().filter(|object| {
        object.kind == kind
            && (object.name.eq_ignore_ascii_case(name)
                || (!name.contains('.')
                    && object
                        .name
                        .rsplit('.')
                        .next()
                        .is_some_and(|tail| tail.eq_ignore_ascii_case(name))))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn publish(
    db: &Connection,
    objects: &[Object],
    files: &Files,
    context: &OperationContext,
) -> Result<(), Failure> {
    let ids = objects
        .iter()
        .map(|object| object.id.as_str())
        .collect::<BTreeSet<_>>();
    let old = {
        let mut statement =
            sql(db.prepare("SELECT id,file_path FROM nodes WHERE id LIKE 'sql-local:%'"))?;
        sql(sql(statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }))?
        .collect::<rusqlite::Result<Vec<_>>>())?
    };
    let old_paths = old
        .iter()
        .map(|(_, path)| path.as_str())
        .collect::<BTreeSet<_>>();
    for (id, _) in &old {
        context.check()?;
        if !ids.contains(id.as_str()) {
            sql(db.execute("DELETE FROM nodes WHERE id=?1", [id]))?;
        }
    }
    for path in &old_paths {
        if !files.contains_key(*path) {
            sql(db.execute("DELETE FROM files WHERE path=?1 AND language='sql' AND NOT EXISTS(SELECT 1 FROM nodes WHERE file_path=?1)", [path]))?;
        }
    }
    for object in objects {
        context.check()?;
        sql(db.execute(
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES(?1,?2,?3,?4,?5,'sql',?6,?7,0,0,unixepoch()*1000) ON CONFLICT(id) DO UPDATE SET start_line=excluded.start_line,end_line=excluded.end_line,updated_at=excluded.updated_at",
            params![object.id,object.kind,object.name.rsplit('.').next().unwrap_or(&object.name),object.name,object.path,object.line as i64,object.end as i64],
        ))?;
        sql(db.execute("DELETE FROM edges WHERE source=?1 AND json_extract(metadata,'$.resolvedBy')='local-sql-literal-v1'", [&object.id]))?;
    }
    for (path, (digest, size)) in files {
        let count = objects.iter().filter(|object| &object.path == path).count();
        if count == 0 && !old_paths.contains(path.as_str()) {
            continue;
        }
        // Keep a file identity even after the last DDL declaration is removed,
        // so a later file deletion can clean up only our own tracked SQL rows.
        let file_id = format!("sql-local:file:{}", hash(path.as_bytes()));
        let end = objects
            .iter()
            .filter(|object| &object.path == path)
            .map(|object| object.end)
            .max()
            .unwrap_or(1);
        sql(db.execute("INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES(?1,'file',?2,?2,?2,'sql',1,?3,0,0,unixepoch()*1000) ON CONFLICT(id) DO UPDATE SET end_line=excluded.end_line,updated_at=excluded.updated_at", params![file_id,path,end as i64]))?;
        sql(db.execute("INSERT INTO files(path,content_hash,language,size,modified_at,indexed_at,node_count) VALUES(?1,?2,'sql',?3,0,unixepoch()*1000,?4) ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,size=excluded.size,indexed_at=excluded.indexed_at,node_count=excluded.node_count", params![path,digest,*size as i64,(count+1) as i64]))?;
        for object in objects.iter().filter(|object| &object.path == path) {
            sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,provenance) VALUES(?1,?2,'contains',?3,0,'heuristic')",params![file_id,object.id,object.line as i64]))?;
        }
    }
    for object in objects
        .iter()
        .filter(|object| object.kind == "stored_procedure")
    {
        for (name, line) in &object.dependencies {
            context.check()?;
            let Some(target) = unique(objects, "table", name) else {
                continue;
            };
            let metadata = json!({"resolvedBy":"local-sql-literal-v1","refName":name,
                "sourceHash":object.hash,"targetHash":target.hash,"confidence":0.9})
            .to_string();
            sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'references',?3,0,?4,'heuristic')",params![object.id,target.id,*line as i64,metadata]))?;
        }
    }
    Ok(())
}

fn qualified(tokens: &[Token], start: usize) -> Option<(String, usize)> {
    let first = tokens.get(start)?;
    if !first.identifier {
        return None;
    }
    let mut name = first.text.clone();
    let mut at = start + 1;
    while tokens.get(at).is_some_and(|token| token.text == ".") {
        let next = tokens.get(at + 1)?;
        if !next.identifier {
            return None;
        }
        name.push('.');
        name.push_str(&next.text);
        at += 2;
    }
    // Cross-database/server names depend on connection context.
    if name.split('.').count() > 2 || name.starts_with(['@', '#']) {
        return None;
    }
    Some((name, at))
}

fn parse(path: &str, source: &str, digest: &str) -> Vec<Object> {
    let tokens = tokenize(source);
    let mut objects = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if token.quoted || !token.text.eq_ignore_ascii_case("CREATE") {
            continue;
        }
        let mut next = at + 1;
        if tokens
            .get(next)
            .is_some_and(|token| token.text.eq_ignore_ascii_case("OR"))
            && tokens
                .get(next + 1)
                .is_some_and(|token| token.text.eq_ignore_ascii_case("ALTER"))
        {
            next += 2;
        }
        let Some(kind) = tokens.get(next) else {
            continue;
        };
        let kind = match kind.text.to_ascii_uppercase().as_str() {
            "PROC" | "PROCEDURE" => "stored_procedure",
            "TABLE" => "table",
            _ => continue,
        };
        let Some((name, body)) = qualified(&tokens, next + 1) else {
            continue;
        };
        let end = (body..tokens.len())
            .find(|&index| {
                let token = &tokens[index];
                !token.quoted
                    && token.text.eq_ignore_ascii_case("GO")
                    && (index == 0 || tokens[index - 1].line < token.line)
                    && tokens
                        .get(index + 1)
                        .is_none_or(|next| next.line > token.line)
            })
            .unwrap_or(tokens.len());
        let mut dependencies = Vec::new();
        if kind == "stored_procedure" {
            let ctes = (body..end)
                .filter_map(|index| {
                    let token = &tokens[index];
                    (token.identifier
                        && tokens
                            .get(index + 1)
                            .is_some_and(|next| next.text.eq_ignore_ascii_case("AS"))
                        && tokens.get(index + 2).is_some_and(|next| next.text == "("))
                    .then(|| token.text.to_ascii_lowercase())
                })
                .collect::<BTreeSet<_>>();
            // Column lists in CTE declarations need a deeper SQL grammar.
            // Defer dependencies for that batch rather than bind a homonym.
            let complex_cte = (body..end).any(|index| {
                tokens[index].text.eq_ignore_ascii_case("WITH")
                    && tokens.get(index + 1).is_some_and(|next| next.identifier)
                    && tokens.get(index + 2).is_some_and(|next| next.text == "(")
            });
            let has_from = tokens[body..end].iter().any(|token| {
                !token.quoted && matches!(token.text.to_ascii_uppercase().as_str(), "FROM" | "JOIN")
            });
            for index in body..end {
                if complex_cte {
                    break;
                }
                let token = &tokens[index];
                if !token.quoted
                    && matches!(
                        token.text.to_ascii_uppercase().as_str(),
                        "FROM" | "JOIN" | "UPDATE" | "INTO"
                    )
                {
                    if let Some((name, after)) = qualified(&tokens, index + 1) {
                        if ctes.contains(&name.to_ascii_lowercase())
                            || (token.text.eq_ignore_ascii_case("UPDATE")
                                && has_from
                                && !name.contains('.'))
                            || (matches!(token.text.to_ascii_uppercase().as_str(), "FROM" | "JOIN")
                                && tokens.get(after).is_some_and(|next| next.text == "("))
                        {
                            continue;
                        }
                        dependencies.push((name, token.line));
                    }
                }
            }
        }
        objects.push(Object {
            id: format!(
                "sql-local:{}",
                hash(format!("{path}:{kind}:{}", name.to_ascii_lowercase()).as_bytes())
            ),
            kind,
            name,
            path: path.to_owned(),
            hash: digest.to_owned(),
            line: token.line,
            end: tokens
                .get(end.saturating_sub(1))
                .map_or(token.line, |token| token.line),
            dependencies,
        });
    }
    objects
}

/// Tokenize identifiers while removing comments and all SQL string contents.
/// A string is retained as a barrier so dynamic FROM/EXEC cannot join tokens.
fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let (mut at, mut line) = (0, 1);
    while at < bytes.len() {
        let start = at;
        let start_line = line;
        if bytes[at].is_ascii_whitespace() {
            if bytes[at] == b'\n' {
                line += 1
            }
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"--") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth > 0 {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    if bytes[at] == b'\n' {
                        line += 1
                    }
                    at += 1;
                }
            }
            continue;
        }
        if matches!(bytes[at], b'\'' | b'"' | b'[') {
            let open = bytes[at];
            let close = if open == b'[' { b']' } else { open };
            at += 1;
            let mut value = Vec::new();
            while at < bytes.len() {
                if bytes[at] == close {
                    at += 1;
                    if bytes.get(at) == Some(&close) {
                        value.push(close);
                        at += 1;
                        continue;
                    }
                    break;
                }
                if bytes[at] == b'\n' {
                    line += 1
                }
                value.push(bytes[at]);
                at += 1;
            }
            tokens.push(Token {
                text: if open == b'\'' {
                    String::new()
                } else {
                    String::from_utf8_lossy(&value).into_owned()
                },
                line: start_line,
                identifier: open != b'\'',
                quoted: true,
            });
            continue;
        }
        let identifier = bytes[at].is_ascii_alphabetic()
            || matches!(bytes[at], b'_' | b'@' | b'#')
            || bytes[at] >= 128;
        at += 1;
        if identifier {
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric()
                    || matches!(bytes[at], b'_' | b'@' | b'#' | b'$')
                    || bytes[at] >= 128)
            {
                at += 1
            }
        }
        tokens.push(Token {
            text: source[start..at].to_owned(),
            line: start_line,
            identifier,
            quoted: false,
        });
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_literals_and_comments_do_not_create_objects_or_dependencies() {
        let source = "-- CREATE TABLE dbo.Fake(x int)\nCREATE TABLE [dbo].[Orders](id int);\nGO\nCREATE OR ALTER PROC [dbo].[Find] AS\nSELECT 'FROM dbo.Fake' FROM [dbo].[Orders]; /* JOIN dbo.Fake */\nGO";
        let objects = parse("schema.sql", source, "hash");
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[1].name, "dbo.Find");
        assert_eq!(objects[1].dependencies, vec![("dbo.Orders".to_owned(), 5)]);
    }

    #[test]
    fn cte_and_table_valued_function_homonyms_are_not_bound_to_tables() {
        let objects = parse(
            "schema.sql",
            "CREATE PROC Find AS\nWITH Orders AS (SELECT 1 AS Id) SELECT * FROM Orders;\nSELECT * FROM dbo.Orders(1);\nGO",
            "hash",
        );
        assert!(objects[0].dependencies.is_empty());
    }

    #[test]
    fn update_alias_does_not_bind_an_unrelated_table_with_that_name() {
        let objects = parse(
            "schema.sql",
            "CREATE PROC Find AS UPDATE Alias SET Id=1 FROM dbo.Orders Alias;\nGO",
            "hash",
        );
        assert_eq!(objects[0].dependencies, vec![("dbo.Orders".to_owned(), 1)]);
    }

    #[test]
    fn removing_last_declaration_then_sql_file_cleans_its_tracking_row() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        let context = OperationContext::default();
        let objects = parse("schema.sql", "CREATE TABLE dbo.Orders(id int);", "first");
        let mut files = Files::from([("schema.sql".to_owned(), ("first".to_owned(), 32))]);
        publish(&db, &objects, &files, &context).unwrap();
        files.insert("schema.sql".to_owned(), ("empty".to_owned(), 0));
        publish(&db, &[], &files, &context).unwrap();
        let digest: String = db
            .query_row(
                "SELECT content_hash FROM files WHERE path='schema.sql'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(digest, "empty");
        publish(&db, &[], &Files::new(), &context).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM files", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

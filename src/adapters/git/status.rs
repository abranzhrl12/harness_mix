use super::reader::Reader;
use crate::adapters::project::Project;
use harness_domain::{
    Failure, contracts::GitStatusArgs, execution::OperationContext, results::utf16_len,
};
use serde_json::{Value, json};
use std::time::Instant;

pub(super) fn parse_status(text: &str) -> Result<Value, Failure> {
    if !text.is_empty() && !text.ends_with('\0') {
        return Err(("GIT_FORMAT", "Incomplete Git status"));
    }
    let mut result = json!({"branch":null,"head":null,"upstream":null,"ahead":null,"behind":null,"detached":false,"unborn":false,"entries":[]});
    let mut entries = Vec::new();
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if let Some(head) = record.strip_prefix("# branch.oid ") {
            result["unborn"] = json!(head == "(initial)");
            if head != "(initial)" {
                result["head"] = json!(head);
            }
        } else if let Some(branch) = record.strip_prefix("# branch.head ") {
            result["detached"] = json!(branch == "(detached)");
            if branch != "(detached)" {
                result["branch"] = json!(branch);
            }
        } else if let Some(upstream) = record.strip_prefix("# branch.upstream ") {
            result["upstream"] = json!(upstream);
        } else if let Some(ab) = record.strip_prefix("# branch.ab ") {
            let (a, b) = ab
                .split_once(' ')
                .ok_or(("GIT_FORMAT", "Invalid branch counts"))?;
            let parse = |s: Option<&str>| {
                s.and_then(|s| s.parse::<u64>().ok())
                    .ok_or(("GIT_FORMAT", "Invalid branch counts"))
            };
            result["ahead"] = json!(parse(a.strip_prefix('+'))?);
            result["behind"] = json!(parse(b.strip_prefix('-'))?);
        } else if let Some(path) = record.strip_prefix("? ") {
            entries.push(json!({"path":path,"xy":"??","kind":"untracked"}));
        } else if record.starts_with("# ") {
            continue;
        } else {
            let fields = match record.as_bytes()[0] {
                b'1' => 8,
                b'2' => 9,
                b'u' => 10,
                _ => return Err(("GIT_FORMAT", "Unknown status record")),
            };
            let parts: Vec<_> = record.splitn(fields + 1, ' ').collect();
            if parts.len() != fields + 1 || parts[1].len() != 2 || parts[fields].is_empty() {
                return Err(("GIT_FORMAT", "Incomplete status record"));
            }
            let mut entry = json!({"path":parts[fields],"xy":parts[1],"kind":if record.starts_with('u') {"conflict"} else {"tracked"}});
            if record.starts_with('2') {
                entry["from"] = json!(
                    records
                        .next()
                        .filter(|s| !s.is_empty())
                        .ok_or(("GIT_FORMAT", "Missing rename source"))?
                );
            }
            entries.push(entry);
        }
    }
    result["entries"] = json!(entries);
    Ok(result)
}

pub(super) fn status(
    project: &Project,
    args: &GitStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let limit = args.limit.unwrap_or(30);
    let offset = args.offset.unwrap_or(0);
    if !(1..=100).contains(&limit) || offset > 1000000 {
        return Err(("ARGUMENT", "Git limit 1..100, offset 0..1000000"));
    }
    let started = Instant::now();
    let reader = Reader::new(project, context)?;
    let mode = if args.include_untracked == Some(false) {
        "--untracked-files=no"
    } else {
        "--untracked-files=normal"
    };
    let text = reader.execute(
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--no-renames",
            "--ignore-submodules=all",
            mode,
        ],
        false,
    )?;
    let mut full = parse_status(&text)?;
    let entries = full["entries"]
        .as_array()
        .ok_or(("GIT_FORMAT", "Missing entries"))?;
    let mut counts = json!({"staged":0,"modified":0,"untracked":0,"conflicts":0});
    for entry in entries {
        let mut increment = |key: &str| {
            counts[key] = json!(counts[key].as_u64().unwrap_or(0) + 1);
        };
        match entry["kind"].as_str() {
            Some("untracked") => increment("untracked"),
            Some("conflict") => increment("conflicts"),
            _ => {
                let xy = entry["xy"].as_str().unwrap_or("..").as_bytes();
                if xy[0] != b'.' {
                    increment("staged");
                }
                if xy[1] != b'.' {
                    increment("modified");
                }
            }
        }
    }
    let total = entries.len();
    let mut selected = Vec::new();
    let mut used = 0;
    for entry in entries.iter().skip(offset).take(limit) {
        let size = utf16_len(&entry.to_string());
        if used + size > 6000 {
            if selected.is_empty() {
                return Err(("GIT_SIZE", "One status entry exceeds the response budget"));
            }
            break;
        }
        used += size;
        selected.push(entry.clone());
    }
    let next = (offset + selected.len() < total).then_some(offset + selected.len());
    let detail = full.clone();
    full["entries"] = json!(selected);
    full["counts"] = counts;
    full["total"] = json!(total);
    full["offset"] = json!(offset);
    full["nextOffset"] = json!(next);
    full["milliseconds"] = json!(started.elapsed().as_millis() as u64);
    full["scope"] = json!("workspace; submodules excluded; untracked directories grouped");
    if next.is_some() {
        full["truncated"] = json!(true);
        full["_detail"] = detail;
    }
    Ok(full)
}

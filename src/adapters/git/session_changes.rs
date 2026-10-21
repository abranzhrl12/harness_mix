use super::reader::Reader;
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Report observed edits even when Git omits ignored paths from ordinary status.
/// Content is never read; persisted means the edited path still exists on disk.
pub(super) fn session_changes(
    project: &Project,
    edits: &[Value],
    context: &OperationContext,
) -> Result<Value, Failure> {
    if edits.is_empty() {
        return Ok(json!([]));
    }
    let reader = Reader::new(project, context)?;
    let mut seen = BTreeSet::new();
    let mut changes = Vec::new();
    for edit in edits.iter().rev() {
        let Some(path) = edit["path"].as_str() else {
            continue;
        };
        if !seen.insert(path.to_string()) {
            continue;
        }
        let target = project.target_optional(path)?;
        let output = reader.execute(
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--ignored=matching",
                "--untracked-files=all",
                "--ignore-submodules=all",
                "--",
                path,
            ],
            false,
        )?;
        let code = output.get(..2).unwrap_or("");
        let git_status = match code {
            "!!" => "ignored",
            "??" => "untracked",
            "" => "clean",
            _ => "tracked",
        };
        changes.push(
            json!({"path":path,"gitStatus":git_status,"xy":code,"persisted":target.is_file()}),
        );
    }
    Ok(json!(changes))
}

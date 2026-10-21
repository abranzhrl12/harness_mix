use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use std::path::{Path, PathBuf};

pub(super) const METADATA_ERROR: Failure =
    ("GIT_REPO", "Invalid or unsupported Git worktree metadata");

// Git worktree metadata may live outside the workspace. Authorize only the
// reciprocal .git <-> common/worktrees/id registration, never general file access.
pub(super) fn metadata_path(path: &Path) -> Result<PathBuf, Failure> {
    if path.components().count() > 128 {
        return Err(METADATA_ERROR);
    }
    // Inspect the original spelling before canonicalization, including ancestors
    // before `..`, so canonicalization cannot hide a link or reparse point.
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let meta = std::fs::symlink_metadata(ancestor).map_err(|_| METADATA_ERROR)?;
        if meta.file_type().is_symlink() {
            return Err(METADATA_ERROR);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(METADATA_ERROR);
            }
        }
    }
    path.canonicalize().map_err(|_| METADATA_ERROR)
}

fn metadata_reference(base: &Path, text: &str) -> Result<PathBuf, Failure> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
        return Err(METADATA_ERROR);
    }
    let path = Path::new(text);
    // Drive-relative paths are ambiguous on Windows; fully relative paths are OK.
    if !path.is_absolute()
        && path.components().any(|c| {
            matches!(
                c,
                std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
    {
        return Err(METADATA_ERROR);
    }
    metadata_path(&base.join(path))
}

fn metadata_text(
    directory: &Path,
    name: &str,
    context: &OperationContext,
) -> Result<String, Failure> {
    metadata_path(&directory.join(name))?;
    let project = Project::new(directory).map_err(|_| METADATA_ERROR)?;
    let bytes = project
        .bytes_with_limit(name, context, 8192)
        .map_err(|error| {
            if matches!(error.0, "CANCELLED" | "TIMEOUT") {
                error
            } else {
                METADATA_ERROR
            }
        })?;
    String::from_utf8(bytes).map_err(|_| METADATA_ERROR)
}

pub(super) fn repository_directory(
    project: &Project,
    context: &OperationContext,
) -> Result<PathBuf, Failure> {
    let dotgit = project.target(".git").map_err(|_| METADATA_ERROR)?;
    if dotgit.is_dir() {
        return metadata_path(&dotgit);
    }
    let text = metadata_text(&project.root, ".git", context)?;
    let reference = text.strip_prefix("gitdir: ").ok_or(METADATA_ERROR)?;
    let directory = metadata_reference(&project.root, reference)?;
    if !directory.is_dir() {
        return Err(METADATA_ERROR);
    }
    let backlink = metadata_text(&directory, "gitdir", context)?;
    if metadata_reference(&directory, &backlink)? != metadata_path(&dotgit)? {
        return Err(METADATA_ERROR);
    }
    let common = metadata_text(&directory, "commondir", context)?;
    let common = metadata_reference(&directory, &common)?;
    if !common.is_dir()
        || directory.parent() != Some(metadata_path(&common.join("worktrees"))?.as_path())
    {
        return Err(METADATA_ERROR);
    }
    Ok(directory)
}

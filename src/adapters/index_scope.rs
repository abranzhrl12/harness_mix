//! Bounded CodeGraph include/exclude policy using the existing ignore crate.
use super::super::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const CONFIG: Failure = (
    "SYNC_SCOPE",
    "Unsupported or invalid codegraph.json; native scope was not applied",
);

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn configuration_changed_after_sql_writes_rolls_back_the_file() {
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        super::super::initialize(root.path(), &context).unwrap();
        std::fs::write(root.path().join("a.rs"), "pub fn sample() {}\n").unwrap();
        let failure = super::super::sync_file_with(root.path(), "a.rs", &context, || {
            std::fs::write(
                root.path().join("codegraph.json"),
                r#"{"exclude":["a.rs"]}"#,
            )
            .unwrap();
        })
        .unwrap_err();
        assert_eq!(failure.0, "STALE");
        let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

pub(crate) struct Snapshot {
    digest: Option<String>,
    includes: Gitignore,
    excludes: Gitignore,
    pub(crate) has_includes: bool,
}

fn bytes(project: &Project, context: &OperationContext) -> Result<Option<Vec<u8>>, Failure> {
    context.check()?;
    let path = project.target_optional("codegraph.json")?;
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CONFIG),
        Ok(_) => {
            let mut guarded = project.clone();
            guarded.external = true;
            guarded
                .bytes_with_limit("codegraph.json", context, 128 * 1024)
                .map(Some)
        }
    }
}

fn patterns(project: &Project, values: &[String]) -> Result<Gitignore, Failure> {
    if values.len() > 128 {
        return Err(CONFIG);
    }
    let mut builder = GitignoreBuilder::new(&project.root);
    for value in values {
        if value.is_empty()
            || value.len() > 1000
            || value.starts_with(['!', '#'])
            || value.contains(['\\', ':'])
            || value.chars().any(char::is_control)
            || value.split('/').any(|part| part == "..")
        {
            return Err(CONFIG);
        }
        builder.add_line(None, value).map_err(|_| CONFIG)?;
    }
    builder.build().map_err(|_| CONFIG)
}

impl Snapshot {
    pub(crate) fn load(project: &Project, context: &OperationContext) -> Result<Self, Failure> {
        let source = bytes(project, context)?;
        let config: Configuration = source
            .as_ref()
            .map(|b| serde_json::from_slice(b).map_err(|_| CONFIG))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            digest: source.as_ref().map(|b| hash(b)),
            includes: patterns(project, &config.include)?,
            excludes: patterns(project, &config.exclude)?,
            has_includes: !config.include.is_empty(),
        })
    }
    pub(crate) fn excluded(&self, path: &str) -> bool {
        self.excludes
            .matched_path_or_any_parents(path, false)
            .is_ignore()
    }
    pub(crate) fn included(&self, path: &str) -> bool {
        self.includes
            .matched_path_or_any_parents(path, false)
            .is_ignore()
    }
    pub(crate) fn stamp(&self) -> Value {
        json!({"hash":self.digest,"profile":"include-exclude-v1","providerConfigApplied":self.digest.is_some()})
    }
    pub(crate) fn recheck(
        &self,
        project: &Project,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        if bytes(project, context)?.as_ref().map(|b| hash(b)) != self.digest {
            return Err((
                "STALE",
                "codegraph.json changed during indexing; retry with a stable scope",
            ));
        }
        Ok(())
    }
}

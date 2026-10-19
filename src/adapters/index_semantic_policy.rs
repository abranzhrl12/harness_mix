//! Workspace policy for deep JS/TS analysis; basic extraction remains enabled.
use super::{Failure, OperationContext, Project, hash};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde_json::{Value, json};

const PATH: &str = ".agents/harness.config.json";
const INVALID: Failure = (
    "SYNC_CONFIG",
    "Invalid indexing.semanticExclude in .agents/harness.config.json; use at most 128 relative forward-slash globs",
);

#[derive(Clone)]
pub(super) struct Snapshot {
    digest: Option<String>,
    matcher: GlobSet,
    count: usize,
}

impl Snapshot {
    pub fn load(project: &Project, context: &OperationContext) -> Result<Self, Failure> {
        context.check()?;
        let target = project.target_optional(PATH)?;
        let policy = match std::fs::symlink_metadata(target) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(INVALID),
            Ok(_) => {
                let bytes = project.bytes_with_limit(PATH, context, 128 * 1024)?;
                let source = std::str::from_utf8(&bytes).map_err(|_| INVALID)?;
                let root: Value = serde_json::from_str(source.trim_start_matches('\u{feff}'))
                    .map_err(|_| INVALID)?;
                if !root.is_object() {
                    return Err(INVALID);
                }
                match root.get("indexing") {
                    None => None,
                    Some(indexing) if indexing.is_object() => {
                        indexing.get("semanticExclude").cloned()
                    }
                    _ => return Err(INVALID),
                }
            }
        };
        let values: Vec<String> = policy
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| INVALID))
            .transpose()?
            .unwrap_or_default();
        if values.len() > 128 {
            return Err(INVALID);
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in &values {
            if pattern.is_empty()
                || pattern.len() > 1000
                || pattern.starts_with(['/', '!', '#'])
                || pattern.contains(['\\', ':'])
                || pattern.chars().any(char::is_control)
                || pattern.split('/').any(|p| p == "..")
            {
                return Err(INVALID);
            }
            builder.add(
                GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .case_insensitive(cfg!(windows))
                    .build()
                    .map_err(|_| INVALID)?,
            );
        }
        Ok(Self {
            digest: policy.map(|v| hash(v.to_string().as_bytes())),
            matcher: builder.build().map_err(|_| INVALID)?,
            count: values.len(),
        })
    }

    pub fn excluded(&self, path: &str) -> bool {
        self.matcher.is_match(path.replace('\\', "/"))
    }

    pub fn stamp(&self) -> Option<Value> {
        self.digest.as_ref().map(|digest| json!({"hash":digest,"excludedPatterns":self.count,"configPath":PATH,"coverage":"deep JS/TS semantic analysis only; basic symbols retained"}))
    }

    pub fn recheck(&self, project: &Project, context: &OperationContext) -> Result<(), Failure> {
        if Self::load(project, context)?.digest != self.digest {
            return Err((
                "STALE",
                "Semantic exclusions changed during indexing; transaction rolled back",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_globs_and_configuration_freshness() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".agents")).unwrap();
        let file = dir.path().join(PATH);
        std::fs::write(&file, r#"{"sqlserver":{"enabled":false},"indexing":{"semanticExclude":["Content/plugins/**","Scripts/vendor/**"]}}"#).unwrap();
        let project = Project::new(dir.path()).unwrap();
        let context = OperationContext::default();
        let policy = Snapshot::load(&project, &context).unwrap();
        assert!(policy.excluded("Content/plugins/old/min.js"));
        assert!(policy.excluded("Scripts\\vendor\\old.js"));
        assert!(!policy.excluded("Scripts/app.js"));
        std::fs::write(&file, r#"{"indexing":{"semanticExclude":[]}}"#).unwrap();
        assert_eq!(policy.recheck(&project, &context).unwrap_err().0, "STALE");
        for invalid in [
            r#"["../outside/**"]"#,
            r#"["C:/vendor/**"]"#,
            r#"["/vendor/**"]"#,
            r#"["[broken"]"#,
            r#""wrong""#,
        ] {
            std::fs::write(
                &file,
                format!(r#"{{"indexing":{{"semanticExclude":{invalid}}}}}"#),
            )
            .unwrap();
            assert!(Snapshot::load(&project, &context).is_err());
        }
    }
}

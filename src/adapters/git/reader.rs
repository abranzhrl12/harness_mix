use super::metadata::{METADATA_ERROR, metadata_path, repository_directory};
use crate::adapters::{process::NativeProcessRunner, project::Project};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    execution::OperationContext,
    process::{ProcessSpec, Program, StopReason},
};
use std::{collections::BTreeSet, path::Path, time::Duration};

pub(super) struct Reader<'a> {
    runner: NativeProcessRunner,
    context: &'a OperationContext,
    base: Vec<String>,
}
impl<'a> Reader<'a> {
    pub(super) fn execute(&self, args: &[&str], absent: bool) -> Result<String, Failure> {
        self.context.check()?;
        let mut command = self.base.clone();
        command.extend(args.iter().map(|s| s.to_string()));
        let spec = ProcessSpec::new(
            Program::Git,
            command,
            Duration::from_secs(15),
            2 * 1024 * 1024,
        )?;
        let result = self.runner.run(&spec, self.context)?;
        match result.reason {
            StopReason::Cancelled => return Err(("CANCELLED", "Git query cancelled")),
            StopReason::TimedOut => return Err(("TIMEOUT", "Git query timed out")),
            StopReason::OutputLimit => return Err(("GIT_SIZE", "Git output exceeded 2 MiB")),
            StopReason::Exited => {}
        }
        // stderr/config values may contain credentials: never forward them.
        if result.exit_code != Some(0) && !(absent && result.exit_code == Some(1)) {
            return Err((
                "GIT_READ",
                "Git cannot read this repository; check permissions and configuration",
            ));
        }
        String::from_utf8(result.stdout).map_err(|_| ("GIT_ENCODING", "Git output must be UTF-8"))
    }
    pub(super) fn new(project: &Project, context: &'a OperationContext) -> Result<Self, Failure> {
        context.check()?;
        let directory = repository_directory(project, context)?;
        let mut reader = Self {
            runner: NativeProcessRunner::new(&project.root)?,
            context,
            base: [
                "--no-pager",
                "--no-optional-locks",
                "--literal-pathspecs",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
                "-c",
                "core.quotePath=false",
                "-c",
                "color.ui=false",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        };
        let root = reader.execute(&["rev-parse", "--show-toplevel"], false)?;
        let actual = Path::new(root.trim_end_matches(['\r', '\n']))
            .canonicalize()
            .map_err(|_| ("GIT_REPO", "Git root unavailable"))?;
        if actual != project.root {
            return Err((
                "GIT_REPO",
                "Git worktree differs from the authorized workspace",
            ));
        }
        let actual_directory = reader.execute(&["rev-parse", "--absolute-git-dir"], false)?;
        if metadata_path(Path::new(actual_directory.trim_end_matches(['\r', '\n'])))? != directory {
            return Err(METADATA_ERROR);
        }
        let names = reader.execute(
            &[
                "config",
                "--includes",
                "--null",
                "--name-only",
                "--get-regexp",
                r"^filter\..*\.(clean|process|required)$",
            ],
            true,
        )?;
        let mut filters = BTreeSet::new();
        for key in names.split('\0').filter(|s| !s.is_empty()) {
            if key.len() > 240 || key.contains('=') || key.chars().any(char::is_control) {
                return Err(("GIT_FILTERS", "Unsupported filter name; query refused"));
            }
            let (name, suffix) = key
                .rsplit_once('.')
                .ok_or(("GIT_FILTERS", "Invalid filter key"))?;
            if !key.to_ascii_lowercase().starts_with("filter.")
                || !["clean", "process", "required"].contains(&suffix.to_ascii_lowercase().as_str())
            {
                return Err(("GIT_FILTERS", "Invalid filter key"));
            }
            filters.insert(name.to_owned());
            if filters.len() > 12 {
                return Err((
                    "GIT_FILTERS",
                    "Too many configured filters for a bounded safe query",
                ));
            }
        }
        for name in filters {
            for value in [
                format!("{name}.clean="),
                format!("{name}.process="),
                format!("{name}.required=false"),
            ] {
                reader.base.extend(["-c".into(), value]);
            }
        }
        Ok(reader)
    }
}

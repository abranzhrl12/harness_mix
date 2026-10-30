use std::{ffi::OsString, path::PathBuf};

/// Workspace selection requested by a parsed command.
pub enum Root {
    Explicit(PathBuf),
    Detect,
    DetectOrCurrent,
}

/// Parsed CLI request, independent of the host that executes it.
pub enum Command {
    Workspace {
        root: Root,
        operation: WorkspaceOperation,
    },
    Help,
    Version,
    #[cfg(feature = "antigravity")]
    Active {
        root: Root,
        provider: Option<String>,
    },
    #[cfg(feature = "antigravity")]
    Config,
    Serve {
        root: Root,
        workspace_config: Option<PathBuf>,
        persist_logs: bool,
    },
    InitConfig {
        root: Root,
    },
    InitIndex {
        root: Root,
    },
    IndexProject {
        root: Root,
    },
    SyncWorkspace {
        root: Root,
    },
    Rebuild {
        root: Root,
    },
    Plan {
        root: Root,
        summary: bool,
    },
    SyncFile {
        root: Root,
        relative: OsString,
        config: Option<OsString>,
        force: bool,
    },
    SkillsSync {
        root: Root,
        check: bool,
        dry_run: bool,
        enable_auto: bool,
        disable_auto: bool,
    },
    InitProject {
        root: Root,
    },
    UninstallProject {
        root: Root,
    },
}

pub enum WorkspaceOperation {
    Init,
    Add { name: String, path: PathBuf },
    List,
    Check,
}

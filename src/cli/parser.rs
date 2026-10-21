use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

pub(super) enum Root {
    Explicit(PathBuf),
    Detect,
    DetectOrCurrent,
}

pub(super) enum Command {
    Help,
    Version,
    Serve {
        root: Root,
        persist_logs: bool,
    },
    InitConfig {
        root: Root,
    },
    InitIndex {
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
    },
    SyncFile {
        root: Root,
        relative: OsString,
        config: Option<OsString>,
        force: bool,
    },
}

pub(super) fn parse(mut args: Vec<OsString>) -> Result<Command, String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::Help);
    }

    let auto_root = needs_workspace_root(&args);
    if auto_root {
        args.splice(0..0, [OsString::from("--root"), OsString::new()]);
    }

    let eq = |args: &[OsString], index: usize, expected: &str| {
        args.get(index).is_some_and(|value| value == expected)
    };
    let root_at = |args: &[OsString]| {
        if auto_root {
            Root::Detect
        } else {
            Root::Explicit(PathBuf::from(args[1].clone()))
        }
    };

    if args.len() == 3 && eq(&args, 0, "--root") && eq(&args, 2, "--init-config") {
        return Ok(Command::InitConfig {
            root: root_at(&args),
        });
    }
    if args.len() == 6
        && eq(&args, 0, "--root")
        && eq(&args, 2, "--sync-file")
        && eq(&args, 4, "--tsconfig")
    {
        return Ok(Command::SyncFile {
            root: root_at(&args),
            relative: args[3].clone(),
            config: Some(args[5].clone()),
            force: false,
        });
    }
    if args.len() > 5 {
        return Err("too many command-line arguments".into());
    }
    let arg = |index: usize| args.get(index).map(OsString::as_os_str);
    let root = || root_at(&args);
    let is = |index: usize, expected: &str| arg(index) == Some(OsStr::new(expected));

    if arg(0) == Some(OsStr::new("--root")) {
        if args.len() == 3 && is(2, "--reindex")
            || args.len() == 4 && is(2, "--reindex")
            || args.len() == 4 && is(2, "--index") && is(3, "rebuild")
        {
            return Ok(Command::Rebuild { root: root() });
        }
        if args.len() == 3 && is(2, "--init-index")
            || args.len() == 4 && is(2, "--init-index")
            || args.len() == 4 && is(2, "--index") && is(3, "init")
        {
            return Ok(Command::InitIndex { root: root() });
        }
        if args.len() == 3 && is(2, "--sync")
            || args.len() == 4 && is(2, "--sync")
            || args.len() == 4 && is(2, "--index") && is(3, "sync")
        {
            return Ok(Command::SyncWorkspace { root: root() });
        }
        if args.len() == 4 && is(2, "--sync-file") && arg(3).is_some() {
            return Ok(Command::SyncFile {
                root: root(),
                relative: args[3].clone(),
                config: None,
                force: false,
            });
        }
        if args.len() == 5 && is(2, "--sync-file") && is(4, "--force") {
            return Ok(Command::SyncFile {
                root: root(),
                relative: args[3].clone(),
                config: None,
                force: true,
            });
        }
        if args.len() == 3 && is(2, "--sync-plan")
            || args.len() == 4 && is(2, "--sync-plan")
            || args.len() == 4 && is(2, "--index") && is(3, "plan")
        {
            return Ok(Command::Plan { root: root() });
        }
        if args.len() == 3 && is(2, "--persist-logs") {
            return Ok(Command::Serve {
                root: root(),
                persist_logs: true,
            });
        }
        if args.len() == 2 {
            return Ok(Command::Serve {
                root: root(),
                persist_logs: false,
            });
        }
    } else if args.is_empty() {
        return Ok(Command::Serve {
            root: Root::DetectOrCurrent,
            persist_logs: false,
        });
    } else if args.len() == 1 && eq(&args, 0, "--version") {
        return Ok(Command::Version);
    }

    Err("usage: agent-harness [--root <workspace>] [--index <init|sync|rebuild|plan> | --persist-logs | --sync-file <relative> [--tsconfig <relative.json>]] | --version".into())
}

pub(super) fn needs_workspace_root(args: &[OsString]) -> bool {
    args.first().is_some_and(|flag| {
        [
            "--index",
            "--init-index",
            "--init-config",
            "--sync",
            "--reindex",
            "--sync-plan",
            "--sync-file",
            "--persist-logs",
        ]
        .iter()
        .any(|option| flag == *option)
    })
}

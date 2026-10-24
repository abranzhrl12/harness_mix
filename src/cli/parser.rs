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
    Active {
        root: Root,
        provider: Option<String>,
    },
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
        root: PathBuf,
    },
}

pub(super) fn parse(mut args: Vec<OsString>) -> Result<Command, String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return Ok(Command::Help);
    }

    if args
        .first()
        .is_some_and(|arg| arg == "config" || arg == "/config")
    {
        return if args.len() == 1 {
            Ok(Command::Config)
        } else {
            Err("usage: agent-harness config".into())
        };
    }
    if args.first().is_some_and(|arg| arg == "active") {
        return parse_active_command(&args[1..], None);
    }
    if args.len() >= 3 && args[0] == "--root" && args[2] == "active" {
        return parse_active_command(&args[3..], Some(PathBuf::from(args[1].clone())));
    }

    if args.first().is_some_and(|arg| arg == "--workspace-config") {
        if (args.len() == 2) || (args.len() == 3 && args[2] == "--persist-logs") {
            return Ok(Command::Serve {
                root: Root::DetectOrCurrent,
                workspace_config: Some(PathBuf::from(args[1].clone())),
                persist_logs: args.len() == 3,
            });
        }
        return Err(
            "usage: agent-harness --workspace-config <manifest.json> [--persist-logs]".into(),
        );
    }

    if args.first().is_some_and(|arg| arg == "skills") {
        return parse_skills_command(&args[1..], Root::Detect);
    }

    if args.first().is_some_and(|arg| arg == "init") {
        return parse_init_command(&args[1..], None);
    }
    if args.len() >= 3 && args[0] == "--root" && args[2] == "init" {
        return parse_init_command(&args[3..], Some(PathBuf::from(args[1].clone())));
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
    let arg = |index: usize| args.get(index).map(OsString::as_os_str);
    let root = || root_at(&args);
    let is = |index: usize, expected: &str| arg(index) == Some(OsStr::new(expected));

    if args.iter().any(|arg| arg == "--summary") {
        let plan_alias =
            args.len() == 4 && is(0, "--root") && is(2, "--sync-plan") && is(3, "--summary");
        let plan_action = args.len() == 5
            && is(0, "--root")
            && is(2, "--index")
            && is(3, "plan")
            && is(4, "--summary");
        if plan_alias || plan_action {
            return Ok(Command::Plan {
                root: root(),
                summary: true,
            });
        }
        return Err("--summary is supported only with --index plan or --sync-plan".into());
    }

    if args.len() >= 4 && is(0, "--root") && is(2, "skills") && is(3, "sync") {
        return parse_skills_command(&args[3..], root());
    }
    if args.len() > 5 {
        return Err("too many command-line arguments".into());
    }

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
            return Ok(Command::Plan {
                root: root(),
                summary: false,
            });
        }
        if args.len() == 3 && is(2, "--persist-logs") {
            return Ok(Command::Serve {
                root: root(),
                workspace_config: None,
                persist_logs: true,
            });
        }
        if args.len() == 2 {
            return Ok(Command::Serve {
                root: root(),
                workspace_config: None,
                persist_logs: false,
            });
        }
    } else if args.is_empty() {
        return Ok(Command::Serve {
            root: Root::DetectOrCurrent,
            workspace_config: None,
            persist_logs: false,
        });
    } else if args.len() == 1 && eq(&args, 0, "--version") {
        return Ok(Command::Version);
    }

    Err("usage: agent-harness [--root <workspace>] [--workspace-config <manifest.json>] [--index <init|sync|rebuild|plan> | --persist-logs | --sync-file <relative> [--tsconfig <relative.json>]] | --version".into())
}

fn parse_active_command(args: &[OsString], mut root: Option<PathBuf>) -> Result<Command, String> {
    let usage = "usage: agent-harness active [antigravity] [--root <workspace>]";
    let mut provider = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--root" && index + 1 < args.len() && root.is_none() {
            root = Some(PathBuf::from(&args[index + 1]));
            index += 2;
        } else if args[index] == "antigravity" && provider.is_none() {
            provider = Some("antigravity".to_owned());
            index += 1;
        } else {
            return Err(usage.into());
        }
    }
    if root
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err("active requires a non-empty --root path".into());
    }
    Ok(Command::Active {
        root: root.map_or(Root::DetectOrCurrent, Root::Explicit),
        provider,
    })
}

fn parse_skills_command(args: &[OsString], root: Root) -> Result<Command, String> {
    if args.first().is_none_or(|arg| arg != "sync") {
        return Err("usage: agent-harness [--root <workspace>] skills sync [--check | --dry-run | --enable-auto | --disable-auto]".into());
    }
    let mut check = false;
    let mut dry_run = false;
    let mut enable_auto = false;
    let mut disable_auto = false;
    let mut resolved_root = root;
    let mut root_option_seen = matches!(&resolved_root, Root::Explicit(_));
    let mut index = 1;
    while index < args.len() {
        match args[index].to_str() {
            Some("--check") => check = true,
            Some("--dry-run") => dry_run = true,
            Some("--enable-auto") => enable_auto = true,
            Some("--disable-auto") => disable_auto = true,
            Some("--root") if index + 1 < args.len() => {
                if root_option_seen {
                    return Err("skills sync accepts --root only once".into());
                }
                resolved_root = Root::Explicit(PathBuf::from(args[index + 1].clone()));
                root_option_seen = true;
                index += 1;
            }
            _ => return Err("unknown or incomplete skills sync option".into()),
        }
        index += 1;
    }
    let selected_modes = [check, dry_run, enable_auto, disable_auto]
        .into_iter()
        .filter(|selected| *selected)
        .count();
    if selected_modes > 1 {
        return Err("skills sync modes are mutually exclusive".into());
    }
    Ok(Command::SkillsSync {
        root: resolved_root,
        check,
        dry_run,
        enable_auto,
        disable_auto,
    })
}

fn parse_init_command(args: &[OsString], root: Option<PathBuf>) -> Result<Command, String> {
    let mut resolved_root = root;
    let mut index = 0;
    while index < args.len() {
        if args[index] != "--root" || index + 1 >= args.len() || resolved_root.is_some() {
            return Err("usage: agent-harness init --root <workspace>".into());
        }
        let path = PathBuf::from(args[index + 1].clone());
        if path.as_os_str().is_empty() {
            return Err("init requires a non-empty --root path".into());
        }
        resolved_root = Some(path);
        index += 2;
    }
    let root = resolved_root.ok_or("usage: agent-harness init --root <workspace>")?;
    if root.as_os_str().is_empty() {
        return Err("init requires a non-empty --root path".into());
    }
    Ok(Command::InitProject { root })
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
            "skills",
        ]
        .iter()
        .any(|option| flag == *option)
    })
}

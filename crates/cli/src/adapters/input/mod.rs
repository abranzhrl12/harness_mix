use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::{ArgumentParser, Command, Root};

mod subcommands;

pub struct Parser;

impl ArgumentParser for Parser {
    fn needs_workspace_root(&self, args: &[OsString]) -> bool {
        needs_workspace_root(args)
    }

    fn parse(&self, args: Vec<OsString>) -> Result<Command, String> {
        parse(args)
    }
}
use subcommands::{
    parse_active_command, parse_init_command, parse_skills_command, parse_uninstall_command,
};

pub fn parse(mut args: Vec<OsString>) -> Result<Command, String> {
    if args.first().is_some_and(|arg| arg == "workspace") {
        return subcommands::parse_workspace_command(&args[1..]);
    }
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
    if args.first().is_some_and(|arg| arg == "uninstall") {
        return parse_uninstall_command(&args[1..], None);
    }
    if args.len() >= 3 && args[0] == "--root" && args[2] == "uninstall" {
        return parse_uninstall_command(&args[3..], Some(PathBuf::from(args[1].clone())));
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

pub fn needs_workspace_root(args: &[OsString]) -> bool {
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

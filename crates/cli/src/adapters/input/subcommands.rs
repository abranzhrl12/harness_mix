use std::{ffi::OsString, path::PathBuf};

use crate::{Command, Root};

pub(super) fn parse_active_command(
    args: &[OsString],
    mut root: Option<PathBuf>,
) -> Result<Command, String> {
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

pub(super) fn parse_skills_command(args: &[OsString], root: Root) -> Result<Command, String> {
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

pub(super) fn parse_init_command(
    args: &[OsString],
    root: Option<PathBuf>,
) -> Result<Command, String> {
    let mut resolved_root = root;
    let mut index = 0;
    while index < args.len() {
        if args[index] != "--root" || index + 1 >= args.len() || resolved_root.is_some() {
            return Err("usage: agent-harness init [--root <workspace>]".into());
        }
        let path = PathBuf::from(args[index + 1].clone());
        if path.as_os_str().is_empty() {
            return Err("init requires a non-empty --root path".into());
        }
        resolved_root = Some(path);
        index += 2;
    }
    if resolved_root
        .as_ref()
        .is_some_and(|root| root.as_os_str().is_empty())
    {
        return Err("init requires a non-empty --root path".into());
    }
    Ok(Command::InitProject {
        root: resolved_root.map_or(Root::DetectOrCurrent, Root::Explicit),
    })
}

pub(super) fn parse_uninstall_command(
    args: &[OsString],
    root: Option<PathBuf>,
) -> Result<Command, String> {
    match parse_init_command(args, root) {
        Ok(Command::InitProject { root }) => Ok(Command::UninstallProject { root }),
        Err(error) => Err(error.replace("init", "uninstall")),
        _ => unreachable!(),
    }
}

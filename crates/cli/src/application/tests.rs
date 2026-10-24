use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::{ArgumentParser, CliApplication, WorkspaceLocator};
use crate::{Command, Root};

#[derive(Clone)]
struct FakeParser {
    needs_root: bool,
    fail_parse: bool,
    events: Arc<Mutex<Vec<String>>>,
}

impl ArgumentParser for FakeParser {
    fn needs_workspace_root(&self, _args: &[OsString]) -> bool {
        self.events.lock().unwrap().push("needs-root".into());
        self.needs_root
    }

    fn parse(&self, args: Vec<OsString>) -> Result<Command, String> {
        self.events.lock().unwrap().push(format!("parse:{args:?}"));
        if self.fail_parse {
            return Err("parser failure".into());
        }
        if args.first().is_some_and(|arg| arg == "--version") {
            Ok(Command::Version)
        } else {
            Ok(Command::Help)
        }
    }
}

#[derive(Clone)]
struct FakeWorkspace {
    detect_result: Result<PathBuf, &'static str>,
    current_result: Result<PathBuf, &'static str>,
    events: Arc<Mutex<Vec<String>>>,
}

impl WorkspaceLocator for FakeWorkspace {
    fn detect_workspace(&self) -> Result<PathBuf, String> {
        self.events.lock().unwrap().push("detect".into());
        self.detect_result.clone().map_err(str::to_owned)
    }

    fn current_dir(&self) -> Result<PathBuf, String> {
        self.events.lock().unwrap().push("current-dir".into());
        self.current_result.clone().map_err(str::to_owned)
    }
}

fn events() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

#[test]
fn help_and_version_do_not_touch_workspace() {
    for (arg, expected) in [("--help", "help"), ("--version", "version")] {
        let events = events();
        let app = CliApplication::new(
            FakeParser {
                needs_root: false,
                fail_parse: false,
                events: events.clone(),
            },
            FakeWorkspace {
                detect_result: Err("must not detect".into()),
                current_result: Err("must not read cwd".into()),
                events: events.clone(),
            },
        );

        let command = app.prepare([OsString::from(arg)]).unwrap();
        assert!(matches!(
            (expected, command),
            ("help", Command::Help) | ("version", Command::Version)
        ));
        assert_eq!(
            *events.lock().unwrap(),
            vec!["needs-root".to_owned(), format!("parse:[\"{arg}\"]")]
        );
    }
}

#[test]
fn workspace_failure_precedes_parser_failure() {
    let events = events();
    let app = CliApplication::new(
        FakeParser {
            needs_root: true,
            fail_parse: true,
            events: events.clone(),
        },
        FakeWorkspace {
            detect_result: Err("workspace failure".into()),
            current_result: Err("unused".into()),
            events: events.clone(),
        },
    );

    let error = app.prepare([OsString::from("--index")]).err().unwrap();
    assert_eq!(error, "workspace failure");
    assert_eq!(
        *events.lock().unwrap(),
        vec!["needs-root".to_owned(), "detect".to_owned()]
    );
}

#[test]
fn parser_error_is_propagated_after_workspace_detection() {
    let events = events();
    let app = CliApplication::new(
        FakeParser {
            needs_root: true,
            fail_parse: true,
            events: events.clone(),
        },
        FakeWorkspace {
            detect_result: Ok(PathBuf::from("workspace")),
            current_result: Err("unused".into()),
            events: events.clone(),
        },
    );

    let error = app.prepare([OsString::from("--index")]).err().unwrap();
    assert_eq!(error, "parser failure");
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "needs-root".to_owned(),
            "detect".to_owned(),
            "parse:[\"--root\", \"workspace\", \"--index\"]".to_owned()
        ]
    );
}

#[test]
fn skills_command_keeps_its_existing_detection_exception() {
    let events = events();
    let app = CliApplication::new(
        FakeParser {
            needs_root: true,
            fail_parse: false,
            events: events.clone(),
        },
        FakeWorkspace {
            detect_result: Err("must not detect".into()),
            current_result: Err("unused".into()),
            events: events.clone(),
        },
    );

    assert!(matches!(
        app.prepare([OsString::from("skills")]).unwrap(),
        Command::Help
    ));
    assert_eq!(
        *events.lock().unwrap(),
        vec!["needs-root".to_owned(), "parse:[\"skills\"]".to_owned()]
    );
}

#[test]
fn explicit_root_never_uses_workspace_port() {
    let events = events();
    let app = CliApplication::new(
        FakeParser {
            needs_root: false,
            fail_parse: false,
            events: events.clone(),
        },
        FakeWorkspace {
            detect_result: Err("unused".into()),
            current_result: Err("unused".into()),
            events: events.clone(),
        },
    );

    assert_eq!(
        app.resolve_root(Root::Explicit(PathBuf::from("given")))
            .unwrap(),
        PathBuf::from("given")
    );
    assert!(events.lock().unwrap().is_empty());
}

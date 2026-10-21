use super::*;
use harness_domain::{
    execution::OperationContext,
    process::{ProcessOutcome, ProcessSpec, Program, StopReason},
};
use std::{
    io::Write,
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[test]
fn finds_web_targets_from_both_msbuild_architectures() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("MSBuild");
    let targets = base.join("Microsoft/VisualStudio/v17.0/WebApplications");
    let executable = base.join("Current/Bin/MSBuild.exe");
    assert!(!has_visual_studio_web_targets(&executable));
    std::fs::create_dir_all(&targets).unwrap();
    std::fs::write(
        targets.join("Microsoft.WebApplication.targets"),
        "<Project/>",
    )
    .unwrap();
    assert!(has_visual_studio_web_targets(&executable));
    assert!(has_visual_studio_web_targets(
        &base.join("Current/Bin/amd64/MSBuild.exe")
    ));
    assert!(!has_visual_studio_web_targets(
        &root.path().join("Other/MSBuild.exe")
    ));
}

#[test]
fn dotnet_policy_disables_startup_hooks_profilers_and_node_reuse() {
    let mut command = Command::new("unused-fixture");
    command
        .env("DOTNET_STARTUP_HOOKS", "private-hook")
        .env("MSBUILD_EXE_PATH", "private-exe")
        .env("MSBuildSDKsPath", "private-sdk");
    dotnet_environment(&mut command);
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    for key in [
        "DOTNET_STARTUP_HOOKS",
        "MSBUILD_EXE_PATH",
        "MSBuildSDKsPath",
    ] {
        assert_eq!(env[std::ffi::OsStr::new(key)], None);
    }
    for (key, value) in [
        ("DOTNET_CLI_TELEMETRY_OPTOUT", "1"),
        ("MSBUILDDISABLENODEREUSE", "1"),
        ("CORECLR_ENABLE_PROFILING", "0"),
        ("COR_ENABLE_PROFILING", "0"),
    ] {
        assert_eq!(
            env[std::ffi::OsStr::new(key)],
            Some(std::ffi::OsStr::new(value))
        );
    }
}

#[test]
fn node_policy_clears_preloads_and_inherited_module_paths() {
    let mut command = Command::new("unused-fixture");
    command
        .env("NODE_OPTIONS", "--require=private-hook")
        .env("NODE_PATH", "private-path")
        .env("NODE_V8_COVERAGE", "private-output");
    node_environment(&mut command);
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert_eq!(
        env[std::ffi::OsStr::new("NODE_OPTIONS")],
        Some(std::ffi::OsStr::new(""))
    );
    assert_eq!(
        env[std::ffi::OsStr::new("NODE_PATH")],
        Some(std::ffi::OsStr::new(""))
    );
    assert_eq!(env[std::ffi::OsStr::new("NODE_V8_COVERAGE")], None);
}

#[test]
fn go_policy_overrides_injected_flags_and_disables_downloads() {
    let mut command = Command::new("unused-fixture");
    command
        .env("GOFLAGS", "-toolexec=private-command")
        .env("GOCACHEPROG", "private-command")
        .env("GOOS", "foreign");
    go_environment(&mut command);
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    for (key, value) in [
        ("GOENV", "off"),
        ("GOFLAGS", ""),
        ("GOCACHEPROG", ""),
        ("GOWORK", "off"),
        ("GOTOOLCHAIN", "local"),
        ("GOPROXY", "off"),
        ("GOSUMDB", "off"),
        ("GONOPROXY", "none"),
        ("GOVCS", "*:off"),
        ("CGO_ENABLED", "0"),
    ] {
        assert_eq!(
            env[std::ffi::OsStr::new(key)],
            Some(std::ffi::OsStr::new(value))
        );
    }
    assert_eq!(env[std::ffi::OsStr::new("GOOS")], None);
    assert_eq!(env[std::ffi::OsStr::new("GOARCH")], None);
}

#[test]
#[allow(clippy::zombie_processes)] // The orphan fixture deliberately exits before wait.
fn fixture() {
    let Ok(mode) = std::env::var("HARNESS_RUST_PROCESS_FIXTURE") else {
        return;
    };
    match mode.as_str() {
        "sleep" => thread::sleep(Duration::from_secs(20)),
        "flood" => {
            for _ in 0..4096 {
                let _ = std::io::stdout().write_all(&[b'x'; 1024]);
                let _ = std::io::stderr().write_all(&[b'y'; 1024]);
            }
        }
        "tree" | "orphan" => {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "adapters::process::tests::fixture",
                    "--nocapture",
                ])
                .env("HARNESS_RUST_PROCESS_FIXTURE", "sleep")
                .spawn()
                .unwrap();
            println!("DESCENDANT={}", child.id());
            std::io::stdout().flush().unwrap();
            // Signal only after the descendant exists and its PID was flushed.
            // Each test supplies a private temporary path; no global env changes.
            if let Some(path) = std::env::var_os("HARNESS_RUST_PROCESS_READY") {
                std::fs::write(path, child.id().to_string()).unwrap();
            }
            if mode == "orphan" {
                std::process::exit(0);
            }
            let _ = child.wait();
        }
        _ => {
            print!("UTF8=🙂");
            eprint!("stderr-marker");
        }
    }
}

fn run(mode: &str, timeout: Duration, limit: usize, context: &OperationContext) -> ProcessOutcome {
    let spec = ProcessSpec::new(Program::Cargo, vec![], timeout, limit).unwrap();
    run_command(fixture_command(mode), &spec, context).unwrap()
}

fn fixture_command(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "adapters::process::tests::fixture",
            "--nocapture",
        ])
        .env("HARNESS_RUST_PROCESS_FIXTURE", mode);
    command.env_remove("HARNESS_RUST_PROCESS_READY");
    command
}

#[test]
fn captures_both_pipes_without_node() {
    let result = run(
        "normal",
        Duration::from_secs(5),
        4096,
        &OperationContext::default(),
    );
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.reason, StopReason::Exited);
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("UTF8=🙂")
    );
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("stderr-marker")
    );
}

#[test]
fn bounds_flood_and_times_out() {
    let result = run(
        "flood",
        Duration::from_secs(5),
        1024,
        &OperationContext::default(),
    );
    assert_eq!(result.reason, StopReason::OutputLimit);
    assert_eq!(result.stdout.len() + result.stderr.len(), 1024);
    let result = run(
        "sleep",
        Duration::from_millis(150),
        1024,
        &OperationContext::default(),
    );
    assert_eq!(result.reason, StopReason::TimedOut);
    assert!(result.elapsed < Duration::from_secs(5));
}

#[test]
fn background_output_arrives_before_exit_and_remains_cancellable() {
    let context = OperationContext::without_deadline();
    let spec = ProcessSpec::new(Program::Cargo, vec![], Duration::from_secs(1), 4096)
        .unwrap()
        .with_background_timeout(None)
        .unwrap();
    assert_eq!(spec.timeout_option(), None);
    let mut received_output = false;
    let result = run_command_observed(fixture_command("tree"), &spec, &context, &mut |_, bytes| {
        if String::from_utf8_lossy(bytes).contains("DESCENDANT=") {
            received_output = true;
            context.cancel();
        }
    })
    .unwrap();
    assert!(received_output);
    assert_eq!(result.reason, StopReason::Cancelled);
}

#[test]
fn cancellation_terminates_contained_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("descendant-ready");
    let mut command = fixture_command("tree");
    command.env("HARNESS_RUST_PROCESS_READY", &ready);
    let context = OperationContext::default();
    let cancel = context.clone();
    let watchdog = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.is_file() {
            if Instant::now() >= deadline {
                cancel.cancel();
                return false;
            }
            // Poll for a condition, never assume startup finished after a delay.
            thread::sleep(Duration::from_millis(5));
        }
        cancel.cancel();
        true
    });
    let spec = ProcessSpec::new(Program::Cargo, vec![], Duration::from_secs(15), 4096).unwrap();
    let result = run_command(command, &spec, &context);
    let was_ready = watchdog.join().unwrap();
    assert!(
        was_ready,
        "descendant did not signal readiness before watchdog"
    );
    let result = result.unwrap();
    assert_eq!(result.reason, StopReason::Cancelled);
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("DESCENDANT="));
    #[cfg(windows)]
    assert!(result.tree_termination_confirmed);
}

#[test]
fn normal_leader_exit_and_timeout_do_not_leave_descendants() {
    let result = run(
        "orphan",
        Duration::from_secs(5),
        4096,
        &OperationContext::default(),
    );
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.reason, StopReason::Exited);
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("DESCENDANT=")
    );
    #[cfg(windows)]
    assert!(result.tree_termination_confirmed);
    let result = run(
        "tree",
        Duration::from_secs(1),
        4096,
        &OperationContext::default(),
    );
    assert_eq!(result.reason, StopReason::TimedOut);
    // A timeout may expire before the fixture gets CPU time. Descendant
    // creation is proved separately by the readiness-gated cancellation test
    // and the normal-leader-exit case above, not by this wall-clock deadline.
    #[cfg(windows)]
    assert!(result.tree_termination_confirmed);
    assert!(result.elapsed < Duration::from_secs(5));
}

#[test]
fn cancelled_context_never_spawns_and_spawn_failure_is_reported() {
    let context = OperationContext::default();
    context.cancel();
    let spec = ProcessSpec::new(Program::Cargo, vec![], Duration::from_secs(1), 1).unwrap();
    assert_eq!(
        run_command(Command::new("missing-harness-fixture"), &spec, &context)
            .unwrap_err()
            .0,
        "CANCELLED"
    );
    assert_eq!(
        run_command(
            Command::new("missing-harness-fixture"),
            &spec,
            &OperationContext::default()
        )
        .unwrap_err()
        .0,
        "PROCESS_SPAWN"
    );
}

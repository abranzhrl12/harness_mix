//! Bounded native process execution. No shell, Node, taskkill or user-selected executable.
use super::process_tree::{self, Tree};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    execution::OperationContext,
    process::{ProcessOutcome, ProcessSpec, Program, StopReason},
    process_output::Output,
};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, SyncSender},
    thread,
    time::{Duration, Instant},
};

/// Root is fixed at construction, never taken from unvalidated request text.
pub struct NativeProcessRunner {
    root: PathBuf,
}

impl NativeProcessRunner {
    pub fn new(root: &Path) -> Result<Self, Failure> {
        let root = root
            .canonicalize()
            .map_err(|_| ("PATH", "Workspace unavailable"))?;
        if !root.is_dir() {
            return Err(("PATH", "Workspace must be a directory"));
        }
        Ok(Self { root })
    }
}

pub(super) fn executable(program: Program) -> Result<PathBuf, Failure> {
    let name = match program {
        Program::Cargo => "cargo",
        Program::Rustc => "rustc",
        Program::Git => "git",
        Program::Go => "go",
        Program::Python => "python",
    };
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    for directory in std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
    {
        // No current-directory or relative-PATH lookup, even on Windows.
        if !directory.is_absolute() {
            continue;
        }
        let path = directory.join(&filename);
        if !path.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !path
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            {
                continue;
            }
        }
        return path
            .canonicalize()
            .map_err(|_| ("PROCESS_EXECUTABLE", "Executable unavailable"));
    }
    Err((
        "PROCESS_EXECUTABLE",
        "Required executable not found in absolute PATH entries",
    ))
}

fn go_environment(command: &mut Command) {
    // These overrides apply to the child only; never mutate the harness environment.
    command
        .env("GOENV", "off")
        .env("GOFLAGS", "")
        .env("GOCACHEPROG", "")
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GO111MODULE", "on")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GONOPROXY", "none")
        .env("GOPRIVATE", "")
        .env("GOVCS", "*:off")
        .env("CGO_ENABLED", "0")
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env_remove("GOEXPERIMENT");
}

impl ProcessRunner for NativeProcessRunner {
    fn run(
        &self,
        spec: &ProcessSpec,
        context: &OperationContext,
    ) -> Result<ProcessOutcome, Failure> {
        context.check()?;
        let mut command = Command::new(executable(spec.program())?);
        command.args(spec.arguments()).current_dir(&self.root);
        if spec.program() == Program::Go {
            // Closed local-toolchain policy, not a sandbox for project/test code.
            go_environment(&mut command);
        }
        if spec.program() == Program::Git {
            // Never inherit alternate repositories, injected -c values, or askpass.
            for (key, _) in std::env::vars_os() {
                if key
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("GIT_")
                {
                    command.env_remove(key);
                }
            }
            command
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_NO_LAZY_FETCH", "1")
                .env("LC_ALL", "C");
        }
        run_command(command, spec, context)
    }
}

struct ChildGuard {
    child: std::process::Child,
    tree: Option<Tree>,
    reaped: bool,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        if let Some(tree) = &self.tree {
            let _ = tree.stop();
        }
        if self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

enum Packet {
    Bytes(bool, Vec<u8>),
    End,
    Error,
}

fn reader(
    mut pipe: impl Read + Send + 'static,
    stderr: bool,
    sender: SyncSender<Packet>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("harness-process-pipe".into())
        .spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(Packet::End);
                        return;
                    }
                    Ok(size) => {
                        if sender
                            .send(Packet::Bytes(stderr, buffer[..size].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = sender.send(Packet::Error);
                        return;
                    }
                }
            }
        })
}

fn run_command(
    mut command: Command,
    spec: &ProcessSpec,
    context: &OperationContext,
) -> Result<ProcessOutcome, Failure> {
    context.check()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process_tree::configure(&mut command);
    let started = Instant::now();
    let child = command
        .spawn()
        .map_err(|_| ("PROCESS_SPAWN", "Could not start contained process"))?;
    let mut guard = ChildGuard {
        child,
        tree: None,
        reaped: false,
    };
    guard.tree = Some(Tree::attach(&guard.child).map_err(|_| {
        (
            "PROCESS_CONTAINMENT",
            "Cannot contain child before execution",
        )
    })?);
    let tree = guard.tree.as_ref().unwrap();
    let child = &mut guard.child;
    let stdout = child
        .stdout
        .take()
        .ok_or(("PROCESS_PIPE", "Missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or(("PROCESS_PIPE", "Missing stderr pipe"))?;
    let (sender, receiver) = mpsc::sync_channel(8);
    let out = reader(stdout, false, sender.clone())
        .map_err(|_| ("PROCESS_PIPE", "Cannot create stdout reader"))?;
    let err = reader(stderr, true, sender)
        .map_err(|_| ("PROCESS_PIPE", "Cannot create stderr reader"))?;
    let mut output = Output::new(spec);
    let mut ended = 0;
    let mut reason = StopReason::Exited;
    let mut stopped = false;
    let mut status = None;
    let mut cleanup_deadline = None;
    loop {
        if stopped {
            let deadline =
                cleanup_deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(5));
            if Instant::now() >= *deadline {
                return Err((
                    "PROCESS_CLEANUP",
                    "Process tree or output pipes did not close after termination",
                ));
            }
        }
        if !stopped {
            reason = match context.check() {
                Err(("CANCELLED", _)) => StopReason::Cancelled,
                Err(_) => StopReason::TimedOut,
                Ok(()) if started.elapsed() >= spec.timeout() => StopReason::TimedOut,
                Ok(()) => reason,
            };
            if reason != StopReason::Exited {
                tree.stop()
                    .map_err(|_| ("PROCESS_TERMINATION", "Could not terminate process tree"))?;
                stopped = true;
            }
        }
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|_| ("PROCESS_WAIT", "Could not inspect child"))?;
            // A tool must not leave background descendants after its leader exits.
            if status.is_some() && !stopped {
                tree.stop()
                    .map_err(|_| ("PROCESS_TERMINATION", "Could not terminate descendants"))?;
                stopped = true;
            }
        }
        if status.is_some()
            && ended == 2
            && tree
                .empty()
                .map_err(|_| ("PROCESS_WAIT", "Could not inspect process tree"))?
        {
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok(Packet::Bytes(stderr, bytes)) => {
                if !output.push(stderr, &bytes) && reason == StopReason::Exited {
                    reason = StopReason::OutputLimit;
                    tree.stop()
                        .map_err(|_| ("PROCESS_TERMINATION", "Could not stop excessive output"))?;
                    stopped = true;
                }
            }
            Ok(Packet::End) => ended += 1,
            Ok(Packet::Error) => return Err(("PROCESS_PIPE", "Error reading process output")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) if ended == 2 => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err(("PROCESS_PIPE", "Process readers disconnected")),
        }
    }
    let exit = child
        .wait()
        .map_err(|_| ("PROCESS_WAIT", "Could not reap process tree"))?;
    guard.reaped = true;
    out.join()
        .map_err(|_| ("PROCESS_PIPE", "Stdout reader failed"))?;
    err.join()
        .map_err(|_| ("PROCESS_PIPE", "Stderr reader failed"))?;
    let (stdout, stderr, _) = output.finish();
    Ok(ProcessOutcome {
        exit_code: exit.code(),
        stdout,
        stderr,
        elapsed: started.elapsed(),
        reason,
        tree_termination_confirmed: cfg!(windows),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn go_policy_overrides_injected_flags_and_disables_downloads() {
        let mut command = std::process::Command::new("unused-fixture");
        command
            .env("GOFLAGS", "-toolexec=private-command")
            .env("GOCACHEPROG", "private-command")
            .env("GOOS", "foreign");
        super::go_environment(&mut command);
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
    use super::*;
    use std::io::Write;

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

    fn run(
        mode: &str,
        timeout: Duration,
        limit: usize,
        context: &OperationContext,
    ) -> ProcessOutcome {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "adapters::process::tests::fixture",
                "--nocapture",
            ])
            .env("HARNESS_RUST_PROCESS_FIXTURE", mode);
        let spec = ProcessSpec::new(Program::Cargo, vec![], timeout, limit).unwrap();
        run_command(command, &spec, context).unwrap()
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
    fn cancellation_terminates_contained_descendants() {
        let context = OperationContext::default();
        let cancel = context.clone();
        let thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            cancel.cancel();
        });
        let result = run("tree", Duration::from_secs(5), 4096, &context);
        thread.join().unwrap();
        assert_eq!(result.reason, StopReason::Cancelled);
        assert!(result.elapsed < Duration::from_secs(5));
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
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .contains("DESCENDANT=")
        );
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
}

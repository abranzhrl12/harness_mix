use super::super::process_tree::{self, Tree};
use harness_domain::{
    Failure,
    execution::OperationContext,
    process::{ProcessOutcome, ProcessSpec, StopReason},
    process_output::Output,
};
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, SyncSender},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard {
    child: Child,
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

#[cfg(test)]
pub(super) fn run_command(
    command: Command,
    spec: &ProcessSpec,
    context: &OperationContext,
) -> Result<ProcessOutcome, Failure> {
    run_command_observed(command, spec, context, &mut |_, _| {})
}

pub(super) fn run_command_observed(
    mut command: Command,
    spec: &ProcessSpec,
    context: &OperationContext,
    observer: &mut dyn FnMut(bool, &[u8]),
) -> Result<ProcessOutcome, Failure> {
    context.check()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process_tree::configure(&mut command);
    let started = Instant::now();
    let child = command.spawn().map_err(|error| {
        if matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) {
            (
                "PROCESS_SPAWN_TRANSIENT",
                "Temporary failure before process launch",
            )
        } else {
            ("PROCESS_SPAWN", "Could not start contained process")
        }
    })?;
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
    let mut observed_bytes = 0usize;
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
                Ok(())
                    if spec
                        .timeout_option()
                        .is_some_and(|timeout| started.elapsed() >= timeout) =>
                {
                    StopReason::TimedOut
                }
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
                let retained = bytes
                    .len()
                    .min(spec.output_limit().saturating_sub(observed_bytes));
                if retained > 0 {
                    observer(stderr, &bytes[..retained]);
                    observed_bytes += retained;
                }
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

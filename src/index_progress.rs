//! Human-readable progress for index maintenance CLI commands.
//! Progress is written only to stderr so the final machine-readable JSON on
//! stdout remains usable in scripts.
use harness_domain::execution::ProgressUpdate;
use std::{
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

const TTY_INTERVAL: Duration = Duration::from_millis(250);
const PIPE_INTERVAL: Duration = Duration::from_secs(1);
const MIN_ETA_SAMPLES: usize = 3;
const MIN_ETA_ELAPSED: Duration = Duration::from_secs(1);
const MAX_CURRENT_CHARS: usize = 120;

#[derive(Clone)]
pub(crate) struct IndexProgressReporter {
    inner: Arc<Inner>,
}

struct Inner {
    enabled: bool,
    tty: bool,
    started: Instant,
    state: Mutex<State>,
}

struct State {
    last_emit: Option<Instant>,
    phase: Option<&'static str>,
    phase_started: Instant,
    phase_start_completed: usize,
    finished: bool,
    last_line_len: usize,
}

#[derive(Clone)]
struct Snapshot {
    phase: &'static str,
    completed: usize,
    total: Option<usize>,
    current: Option<String>,
}

impl IndexProgressReporter {
    /// Detect stderr interactivity once. The reporter can be cheaply cloned
    /// into the OperationContext callback and serializes concurrent updates.
    pub(crate) fn new() -> Self {
        Self::with_terminal(io::stderr().is_terminal())
    }

    fn with_terminal(tty: bool) -> Self {
        let now = Instant::now();
        Self {
            inner: Arc::new(Inner {
                enabled: true,
                tty,
                started: now,
                state: Mutex::new(State {
                    last_emit: None,
                    phase: None,
                    phase_started: now,
                    phase_start_completed: 0,
                    finished: false,
                    last_line_len: 0,
                }),
            }),
        }
    }

    pub(crate) fn report(&self, update: ProgressUpdate<'_>) {
        if !self.inner.enabled {
            return;
        }
        let now = Instant::now();
        let mut state = lock_recover(&self.inner.state);
        if state.finished {
            return;
        }

        let changed_phase = state.phase != Some(update.phase);
        if changed_phase {
            state.phase = Some(update.phase);
            state.phase_started = now;
            state.phase_start_completed = update.completed;
        }
        let interval = if self.inner.tty {
            TTY_INTERVAL
        } else {
            PIPE_INTERVAL
        };
        if !changed_phase
            && state
                .last_emit
                .is_some_and(|last| now.duration_since(last) < interval)
        {
            return;
        }
        // Avoid allocating/sanitizing each current path when a throttled update
        // will not be rendered.
        let snapshot = Snapshot {
            phase: update.phase,
            completed: update.completed,
            total: update.total,
            current: update.current.map(sanitize_current),
        };
        let elapsed = now.duration_since(self.inner.started);
        let phase_elapsed = now.duration_since(state.phase_started);
        let eta = estimate_remaining(
            snapshot
                .completed
                .saturating_sub(state.phase_start_completed),
            phase_elapsed,
            snapshot
                .total
                .map(|total| total.saturating_sub(snapshot.completed)),
        );
        let line = format_progress(&snapshot, elapsed, eta, self.inner.tty);
        emit_line(&mut state, &line, self.inner.tty);
        state.last_emit = Some(now);
    }

    /// Print a terminal outcome only after the caller receives the completed
    /// operation result (which includes its commit/partial status).
    pub(crate) fn finish(&self, outcome: &str) {
        if !self.inner.enabled {
            return;
        }
        let mut state = lock_recover(&self.inner.state);
        if state.finished {
            return;
        }
        let elapsed = self.inner.started.elapsed();
        let line = format!(
            "Index maintenance {outcome} after {}",
            format_duration(elapsed)
        );
        emit_line(&mut state, &line, self.inner.tty);
        if self.inner.tty {
            let _ = writeln!(io::stderr().lock());
        }
        state.finished = true;
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn emit_line(state: &mut State, line: &str, tty: bool) {
    let mut stderr = io::stderr().lock();
    if tty {
        let padding = state.last_line_len.saturating_sub(line.chars().count());
        let _ = write!(stderr, "\r{line}{}", " ".repeat(padding));
        state.last_line_len = line.chars().count();
    } else {
        let _ = writeln!(stderr, "{line}");
    }
    let _ = stderr.flush();
}

fn format_progress(
    snapshot: &Snapshot,
    elapsed: Duration,
    eta: Option<Duration>,
    tty: bool,
) -> String {
    let mut line = match snapshot.total {
        Some(total) if total > 0 => {
            let completed = snapshot.completed.min(total);
            let percent = (completed as u128 * 100 / total as u128) as usize;
            if tty {
                const WIDTH: usize = 20;
                let filled = percent * WIDTH / 100;
                let bar = format!("{}{}", "#".repeat(filled), ".".repeat(WIDTH - filled));
                format!(
                    "Index {}/{} [{bar}] {:>3}% [{}]",
                    completed, total, percent, snapshot.phase
                )
            } else {
                format!(
                    "Index {}/{} ({}%) [{}]",
                    completed, total, percent, snapshot.phase
                )
            }
        }
        Some(_) => format!("Index 0/0 [{}]", snapshot.phase),
        None => format!("Index {} [{}]", snapshot.completed, snapshot.phase),
    };
    if let Some(current) = snapshot.current.as_deref() {
        line.push_str(" — ");
        line.push_str(current);
    }
    line.push_str(" | elapsed ");
    line.push_str(&format_duration(elapsed));
    if let Some(eta) = eta {
        line.push_str(" | phase ETA ");
        line.push_str(&format_duration(eta));
    }
    line
}

fn estimate_remaining(
    done: usize,
    elapsed: Duration,
    remaining: Option<usize>,
) -> Option<Duration> {
    let remaining = remaining?;
    if done < MIN_ETA_SAMPLES || elapsed < MIN_ETA_ELAPSED {
        return None;
    }
    let seconds_per_item = elapsed.as_secs_f64() / done as f64;
    if !seconds_per_item.is_finite() || seconds_per_item <= 0.0 {
        return None;
    }
    let seconds = (seconds_per_item * remaining as f64).ceil();
    if !seconds.is_finite() || seconds > u64::MAX as f64 {
        return None;
    }
    Some(Duration::from_secs(seconds as u64))
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{}h{:02}m{:02}s",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        )
    } else if seconds >= 60 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}s", seconds)
    }
}

fn sanitize_current(value: &str) -> String {
    let mut sanitized = String::with_capacity(MAX_CURRENT_CHARS);
    let mut chars = value.chars();
    for ch in chars.by_ref().take(MAX_CURRENT_CHARS) {
        sanitized.push(if ch.is_control() { ' ' } else { ch });
    }
    if chars.next().is_some() {
        sanitized.push('…');
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatter_shows_phase_percentage_and_never_claims_workspace_completion() {
        let snapshot = Snapshot {
            phase: "indexing",
            completed: 4,
            total: Some(10),
            current: Some("src/lib.rs".to_owned()),
        };
        let line = format_progress(
            &snapshot,
            Duration::from_secs(5),
            Some(Duration::from_secs(8)),
            false,
        );
        assert!(line.contains("4/10 (40%)"));
        assert!(line.contains("[indexing]"));
        assert!(line.contains("src/lib.rs"));
        assert!(line.contains("phase ETA 8s"));
        assert!(!line.contains("overall"));
    }

    #[test]
    fn formatter_omits_percentage_and_eta_when_total_or_samples_are_unknown() {
        let snapshot = Snapshot {
            phase: "inventory",
            completed: 7,
            total: None,
            current: None,
        };
        let line = format_progress(&snapshot, Duration::from_secs(2), None, false);
        assert!(line.contains("Index 7 [inventory]"));
        assert!(!line.contains('%'));
        assert!(!line.contains("ETA"));
    }

    #[test]
    fn eta_requires_a_minimum_sample_and_known_remaining_work() {
        assert_eq!(
            estimate_remaining(2, Duration::from_secs(5), Some(10)),
            None
        );
        assert_eq!(
            estimate_remaining(3, Duration::from_millis(999), Some(10)),
            None
        );
        assert_eq!(estimate_remaining(3, Duration::from_secs(3), None), None);
        assert_eq!(
            estimate_remaining(3, Duration::from_secs(3), Some(7)),
            Some(Duration::from_secs(7))
        );
    }

    #[test]
    fn tty_formatter_uses_a_compact_ascii_progress_bar() {
        let snapshot = Snapshot {
            phase: "indexing",
            completed: 3,
            total: Some(10),
            current: None,
        };
        let line = format_progress(&snapshot, Duration::from_secs(2), None, true);
        assert!(line.contains("[######..............]"));
        assert!(line.contains(" 30%"));
        assert!(!line.contains("100%"));
    }

    #[test]
    fn current_path_cannot_inject_terminal_control_sequences() {
        let value = sanitize_current("src/a\n\u{1b}[31mred");
        assert!(!value.contains('\n'));
        assert!(!value.contains('\u{1b}'));
        assert!(value.contains("src/a"));
    }
}

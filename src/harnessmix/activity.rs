use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug)]
struct State {
    active: bool,
    label: String,
    frame: usize,
    started: Instant,
}

/// Shared activity state. The event loop renders its label through the active
/// line editor so partial user input stays intact.
pub struct Activity {
    enabled: bool,
    state: Mutex<State>,
}

impl Activity {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            state: Mutex::new(State {
                active: false,
                label: String::new(),
                frame: 0,
                started: Instant::now(),
            }),
        }
    }

    pub fn start(&self, label: &str) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let mut state = self.state.lock().map_err(|_| "activity state unavailable")?;
        state.label.clear();
        state.label.push_str(label);
        state.started = Instant::now();
        state.active = true;
        state.frame = state.frame.wrapping_add(1);
        Ok(())
    }

    pub fn pause(&self) -> Result<bool, String> {
        if !self.enabled {
            return Ok(false);
        }
        let mut state = self.state.lock().map_err(|_| "activity state unavailable")?;
        let was_active = state.active;
        state.active = false;
        Ok(was_active)
    }

    pub fn resume(&self, was_active: bool) -> Result<(), String> {
        if !self.enabled || !was_active {
            return Ok(());
        }
        self.state
            .lock()
            .map_err(|_| "activity state unavailable")?
            .active = true;
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        self.state
            .lock()
            .map_err(|_| "activity state unavailable")?
            .active = false;
        Ok(())
    }

    pub fn next_frame(&self) -> Result<Option<(char, String)>, String> {
        if !self.enabled {
            return Ok(None);
        }
        const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
        let mut state = self.state.lock().map_err(|_| "activity state unavailable")?;
        if !state.active {
            return Ok(None);
        }
        let frame = FRAMES[state.frame % FRAMES.len()];
        state.frame = state.frame.wrapping_add(1);
        Ok(Some((frame, format!("{} ({} s)", state.label, state.started.elapsed().as_secs()))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_shows_elapsed_time_and_stops() {
        let activity = Activity::new(true);
        activity.start("Conectando").unwrap();
        assert_eq!(activity.next_frame().unwrap().unwrap().1, "Conectando (0 s)");
        activity.stop().unwrap();
        assert!(activity.next_frame().unwrap().is_none());
    }
}

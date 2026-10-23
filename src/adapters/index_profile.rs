//! Opt-in, bounded publication-local measurements. No filesystem or SQL access.
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, time::Instant};

#[derive(Default)]
struct State {
    enabled: bool,
    profiled: bool,
    generation: u64,
    stage: &'static str,
    operations: BTreeMap<&'static str, (u64, u128)>,
    counters: BTreeMap<&'static str, u64>,
    paths: BTreeMap<String, u64>,
}
thread_local! { static CURRENT: RefCell<State> = RefCell::new(State::default()); }

pub(in crate::adapters) struct Timer {
    name: &'static str,
    started: Option<Instant>,
    generation: u64,
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            let nanos = started.elapsed().as_nanos();
            CURRENT.with(|slot| {
                let mut state = slot.borrow_mut();
                if state.enabled && state.generation == self.generation {
                    let entry = state.operations.entry(self.name).or_default();
                    entry.0 += 1;
                    entry.1 += nanos;
                }
            });
        }
    }
}
pub(in crate::adapters) fn begin(enabled: bool) {
    CURRENT.with(|slot| {
        let generation = slot.borrow().generation.wrapping_add(1);
        *slot.borrow_mut() = State {
            enabled,
            profiled: enabled,
            generation,
            ..State::default()
        };
    });
}
pub(in crate::adapters) fn stage(name: &'static str) {
    CURRENT.with(|slot| slot.borrow_mut().stage = name);
}
pub(in crate::adapters) fn operation(name: &'static str) -> Timer {
    CURRENT.with(|slot| {
        let state = slot.borrow();
        Timer {
            name,
            started: state.enabled.then(Instant::now),
            generation: state.generation,
        }
    })
}
pub(in crate::adapters) fn count(name: &'static str, value: u64) {
    CURRENT.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.enabled {
            *state.counters.entry(name).or_default() += value;
        }
    });
}
pub(in crate::adapters) fn source_read(path: &str) -> Timer {
    let name = CURRENT.with(|slot| {
        let mut state = slot.borrow_mut();
        if state.enabled {
            if path.len() <= 1000 && (state.paths.contains_key(path) || state.paths.len() < 256) {
                *state.paths.entry(path.to_owned()).or_default() += 1;
            } else {
                *state.counters.entry("source_paths_omitted").or_default() += 1;
            }
        }
        match state.stage {
            "resolve_references" => "source_read_resolution",
            "recheck_sources" => "source_read_recheck",
            _ => "source_read_other",
        }
    });
    operation(name)
}
pub(in crate::adapters) fn finish() {
    CURRENT.with(|slot| slot.borrow_mut().enabled = false);
}
pub(in crate::adapters) fn resolver_micros() -> BTreeMap<&'static str, u128> {
    CURRENT.with(|slot| slot.borrow().operations.iter()
        .filter(|(name, _)| name.starts_with("resolver_"))
        .map(|(name, (_, nanos))| (*name, nanos / 1000)).collect())
}
pub(in crate::adapters) fn metrics() -> Option<Value> {
    CURRENT.with(|slot| {
        let state = slot.borrow();
        if !state.profiled {
            return None;
        }
        let operations: serde_json::Map<String, Value> = state
            .operations
            .iter()
            .map(|(name, (calls, nanos))| {
                (
                    (*name).to_owned(),
                    json!({"calls":calls,"totalNanos":nanos}),
                )
            })
            .collect();
        Some(json!({"operations":operations,"counters":state.counters,"sourceReads":state.paths}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_measurements_are_empty_and_new_publications_reset_metrics() {
        begin(false);
        {
            let _timer = operation("test");
            count("bytes", 4);
        }
        assert!(metrics().is_none());
        begin(true);
        {
            let _timer = operation("test");
            count("bytes", 4);
        }
        finish();
        let measured = metrics().unwrap();
        assert_eq!(measured["operations"]["test"]["calls"], 1);
        assert_eq!(measured["counters"]["bytes"], 4);
        begin(true);
        assert!(
            metrics().unwrap()["operations"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn path_cardinality_is_bounded_and_stale_guards_do_not_cross_publications() {
        begin(true);
        stage("recheck_sources");
        for index in 0..300 {
            let _timer = source_read(&format!("file-{index}.rs"));
        }
        let measured = metrics().unwrap();
        assert_eq!(measured["sourceReads"].as_object().unwrap().len(), 256);
        assert_eq!(measured["counters"]["source_paths_omitted"], 44);
        assert_eq!(measured["operations"]["source_read_recheck"]["calls"], 300);
        let stale = operation("stale");
        begin(true);
        drop(stale);
        assert!(metrics().unwrap()["operations"]["stale"].is_null());
    }
}

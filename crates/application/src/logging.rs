//! Fixed-shape metadata only. Never store raw diagnostic strings, arguments or outputs.
use harness_domain::{Failure, contracts::LogsArgs};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex, time::Instant};
const CAPACITY: usize = 512;
const SLOW_MICROS: u64 = 1000000;
#[derive(Default)]
struct State {
    loaded_through: u64,
    saved_sequence: u64,
    persistence_revision: u64,
    write_errors: u64,
    persistence_error: Option<&'static str>,
    sequence: u64,
    discarded: u64,
    events: VecDeque<Value>,
}
pub(super) struct Log {
    persistence: bool,
    started: Instant,
    state: Mutex<State>,
}
impl Default for Log {
    fn default() -> Self {
        Self {
            persistence: false,
            started: Instant::now(),
            state: Mutex::default(),
        }
    }
}

fn code(value: Option<&str>) -> Option<&'static str> {
    value.map(|value| match value {
        "PATH" => "PATH",
        "ARGUMENT" => "ARGUMENT",
        "CONFIG_XML" => "CONFIG_XML",
        "SECTION" => "SECTION",
        "AMBIGUOUS_SECTION" => "AMBIGUOUS_SECTION",
        "AUTHORIZATION" => "AUTHORIZATION",
        "EXTERNAL_SCOPE" => "EXTERNAL_SCOPE",
        "EXTERNAL_SCOPE_LIMIT" => "EXTERNAL_SCOPE_LIMIT",
        "STALE_SCOPE" => "STALE_SCOPE",
        "QUEUE_FULL" => "QUEUE_FULL",
        "CANCELLED" => "CANCELLED",
        "TIMEOUT" => "TIMEOUT",
        "ENCODING" => "ENCODING",
        "SYNTAX_ERROR" => "SYNTAX_ERROR",
        "PARSER_UNAVAILABLE" => "PARSER_UNAVAILABLE",
        "STALE_HASH" => "STALE_HASH",
        "STALE_SYMBOL" => "STALE_SYMBOL",
        "AMBIGUOUS_SYMBOL" => "AMBIGUOUS_SYMBOL",
        "VERIFY_FAILED" => "VERIFY_FAILED",
        "BATCH_PARTIAL" => "BATCH_PARTIAL",
        "UNSUPPORTED" => "UNSUPPORTED",
        "GIT_REPO" => "GIT_REPO",
        "GIT_READ" => "GIT_READ",
        "GIT_SIZE" => "GIT_SIZE",
        "SESSION_BUSY" => "SESSION_BUSY",
        "INTERNAL" => "INTERNAL",
        _ => "TOOL_ERROR",
    })
}
impl Log {
    pub fn configured(enabled: bool, history: Result<Option<Value>, Failure>) -> Self {
        let log = Self {
            persistence: enabled,
            ..Self::default()
        };
        if !enabled {
            return log;
        }
        {
            let mut state = log.state.lock().unwrap_or_else(|e| e.into_inner());
            match history.and_then(|value| {
                if let Some(value) = &value {
                    harness_domain::log_archive::validate(value)?;
                }
                Ok(value)
            }) {
                Ok(Some(value)) => {
                    state.sequence = value["lastSequence"].as_u64().unwrap_or(0);
                    state.loaded_through = state.sequence;
                    state.saved_sequence = state.sequence;
                    state.discarded = value["discarded"].as_u64().unwrap_or(0);
                    state.events = value["events"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned()
                        .collect();
                }
                Ok(None) => {}
                Err((code, _)) => state.persistence_error = Some(code),
            }
        }
        log
    }
    pub fn archive(&self) -> Option<Value> {
        if !self.persistence {
            return None;
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Some(
            json!({"format":harness_domain::log_archive::FORMAT,"lastSequence":state.sequence,"discarded":state.discarded,"events":state.events}),
        )
    }
    pub fn saved(&self, sequence: u64, error: Option<&'static str>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if error.is_some() {
            state.write_errors = state.write_errors.saturating_add(1);
            if sequence > state.saved_sequence && sequence >= state.persistence_revision {
                state.persistence_error = error;
                state.persistence_revision = sequence;
            }
        } else {
            if sequence >= state.persistence_revision {
                state.persistence_error = None;
                state.persistence_revision = sequence;
            }
            state.saved_sequence = state.saved_sequence.max(sequence);
        }
    }
    #[cfg(test)]
    pub fn record(
        &self,
        tool: &'static str,
        kind: &'static str,
        outcome: Option<&str>,
        raw_code: Option<&str>,
        micros: Option<u64>,
    ) {
        self.record_correlated(
            tool,
            kind,
            outcome,
            raw_code,
            micros,
            &harness_domain::execution::new_request_id(),
        );
    }
    pub fn record_correlated(
        &self,
        tool: &'static str,
        kind: &'static str,
        outcome: Option<&str>,
        raw_code: Option<&str>,
        micros: Option<u64>,
        request_id: &str,
    ) {
        let outcome = outcome.map(|s| match s {
            "ok" => "ok",
            "cancelled" => "cancelled",
            "timeout" => "timeout",
            _ => "error",
        });
        let code = code(raw_code);
        let level = if outcome == Some("ok") {
            "info"
        } else if kind == "abandoned"
            || outcome == Some("cancelled")
            || outcome == Some("timeout")
            || matches!(code, Some("QUEUE_FULL" | "CANCELLED" | "TIMEOUT"))
        {
            "warn"
        } else {
            "error"
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sequence = state.sequence.saturating_add(1);
        let sequence = state.sequence;
        let event = json!({"sequence":sequence,"requestId":request_id,"uptimeMs":self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            "tool":tool,"kind":kind,"state":outcome,"level":level,"code":code,"durationMicros":micros,
            "slow":micros.is_some_and(|n|n>=SLOW_MICROS)});
        if state.events.len() == CAPACITY {
            state.events.pop_front();
            state.discarded = state.discarded.saturating_add(1);
        }
        state.events.push_back(event);
    }
    pub fn status(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        json!({"enabled":true,"persistenceRequested":self.persistence,"persistent":self.persistence && state.persistence_error.is_none(),"durability":"best_effort","requestCorrelation":true,"scope":"native_log_history","capacity":CAPACITY,
            "loadedThrough":state.loaded_through,"lastSavedSequence":state.saved_sequence,"writeErrors":state.write_errors,"persistenceError":state.persistence_error,
            "retained":state.events.len(),"discarded":state.discarded,"lastSequence":state.sequence,"slowThresholdMs":1000})
    }
    pub fn read(&self, args: &LogsArgs) -> Result<Value, Failure> {
        let limit = args.limit.unwrap_or(20);
        if !(1..=50).contains(&limit)
            || args
                .level
                .as_deref()
                .is_some_and(|s| !["info", "warn", "error"].contains(&s))
            || args
                .tool
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 80)
        {
            return Err((
                "ARGUMENT",
                "Logs limit 1..50, level info/warn/error, tool 1..80 bytes",
            ));
        }
        if args
            .request_id
            .as_ref()
            .is_some_and(|id| !harness_domain::execution::valid_request_id(id))
        {
            return Err(("ARGUMENT", "requestId must be a canonical lowercase UUID"));
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if args.after.is_some_and(|n| n > state.sequence) {
            return Err((
                "LOG_CURSOR",
                "Cursor belongs to a future sequence or another instance; restart without after",
            ));
        }
        let matches = |event: &&Value| {
            args.level
                .as_ref()
                .is_none_or(|level| event["level"] == *level)
                && args.tool.as_ref().is_none_or(|tool| event["tool"] == *tool)
                && args
                    .request_id
                    .as_ref()
                    .is_none_or(|id| event["requestId"] == *id)
                && (args.slow_only != Some(true) || event["slow"] == true)
                && event["sequence"].as_u64().unwrap_or(0) > args.after.unwrap_or(0)
        };
        let matching: Vec<_> = state.events.iter().filter(matches).collect();
        let mut selected: Vec<_> = if args.after.is_some() {
            matching.iter().take(limit).copied().cloned().collect()
        } else {
            matching
                .iter()
                .skip(matching.len().saturating_sub(limit))
                .copied()
                .cloned()
                .collect()
        };
        let more = args.after.is_some() && matching.len() > selected.len();
        let next = if more {
            selected
                .last()
                .and_then(|e| e["sequence"].as_u64())
                .unwrap_or(state.sequence)
        } else {
            state.sequence
        };
        let oldest = state
            .events
            .front()
            .and_then(|e| e["sequence"].as_u64())
            .unwrap_or(1);
        for event in &mut selected {
            event["historical"] =
                json!(event["sequence"].as_u64().unwrap_or(0) <= state.loaded_through);
        }
        Ok(
            json!({"events":selected,"nextAfter":next,"lastSequence":state.sequence,"historyPruned":args.after.is_some_and(|n|n<oldest.saturating_sub(1)),
            "more":more,"retained":state.events.len(),"discarded":state.discarded,"capacity":CAPACITY,
            "persistenceRequested":self.persistence,"persistent":self.persistence && state.persistence_error.is_none(),"durability":"best_effort","scope":"native_log_history","loadedThrough":state.loaded_through,"persistenceError":state.persistence_error,
            "cursorResetOnRestart":!self.persistence || state.persistence_error.is_some(),
            "coverage":"worker terminal/rejected/abandoned metadata only; not RPC transport or Node log files"}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> LogsArgs {
        LogsArgs {
            limit: Some(2),
            level: None,
            tool: None,
            slow_only: None,
            request_id: None,
            after: Some(0),
            session: None,
        }
    }
    #[test]
    fn delayed_flush_reports_cannot_hide_a_newer_storage_failure() {
        let log = Log::configured(true, Ok(None));
        log.saved(2, None);
        log.saved(3, Some("LOG_IO"));
        log.saved(2, None);
        assert_eq!(log.status()["persistenceError"], "LOG_IO");
        log.saved(4, None);
        log.saved(3, Some("LOG_IO"));
        assert!(log.status()["persistenceError"].is_null());
    }
    #[test]
    fn bounded_history_cursor_and_sanitized_codes() {
        let log = Log::default();
        for _ in 0..514 {
            log.record(
                "harness_read",
                "completed",
                Some("error"),
                Some("credential=must-not-survive"),
                Some(12),
            );
        }
        let page = log.read(&args()).unwrap();
        assert_eq!(page["retained"], 512);
        assert_eq!(page["discarded"], 2);
        assert_eq!(page["historyPruned"], true);
        assert_eq!(page["nextAfter"], 4);
        assert_eq!(page["events"][0]["code"], "TOOL_ERROR");
        assert!(!page.to_string().contains("credential"));
        let mut filter = args();
        filter.tool = Some("harness_other".into());
        assert_eq!(log.read(&filter).unwrap()["nextAfter"], 514);
    }

    #[test]
    fn level_tool_and_slow_filters_do_not_retain_arbitrary_values() {
        let log = Log::default();
        log.record(
            "harness_read",
            "completed",
            Some("ok"),
            None,
            Some(SLOW_MICROS),
        );
        log.record(
            "harness_search",
            "completed",
            Some("error"),
            Some("ARGUMENT"),
            Some(5),
        );
        let mut query = args();
        query.after = None;
        query.slow_only = Some(true);
        let page = log.read(&query).unwrap();
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
        assert_eq!(page["events"][0]["tool"], "harness_read");
        query.level = Some("error".into());
        assert!(
            log.read(&query).unwrap()["events"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        query.after = Some(3);
        assert!(log.read(&query).is_err());
        query.after = None;
        query.limit = Some(0);
        assert!(log.read(&query).is_err());
    }
}

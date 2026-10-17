//! Declared task state is never evidence of a successful build or a current check.
use crate::{Failure, results::utf16_len};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const IDS: [&str; 4] = ["analysis", "implementation", "verification", "review"];
pub const TITLES: [&str; 4] = [
    "Analizar flujo",
    "Implementar cambio",
    "Verificar",
    "Revisar diff",
];
pub const STATUSES: [&str; 5] = [
    "pending",
    "in_progress",
    "blocked",
    "completed",
    "cancelled",
];

pub fn note_is_blank(note: &str) -> bool {
    note.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .is_empty()
}

pub struct TaskBoard {
    phases: Vec<Value>,
    sequence: u64,
    events: VecDeque<Value>,
}
impl TaskBoard {
    pub fn new(time: &str) -> Self {
        let mut board = Self {
            phases: IDS
                .iter()
                .zip(TITLES)
                .map(|(id, title)| json!({"id":id,"title":title,"status":"pending"}))
                .collect(),
            sequence: 0,
            events: VecDeque::new(),
        };
        board.event("plan_created", time, json!({"source":"harness"}));
        board
    }
    /// Input phases must already have passed the persistent-state validator.
    pub fn restore(phases: Vec<Value>, time: &str) -> Self {
        let mut board = Self::new(time);
        board.phases = phases;
        board.event("session_restored", time, json!({"source":"harness"}));
        board
    }
    pub fn phases(&self) -> Vec<Value> {
        self.phases.clone()
    }
    pub fn event(&mut self, kind: &str, time: &str, data: Value) {
        self.sequence += 1;
        let mut event = json!({"seq":self.sequence,"time":time,"type":kind});
        if let Some(data) = data.as_object() {
            event.as_object_mut().unwrap().extend(data.clone());
        }
        self.events.push_back(event);
        if self.events.len() > 128 {
            self.events.pop_front();
        }
    }
    pub fn summary(&self) -> Value {
        json!({"phases":self.phases,"active":self.phases.iter().find(|p|p["status"]=="in_progress").map(|p|&p["id"]),
            "completed":self.phases.iter().filter(|p|p["status"]=="completed").count(),"total":4,"statusProvenance":"agent-declared","latestEvent":self.sequence})
    }
    pub fn update(
        &mut self,
        phase: &str,
        status: &str,
        note: &str,
        time: &str,
    ) -> Result<bool, Failure> {
        let index = self
            .phases
            .iter()
            .position(|p| p["id"] == phase)
            .ok_or(("TASK", "Invalid phase"))?;
        if !STATUSES.contains(&status) || utf16_len(note) > 300 {
            return Err(("TASK", "Invalid status or note"));
        }
        let previous = self.phases[index]["status"]
            .as_str()
            .ok_or(("TASK", "Invalid phase state"))?
            .to_owned();
        if status == "blocked" && note_is_blank(note) {
            return Err(("TASK", "Blocked phase requires a reason"));
        }
        if ["completed", "cancelled"].contains(&previous.as_str())
            && previous != status
            && note_is_blank(note)
        {
            return Err(("TASK", "Reopening a phase requires a reason"));
        }
        if status == "in_progress"
            && self
                .phases
                .iter()
                .any(|p| p["id"] != phase && p["status"] == "in_progress")
        {
            return Err(("TASK", "Finish or block the active phase first"));
        }
        if previous == status && self.phases[index]["note"].as_str().unwrap_or("") == note {
            return Ok(false);
        }
        self.phases[index]["status"] = json!(status);
        if note.is_empty() {
            self.phases[index].as_object_mut().unwrap().remove("note");
        } else {
            self.phases[index]["note"] = json!(note);
        }
        let mut data =
            json!({"phase":phase,"previous":previous,"status":status,"source":"agent-declared"});
        if !note.is_empty() {
            data["note"] = json!(note);
        }
        self.event("phase_updated", time, data);
        Ok(true)
    }
    pub fn timeline(&self, after: u64, limit: usize) -> Result<Value, Failure> {
        if after > 9_007_199_254_740_991 || !(1..=20).contains(&limit) {
            return Err(("TASK", "Invalid timeline pagination"));
        }
        let all: Vec<_> = self
            .events
            .iter()
            .filter(|e| e["seq"].as_u64().unwrap() > after)
            .collect();
        let events: Vec<_> = all.iter().take(limit).copied().collect();
        Ok(
            json!({"events":events,"nextAfter":if all.len()>limit { events.last().map(|e|e["seq"].clone()) } else { None },
            "historyPruned":self.events.front().is_some_and(|e|after<e["seq"].as_u64().unwrap()-1),"latestEvent":self.sequence}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions_match_declared_policy_and_noops_do_not_create_events() {
        let mut board = TaskBoard::new("fixed-time");
        assert!(!board.update("analysis", "pending", "", "t").unwrap());
        board.update("analysis", "in_progress", "", "t").unwrap();
        assert!(
            board
                .update("implementation", "in_progress", "", "t")
                .is_err()
        );
        assert!(board.update("analysis", "blocked", " ", "t").is_err());
        board.update("analysis", "completed", "", "t").unwrap();
        assert!(board.update("analysis", "pending", "", "t").is_err());
        board
            .update("analysis", "pending", "new work", "t")
            .unwrap();
        assert_eq!(board.summary()["statusProvenance"], "agent-declared");
        assert_eq!(board.summary()["latestEvent"], 4);
    }
    #[test]
    fn timeline_is_bounded_and_reports_pruning() {
        let mut board = TaskBoard::new("t");
        for _ in 0..140 {
            board.event("test", "t", json!({"source":"harness-observed"}));
        }
        let first = board.timeline(0, 20).unwrap();
        assert_eq!(first["historyPruned"], true);
        assert_eq!(first["events"].as_array().unwrap().len(), 20);
        assert!(first["nextAfter"].is_number());
        assert!(board.timeline(0, 21).is_err());
    }
}

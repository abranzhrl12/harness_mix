use harness_domain::workflow::{IDS, TITLES};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize)]
pub struct Declared {
    pub goal: String,
    pub decisions: Vec<String>,
    pub pending: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    pub declared: Declared,
    pub phases: Vec<Value>,
    pub edits: Vec<Value>,
    pub checks: Vec<Value>,
}

pub struct Session {
    pub(in crate::adapters) tasks: harness_domain::workflow::TaskBoard,
    pub(in crate::adapters) workflow_gate: std::sync::Arc<std::sync::Mutex<()>>,
    pub(in crate::adapters) observed_edits: Vec<Value>,
    pub(in crate::adapters) observed_checks: Vec<Value>,
    pub(super) in_flight: usize,
    pub state: State,
    pub rules_hash: Option<String>,
    pub capabilities_hash: Option<String>,
    pub restored: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tasks: harness_domain::workflow::TaskBoard::new(&super::now()),
            workflow_gate: std::sync::Arc::new(std::sync::Mutex::new(())),
            observed_edits: vec![],
            observed_checks: vec![],
            in_flight: 0,
            state: State {
                declared: Declared {
                    goal: String::new(),
                    decisions: vec![],
                    pending: vec![],
                },
                phases: IDS
                    .iter()
                    .zip(TITLES)
                    .map(|(id, title)| json!({"id":id,"title":title,"status":"pending"}))
                    .collect(),
                edits: vec![],
                checks: vec![],
            },
            rules_hash: None,
            capabilities_hash: None,
            restored: false,
        }
    }
}

impl Session {
    pub fn summary(&self) -> Value {
        self.tasks.summary()
    }
}

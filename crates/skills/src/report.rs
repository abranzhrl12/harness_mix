//! Public synchronization report types and mutation helpers.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Report {
    pub package_version: &'static str,
    pub contract_version: &'static str,
    pub changes: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub actions: Vec<Action>,
}

#[derive(Debug, Serialize)]
pub struct Action {
    pub skill: String,
    pub action: &'static str,
    pub path: String,
}

impl Report {
    pub fn new() -> Self {
        Self {
            package_version: env!("CARGO_PKG_VERSION"),
            contract_version: crate::manifest::contract_version(),
            changes: 0,
            conflicts: 0,
            errors: 0,
            actions: Vec::new(),
        }
    }

    pub fn add_action(&mut self, skill: &str, action: &'static str, path: &str) {
        self.changes += usize::from(matches!(action, "created" | "enabled"));
        self.actions.push(Action {
            skill: skill.into(),
            action,
            path: path.into(),
        });
    }

    pub fn add_error(&mut self, skill: &str, path: &str, error: impl std::fmt::Display) {
        self.errors += 1;
        self.actions.push(Action {
            skill: skill.into(),
            action: "error",
            path: format!("{path}: {error}"),
        });
    }
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

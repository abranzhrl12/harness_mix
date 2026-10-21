use super::Request;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TasksAction {
    Status,
    Update,
    Timeline,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TasksArgs {
    pub session: String,
    pub action: Option<TasksAction>,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub note: Option<String>,
    pub after: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[serde(try_from = "CheckpointInput")]
pub struct CheckpointArgs {
    pub session: String,
    pub goal: Option<String>,
    pub decisions: Option<Vec<String>>,
    pub pending: Option<Vec<String>>,
    #[serde(rename = "refreshFreshness")]
    pub refresh_freshness: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct CheckpointInput {
    session: String,
    goal: Option<String>,
    decisions: Option<Vec<String>>,
    pending: Option<Vec<String>>,
    #[serde(rename = "refreshFreshness")]
    refresh_freshness: Option<bool>,
    /// Alternate envelope for goal, decisions and pending. Duplicate fields are rejected.
    data: Option<CheckpointData>,
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct CheckpointData {
    goal: Option<String>,
    decisions: Option<Vec<String>>,
    pending: Option<Vec<String>>,
}

impl TryFrom<CheckpointInput> for CheckpointArgs {
    type Error = String;
    fn try_from(input: CheckpointInput) -> Result<Self, Self::Error> {
        let data = input.data.unwrap_or_default();
        fn merge<T>(root: Option<T>, nested: Option<T>, field: &str) -> Result<Option<T>, String> {
            if root.is_some() && nested.is_some() {
                return Err(format!(
                    "duplicate checkpoint field {field}: supply it at the root or in data, not both"
                ));
            }
            Ok(root.or(nested))
        }
        Ok(Self {
            session: input.session,
            goal: merge(input.goal, data.goal, "goal")?,
            decisions: merge(input.decisions, data.decisions, "decisions")?,
            pending: merge(input.pending, data.pending, "pending")?,
            refresh_freshness: input.refresh_freshness,
        })
    }
}
impl CheckpointArgs {
    pub fn has_updates(&self) -> bool {
        self.goal.is_some() || self.decisions.is_some() || self.pending.is_some()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum HandoverKind {
    #[default]
    Summary,
    Adr,
    PrBody,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct HandoverArgs {
    pub session: String,
    pub kind: Option<HandoverKind>,
    #[serde(rename = "targetFile")]
    pub target_file: Option<String>,
    #[serde(rename = "expectedHash")]
    pub expected_hash: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SessionTasksArgs {
    pub session: String,
    pub subaction: Option<TasksAction>,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub note: Option<String>,
    pub after: Option<u64>,
    pub limit: Option<usize>,
}

impl SessionTasksArgs {
    pub fn into_tasks_args(self) -> TasksArgs {
        let action = self.subaction.or_else(|| {
            if self.phase.is_some() || self.status.is_some() || self.note.is_some() {
                Some(TasksAction::Update)
            } else if self.after.is_some() || self.limit.is_some() {
                Some(TasksAction::Timeline)
            } else {
                Some(TasksAction::Status)
            }
        });
        TasksArgs {
            session: self.session,
            action,
            phase: self.phase,
            status: self.status,
            note: self.note,
            after: self.after,
            limit: self.limit,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SessionStatusArgs {
    pub session: String,
}

/// Unified session and workflow orchestration.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("type" = "object")))]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionArgs {
    Checkpoint(CheckpointArgs),
    Tasks(SessionTasksArgs),
    Handover(HandoverArgs),
    Status(SessionStatusArgs),
}

impl SessionArgs {
    pub fn into_request(self) -> Request {
        match self {
            Self::Checkpoint(args) => Request::Checkpoint(args),
            Self::Tasks(args) => Request::Tasks(args.into_tasks_args()),
            Self::Handover(args) => Request::Handover(args),
            Self::Status(args) => Request::Checkpoint(CheckpointArgs {
                session: args.session,
                goal: None,
                decisions: None,
                pending: None,
                refresh_freshness: Some(false),
            }),
        }
    }
}

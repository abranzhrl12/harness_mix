use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Node {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) file_path: String,
    pub(super) start_line: i64,
}

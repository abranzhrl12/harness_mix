use serde_json::Value;

/// Owns rows returned by a scan and enforces the shared metadata budget.
#[derive(Default)]
pub(super) struct MatchResults {
    pub(super) rows: Vec<Value>,
    metadata_bytes: usize,
}

impl MatchResults {
    /// Accounts for the candidate row before applying the metadata and optional
    /// row-count limits, matching the scan's historical admission order.
    pub(super) fn add(&mut self, row: Value, enforce_row_limit: bool) -> bool {
        self.metadata_bytes += row.to_string().len();
        if self.metadata_bytes > 4 * 1024 * 1024 || (enforce_row_limit && self.rows.len() >= 10000)
        {
            return false;
        }
        self.rows.push(row);
        true
    }
}

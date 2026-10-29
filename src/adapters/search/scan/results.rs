use serde_json::Value;
use std::io::{self, Write};

struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialized_size(value: &Value) -> usize {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, value).expect("the in-memory counter writer cannot fail");
    counter.0
}

/// Owns rows returned by a scan and enforces the shared metadata budget.
#[derive(Default)]
pub(super) struct MatchResults {
    pub(super) rows: Vec<Value>,
    pub(super) skip_remaining: usize,
    metadata_bytes: usize,
}

impl MatchResults {
    pub(super) fn limit_reason(&self) -> &'static str {
        if self.metadata_bytes > 4 * 1024 * 1024 {
            "metadata_budget"
        } else {
            "match_count_budget"
        }
    }

    /// Accounts for the candidate row before applying the metadata and optional
    /// row-count limits, matching the scan's historical admission order.
    pub(super) fn add(&mut self, row: Value, enforce_row_limit: bool) -> bool {
        if self.skip_remaining > 0 {
            self.skip_remaining -= 1;
            return true;
        }
        self.metadata_bytes = self.metadata_bytes.saturating_add(serialized_size(&row));
        if self.metadata_bytes > 4 * 1024 * 1024 || (enforce_row_limit && self.rows.len() >= 10000)
        {
            return false;
        }
        self.rows.push(row);
        true
    }
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod tests;

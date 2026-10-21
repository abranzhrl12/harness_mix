//! Persist fixed-shape metadata only; bounded diagnostic context lives in memory.
mod sanitize;
mod state;

#[cfg(test)]
mod tests;

pub(super) use state::Log;

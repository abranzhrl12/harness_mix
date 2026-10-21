pub mod adapters;
pub mod candidate;
pub use adapters::baseline;

/// Production serialization intentionally omits byte ranges; include them in parity checks.
pub fn snapshot(symbols: &[harness_domain::parsing::Symbol]) -> serde_json::Value {
    serde_json::Value::Array(symbols.iter().map(|s| serde_json::json!({
        "symbol":s,"byteStart":s.bytes.start,"byteEnd":s.bytes.end
    })).collect())
}

//! Path-module shim for the real production call resolver. No copied logic.
use harness_domain::{Failure, execution::OperationContext};
use std::collections::BTreeMap;

#[path = "../../../baseline/index_rust.rs"]
pub mod rust;

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub functions: Vec<(String, (i64, bool))>,
    pub modules: Vec<String>,
    pub call_queries: BTreeMap<(String, i64, i64), Option<Vec<String>>>,
    pub reserved: usize,
}

pub fn snapshot(source: &str, probes: &[(String, i64, i64)], context: &OperationContext)
    -> Result<Snapshot, Failure> {
    let facts = rust::analyze(source, context)?;
    let call_queries = probes.iter().cloned().map(|(name, line, col)| {
        let value = facts.call(&name, line, col).map(<[String]>::to_vec);
        ((name, line, col), value)
    }).collect();
    Ok(Snapshot {
        functions: facts.functions.iter().map(|(name, value)| (name.clone(), *value)).collect(),
        modules: facts.modules.iter().cloned().collect(),
        call_queries,
        reserved: facts.reserved(),
    })
}

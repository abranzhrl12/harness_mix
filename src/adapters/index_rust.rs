//! Bounded RA call proofs. No receiver/type or runtime guesses.
use super::{Failure, OperationContext};
use ra_ap_syntax::AstNode;
use std::collections::{BTreeMap, BTreeSet};
#[path = "index_rust/bindings.rs"]
mod bindings;
#[path = "index_rust/calls.rs"]
mod calls;
#[derive(Default)]
pub(super) struct Facts {
    pub functions: BTreeMap<String, (i64, bool)>,
    pub modules: BTreeSet<String>,
    calls: BTreeMap<(String, i64, i64), Vec<String>>,
}

impl Facts {
    pub fn reserved(&self) -> usize {
        self.functions.keys().map(|s| 256 + s.capacity()).sum::<usize>()
            + self.modules.iter().map(|s| 256 + s.capacity()).sum::<usize>()
            + self.calls.iter().map(|((name, _, _), parts)| {
                256 + name.capacity() + parts.capacity() * std::mem::size_of::<String>()
                    + parts.iter().map(String::capacity).sum::<usize>()
            }).sum::<usize>()
    }

    pub fn call(&self, name: &str, line: i64, col: i64) -> Option<&[String]> {
        self.calls.get(&(name.to_owned(), line, col)).map(Vec::as_slice)
    }

}


pub(super) fn analyze(source: &str, context: &OperationContext) -> Result<Facts, Failure> {
    super::super::rust_syntax::with_ast(source, context, |file, base| {
        let root = file.syntax();
        let mut facts = Facts::default();
        bindings::collect_top_level(root, source, base, context, &mut facts)?;
        let bindings = bindings::bindings(root, context)?;
        let mut receivers = BTreeSet::new();
        let functions = facts.functions.keys().cloned().collect();
        calls::visit_calls(root, source, base, context, &bindings, &functions, &mut receivers, &mut facts.calls)?;
        for receiver in &receivers { facts.calls.remove(receiver); }
        context.check()?;
        Ok(facts)
    })
}

fn position(source: &str, offset: usize) -> Option<(i64, i64)> {
    let prefix = source.get(..offset)?;
    Some((prefix.bytes().filter(|b| *b == b'\n').count() as i64 + 1,
        prefix.rsplit('\n').next()?.encode_utf16().count() as i64))
}

#[cfg(test)]
#[path = "index_rust/tests.rs"]
mod tests;

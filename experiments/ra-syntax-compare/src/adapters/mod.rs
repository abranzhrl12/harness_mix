pub mod parsers;
pub(in crate::adapters) use parsers::rust::syntax as rust_syntax;
#[path = "store/mod.rs"]
pub mod store;

pub fn baseline(source: &str) -> Result<Vec<harness_domain::parsing::Symbol>, harness_domain::Failure> {
    parsers::rust::symbols::collect(
        source,
        &harness_domain::hash(source.as_bytes()),
        &harness_domain::execution::OperationContext::default(),
    )
}

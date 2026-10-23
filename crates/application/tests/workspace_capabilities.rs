use harness_application::ports::workspace::SourceQueries;
use harness_domain::{
    Failure,
    contracts::{OutlineArgs, ReadArgs, SearchArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};

// This adapter deliberately implements no database, mutation or metadata ports.
struct MemorySources;

impl SourceQueries for MemorySources {
    fn read(&self, args: &ReadArgs, context: &OperationContext) -> Result<Value, Failure> {
        context.check()?;
        Ok(json!({"path":args.path,"text":"in-memory source"}))
    }

    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure> {
        context.check()?;
        Ok(json!({"query":args.query,"matches":[]}))
    }
}

fn load_source(reader: &dyn SourceQueries, args: &ReadArgs) -> Result<Value, Failure> {
    reader.read(args, &OperationContext::default())
}

#[test]
fn a_source_reader_can_be_injected_without_unrelated_workspace_capabilities() {
    let args = ReadArgs {
        external_scope: None,
        path: "sample.rs".into(),
        start: 1,
        end: 1,
        compact: false,
        line_numbers: false,
        session: None,
    };
    assert_eq!(
        load_source(&MemorySources, &args).unwrap(),
        json!({"path":"sample.rs","text":"in-memory source"})
    );
}

#[test]
fn optional_query_operations_keep_their_explicit_unsupported_result() {
    let args: OutlineArgs = serde_json::from_value(json!({"path":"sample.rs"})).unwrap();
    assert_eq!(
        MemorySources
            .outline(&args, &OperationContext::default())
            .unwrap_err(),
        ("UNSUPPORTED", "Outline adapter unavailable")
    );
}

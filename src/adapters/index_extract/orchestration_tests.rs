    use super::*;

    #[test]
    fn literal_sql_commands_bind_to_the_enclosing_method_only() {
        let source = r#"class C { void Run() { new SqlCommand("dbo.Work", connection); var text = "new SqlCommand(\"Fake\")"; /* new SqlCommand("No") */ var verbatim = @"doubled "" new SqlCommand(""VerbatimFake"")"; var raw = """new SqlCommand("RawFake")"""; } }"#;
        let mut graph = json!({
            "nodes":[
                {"id":"file","kind":"file","filePath":"Data.cs"},
                {"id":"run","kind":"method","name":"Run","startLine":1,"endLine":1}
            ],
            "unresolvedReferences":[]
        });
        collect_sql_command_refs(source, &mut graph, &OperationContext::default()).unwrap();
        let references = graph["unresolvedReferences"].as_array().unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0]["fromNodeId"], "run");
        assert_eq!(references[0]["referenceName"], "dbo.Work");
        assert_eq!(references[0]["referenceKind"], "stored_procedure_call");
    }

    #[test]
    fn literal_sql_commands_on_same_line_bind_to_the_method_column_interval() {
        let source = "class C { void First() { new SqlCommand(\"dbo.First\"); } void Second() { new SqlCommand(\"dbo.Second\"); } }";
        let first_start = source.find("void First").unwrap();
        let first_end = source.find("} void Second").unwrap() + 1;
        let second_start = source.find("void Second").unwrap();
        let second_end = source.len();
        let mut graph = json!({
            "nodes":[
                {"id":"file","kind":"file","filePath":"Data.cs"},
                {"id":"first","kind":"method","name":"First","startLine":1,"startColumn":first_start,"endLine":1,"endColumn":first_end},
                {"id":"second","kind":"method","name":"Second","startLine":1,"startColumn":second_start,"endLine":1,"endColumn":second_end}
            ],
            "unresolvedReferences":[]
        });

        collect_sql_command_refs(source, &mut graph, &OperationContext::default()).unwrap();

        let references = graph["unresolvedReferences"].as_array().unwrap();
        assert_eq!(references.len(), 2);
        assert_eq!(references[0]["fromNodeId"], "first");
        assert_eq!(references[1]["fromNodeId"], "second");
    }

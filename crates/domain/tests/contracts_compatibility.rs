#[cfg(feature = "schema")]
use harness_domain::contracts::CheckpointArgs;
use harness_domain::contracts::{
    BatchJob, BatchTool, DoctorToolArgs, GitArgs, ReadArgs, ReplaceArgs, SearchArgs, SessionArgs,
    SymbolsArgs,
};

#[cfg(test)]
mod symbols_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_selectors_views_scope_and_session_in_both_actions() {
        for value in [
            json!({"action":"outline","path":"src/lib.rs","query":"target","maxChars":2500,"externalScope":"scope","session":"s"}),
            json!({"action":"outline","path":"src/lib.rs","filterKind":"method","view":"compact","session":"s"}),
            json!({"action":"read","path":"src/lib.rs","symbol":"target","container":"mod","symbolId":"0123456789abcdef01234567","view":"full","externalScope":"scope","session":"s"}),
            json!({"action":"read","path":"src/lib.rs","symbol":"target","view":"signature","session":"s"}),
            json!({"action":"extract_preview","path":"Parent.vue","expectedHash":"a".repeat(64),"symbolId":"b".repeat(24),"targetPath":"Child.vue","componentName":"ChildView","session":"s"}),
        ] {
            let request = serde_json::from_value::<SymbolsArgs>(value.clone())
                .unwrap()
                .into_request();
            assert_eq!(request.name(), "harness_symbols");
            assert!(!request.is_mutating());
            let restored = request.arguments();
            for (key, expected) in value.as_object().unwrap() {
                assert_eq!(&restored[key], expected);
            }
            assert!(serde_json::from_value::<SymbolsArgs>(restored).is_ok());
        }
        let legacy_session: SymbolsArgs = serde_json::from_value(
            json!({"action":"read","path":"x.rs","symbol":"x","sessionId":"s"}),
        )
        .unwrap();
        assert_eq!(legacy_session.into_request().arguments()["session"], "s");
    }

    #[test]
    fn rejects_incomplete_or_cross_action_arguments_and_supports_batch() {
        for value in [
            json!({}),
            json!({"action":"outline"}),
            json!({"action":"read","path":"x.rs"}),
            json!({"action":"read","path":"x.rs","name":"x"}),
            json!({"action":"outline","path":"x.rs","view":"full"}),
            json!({"action":"read","path":"x.rs","symbol":"x","query":"y"}),
            json!({"action":"delete","path":"x.rs"}),
            json!({"action":"extract_preview","path":"Parent.vue"}),
            json!({"action":"extract_preview","path":"Parent.vue","expectedHash":"a".repeat(64),"symbolId":"b".repeat(24),"targetPath":"Child.vue","componentName":"ChildView","apply":true}),
        ] {
            assert!(serde_json::from_value::<SymbolsArgs>(value).is_err());
        }
        for action in ["outline", "read"] {
            let mut value =
                json!({"tool":"harness_symbols","arguments":{"action":action,"path":"x.rs"}});
            if action == "read" {
                value["arguments"]["symbol"] = json!("x");
            }
            let request = serde_json::from_value::<BatchJob>(value)
                .unwrap()
                .into_request(Some("s"))
                .unwrap();
            assert_eq!(request.arguments()["action"], action);
            assert_eq!(request.arguments()["session"], "s");
        }
    }

    #[cfg(feature = "schema")]
    #[test]
    fn symbols_schema_requires_path_and_symbol_only_for_read() {
        let schema = serde_json::to_value(schemars::schema_for!(SymbolsArgs)).unwrap();
        assert_eq!(schema["type"], "object");
        let branches = schema["oneOf"].as_array().unwrap();
        assert_eq!(branches.len(), 3);
        for branch in branches {
            let required = branch["required"].as_array().unwrap();
            assert!(required.contains(&json!("path")));
            assert_eq!(
                required.contains(&json!("symbol")),
                branch["properties"]["action"]["const"] == "read"
            );
        }
        let outline = branches
            .iter()
            .find(|branch| branch["properties"]["action"]["const"] == "outline")
            .unwrap();
        assert!(outline["properties"].get("filterKind").is_some());
        assert!(outline["properties"].get("view").is_some());
    }
}

#[cfg(test)]
mod compatibility_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inocuous_envelope_fields_are_tolerated_for_edit_and_search() {
        use harness_domain::hash;
        let replace: ReplaceArgs = serde_json::from_value(json!({
            "action":"harness_replace","object":"file","objectType":"text",
            "path":"a.txt","oldText":"a","newText":"b","baseHash":hash(b"abc")
        }))
        .unwrap();
        assert_eq!(replace.old_text, "a");
        assert_eq!(replace.hash, hash(b"abc"));
        let search: SearchArgs = serde_json::from_value(json!({
            "action":"harness_search","object":"files",
            "kind":"files","query":"a","scope":"."  }))
        .unwrap();
        assert_eq!(search.query, "a");
        assert!(
            serde_json::from_value::<ReplaceArgs>(json!({
                "path":"a.txt","oldText":"a","newText":"b","unknown":true}))
            .is_err()
        );
    }

    #[test]
    fn search_aliases_normalize_and_reject_duplicates_with_example() {
        let args: SearchArgs = serde_json::from_value(json!({
            "kind":"text", "query":"Controller", "path":"Controllers", "includes":["*.cs"]
        }))
        .unwrap();
        assert_eq!(args.scope.as_deref(), Some("Controllers"));
        assert_eq!(args.globs.unwrap(), vec!["*.cs"]);
        for input in [
            json!({"kind":"text","path":"a","scope":"b"}),
            json!({"kind":"text","scope":42}),
            json!({"kind":"text","includes":["*.cs"],"globs":["*.js"]}),
        ] {
            let error = serde_json::from_value::<SearchArgs>(input)
                .unwrap_err()
                .to_string();
            assert!(error.contains("\"scope\":\"Controllers\""), "{error}");
        }
    }

    #[test]
    fn read_file_alias_normalizes_and_rejects_duplicate_selectors() {
        let args: ReadArgs =
            serde_json::from_value(json!({"file":"view.cshtml","start":1,"lines":5})).unwrap();
        assert_eq!(args.path, "view.cshtml");
        assert_eq!(args.end, 5);
        let error =
            serde_json::from_value::<ReadArgs>(json!({"file":"a","path":"b","start":1,"end":5}))
                .unwrap_err()
                .to_string();
        assert!(error.contains("duplicate field `path`"), "{error}");
    }

    #[test]
    fn checkpoint_data_normalizes_dispatches_and_rejects_conflicts() {
        let args: SessionArgs = serde_json::from_value(json!({
            "action":"checkpoint","session":"s","data":{"goal":"fix","decisions":["aliases"],"pending":["test"]}
        })).unwrap();
        let request = args.into_request();
        assert!(request.is_mutating());
        let canonical = request.arguments();
        assert_eq!(canonical["goal"], "fix");
        assert_eq!(canonical["decisions"], json!(["aliases"]));
        assert!(canonical.get("data").is_none());
        assert!(
            serde_json::from_value::<SessionArgs>(json!({
                "action":"checkpoint","session":"s","goal":"a","data":{"goal":"b"}
            }))
            .unwrap_err()
            .to_string()
            .contains("duplicate checkpoint field goal")
        );
        assert!(
            serde_json::from_value::<SessionArgs>(json!({
                "action":"checkpoint","session":"s","data":{"unknown":true}
            }))
            .is_err()
        );
    }

    #[cfg(feature = "schema")]
    #[test]
    fn input_schemas_advertise_aliases_and_checkpoint_envelope() {
        let search = serde_json::to_value(schemars::schema_for!(SearchArgs)).unwrap();
        assert!(
            !search["required"]
                .as_array()
                .unwrap_or(&Vec::new())
                .contains(&json!("kind"))
        );
        assert_eq!(search["properties"]["kind"]["default"], "text");
        assert!(search["properties"].get("path").is_some());
        assert!(search["properties"].get("includes").is_some());
        assert!(search["properties"].get("contextLines").is_some());
        assert!(search["properties"].get("context_lines").is_some());
        let context_description = search["properties"]["contextLines"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(context_description.contains("matches[].context.text"));
        assert!(context_description.contains("matches[].hash"));
        assert!(context_description.contains("context: {start, end, text}"));
        assert!(context_description.contains("original line endings"));
        assert!(
            !search["allOf"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .any(|constraint| constraint["not"]["required"] == json!(["scope", "path"]))
        );
        let replace = serde_json::to_value(schemars::schema_for!(ReplaceArgs)).unwrap();
        for key in [
            "oldText",
            "old_text",
            "newText",
            "new_text",
            "allowUnvalidatedText",
            "allow_unvalidated_text",
            "line",
        ] {
            assert!(replace["properties"].get(key).is_some(), "{key}: {replace}");
        }
        assert!(
            !replace["required"]
                .as_array()
                .unwrap()
                .contains(&json!("hash"))
        );
        assert!(
            replace["properties"]["hash"]["description"]
                .as_str()
                .unwrap_or_default()
                .contains("matches[].hash")
        );
        let read = serde_json::to_value(schemars::schema_for!(ReadArgs)).unwrap();
        assert!(read["properties"].get("file").is_some());
        assert!(
            !read["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path"))
        );
        let required = read["required"].as_array().unwrap();
        assert!(!required.contains(&json!("start")));
        assert!(!required.contains(&json!("end")));
        let checkpoint = serde_json::to_value(schemars::schema_for!(CheckpointArgs)).unwrap();
        assert!(checkpoint["properties"].get("data").is_some());
    }
}

#[cfg(test)]
mod search_ergonomics_tests {
    use harness_domain::contracts::{Kind, SearchArgs};
    use serde_json::json;

    #[test]
    fn search_defaults_to_text_and_accepts_equivalent_scope_aliases() {
        let args: SearchArgs = serde_json::from_value(json!({
            "query": "needle",
            "scope": "src\\adapters/",
            "path": "src/adapters"
        }))
        .unwrap();
        assert!(matches!(args.kind, Kind::Text));
        assert_eq!(args.scope.as_deref(), Some("src\\adapters/"));
    }

    #[test]
    fn search_rejects_conflicting_scope_aliases() {
        let error = serde_json::from_value::<SearchArgs>(json!({
            "kind": "text",
            "query": "needle",
            "scope": "src",
            "path": "tests"
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("scope and path refer to different locations"));
    }

    #[test]
    fn bootstrap_action_recommends_doctor() {
        let error = serde_json::from_value::<harness_domain::contracts::BootstrapArgs>(json!({
            "action": "env"
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("harness_doctor"));
        assert!(error.contains("action"));
    }
}

#[cfg(test)]
mod git_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unified_git_preserves_action_options_and_read_only_admission() {
        for value in [
            json!({"action":"status","includeUntracked":false,"offset":5,"limit":10,"session":"s"}),
            json!({"action":"diff","path":"deleted.txt","staged":true,"maxLines":20,"maxChars":900,"session":"s"}),
        ] {
            let request = serde_json::from_value::<GitArgs>(value.clone())
                .unwrap()
                .into_request();
            assert_eq!(request.name(), "harness_git");
            assert!(!request.is_mutating());
            let restored = request.arguments();
            for (key, expected) in value.as_object().unwrap() {
                assert_eq!(&restored[key], expected);
            }
            assert!(serde_json::from_value::<GitArgs>(restored).is_ok());
        }
    }

    #[test]
    fn unified_git_rejects_missing_or_cross_action_fields_and_decodes_batches() {
        for value in [
            json!({}),
            json!({"action":"diff"}),
            json!({"action":"commit"}),
            json!({"action":"status","path":"x"}),
            json!({"action":"diff","path":"x","limit":5}),
            json!({"action":"status","unknown":true}),
        ] {
            assert!(serde_json::from_value::<GitArgs>(value).is_err());
        }
        let job: BatchJob = serde_json::from_value(
            json!({"tool":"harness_git","arguments":{"action":"diff","path":"x"}}),
        )
        .unwrap();
        let request = job.into_request(Some("session")).unwrap();
        assert_eq!(request.arguments()["action"], "diff");
        assert_eq!(request.arguments()["session"], "session");
        assert!(serde_json::from_value::<BatchTool>(json!("harness_git_status")).is_err());
        let job: BatchJob =
            serde_json::from_value(json!({"tool":"harness_git","arguments":{"action":"diff"}}))
                .unwrap();
        assert_eq!(job.into_request(None).err().unwrap().0, "ARGUMENT");
    }

    #[cfg(feature = "schema")]
    #[test]
    fn git_schema_has_root_object_and_action_specific_branches() {
        let schema = serde_json::to_value(schemars::schema_for!(GitArgs)).unwrap();
        assert_eq!(schema["type"], "object");
        let branches = schema["oneOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        let diff = branches
            .iter()
            .find(|branch| branch["properties"]["action"]["const"] == "diff")
            .unwrap();
        assert!(
            diff["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path"))
        );
    }
}

#[cfg(test)]
mod session_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_args_deserializes_and_dispatches_all_actions() {
        // checkpoint
        let ckpt: SessionArgs = serde_json::from_value(json!({
            "action": "checkpoint",
            "session": "s1",
            "goal": "build session tool",
            "decisions": ["unified"],
            "pending": ["tests"]
        }))
        .unwrap();
        let req = ckpt.into_request();
        assert_eq!(req.name(), "harness_checkpoint");
        assert!(req.is_mutating());

        // tasks update
        let tasks_up: SessionArgs = serde_json::from_value(json!({
            "action": "tasks",
            "session": "s1",
            "phase": "execution",
            "status": "in_progress",
            "note": "implementing session"
        }))
        .unwrap();
        let req = tasks_up.into_request();
        assert_eq!(req.name(), "harness_tasks");
        assert!(req.is_mutating());

        // tasks timeline
        let tasks_tl: SessionArgs = serde_json::from_value(json!({
            "action": "tasks",
            "session": "s1",
            "after": 100,
            "limit": 10
        }))
        .unwrap();
        let req = tasks_tl.into_request();
        assert_eq!(req.name(), "harness_tasks");
        assert!(!req.is_mutating());

        // status
        let status: SessionArgs = serde_json::from_value(json!({
            "action": "status",
            "session": "s1"
        }))
        .unwrap();
        let req = status.into_request();
        assert_eq!(req.name(), "harness_checkpoint");
        assert!(!req.is_mutating());

        // handover
        let handover: SessionArgs = serde_json::from_value(json!({
            "action": "handover",
            "session": "s1",
            "kind": "summary"
        }))
        .unwrap();
        let req = handover.into_request();
        assert_eq!(req.name(), "harness_handover");
        assert!(!req.is_mutating());
    }

    #[test]
    fn session_args_rejects_invalid_actions_or_missing_session() {
        for value in [
            json!({}),
            json!({"action": "unknown", "session": "s"}),
            json!({"action": "checkpoint"}),
            json!({"action": "tasks"}),
            json!({"action": "handover"}),
            json!({"action": "status"}),
        ] {
            assert!(serde_json::from_value::<SessionArgs>(value).is_err());
        }
    }
}

#[cfg(test)]
mod doctor_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn doctor_tool_args_deserializes_and_dispatches_all_actions() {
        // env
        let env: DoctorToolArgs = serde_json::from_value(json!({
            "action": "env",
            "versions": true,
            "connection": true,
            "session": "s1"
        }))
        .unwrap();
        let req = env.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());

        // syntax
        let syntax: DoctorToolArgs = serde_json::from_value(json!({
            "action": "syntax",
            "files": ["src/lib.rs"],
            "maxFiles": 5,
            "session": "s1"
        }))
        .unwrap();
        let req = syntax.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());

        // logs
        let logs: DoctorToolArgs = serde_json::from_value(json!({
            "action": "logs",
            "limit": 10,
            "level": "error",
            "tool": "harness_read",
            "session": "s1"
        }))
        .unwrap();
        let req = logs.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());

        // process
        let proc: DoctorToolArgs = serde_json::from_value(json!({
            "action": "process",
            "pid": 1234,
            "session": "s1"
        }))
        .unwrap();
        let req = proc.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());

        // health
        let health: DoctorToolArgs = serde_json::from_value(json!({
            "action": "health",
            "port": 8080,
            "session": "s1"
        }))
        .unwrap();
        let req = health.into_request();
        assert_eq!(req.name(), "harness_doctor");
        assert!(!req.is_mutating());
    }

    #[test]
    fn doctor_tool_args_rejects_unknown_action() {
        assert!(serde_json::from_value::<DoctorToolArgs>(json!({"action": "repair"})).is_err());
        assert!(serde_json::from_value::<DoctorToolArgs>(json!({})).is_ok());
    }
}

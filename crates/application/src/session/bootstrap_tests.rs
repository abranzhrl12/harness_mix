use super::initial_examples;
use harness_domain::{contracts::{ReadArgs, SearchArgs, SymbolsArgs, VerifyArgs, WordArgs}, tasks::TaskArgs};

#[test]
fn initial_examples_respect_the_selected_tool_profile() {
    for profile in [super::ToolProfile::Full, super::ToolProfile::Investigate, super::ToolProfile::Edit] {
        let examples = initial_examples("example-session", profile);
        for example in examples.as_array().unwrap() {
            assert!(profile.allows(example["tool"].as_str().unwrap()));
        }
    }
}

#[test]
fn background_verification_example_deduplicates_identical_work() {
    let examples = initial_examples("example-session", super::ToolProfile::Full);
    let job = examples
        .as_array()
        .unwrap()
        .iter()
        .find(|example| example["tool"] == "harness_jobs" && example["arguments"]["action"] == "start")
        .unwrap();
    assert_eq!(job["arguments"]["dedupe"], true);
}

#[test]
fn bootstrap_templates_cover_requested_read_tools_with_fillable_arguments() {
    let templates = super::initial_templates("example-session", super::ToolProfile::Full);
    let entries = templates.as_array().unwrap();
    for tool in ["harness_search", "harness_read", "harness_symbols", "harness_batch", "harness_detail"] {
        assert!(entries.iter().any(|entry| entry["tool"] == tool), "missing template for {tool}");
    }
    let detail = entries.iter().find(|entry| entry["tool"] == "harness_detail").unwrap();
    assert_eq!(detail["arguments"]["ref"], "<detail.ref>");
    let batch = entries.iter().find(|entry| entry["tool"] == "harness_batch").unwrap();
    assert_eq!(batch["arguments"]["jobs"].as_array().unwrap().len(), 2);
}

#[test]
fn initial_examples_deserialize_through_public_contracts() {
    for example in initial_examples("example-session", super::ToolProfile::Full).as_array().unwrap() {
        let args = example["arguments"].clone();
        match example["tool"].as_str().unwrap() {
            "harness_read" => { serde_json::from_value::<ReadArgs>(args).unwrap(); }
            "harness_search" => { serde_json::from_value::<SearchArgs>(args).unwrap(); }
            "harness_symbols" => { serde_json::from_value::<SymbolsArgs>(args).unwrap(); }
            "harness_verify" => { serde_json::from_value::<VerifyArgs>(args).unwrap(); }
            "harness_word" => { serde_json::from_value::<WordArgs>(args).unwrap(); }
            "harness_jobs" => { serde_json::from_value::<TaskArgs>(args).unwrap(); }
            other => panic!("example without a contract check: {other}"),
        }
    }
}

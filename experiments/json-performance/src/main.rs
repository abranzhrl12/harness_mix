//! Local CPU measurements, not an IDE/network/LLM latency benchmark.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use harness_domain::{contracts::DataQueryArgs, execution::OperationContext, results::Results};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    hint::black_box,
    sync::Mutex,
    time::{Duration, Instant},
};

// Compile the actual production handler for its in-memory data path. File reads
// are deliberately unavailable here; this is not a substitute filesystem adapter.
mod project {
    pub struct Project;
    impl Project {
        pub fn bytes_with_context(
            &self,
            _: &str,
            _: &harness_domain::execution::OperationContext,
        ) -> Result<Vec<u8>, harness_domain::Failure> {
            Err((
                "UNSUPPORTED",
                "Benchmark only supports in-memory resultRef data",
            ))
        }
    }
}
#[path = "../../../src/adapters/data_query.rs"]
mod data_query;

const SAMPLES: usize = 7;
const SAMPLE_TIME: Duration = Duration::from_millis(50);

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn measure(mut run: impl FnMut()) -> f64 {
    let started = Instant::now();
    let mut iterations = 0u64;
    loop {
        run();
        iterations += 1;
        if started.elapsed() >= SAMPLE_TIME {
            break;
        }
    }
    started.elapsed().as_secs_f64() * 1e6 / iterations as f64
}

// Parsing consumes/mutates simd-json's input. Reset outside the inner timer to
// report parse-only separately from the realistic immutable-input copy variant.
// Both parsers use identical per-operation clock overhead in this measurement.
fn parse_only<T: DeserializeOwned>(input: &[u8], simd: bool) -> f64 {
    let wall = Instant::now();
    let mut elapsed = Duration::ZERO;
    let mut iterations = 0u64;
    let mut buffer = input.to_vec();
    loop {
        buffer.copy_from_slice(input);
        let start = Instant::now();
        if simd {
            black_box(simd_json::serde::from_slice::<T>(black_box(&mut buffer)).unwrap());
        } else {
            black_box(serde_json::from_slice::<T>(black_box(&buffer)).unwrap());
        }
        elapsed += start.elapsed();
        iterations += 1;
        if wall.elapsed() >= SAMPLE_TIME {
            break;
        }
    }
    elapsed.as_secs_f64() * 1e6 / iterations as f64
}

fn payload(target: usize) -> Value {
    let count = 128;
    let docs = "Documentación ñ🙂 con código: \"x\"\n".repeat((target / count / 42).max(1));
    json!({"format":"synthetic-graph","nodes":(0..count).map(|i| json!({
        "id":format!("function:{i:08x}"), "name":format!("handler_{i}"), "kind":"function",
        "path":format!("src/module_{}.ts", i%8), "line":i+1,
        "signature":format!("export function handler_{i}(arg: string): string"),
        "doc":docs, "apiKey":"fixture-secret", "exported":true
    })).collect::<Vec<_>>(),"edges":(1..count).map(|i| json!({"source":format!("function:{:08x}",i-1),"target":format!("function:{i:08x}"),"kind":"calls"})).collect::<Vec<_>>()})
}

fn fixtures() -> Vec<(&'static str, bool, Vec<u8>)> {
    let request = |id, name, args| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}});
    let mut fixtures = vec![
        ("git_request", false, serde_json::to_vec(&request(1,"harness_git",json!({"action":"status"}))).unwrap()),
        ("symbol_request", false, serde_json::to_vec(&request(2,"harness_symbols",json!({"action":"read","path":"src/ñ/example.ts","symbol":"handler","view":"full","session":"12345678-1234-4234-8234-123456789abc"}))).unwrap()),
        ("replace_request", false, serde_json::to_vec(&request(3,"harness_replace",json!({"path":"src/example.ts","hash":"a".repeat(64),"chunks":(0..32).map(|i|json!({"oldText":format!("const value_{i} = \"{}\";\r\n", "old".repeat(16)),"newText":format!("const value_{i} = \"{}\";\r\n", "new".repeat(16))})).collect::<Vec<_>>()}))).unwrap()),
    ];
    for (label, size) in [
        ("inline_32k_request", 32 * 1024),
        ("inline_120k_request", 120 * 1024),
    ] {
        let bytes = serde_json::to_vec(&request(
            4,
            "harness_data_query",
            json!({"data":{"text":"x".repeat(size-1024)},"query":"length(text)"}),
        ))
        .unwrap();
        assert!(bytes.len() <= 128 * 1024);
        fixtures.push((label, false, bytes));
    }
    for (label, size) in [
        ("graph_32k_response", 32 * 1024),
        ("graph_256k_response", 256 * 1024),
        ("graph_768k_response", 768 * 1024),
    ] {
        fixtures.push((label,true,serde_json::to_vec(&json!({"jsonrpc":"2.0","id":5,"result":{"content":[],"isError":false,"structuredContent":{"state":"ok","data":payload(size)}}})).unwrap()));
    }
    fixtures
}

fn parser_case<T: DeserializeOwned + Serialize>(label: &str, kind: &str, input: &[u8]) {
    let expected = serde_json::from_slice::<T>(input).unwrap();
    let got = simd_json::serde::from_slice::<T>(&mut input.to_vec()).unwrap();
    assert_eq!(
        serde_json::to_value(&expected).unwrap(),
        serde_json::to_value(got).unwrap()
    );
    let mut samples: [Vec<f64>; 4] = Default::default();
    for round in 0..SAMPLES {
        for step in 0..4 {
            let variant = (round + step) % 4;
            let time = match variant {
                0 => parse_only::<T>(input, false),
                1 => parse_only::<T>(input, true),
                2 => measure(|| {
                    let mut copy = black_box(input).to_vec();
                    black_box(simd_json::serde::from_slice::<T>(&mut copy).unwrap());
                }),
                _ => measure(|| {
                    black_box(serde_json::to_vec(black_box(&expected)).unwrap());
                }),
            };
            samples[variant].push(time);
        }
    }
    let [standard, simd, copy, encode] = samples.map(median);
    println!(
        "PARSE,{label},{kind},{},{standard:.3},{simd:.3},{copy:.3},{encode:.3},{:.3},{:.3}",
        input.len(),
        standard / simd,
        standard / copy
    );
}

fn parser_bench() {
    println!(
        "PARSE,case,representation,bytes,serde_parse_us,simd_parse_us,simd_copy_parse_us,serde_encode_us,simd_parse_speedup,simd_copy_speedup"
    );
    for (label, response, input) in fixtures() {
        parser_case::<Value>(label, "Value", &input);
        if response {
            parser_case::<rmcp::model::ServerJsonRpcMessage>(label, "MCP_typed", &input);
        } else {
            parser_case::<rmcp::model::ClientJsonRpcMessage>(label, "MCP_typed", &input);
        }
    }
}

fn project(value: Value, query: &str) -> Value {
    let args = DataQueryArgs {
        path: None,
        data: Some(value),
        result_ref: None,
        query: query.into(),
        max_chars: Some(32000),
        session: None,
    };
    data_query::handle_data_query(
        &project::Project,
        &args,
        &OperationContext::default(),
    )
    .unwrap()
}

fn cache_bench() {
    println!(
        "CACHE,case,bytes,serde_retrieve_us,simd_retrieve_us,clone_retrieve_us,serde_query_us,simd_query_us,clone_query_us,actual_save_us,prototype_save_us"
    );
    for (label, size) in [
        ("graph_16k", 16 * 1024),
        ("graph_128k", 128 * 1024),
        ("graph_768k", 768 * 1024),
    ] {
        let value = payload(size);
        let serialized = serde_json::to_vec(&value).unwrap();
        assert!(serialized.len() < 1024 * 1024);
        let mut actual = Results::default();
        let detail = actual.save(&value, true).unwrap();
        let id = detail["ref"].as_str().unwrap().to_owned();
        let actual = Mutex::new(actual);
        let saving = Mutex::new(Results::default());
        let simd_stored = Mutex::new(serialized.clone());
        let retained = Mutex::new(value.clone());
        let query = "nodes[?kind == 'function'] | [0:8].{name:name,path:path}";
        let retrieve = |variant| -> Value {
            match variant {
                0 => actual.lock().unwrap().value(&id).unwrap(),
                1 => {
                    simd_json::serde::from_slice::<Value>(&mut simd_stored.lock().unwrap().clone())
                        .unwrap()
                }
                _ => retained.lock().unwrap().clone(),
            }
        };
        let expected = project(retrieve(0), query);
        for variant in [1, 2] {
            assert_eq!(project(retrieve(variant), query), expected);
        }
        assert_eq!(
            retained.lock().unwrap()["nodes"][0]["apiKey"],
            "fixture-secret"
        );
        assert_eq!(
            project(retrieve(0), "nodes[0].apiKey")["result"],
            "[REDACTED]"
        );
        let mut samples: [Vec<f64>; 8] = Default::default();
        for round in 0..SAMPLES {
            for step in 0..8 {
                let variant = (round + step) % 8;
                samples[variant].push(measure(|| {
                    match variant {
                        0..=2 => {
                            black_box(retrieve(black_box(variant)));
                        }
                        3..=5 => {
                            black_box(project(retrieve(black_box(variant - 3)), black_box(query)));
                        }
                        6 => {
                            black_box(
                                saving
                                    .lock()
                                    .unwrap()
                                    .save(black_box(&value), true)
                                    .unwrap(),
                            );
                        }
                        _ => {
                            // Prototype keeps both text and Value: extra clone/memory, not a runtime patch.
                            let detail = saving
                                .lock()
                                .unwrap()
                                .save(black_box(&value), true)
                                .unwrap();
                            black_box((detail, black_box(&value).clone()));
                        }
                    }
                }));
            }
        }
        let [a, b, c, d, e, f, g, h] = samples.map(median);
        println!(
            "CACHE,{label},{},{a:.3},{b:.3},{c:.3},{d:.3},{e:.3},{f:.3},{g:.3},{h:.3}",
            serialized.len()
        );
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("parse") => parser_bench(),
        Some("cache") => cache_bench(),
        _ => {
            parser_bench();
            cache_bench();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_mcp_messages_are_valid_and_parse_equivalently() {
        for (_, response, input) in fixtures() {
            let expected: Value = serde_json::from_slice(&input).unwrap();
            assert_eq!(
                simd_json::serde::from_slice::<Value>(&mut input.clone()).unwrap(),
                expected
            );
            if response {
                serde_json::from_slice::<rmcp::model::ServerJsonRpcMessage>(&input).unwrap();
            } else {
                serde_json::from_slice::<rmcp::model::ClientJsonRpcMessage>(&input).unwrap();
            }
        }
    }
    #[test]
    fn retained_values_do_not_bypass_redaction_or_mutate_shared_results() {
        let value = payload(16 * 1024);
        assert_eq!(
            project(value.clone(), "nodes[0].apiKey")["result"],
            "[REDACTED]"
        );
        assert_eq!(value["nodes"][0]["apiKey"], "fixture-secret");
    }
    #[test]
    fn parsers_agree_on_common_invalid_inputs_and_representative_numbers() {
        for input in [b"{\"x\":}".as_slice(), b"{\"x\":\"\xff\"}", b"[1,]"] {
            assert!(serde_json::from_slice::<Value>(input).is_err());
            let parsed = simd_json::serde::from_slice::<Value>(&mut input.to_vec());
            assert!(parsed.is_err(), "simd-json accepted {input:?}: {parsed:?}");
        }
        for input in [
            br#"{"i":18446744073709551615,"n":-9223372036854775808}"#.as_slice(),
            r#"{"text":"ñ🙂\n\"","f":1.25,"list":[null,true,false]}"#.as_bytes(),
        ] {
            let expected: Value = serde_json::from_slice(input).unwrap();
            assert_eq!(
                simd_json::serde::from_slice::<Value>(&mut input.to_vec()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn pinned_parsers_differ_on_unpaired_surrogate() {
        let input = br#"{"x":"\uD800"}"#;
        assert!(serde_json::from_slice::<Value>(input).is_err());
        // Record the observed compatibility difference instead of hiding it in timings.
        assert_eq!(
            simd_json::serde::from_slice::<Value>(&mut input.to_vec()).unwrap(),
            json!({"x":"\0"})
        );
    }
}

use ra_ap_syntax::{Edition, SourceFile};
use std::{hint::black_box, time::Instant};

fn generated(functions: usize) -> String {
    let mut source = String::new();
    for i in 0..functions {
        source.push_str(&format!(
            "/// Documented item {i}.\r\n#[allow(dead_code)]\r\npub fn item_{i}(arg: Option<u32>) -> u32 {{\r\n    let label = \"emoji 😀\";\r\n    arg.unwrap_or(label.len() as u32)\r\n}}\r\n\r\n"
        ));
    }
    source
}

fn syn_input(source: &str) -> String {
    let body = source.strip_prefix('\u{feff}').unwrap_or(source);
    if body.starts_with("#!") && !body.starts_with("#![") {
        let end = body.find('\n').unwrap_or(body.len());
        format!("{}{}", " ".repeat(end), &body[end..])
    } else {
        body.to_owned()
    }
}

fn run_syn(source: &str) -> (bool, usize) {
    let parsed = syn::parse_file(black_box(source));
    let result = match parsed {
        Ok(file) => (true, file.items.len()),
        Err(_) => (false, 0),
    };
    proc_macro2::extra::invalidate_current_thread_spans();
    result
}

fn run_syn_policy(source: &str) -> (bool, usize) {
    if source.len() > 256 * 1024 { return (false, 0); }
    let mut depth = 0usize;
    for byte in source.as_bytes() {
        if matches!(byte, b'(' | b'{' | b'[') { depth += 1; }
        else if matches!(byte, b')' | b'}' | b']') { depth = depth.saturating_sub(1); }
        if depth > 64 { return (false, 0); }
    }
    let body = source.strip_prefix('\u{feff}').unwrap_or(source);
    let prepared = if body.starts_with("#!") && !body.starts_with("#![") {
        let end = body.find('\n').unwrap_or(body.len());
        format!("{}{}", " ".repeat(end), &body[end..])
    } else { body.to_owned() };
    run_syn(&prepared)
}

fn run_ra(source: &str) -> (bool, usize) {
    let parsed = SourceFile::parse(black_box(source), Edition::Edition2024);
    let errors = parsed.errors().len();
    (errors == 0, errors)
}

fn ra_tree_nodes(source: &str) -> usize {
    SourceFile::parse(source, Edition::Edition2024).syntax_node().descendants().count()
}

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn measure(mut parse: impl FnMut(&str) -> (bool, usize), source: &str) -> (u128, bool, usize) {
    for _ in 0..3 { black_box(parse(source)); }
    let mut samples = Vec::new();
    let mut observed = (false, 0);
    for _ in 0..9 {
        let started = Instant::now();
        for _ in 0..20 { observed = black_box(parse(black_box(source))); }
        samples.push(started.elapsed().as_nanos() / 20);
    }
    (median(samples), observed.0, observed.1)
}

fn report(name: &str, source: &str) {
    let prepared = syn_input(source);
    let (syn_ns, syn_valid, syn_items) = measure(run_syn, &prepared);
    let (syn_policy_ns, syn_policy_valid, _) = measure(run_syn_policy, source);
    let (ra_ns, ra_valid, ra_errors) = measure(run_ra, &prepared);
    let ra_nodes = ra_tree_nodes(&prepared);
    println!("{{\"case\":\"{name}\",\"sourceBytes\":{},\"commonInputBytes\":{},\"synParseDropResetMedianNs\":{syn_ns},\"synValid\":{syn_valid},\"synTopLevelItems\":{syn_items},\"synWithPolicyMedianNs\":{syn_policy_ns},\"synWithPolicyValid\":{syn_policy_valid},\"raParseAndErrorsMedianNs\":{ra_ns},\"raValid\":{ra_valid},\"raErrorCount\":{ra_errors},\"raSyntaxNodesUntimed\":{ra_nodes}}}", source.len(), prepared.len());
}

fn main() {
    report("small_valid", &generated(16));
    report("medium_valid", &generated(240));
    report("large_valid", &generated(1180));
    for (name, functions) in [("small_invalid", 16), ("medium_invalid", 240), ("large_invalid", 1180)] {
        let mut source = generated(functions);
        source.push_str("pub fn broken( {\n");
        report(name, &source);
    }
    let edge = "\u{feff}#!/usr/bin/env rustx\r\n//! module docs 😀\r\n/// item docs\r\npub fn edge() {}\r\n";
    report("bom_shebang_docs_utf16_crlf", edge);
}

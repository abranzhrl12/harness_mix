use aho_corasick::AhoCorasickBuilder;
use regex::RegexBuilder;
use std::{hint::black_box, time::{Duration, Instant}};

const REPEATS: usize = 9;
const LINES: usize = 16_384;
const ROUNDS: usize = 8;

fn query_terms(count: usize) -> Vec<String> {
    (0..count).map(|i| format!("term{i:02}")).collect()
}

fn corpus(terms: &[String], hit_percent: usize) -> Vec<String> {
    (0..LINES).map(|i| {
        if i % 100 < hit_percent {
            format!("fn handler_{i}() {{ let payload = {}; process_request(payload); }}", terms[i % terms.len()])
        } else {
            format!("fn handler_{i}() {{ let payload = unrelated_{i}; process_request(payload); }}")
        }
    }).collect()
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn main() {
    println!("search matcher microbenchmark; lines={LINES}; rounds={ROUNDS}; samples={REPEATS}");
    println!("regex=regex::RegexBuilder (Unicode case-insensitive); aho=aho-corasick ASCII case-insensitive");
    println!("matcher compilation excluded; ns/line, median of {REPEATS} full scans");
    for term_count in [2, 8, 16] {
        let terms = query_terms(term_count);
        let pattern = terms.iter().map(|term| regex::escape(term)).collect::<Vec<_>>().join("|");
        let regex = RegexBuilder::new(&pattern).case_insensitive(true).build().expect("valid benchmark regex");
        let aho = AhoCorasickBuilder::new().ascii_case_insensitive(true).build(&terms).expect("valid benchmark patterns");
        for hit_percent in [0, 10, 50] {
            let lines = corpus(&terms, hit_percent);
            let regex_matches: Vec<bool> = lines.iter().map(|line| regex.is_match(line)).collect();
            let aho_matches: Vec<bool> = lines.iter().map(|line| aho.is_match(line)).collect();
            assert_eq!(regex_matches, aho_matches, "ASCII semantic mismatch");
            let mut regex_samples = Vec::with_capacity(REPEATS);
            let mut aho_samples = Vec::with_capacity(REPEATS);
            for sample in 0..REPEATS {
                let (regex_first, aho_first) = if sample % 2 == 0 { (true, false) } else { (false, true) };
                for is_regex in [regex_first, aho_first] {
                    let start = Instant::now();
                    for _ in 0..ROUNDS {
                        let count = if is_regex {
                            lines.iter().filter(|line| regex.is_match(black_box(line))).count()
                        } else {
                            lines.iter().filter(|line| aho.is_match(black_box(line))).count()
                        };
                        black_box(count);
                    }
                    if is_regex { regex_samples.push(start.elapsed()); } else { aho_samples.push(start.elapsed()); }
                }
            }
            let regex_ns = median(regex_samples).as_nanos() as f64 / (LINES * ROUNDS) as f64;
            let aho_ns = median(aho_samples).as_nanos() as f64 / (LINES * ROUNDS) as f64;
            println!("terms={term_count:2} hits={hit_percent:2}% regex={regex_ns:8.2} ns/line aho={aho_ns:8.2} ns/line speedup={:.2}x", regex_ns / aho_ns);
        }
    }
    let unicode_regex = RegexBuilder::new("café").case_insensitive(true).build().unwrap();
    let unicode_aho = AhoCorasickBuilder::new().ascii_case_insensitive(true).build(["café"]).unwrap();
    let unicode_probe = "CAFÉ";
    println!("unicode probe query=café text={unicode_probe:?} regex={} aho={}", unicode_regex.is_match(unicode_probe), unicode_aho.is_match(unicode_probe));
}

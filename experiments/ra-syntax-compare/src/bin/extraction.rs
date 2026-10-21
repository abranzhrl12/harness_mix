//! Timings include production symbol projection, not only parser construction.
use ra_syntax_compare::{baseline, candidate};
use std::{hint::black_box, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let engine = args.get(1).map(String::as_str).unwrap_or("syn");
    let count: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(128);
    let source = (0..count).map(|i| format!(
        "/// Documented 😀 {i}\r\n#[allow(dead_code)]\r\npub fn item_{i}(x:u8) -> u8 {{ x }}\r\n"
    )).collect::<String>();
    if engine == "check" {
        assert_eq!(ra_syntax_compare::snapshot(&baseline(&source).unwrap()),
            ra_syntax_compare::snapshot(&candidate::collect(&source).unwrap()));
        println!("symbol equality passed: {count}");
        return;
    }
    let extract = |text: &str| match engine {
        "syn" => baseline(text),
        "ra" => candidate::collect(text),
        _ => panic!("expected syn or ra"),
    };
    for _ in 0..3 { black_box(extract(black_box(&source)).unwrap()); }
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        for _ in 0..5 { black_box(extract(black_box(&source)).unwrap()); }
        samples.push(start.elapsed().as_nanos() / 5);
    }
    samples.sort_unstable();
    println!("{}", serde_json::json!({
        "engine":engine,"symbols":count,"sourceBytes":source.len(),
        "sourceHash":harness_domain::hash(source.as_bytes()),
        "medianNsPerExtraction":samples[4],"samplesNsPerExtraction":samples,
        "warmups":3,"samples":9,"operationsPerSample":5,
        "symbolEqualityGate":"run separately with check to avoid contaminating process memory"
    }));
}

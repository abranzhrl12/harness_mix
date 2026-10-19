//! Release-mode comparison; no filesystem publication or syntax parser is timed.
use contracts::{ReplaceArgs, ReplaceChunk};
pub use harness_domain::{Failure, contracts, hash};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
mod baseline;
#[path = "../../../crates/domain/src/editing.rs"]
mod current;
mod candidate {
    include!(concat!(env!("OUT_DIR"), "/candidate.rs"));
}

type Prepare = fn(&[u8], &ReplaceArgs) -> Result<String, Failure>;

fn fixture(target_bytes: usize, count: usize, eol: &str) -> (String, ReplaceArgs) {
    let lines = (target_bytes / 56).max(count);
    let mut source = String::with_capacity(target_bytes + 1024);
    let mut rows = Vec::with_capacity(lines);
    for index in 0..lines {
        let ending = match eol {
            "mixed" if index % 3 == 0 => "\n",
            "lf" => "\n",
            _ => "\r\n",
        };
        let row = format!("let item_{index:06} = \"payload_ñ🙂\"; // padding padding{ending}");
        source.push_str(&row);
        rows.push(row);
    }
    let chunks = (0..count)
        .rev()
        .map(|index| {
            let row = &rows[(index + 1) * (lines - 1) / count];
            ReplaceChunk {
                old_text: row.replace("\r\n", "\n"),
                new_text: row.replace("payload", "updated").replace("\r\n", "\n"),
            }
        })
        .collect();
    let args = ReplaceArgs {
        path: "fixture.txt".into(),
        hash: hash(source.as_bytes()),
        chunks: Some(chunks),
        old_text: String::new(),
        new_text: String::new(),
        allow_unvalidated_text: false,
        session: None,
    };
    (source, args)
}

fn measure(mut run: impl FnMut()) -> f64 {
    let start = Instant::now();
    let mut iterations = 0u64;
    loop {
        run();
        iterations += 1;
        if start.elapsed() >= Duration::from_millis(40) {
            break;
        }
    }
    start.elapsed().as_secs_f64() * 1e6 / iterations as f64
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn editing_bench() {
    println!(
        "EDIT,eol,bytes,chunks,baseline_us,one_pass_us,memchr_us,mapper_speedup,memchr_speedup"
    );
    let functions: [Prepare; 3] = [baseline::prepare, current::prepare, candidate::prepare];
    for eol in ["lf", "crlf", "mixed"] {
        for size in [16 * 1024, 128 * 1024, 1024 * 1024] {
            for count in [1, 16, 128] {
                let (source, args) = fixture(size, count, eol);
                let expected = baseline::prepare(source.as_bytes(), &args).unwrap();
                assert_eq!(
                    current::prepare(source.as_bytes(), &args).unwrap(),
                    expected
                );
                assert_eq!(
                    candidate::prepare(source.as_bytes(), &args).unwrap(),
                    expected
                );
                let mut samples: [Vec<f64>; 3] = Default::default();
                // Rotate order between samples to reduce systematic warm-cache/order bias.
                for round in 0..5 {
                    for step in 0..3 {
                        let index = (round + step) % 3;
                        samples[index].push(measure(|| {
                            black_box(
                                functions[index](black_box(source.as_bytes()), black_box(&args))
                                    .unwrap(),
                            );
                        }));
                    }
                }
                let [old, new, memchr] = samples.map(median);
                println!(
                    "EDIT,{eol},{},{count},{old:.3},{new:.3},{memchr:.3},{:.3},{:.3}",
                    source.len(),
                    old / new,
                    new / memchr
                );
            }
        }
    }
}

fn search_bench() {
    println!("SEARCH,case,bytes,std_us,memchr_reused_us,speedup");
    let (source, _) = fixture(1024 * 1024, 1, "crlf");
    for query in [
        "absent_identifier",
        "item_000001",
        "payload_ñ🙂",
        "x",
        "padding padding\r\n",
    ] {
        let finder = memchr::memmem::Finder::new(query.as_bytes());
        let standard = || {
            let query = black_box(query);
            black_box(source.as_str())
                .split_inclusive('\n')
                .filter(|line| line.contains(query))
                .count()
        };
        let fast = || {
            let finder = black_box(&finder);
            black_box(source.as_str())
                .split_inclusive('\n')
                .filter(|line| finder.find(line.as_bytes()).is_some())
                .count()
        };
        assert_eq!(standard(), fast());
        let mut samples: [Vec<f64>; 2] = Default::default();
        for round in 0..5 {
            for step in 0..2 {
                let index = (round + step) % 2;
                samples[index].push(measure(|| {
                    black_box(if index == 0 { standard() } else { fast() });
                }));
            }
        }
        let [standard, fast] = samples.map(median);
        println!(
            "SEARCH,{query:?},{},{standard:.3},{fast:.3},{:.3}",
            source.len(),
            standard / fast
        );
    }
}

fn main() {
    editing_bench();
    search_bench();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_preserves_overlapping_match_rejection() {
        let args = ReplaceArgs {
            path: "x.txt".into(),
            hash: hash(b"aaa"),
            chunks: None,
            old_text: "aa".into(),
            new_text: "x".into(),
            allow_unvalidated_text: false,
            session: None,
        };
        for prepare in [baseline::prepare, current::prepare, candidate::prepare] {
            assert_eq!(prepare(b"aaa", &args).unwrap_err().0, "MATCH");
        }
    }
    #[test]
    fn fixtures_are_identical_across_all_three_implementations() {
        for eol in ["lf", "crlf", "mixed"] {
            for count in [1, 16, 128] {
                let (source, args) = fixture(16 * 1024, count, eol);
                let expected = baseline::prepare(source.as_bytes(), &args).unwrap();
                assert_eq!(
                    current::prepare(source.as_bytes(), &args).unwrap(),
                    expected
                );
                assert_eq!(
                    candidate::prepare(source.as_bytes(), &args).unwrap(),
                    expected
                );
            }
        }
    }
}

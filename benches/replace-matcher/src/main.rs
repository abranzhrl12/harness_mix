use aho_corasick::{AhoCorasickBuilder, MatchKind};
use harness_domain::{
    contracts::{ReplaceArgs, ReplaceChunk},
    editing::prepare,
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exact {
    Missing,
    Unique(usize, usize),
    Ambiguous,
}

fn exact_reference(hay: &str, needle: &str) -> Exact {
    match hay.find(needle) {
        None => Exact::Missing,
        Some(first) => {
            let next = first + hay[first..].chars().next().unwrap_or(' ').len_utf8();
            if hay[next..].contains(needle) {
                Exact::Ambiguous
            } else {
                Exact::Unique(first, first + needle.len())
            }
        }
    }
}

// Matcher build is intentionally inside this function: callers can time build+scan.
fn aho_exact(hay: &str, needles: &[String]) -> Vec<Exact> {
    let mut out = vec![Exact::Missing; needles.len()];
    if needles.is_empty() {
        return out;
    }
    let ac = AhoCorasickBuilder::new()
        .match_kind(MatchKind::Standard)
        .build(needles)
        .unwrap();
    let mut counts = vec![0usize; needles.len()];
    let mut ranges = vec![(0usize, 0usize); needles.len()];
    for m in ac.find_overlapping_iter(hay) {
        let id = m.pattern().as_usize();
        counts[id] += 1;
        ranges[id] = (m.start(), m.end());
    }
    for i in 0..needles.len() {
        out[i] = match counts[i] {
            0 => Exact::Missing,
            1 => Exact::Unique(ranges[i].0, ranges[i].1),
            _ => Exact::Ambiguous,
        };
    }
    out
}

fn apply_exact(hay: &str, chunks: &[ReplaceChunk], found: &[Exact]) -> Option<String> {
    if found.len() != chunks.len() {
        return None;
    }
    let mut edits: Vec<_> = found
        .iter()
        .zip(chunks)
        .map(|(m, c)| match m {
            Exact::Unique(start, end) => Some((*start, *end, c.new_text.as_str())),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    edits.sort_unstable_by_key(|edit| edit.0);
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return None;
    }
    let added: usize = edits.iter().map(|(_, _, value)| value.len()).sum();
    let mut output = String::with_capacity(hay.len().saturating_add(added));
    let mut cursor = 0;
    for (start, end, replacement) in edits {
        output.push_str(&hay[cursor..start]);
        output.push_str(replacement);
        cursor = end;
    }
    output.push_str(&hay[cursor..]);
    Some(output)
}

fn args(chunks: &[ReplaceChunk]) -> ReplaceArgs {
    ReplaceArgs {
        chunks: Some(chunks.to_vec()),
        ..ReplaceArgs::default()
    }
}

fn corpus(size: usize, count: usize) -> (String, Vec<ReplaceChunk>) {
    let mut rows = Vec::new();
    let mut text_len = 0;
    let mut i = 0;
    while text_len < size {
        let needle = format!("needle_{i:08x}");
        let row = format!("r{i:05} {needle}\n");
        if text_len + row.len() > size {
            break;
        }
        text_len += row.len();
        rows.push(row);
        i += 1;
    }
    assert!(
        count <= rows.len(),
        "requested more unique chunks than file rows"
    );
    let text = rows.concat();
    let chunks = (0..count)
        .map(|slot| {
            let row = slot * rows.len() / count;
            let old_text = format!("needle_{row:08x}");
            ReplaceChunk {
                old_text,
                new_text: format!("changed_{slot:03x}"),
            }
        })
        .collect();
    (text, chunks)
}

fn measure(mut f: impl FnMut(), iterations: usize) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    start.elapsed() / iterations as u32
}

fn main() {
    println!(
        "replace matcher benchmark; rustc={}, target={}",
        option_env!("RUSTC_VERSION").unwrap_or("record rustc --version separately"),
        std::env::consts::ARCH
    );
    println!("method,bytes,chunks,ns/op");
    for &target_bytes in &[4096usize, 64 * 1024, 512 * 1024, 2 * 1024 * 1024] {
        for &count in &[1usize, 2, 8, 16, 64, 128] {
            let (hay, chunks) = corpus(target_bytes, count);
            let bytes = hay.len();
            let request = args(&chunks);
            let needles: Vec<String> = chunks.iter().map(|c| c.old_text.clone()).collect();
            let expected: Vec<_> = needles.iter().map(|n| exact_reference(&hay, n)).collect();
            let got = aho_exact(&hay, &needles);
            assert_eq!(
                got, expected,
                "matcher equivalence bytes={bytes} chunks={count}"
            );
            let production = prepare(hay.as_bytes(), &request).unwrap();
            let candidate = apply_exact(&hay, &chunks, &got).unwrap();
            assert_eq!(
                candidate.as_bytes(),
                production.as_bytes(),
                "output equivalence bytes={bytes} chunks={count}"
            );

            let loops = if bytes >= 512 * 1024 {
                5
            } else if bytes >= 64 * 1024 {
                20
            } else {
                100
            };
            let repeated = measure(
                || {
                    let values: Vec<_> = needles
                        .iter()
                        .map(|n| black_box(exact_reference(black_box(&hay), black_box(n))))
                        .collect();
                    black_box(values);
                },
                loops,
            );
            let aho_build_scan = measure(
                || {
                    black_box(aho_exact(black_box(&hay), black_box(&needles)));
                },
                loops,
            );
            let aho_exact_apply = measure(
                || {
                    let found = aho_exact(black_box(&hay), black_box(&needles));
                    black_box(apply_exact(
                        black_box(&hay),
                        black_box(&chunks),
                        black_box(&found),
                    ));
                },
                loops,
            );
            let full_prepare = measure(
                || {
                    let result = prepare(black_box(hay.as_bytes()), black_box(&request));
                    let _ = black_box(result);
                },
                loops,
            );
            println!("repeated_find,{bytes},{count},{}", repeated.as_nanos());
            println!(
                "aho_build_plus_scan,{bytes},{count},{}",
                aho_build_scan.as_nanos()
            );
            println!(
                "aho_exact_build_scan_apply,{bytes},{count},{}",
                aho_exact_apply.as_nanos()
            );
            println!(
                "prepare_current,{bytes},{count},{}",
                full_prepare.as_nanos()
            );
        }
    }
    edge_checks();
}

fn edge_checks() {
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("aaaa", vec!["aa"]),            // overlapping occurrences count as ambiguous
        ("abc", vec!["a", "ab", "abc"]), // substring patterns
        ("x λ x λ", vec!["λ"]),          // UTF-8 boundaries
        ("a\r\nb\r\n", vec!["a\nb"]),    // normalized CRLF case is checked by production prepare
        ("same same", vec!["same", "same"]), // duplicate patterns retain per-input identity
        ("single", vec!["single", "single"]), // duplicate patterns at one match
        ("aaa", vec!["aa", "a"]),        // cross-pattern overlapping matches
        ("alpha\nbeta", vec!["alpha\ngamma"]), // missing exact path/fallback/error
    ];
    for (hay, pats) in cases {
        let needles: Vec<String> = pats.iter().map(|s| s.to_string()).collect();
        let expected: Vec<_> = needles.iter().map(|n| exact_reference(hay, n)).collect();
        assert_eq!(aho_exact(hay, &needles), expected, "edge case {hay:?}");
    }
    // Production behavior is the oracle for fallback, normalized CRLF, and overlaps.
    for (hay, old, expected_code) in [
        ("alpha\nbeta\n", "  alpha\n  beta", "OK"),
        ("a\r\nb\r\n", "a\nb", "OK"),
        ("same same", "same", "MATCH"),
        ("aaaa", "aa", "MATCH"),
        ("nothing here", "absent", "MATCH"),
    ] {
        let chunk = ReplaceChunk {
            old_text: old.into(),
            new_text: "REPLACED".into(),
        };
        let result = prepare(hay.as_bytes(), &args(&[chunk]));
        let code = match &result {
            Ok(_) => "OK",
            Err(error) => error.0,
        };
        assert_eq!(code, expected_code);
    }
    println!("edge_checks=passed (production fallback/CRLF/ambiguous/missing oracle)");
}

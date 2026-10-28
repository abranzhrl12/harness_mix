use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{Searcher, SearcherBuilder};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn builder() -> RegexMatcherBuilder {
    let mut b = RegexMatcherBuilder::new();
    b.case_insensitive(true).line_terminator(Some(b'\n'));
    b
}

fn baseline(terms: &[String]) -> RegexMatcher {
    // Terms here are alphanumeric, exactly the safe subset in production fallback.
    builder().build(&terms.join("|")).unwrap()
}

fn literals(terms: &[String]) -> RegexMatcher {
    builder().fixed_strings(true).build_literals(terms).unwrap()
}

fn aho(terms: &[String]) -> AhoCorasick {
    AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build(terms)
        .unwrap()
}

fn searcher() -> Searcher {
    SearcherBuilder::new()
        .line_number(true)
        .bom_sniffing(false)
        .build()
}

fn grep_lines(searcher: &mut Searcher, matcher: &RegexMatcher, file: &[u8]) -> Vec<u64> {
    let mut rows = Vec::new();
    searcher
        .search_slice(
            matcher,
            file,
            grep_searcher::sinks::UTF8(|line, _| {
                rows.push(line);
                Ok(true)
            }),
        )
        .unwrap();
    rows
}

fn aho_lines(matcher: &AhoCorasick, file: &[u8]) -> Vec<u64> {
    let (mut offset, mut number) = (0, 1_u64);
    let mut rows = Vec::new();
    while let Some(found) = matcher.find(&file[offset..]) {
        let start = offset + found.start();
        number += memchr::memchr_iter(b'\n', &file[offset..start]).count() as u64;
        rows.push(number);
        offset = memchr::memchr(b'\n', &file[start..]).map_or(file.len(), |next| start + next + 1);
        number += 1;
    }
    rows
}

fn hybrid_lines(
    searcher: &mut Searcher,
    regex: &RegexMatcher,
    aho: &AhoCorasick,
    ascii_terms: bool,
    file: &[u8],
) -> Vec<u64> {
    // ASCII query alone is insufficient: s also matches long-s in Unicode mode.
    if ascii_terms && file.is_ascii() {
        aho_lines(aho, file)
    } else {
        grep_lines(searcher, regex, file)
    }
}

fn median(mut samples: Vec<Duration>) -> f64 {
    samples.sort_unstable();
    samples[samples.len() / 2].as_secs_f64() * 1e6
}

fn time(mut operation: impl FnMut(), rounds: usize) -> f64 {
    operation();
    median(
        (0..9)
            .map(|_| {
                let start = Instant::now();
                for _ in 0..rounds {
                    operation();
                }
                start.elapsed() / rounds as u32
            })
            .collect(),
    )
}

fn corpus(terms: &[String], hit_percent: usize, unicode: bool) -> Vec<Vec<u8>> {
    (0..64)
        .map(|file| {
            (0..256)
                .map(|line| {
                    let id = file * 256 + line;
                    let prefix = if unicode {
                        "// acción café "
                    } else {
                        "// source "
                    };
                    if id % 100 < hit_percent {
                        format!(
                            "{prefix}fn handler_{id}() {{ {}(); }}\r\n",
                            terms[id % terms.len()]
                        )
                    } else {
                        format!("{prefix}fn handler_{id}() {{ unrelated_value(); }}\r\n")
                    }
                })
                .collect::<String>()
                .into_bytes()
        })
        .collect()
}

fn check_edges() {
    let fixtures: &[(&[&str], &str)] = &[
        (&["s", "x"], "ſ\ns\nS\nx\n"),
        (&["k", "x"], "K\nk\nK\nx\n"),
        (&["café", "niño"], "CAFÉ\r\nniño\r\nnone\n"),
        (&["term", "term1"], "term1 term term\r\n\r\nterm\n"),
        (&["missing", "absent"], "first\nlast"),
        (&["term", "other"], "term\nlast term"),
        (&["term", "other"], "no hit\n"),
        (&["term", "other"], ""),
    ];
    let mut s = searcher();
    for (patterns, text) in fixtures {
        let terms = patterns.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let regex = baseline(&terms);
        let direct = literals(&terms);
        let ac = aho(&terms);
        let expected = grep_lines(&mut s, &regex, text.as_bytes());
        assert_eq!(expected, grep_lines(&mut s, &direct, text.as_bytes()));
        assert_eq!(
            expected,
            hybrid_lines(
                &mut s,
                &regex,
                &ac,
                terms.iter().all(|t| t.is_ascii()),
                text.as_bytes()
            )
        );
    }
    let regex = baseline(&["s".into()]);
    let ac = aho(&["s".into()]);
    assert_eq!(grep_lines(&mut s, &regex, "ſ\n".as_bytes()), vec![1]);
    assert!(aho_lines(&ac, "ſ\n".as_bytes()).is_empty());
    println!(
        "edge_checks=PASS; ASCII-only query s against Unicode long-s: regex matches, bare Aho misses"
    );
}

fn scan_bench(label: &str, terms: &[String], files: &[Vec<u8>]) {
    let regex = baseline(terms);
    let direct = literals(terms);
    let ac = aho(terms);
    let ascii = terms.iter().all(|t| t.is_ascii());
    let mut s = searcher();
    for file in files {
        let expected = grep_lines(&mut s, &regex, file);
        assert_eq!(expected, grep_lines(&mut s, &direct, file), "{label}");
        assert_eq!(
            expected,
            hybrid_lines(&mut s, &regex, &ac, ascii, file),
            "{label}"
        );
    }
    let mut samples = [Vec::new(), Vec::new(), Vec::new()];
    for sample in 0..9 {
        for index in 0..3 {
            let engine = (index + sample) % 3;
            let start = Instant::now();
            for _ in 0..4 {
                for file in files {
                    let rows = match engine {
                        0 => grep_lines(&mut s, &regex, black_box(file)),
                        1 => grep_lines(&mut s, &direct, black_box(file)),
                        _ => hybrid_lines(&mut s, &regex, &ac, ascii, black_box(file)),
                    };
                    black_box(rows);
                }
            }
            samples[engine].push(start.elapsed() / 4);
        }
    }
    let [a, b, c] = samples.map(median);
    println!(
        "scan {label} bytes={} terms={} grep_us={a:.2} literals_us={b:.2} hybrid_us={c:.2} hybrid_speedup={:.3}",
        files.iter().map(Vec::len).sum::<usize>(),
        terms.len(),
        a / c
    );
}

fn double_pass(
    files: &[Vec<u8>],
    exact: &RegexMatcher,
    fallback: &RegexMatcher,
) -> Vec<(usize, u64)> {
    let mut s = searcher();
    let mut exact_rows = Vec::new();
    for (id, file) in files.iter().enumerate() {
        let read = black_box(file.clone());
        exact_rows.extend(
            grep_lines(&mut s, exact, &read)
                .into_iter()
                .map(|line| (id, line)),
        );
    }
    if !exact_rows.is_empty() {
        return exact_rows;
    }
    let mut rows = Vec::new();
    for (id, file) in files.iter().enumerate() {
        let read = black_box(file.clone());
        rows.extend(
            grep_lines(&mut s, fallback, &read)
                .into_iter()
                .map(|line| (id, line)),
        );
    }
    rows
}

fn same_buffer(
    files: &[Vec<u8>],
    exact: &RegexMatcher,
    fallback: &RegexMatcher,
) -> Vec<(usize, u64)> {
    let mut s = searcher();
    let mut exact_rows = Vec::new();
    let mut candidates = Vec::new();
    for (id, file) in files.iter().enumerate() {
        let read = black_box(file.clone());
        exact_rows.extend(
            grep_lines(&mut s, exact, &read)
                .into_iter()
                .map(|line| (id, line)),
        );
        if exact_rows.is_empty() {
            candidates.extend(
                grep_lines(&mut s, fallback, &read)
                    .into_iter()
                    .map(|line| (id, line)),
            );
        } else {
            candidates.clear();
        }
    }
    if exact_rows.is_empty() {
        candidates
    } else {
        exact_rows
    }
}

fn main() {
    println!(
        "grep-regex 0.1.14 + grep-searcher 0.1.17; 9 samples, rotating engine order; medians in microseconds"
    );
    println!(
        "scan includes line numbers and Vec allocation; excludes filesystem, policy, hashes, context, JSON, BM25 and matcher construction"
    );
    check_edges();
    for count in [2, 8, 16] {
        let terms = (0..count)
            .map(|i| format!("term{i:03}"))
            .collect::<Vec<_>>();
        let a = time(
            || {
                black_box(baseline(black_box(&terms)));
            },
            20,
        );
        let b = time(
            || {
                black_box(literals(black_box(&terms)));
            },
            20,
        );
        let c = time(
            || {
                black_box(aho(black_box(&terms)));
            },
            20,
        );
        let cached = baseline(&terms);
        let clone_us = time(
            || {
                black_box(black_box(&cached).clone());
            },
            100,
        );
        let phrase = terms.join(" ");
        let exact_build_us = time(
            || {
                black_box(
                    RegexMatcherBuilder::new()
                        .fixed_strings(true)
                        .multi_line(true)
                        .line_terminator(Some(b'\n'))
                        .build(black_box(&phrase))
                        .unwrap(),
                );
            },
            20,
        );
        println!(
            "build terms={count} grep_us={a:.2} literals_us={b:.2} aho_us={c:.2} hybrid_total_build_us={:.2} cached_clone_us={clone_us:.3} redundant_exact_build_us={exact_build_us:.2}",
            a + c
        );
        for hit in [0, 1, 25] {
            scan_bench(
                &format!("ascii_hits{hit}"),
                &terms,
                &corpus(&terms, hit, false),
            );
        }
        scan_bench("unicode_hits1", &terms, &corpus(&terms, 1, true));
    }
    let actual = [
        include_bytes!("../../../src/adapters/search.rs").to_vec(),
        include_bytes!("../../../src/adapters/search/ranking.rs").to_vec(),
        include_bytes!("../../../src/adapters/search/scan.rs").to_vec(),
        include_bytes!("../../../src/adapters/search/scan/matches.rs").to_vec(),
    ];
    scan_bench(
        "actual_sources",
        &["context".into(), "matcher".into(), "search".into()],
        &actual,
    );
    let terms = vec!["term000".into(), "term001".into()];
    let fallback = baseline(&terms);
    let exact = RegexMatcherBuilder::new()
        .fixed_strings(true)
        .multi_line(true)
        .line_terminator(Some(b'\n'))
        .build("term000 term001")
        .unwrap();
    for hit in [0, 1, 25] {
        let files = corpus(&terms, hit, false);
        assert_eq!(
            double_pass(&files, &exact, &fallback),
            same_buffer(&files, &exact, &fallback)
        );
        let a = time(
            || {
                black_box(double_pass(black_box(&files), &exact, &fallback));
            },
            4,
        );
        let b = time(
            || {
                black_box(same_buffer(black_box(&files), &exact, &fallback));
            },
            4,
        );
        println!(
            "read_reuse no_exact hits={hit} two_pass_us={a:.2} same_buffer_us={b:.2} speedup={:.3}",
            a / b
        );
    }
    let mut files = corpus(&terms, 1, false);
    files
        .last_mut()
        .unwrap()
        .extend_from_slice(b"term000 term001\n");
    assert_eq!(
        double_pass(&files, &exact, &fallback),
        same_buffer(&files, &exact, &fallback)
    );
    let a = time(
        || {
            black_box(double_pass(black_box(&files), &exact, &fallback));
        },
        4,
    );
    let b = time(
        || {
            black_box(same_buffer(black_box(&files), &exact, &fallback));
        },
        4,
    );
    println!(
        "read_reuse exact_in_last_file two_pass_us={a:.2} same_buffer_us={b:.2} speedup={:.3}",
        a / b
    );
    println!(
        "read_reuse uses Vec copies as a read-cost floor; no real IO, traversal, context or budget semantics; not a production replacement"
    );
}

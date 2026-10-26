//! Offline experiment only: no production search behavior is changed.
use bm25::{EmbedderBuilder, Scorer, Tokenizer};
use serde_json::json;
use std::{fs, process::Command, time::Instant};
use syn::{spanned::Spanned, visit::Visit};

struct TestRanges(Vec<std::ops::RangeInclusive<usize>>);
fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || matches!(&attr.meta,
        syn::Meta::List(list) if list.path.is_ident("cfg") && list.tokens.to_string() == "test")
    })
}
impl<'ast> Visit<'ast> for TestRanges {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if test_only(&node.attrs) {
            self.0
                .push(node.span().start().line..=node.span().end().line);
        } else {
            syn::visit::visit_item_mod(self, node);
        }
    }
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if test_only(&node.attrs) {
            self.0
                .push(node.span().start().line..=node.span().end().line);
        } else {
            syn::visit::visit_item_fn(self, node);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CodeTokens;
impl Tokenizer for CodeTokens {
    fn tokenize(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        for word in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if word.is_empty() {
                continue;
            }
            tokens.push(word.to_lowercase());
            for part in word.split('_').filter(|part| !part.is_empty()) {
                let chars: Vec<_> = part.char_indices().collect();
                let mut start = 0;
                for n in 1..chars.len() {
                    let previous = chars[n - 1].1;
                    let current = chars[n].1;
                    let next_lower = chars.get(n + 1).is_some_and(|(_, c)| c.is_lowercase());
                    if (previous.is_lowercase() && current.is_uppercase())
                        || (previous.is_uppercase() && current.is_uppercase() && next_lower)
                        || previous.is_numeric() != current.is_numeric()
                    {
                        tokens.push(part[start..chars[n].0].to_lowercase());
                        start = chars[n].0;
                    }
                }
                let tail = &part[start..];
                if tail != word {
                    tokens.push(tail.to_lowercase());
                }
            }
        }
        tokens
    }
}

struct Chunk {
    path: String,
    line: usize,
    text: String,
    context: String,
}
struct Case {
    query: &'static str,
    expected: &'static str,
}

fn rank(chunks: &[Chunk], ids: &[usize], query: &str) -> Vec<usize> {
    if ids.is_empty() {
        return Vec::new();
    }
    let corpus: Vec<&str> = ids.iter().map(|id| chunks[*id].context.as_str()).collect();
    let embedder =
        EmbedderBuilder::<u32, CodeTokens>::with_tokenizer_and_fit_to_corpus(CodeTokens, &corpus)
            .build();
    let mut scorer = Scorer::<usize>::new();
    for &id in ids {
        scorer.upsert(&id, embedder.embed(&chunks[id].context));
    }
    let query_embedding = embedder.embed(query);
    let mut ranked: Vec<_> = ids
        .iter()
        .map(|&id| (id, scorer.score(&id, &query_embedding).unwrap_or(0.0)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().map(|(id, _)| id).collect()
}

fn summary(chunks: &[Chunk], ids: &[usize], expected: &str) -> serde_json::Value {
    let position = ids
        .iter()
        .position(|id| chunks[*id].path == expected)
        .map(|n| n + 1);
    json!({"firstRelevantRank":position,"hitAt5":position.is_some_and(|n| n <= 5),
        "top5":ids.iter().take(5).map(|id| json!({"file":chunks[*id].path,"line":chunks[*id].line})).collect::<Vec<_>>()})
}

fn main() {
    let root = std::env::args().nth(1).expect("workspace path required");
    let report_path = std::env::args().nth(2);
    let load_started = Instant::now();
    let output = Command::new("git")
        .args(["ls-files", "-z", "--", "src", "crates"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut paths: Vec<_> = output
        .stdout
        .split(|b| *b == 0)
        .filter_map(|p| std::str::from_utf8(p).ok())
        .filter(|p| {
            p.ends_with(".rs")
                && !p.contains("/tests/")
                && !p.ends_with("_tests.rs")
                && !p.ends_with("/tests.rs")
        })
        .collect();
    paths.sort();
    let mut chunks = Vec::new();
    for path in &paths {
        let Ok(text) = fs::read_to_string(std::path::Path::new(&root).join(path)) else {
            continue;
        };
        let syntax =
            syn::parse_file(&text).unwrap_or_else(|error| panic!("cannot parse {path}: {error}"));
        let mut test_ranges = TestRanges(Vec::new());
        test_ranges.visit_file(&syntax);
        let retained: Vec<_> = text
            .lines()
            .enumerate()
            .map(|(n, line)| {
                if test_ranges.0.iter().any(|range| range.contains(&(n + 1))) {
                    ""
                } else {
                    line
                }
            })
            .collect();
        let lines = retained;
        for (n, text) in lines.iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            chunks.push(Chunk {
                path: (*path).to_owned(),
                line: n + 1,
                text: (*text).to_owned(),
                context: lines[n.saturating_sub(2)..(n + 3).min(lines.len())].join("\n"),
            });
        }
    }
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    // File-level relevance labels, chosen before running the scorer. These are
    // engineering examples, not an independent or statistically validated set.
    let cases = [
        Case {
            query: "context",
            expected: "src/adapters/search/scan/matches.rs",
        },
        Case {
            query: "metadata",
            expected: "src/adapters/search/scan/results.rs",
        },
        Case {
            query: "cursor",
            expected: "crates/domain/src/results.rs",
        },
        Case {
            query: "timeout",
            expected: "crates/domain/src/execution.rs",
        },
        Case {
            query: "session",
            expected: "crates/application/src/dispatch.rs",
        },
        Case {
            query: "regex",
            expected: "src/adapters/search/plan.rs",
        },
        Case {
            query: "hash",
            expected: "src/adapters/search/scan/matches.rs",
        },
        Case {
            query: "globs",
            expected: "src/adapters/search/plan.rs",
        },
        Case {
            query: "SEARCH_INCOMPLETE",
            expected: "src/adapters/search/presentation.rs",
        },
        Case {
            query: "context lines hash",
            expected: "src/adapters/search/scan/matches.rs",
        },
        Case {
            query: "search cursor pagination",
            expected: "crates/domain/src/results.rs",
        },
        Case {
            query: "regex case sensitive",
            expected: "src/adapters/search/plan.rs",
        },
        Case {
            query: "metadata bytes budget",
            expected: "src/adapters/search/scan/results.rs",
        },
        Case {
            query: "cancel execution timeout",
            expected: "crates/domain/src/execution.rs",
        },
        Case {
            query: "search_page",
            expected: "crates/domain/src/results.rs",
        },
        Case {
            query: "with_timeout",
            expected: "crates/domain/src/execution.rs",
        },
        Case {
            query: "RegexMatcherBuilder",
            expected: "src/adapters/search/plan.rs",
        },
        Case {
            query: "metadata_bytes",
            expected: "src/adapters/search/scan/results.rs",
        },
        Case {
            query: "glob_matcher",
            expected: "src/adapters/search/plan.rs",
        },
    ];
    let mut results = Vec::new();
    let mut exact_hits = 0;
    let mut rerank_hits = 0;
    let mut retrieval_hits = 0;
    let mut ranking_ms = Vec::new();
    for case in &cases {
        assert!(
            paths.contains(&case.expected),
            "missing relevance label: {}",
            case.expected
        );
        let filter_start = Instant::now();
        let exact: Vec<_> = chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.text.contains(case.query))
            .map(|(n, _)| n)
            .collect();
        let exact_filter_ms = filter_start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let reranked = rank(&chunks, &exact, case.query);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if !exact.is_empty() {
            ranking_ms.push(elapsed);
        }
        let term_filter_start = Instant::now();
        let query_terms = CodeTokens.tokenize(case.query);
        let candidates: Vec<_> = chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| {
                let tokens = CodeTokens.tokenize(&chunk.text);
                query_terms.iter().any(|term| tokens.contains(term))
            })
            .map(|(n, _)| n)
            .collect();
        let term_filter_ms = term_filter_start.elapsed().as_secs_f64() * 1000.0;
        let broad_start = Instant::now();
        let retrieved = rank(&chunks, &candidates, case.query);
        let broad_ms = broad_start.elapsed().as_secs_f64() * 1000.0;
        let baseline = summary(&chunks, &exact, case.expected);
        let rerank = summary(&chunks, &reranked, case.expected);
        let retrieval = summary(&chunks, &retrieved, case.expected);
        exact_hits += usize::from(baseline["hitAt5"] == true);
        rerank_hits += usize::from(rerank["hitAt5"] == true);
        retrieval_hits += usize::from(retrieval["hitAt5"] == true);
        results.push(
            json!({"query":case.query,"expectedFile":case.expected,"exactCandidates":exact.len(),
            "baseline":baseline,"rerankExact":rerank,"termRetrieval":retrieval,
            "exactFilterMs":exact_filter_ms,"termFilterMs":term_filter_ms,
            "rerankFitAndScoreMs":elapsed,"termFitAndScoreMs":broad_ms}),
        );
    }
    ranking_ms.sort_by(f64::total_cmp);
    let report = json!({"files":paths.len(),"chunks":chunks.len(),"cases":cases.len(),"loadAndParseMs":load_ms,
        "hitAt5":{"baseline":exact_hits,"rerankExact":rerank_hits,"termRetrieval":retrieval_hits},
        "rerankNonemptyMedianMs":ranking_ms[ranking_ms.len()/2],"results":results,
        "limitations":["file-level labels; small hand-selected exploratory set; broad single words are ambiguous", "scope src/crates tracked Rust only; test files and inline test modules/functions excluded", "five-line contexts; overlapping results retained", "path/line baseline approximates scan ordering; not a live MCP benchmark", "single run; concurrent compilation can affect timings", "termRetrieval changes candidate semantics, unlike rerankExact", "no production integration or persistent index"]});
    if let Some(path) = report_path {
        fs::write(path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
        println!(
            "{}",
            json!({"files":report["files"],"chunks":report["chunks"],"cases":report["cases"],"hitAt5":report["hitAt5"]})
        );
        for row in report["results"].as_array().unwrap() {
            println!(
                "{}: path={} rerank={} terms={} | rerank {:.2}ms terms+filter {:.2}ms",
                row["query"],
                row["baseline"]["firstRelevantRank"],
                row["rerankExact"]["firstRelevantRank"],
                row["termRetrieval"]["firstRelevantRank"],
                row["rerankFitAndScoreMs"].as_f64().unwrap(),
                row["termFitAndScoreMs"].as_f64().unwrap() + row["termFilterMs"].as_f64().unwrap()
            );
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}

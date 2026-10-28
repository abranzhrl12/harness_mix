//! Automatic lexical fallback. BM25 is provided by the library, without a model
//! or persistent index; exact, regex and structural searches retain precedence.
use super::matcher_cache::{self, Options};
use bm25::{EmbedderBuilder, Scorer, Tokenizer};
use harness_domain::{
    Failure,
    contracts::{Kind, SearchArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct CodeTokens;
impl Tokenizer for CodeTokens {
    fn tokenize(&self, text: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        for word in text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty())
        {
            tokens.push(word.to_lowercase());
            for part in word.split('_').filter(|s| !s.is_empty()) {
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
                if &part[start..] != word {
                    tokens.push(part[start..].to_lowercase());
                }
            }
        }
        tokens
    }
}

pub(super) fn fallback_matcher(
    args: &SearchArgs,
) -> Result<Option<grep_regex::RegexMatcher>, Failure> {
    if !matches!(args.kind, Kind::Text)
        || args.ast.is_some()
        || args.regex.is_some()
        || args.case_sensitive.is_some()
        || args
            .query
            .chars()
            .any(|c| !c.is_alphanumeric() && c != '_' && !c.is_whitespace())
    {
        return Ok(None);
    }
    let mut terms = CodeTokens.tokenize(&args.query);
    terms.sort();
    terms.dedup();
    if args.query.split_whitespace().count() < 2 || !(2..=16).contains(&terms.len()) {
        return Ok(None);
    }
    // Ripgrep gathers candidates quickly; the tokenizer below rejects substring
    // false positives such as `cat` inside `concatenate` before ranking.
    let pattern = terms
        .iter()
        .map(|term| regex::escape(term))
        .collect::<Vec<_>>()
        .join("|");
    matcher_cache::get(
        &pattern,
        Options {
            literal: false,
            case_sensitive: false,
            multi_line: false,
        },
    )
    .map(Some)
    .map_err(|_| ("INTERNAL", "could not compile term retrieval matcher"))
}

pub(super) fn rank(
    rows: &mut Vec<Value>,
    query: &str,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    let query_terms = CodeTokens.tokenize(query);
    rows.retain(|row| {
        CodeTokens
            .tokenize(row["text"].as_str().unwrap_or_default())
            .iter()
            .any(|term| query_terms.contains(term))
    });
    if rows.is_empty() {
        return Ok(());
    }
    let corpus: Vec<&str> = rows
        .iter()
        .map(|row| {
            row["context"]["text"]
                .as_str()
                .or_else(|| row["text"].as_str())
                .unwrap_or_default()
        })
        .collect();
    let embedder =
        EmbedderBuilder::<u32, CodeTokens>::with_tokenizer_and_fit_to_corpus(CodeTokens, &corpus)
            .build();
    context.check()?;
    let mut scorer = Scorer::<usize>::new();
    for (id, text) in corpus.iter().enumerate() {
        if id % 128 == 0 {
            context.check()?;
        }
        scorer.upsert(&id, embedder.embed(text));
    }
    let query_embedding = embedder.embed(query);
    for (id, row) in rows.iter_mut().enumerate() {
        row["score"] = json!(scorer.score(&id, &query_embedding).unwrap_or(0.0));
    }
    rows.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
            .then_with(|| a["file"].as_str().cmp(&b["file"].as_str()))
            .then_with(|| a["line"].as_u64().cmp(&b["line"].as_u64()))
    });
    context.check()
}

use super::results::MatchResults;
use harness_domain::{
    contracts::SearchArgs, execution::OperationContext, parsing::Symbol, results::utf16_len,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Evaluation {
    Continue,
    Stop,
}

/// Per-file state shared by the matching strategies.
pub(super) struct MatchState<'a> {
    pub(super) relative: &'a str,
    pub(super) digest: &'a str,
    pub(super) pending_path: &'a Option<String>,
    pub(super) context: &'a OperationContext,
    pub(super) started: Instant,
    pub(super) results: &'a mut MatchResults,
    pub(super) skipped_files: &'a mut Vec<Value>,
    pub(super) bounded: &'a mut bool,
    pub(super) timed_out: &'a mut bool,
    pub(super) cancelled: &'a mut bool,
    pub(super) truncated: &'a mut bool,
    pub(super) stable_files: &'a mut usize,
}

impl MatchState<'_> {
    fn check_budget(&mut self) -> bool {
        if let Err((code, _)) = self.context.check() {
            *self.cancelled = code == "CANCELLED";
            *self.timed_out = code == "TIMEOUT";
            self.skipped_files.push(json!({"path":self.pending_path,"reason":if *self.cancelled { "cancelled" } else { "time_budget" }}));
            return false;
        }
        if self.started.elapsed() >= Duration::from_secs(5) {
            *self.timed_out = true;
            self.skipped_files
                .push(json!({"path":self.pending_path,"reason":"time_budget"}));
            return false;
        }
        true
    }

    fn add(&mut self, row: Value, enforce_row_limit: bool) -> bool {
        if self.results.add(row, enforce_row_limit) {
            true
        } else {
            *self.bounded = true;
            self.skipped_files
                .push(json!({"path":self.pending_path,"reason":"result_budget"}));
            false
        }
    }
}

pub(super) fn symbol_matches(
    symbols: Vec<Symbol>,
    kind: &str,
    name: Option<&str>,
    state: &mut MatchState<'_>,
) -> Evaluation {
    for symbol in symbols {
        if !state.check_budget() {
            return Evaluation::Stop;
        }
        if super::symbol_graph::canonical_kind(symbol.kind) == Some(kind)
            && name.is_none_or(|name| symbol.name == name)
        {
            let row = json!({"file":state.relative,"hash":state.digest,"name":symbol.name,
                "container":symbol.container,"kind":symbol.kind,"start":symbol.start,
                "end":symbol.end,"signature":symbol.signature,"symbolId":symbol.id});
            if !state.add(row, true) {
                return Evaluation::Stop;
            }
        }
    }
    Evaluation::Continue
}

pub(super) fn text_matches(
    source: &str,
    file: &[u8],
    args: &SearchArgs,
    context_lines: usize,
    state: &mut MatchState<'_>,
) -> Evaluation {
    // Offsets refer to the bytes already read, so the hash and context describe
    // one source snapshot and preserve CRLF and trailing whitespace exactly.
    let offsets: Vec<usize> = if args.context_lines.is_some() {
        std::iter::once(0)
            .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
            .filter(|offset| *offset < source.len())
            .collect()
    } else {
        Vec::new()
    };
    let digest = args.context_lines.map(|_| harness_domain::hash(file));
    for (line, text) in source.split_inclusive('\n').enumerate() {
        if !state.check_budget() {
            return Evaluation::Stop;
        }
        if text.contains(&args.query) {
            *state.truncated |= utf16_len(text) > 500;
            let mut row = json!({"file":state.relative,"line":line + 1,"text":text});
            if let Some(digest) = &digest {
                let start = line.saturating_sub(context_lines);
                let end = (line + context_lines + 1).min(offsets.len());
                let end_offset = offsets.get(end).copied().unwrap_or(source.len());
                row["hash"] = json!(digest);
                row["context"] = json!({"start":start + 1,"end":end,
                    "text":&source[offsets[start]..end_offset]});
            }
            if !state.add(row, false) {
                return Evaluation::Stop;
            }
        }
        // Bound stored results and a single file containing millions of lines.
        if state.results.rows.len() >= 10000 {
            *state.bounded = true;
            state
                .skipped_files
                .push(json!({"path":state.pending_path,"reason":"result_budget"}));
            break;
        }
    }
    Evaluation::Continue
}

pub(super) fn file_size_matches(
    file: &[u8],
    min_lines: usize,
    state: &mut MatchState<'_>,
) -> Evaluation {
    // Historically file-size scans check the wall-clock budget without calling
    // the operation context, then count a stable file before applying limits.
    if state.started.elapsed() >= Duration::from_secs(5) {
        *state.timed_out = true;
        state
            .skipped_files
            .push(json!({"path":state.pending_path,"reason":"time_budget"}));
        return Evaluation::Stop;
    }
    *state.stable_files += 1;
    let lines = file.iter().filter(|&&byte| byte == b'\n').count()
        + usize::from(!file.is_empty() && file.last() != Some(&b'\n'));
    if lines > min_lines {
        let row = json!({"path":state.relative,"lines":lines,"bytes":file.len()});
        if !state.add(row, false) {
            return Evaluation::Stop;
        }
    }
    Evaluation::Continue
}

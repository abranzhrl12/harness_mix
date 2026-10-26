use super::results::MatchResults;
use harness_domain::{execution::OperationContext, parsing::Symbol, results::utf16_len};
use serde_json::{Value, json};
use std::time::Instant;

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
        if self.started.elapsed() >= super::SCAN_TIMEOUT {
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
                .push(json!({"path":self.pending_path,"reason":self.results.limit_reason()}));
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
    include_context: bool,
    context_lines: usize,
    matcher: &grep_regex::RegexMatcher,
    searcher: &mut grep_searcher::Searcher,
    state: &mut MatchState<'_>,
) -> Result<Evaluation, harness_domain::Failure> {
    if !state.check_budget() {
        return Ok(Evaluation::Stop);
    }
    // Most candidate files do not match. Allocate line offsets and hash only
    // when a row will actually include context; both still refer to these bytes.
    let mut offsets: Option<Vec<usize>> = None;
    let mut digest: Option<String> = None;
    let mut evaluation = Evaluation::Continue;
    searcher
        .search_slice(
            matcher,
            file,
            grep_searcher::sinks::UTF8(|number, text| {
                if !state.check_budget() {
                    evaluation = Evaluation::Stop;
                    return Ok(false);
                }
                let line = number as usize - 1;
                *state.truncated |= utf16_len(text) > 500;
                let mut row = json!({"file":state.relative,"line":line + 1,"text":text});
                if include_context {
                    let offsets = offsets.get_or_insert_with(|| {
                        std::iter::once(0)
                            .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
                            .filter(|offset| *offset < source.len())
                            .collect()
                    });
                    let digest = digest.get_or_insert_with(|| harness_domain::hash(file));
                    let start = line.saturating_sub(context_lines);
                    let end = (line + context_lines + 1).min(offsets.len());
                    let end_offset = offsets.get(end).copied().unwrap_or(source.len());
                    row["hash"] = json!(digest);
                    row["context"] = json!({"start":start + 1,"end":end,
                    "text":&source[offsets[start]..end_offset]});
                }
                if !state.add(row, false) {
                    evaluation = Evaluation::Stop;
                    return Ok(false);
                }
                // Bound stored results and a single file containing millions of lines.
                if state.results.rows.len() >= 10000 {
                    *state.bounded = true;
                    state
                        .skipped_files
                        .push(json!({"path":state.pending_path,"reason":"match_count_budget"}));
                    evaluation = Evaluation::Stop;
                    return Ok(false);
                }
                Ok(true)
            }),
        )
        .map_err(|_| ("INTERNAL", "text search failed"))?;
    if evaluation == Evaluation::Continue && !state.check_budget() {
        evaluation = Evaluation::Stop;
    }
    Ok(evaluation)
}

pub(super) fn structural_matches(
    source: &str,
    file: &[u8],
    search: &super::super::structural::StructuralSearch,
    context_lines: usize,
    state: &mut MatchState<'_>,
) -> Result<Evaluation, harness_domain::Failure> {
    if !state.check_budget() {
        return Ok(Evaluation::Stop);
    }
    let tree = ast_grep_core::AstGrep::try_new(source, search.language)
        .map_err(|_| ("PARSE", "ast-grep could not parse source"))?;
    if !state.check_budget() {
        return Ok(Evaluation::Stop);
    }
    if tree.root().get_doc().tree.root_node().has_error() {
        return Err((
            "PARSE",
            "source has syntax errors; structural coverage is incomplete",
        ));
    }
    *state.stable_files += 1;
    let mut offsets: Option<Vec<usize>> = None;
    let mut digest: Option<String> = None;
    let root = tree.root();
    for node in root.find_all(&search.pattern) {
        if !state.check_budget() {
            return Ok(Evaluation::Stop);
        }
        let range = node.range();
        if range.is_empty() {
            continue;
        }
        let offsets = offsets.get_or_insert_with(|| {
            std::iter::once(0)
                .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
                .filter(|offset| *offset < source.len())
                .collect()
        });
        let digest = digest.get_or_insert_with(|| harness_domain::hash(file));
        let line = node.start_pos().line();
        let end = node.end_pos();
        // Tree-sitter ends are exclusive; an end at column zero belongs to
        // the preceding line for our inclusive context range.
        let end_line = end.line() + usize::from(end.byte_point().1 > 0);
        let context_start = line.saturating_sub(context_lines);
        let context_end = (end_line + context_lines).min(offsets.len());
        let end_offset = offsets.get(context_end).copied().unwrap_or(source.len());
        let text = &source[range.clone()];
        *state.truncated |= utf16_len(text) > 500;
        let row = json!({
            "file":state.relative, "line":line + 1, "endLine":end_line,
            "startByte":range.start, "endByte":range.end,
            "text":text, "hash":digest,
            "context":{"start":context_start + 1,"end":context_end,
                "text":&source[offsets[context_start]..end_offset]}
        });
        if !state.add(row, true) {
            return Ok(Evaluation::Stop);
        }
    }
    if !state.check_budget() {
        return Ok(Evaluation::Stop);
    }
    Ok(Evaluation::Continue)
}

pub(super) fn file_size_matches(
    file: &[u8],
    min_lines: usize,
    state: &mut MatchState<'_>,
) -> Evaluation {
    // Historically file-size scans check the wall-clock budget without calling
    // the operation context, then count a stable file before applying limits.
    if state.started.elapsed() >= super::SCAN_TIMEOUT {
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

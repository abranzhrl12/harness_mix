//! Shared, pure source-coordinate helpers for native framework adapters.
use harness_domain::{Failure, hash};
use serde_json::{Value, json};
use std::ops::Range;

const RANGE: Failure = ("EXTRACTION_CONTRACT", "Invalid framework source range");

/// Sparse coordinate index: ASCII columns require no per-character storage.
/// Non-ASCII boundaries record cumulative UTF-8/UTF-16 width differences.
pub(in crate::adapters) struct Positions<'a> {
    source: &'a str,
    lines: Vec<usize>,
    adjustments: Vec<(usize, usize)>,
}

impl<'a> Positions<'a> {
    pub(in crate::adapters) fn new(source: &'a str) -> Self {
        let mut lines = vec![0];
        let mut adjustments = Vec::new();
        let mut difference = 0;
        for (at, ch) in source.char_indices() {
            if ch == '\n' {
                lines.push(at + 1);
            }
            if !ch.is_ascii() {
                difference += ch.len_utf8() - ch.len_utf16();
                adjustments.push((at + ch.len_utf8(), difference));
            }
        }
        Self {
            source,
            lines,
            adjustments,
        }
    }

    fn adjustment(&self, at: usize) -> usize {
        let index = self
            .adjustments
            .partition_point(|&(boundary, _)| boundary <= at);
        index
            .checked_sub(1)
            .map_or(0, |index| self.adjustments[index].1)
    }

    pub(in crate::adapters) fn position(&self, at: usize) -> Result<(usize, usize), Failure> {
        if !self.source.is_char_boundary(at) {
            return Err(RANGE);
        }
        let line = self.lines.partition_point(|&start| start <= at) - 1;
        let start = self.lines[line];
        Ok((
            line + 1,
            at - start - (self.adjustment(at) - self.adjustment(start)),
        ))
    }
}

/// Reserved namespace distinguishes native framework nodes from provider synthesis.
pub(super) fn id(framework: &str, path: &str, kind: &str, at: usize) -> String {
    format!(
        "native:framework:{}",
        hash(format!("{framework}\0{path}\0{kind}\0{at}").as_bytes())
    )
}

/// Provider coordinates: one-based lines and zero-based UTF-16 columns.
pub(super) fn position(source: &str, at: usize) -> Result<(usize, usize), Failure> {
    let prefix = source.get(..at).ok_or(RANGE)?;
    Ok((
        prefix.bytes().filter(|&byte| byte == b'\n').count() + 1,
        prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count(),
    ))
}

/// Keep one source region and mask everything else, preserving UTF-16 coordinates.
/// Bytes can differ for non-ASCII masked text; callers must use line/column spans.
pub(in crate::adapters) fn project(source: &str, body: Range<usize>) -> Result<String, Failure> {
    if body.start > body.end || source.get(body.clone()).is_none() {
        return Err(RANGE);
    }
    let mut output = String::with_capacity(source.len());
    for (at, ch) in source.char_indices() {
        if body.contains(&at) || matches!(ch, '\r' | '\n') {
            output.push(ch);
        } else {
            for _ in 0..ch.len_utf16() {
                output.push(' ');
            }
        }
    }
    Ok(output)
}

pub(super) fn file_node(path: &str, source: &str, language: &str) -> Value {
    let (line, column) = position(source, source.len()).expect("source length is a UTF-8 boundary");
    json!({"id":format!("file:{path}"),"kind":"file","name":path,
        "qualifiedName":path,"filePath":path,"language":language,
        "startLine":1,"startColumn":0,"endLine":line,"endColumn":column})
}

#[cfg(test)]
#[path = "framework_tests.rs"]
mod tests;

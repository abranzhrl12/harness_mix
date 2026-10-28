//! Exact multi-pattern lookup; publication and edit validation stay in prepare.
use super::matching::locate_chunk;
use crate::Failure;
use aho_corasick::{AhoCorasickBuilder, MatchKind};
use std::borrow::Cow;

/// Building an automaton only pays for many short fragments in a large source.
pub(super) fn eligible(
    source_bytes: usize,
    count: usize,
    pattern_bytes: impl FnOnce() -> usize,
) -> bool {
    if source_bytes < 64 * 1024 || !(16..=128).contains(&count) {
        return false;
    }
    let pattern_bytes = pattern_bytes();
    pattern_bytes <= 4096 && pattern_bytes <= count * 64
}

fn serial_tail(
    hay: &str,
    needles: &[Cow<'_, str>],
    first: (usize, usize),
) -> Result<Vec<(usize, usize)>, Failure> {
    let mut ranges = Vec::with_capacity(needles.len());
    ranges.push(first);
    for needle in &needles[1..] {
        ranges.push(locate_chunk(hay, needle)?);
    }
    Ok(ranges)
}

/// Inputs are normalized UTF-8. Every returned range refers to that same input.
pub(super) fn locate_chunks(
    hay: &str,
    needles: &[Cow<'_, str>],
) -> Result<Vec<(usize, usize)>, Failure> {
    let Some(needle) = needles.first() else {
        return Ok(Vec::new());
    };
    // Preserve the cheap first-error path. If the first fragment needs whitespace
    // tolerance, avoid a speculative exact scan of the rest of the request.
    let first = locate_chunk(hay, needle)?;
    if needles.len() == 1 || &hay[first.0..first.1] != needle.as_ref() {
        return serial_tail(hay, needles, first);
    }
    let Ok(matcher) = AhoCorasickBuilder::new()
        .match_kind(MatchKind::Standard)
        .build(needles[1..].iter().map(|needle| needle.as_bytes()))
    else {
        // An optimization must not introduce a new failure for valid requests.
        return serial_tail(hay, needles, first);
    };

    let mut exact = vec![None; needles.len() - 1];
    for found in matcher.find_overlapping_iter(hay) {
        let slot = &mut exact[found.pattern().as_usize()];
        if slot.replace((found.start(), found.end())).is_some() {
            // All fragment-location failures use this same error. Rejecting any
            // ambiguous exact match now preserves prepare's error precedence:
            // overlap/no-op/BOM checks only run after every fragment is located.
            return Err(("MATCH", "Original text must appear exactly once"));
        }
    }

    let mut ranges = Vec::with_capacity(needles.len());
    ranges.push(first);
    for (needle, range) in needles[1..].iter().zip(exact) {
        // Only missing exact matches need the existing whitespace/KMP fallback.
        ranges.push(match range {
            Some(range) => range,
            None => locate_chunk(hay, needle)?,
        });
    }
    Ok(ranges)
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;

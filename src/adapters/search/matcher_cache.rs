//! Bounded process-local cache of compiled text matchers, never source/results.
use grep_regex::{Error, RegexMatcher, RegexMatcherBuilder};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

const CAPACITY: usize = 32;
// Long patterns remain supported, but must not displace common short queries.
const MAX_PATTERN_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Options {
    pub(super) literal: bool,
    pub(super) case_sensitive: bool,
    pub(super) multi_line: bool,
}

struct Entry {
    pattern: String,
    options: Options,
    matcher: RegexMatcher,
}

struct Cache {
    capacity: usize,
    entries: VecDeque<Entry>,
}

impl Cache {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    fn get(&mut self, pattern: &str, options: Options) -> Option<RegexMatcher> {
        let position = self
            .entries
            .iter()
            .position(|entry| entry.pattern == pattern && entry.options == options)?;
        let entry = self.entries.remove(position).expect("located cache entry");
        let matcher = entry.matcher.clone();
        self.entries.push_front(entry);
        Some(matcher)
    }

    fn insert(&mut self, pattern: &str, options: Options, matcher: RegexMatcher) {
        // Simultaneous cold misses may compile independently outside the lock.
        self.entries
            .retain(|entry| entry.pattern != pattern || entry.options != options);
        if self.entries.len() == self.capacity {
            self.entries.pop_back();
        }
        self.entries.push_front(Entry {
            pattern: pattern.to_owned(),
            options,
            matcher,
        });
    }
}

fn cached(
    cache: &Mutex<Cache>,
    pattern: &str,
    options: Options,
    compile: impl FnOnce() -> Result<RegexMatcher, Error>,
) -> Result<RegexMatcher, Error> {
    let retain = pattern.len() <= MAX_PATTERN_BYTES;
    if retain
        && let Ok(mut cache) = cache.lock()
        && let Some(matcher) = cache.get(pattern, options)
    {
        return Ok(matcher);
    }

    // Compilation can be expensive. Never serialize other searches behind it.
    // Compilation errors are returned unchanged and are never cached.
    let matcher = compile()?;
    if retain && let Ok(mut cache) = cache.lock() {
        cache.insert(pattern, options, matcher.clone());
    }
    Ok(matcher)
}

pub(super) fn get(pattern: &str, options: Options) -> Result<RegexMatcher, Error> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    cached(
        CACHE.get_or_init(|| Mutex::new(Cache::new(CAPACITY))),
        pattern,
        options,
        || {
            RegexMatcherBuilder::new()
                .fixed_strings(options.literal)
                .case_insensitive(!options.case_sensitive)
                .multi_line(options.multi_line)
                // All callers use the same LF terminator; it is not a varying cache option.
                .line_terminator(Some(b'\n'))
                .build(pattern)
        },
    )
}

#[cfg(test)]
#[path = "matcher_cache/tests.rs"]
mod tests;

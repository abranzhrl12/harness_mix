use super::*;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{SearcherBuilder, sinks::UTF8};
use std::cell::Cell;

const EXACT: Options = Options {
    literal: true,
    case_sensitive: true,
    multi_line: true,
};

fn request(
    cache: &Mutex<Cache>,
    count: &Cell<usize>,
    pattern: &str,
    options: Options,
) -> Result<RegexMatcher, Error> {
    cached(cache, pattern, options, || {
        count.set(count.get() + 1);
        RegexMatcherBuilder::new()
            .fixed_strings(options.literal)
            .case_insensitive(!options.case_sensitive)
            .multi_line(options.multi_line)
            .line_terminator(Some(b'\n'))
            .build(pattern)
    })
}

fn lines(matcher: &RegexMatcher, text: &str) -> Vec<u64> {
    let mut result = Vec::new();
    SearcherBuilder::new()
        .line_number(true)
        .bom_sniffing(false)
        .build()
        .search_slice(
            matcher,
            text.as_bytes(),
            UTF8(|line, _| {
                result.push(line);
                Ok(true)
            }),
        )
        .unwrap();
    result
}

#[test]
fn repeated_pattern_compiles_once_but_searches_current_source() {
    let cache = Mutex::new(Cache::new(2));
    let count = Cell::new(0);
    assert_eq!(
        lines(&request(&cache, &count, "Stock", EXACT).unwrap(), "Stock\n"),
        [1]
    );
    assert!(
        lines(
            &request(&cache, &count, "Stock", EXACT).unwrap(),
            "changed\n"
        )
        .is_empty()
    );
    assert_eq!(
        count.get(),
        1,
        "the second request must reuse the compiled matcher"
    );
}

#[test]
fn options_do_not_leak_between_cached_matchers_and_unicode_is_preserved() {
    let cache = Mutex::new(Cache::new(8));
    let count = Cell::new(0);
    let insensitive = Options {
        case_sensitive: false,
        ..EXACT
    };
    assert!(lines(&request(&cache, &count, "s", EXACT).unwrap(), "S\nſ\n").is_empty());
    assert_eq!(
        lines(
            &request(&cache, &count, "s", insensitive).unwrap(),
            "S\nſ\n"
        ),
        [1, 2]
    );
    assert_eq!(
        lines(
            &request(&cache, &count, "café", insensitive).unwrap(),
            "CAFÉ\r\n"
        ),
        [1]
    );
    assert!(lines(&request(&cache, &count, ".", EXACT).unwrap(), "a\n").is_empty());
    let regex = Options {
        literal: false,
        ..EXACT
    };
    assert_eq!(
        lines(&request(&cache, &count, ".", regex).unwrap(), "a\n"),
        [1]
    );
    assert_eq!(
        lines(&request(&cache, &count, "^b", regex).unwrap(), "a\nb\n"),
        [2]
    );
    let non_multiline = Options {
        multi_line: false,
        ..regex
    };
    // grep-searcher also permits line-local anchors without the multiline flag.
    assert_eq!(
        lines(
            &request(&cache, &count, "^b", non_multiline).unwrap(),
            "a\nb\n"
        ),
        [2]
    );
    request(&cache, &count, "s", insensitive).unwrap();
    assert_eq!(count.get(), 7);
}

#[test]
fn eviction_keeps_the_recently_used_matcher() {
    let cache = Mutex::new(Cache::new(2));
    let count = Cell::new(0);
    for pattern in ["a", "b", "a", "c", "a"] {
        request(&cache, &count, pattern, EXACT).unwrap();
    }
    assert_eq!(
        count.get(),
        3,
        "a must survive because a cache hit refreshes recency"
    );
    request(&cache, &count, "b", EXACT).unwrap();
    assert_eq!(
        count.get(),
        4,
        "the least recently used matcher must be rebuilt"
    );
}

#[test]
fn oversized_patterns_are_valid_but_not_retained() {
    let cache = Mutex::new(Cache::new(2));
    let count = Cell::new(0);
    let pattern = "a".repeat(MAX_PATTERN_BYTES + 1);
    for _ in 0..2 {
        let matcher = request(&cache, &count, &pattern, EXACT).unwrap();
        assert_eq!(lines(&matcher, &pattern), [1]);
    }
    assert_eq!(count.get(), 2);
}

#[test]
fn failed_compilation_does_not_enter_the_cache_or_evict_a_good_matcher() {
    let cache = Mutex::new(Cache::new(1));
    let count = Cell::new(0);
    request(&cache, &count, "valid", EXACT).unwrap();
    let regex = Options {
        literal: false,
        ..EXACT
    };
    for _ in 0..2 {
        assert!(request(&cache, &count, "(", regex).is_err());
    }
    request(&cache, &count, "valid", EXACT).unwrap();
    assert_eq!(count.get(), 3);
}

#[test]
fn compilation_does_not_hold_the_cache_lock() {
    let cache = Mutex::new(Cache::new(2));
    cached(&cache, "a", EXACT, || {
        assert!(cache.try_lock().is_ok());
        RegexMatcher::new("a")
    })
    .unwrap();
}

#[test]
fn poisoned_cache_falls_back_to_normal_compilation() {
    let cache = Mutex::new(Cache::new(2));
    let _ = std::panic::catch_unwind(|| {
        let _guard = cache.lock().unwrap();
        panic!("simulate interrupted cache mutation");
    });
    let count = Cell::new(0);
    assert_eq!(
        lines(
            &request(
                &cache,
                &count,
                "s",
                Options {
                    case_sensitive: false,
                    ..EXACT
                }
            )
            .unwrap(),
            "ſ\n"
        ),
        [1]
    );
}

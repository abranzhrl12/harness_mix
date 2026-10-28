use super::super::matching::locate_chunk;
use super::*;

fn locate(hay: &str, needles: &[&str]) -> Result<Vec<(usize, usize)>, Failure> {
    locate_chunks(
        hay,
        &needles
            .iter()
            .map(|needle| Cow::Borrowed(*needle))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn unicode_ranges_remain_utf8_byte_offsets() {
    assert_eq!(
        locate("λ one\nβ two\n", &["λ one", "β two"]),
        Ok(vec![(0, 6), (7, 13)])
    );
}

#[test]
fn missing_exact_match_uses_existing_whitespace_fallback() {
    assert_eq!(
        locate("  alpha\n    beta\ntail", &["alpha\nbeta", "tail"]),
        Ok(vec![(0, 16), (17, 21)])
    );
}

#[test]
fn exact_match_retains_precedence_over_whitespace_equivalents() {
    assert_eq!(
        locate(" foo\nfoo\nend", &[" foo", "end"]),
        Ok(vec![(0, 4), (9, 12)])
    );
}

#[test]
fn duplicate_and_nested_patterns_leave_overlap_validation_to_prepare() {
    assert_eq!(locate("abc", &["abc", "abc"]), Ok(vec![(0, 3), (0, 3)]));
    assert_eq!(
        locate("abc", &["a", "ab", "abc"]),
        Ok(vec![(0, 1), (0, 2), (0, 3)])
    );
}

#[test]
fn overlapping_occurrences_are_ambiguous_even_for_later_patterns() {
    assert_eq!(
        locate("head\naaaa", &["head", "aa"]),
        Err(("MATCH", "Original text must appear exactly once"))
    );
}

#[test]
fn bulk_results_match_sequential_matching_across_adversarial_inputs() {
    let mut cases = vec![
        ("head\nsame same".to_owned(), vec!["head", "same"]),
        ("head\nnothing".to_owned(), vec!["missing", "head"]),
        (
            "head\n  a\n    b\ntail".to_owned(),
            vec!["head", "a\nb", "tail"],
        ),
        (
            "prefix\naaaabbbb\nλ🙂".to_owned(),
            vec!["prefix", "aa", "bbb", "λ"],
        ),
        (
            "prefix\na\n  a\nlast".to_owned(),
            vec!["prefix", "a", "last"],
        ),
        (
            "one\n  two\nthree".to_owned(),
            vec!["one", "two", "missing"],
        ),
    ];
    for n in 1..=64 {
        let text = format!("first\n{}last\n", "a\nβ\n🙂\n".repeat(n));
        cases.push((text, vec!["first", "β", "last"]));
    }
    for (hay, needles) in cases {
        let expected = needles
            .iter()
            .map(|needle| locate_chunk(&hay, needle))
            .collect::<Result<Vec<_>, _>>();
        assert_eq!(locate(&hay, &needles), expected, "{hay:?} {needles:?}");
    }
}

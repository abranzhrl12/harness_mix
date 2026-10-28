use harness_domain::{
    contracts::{ReplaceArgs, ReplaceChunk},
    editing::prepare,
    hash,
};

fn source() -> String {
    let mut text = "\u{feff}".to_owned();
    text.push_str(&"padding café🙂\r\n".repeat(5000));
    for i in 0..16 {
        text.push_str(&format!("token_{i:02}\r\n"));
    }
    text.push_str("alpha\r\nbeta\r\naaaa\r\n");
    text
}

fn request(text: &str) -> ReplaceArgs {
    ReplaceArgs {
        path: "batch.rs".into(),
        hash: hash(text.as_bytes()),
        chunks: Some(
            (0..16)
                .rev()
                .map(|i| ReplaceChunk {
                    old_text: format!("token_{i:02}"),
                    new_text: format!("changed_{i:02}\nmore"),
                })
                .collect(),
        ),
        ..ReplaceArgs::default()
    }
}

#[test]
fn bulk_prepare_preserves_bom_crlf_unicode_and_original_chunk_coordinates() {
    let text = source();
    let mut expected = "\u{feff}".to_owned();
    expected.push_str(&"padding café🙂\r\n".repeat(5000));
    for i in 0..16 {
        expected.push_str(&format!("changed_{i:02}\r\nmore\r\n"));
    }
    expected.push_str("alpha\r\nbeta\r\naaaa\r\n");
    assert_eq!(prepare(text.as_bytes(), &request(&text)).unwrap(), expected);
}

#[test]
fn normalized_duplicate_patterns_report_overlap_instead_of_ambiguity() {
    let text = source();
    let mut args = request(&text);
    let chunks = args.chunks.as_mut().unwrap();
    chunks[1].old_text = "alpha\r\nbeta".into();
    chunks[2].old_text = "alpha\nbeta".into();
    assert_eq!(
        prepare(text.as_bytes(), &args),
        Err(("MATCH", "Replacement chunks overlap"))
    );
}

#[test]
fn later_overlapping_occurrences_cannot_be_silently_replaced() {
    let text = source();
    let mut args = request(&text);
    args.chunks.as_mut().unwrap()[1].old_text = "aa".into();
    assert_eq!(
        prepare(text.as_bytes(), &args),
        Err(("MATCH", "Original text must appear exactly once"))
    );
}

#[test]
fn a_later_whitespace_fallback_retains_its_full_original_line_range() {
    let text = source();
    let mut args = request(&text);
    args.chunks.as_mut().unwrap()[1].old_text = "  alpha\n    beta".into();
    args.chunks.as_mut().unwrap()[1].new_text = "replacement".into();
    let actual = prepare(text.as_bytes(), &args).unwrap();
    assert!(actual.contains("\r\nreplacement\r\naaaa\r\n"));
    assert!(!actual.contains("alpha"));
    assert!(!actual.contains("beta"));
    // This chunk now targets alpha/beta, so token_14 remains untouched.
    assert!(actual.contains("token_14\r\n"));
    assert!(actual.contains("changed_15\r\nmore\r\n"));
}

#[test]
fn stale_hash_precedes_bulk_matching_and_nul_is_still_rejected() {
    let text = source();
    let mut args = request(&text);
    args.hash = "0".repeat(64);
    assert_eq!(
        prepare(text.as_bytes(), &args),
        Err(("STALE", "File changed since it was read"))
    );
    args.hash = hash(text.as_bytes());
    args.chunks.as_mut().unwrap()[0].new_text = "\0".into();
    assert_eq!(
        prepare(text.as_bytes(), &args),
        Err(("EDIT", "Replacement contains NUL"))
    );
}

use harness_domain::{
    contracts::ReplaceArgs,
    editing::{MAX_FILE, match_hint, prepare, utf8_bom},
    hash,
};

#[cfg(test)]
mod bom_tests {
    use super::*;
    #[test]
    fn bom_selection_preserves_existing_and_never_duplicates_markers() {
        assert_eq!(
            utf8_bom("a.txt", "á", None, Some(b"\xef\xbb\xbfprevious")).unwrap(),
            "\u{feff}á"
        );
        assert_eq!(
            utf8_bom("a.txt", "\u{feff}\u{feff}á", Some(true), None).unwrap(),
            "\u{feff}á"
        );
        assert_eq!(
            utf8_bom("a.cshtml", "\u{feff}á", Some(false), None).unwrap(),
            "á"
        );
        let expected = if cfg!(windows) { "\u{feff}á" } else { "á" };
        assert_eq!(utf8_bom("a.CSHTML", "á", None, None).unwrap(), expected);
        assert_eq!(utf8_bom("a.txt", "á", None, None).unwrap(), "á");
    }

    #[test]
    fn byte_limit_includes_the_encoding_marker() {
        let text = "a".repeat(MAX_FILE - 2);
        assert_eq!(
            utf8_bom("a.txt", &text, Some(true), None).unwrap_err().0,
            "SIZE"
        );
        assert!(utf8_bom("a.txt", &text, Some(false), None).is_ok());
    }

    #[test]
    fn replace_bom_false_round_trips_through_custom_serde() {
        let args: ReplaceArgs = serde_json::from_value(serde_json::json!({
            "path":"a.txt","oldText":"a","newText":"b","bom":false
        }))
        .unwrap();
        assert_eq!(args.bom, Some(false));
        assert_eq!(serde_json::to_value(args).unwrap()["bom"], false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn multi(text: &str, chunks: &[(&str, &str)]) -> ReplaceArgs {
        let mut args = args(text, "", "");
        args.chunks = Some(
            chunks
                .iter()
                .map(|(old, new)| harness_domain::contracts::ReplaceChunk {
                    old_text: (*old).into(),
                    new_text: (*new).into(),
                })
                .collect(),
        );
        args
    }
    #[test]
    fn match_hint_locates_first_diverging_line() {
        let text = "fn a() {}\nfn b() {}\nfn c() {}\n";
        let hint = match_hint(
            text.as_bytes(),
            &args(text, "fn a() {}\nfn B() {}\nfn c() {}", "x"),
        );
        assert_eq!(hint["reason"], "differing_line");
        assert_eq!(hint["firstDivergentLine"], 2);
        assert_eq!(hint["expectedLine"], "fn B() {}");
        assert_eq!(hint["foundLine"], "fn b() {}");
        assert_eq!(
            match_hint(b"x\nx", &args("x\nx", "x", "y"))["reason"],
            "ambiguous"
        );
        assert_eq!(
            match_hint(text.as_bytes(), &args(text, "zzz", "y"))["reason"],
            "first_line_not_found"
        );
    }

    #[test]
    fn wire_modes_are_exclusive_and_round_trip_deletions() {
        for args in [args("abc", "a", ""), multi("abc", &[("a", "")])] {
            let value = serde_json::to_value(&args).unwrap();
            let decoded: ReplaceArgs = serde_json::from_value(value).unwrap();
            assert_eq!(prepare(b"abc", &decoded).unwrap(), "bc");
        }
        for fields in [
            serde_json::json!({"oldText":"a"}),
            serde_json::json!({"chunks":[],"oldText":"","newText":""}),
            serde_json::json!({"chunks":[{"oldText":"a"}]}),
        ] {
            let mut value = serde_json::json!({"path":"file.txt","hash":hash(b"abc")});
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            assert!(serde_json::from_value::<ReplaceArgs>(value).is_err());
        }
        assert_eq!(
            prepare(b"ab", &multi("ab", &[("a", "x"), ("b", "y")])).unwrap(),
            "xy"
        );
        assert_eq!(
            prepare(b"a", &multi("a", &vec![("a", "x"); 129]))
                .unwrap_err()
                .0,
            "EDIT"
        );
        let original = "a".repeat(MAX_FILE - 1) + "b";
        assert_eq!(
            prepare(original.as_bytes(), &multi(&original, &[("b", "bb")]))
                .unwrap_err()
                .0,
            "SIZE"
        );
    }

    #[test]
    fn multiple_chunks_use_original_offsets_and_preserve_eol() {
        let text = "\u{feff}🙂\r\nalpha\r\nbeta\r\ngamma\nend";
        let args = multi(text, &[("gamma", "G\nH"), ("alpha\nbeta", "A\nB")]);
        assert_eq!(
            prepare(text.as_bytes(), &args).unwrap(),
            "\u{feff}🙂\r\nA\r\nB\r\nG\r\nH\nend"
        );
        let args = multi("abc", &[("a", "b"), ("b", "a")]);
        assert_eq!(prepare(b"abc", &args).unwrap(), "bac");
    }

    #[test]
    fn multi_rejects_ambiguity_overlap_missing_and_mixed_modes() {
        for chunks in [
            vec![("aa", "x")],
            vec![("aaa", "x"), ("aa", "y")],
            vec![("missing", "x")],
            vec![("a", "x"), ("a", "y")],
        ] {
            assert_eq!(
                prepare(b"aaa", &multi("aaa", &chunks)).unwrap_err().0,
                "MATCH"
            );
        }
        assert_eq!(
            prepare(b"abc", &multi("abc", &[("ab", "x"), ("bc", "y")]))
                .unwrap_err()
                .0,
            "MATCH"
        );
        let mut args = multi("abc", &[("a", "x")]);
        args.old_text = "a".into();
        assert_eq!(prepare(b"abc", &args).unwrap_err().0, "EDIT");
        assert_eq!(prepare(b"abc", &multi("abc", &[])).unwrap_err().0, "EDIT");
        assert_eq!(
            prepare(b"abc", &multi("abc", &[("a", "\0")]))
                .unwrap_err()
                .0,
            "EDIT"
        );
        assert_eq!(
            prepare(
                b"abc",
                &multi("abc", &[("a", "x"), ("b", &"z".repeat(64000))])
            )
            .unwrap_err()
            .0,
            "EDIT"
        );
        assert_eq!(
            prepare(b"abd", &multi("abc", &[("a", "x")])).unwrap_err().0,
            "STALE"
        );
    }
    fn args(text: &str, old: &str, new: &str) -> ReplaceArgs {
        ReplaceArgs {
            bom: None,
            path: "file.txt".into(),
            hash: hash(text.as_bytes()),
            line: None,
            chunks: None,
            old_text: old.into(),
            new_text: new.into(),
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        }
    }
    #[test]
    fn preserves_bom_unicode_and_matched_newline_style() {
        let text = "\u{feff}🙂\nstart\r\nend\r\ntail";
        assert_eq!(
            prepare(text.as_bytes(), &args(text, "start\nend", "nuevo\nfin")).unwrap(),
            "\u{feff}🙂\nnuevo\r\nfin\r\ntail"
        );
    }
    #[test]
    fn rejects_overlap_and_newline_equivalent_duplicates() {
        for (text, old) in [("aaa", "aa"), ("🙂🙂🙂", "🙂🙂"), ("a\r\nb a\nb", "a\nb")] {
            assert_eq!(
                prepare(text.as_bytes(), &args(text, old, "x"))
                    .unwrap_err()
                    .0,
                "MATCH"
            );
        }
    }
    #[test]
    fn rejects_stale_noop_binary_and_oversized_edits() {
        let mut a = args("x", "x", "y");
        assert_eq!(prepare(b"z", &a).unwrap_err().0, "STALE");
        a.old_text.clear();
        assert_eq!(prepare(b"x", &a).unwrap_err().0, "EDIT");
        assert_eq!(
            prepare(b"x\0", &args("x\0", "x", "y")).unwrap_err().0,
            "ENCODING"
        );
        assert_eq!(
            prepare(b"a\r\nb", &args("a\r\nb", "a\nb", "a\r\nb"))
                .unwrap_err()
                .0,
            "EDIT"
        );
    }

    #[test]
    fn whitespace_and_indentation_tolerant_matching_works() {
        let text = "fn main() {\n    let a = 1;\n    let b = 2;\n}";
        // LLM provides 2 spaces instead of 4
        let old_with_diff_indent = "  let a = 1;\n  let b = 2;";
        let new_code = "    let a = 100;\n    let b = 200;";
        let updated =
            prepare(text.as_bytes(), &args(text, old_with_diff_indent, new_code)).unwrap();
        assert_eq!(
            updated,
            "fn main() {\n    let a = 100;\n    let b = 200;\n}"
        );
    }

    #[test]
    fn direct_replace_binds_unique_text_or_a_validated_line() {
        let source = "🙂\r\n  padding: 5px;\r\n  padding: 5px;\r\nend";
        let mut request = args(source, "padding: 5px;", "padding: 8px;");
        request.hash.clear();
        assert_eq!(prepare(source.as_bytes(), &request).unwrap_err().0, "MATCH");
        request.line = Some(3);
        assert_eq!(
            prepare(source.as_bytes(), &request).unwrap(),
            "🙂\r\n  padding: 5px;\r\n  padding: 8px;\r\nend"
        );
        for line in [1, 4, 5, usize::MAX] {
            request.line = Some(line);
            assert_eq!(prepare(source.as_bytes(), &request).unwrap_err().0, "MATCH");
        }
        request.line = Some(0);
        assert_eq!(prepare(source.as_bytes(), &request).unwrap_err().0, "EDIT");
        request.line = Some(2);
        request.hash = hash(b"stale");
        assert_eq!(prepare(source.as_bytes(), &request).unwrap_err().0, "STALE");
        let mut request = args("abc", "b", "B");
        request.hash.clear();
        assert_eq!(prepare(b"abc", &request).unwrap(), "aBc");
    }

    #[test]
    fn line_replace_preserves_multiline_offsets_and_rejects_ambiguity() {
        let source = "🙂\r\n  a\r\n  b\r\n  a\r\n  b\r\n";
        let mut request = args(source, "a\nb", "A\nB");
        request.hash.clear();
        request.line = Some(4);
        assert_eq!(
            prepare(source.as_bytes(), &request).unwrap(),
            "🙂\r\n  a\r\n  b\r\nA\r\nB\r\n"
        );
        request = args("same same\n", "same", "new");
        request.line = Some(1);
        assert_eq!(prepare(b"same same\n", &request).unwrap_err().0, "MATCH");
        let mut request = args("a\nb\nc\n", "b\n", "B\n");
        request.line = Some(1);
        assert_eq!(prepare(b"a\nb\nc\n", &request).unwrap_err().0, "MATCH");
        request.line = Some(2);
        assert_eq!(prepare(b"a\nb\nc\n", &request).unwrap(), "a\nB\nc\n");
        let mut request = args("a\nb\n", "\n", "!");
        request.line = Some(1);
        assert_eq!(prepare(b"a\nb\n", &request).unwrap(), "a!b\n");
        let mut request = multi("abc", &[("a", "A")]);
        request.line = Some(1);
        assert_eq!(prepare(b"abc", &request).unwrap_err().0, "EDIT");
    }
}

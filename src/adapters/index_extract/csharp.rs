use harness_domain::Failure;

// Port of languages/csharp.ts blankCsharpPreprocessorDirectives. JS replaces
// each UTF-16 unit (including CR) with one space; both conditional branches stay.
pub(super) fn blank_csharp(source: &str) -> Result<String, Failure> {
    if source.contains(['\u{2028}', '\u{2029}']) || source.replace("\r\n", "").contains('\r') {
        return Err((
            "EXTRACTION_DEFERRED",
            "Unported C# preprocessor line separator",
        ));
    }
    let mut result = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let tail = body.trim_start_matches([' ', '\t']);
        let directive = tail
            .strip_prefix('#')
            .map(|s| s.trim_start_matches([' ', '\t']));
        let conditional = directive.is_some_and(|s| {
            ["if", "elif", "else", "endif"].iter().any(|word| {
                s.strip_prefix(word).is_some_and(|rest| {
                    !rest
                        .as_bytes()
                        .first()
                        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                })
            })
        });
        if conditional {
            let indent = body.len() - tail.len();
            result.push_str(&body[..indent]);
            result.push_str(&" ".repeat(tail.encode_utf16().count()));
        } else {
            result.push_str(body);
        }
        if line.ends_with('\n') {
            result.push('\n');
        }
    }
    Ok(result)
}

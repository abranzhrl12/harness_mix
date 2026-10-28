use super::*;

#[test]
fn monotonic_offsets_match_independent_normalized_prefixes() {
    for text in [
        "",
        "\r\n",
        "🙂\r\na\nñ\r\nb\r",
        "\r\n\r\nlast\r\n",
        "no newlines",
    ] {
        let normalized = normalize(text);
        let mut offsets = OriginalOffsets::default();
        for offset in 0..=normalized.len() {
            let expected = (0..=text.len())
                .find(|&raw| {
                    // Only compare complete normalization boundaries, not half a CRLF.
                    !(raw > 0
                        && text.as_bytes().get(raw - 1) == Some(&b'\r')
                        && text.as_bytes().get(raw) == Some(&b'\n'))
                        && raw
                            - text.as_bytes()[..raw]
                                .windows(2)
                                .filter(|pair| *pair == b"\r\n")
                                .count()
                            == offset
                })
                .unwrap();
            assert_eq!(
                offsets.at(text.as_bytes(), offset),
                expected,
                "{text:?}, offset={offset}"
            );
        }
    }
}

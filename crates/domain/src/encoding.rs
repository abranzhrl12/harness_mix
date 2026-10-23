//! Decoding for source files whose historical Windows encoding predates UTF-8.
use std::borrow::Cow;

use crate::Failure;

/// Decode UTF-8 source as-is, with Windows-1252 as a fallback for legacy files.
/// UTF-16/32 and binary-looking data stay errors instead of being reinterpreted.
pub fn decode_source(bytes: &[u8]) -> Result<Cow<'_, str>, Failure> {
    const ENCODING: Failure = ("ENCODING", "Source is neither UTF-8 nor valid Windows-1252 text");

    if bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
        || bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff])
        || bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00])
        || bytes.contains(&0)
    {
        return Err(ENCODING);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(Cow::Borrowed(text));
    }

    let (text, had_errors) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
    if had_errors || text.chars().any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r')) {
        return Err(ENCODING);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::decode_source;
    use std::borrow::Cow;

    #[test]
    fn preserves_utf8_and_decodes_windows_1252_accents() {
        assert!(matches!(decode_source("acción 😀".as_bytes()).unwrap(), Cow::Borrowed(_)));
        assert_eq!(decode_source(b"acci\xf3n y ni\xf1ez").unwrap(), "acción y niñez");
        assert_eq!(decode_source(b"precio \x80").unwrap(), "precio €");
    }

    #[test]
    fn rejects_utf16_and_binary_data_instead_of_guessing_windows_1252() {
        for bytes in [
            &[0xff, 0xfe, b'a', 0][..],
            &[0xfe, 0xff, 0, b'a'][..],
            &[b'a', 0, b'b'][..],
            &[0x81][..],
        ] {
            assert_eq!(decode_source(bytes).unwrap_err().0, "ENCODING");
        }
    }
}

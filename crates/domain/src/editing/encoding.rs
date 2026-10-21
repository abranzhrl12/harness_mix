use crate::Failure;

pub const MAX_FILE: usize = 2 * 1024 * 1024;

/// Select the on-disk UTF-8 representation before hashing or publishing it.
/// A single leading marker is encoding metadata, not part of the document body.
pub fn utf8_bom(
    path: &str,
    text: &str,
    bom: Option<bool>,
    original: Option<&[u8]>,
) -> Result<String, Failure> {
    let enabled = bom.unwrap_or_else(|| {
        original.is_some_and(|bytes| bytes.starts_with(b"\xef\xbb\xbf"))
            || text.starts_with('\u{feff}')
            || (cfg!(windows)
                && std::path::Path::new(path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("cshtml")))
    });
    let body = text.trim_start_matches('\u{feff}');
    let size = body.len() + if enabled { 3 } else { 0 };
    if size > MAX_FILE {
        return Err(("SIZE", "Updated file exceeds 2 MiB including UTF-8 BOM"));
    }
    let mut output = String::with_capacity(size);
    if enabled {
        output.push('\u{feff}');
    }
    output.push_str(body);
    Ok(output)
}

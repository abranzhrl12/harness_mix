use std::{
    fmt,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use docx_rs::{
    AlignmentType, BreakType, Docx, Footer, Header, Paragraph, Pic, Run, RunFonts, Table, TableCell,
    TableRow,
};
use harness_domain::word::{Block, WordDocument};

/// Errors returned while validating or generating a DOCX document.
#[derive(Debug)]
pub enum WordRenderError {
    InvalidDocument(String),
    UnsupportedImage(String),
    Render(String),
}

impl fmt::Display for WordRenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDocument(message) => write!(f, "invalid Word document: {message}"),
            Self::UnsupportedImage(message) => write!(f, "unsupported image: {message}"),
            Self::Render(message) => write!(f, "failed to render Word document: {message}"),
        }
    }
}

impl std::error::Error for WordRenderError {}

/// Render a structured Word document into an in-memory DOCX package.
pub fn render_docx(document: &WordDocument) -> Result<Vec<u8>, WordRenderError> {
    validate_document(document)?;

    let mut docx = Docx::new();

    if let Some(header) = &document.header {
        docx = docx.header(Header::new().add_paragraph(plain_paragraph(header)));
    }
    if let Some(footer) = &document.footer {
        docx = docx.footer(Footer::new().add_paragraph(plain_paragraph(footer)));
    }

    if let Some(cover) = &document.cover {
        docx = add_cover(docx, cover);
    } else if let Some(title) = &document.title {
        docx = docx.add_paragraph(
            Paragraph::new()
                .style("Title")
                .align(AlignmentType::Center)
                .add_run(Run::new().add_text(title)),
        );
    }

    if let Some(author) = document
        .cover
        .as_ref()
        .and_then(|cover| cover.author.as_deref())
        .or(document.author.as_deref())
    {
        docx = docx.custom_property("Author", author);
    }
    if let Some(title) = document
        .cover
        .as_ref()
        .map(|cover| cover.title.as_str())
        .or(document.title.as_deref())
    {
        docx = docx.custom_property("Title", title);
    }

    for block in &document.blocks {
        docx = add_block(docx, block)?;
    }

    let mut bytes = Vec::new();
    docx.build()
        .pack(Cursor::new(&mut bytes))
        .map_err(|error| WordRenderError::Render(error.to_string()))?;
    Ok(bytes)
}

fn validate_document(document: &WordDocument) -> Result<(), WordRenderError> {
    for block in &document.blocks {
        match block {
            Block::Heading { level, .. } if !(1..=6).contains(level) => {
                return Err(WordRenderError::InvalidDocument(format!(
                    "heading level must be between 1 and 6, got {level}"
                )));
            }
            Block::Table { headers, rows } => {
                let column_count = headers.len().max(rows.first().map_or(0, Vec::len));
                if column_count == 0 {
                    return Err(WordRenderError::InvalidDocument(
                        "table must contain a header or at least one non-empty row".into(),
                    ));
                }
                if !headers.is_empty() && headers.len() != column_count {
                    return Err(WordRenderError::InvalidDocument(
                        "table headers and rows must have the same number of columns".into(),
                    ));
                }
                if rows.iter().any(|row| row.len() != column_count) {
                    return Err(WordRenderError::InvalidDocument(
                        "every table row must have the same number of columns".into(),
                    ));
                }
            }
            Block::List { items, .. } if items.is_empty() => {
                return Err(WordRenderError::InvalidDocument(
                    "list must contain at least one item".into(),
                ));
            }
            Block::Image { data_base64, mime_type, .. } => {
                decode_image(data_base64, mime_type)?;
            }
            Block::CodeImage { code, .. } if code.trim().is_empty() => {
                return Err(WordRenderError::InvalidDocument(
                    "code image cannot be generated from empty code".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn add_cover(mut docx: Docx, cover: &harness_domain::word::CoverPage) -> Docx {
    docx = docx.add_paragraph(
        Paragraph::new()
            .style("Title")
            .align(AlignmentType::Center)
            .add_run(Run::new().add_text(&cover.title)),
    );
    if let Some(subtitle) = &cover.subtitle {
        docx = docx.add_paragraph(
            Paragraph::new()
                .style("Subtitle")
                .align(AlignmentType::Center)
                .add_run(Run::new().add_text(subtitle)),
        );
    }
    if let Some(author) = &cover.author {
        docx = docx.add_paragraph(
            Paragraph::new()
                .align(AlignmentType::Center)
                .add_run(Run::new().add_text(author)),
        );
    }
    if let Some(date) = &cover.date {
        docx = docx.add_paragraph(
            Paragraph::new()
                .align(AlignmentType::Center)
                .add_run(Run::new().add_text(date)),
        );
    }
    docx.add_paragraph(
        Paragraph::new().add_run(Run::new().add_break(BreakType::Page)),
    )
}

fn add_block(mut docx: Docx, block: &Block) -> Result<Docx, WordRenderError> {
    match block {
        Block::Paragraph { text, bold, italic } => {
            let mut run = Run::new().add_text(text);
            if *bold {
                run = run.bold();
            }
            if *italic {
                run = run.italic();
            }
            docx = docx.add_paragraph(Paragraph::new().add_run(run));
        }
        Block::Heading { level, text } => {
            let style = format!("Heading{level}");
            docx = docx.add_paragraph(
                Paragraph::new()
                    .style(&style)
                    .add_run(Run::new().add_text(text)),
            );
        }
        Block::List { ordered, items } => {
            for (index, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", index + 1)
                } else {
                    "• ".to_owned()
                };
                docx = docx.add_paragraph(
                    Paragraph::new().add_run(Run::new().add_text(format!("{marker}{item}"))),
                );
            }
        }
        Block::Table { headers, rows } => {
            let mut table_rows = Vec::with_capacity(rows.len() + if headers.is_empty() { 0 } else { 1 });
            if !headers.is_empty() {
                table_rows.push(table_row(headers, true));
            }
            table_rows.extend(rows.iter().map(|row| table_row(row, false)));
            docx = docx.add_table(Table::new(table_rows));
        }
        Block::Code { text, .. } => {
            for line in text.lines() {
                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .size(20)
                            .fonts(RunFonts::new().ascii("Consolas").hi_ansi("Consolas"))
                            .add_text(line),
                    ),
                );
            }
            if text.is_empty() {
                docx = docx.add_paragraph(Paragraph::new());
            }
        }
        Block::Image { data_base64, mime_type, width_px, .. } => {
            let bytes = decode_image(data_base64, mime_type)?;
            let pic = make_picture(&bytes, *width_px)?;
            docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_image(pic)));
        }
        Block::CodeImage { code, language, width_px, .. } => {
            let bytes = crate::adapters::code_snapshot::render_code_image(code, language.as_deref())
                .map_err(|error| WordRenderError::Render(error.to_string()))?;
            let pic = make_picture(&bytes, *width_px)?;
            docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_image(pic)));
        }
        Block::PageBreak => {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_break(BreakType::Page)),
            );
        }
    }
    Ok(docx)
}

fn plain_paragraph(text: &str) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text))
}

fn table_row(values: &[String], is_header: bool) -> TableRow {
    TableRow::new(
        values
            .iter()
            .map(|value| {
                let mut run = Run::new().add_text(value);
                if is_header {
                    run = run.bold();
                }
                TableCell::new().add_paragraph(Paragraph::new().add_run(run))
            })
            .collect(),
    )
}

fn decode_image(data_base64: &str, mime_type: &str) -> Result<Vec<u8>, WordRenderError> {
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|error| WordRenderError::UnsupportedImage(error.to_string()))?;
    let signature_matches = match mime_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF8"),
        "image/bmp" => bytes.starts_with(b"BM"),
        other => {
            return Err(WordRenderError::UnsupportedImage(format!(
                "MIME type {other:?} is not supported"
            )));
        }
    };
    if !signature_matches {
        return Err(WordRenderError::UnsupportedImage(format!(
            "data does not match MIME type {mime_type:?}"
        )));
    }
    if bytes.is_empty() {
        return Err(WordRenderError::UnsupportedImage("image is empty".into()));
    }
    Ok(bytes)
}

fn make_picture(bytes: &[u8], width_px: Option<u32>) -> Result<Pic, WordRenderError> {
    let pic = catch_unwind(AssertUnwindSafe(|| Pic::new(bytes))).map_err(|_| {
        WordRenderError::UnsupportedImage("image data could not be decoded".into())
    })?;
    let Some(width_px) = width_px else {
        return Ok(pic);
    };
    if width_px == 0 {
        return Err(WordRenderError::InvalidDocument(
            "image width_px must be greater than zero".into(),
        ));
    }

    let (source_width, source_height) = pic.size;
    if source_width == 0 {
        return Err(WordRenderError::UnsupportedImage(
            "decoded image has zero width".into(),
        ));
    }
    let width_emu = width_px.saturating_mul(9_525);
    let height_emu = ((u64::from(source_height) * u64::from(width_emu))
        / u64::from(source_width))
        .min(u64::from(u32::MAX)) as u32;
    Ok(pic.size(width_emu, height_emu))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_valid_docx_package_with_document_content() {
        let document = WordDocument {
            title: Some("Quarterly report".into()),
            author: Some("Harness".into()),
            cover: None,
            header: Some("Internal".into()),
            footer: Some("Page".into()),
            blocks: vec![
                Block::Heading { level: 1, text: "Summary".into() },
                Block::Paragraph { text: "Revenue increased.".into(), bold: false, italic: false },
                Block::List { ordered: false, items: vec!["Cash flow".into()] },
                Block::Table { headers: vec!["Metric".into()], rows: vec![vec!["Revenue".into()]] },
                Block::Code { text: "fn main() {}".into(), language: Some("rust".into()) },
                Block::CodeImage {
                    code: "fn main() {}".into(),
                    language: Some("rust".into()),
                    alt: Some("Rust main function".into()),
                    width_px: Some(500),
                },
                Block::PageBreak,
            ],
        };

        let bytes = render_docx(&document).expect("document should render");
        assert!(bytes.len() > 256, "expected a non-empty Office Open XML package");
        assert_eq!(&bytes[..4], b"PK\x03\x04", "DOCX must be a ZIP package");
    }

    #[test]
    fn rejects_invalid_heading_levels_before_rendering() {
        let document = WordDocument {
            title: None,
            author: None,
            cover: None,
            header: None,
            footer: None,
            blocks: vec![Block::Heading { level: 0, text: "Invalid".into() }],
        };

        assert!(matches!(
            render_docx(&document),
            Err(WordRenderError::InvalidDocument(message)) if message.contains("heading level")
        ));
    }

    #[test]
    fn rejects_mismatched_image_mime_and_data() {
        let document = WordDocument {
            title: None,
            author: None,
            cover: None,
            header: None,
            footer: None,
            blocks: vec![Block::Image {
                data_base64: STANDARD.encode(b"not an image"),
                mime_type: "image/png".into(),
                alt: None,
                width_px: None,
            }],
        };

        assert!(matches!(
            render_docx(&document),
            Err(WordRenderError::UnsupportedImage(message)) if message.contains("does not match")
        ));
    }
}

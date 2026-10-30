pub(crate) use crate::terminal::console::ClipboardContent;
use harness_domain::conversation::ImageAttachment;

const MAX_CLIPBOARD_BYTES: usize = 20 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4_096;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

enum RawClipboardContent {
    Png(Vec<u8>),
    Dib(Vec<u8>),
    Text(String),
}

trait ClipboardProvider {
    fn read_content(&self) -> Result<Option<RawClipboardContent>, String>;
}

/// Reads the clipboard only after the caller receives an explicit Ctrl+V event.
pub async fn read_content() -> Result<Option<ClipboardContent>, String> {
    tokio::task::spawn_blocking(read_content_blocking)
        .await
        .map_err(|error| format!("No se pudo leer el portapapeles: {error}"))?
}

#[cfg(windows)]
fn read_content_blocking() -> Result<Option<ClipboardContent>, String> {
    read_with_provider(&windows_clipboard::WindowsClipboardProvider)
}

#[cfg(not(windows))]
fn read_content_blocking() -> Result<Option<ClipboardContent>, String> {
    read_with_provider(&NoClipboardProvider)
}

fn read_with_provider(
    provider: &dyn ClipboardProvider,
) -> Result<Option<ClipboardContent>, String> {
    let Some(content) = provider.read_content()? else {
        return Ok(None);
    };
    match content {
        RawClipboardContent::Png(bytes) => Ok(Some(ClipboardContent::Image(ImageAttachment {
            mime_type: "image/png".into(),
            data: validate_png(bytes)?,
        }))),
        RawClipboardContent::Dib(bytes) => Ok(Some(ClipboardContent::Image(ImageAttachment {
            mime_type: "image/png".into(),
            data: decode_and_encode_png(&wrap_dib_as_bmp(&bytes)?)?,
        }))),
        RawClipboardContent::Text(text) => Ok(Some(ClipboardContent::Text(text))),
    }
}

#[cfg(not(windows))]
struct NoClipboardProvider;
#[cfg(not(windows))]
impl ClipboardProvider for NoClipboardProvider {
    fn read_content(&self) -> Result<Option<RawClipboardContent>, String> {
        Ok(None)
    }
}

fn wrap_dib_as_bmp(dib: &[u8]) -> Result<Vec<u8>, String> {
    if dib.len() < 40 || dib.len() > MAX_CLIPBOARD_BYTES {
        return Err("El bitmap del portapapeles no es válido o supera 20 MiB".into());
    }
    let header_size = u32::from_le_bytes(dib[0..4].try_into().unwrap()) as usize;
    if header_size < 40 || header_size > dib.len() {
        return Err("La cabecera bitmap del portapapeles no es válida".into());
    }
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(dib[16..20].try_into().unwrap());
    let colors_used = u32::from_le_bytes(dib[32..36].try_into().unwrap()) as usize;
    let palette_entries = if colors_used != 0 {
        colors_used
    } else if bit_count <= 8 {
        1usize << bit_count
    } else {
        0
    };
    let masks = if header_size == 40 && (compression == 3 || compression == 6) {
        if compression == 6 { 16 } else { 12 }
    } else {
        0
    };
    let palette_bytes = palette_entries
        .checked_mul(4)
        .ok_or("Tabla de colores bitmap no válida")?;
    let pixel_offset = 14usize
        .checked_add(header_size)
        .and_then(|n| n.checked_add(masks))
        .and_then(|n| n.checked_add(palette_bytes))
        .ok_or("Tamaño bitmap no válido")?;
    let file_size = 14usize
        .checked_add(dib.len())
        .ok_or("Tamaño bitmap no válido")?;
    if pixel_offset > file_size || file_size > MAX_CLIPBOARD_BYTES || file_size > u32::MAX as usize
    {
        return Err("La cabecera bitmap está incompleta o supera 20 MiB".into());
    }
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

fn decode_and_encode_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_CLIPBOARD_BYTES {
        return Err("La imagen del portapapeles supera 20 MiB".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "No se pudo reconocer la imagen del portapapeles".to_owned())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "La imagen está dañada o supera los límites admitidos".to_owned())?;
    if u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS {
        return Err("La imagen del portapapeles supera 16 megapíxeles".into());
    }
    let mut output = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(BoundedWriter(&mut output)),
        image.as_bytes(),
        image.width(),
        image.height(),
        image.color().into(),
    )
    .map_err(|_| "No se pudo codificar la imagen del portapapeles".to_owned())?;
    Ok(output)
}

struct BoundedWriter<'a>(&'a mut Vec<u8>);
impl std::io::Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = MAX_CLIPBOARD_BYTES.saturating_sub(self.0.len());
        if bytes.len() > remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "PNG output exceeds clipboard size limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn validate_png(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("El formato PNG del portapapeles no contiene datos PNG válidos".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "No se pudo reconocer la imagen PNG del portapapeles".to_owned())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "La imagen PNG está dañada o supera los límites admitidos".to_owned())?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > MAX_PIXELS {
        return Err("La imagen del portapapeles supera 16 megapíxeles".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
mod windows_clipboard {
    use super::{ClipboardProvider, MAX_CLIPBOARD_BYTES, MAX_TEXT_BYTES, RawClipboardContent};
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };

    struct ClipboardGuard;
    impl ClipboardGuard {
        fn open() -> Result<Self, String> {
            unsafe { OpenClipboard(None) }
                .map(|_| Self)
                .map_err(|_| "El portapapeles está ocupado o no disponible".to_owned())
        }
    }
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    pub(super) struct WindowsClipboardProvider;
    impl ClipboardProvider for WindowsClipboardProvider {
        fn read_content(&self) -> Result<Option<RawClipboardContent>, String> {
            read_system_clipboard()
        }
    }

    fn read_system_clipboard() -> Result<Option<RawClipboardContent>, String> {
        let _guard = ClipboardGuard::open()?;
        let png_format = unsafe { RegisterClipboardFormatW(windows::core::w!("PNG")) };
        if png_format == 0 {
            return Err("No se pudo consultar el formato PNG del portapapeles".into());
        }
        if let Some(bytes) = read_global(png_format)? {
            return Ok(Some(RawClipboardContent::Png(bytes)));
        }
        for format in [17u32, 8u32] {
            if let Some(bytes) = read_global(format)? {
                return Ok(Some(RawClipboardContent::Dib(bytes)));
            }
        }
        if let Some(bytes) = read_global(13)? {
            if bytes.len() > MAX_TEXT_BYTES {
                return Err("El texto del portapapeles supera 4 MiB".into());
            }
            let units = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|unit| *unit != 0)
                .collect::<Vec<_>>();
            return Ok(Some(RawClipboardContent::Text(String::from_utf16_lossy(
                &units,
            ))));
        }
        Ok(None)
    }

    fn read_global(format: u32) -> Result<Option<Vec<u8>>, String> {
        if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
            return Ok(None);
        }
        let handle = unsafe { GetClipboardData(format) }
            .map_err(|_| "No se pudo obtener el contenido del portapapeles".to_owned())?;
        let global = HGLOBAL(handle.0);
        let size = unsafe { GlobalSize(global) };
        let limit = if format == 13 {
            MAX_TEXT_BYTES
        } else {
            MAX_CLIPBOARD_BYTES
        };
        if size == 0 || size > limit {
            return Err("El contenido del portapapeles supera el límite permitido".into());
        }
        let locked = unsafe { GlobalLock(global) };
        if locked.is_null() {
            return Err("No se pudo leer el contenido del portapapeles".into());
        }
        let guard = GlobalUnlockGuard(global);
        let bytes = unsafe { std::slice::from_raw_parts(locked.cast::<u8>(), size) }.to_vec();
        drop(guard);
        Ok(Some(bytes))
    }

    struct GlobalUnlockGuard(HGLOBAL);
    impl Drop for GlobalUnlockGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = GlobalUnlock(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedWriter, ClipboardContent, ClipboardProvider, MAX_CLIPBOARD_BYTES,
        RawClipboardContent, decode_and_encode_png, read_with_provider, validate_png,
        wrap_dib_as_bmp,
    };
    use image::ImageEncoder;

    struct FakeClipboard(Result<Option<RawClipboardContent>, String>);
    impl ClipboardProvider for FakeClipboard {
        fn read_content(&self) -> Result<Option<RawClipboardContent>, String> {
            Ok(match self.0.as_ref().map_err(|error| error.clone())? {
                Some(RawClipboardContent::Text(text)) => {
                    Some(RawClipboardContent::Text(text.clone()))
                }
                Some(RawClipboardContent::Png(bytes)) => {
                    Some(RawClipboardContent::Png(bytes.clone()))
                }
                Some(RawClipboardContent::Dib(bytes)) => {
                    Some(RawClipboardContent::Dib(bytes.clone()))
                }
                None => None,
            })
        }
    }

    fn tiny_png() -> Vec<u8> {
        let mut output = Vec::new();
        image::codecs::png::PngEncoder::new(&mut output)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        output
    }

    fn dib(
        width: i32,
        height: i32,
        bit_count: u16,
        compression: u32,
        colors_used: u32,
        extra: &[u8],
        pixels: &[u8],
    ) -> Vec<u8> {
        let row_stride = (((width.unsigned_abs() as usize * bit_count as usize + 31) / 32) * 4)
            .max(pixels.len());
        let mut data = Vec::new();
        data.extend_from_slice(&40u32.to_le_bytes());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&bit_count.to_le_bytes());
        data.extend_from_slice(&compression.to_le_bytes());
        data.extend_from_slice(&(row_stride as u32).to_le_bytes());
        data.extend_from_slice(&[0; 8]);
        data.extend_from_slice(&colors_used.to_le_bytes());
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(extra);
        data.extend_from_slice(pixels);
        data
    }

    #[test]
    fn injected_provider_returns_text_without_accessing_the_system_clipboard() {
        let provider = FakeClipboard(Ok(Some(RawClipboardContent::Text("hello".into()))));
        assert!(matches!(
            read_with_provider(&provider).unwrap(),
            Some(ClipboardContent::Text(text)) if text == "hello"
        ));
    }

    #[test]
    fn png_clipboard_bytes_are_validated_and_preserved_without_reencoding() {
        let bytes = tiny_png();
        assert_eq!(validate_png(bytes.clone()).unwrap(), bytes);
        assert!(validate_png(vec![0; MAX_CLIPBOARD_BYTES + 1]).is_err());
        assert!(validate_png(b"not png".to_vec()).is_err());
    }

    #[test]
    fn dib_24_bit_top_down_and_bottom_up_are_decoded_to_png() {
        for height in [1, -1] {
            let raw = dib(1, height, 24, 0, 0, &[], &[0, 0, 255, 0]);
            let bmp = wrap_dib_as_bmp(&raw).unwrap();
            let png = decode_and_encode_png(&bmp).unwrap();
            let decoded =
                image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (1, 1));
        }
    }

    #[test]
    fn dib_bitfields_and_palette_offsets_are_accounted_for() {
        let masks = [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let bitfields = dib(1, 1, 32, 3, 0, &masks, &[0, 0, 255, 255]);
        assert!(decode_and_encode_png(&wrap_dib_as_bmp(&bitfields).unwrap()).is_ok());

        let palette = [0u8, 0, 255, 0];
        let indexed = dib(1, 1, 8, 0, 1, &palette, &[0, 0, 0, 0]);
        assert!(decode_and_encode_png(&wrap_dib_as_bmp(&indexed).unwrap()).is_ok());
    }

    #[test]
    fn rejects_malformed_and_oversized_dib_headers_before_decoding() {
        assert!(wrap_dib_as_bmp(&[0; 39]).is_err());
        let mut oversized_header = vec![0; 40];
        oversized_header[..4].copy_from_slice(&124u32.to_le_bytes());
        assert!(wrap_dib_as_bmp(&oversized_header).is_err());

        let too_wide = dib(4_097, 1, 24, 0, 0, &[], &[0; 12_292]);
        assert!(decode_and_encode_png(&wrap_dib_as_bmp(&too_wide).unwrap()).is_err());
    }

    #[test]
    fn bounded_png_writer_never_accepts_bytes_past_the_output_limit() {
        let mut output = vec![0; MAX_CLIPBOARD_BYTES - 1];
        let mut writer = BoundedWriter(&mut output);
        use std::io::Write;
        assert_eq!(writer.write(&[1]).unwrap(), 1);
        assert!(writer.write(&[2]).is_err());
    }

    #[test]
    fn injected_provider_errors_are_returned_to_the_input_loop() {
        let provider = FakeClipboard(Err("busy".into()));
        assert_eq!(read_with_provider(&provider).unwrap_err(), "busy");
    }
}

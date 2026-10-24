use harness_domain::conversation::ImageAttachment;
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
};

const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Loads a local image without blocking the async runtime or reading clipboard data.
pub async fn load_image(path: PathBuf) -> Result<ImageAttachment, String> {
    tokio::task::spawn_blocking(move || load_image_blocking(&path))
        .await
        .map_err(|error| format!("No se pudo cargar la imagen: {error}"))?
}

fn load_image_blocking(path: &PathBuf) -> Result<ImageAttachment, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("No se pudo abrir la imagen '{}': {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("La ruta no es un archivo: {}", path.display()));
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "La imagen supera el límite de 20 MiB: {}",
            path.display()
        ));
    }

    // Bound the actual read too, in case the file grows after metadata was checked.
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("No se pudo leer la imagen '{}': {error}", path.display()))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!(
            "La imagen supera el límite de 20 MiB: {}",
            path.display()
        ));
    }

    let mime_type = detect_mime_type(&bytes).ok_or_else(|| {
        format!(
            "Formato de imagen no compatible en '{}'. Se aceptan PNG, JPEG, GIF y WebP; SVG no está soportado.",
            path.display()
        )
    })?;

    Ok(ImageAttachment {
        mime_type: mime_type.to_string(),
        data: bytes,
    })
}

fn detect_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_IMAGE_BYTES, load_image};
    use std::{fs, path::PathBuf};

    #[tokio::test]
    async fn detects_supported_formats_from_files_whose_paths_contain_spaces() {
        let directory = tempfile::tempdir().unwrap();
        let cases: [(&str, &[u8], &str); 4] = [
            ("png", b"\x89PNG\r\n\x1a\nbody", "image/png"),
            ("jpg", b"\xff\xd8\xffbody", "image/jpeg"),
            ("gif", b"GIF89abody", "image/gif"),
            ("webp", b"RIFFxxxxWEBPbody", "image/webp"),
        ];

        for (extension, bytes, expected_mime) in cases {
            let path = directory
                .path()
                .join(format!("image with spaces.{extension}"));
            fs::write(&path, bytes).unwrap();
            let image = load_image(path).await.unwrap();
            assert_eq!(image.mime_type, expected_mime);
            assert_eq!(image.data, bytes);
        }
    }

    #[tokio::test]
    async fn rejects_invalid_data_and_svg_with_a_readable_supported_formats_message() {
        let directory = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("invalid.bin", b"not an image".as_slice()),
            ("vector.svg", b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".as_slice()),
        ] {
            let path = directory.path().join(name);
            fs::write(&path, bytes).unwrap();
            let error = load_image(path).await.unwrap_err();
            assert!(error.contains("PNG, JPEG, GIF y WebP"), "{error}");
            assert!(error.contains("SVG no está soportado"), "{error}");
        }
    }

    #[tokio::test]
    async fn rejects_oversized_sparse_files_before_loading_their_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path: PathBuf = directory.path().join("large sparse image.png");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();

        let error = load_image(path).await.unwrap_err();
        assert!(error.contains("20 MiB"), "{error}");
    }
}

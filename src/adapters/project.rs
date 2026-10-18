use harness_domain::execution::OperationContext;
pub use harness_domain::{Failure, hash};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_FILE: u64 = 2 * 1024 * 1024;

pub(super) fn external_sensitive(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or("");
    name.contains(".config")
        || name.starts_with(".npmrc")
        || (name.starts_with("appsettings") && name.contains(".json"))
        || ["pem", "key", "pfx", "p12"]
            .iter()
            .any(|ext| name.ends_with(&format!(".{ext}")))
}

pub(super) fn external_restricted(relative: &str) -> bool {
    relative.replace('\\', "/").split('/').any(|part| {
        let name = part.to_ascii_lowercase();
        [".git", ".hg", ".svn"].contains(&name.as_str())
            || [
                ".env",
                "credential",
                "secret",
                "mcp_config.json",
                "oauth_creds.json",
                "id_rsa",
                "id_ed25519",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    })
}

#[derive(Clone)]
pub struct Project {
    pub root: PathBuf,
    pub(crate) external: bool,
    pub(crate) single_file: Option<PathBuf>,
}

impl Project {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::other("workspace must be a directory"));
        }
        Ok(Self {
            root,
            external: false,
            single_file: None,
        })
    }

    pub(crate) fn target(&self, relative: &str) -> Result<PathBuf, Failure> {
        self.resolve_target(relative, false)
    }

    /// Read-only Git may address deleted files; validate every existing ancestor.
    pub(crate) fn target_optional(&self, relative: &str) -> Result<PathBuf, Failure> {
        self.resolve_target(relative, true)
    }

    fn resolve_target(&self, relative: &str, allow_missing: bool) -> Result<PathBuf, Failure> {
        if relative.is_empty()
            || relative.contains([':', '\0'])
            || relative.starts_with(['/', '\\'])
        {
            return Err(("PATH", "ruta relativa inválida"));
        }
        // Treat Windows separators identically on every platform.
        let normalized = relative.replace('\\', "/");
        if self.external && external_restricted(&normalized) {
            return Err(("PATH", "Restricted external credential or metadata path"));
        }
        if self.external && Path::new(&normalized).components().count() > 64 {
            return Err(("PATH", "External path depth exceeds 64"));
        }
        let mut target = self.root.clone();
        for component in Path::new(&normalized).components() {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => {
                    let name = part.to_string_lossy().to_lowercase();
                    let base = name.split('.').next().unwrap_or("");
                    // Windows aliases must not bypass secret checks or open device files.
                    if name.ends_with([' ', '.'])
                        || name.contains(['<', '>', '"', '|', '?', '*'])
                        || ["con", "prn", "aux", "nul", "conin$", "conout$"].contains(&base)
                        || (base.len() == 4
                            && (base.starts_with("com") || base.starts_with("lpt"))
                            && matches!(base.as_bytes()[3], b'1'..=b'9'))
                    {
                        return Err(("PATH", "alias o dispositivo Windows no admitido"));
                    }
                    if name == ".env"
                        || name.starts_with(".env.")
                        || ["credential", "credentials", "secret", "secrets"].contains(&base)
                        || [
                            "id_rsa",
                            "id_ed25519",
                            "oauth_creds.json",
                            "mcp_config.json",
                        ]
                        .contains(&name.as_str())
                    {
                        return Err(("PATH", "ruta restringida"));
                    }
                    target.push(part);
                    let meta = match fs::symlink_metadata(&target) {
                        Ok(meta) => meta,
                        Err(error)
                            if allow_missing && error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            continue;
                        }
                        Err(_) => return Err(("PATH", "archivo no accesible")),
                    };
                    if meta.file_type().is_symlink() {
                        return Err(("PATH", "no se admiten enlaces en la ruta"));
                    }
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if meta.file_attributes() & 0x400 != 0 {
                            return Err(("PATH", "no se admiten reparse points"));
                        }
                    }
                }
                _ => return Err(("PATH", "ruta restringida o fuera del proyecto")),
            }
        }
        let existing = if allow_missing && !target.exists() {
            target
                .ancestors()
                .skip(1)
                .find(|path| path.exists())
                .ok_or(("PATH", "directorio no accesible"))?
        } else {
            target.as_path()
        };
        let real = existing
            .canonicalize()
            .map_err(|_| ("PATH", "archivo no accesible"))?;
        if self
            .single_file
            .as_ref()
            .is_some_and(|single| *single != real)
        {
            return Err(("PATH", "Scope authorizes only its registered file"));
        }
        if !real.starts_with(&self.root) {
            return Err(("PATH", "destino fuera del proyecto"));
        }
        Ok(target)
    }

    pub fn bytes(&self, relative: &str) -> Result<Vec<u8>, Failure> {
        self.bytes_with_context(relative, &OperationContext::default())
    }
    pub fn bytes_with_context(
        &self,
        relative: &str,
        context: &OperationContext,
    ) -> Result<Vec<u8>, Failure> {
        self.bytes_with_limit(relative, context, MAX_FILE)
    }
    pub(crate) fn bytes_with_limit(
        &self,
        relative: &str,
        context: &OperationContext,
        limit: u64,
    ) -> Result<Vec<u8>, Failure> {
        self.input_bytes(relative, context, limit, false)
    }
    pub(crate) fn redacted_input_bytes(
        &self,
        relative: &str,
        context: &OperationContext,
        limit: u64,
    ) -> Result<Vec<u8>, Failure> {
        self.input_bytes(relative, context, limit, true)
    }
    fn input_bytes(
        &self,
        relative: &str,
        context: &OperationContext,
        limit: u64,
        redacted: bool,
    ) -> Result<Vec<u8>, Failure> {
        if self.external && !redacted && external_sensitive(relative) {
            return Err((
                "PATH",
                "External configuration/credential content requires the redacted config tool",
            ));
        }
        let limit = limit.min(MAX_FILE);
        context.check()?;
        let path = self.target(relative)?;
        #[cfg(windows)]
        let _pins = if self.external {
            Some(super::edit_commit::pin_directory(
                self,
                path.parent().ok_or(("PATH", "Missing parent"))?,
            )?)
        } else {
            None
        };
        let mut options = File::options();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Do not follow a final-component reparse point introduced after target().
            options.custom_flags(0x0020_0000);
        }
        let file = options
            .open(&path)
            .map_err(|_| ("PATH", "archivo no accesible"))?;
        let before = file
            .metadata()
            .map_err(|_| ("IO", "no se pudo leer metadata"))?;
        #[cfg(windows)]
        if self.external {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::{
                Foundation::HANDLE,
                Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
            };
            let mut identity = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity)
                    .map_err(|_| ("PATH", "External identity unavailable"))?;
            }
            if identity.nNumberOfLinks != 1 {
                return Err(("PATH", "External hard-linked files are not accepted"));
            }
        }
        if !before.is_file() || before.len() > limit {
            return Err(("SIZE", "archivo no admitido o supera límite de lectura"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if before.file_attributes() & 0x400 != 0 {
                return Err(("PATH", "no se admiten reparse points"));
            }
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        let mut reader = (&file).take(limit + 1);
        let mut chunk = [0_u8; 65536];
        loop {
            context.check()?;
            let count = reader
                .read(&mut chunk)
                .map_err(|_| ("IO", "no se pudo leer el archivo"))?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        context.check()?;
        let after = file
            .metadata()
            .map_err(|_| ("IO", "no se pudo leer metadata"))?;
        if bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(("STALE", "el archivo cambió durante la lectura"));
        }
        Ok(bytes)
    }

    #[cfg(test)]
    pub fn read(&self, relative: &str, start: usize, end: usize) -> Result<Value, Failure> {
        self.read_with_context(relative, start, end, &OperationContext::default())
    }
    pub fn read_with_context(
        &self,
        relative: &str,
        start: usize,
        end: usize,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if start == 0 || end < start || end - start >= 100 {
            return Err(("RANGE", "indicar rango positivo de máximo 100 líneas"));
        }
        let bytes = self.bytes_with_context(relative, context)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "archivo no UTF-8"))?;
        let normalized = text.replace("\r\n", "\n");
        let lines: Vec<_> = normalized.split('\n').collect();
        if start > lines.len() {
            return Err(("RANGE", "inicio supera el final del archivo"));
        }
        let actual_end = end.min(lines.len());
        let snippet =
            lines[start - 1..actual_end].join(if text.contains("\r\n") { "\r\n" } else { "\n" });
        if snippet.len() > 16000 {
            return Err(("SIZE", "fragmento demasiado grande; reducir el rango"));
        }
        Ok(
            json!({"path":relative,"start":start,"end":actual_end,"totalLines":lines.len(),"hash":hash(&bytes),"text":snippet,"anchors":harness_domain::navigation::anchors(text,start,actual_end)}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_limits_hash_and_newlines() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("example.txt"), "uno\r\ndos\r\n").unwrap();
        let p = Project::new(dir.path()).unwrap();
        let value = p.read("example.txt", 1, 2).unwrap();
        assert_eq!(value["text"], "uno\r\ndos");
        assert_eq!(value["totalLines"], 3);
        assert_eq!(value["hash"], hash(b"uno\r\ndos\r\n"));
        for (start, end) in [(0, 1), (2, 1), (1, 101), (4, 4)] {
            assert_eq!(p.read("example.txt", start, end).unwrap_err().0, "RANGE");
        }
    }
    #[test]
    fn rejects_unsafe_and_sensitive_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::new(dir.path()).unwrap();
        for name in [
            ".env",
            ".env.local",
            "credentials.json",
            "secret.txt",
            "mcp_config.json",
        ] {
            fs::write(dir.path().join(name), "private").unwrap();
            assert_eq!(p.bytes(name).unwrap_err().0, "PATH");
        }
        for name in [
            "../outside",
            "C:\\outside",
            "/outside",
            "\\\\server\\share",
            "a\0b",
            ".env ",
            "mcp_config.json.",
            "CON",
            "NUL.txt",
            "COM1",
        ] {
            assert_eq!(p.bytes(name).unwrap_err().0, "PATH");
        }
    }
    #[test]
    fn rejects_large_binary_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::new(dir.path()).unwrap();
        fs::write(dir.path().join("large"), vec![b'a'; MAX_FILE as usize + 1]).unwrap();
        fs::write(dir.path().join("binary"), [0xff]).unwrap();
        assert_eq!(p.bytes("large").unwrap_err().0, "SIZE");
        assert_eq!(p.read("binary", 1, 1).unwrap_err().0, "ENCODING");
        assert!(p.bytes(".").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn rejects_junctions() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("file"), "private").unwrap();
        let junction = dir.path().join("junction");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(status.status.success());
        assert_eq!(
            Project::new(dir.path())
                .unwrap()
                .bytes("junction/file")
                .unwrap_err()
                .0,
            "PATH"
        );
        fs::remove_dir(junction).unwrap();
    }
}

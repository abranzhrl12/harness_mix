use harness_domain::execution::OperationContext;
pub use harness_domain::{Failure, hash};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_FILE: u64 = 2 * 1024 * 1024;

pub(crate) fn numbered_text(text: &str, start: usize, end: usize) -> String {
    if end < start {
        return String::new();
    }
    let mut numbered = String::with_capacity(text.len() + text.lines().count() * 12);
    let mut line_number = start;

    if text.is_empty() {
        return format!("{line_number} | ");
    }

    for line in text.split_inclusive('\n') {
        if line_number > end {
            break;
        }
        numbered.push_str(&format!("{line_number} | "));
        numbered.push_str(line);
        line_number += 1;
    }

    if text.ends_with('\n') && line_number <= end {
        numbered.push_str(&format!("{line_number} | "));
    }

    numbered
}

pub(super) fn external_sensitive(relative: &str) -> bool {
    external_sensitive_reason(relative).is_some()
}

pub(super) fn external_sensitive_reason(relative: &str) -> Option<&'static str> {
    let normalized = relative.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or("");
    if name.contains(".config") {
        Some("config_filename")
    } else if name.starts_with(".npmrc") {
        Some("npmrc_filename")
    } else if name.starts_with("appsettings") && name.contains(".json") {
        Some("appsettings_filename")
    } else if ["pem", "key", "pfx", "p12"]
        .iter()
        .any(|ext| name.ends_with(&format!(".{ext}")))
    {
        Some("private_key_extension")
    } else {
        None
    }
}

pub(super) fn external_restricted(relative: &str) -> bool {
    external_restricted_reason(relative).is_some()
}

pub(super) fn external_restricted_reason(relative: &str) -> Option<&'static str> {
    relative.replace('\\', "/").split('/').find_map(|part| {
        let name = part.to_ascii_lowercase();
        if [".git", ".hg", ".svn"].contains(&name.as_str()) {
            Some("version_control_directory")
        } else if [
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
        {
            Some("sensitive_path_prefix")
        } else {
            None
        }
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
        self.resolve_target_mode(relative, allow_missing, false)
    }

    fn resolve_target_mode(
        &self,
        relative: &str,
        allow_missing: bool,
        credential: bool,
    ) -> Result<PathBuf, Failure> {
        let target = self.build_target(relative, credential, |path| {
            let meta = match fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(());
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
            Ok(())
        })?;
        let existing = if allow_missing && !target.exists() {
            target.ancestors().skip(1).find(|path| path.exists())
                .ok_or(("PATH", "directorio no accesible"))?
        } else {
            target.as_path()
        };
        let real = existing.canonicalize().map_err(|_| ("PATH", "archivo no accesible"))?;
        if self.single_file.as_ref().is_some_and(|single| *single != real) {
            return Err(("PATH", "Scope authorizes only its registered file"));
        }
        if !real.starts_with(&self.root) {
            return Err(("PATH", "destino fuera del proyecto"));
        }
        Ok(target)
    }

    /// Shared lexical policy. The visitor decides how each ancestor is checked;
    /// this private builder alone is not proof that a filesystem path is safe.
    fn build_target(
        &self,
        relative: &str,
        credential: bool,
        mut visit: impl FnMut(&Path) -> Result<(), Failure>,
    ) -> Result<PathBuf, Failure> {
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
                    if (!credential && (name == ".env" || name.starts_with(".env.")))
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
                    visit(&target)?;
                }
                _ => return Err(("PATH", "ruta restringida o fuera del proyecto")),
            }
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

    /// Internal credential resolution only. Normal read/config/search tools cannot use this path.
    pub(super) fn credential_bytes(
        &self,
        relative: &str,
        context: &OperationContext,
    ) -> Result<Option<Vec<u8>>, Failure> {
        if self.external || self.single_file.is_some() {
            return Err((
                "PATH",
                "Credential fallback is available only inside the primary project",
            ));
        }
        let path = self.resolve_target_mode(relative, true, true)?;
        if !path.exists() {
            return Ok(None);
        }
        self.read_input_path(path, context, 64 * 1024).map(Some)
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
        let path = {
            let _timer = crate::adapters::index_profile::operation("io_path_validation");
            self.target(relative)?
        };
        self.read_input_path(path, context, limit)
    }

    /// Index-source read: validate once, then keep Windows ancestors pinned until
    /// the handle-based read completes. Never retain this validation across reads.
    #[cfg(windows)]
    pub(super) fn index_source_bytes(
        &self,
        relative: &str,
        context: &OperationContext,
        limit: u64,
    ) -> Result<Option<Vec<u8>>, Failure> {
        if std::env::var("HARNESS_INDEX_PINNED_READ").as_deref() != Ok("0") {
            self.index_source_bytes_pinned(relative, context, limit)
        } else {
            self.index_source_bytes_checked(relative, context, limit)
        }
    }

    #[cfg(windows)]
    fn index_source_bytes_pinned(
        &self,
        relative: &str,
        context: &OperationContext,
        limit: u64,
    ) -> Result<Option<Vec<u8>>, Failure> {
        use crate::adapters::index_profile as profile;
        // External and single-file scopes retain canonical identity validation.
        if self.external || self.single_file.is_some() {
            return self.index_source_bytes_checked(relative, context, limit);
        }
        context.check()?;
        let path = {
            let _timer = profile::operation("source_path_policy");
            self.build_target(relative, false, |_| Ok(()))?
        };
        let pins = {
            let _timer = profile::operation("source_pin_directory");
            super::edit_commit::pin_directory(self, path.parent().ok_or(("PATH", "Missing parent"))?)
        };
        let _pins = match pins {
            Ok(pins) => pins,
            Err(_) => {
                // Classify absent ancestors and preserve the original failure
                // policy. Never treat a failed pin as proof of absence.
                profile::count("source_pin_fallback", 1);
                return self.index_source_bytes_checked(relative, context, limit);
            }
        };
        // Ancestors were opened root-first, rejecting reparses at every step,
        // and cannot be renamed/deleted while these handles remain alive.
        // Do not query the leaf through an unvalidated ancestor beforehand.
        if !Self::source_present(&path)? {
            return Ok(None);
        }
        profile::count("source_pinned_reads", 1);
        self.read_input_path_with(path, context, limit.min(MAX_FILE), |file| {
            self.verify_opened_source(file)
        }).map(Some)
    }

    #[cfg(windows)]
    fn verify_opened_source(&self, file: &File) -> Result<(), Failure> {
        use std::{ffi::OsString, os::windows::{ffi::OsStringExt, io::AsRawHandle}};
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{
            GetFinalPathNameByHandleW, GETFINALPATHNAMEBYHANDLE_FLAGS, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS,
        }};
        let _timer = crate::adapters::index_profile::operation("source_handle_boundary");
        let mut buffer = vec![0u16; 512];
        let handle = HANDLE(file.as_raw_handle());
        let mut length = unsafe {
            GetFinalPathNameByHandleW(handle, &mut buffer, GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0))
        } as usize;
        if length >= buffer.len() {
            if length > 32768 {
                return Err(("PATH", "Opened source path exceeds Windows path budget"));
            }
            buffer.resize(length + 1, 0);
            length = unsafe {
                GetFinalPathNameByHandleW(handle, &mut buffer, GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0))
            } as usize;
        }
        if length == 0 || length >= buffer.len() {
            return Err(("PATH", "Opened source identity unavailable"));
        }
        let opened = PathBuf::from(OsString::from_wide(&buffer[..length]));
        if !opened.starts_with(&self.root) {
            return Err(("PATH", "Opened source is outside the project"));
        }
        Ok(())
    }

    #[cfg(windows)]
    fn source_present(path: &Path) -> Result<bool, Failure> {
        use crate::adapters::index_profile as profile;
        let metadata = {
            let _timer = profile::operation("source_metadata");
            fs::symlink_metadata(path)
        };
        match metadata {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                profile::count("source_missing", 1);
                Ok(false)
            }
            Err(_) => Err(("IO", "Source metadata unavailable; absence not established")),
            Ok(metadata) if !metadata.is_file() => Err(("PATH", "Source is not a regular file")),
            Ok(_) => Ok(true),
        }
    }

    #[cfg(windows)]
    fn index_source_bytes_checked(
        &self,
        relative: &str,
        context: &OperationContext,
        limit: u64,
    ) -> Result<Option<Vec<u8>>, Failure> {
        use crate::adapters::index_profile as profile;
        context.check()?;
        let path = {
            let _timer = profile::operation("source_path_guard");
            self.target_optional(relative)?
        };
        if !Self::source_present(&path)? {
            return Ok(None);
        }
        let _pins = {
            let _timer = profile::operation("source_pin_directory");
            super::edit_commit::pin_directory(
                self,
                path.parent().ok_or(("PATH", "Missing parent"))?,
            )?
        };
        if self.external && external_sensitive(relative) {
            return Err(("PATH", "External configuration/credential content requires the redacted config tool"));
        }
        // pin_directory checks every ancestor for reparse points and denies
        // rename/delete sharing. read_input_path rejects a replaced final link,
        // checks the opened handle and checks metadata again after reading.
        self.read_input_path(path, context, limit.min(MAX_FILE)).map(Some)
    }

    fn read_input_path(
        &self,
        path: PathBuf,
        context: &OperationContext,
        limit: u64,
    ) -> Result<Vec<u8>, Failure> {
        self.read_input_path_with(path, context, limit, |_| Ok(()))
    }

    fn read_input_path_with(
        &self,
        path: PathBuf,
        context: &OperationContext,
        limit: u64,
        verify: impl FnOnce(&File) -> Result<(), Failure>,
    ) -> Result<Vec<u8>, Failure> {
        let _profile = crate::adapters::index_profile::operation("io_project_read");
        context.check()?;
        #[cfg(windows)]
        let _pins = if self.external {
            let _timer = crate::adapters::index_profile::operation("io_external_pin");
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
        let file = {
            let _timer = crate::adapters::index_profile::operation("io_open");
            options.open(&path).map_err(|_| ("PATH", "archivo no accesible"))?
        };
        let before = {
            let _timer = crate::adapters::index_profile::operation("io_metadata_before");
            file.metadata().map_err(|_| ("IO", "no se pudo leer metadata"))?
        };
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
        verify(&file)?;
        let mut reader = (&file).take(limit + 1);
        let mut chunk = [0_u8; 65536];
        {
            let _timer = crate::adapters::index_profile::operation("io_read_bytes");
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
        }
        crate::adapters::index_profile::count("io_bytes_read", bytes.len() as u64);
        context.check()?;
        let after = {
            let _timer = crate::adapters::index_profile::operation("io_metadata_after");
            file.metadata().map_err(|_| ("IO", "no se pudo leer metadata"))?
        };
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
        if start == 0 || end < start || end - start >= 420 {
            return Err(("RANGE", "indicar rango positivo de máximo 420 líneas"));
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

#[cfg(all(test, windows))]
#[path = "project/index_read_tests.rs"]
mod index_read_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn index_read_keeps_limits_absence_and_path_policy() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/file.rs"), b"abc").unwrap();
        let p = Project::new(dir.path()).unwrap();
        let context = OperationContext::default();
        assert_eq!(p.index_source_bytes("nested/file.rs", &context, 3).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(p.index_source_bytes("nested/file.rs", &context, 2).unwrap_err().0, "SIZE");
        assert_eq!(p.index_source_bytes("nested/absent.rs", &context, 3).unwrap(), None);
        assert_eq!(p.index_source_bytes("nested", &context, 3).unwrap_err().0, "PATH");
        for path in ["../outside", "nested/../file.rs", ".env", "NUL.rs"] {
            assert_eq!(p.index_source_bytes(path, &context, 3).unwrap_err().0, "PATH");
        }
        // A validation never survives into the next read.
        fs::write(dir.path().join("nested/file.rs"), b"xyz").unwrap();
        assert_eq!(p.index_source_bytes("nested/file.rs", &context, 3).unwrap(), Some(b"xyz".to_vec()));
        let external = Project { external: true, ..p.clone() };
        fs::write(dir.path().join("appsettings.json"), b"{}").unwrap();
        assert_eq!(external.index_source_bytes("appsettings.json", &context, 3).unwrap_err().0, "PATH");
        let single = Project { single_file: Some(dir.path().join("nested/file.rs").canonicalize().unwrap()), ..p };
        fs::write(dir.path().join("other.rs"), b"abc").unwrap();
        assert_eq!(single.index_source_bytes("other.rs", &context, 3).unwrap_err().0, "PATH");
    }
    #[test]
    fn read_limits_hash_and_newlines() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("example.txt"), "uno\r\ndos\r\n").unwrap();
        let p = Project::new(dir.path()).unwrap();
        let value = p.read("example.txt", 1, 2).unwrap();
        assert_eq!(value["text"], "uno\r\ndos");
        assert_eq!(value["totalLines"], 3);
        assert_eq!(value["hash"], hash(b"uno\r\ndos\r\n"));
        for (start, end) in [(0, 1), (2, 1), (1, 421), (4, 4)] {
            assert_eq!(p.read("example.txt", start, end).unwrap_err().0, "RANGE");
        }
    }
    #[test]
    fn numbered_text_preserves_newlines_and_uses_absolute_line_numbers() {
        assert_eq!(numbered_text("uno\r\ndos\r\n", 8, 10), "8 | uno\r\n9 | dos\r\n10 | ");
        assert_eq!(numbered_text("uno\ndos", 4, 5), "4 | uno\n5 | dos");
    }
    #[test]
    fn numbered_text_handles_eof_without_a_final_newline_and_empty_text() {
        assert_eq!(numbered_text("fin", 12, 12), "12 | fin");
        assert_eq!(numbered_text("a\n", 12, 12), "12 | a\n");
        assert_eq!(numbered_text("", 1, 1), "1 | ");
        assert_eq!(numbered_text("", 1, 0), "");
    }
    #[test]
    fn wider_reads_keep_byte_and_range_limits() {
        let dir = tempfile::tempdir().unwrap();
        let p = Project::new(dir.path()).unwrap();
        fs::write(dir.path().join("many.txt"), "x\r\n".repeat(450)).unwrap();
        let value = p.read("many.txt", 1, 420).unwrap();
        assert_eq!(value["end"], 420);
        assert_eq!(value["text"].as_str().unwrap().split("\r\n").count(), 420);
        assert_eq!(p.read("many.txt", 1, 421).unwrap_err().0, "RANGE");
        fs::write(
            dir.path().join("long.txt"),
            format!("{}\n", "x".repeat(100)).repeat(300),
        )
        .unwrap();
        assert_eq!(p.read("long.txt", 1, 300).unwrap_err().0, "SIZE");
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
        assert_eq!(
            Project::new(dir.path()).unwrap()
                .index_source_bytes("junction/file", &OperationContext::default(), 1024)
                .unwrap_err().0,
            "PATH"
        );
        fs::remove_dir(junction).unwrap();
    }
}

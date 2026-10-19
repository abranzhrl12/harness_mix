//! One explicitly selected workspace configuration; never expose a free OS filesystem to Oxc.
use super::{Failure, INDEX, OperationContext, Project, hash, index_inventory, sql};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

pub(super) const KEY: &str = "native_tsconfig_v1";
const INVALID: Failure = (
    "SYNC_CONFIG",
    "Unsupported or invalid bounded tsconfig; no parser/source details returned",
);

#[derive(Clone, Default)]
pub(super) struct Snapshot {
    pub semantic: Option<super::semantic_policy::Snapshot>,
    pub path: Option<String>,
    pub digest: Option<String>,
    pub json: Option<String>,
    pub base: Option<String>,
    patterns: Vec<String>,
    _pins: std::sync::Arc<Vec<std::fs::File>>,
    _parent: Option<Box<Snapshot>>,
    dependencies: Vec<Value>,
    base_url: Option<String>,
    raw_paths: Option<Value>,
    paths_directory: Option<String>,
}

// Strip JSONC trivia without interpreting strings or accepting JSON5 syntax.
// Keep byte positions/newlines and hash the original input, not this view.
fn parse_jsonc(text: &str) -> Result<Value, Failure> {
    let mut bytes = text.trim_start_matches('\u{feff}').as_bytes().to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            match bytes[i] {
                b'\\' => i += 1,
                b'"' => quoted = false,
                _ => {}
            }
        } else {
            match bytes[i] {
                b'"' => quoted = true,
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                        bytes[i] = b' ';
                        i += 1;
                    }
                    continue;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    bytes[i] = b' ';
                    bytes[i + 1] = b' ';
                    i += 2;
                    loop {
                        if i + 1 >= bytes.len() {
                            return Err(INVALID);
                        }
                        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            bytes[i] = b' ';
                            bytes[i + 1] = b' ';
                            i += 2;
                            break;
                        }
                        if !matches!(bytes[i], b'\r' | b'\n') {
                            bytes[i] = b' ';
                        }
                        i += 1;
                    }
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }
    i = 0;
    quoted = false;
    while i < bytes.len() {
        if quoted {
            match bytes[i] {
                b'\\' => i += 1,
                b'"' => quoted = false,
                _ => {}
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i] == b',' {
            let next = bytes[i + 1..]
                .iter()
                .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
            if matches!(next, Some(b']' | b'}')) {
                // A comma is trailing only after a value, not after an opening
                // delimiter or another comma (which would accept malformed JSON).
                let previous = bytes[..i]
                    .iter()
                    .rfind(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
                if matches!(previous, None | Some(b'[' | b'{' | b',' | b':')) {
                    return Err(INVALID);
                }
                bytes[i] = b' ';
            }
        }
        i += 1;
    }
    serde_json::from_slice(&bytes).map_err(|_| INVALID)
}

fn normalized(base: &str, value: &str) -> Result<String, Failure> {
    if value.len() > 1000
        || value.starts_with('/')
        || value.contains(['\\', ':', '?', '#', '$'])
        || value.chars().any(char::is_control)
    {
        return Err(INVALID);
    }
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or(INVALID)?;
            }
            _ => parts.push(part),
        }
    }
    let result = parts.join("/");
    if parts.len() > 64 || index_inventory::restricted(&result) {
        return Err(INVALID);
    }
    Ok(result)
}

impl Snapshot {
    pub fn load(
        project: &Project,
        path: &str,
        context: &OperationContext,
    ) -> Result<Self, Failure> {
        Self::load_inner(project, path, context, &mut Vec::new(), &mut 0)
    }

    fn load_inner(
        project: &Project,
        path: &str,
        context: &OperationContext,
        stack: &mut Vec<String>,
        total: &mut usize,
    ) -> Result<Self, Failure> {
        context.check()?;
        if stack.len() >= 8 || stack.iter().any(|p| p.eq_ignore_ascii_case(path)) {
            return Err(INVALID);
        }
        stack.push(path.into());
        if path.is_empty()
            || path.len() > 1000
            || path.contains(['\\', ':'])
            || path.split('/').any(|p| matches!(p, "" | "." | ".."))
            || !path.ends_with(".json")
            || index_inventory::restricted(path)
        {
            return Err(INVALID);
        }
        let target = project.target_optional(path)?;
        match std::fs::symlink_metadata(&target) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path: Some(path.into()),
                    ..Self::default()
                });
            }
            Err(_) => return Err(INVALID),
            Ok(_) => {}
        }
        #[cfg(not(windows))]
        let pins = Vec::new();
        #[cfg(windows)]
        let pins = {
            use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
            use windows::Win32::{
                Foundation::HANDLE,
                Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
            };
            let mut pins =
                super::super::edit_commit::pin_directory(project, target.parent().ok_or(INVALID)?)?;
            let file = std::fs::File::options()
                .read(true)
                .share_mode(3)
                .custom_flags(0x0020_0000)
                .open(&target)
                .map_err(|_| INVALID)?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                    .map_err(|_| INVALID)?;
            }
            if info.nNumberOfLinks != 1 || info.dwFileAttributes & 0x400 != 0 {
                return Err(INVALID);
            }
            pins.push(file);
            pins
        };
        let bytes = project.bytes_with_limit(path, context, 256 * 1024)?;
        *total += bytes.len();
        if *total > 1024 * 1024 {
            return Err(INVALID);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| INVALID)?;
        let value = parse_jsonc(text)?;
        let object = value.as_object().ok_or(INVALID)?;
        if object.contains_key("references") {
            return Err(INVALID);
        }
        let directory = path.rsplit_once('/').map_or("", |(p, _)| p);
        let parent = if let Some(value) = object.get("extends") {
            let spec = value.as_str().ok_or(INVALID)?;
            if !spec.starts_with("./") && !spec.starts_with("../") {
                return Err(INVALID);
            }
            let mut target = normalized(directory, spec)?;
            if !target.ends_with(".json") {
                target.push_str(".json");
            }
            let parent = Self::load_inner(project, &target, context, stack, total)?;
            if parent.digest.is_none() {
                return Err(INVALID);
            }
            Some(Box::new(parent))
        } else {
            None
        };
        let empty = serde_json::Map::new();
        let compiler = match object.get("compilerOptions") {
            Some(v) => v.as_object().ok_or(INVALID)?,
            None => &empty,
        };
        for key in [
            "rootDirs",
            "moduleSuffixes",
            "customConditions",
            "pathsBasePath",
        ] {
            if compiler.contains_key(key) {
                return Err(INVALID);
            }
        }
        let base_url = match compiler.get("baseUrl") {
            Some(v) => Some(normalized(directory, v.as_str().ok_or(INVALID)?)?),
            None => parent.as_ref().and_then(|p| p.base_url.clone()),
        };
        let (raw_paths, paths_directory) = match compiler.get("paths") {
            Some(v) => (Some(v.clone()), Some(directory.to_string())),
            None => (
                parent.as_ref().and_then(|p| p.raw_paths.clone()),
                parent.as_ref().and_then(|p| p.paths_directory.clone()),
            ),
        };
        let base = base_url
            .as_deref()
            .or(paths_directory.as_deref())
            .unwrap_or(directory);
        let mut paths = serde_json::Map::new();
        if let Some(value) = &raw_paths {
            let mappings = value.as_object().ok_or(INVALID)?;
            if mappings.len() > 128 {
                return Err(INVALID);
            }
            for (key, values) in mappings {
                context.check()?;
                if key.is_empty()
                    || key.len() > 256
                    || key.starts_with(['.', '/'])
                    || key.contains(['\\', ':', '?', '#', '$'])
                    || key.matches('*').count() > 1
                    || key.chars().any(char::is_control)
                {
                    return Err(INVALID);
                }
                let values = values.as_array().ok_or(INVALID)?;
                if values.is_empty() || values.len() > 8 {
                    return Err(INVALID);
                }
                let mut targets = Vec::new();
                for value in values {
                    let target = value.as_str().ok_or(INVALID)?;
                    if target.matches('*').count() > 1
                        || (target.contains('*') && !key.contains('*'))
                    {
                        return Err(INVALID);
                    }
                    let target = normalized(base, target)?;
                    targets.push(json!(super::paths::virtual_path(&target).to_string_lossy()));
                }
                paths.insert(key.clone(), json!(targets));
            }
        }
        // Flatten only validated targets to virtual absolute paths. Omit baseUrl
        // from Oxc's input: provider scope does not allow a failed paths match to
        // fall through to unrelated bare-package files under baseUrl.
        let patterns = paths.keys().cloned().collect();
        let options = json!({"paths":paths});
        let mut dependencies = Vec::new();
        if let Some(parent) = &parent {
            dependencies.push(json!({"path":parent.path,"hash":parent.digest}));
            dependencies.extend(parent.dependencies.iter().cloned());
        }
        stack.pop();
        Ok(Self {
            path: Some(path.into()),
            digest: Some(hash(&bytes)),
            json: Some(json!({"compilerOptions":options}).to_string()),
            base: Some(directory.into()),
            patterns,
            _pins: std::sync::Arc::new(pins),
            _parent: parent,
            dependencies,
            base_url,
            raw_paths,
            paths_directory,
            semantic: None,
        })
    }

    pub fn stamp(&self) -> Value {
        let mut stamp = json!({"path":self.path,"hash":self.digest});
        if let Some(policy) = self.semantic.as_ref().and_then(|p| p.stamp()) {
            stamp["semanticPolicy"] = policy;
        }
        if !self.dependencies.is_empty() {
            stamp["dependencies"] = json!(self.dependencies);
        }
        stamp
    }

    pub fn matches(&self, module: &str) -> bool {
        // Provider scope: baseUrl anchors paths; it does not turn every bare
        // package/builtin into a local import. Do not fall back to homonyms.
        if [
            "fs",
            "path",
            "os",
            "crypto",
            "http",
            "https",
            "url",
            "util",
            "events",
            "stream",
            "child_process",
            "buffer",
        ]
        .contains(&module)
        {
            return false;
        }
        self.patterns
            .iter()
            .any(|pattern| match pattern.split_once('*') {
                None => module == pattern,
                Some((prefix, suffix)) => {
                    module.len() >= prefix.len() + suffix.len()
                        && module.starts_with(prefix)
                        && module.ends_with(suffix)
                }
            })
    }

    pub fn select(
        db: &Connection,
        project: &Project,
        requested: Option<&str>,
        context: &OperationContext,
    ) -> Result<(Self, bool), Failure> {
        let prior: Option<String> = sql(db
            .query_row(
                "SELECT value FROM project_metadata WHERE key=?1",
                [KEY],
                |r| r.get(0),
            )
            .optional())?;
        let prior: Value = match prior {
            Some(text) if text.len() <= 16 * 1024 => {
                serde_json::from_str(&text).map_err(|_| INDEX)?
            }
            Some(_) => return Err(INDEX),
            None => json!({"path":null,"hash":null}),
        };
        let path = requested.or_else(|| prior["path"].as_str());
        let mut snapshot = match path {
            Some(path) => Self::load(project, path, context)?,
            None => Self::default(),
        };
        if snapshot.path.is_some()
            && snapshot.digest.is_none()
            && prior["path"].as_str() != snapshot.path.as_deref()
        {
            return Err(("SYNC_CONFIG", "Explicit tsconfig does not exist"));
        }
        snapshot.semantic = Some(super::semantic_policy::Snapshot::load(project, context)?);
        let changed = snapshot.stamp() != prior;
        Ok((snapshot, changed))
    }

    pub fn recheck(&self, project: &Project, context: &OperationContext) -> Result<(), Failure> {
        if let Some(policy) = &self.semantic {
            policy.recheck(project, context)?;
        }
        if let Some(path) = &self.path {
            let mut current = Self::load(project, path, context)?;
            current.semantic = self.semantic.clone();
            if current.stamp() != self.stamp() {
                return Err((
                    "STALE",
                    "Tsconfig changed before publication; transaction rolled back",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod jsonc_tests {
    use super::parse_jsonc;

    #[test]
    fn preserves_strings_and_accepts_only_jsonc_trivia() {
        let value = parse_jsonc(
            "\u{feff}{/* ñ */\"url\":\"https://x/*y*/\",// comment\r\n\"items\":[\"a,}\",],}",
        )
        .unwrap();
        assert_eq!(value["url"], "https://x/*y*/");
        assert_eq!(value["items"][0], "a,}");
        for invalid in [
            "{/*",
            "{,}",
            "[,]",
            "[1,,]",
            "{'x':1}",
            "{x:1}",
            "[NaN]",
            "[1] garbage",
        ] {
            assert!(parse_jsonc(invalid).is_err(), "{invalid}");
        }
    }
}

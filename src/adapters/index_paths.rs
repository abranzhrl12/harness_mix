//! Oxc module resolution over an immutable, bounded SQLite path snapshot.
//! No OS filesystem; only one validated, sanitized configuration snapshot can be read.
use super::{Failure, INDEX, LIMIT, OperationContext, index_extract, index_inventory, sql};
use oxc_resolver::{
    EnforceExtension, FileMetadata, FileSystem, ResolveError, ResolveOptions, ResolverGeneric,
    TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use rusqlite::Connection;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const ROOT: &str = r"C:\__harness_codegraph_snapshot";
pub(super) fn virtual_path(path: &str) -> PathBuf {
    Path::new(ROOT).join(path)
}
const MAX_PROBES: usize = 100_000;
type Resolver = ResolverGeneric<View>;

struct Snapshot {
    files: BTreeSet<PathBuf>,
    dirs: BTreeSet<PathBuf>,
    probes: AtomicUsize,
    context: OperationContext,
    config: Option<(PathBuf, String)>,
}
#[derive(Clone)]
struct View(Arc<Snapshot>);
impl View {
    fn guard(&self, path: &Path) -> io::Result<()> {
        if self.0.context.check().is_err() {
            return Err(io::Error::other("resolution interrupted"));
        }
        if self.0.probes.fetch_add(1, Ordering::Relaxed) >= MAX_PROBES {
            return Err(io::Error::other("resolution budget exceeded"));
        }
        if !path.starts_with(ROOT)
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(io::ErrorKind::NotFound.into());
        }
        Ok(())
    }
}
impl FileSystem for View {
    fn new() -> Self {
        // Views are always built from an explicit, validated snapshot via `new_with_file_system`.
        unreachable!("snapshot view cannot be created without a snapshot")
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.read_to_string(path).map(String::into_bytes)
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        self.guard(path)?;
        if let Some((selected, text)) = &self.0.config
            && selected == path
        {
            return Ok(text.clone());
        }
        // Only the selected sanitized tsconfig can be read; never package.json or OS files.
        Err(io::ErrorKind::NotFound.into())
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.guard(path)?;
        if self.0.files.contains(path) {
            return Ok(FileMetadata::new(true, false, false));
        }
        if self.0.dirs.contains(path) {
            return Ok(FileMetadata::new(false, true, false));
        }
        Err(io::ErrorKind::NotFound.into())
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.metadata(path)
    }
    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        self.guard(path)?;
        Err(io::Error::from(io::ErrorKind::PermissionDenied).into())
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // The snapshot has no symlinks, so a validated path is already canonical.
        self.guard(path)?;
        Ok(path.to_path_buf())
    }
}

pub(super) struct Routes {
    view: View,
    resolvers: BTreeMap<String, Resolver>,
    emitted: Resolver,
    memo: BTreeMap<(String, String, String), Option<String>>,
    memo_bytes: usize,
    bare_directory: PathBuf,
}

fn options(extensions: &[&str], main_files: &[&str]) -> ResolveOptions {
    ResolveOptions {
        extensions: extensions.iter().map(|s| (*s).into()).collect(),
        // CodeGraph tries appended extensions BEFORE the exact filename.
        enforce_extension: EnforceExtension::Enabled,
        main_files: main_files.iter().map(|s| (*s).into()).collect(),
        modules: Vec::new(),
        main_fields: Vec::new(),
        exports_fields: Vec::new(),
        imports_fields: Vec::new(),
        symlinks: false,
        ..ResolveOptions::default()
    }
}

impl Routes {
    #[cfg(test)]
    pub(super) fn load(db: &Connection, context: &OperationContext) -> Result<Self, Failure> {
        Self::load_config(db, context, None)
    }
    pub(super) fn load_config(
        db: &Connection,
        context: &OperationContext,
        config: Option<&super::tsconfig::Snapshot>,
    ) -> Result<Self, Failure> {
        let mut statement = sql(
            db.prepare("SELECT path,language FROM files ORDER BY path COLLATE BINARY LIMIT 20001")
        )?;
        let rows =
            sql(statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))))?;
        let mut files = BTreeSet::new();
        let mut dirs = BTreeSet::from([PathBuf::from(ROOT)]);
        let mut reserved = 0usize;
        for (count, row) in rows.enumerate() {
            context.check()?;
            if count >= 20_000 {
                return Err(LIMIT);
            }
            let (path, language) = sql(row)?;
            if path.is_empty()
                || path.len() > 1000
                || path.contains(['\\', ':'])
                || path.chars().any(char::is_control)
                || path.split('/').any(|p| matches!(p, "" | "." | ".."))
            {
                return Err(INDEX);
            }
            let script_language = matches!(
                language.as_str(),
                "typescript" | "tsx" | "javascript" | "jsx"
            ) && matches!(
                index_extract::language(&path),
                Some("typescript" | "tsx" | "javascript" | "jsx")
            );
            let vue_component = language == "vue" && index_extract::language(&path) == Some("vue");
            if index_inventory::restricted(&path) || !(script_language || vue_component) {
                continue;
            }
            let absolute = Path::new(ROOT).join(&path);
            reserved += path.len() + ROOT.len();
            files.insert(absolute.clone());
            for parent in absolute
                .ancestors()
                .skip(1)
                .take_while(|p| p.starts_with(ROOT))
            {
                if dirs.insert(parent.to_owned()) {
                    reserved += parent.as_os_str().len();
                }
                if dirs.len() > 40_000 || reserved > 16 * 1024 * 1024 {
                    return Err(LIMIT);
                }
            }
        }
        let bare_directory = config
            .and_then(|c| c.base.as_deref())
            .map_or_else(|| PathBuf::from(ROOT), virtual_path);
        for parent in bare_directory
            .ancestors()
            .take_while(|p| p.starts_with(ROOT))
        {
            dirs.insert(parent.to_owned());
        }
        let config = config.and_then(|c| Some((virtual_path(c.path.as_deref()?), c.json.clone()?)));
        if let Some((path, _)) = &config {
            files.insert(path.clone());
            for parent in path.ancestors().skip(1).take_while(|p| p.starts_with(ROOT)) {
                dirs.insert(parent.to_owned());
            }
        }
        let view = View(Arc::new(Snapshot {
            files,
            dirs,
            probes: AtomicUsize::new(0),
            context: context.clone(),
            config,
        }));
        let seed = Resolver::new_with_file_system(view.clone(), options(&[""], &[]));
        let mut resolvers = BTreeMap::new();
        // Explicitly pinned v1.6.2 priorities, NOT Oxc defaults. Directory cases
        // outside these provider lists are deferred after resolution below.
        for (language, extensions) in [
            (
                "typescript",
                vec![".ts", ".tsx", ".d.ts", ".js", ".jsx", ""],
            ),
            ("tsx", vec![".tsx", ".ts", ".d.ts", ".js", ".jsx", ""]),
            ("javascript", vec![".js", ".jsx", ".mjs", ".cjs", ""]),
            ("jsx", vec![".jsx", ".js", ""]),
            ("vue", vec![".vue", ""]),
        ] {
            let mut configured = options(&extensions, &["index"]);
            if let Some((path, _)) = &view.0.config {
                configured.tsconfig = Some(TsconfigDiscovery::Manual(TsconfigOptions {
                    config_file: path.clone(),
                    references: TsconfigReferences::Disabled,
                }));
            }
            resolvers.insert(
                language.into(),
                seed.clone_with_options(configured),
            );
        }
        let mut emitted_options = options(&[""], &[]);
        if let Some((path, _)) = &view.0.config {
            emitted_options.tsconfig = Some(TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: path.clone(),
                references: TsconfigReferences::Disabled,
            }));
        }
        emitted_options.extension_alias = vec![
            (
                ".js".into(),
                vec![".ts".into(), ".tsx".into(), ".d.ts".into()],
            ),
            (".jsx".into(), vec![".tsx".into()]),
            (".mjs".into(), vec![".mts".into(), ".d.mts".into()]),
            (".cjs".into(), vec![".cts".into(), ".d.cts".into()]),
        ];
        Ok(Self {
            view,
            resolvers,
            emitted: seed.clone_with_options(emitted_options),
            memo: BTreeMap::new(),
            memo_bytes: 0,
            bare_directory,
        })
    }

    pub(super) fn resolve(
        &mut self,
        from: &str,
        target: &str,
        language: &str,
        context: &OperationContext,
    ) -> Result<Option<String>, Failure> {
        self.resolve_request(from, target, language, context, false)
    }
    pub(super) fn resolve_bare(
        &mut self,
        from: &str,
        target: &str,
        language: &str,
        context: &OperationContext,
    ) -> Result<Option<String>, Failure> {
        self.resolve_request(from, target, language, context, true)
    }
    fn resolve_request(
        &mut self,
        from: &str,
        target: &str,
        language: &str,
        context: &OperationContext,
        bare: bool,
    ) -> Result<Option<String>, Failure> {
        context.check()?;
        let key = (
            format!("{bare}:{from}"),
            target.to_owned(),
            language.to_owned(),
        );
        if let Some(hit) = self.memo.get(&key) {
            return Ok(hit.clone());
        }
        self.memo_bytes += from.len() + target.len() + language.len() + 1000;
        if self.memo.len() >= 1024 || self.memo_bytes > 4 * 1024 * 1024 {
            return Err(LIMIT);
        }
        let resolver = self.resolvers.get(language).ok_or(INDEX)?;
        // Target is already normalized/validated by the caller. Resolve from
        // the virtual workspace root so ../ never leaves this immutable view.
        let request = if bare {
            target.to_owned()
        } else {
            format!("./{target}")
        };
        // The pinned Oxc version applies manual tsconfig paths only within its
        // paths base. Bare requests are workspace-config-relative, not importer-
        // relative; use that validated virtual directory, never the OS cwd.
        let directory = if bare {
            self.bare_directory.as_path()
        } else {
            Path::new(ROOT)
        };
    let found = resolver
        .resolve(directory, &request)
        .or_else(|_| self.emitted.resolve(directory, &request));
        eprintln!("RESOLVE-DIAG bare={bare} request={request} found={found:?}");
        context.check()?;
        if self.view.0.probes.load(Ordering::Relaxed) >= MAX_PROBES {
            return Err(LIMIT);
        }
        let result = match found {
            Ok(resolved) => {
                let relative = resolved
                    .path()
                    .strip_prefix(ROOT)
                    .map_err(|_| INDEX)?
                    .to_str()
                    .ok_or(INDEX)?
                    .replace('\\', "/");
                if language != "vue" && relative.to_ascii_lowercase().ends_with(".vue") {
                    return Ok(None);
                }
                if !self.view.0.files.contains(resolved.path())
                    || index_inventory::restricted(&relative)
                {
                    return Err(INDEX);
                }
                // Oxc has more directory candidates than CodeGraph. Do not
                // silently pick e.g. index.d.ts where provider would not.
                let directory = relative.strip_prefix(&format!("{target}/"));
                let allowed = match language {
                    "typescript" => &["index.ts", "index.tsx", "index.js"][..],
                    "tsx" => &["index.tsx", "index.ts", "index.js"][..],
                    "javascript" => &["index.js", "index.jsx"][..],
                    "jsx" => &["index.jsx", "index.js"][..],
                    "vue" => &["index.vue"][..],
                    _ => &[][..],
                };
                let bare_index = bare
                    && Path::new(&relative)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|file| file.starts_with("index.") && !allowed.contains(&file));
                if bare_index || directory.is_some_and(|file| !allowed.contains(&file)) {
                    None
                } else {
                    Some(relative)
                }
            }
            Err(_) => None,
        };
        self.memo.insert(key, result.clone());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_tsconfig_paths_and_baseurl_resolve_in_virtual_filesystem() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("tsconfig.json"), r#"{"compilerOptions":{"baseUrl":"src","paths":{"@exact":["one"],"@lib/*":["missing/*","*"]}}}"#).unwrap();
        let config = super::super::tsconfig::Snapshot::load(
            &super::super::Project::new(root.path()).unwrap(),
            "tsconfig.json",
            &OperationContext::default(),
        )
        .unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(path TEXT PRIMARY KEY,language TEXT); INSERT INTO files VALUES('src/one.ts','typescript'),('src/two.ts','typescript');").unwrap();
        let mut routes =
            Routes::load_config(&db, &OperationContext::default(), Some(&config)).unwrap();
        for (request, expected) in [("@exact", "src/one.ts"), ("@lib/two", "src/two.ts")] {
            assert_eq!(
                routes
                    .resolve_bare(
                        "app.ts",
                        request,
                        "typescript",
                        &OperationContext::default()
                    )
                    .unwrap()
                    .as_deref(),
                Some(expected),
                "{request} {}",
                config.json.as_ref().unwrap()
            );
        }
    }
    fn fixture(files: &[&str], context: &OperationContext) -> Routes {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(path TEXT PRIMARY KEY, language TEXT)")
            .unwrap();
        for file in files {
            db.execute(
                "INSERT INTO files VALUES(?1,?2)",
                rusqlite::params![file, index_extract::language(file).unwrap_or("json")],
            )
            .unwrap();
        }
        Routes::load(&db, context).unwrap()
    }
    #[test]
    fn provider_priorities_and_emitted_extensions_use_index_snapshot() {
        let context = OperationContext::default();
        for (language, target, files, expected) in [
            (
                "typescript",
                "lib",
                vec!["lib.ts", "lib.tsx", "lib/index.ts"],
                Some("lib.ts"),
            ),
            ("tsx", "lib", vec!["lib.ts", "lib.tsx"], Some("lib.tsx")),
            (
                "javascript",
                "lib",
                vec!["lib.ts", "lib.js", "lib.jsx"],
                Some("lib.js"),
            ),
            ("jsx", "lib", vec!["lib.js", "lib.jsx"], Some("lib.jsx")),
            (
                "typescript",
                "lib",
                vec!["lib/index.ts", "lib/index.js"],
                Some("lib/index.ts"),
            ),
            (
                "typescript",
                "lib.js",
                vec!["lib.js", "lib.ts"],
                Some("lib.js"),
            ),
            ("typescript", "lib.js", vec!["lib.ts"], Some("lib.ts")),
            (
                "typescript",
                "lib.js",
                vec!["lib.js.ts", "lib.js", "lib.ts"],
                Some("lib.js.ts"),
            ),
            ("typescript", "lib.mjs", vec!["lib.mts"], Some("lib.mts")),
            ("typescript", "lib.cjs", vec!["lib.cts"], Some("lib.cts")),
            ("typescript", "lib.jsx", vec!["lib.tsx"], Some("lib.tsx")),
            ("typescript", "missing", vec!["lib.ts"], None),
        ] {
            let mut routes = fixture(&files, &context);
            assert_eq!(
                routes
                    .resolve("app.ts", target, language, &context)
                    .unwrap()
                    .as_deref(),
                expected,
                "{language}: {target}"
            );
        }
    }
    #[test]
    fn unsupported_directory_variants_defer_and_filesystem_never_exposes_contents() {
        let context = OperationContext::default();
        let mut routes = fixture(
            &["lib/index.d.ts", "package.json", "tsconfig.json", "lib.ts"],
            &context,
        );
        assert_eq!(
            routes
                .resolve("app.ts", "lib", "typescript", &context)
                .unwrap(),
            Some("lib.ts".into())
        );
        let mut unsupported = fixture(&["lib/index.d.ts"], &context);
        assert_eq!(
            unsupported
                .resolve("app.ts", "lib", "typescript", &context)
                .unwrap(),
            None
        );
        for path in [
            Path::new(ROOT).join("lib.ts"),
            Path::new(ROOT).join("package.json"),
            PathBuf::from(r"C:\outside\secret.ts"),
        ] {
            assert!(routes.view.read_to_string(&path).is_err());
            assert!(routes.view.read_link(&path).is_err());
        }
    }
    #[test]
    fn malformed_index_paths_and_cancelled_context_are_rejected() {
        let context = OperationContext::default();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE files(path TEXT,language TEXT); INSERT INTO files VALUES('../escape.ts','typescript')").unwrap();
        assert_eq!(Routes::load(&db, &context).err().unwrap().0, "SYNC_INDEX");
        let mut routes = fixture(&["lib.ts"], &context);
        assert!(
            routes
                .resolve("app.ts", "lib", "typescript", &context)
                .unwrap()
                .is_some()
        );
        context.cancel();
        assert_eq!(
            routes
                .resolve("app.ts", "lib", "typescript", &context)
                .err()
                .unwrap()
                .0,
            "CANCELLED"
        );
    }
    #[test]
    fn excessive_distinct_module_requests_abort_instead_of_silent_partial_results() {
        let context = OperationContext::default();
        let mut routes = fixture(&["lib.ts"], &context);
        let mut limited = false;
        for index in 0..1100 {
            match routes.resolve("app.ts", &format!("missing{index}"), "typescript", &context) {
                Ok(_) => {}
                Err((code, _)) => {
                    assert_eq!(code, "SYNC_LIMIT");
                    limited = true;
                    break;
                }
            }
        }
        assert!(limited);
    }
    #[test]
    #[ignore = "requires HARNESS_CODEGRAPH_PACKAGE and HARNESS_CODEGRAPH_NODE; provider oracle only"]
    fn relative_resolution_matches_codegraph_in_supported_priority_cases() {
        let package = std::env::var("HARNESS_CODEGRAPH_PACKAGE").unwrap();
        let node = std::env::var("HARNESS_CODEGRAPH_NODE").unwrap();
        let script = r#"
const path = require('path');
const api = require(path.join(process.argv[1], 'dist/resolution/import-resolver.js'));
const input = JSON.parse(process.argv[2]);
const ctx = {getProjectRoot: () => process.argv[3], fileExists: p => input.files.includes(p)};
process.stdout.write(JSON.stringify(api.resolveImportPath('./' + input.target, 'app.ts', input.language, ctx)));
"#;
        let context = OperationContext::default();
        for (language, target, files) in [
            (
                "typescript",
                "lib",
                vec!["lib.ts", "lib.tsx", "lib/index.ts"],
            ),
            ("tsx", "lib", vec!["lib.ts", "lib.tsx"]),
            ("javascript", "lib", vec!["lib.js", "lib.jsx"]),
            ("jsx", "lib", vec!["lib.js", "lib.jsx"]),
            ("typescript", "lib", vec!["lib/index.ts", "lib/index.js"]),
            ("typescript", "lib.js", vec!["lib.ts", "lib.js"]),
            ("typescript", "lib.js", vec!["lib.ts"]),
            (
                "typescript",
                "lib.js",
                vec!["lib.js.ts", "lib.js", "lib.ts"],
            ),
            ("typescript", "lib.mjs", vec!["lib.mts"]),
            ("typescript", "lib.cjs", vec!["lib.cts"]),
            ("typescript", "lib.jsx", vec!["lib.tsx"]),
        ] {
            let input = serde_json::json!({"language":language,"target":target,"files":files});
            let output = std::process::Command::new(&node)
                .args(["-e", script, &package, &input.to_string(), ROOT])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected: Option<String> = serde_json::from_slice(&output.stdout).unwrap();
            let actual = fixture(&files, &context)
                .resolve("app.ts", target, language, &context)
                .unwrap();
            assert_eq!(actual, expected, "{language}: {target}");
        }
    }

    #[test]
    fn vue_files_resolve_from_vue_imports_without_changing_script_resolution() {
        let context = OperationContext::default();
        let mut routes = fixture(&["src/App.ts", "src/Parent.vue", "src/Card.vue"], &context);
        assert_eq!(
            routes
                .resolve("src/Parent.vue", "src/Card", "vue", &context)
                .unwrap()
                .as_deref(),
            Some("src/Card.vue")
        );
        assert_eq!(
            routes
                .resolve("src/App.ts", "src/Card", "typescript", &context)
                .unwrap(),
            None
        );
        assert_eq!(
            routes
                .resolve("src/App.ts", "src/Card.vue", "typescript", &context)
                .unwrap(),
            None
        );
    }
}

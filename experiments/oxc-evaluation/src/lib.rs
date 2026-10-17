//! Isolated evaluation. No production graph writes or real filesystem access.
use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};

/// Byte offsets of references bound to a named import, excluding homonymous
/// locals. This is lexical binding analysis, NOT cross-file call resolution.
pub fn imported_references(
    source: &str,
    path: &str,
    local: &str,
) -> Result<Vec<u32>, &'static str> {
    if source.len() > 1024 * 1024 {
        return Err("source budget exceeded");
    }
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).map_err(|_| "unsupported source type")?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.errors.is_empty() {
        return Err("syntax error");
    }
    let built = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if !built.errors.is_empty() {
        return Err("semantic error");
    }
    let semantic = built.semantic;
    let mut offsets = Vec::new();
    for symbol in semantic.scoping().symbol_ids() {
        if semantic.scoping().symbol_name(symbol) != local {
            continue;
        }
        // Oxc semantic is not a TypeScript type checker: some conflicting
        // declarations are recorded rather than emitted as build errors.
        if !semantic.scoping().symbol_redeclarations(symbol).is_empty() {
            return Err("conflicting binding declarations");
        }
        if !matches!(
            semantic.symbol_declaration(symbol).kind(),
            AstKind::ImportSpecifier(_)
        ) {
            continue;
        }
        for reference in semantic.symbol_references(symbol) {
            offsets.push(
                semantic
                    .nodes()
                    .get_node(reference.node_id())
                    .kind()
                    .span()
                    .start,
            );
        }
    }
    offsets.sort_unstable();
    Ok(offsets)
}

pub mod modules {
    use oxc_resolver::{FileMetadata, FileSystem, FsCache, ResolveOptions, ResolverGeneric};
    use std::{
        collections::BTreeMap,
        io,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    /// Immutable, synthetic filesystem. No delegation to std::fs, no links.
    pub struct Snapshot {
        root: PathBuf,
        files: BTreeMap<PathBuf, String>,
        probes: Arc<AtomicUsize>,
    }
    impl Snapshot {
        pub fn new(root: PathBuf, files: &[(&str, &str)]) -> Self {
            assert!(root.is_absolute());
            let files = files
                .iter()
                .map(|(name, contents)| {
                    assert!(
                        Path::new(name)
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_)))
                    );
                    (root.join(name), (*contents).to_owned())
                })
                .collect();
            Self {
                root,
                files,
                probes: Arc::new(AtomicUsize::new(0)),
            }
        }
        fn check(&self, path: &Path) -> io::Result<()> {
            if !path.starts_with(&self.root)
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "outside synthetic snapshot",
                ));
            }
            if self.probes.fetch_add(1, Ordering::Relaxed) >= 4096 {
                return Err(io::Error::other("snapshot probe budget exceeded"));
            }
            Ok(())
        }
    }
    impl FileSystem for Snapshot {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.check(path)?;
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
            self.check(path)?;
            if self.files.contains_key(path) {
                return Ok(FileMetadata::new(true, false, false));
            }
            if path == self.root || self.files.keys().any(|file| file.starts_with(path)) {
                return Ok(FileMetadata::new(false, true, false));
            }
            Err(io::ErrorKind::NotFound.into())
        }
        fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
            self.metadata(path)
        }
        fn read_link(&self, _: &Path) -> io::Result<PathBuf> {
            Err(io::ErrorKind::PermissionDenied.into())
        }
    }
    pub type SnapshotResolver = ResolverGeneric<FsCache<Snapshot>>;
    pub fn resolver(snapshot: Snapshot, options: ResolveOptions) -> SnapshotResolver {
        ResolverGeneric::new_with_cache(Arc::new(FsCache::new(snapshot)), options)
    }
    pub fn options() -> ResolveOptions {
        ResolveOptions {
            extensions: vec![".ts".into(), ".tsx".into(), ".js".into(), ".jsx".into()],
            extension_alias: vec![(
                ".js".into(),
                vec![".ts".into(), ".tsx".into(), ".js".into()],
            )],
            condition_names: vec!["import".into(), "default".into()],
            symlinks: false,
            ..ResolveOptions::default()
        }
    }

    /// A compatibility adjustment for the evaluated emitted-.js case only.
    /// Not a claim that all CodeGraph extension priorities are reproduced.
    pub fn prefer_existing_js() -> ResolveOptions {
        ResolveOptions {
            extension_alias: vec![(
                ".js".into(),
                vec![".js".into(), ".ts".into(), ".tsx".into()],
            )],
            ..options()
        }
    }
}

//! Isolated probe of Rolldown's public Rust API. Not a production adapter.
use oxc_resolver::{FileMetadata, ResolveError};
use rolldown::{BundleFactory, BundleFactoryOptions, BundlerOptions, Platform, TsConfig};
use rolldown_common::Output;
use rolldown_fs::{FileSystem, OxcResolverFileSystem};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const EVALUATED_VERSION: &str = "1.2.14";

#[derive(Clone, Default)]
struct Snapshot {
    files: Arc<BTreeMap<PathBuf, Vec<u8>>>,
    directories: Arc<BTreeSet<PathBuf>>,
    probes: Arc<AtomicUsize>,
    content_reads: Arc<AtomicUsize>,
    denied_reads: Arc<AtomicUsize>,
    denied_writes: Arc<AtomicUsize>,
}
fn missing() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "not in probe snapshot")
}
impl Snapshot {
    fn bounded(root: &Path, files: &[(&str, &str)]) -> Result<Self, String> {
        if !root.is_absolute() || files.len() > 32 {
            return Err("invalid probe input".into());
        }
        let mut sources = BTreeMap::new();
        let mut directories = BTreeSet::new();
        let mut bytes = 0;
        for (name, source) in files {
            if name.len() > 1000 {
                return Err("probe path budget".into());
            }
            let name = Path::new(name);
            if name.as_os_str().is_empty()
                || !name.components().all(|p| matches!(p, Component::Normal(_)))
            {
                return Err("probe paths must be normal relative paths".into());
            }
            bytes += source.len();
            if bytes > 256 * 1024 {
                return Err("probe source budget".into());
            }
            let path = root.join(name);
            for parent in path.ancestors().skip(1) {
                directories.insert(parent.to_path_buf());
            }
            if sources.insert(path, source.as_bytes().to_vec()).is_some() {
                return Err("duplicate probe path".into());
            }
        }
        Ok(Self {
            files: Arc::new(sources),
            directories: Arc::new(directories),
            ..Self::default()
        })
    }
    fn probe(&self) -> io::Result<()> {
        if self.probes.fetch_add(1, Ordering::Relaxed) >= 100_000 {
            Err(io::Error::other("probe budget"))
        } else {
            Ok(())
        }
    }
    fn deny_write(&self) -> io::Result<()> {
        self.denied_writes.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "read-only probe snapshot",
        ))
    }
}
impl OxcResolverFileSystem for Snapshot {
    fn new(_yarn_pnp: bool) -> Self {
        Self::default()
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.probe()?;
        match self.files.get(path) {
            Some(bytes) => {
                self.content_reads.fetch_add(1, Ordering::Relaxed);
                Ok(bytes.clone())
            }
            None => {
                self.denied_reads.fetch_add(1, Ordering::Relaxed);
                Err(missing())
            }
        }
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?).map_err(|_| io::Error::other("invalid UTF-8"))
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.probe()?;
        if self.files.contains_key(path) {
            Ok(FileMetadata::new(true, false, false))
        } else if self.directories.contains(path) {
            Ok(FileMetadata::new(false, true, false))
        } else {
            Err(missing())
        }
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.metadata(path)
    }
    fn read_link(&self, _path: &Path) -> Result<PathBuf, ResolveError> {
        Err(ResolveError::from(missing()))
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.metadata(path)?;
        Ok(path.to_path_buf())
    }
}
impl FileSystem for Snapshot {
    fn remove_dir_all(&self, _path: &Path) -> io::Result<()> {
        self.deny_write()
    }
    fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        self.deny_write()
    }
    fn write(&self, _path: &Path, _content: &[u8]) -> io::Result<()> {
        self.deny_write()
    }
    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        self.deny_write()
    }
    fn exists(&self, path: &Path) -> bool {
        self.metadata(path).is_ok()
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        self.probe()?;
        if !self.directories.contains(path) {
            return Err(missing());
        }
        Ok(self
            .files
            .keys()
            .chain(self.directories.iter())
            .filter(|p| p.parent() == Some(path))
            .cloned()
            .collect())
    }
}

fn relative(root: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "<internal>".into())
}

/// Evaluates only synthetic inputs. `root` must be an empty temporary directory.
/// The custom FS has no OS fallback; this does NOT certify that all factory or
/// built-in-plugin initialization outside that FS is sandboxed.
pub async fn evaluate(root: &Path, files: &[(&str, &str)]) -> Result<Value, String> {
    let snapshot = Snapshot::bounded(root, files)?;
    let mut factory = BundleFactory::new(BundleFactoryOptions {
        bundler_options: BundlerOptions {
            cwd: Some(root.into()),
            input: Some(vec!["./app.ts".to_string().into()]),
            platform: Some(Platform::Neutral),
            tsconfig: Some(TsConfig::Auto(false)),
            ..Default::default()
        },
        disable_tracing_setup: true,
        ..Default::default()
    })
    .map_err(|e| format!("factory: {e:?}"))?;
    let tsconfig = if files.iter().any(|(p, _)| *p == "tsconfig.json") {
        TsConfig::Manual(root.join("tsconfig.json"))
    } else {
        TsConfig::Auto(false)
    };
    let resolver = Arc::new(rolldown_resolver::Resolver::new(
        snapshot.clone(),
        root.into(),
        Platform::Neutral,
        &tsconfig,
        Default::default(),
    ));
    let bundle = factory.create_bundle_with_fs(snapshot.clone(), resolver);
    let handle = bundle.context();
    let result = bundle.generate().await;
    let mut modules = factory
        .module_infos()
        .iter()
        .map(|entry| {
            let module = entry.value();
            let mut exports = module
                .exports
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            exports.sort();
            let mut imports = module
                .imported_ids
                .iter()
                .map(|id| relative(root, id.as_str()))
                .collect::<Vec<_>>();
            imports.sort();
            json!({"path":relative(root,module.id.as_str()),"exports":exports,"imports":imports})
        })
        .collect::<Vec<_>>();
    modules.sort_by_key(|v| v["path"].as_str().unwrap_or("").to_string());
    let mut report = match result {
        Ok(output) => {
            let chunks = output.assets.iter().filter_map(|asset| match asset {
                Output::Chunk(chunk) => Some(json!({"exports":chunk.exports.iter().map(ToString::to_string).collect::<Vec<_>>(),"code":chunk.code,"sourceModules":chunk.module_ids.iter().map(|id|relative(root,id.as_str())).collect::<Vec<_>>()})),
                Output::Asset(_) => None,
            }).collect::<Vec<_>>();
            json!({"status":"ok","chunks":chunks,"warnings":format!("{:?}",output.warnings)})
        }
        Err(error) => json!({"status":"link_error","diagnostics":format!("{error:?}")}),
    };
    handle.close().await.map_err(|e| format!("close: {e:?}"))?;
    report["rolldown"] = json!(EVALUATED_VERSION);
    report["modules"] = json!(modules);
    report["snapshot"] = json!({"probes":snapshot.probes.load(Ordering::Relaxed),"contentReads":snapshot.content_reads.load(Ordering::Relaxed),"deniedReads":snapshot.denied_reads.load(Ordering::Relaxed),"deniedWrites":snapshot.denied_writes.load(Ordering::Relaxed)});
    Ok(report)
}

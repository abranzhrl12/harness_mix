use serde_json::{Value, json};
use std::collections::HashSet;
#[cfg(windows)]
use std::path::Path;

/// Cached probe: recomputed only when PATH/PATHEXT change or after a short TTL.
pub(super) fn capabilities() -> Value {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(String, Instant, Value)>> = Mutex::new(None);
    let key = format!(
        "{:?}|{:?}",
        std::env::var_os("PATH"),
        std::env::var_os("PATHEXT")
    );
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached_key, at, value)) = guard.as_ref() {
        if *cached_key == key && at.elapsed() < Duration::from_secs(300) {
            return value.clone();
        }
    }
    let value = compute();
    *guard = Some((key, Instant::now(), value.clone()));
    value
}

fn compute() -> Value {
    let paths: Vec<_> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(windows)]
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|s| s.starts_with('.') && !s.contains(['/', '\\', ':']))
        .map(str::to_owned)
        .collect();
    #[cfg(not(windows))]
    let extensions = vec![String::new()];
    // One directory enumeration per PATH entry replaces dirs x names x extensions stat calls.
    #[cfg(windows)]
    let listings: Vec<Option<HashSet<String>>> = paths.iter().map(|dir| directory_names(dir)).collect();
    #[cfg(not(windows))]
    let listings: Vec<Option<HashSet<String>>> =
        paths.iter().map(|_| None).collect();
    let mut available = json!({});
    for name in [
        "codegraph",
        "rg",
        "ast-grep",
        "sd",
        "cargo",
        "git",
        "go",
        "python",
        "node",
        "dotnet",
        "msbuild",
        "vstest.console",
    ] {
        available[name] =
            json!(
                paths
                    .iter()
                    .zip(&listings)
                    .any(|(dir, listing)| extensions.iter().any(|ext| {
                        let entry = format!("{name}{ext}");
                        if let Some(names) = listing {
                            return names.contains(&entry.to_ascii_lowercase());
                        }
                        let file = dir.join(&entry);
                        let Ok(meta) = file.metadata() else {
                            return false;
                        };
                        if !meta.is_file() {
                            return false;
                        }
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if meta.permissions().mode() & 0o111 == 0 {
                                return false;
                            }
                        }
                        true
                    }))
            );
    }
    // Verification discovers Visual Studio MSBuild through vswhere, so the
    // bootstrap availability field must use that same bounded resolver.
    record_msbuild_availability(
        &mut available,
        super::process::executable(harness_domain::process::Program::Msbuild).is_ok(),
    );
    record_browser_cdp_status(&mut available, cfg!(feature = "browser-cdp"));
    // Native execution never uses PATHEXT wrappers or relative PATH entries.
    for name in ["cargo", "rustc", "git", "go", "python", "node", "dotnet"] {
        available[name] = json!(native_program_available(name, &paths, &listings));
    }
    available
}

#[cfg(windows)]
fn directory_names(directory: &Path) -> Option<HashSet<String>> {
    let mut names = HashSet::new();
    for entry in std::fs::read_dir(directory).ok()? {
        let entry = entry.ok()?;
        if let Some(name) = entry.file_name().to_str() {
            names.insert(name.to_ascii_lowercase());
        }
    }
    Some(names)
}

fn native_program_available(
    name: &str,
    paths: &[std::path::PathBuf],
    listings: &[Option<HashSet<String>>],
) -> bool {
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let lookup = filename.to_ascii_lowercase();
    for (index, directory) in paths.iter().enumerate() {
        if listings
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|names| !names.contains(&lookup))
        {
            continue;
        }
        let executable = directory.join(&filename);
        if !executable.is_file() {
            continue;
        }
        // Match the native runner: only the first regular PATH candidate is
        // authoritative, and a path that cannot be canonicalized is unavailable.
        return executable.canonicalize().is_ok();
    }
    false
}

fn record_msbuild_availability(available: &mut Value, discovered_by_vswhere: bool) {
    available["msbuild"] = json!(discovered_by_vswhere);
}

fn record_browser_cdp_status(available: &mut Value, backend_compiled: bool) {
    available["browserCdp"] = json!({
        "backendCompiled": backend_compiled,
        "connectionProbed": false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn native_program_probe_matches_first_regular_executable_in_path() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let executable = format!("cargo{}", std::env::consts::EXE_SUFFIX);
        fs::create_dir(first.path().join(&executable)).unwrap();
        fs::write(second.path().join(&executable), b"executable").unwrap();
        let paths = vec![first.path().to_path_buf(), second.path().to_path_buf()];
        let listings = paths
            .iter()
            .map(|path| {
                Some(
                    fs::read_dir(path)
                        .unwrap()
                        .filter_map(Result::ok)
                        .filter_map(|entry| entry.file_name().to_str().map(str::to_ascii_lowercase))
                        .collect(),
                )
            })
            .collect::<Vec<Option<std::collections::HashSet<String>>>>();

        assert!(native_program_available("cargo", &paths, &listings));

        fs::remove_file(second.path().join(&executable)).unwrap();
        fs::write(second.path().join("cargo.cmd"), b"wrapper").unwrap();
        let listings = paths
            .iter()
            .map(|path| {
                Some(
                    fs::read_dir(path)
                        .unwrap()
                        .filter_map(Result::ok)
                        .filter_map(|entry| entry.file_name().to_str().map(str::to_ascii_lowercase))
                        .collect(),
                )
            })
            .collect::<Vec<_>>();

        assert!(!native_program_available("cargo", &paths, &listings));
    }

    #[test]
    fn native_program_probe_falls_back_when_a_path_listing_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let executable = format!("git{}", std::env::consts::EXE_SUFFIX);
        fs::write(dir.path().join(executable), b"executable").unwrap();

        assert!(native_program_available(
            "git",
            &[dir.path().to_path_buf()],
            &[None],
        ));
    }

    #[test]
    fn msbuild_discovered_outside_path_is_reported_as_available() {
        let mut available = json!({"msbuild":false});

        record_msbuild_availability(&mut available, true);

        assert_eq!(available["msbuild"], true);
    }

    #[test]
    fn browser_cdp_status_reports_compilation_without_claiming_connection() {
        for backend_compiled in [false, true] {
            let mut available = json!({});

            record_browser_cdp_status(&mut available, backend_compiled);

            assert_eq!(available["browserCdp"]["backendCompiled"], backend_compiled);
            assert_eq!(available["browserCdp"]["connectionProbed"], false);
        }
    }
}

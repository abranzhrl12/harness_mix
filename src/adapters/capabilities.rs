use serde_json::{Value, json};

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
    let listings: Vec<Option<std::collections::HashSet<String>>> = paths
        .iter()
        .map(|dir| {
            std::fs::read_dir(dir).ok().map(|items| {
                items
                    .filter_map(Result::ok)
                    .filter_map(|e| e.file_name().to_str().map(str::to_ascii_lowercase))
                    .collect()
            })
        })
        .collect();
    #[cfg(not(windows))]
    let listings: Vec<Option<std::collections::HashSet<String>>> = paths.iter().map(|_| None).collect();
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
        available[name] = json!(paths.iter().zip(&listings).any(|(dir, listing)| extensions
            .iter()
            .any(|ext| {
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
            })));
    }
    // Native execution never uses PATHEXT wrappers or relative PATH entries.
    for (name, program) in [
        ("cargo", harness_domain::process::Program::Cargo),
        ("rustc", harness_domain::process::Program::Rustc),
        ("git", harness_domain::process::Program::Git),
        ("go", harness_domain::process::Program::Go),
        ("python", harness_domain::process::Program::Python),
        ("node", harness_domain::process::Program::Node),
        ("dotnet", harness_domain::process::Program::Dotnet),
    ] {
        available[name] = json!(super::process::executable(program).is_ok());
    }
    available
}

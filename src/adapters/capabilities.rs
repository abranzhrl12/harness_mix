use serde_json::{Value, json};

pub(super) fn capabilities() -> Value {
    let paths: Vec<_> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|p| !p.as_os_str().is_empty())
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
    let mut available = json!({});
    for name in [
        "codegraph",
        "rg",
        "ast-grep",
        "sd",
        "cargo",
        "go",
        "python",
        "node",
        "dotnet",
        "msbuild",
        "vstest.console",
    ] {
        available[name] = json!(paths.iter().any(|dir| extensions.iter().any(|ext| {
            let file = dir.join(format!("{name}{ext}"));
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
    available
}

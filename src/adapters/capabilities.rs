use serde_json::{Value, json};

pub(super) fn capabilities() -> Value {
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
    // Native execution never uses PATHEXT wrappers or relative PATH entries.
    for (name, program) in [
        ("cargo", harness_domain::process::Program::Cargo),
        ("rustc", harness_domain::process::Program::Rustc),
        ("git", harness_domain::process::Program::Git),
        ("go", harness_domain::process::Program::Go),
        ("python", harness_domain::process::Program::Python),
        ("node", harness_domain::process::Program::Node),
    ] {
        available[name] = json!(super::process::executable(program).is_ok());
    }
    available
}

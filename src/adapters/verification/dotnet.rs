use super::{NativeProcessRunner, Project, validate_verify_options};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    execution::OperationContext,
    process::{ProcessSpec, Program, StopReason},
};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) fn classic_aspnet(source: &str) -> bool {
    // Comments can contain examples or obsolete imports; ignore them.
    let mut source = source.to_ascii_lowercase();
    while let Some(start) = source.find("<!--") {
        let Some(end) = source[start + 4..].find("-->") else {
            source.truncate(start);
            break;
        };
        source.replace_range(start..start + 4 + end + 3, "");
    }
    source.contains("microsoft.webapplication.targets")
        || (source.contains("<projecttypeguids")
            && source.contains("349c5851-65df-11da-9384-00065b846f21"))
}

fn dotnet_source(path: &std::path::Path) -> Result<String, Failure> {
    if std::fs::metadata(path)
        .map_err(|_| ("VERIFY_SCOPE", "Cannot inspect .NET project"))?
        .len()
        > 2 * 1024 * 1024
    {
        return Err((
            "VERIFY_SCOPE",
            ".NET project metadata exceeds inspection budget",
        ));
    }
    std::fs::read_to_string(path).map_err(|_| ("VERIFY_SCOPE", "Cannot read .NET project metadata"))
}

fn classic_dotnet_target(project: &Project, target: &std::path::Path) -> Result<bool, Failure> {
    let source = dotnet_source(target)?;
    if target
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("csproj"))
    {
        return Ok(classic_aspnet(&source));
    }
    // Inspect only project references declared in this solution, inside the root.
    let parent = target
        .parent()
        .ok_or(("VERIFY_SCOPE", "Solution parent unavailable"))?;
    for line in source
        .lines()
        .filter(|line| line.trim_start().starts_with("Project("))
    {
        let fields: Vec<_> = line.split('"').collect();
        let Some(path) = fields
            .get(5)
            .filter(|path| path.to_ascii_lowercase().ends_with(".csproj"))
        else {
            continue;
        };
        let relative = parent.join(path.replace('\\', "/"));
        let relative = relative
            .strip_prefix(&project.root)
            .map_err(|_| ("PATH", "Solution project outside workspace"))?;
        let child = project.target(
            relative
                .to_str()
                .ok_or(("PATH", "Solution project must be UTF-8"))?,
        )?;
        if classic_aspnet(&dotnet_source(&child)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn changed_verification_files(
    project: &Project,
    context: &OperationContext,
) -> Result<std::collections::BTreeSet<String>, Failure> {
    let runner = NativeProcessRunner::new(&project.root)?;
    let mut paths = std::collections::BTreeSet::new();
    for arguments in [
        vec!["diff", "HEAD", "--name-only", "-z", "--"],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        let spec = ProcessSpec::new(
            Program::Git,
            arguments.into_iter().map(String::from).collect(),
            Duration::from_secs(5),
            2 * 1024 * 1024,
        )?;
        let outcome = runner.run(&spec, context)?;
        if outcome.reason != StopReason::Exited || outcome.exit_code != Some(0) {
            return Err(("VERIFY_GIT", "Changed-file warning coverage unavailable"));
        }
        for path in outcome
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            paths.insert(
                String::from_utf8_lossy(path)
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            );
        }
    }
    Ok(paths)
}

pub(super) fn diagnostic_lines(output: &str, severity: &str) -> std::collections::BTreeSet<String> {
    output
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains(&format!(": {severity} "))
                || lower.contains(&format!(": {severity}:"))
                || (severity == "warning"
                    && (lower.contains(": advertencia ") || lower.contains(": advertencia:")))
        })
        .map(|line| line.trim().to_owned())
        .collect()
}

fn diagnostic_count(output: &str, severity: &str, fallback: usize) -> usize {
    // MSBuild prints final totals, including warnings repeated in its summary.
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim().to_ascii_lowercase();
            let mut words = line.split_whitespace();
            let count = words.next()?.parse::<usize>().ok()?;
            let label = words.next()?;
            (label.starts_with(severity)
                || (severity == "warning" && label.starts_with("advertencia"))
                || (severity == "error" && label.starts_with("errore")))
            .then_some(count)
        })
        .last()
        .unwrap_or(fallback)
}

pub(super) fn warning_matches_file(
    line: &str,
    root: &std::path::Path,
    files: &std::collections::BTreeSet<String>,
) -> bool {
    let normalized = line.replace('\\', "/").to_ascii_lowercase();
    let Some(marker) = [
        ": warning ",
        ": warning:",
        ": advertencia ",
        ": advertencia:",
    ]
    .iter()
    .filter_map(|marker| normalized.find(marker))
    .min() else {
        return false;
    };
    let mut path = normalized[..marker].trim();
    // The final numeric parentheses belong to the source location. Earlier
    // parentheses may be part of directory or file names, e.g. Program Files (x86).
    if path.ends_with(')') {
        if let Some(start) = path.rfind('(') {
            if path[start + 1..path.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit() || c == ',' || c.is_ascii_whitespace())
            {
                path = path[..start].trim_end();
            }
        }
    }
    let root = root
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let root = root.trim_start_matches("//?/").trim_end_matches('/');
    let path = path.trim_start_matches("//?/");
    let relative = path
        .strip_prefix(&format!("{root}/"))
        .unwrap_or(path)
        .trim_start_matches("./");
    if files.contains(relative) {
        return true;
    }
    // Absolute diagnostics must belong to this workspace. Relative diagnostics
    // may be emitted relative to the owning project rather than the build cwd.
    if path.starts_with('/') || path.as_bytes().get(1) == Some(&b':') {
        return false;
    }
    let Some((_, project)) = normalized.rsplit_once('[') else {
        return false;
    };
    let Some(project) = project
        .strip_suffix(']')
        .filter(|project| project.ends_with(".csproj"))
    else {
        return false;
    };
    let project = project.trim_start_matches("//?/");
    let project = project
        .strip_prefix(&format!("{root}/"))
        .unwrap_or(project)
        .trim_start_matches("./");
    if project.starts_with('/') || project.as_bytes().get(1) == Some(&b':') {
        return false;
    }
    let Some((parent, _)) = project.rsplit_once('/') else {
        return false;
    };
    let joined = format!("{parent}/{relative}");
    let mut components = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return false;
                }
            }
            _ => components.push(part),
        }
    }
    files.contains(&components.join("/"))
}

pub(super) fn dotnet_warning_summary(
    project: &Project,
    args: &VerifyArgs,
    output: &str,
    complete: bool,
    context: &OperationContext,
) -> Value {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::{Mutex, OnceLock};
    static BASELINES: OnceLock<Mutex<HashMap<String, BTreeSet<String>>>> = OnceLock::new();
    let warnings = diagnostic_lines(output, "warning");
    let errors = diagnostic_lines(output, "error");
    let files = changed_verification_files(project, context);
    let changed: Vec<_> = warnings
        .iter()
        .filter(|line| {
            files
                .as_ref()
                .is_ok_and(|files| warning_matches_file(line, &project.root, files))
        })
        .cloned()
        .collect();
    let key = format!(
        "{}|{}|{}|{}|{:?}",
        project.root.display(),
        args.session.as_deref().unwrap_or(""),
        args.scope.as_deref().unwrap_or(""),
        args.engine.as_deref().unwrap_or("auto"),
        args.action
    );
    let mut new_warnings = None;
    let mut baseline_available = false;
    if complete && args.session.is_some() {
        if let Ok(mut baselines) = BASELINES.get_or_init(|| Mutex::new(HashMap::new())).lock() {
            if let Some(previous) = baselines.get(&key) {
                baseline_available = true;
                new_warnings = Some(warnings.difference(previous).count());
            }
            // Bounded process cache; session references remain independent.
            if baselines.len() >= 256 && !baselines.contains_key(&key) {
                baselines.clear();
            }
            baselines.insert(key, warnings.clone());
        }
    }
    json!({
        "errors": if complete { Some(diagnostic_count(output, "error", errors.len())) } else { None },
        "warnings": if complete { Some(diagnostic_count(output, "warning", warnings.len())) } else { None },
        "observedErrors":errors.len(), "observedWarnings":warnings.len(),
        "newWarnings":new_warnings, "warningBaselineAvailable":baseline_available,
        "warningBaseline":"previous_complete_verification_in_session",
        "changedFilesWarnings":changed, "changedFilesWarningsComplete":files.is_ok() && complete,
        "warningCountsComplete":complete,
    })
}

pub(super) fn dotnet_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    validate_verify_options(args)?;
    let scope = args
        .scope
        .as_deref()
        .ok_or(("VERIFY_SCOPE", ".NET requires an explicit .csproj or .sln"))?;
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    if !target.is_file()
        || !target
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("csproj") || ext.eq_ignore_ascii_case("sln")
            })
    {
        return Err((
            "VERIFY_SCOPE",
            ".NET scope must be a .csproj or .sln file, not a directory",
        ));
    }
    let relative = target
        .strip_prefix(&project.root)
        .ok()
        .and_then(|p| p.to_str())
        .ok_or(("PATH", ".NET scope must be relative UTF-8"))?;
    #[cfg(windows)]
    let relative = relative.replace('\\', "/");
    let engine = args.engine.as_deref().unwrap_or("auto");
    if !matches!(engine, "auto" | "dotnet" | "msbuild") {
        return Err(("VERIFY_ENGINE", "engine must be auto, dotnet or msbuild"));
    }
    let classic = classic_dotnet_target(project, &target)?;
    let use_msbuild = engine == "msbuild" || (engine == "auto" && classic);
    if classic && engine == "dotnet" {
        return Err((
            "VERIFY_ENGINE",
            "Classic ASP.NET requires engine=auto or msbuild",
        ));
    }
    if use_msbuild {
        if matches!(args.action, Some(VerifyAction::Test)) {
            return Err((
                "VERIFY_ACTION",
                "Visual Studio MSBuild verification currently supports build only",
            ));
        }
        return ProcessSpec::new(
            Program::Msbuild,
            vec![
                format!("./{relative}"),
                "/t:Build".into(),
                "/nologo".into(),
                "/verbosity:minimal".into(),
                "/consoleloggerparameters:Summary".into(),
                "/nodeReuse:false".into(),
                "/maxCpuCount:1".into(),
                "/p:UseSharedCompilation=false".into(),
            ],
            Duration::from_secs(30),
            32 * 1024 * 1024,
        );
    }
    // Closed local build commands; no restore or servers.
    ProcessSpec::new(
        Program::Dotnet,
        vec![
            if matches!(args.action, Some(VerifyAction::Test)) {
                "test".into()
            } else {
                "build".into()
            },
            format!("./{relative}"),
            "--no-restore".into(),
            "--nologo".into(),
            "--verbosity".into(),
            "minimal".into(),
            "--disable-build-servers".into(),
            "-nodeReuse:false".into(),
            "-maxCpuCount:1".into(),
            "-p:UseSharedCompilation=false".into(),
        ],
        Duration::from_secs(30),
        32 * 1024 * 1024,
    )
}

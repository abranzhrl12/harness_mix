//! Conservative, static MSBuild Compile-item membership checks; never evaluates MSBuild code.
use super::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::path::Path;

fn matches(patterns: &str, path: &str) -> Option<bool> {
    let mut found = false;
    for pattern in patterns.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if pattern.contains(['$', '@', '%']) {
            return None;
        }
        let normalized = pattern
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_owned();
        let glob = globset::GlobBuilder::new(&normalized)
            .literal_separator(true)
            .case_insensitive(cfg!(windows))
            .build()
            .ok()?;
        found |= glob.compile_matcher().is_match(path);
    }
    Some(found)
}

fn evaluate(source: &str, path: &str) -> Result<Option<bool>, ()> {
    let doc = roxmltree::Document::parse(source).map_err(|_| ())?;
    let root = doc.root_element();
    if !root.has_tag_name("Project") {
        return Err(());
    }
    let sdks: Vec<_> = root
        .attribute("Sdk")
        .into_iter()
        .flat_map(|s| s.split(';'))
        .chain(
            root.children()
                .filter(|n| n.has_tag_name("Sdk"))
                .filter_map(|n| n.attribute("Name")),
        )
        .collect();
    let sdk = sdks.iter().any(|s| {
        s.split('/').next().is_some_and(|name| {
            name == "Microsoft.NET.Sdk" || name.starts_with("Microsoft.NET.Sdk.")
        })
    });
    let mut default_items = true;
    let mut default_compile = true;
    let mut uncertain = sdks.iter().any(|s| {
        !s.split('/').next().is_some_and(|name| {
            name == "Microsoft.NET.Sdk" || name.starts_with("Microsoft.NET.Sdk.")
        })
    });
    // MSBuild evaluates properties before items, irrespective of source placement.
    for node in root.descendants().filter(|n| {
        n.has_tag_name("EnableDefaultCompileItems") || n.has_tag_name("EnableDefaultItems")
    }) {
        if node.ancestors().any(|n| {
            n.attribute("Condition").is_some()
                || n.has_tag_name("Choose")
                || n.has_tag_name("Target")
        }) {
            uncertain = true;
            continue;
        }
        let value = match node
            .text()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "true" => true,
            "false" => false,
            _ => {
                uncertain = true;
                continue;
            }
        };
        if node.has_tag_name("EnableDefaultCompileItems") {
            default_compile = value;
        } else {
            default_items = value;
        }
    }
    let mut included = sdk
        && default_items
        && default_compile
        && !path
            .split('/')
            .any(|s| matches!(s, "bin" | "obj") || s.starts_with('.'));
    for node in root.descendants().filter(|n| n.is_element()) {
        let conditional = node.ancestors().any(|n| {
            n.attribute("Condition").is_some()
                || n.has_tag_name("Choose")
                || n.has_tag_name("Target")
        });
        match node.tag_name().name() {
            "EnableDefaultCompileItems" | "EnableDefaultItems" => {}
            "DefaultItemExcludes" | "DefaultExcludesInProjectFolder" => uncertain = true,
            "Import" => {
                let import = node.attribute("Project").unwrap_or("");
                // Standard legacy imports provide compiler targets, not the project's source list.
                if !import.ends_with("Microsoft.CSharp.targets")
                    && !import.ends_with("Microsoft.Common.props")
                {
                    uncertain = true;
                }
            }
            "Compile" => {
                if conditional {
                    uncertain = true;
                    continue;
                }
                if let Some(pattern) = node.attribute("Include") {
                    match matches(pattern, path) {
                        Some(true) => match node
                            .attribute("Exclude")
                            .map(|p| matches(p, path))
                            .unwrap_or(Some(false))
                        {
                            Some(false) => included = true,
                            Some(true) => {}
                            None => uncertain = true,
                        },
                        Some(false) => {}
                        None => uncertain = true,
                    }
                }
                if let Some(pattern) = node.attribute("Remove") {
                    match matches(pattern, path) {
                        Some(true) => included = false,
                        Some(false) => {}
                        None => uncertain = true,
                    }
                }
            }
            _ => {}
        }
    }
    Ok(if uncertain { None } else { Some(included) })
}

/// Check the nearest ancestor containing projects, without claiming full MSBuild evaluation.
pub(super) fn check_file(
    project: &Project,
    relative: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let target = project.target(relative)?;
    let mut directory = target.parent();
    let mut inherited = false;
    let mut projects = Vec::new();
    while let Some(dir) = directory {
        context.check()?;
        if !dir.starts_with(&project.root) {
            break;
        }
        for (count, entry) in std::fs::read_dir(dir)
            .map_err(|_| ("IO", "Cannot inspect project directory"))?
            .enumerate()
        {
            context.check()?;
            if count >= 4096 {
                return Ok(
                    json!({"status":"unknown","reason":"directory_entry_limit","projects":[],"warning":null}),
                );
            }
            let entry = entry.map_err(|_| ("IO", "Cannot inspect project directory entry"))?;
            let path = entry.path();
            if path.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                s.eq_ignore_ascii_case("Directory.Build.props")
                    || s.eq_ignore_ascii_case("Directory.Build.targets")
            }) {
                inherited = true;
            }
            if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("csproj"))
            {
                projects.push(path);
            }
        }
        if !projects.is_empty() {
            break;
        }
        if dir == project.root {
            break;
        }
        directory = dir.parent();
    }
    // Directory.Build.* in higher workspace ancestors can also affect membership.
    if let Some(first) = projects.first() {
        let mut ancestor = first.parent().and_then(Path::parent);
        while let Some(dir) = ancestor {
            if !dir.starts_with(&project.root) {
                break;
            }
            inherited |= dir.join("Directory.Build.props").exists()
                || dir.join("Directory.Build.targets").exists();
            ancestor = dir.parent();
        }
    }
    projects.sort();
    let mut checks = Vec::new();
    for path in &projects {
        context.check()?;
        let rel_project = path
            .strip_prefix(&project.root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = project.bytes_with_context(&rel_project, context)?;
        let local = target
            .strip_prefix(path.parent().unwrap())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let result = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| evaluate(s, &local).ok())
            .flatten();
        let status = if inherited {
            "unknown"
        } else {
            match result {
                Some(true) => "included",
                Some(false) => "not_included",
                None => "unknown",
            }
        };
        checks.push(json!({"project":rel_project,"hash":hash(&bytes),"status":status}));
    }
    let status = if checks.is_empty() {
        "no_project"
    } else if checks.iter().any(|c| c["status"] == "included") {
        "included"
    } else if checks.iter().any(|c| c["status"] == "unknown") {
        "unknown"
    } else {
        "not_included"
    };
    let message = (status == "not_included").then(|| {
        format!(
            "Archivo nuevo no incluido en el proyecto: {}",
            relative.replace('\\', "/")
        )
    });
    Ok(
        json!({"status":status,"projects":checks,"warning":message,"code":if message.is_some(){Some("CS_PROJ_FILE_NOT_INCLUDED")}else{None},"coverage":"static nearest-ancestor .csproj Compile items; SDK defaults, Include/Exclude/Remove globs; conditions, custom imports and inherited props/targets require MSBuild evaluation; projects outside workspace are not inspected"}),
    )
}

/// Advisory after publication: never obscure a committed write with a membership failure.
pub(super) fn attach_warning(
    project: &Project,
    path: &str,
    result: &mut Value,
    context: &OperationContext,
) {
    if !Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("cs"))
    {
        return;
    }
    let membership = check_file(project, path, context)
        .unwrap_or_else(|(code, _)| json!({"status":"unknown","reason":code,"warning":null}));
    if let Some(message) = membership["warning"].as_str() {
        if result["diagnostics"].is_null() {
            result["diagnostics"] = json!([]);
        }
        if let Some(diagnostics) = result["diagnostics"].as_array_mut() {
            diagnostics.push(json!({"code":"CS_PROJ_FILE_NOT_INCLUDED","severity":"warning","path":path,"message":message}));
        }
    }
    result["projectMembership"] = membership;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_missing_and_explicit_globs() {
        assert_eq!(
            evaluate(
                r#"<Project><ItemGroup><Compile Include="Controllers\Old.cs"/></ItemGroup></Project>"#,
                "Controllers/XController.cs"
            ),
            Ok(Some(false))
        );
        assert_eq!(
            evaluate(
                r#"<Project><ItemGroup><Compile Include="Controllers\**\*.cs" Exclude="Controllers\Ignored.cs"/></ItemGroup></Project>"#,
                "Controllers/XController.cs"
            ),
            Ok(Some(true))
        );
        assert_eq!(
            evaluate(
                r#"<Project><ItemGroup><Compile Include="**/*.cs" Exclude="Controllers/XController.cs"/></ItemGroup></Project>"#,
                "Controllers/XController.cs"
            ),
            Ok(Some(false))
        );
    }
    #[test]
    fn sdk_defaults_and_removal() {
        assert_eq!(
            evaluate(r#"<Project Sdk="Microsoft.NET.Sdk"/>"#, "Controllers/X.cs"),
            Ok(Some(true))
        );
        assert_eq!(
            evaluate(r#"<Project Sdk="Microsoft.NET.Sdk"/>"#, "obj/Generated.cs"),
            Ok(Some(false))
        );
        assert_eq!(
            evaluate(
                r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><Compile Remove="Controllers/**/*.cs"/></ItemGroup></Project>"#,
                "Controllers/X.cs"
            ),
            Ok(Some(false))
        );
        assert_eq!(
            evaluate(
                r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><EnableDefaultCompileItems>false</EnableDefaultCompileItems></PropertyGroup></Project>"#,
                "X.cs"
            ),
            Ok(Some(false))
        );
    }
    #[test]
    fn conditional_dynamic_and_custom_import_are_unknown() {
        for source in [
            r#"<Project><ItemGroup Condition="'$(Configuration)' == 'Debug'"><Compile Include="X.cs"/></ItemGroup></Project>"#,
            r#"<Project><ItemGroup><Compile Include="$(Sources)/*.cs"/></ItemGroup></Project>"#,
            r#"<Project><Import Project="custom.targets"/></Project>"#,
        ] {
            assert_eq!(evaluate(source, "X.cs"), Ok(None));
        }
    }
    #[test]
    fn discovers_nearest_project_and_reports_new_controller() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Web/Controllers")).unwrap();
        std::fs::write(
            root.path().join("Outer.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\"/>",
        )
        .unwrap();
        std::fs::write(
            root.path().join("Web/FAST.csproj"),
            "<Project><ItemGroup><Compile Include=\"Old.cs\"/></ItemGroup></Project>",
        )
        .unwrap();
        std::fs::write(
            root.path().join("Web/Controllers/XController.cs"),
            "class XController {}",
        )
        .unwrap();
        let project = Project::new(root.path()).unwrap();
        let result = check_file(
            &project,
            "Web/Controllers/XController.cs",
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(result["status"], "not_included");
        assert_eq!(result["projects"][0]["project"], "Web/FAST.csproj");
        assert_eq!(
            result["warning"],
            "Archivo nuevo no incluido en el proyecto: Web/Controllers/XController.cs"
        );
        std::fs::write(root.path().join("Directory.Build.props"), "<Project/>").unwrap();
        assert_eq!(
            check_file(
                &project,
                "Web/Controllers/XController.cs",
                &OperationContext::default()
            )
            .unwrap()["status"],
            "unknown"
        );
    }
    #[test]
    fn disabled_defaults_still_allow_explicit_include() {
        assert_eq!(
            evaluate(
                r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><Compile Include="X.cs"/></ItemGroup><PropertyGroup><EnableDefaultCompileItems>false</EnableDefaultCompileItems></PropertyGroup></Project>"#,
                "X.cs"
            ),
            Ok(Some(true))
        );
        assert!(evaluate("broken xml", "X.cs").is_err());
        assert_eq!(evaluate(r#"<Project Sdk="Custom.Sdk"/>"#, "X.cs"), Ok(None));
    }
}

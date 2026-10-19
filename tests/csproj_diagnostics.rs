use agent_harness::adapters;
use harness_domain::contracts::{DiagnosticsArgs, Request};
use std::fs;
use tempfile::tempdir;

#[test]
fn doctor_warns_for_controller_omitted_from_legacy_project() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("Controllers")).unwrap();
    fs::write(
        dir.path().join("FAST.csproj"),
        "<Project><ItemGroup><Compile Include=\"Old.cs\"/></ItemGroup></Project>",
    )
    .unwrap();
    fs::write(
        dir.path().join("Controllers/XController.cs"),
        "public class XController {}",
    )
    .unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let result = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: None,
        files: Some(vec!["Controllers/XController.cs".into()]),
        max_files: Some(10),
        build_scope: None,
    }));
    assert_eq!(result["state"], "ok", "{result}");
    let file = &result["data"]["files"][0];
    assert_eq!(
        file["projectMembership"]["status"], "not_included",
        "{result}"
    );
    assert!(
        file.to_string()
            .contains("Archivo nuevo no incluido en el proyecto: Controllers/XController.cs"),
        "{result}"
    );
}

#[test]
fn git_selection_expands_new_controller_directory_and_excludes_generated_and_secret_paths() {
    let dir = tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(dir.path().join("FAST.csproj"), "<Project/>").unwrap();
    fs::write(dir.path().join(".gitignore"), "Controllers/Ignored.cs\n").unwrap();
    for folder in [
        "Controllers",
        "Controllers/obj",
        "Controllers/bin",
        "Controllers/secrets",
    ] {
        fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    for file in [
        "Controllers/XController.cs",
        "Controllers/Ignored.cs",
        "Controllers/obj/Generated.cs",
        "Controllers/bin/Generated.cs",
        "Controllers/secrets/Hidden.cs",
    ] {
        fs::write(dir.path().join(file), "public class XController {}").unwrap();
    }
    let app = adapters::application(dir.path()).unwrap();
    let result = app.execute(Request::Diagnostics(DiagnosticsArgs {
        session: None,
        files: None,
        max_files: Some(10),
        build_scope: None,
    }));
    assert_eq!(result["state"], "ok", "{result}");
    let files = result["data"]["files"].as_array().unwrap();
    let controller = files
        .iter()
        .find(|f| f["path"] == "Controllers/XController.cs")
        .expect("new controller must be selected automatically");
    assert_eq!(
        controller["projectMembership"]["status"], "not_included",
        "{result}"
    );
    for excluded in ["Ignored.cs", "Generated.cs", "Hidden.cs"] {
        assert!(
            !files
                .iter()
                .any(|f| f["path"].as_str().unwrap().ends_with(excluded)),
            "{result}"
        );
    }
}

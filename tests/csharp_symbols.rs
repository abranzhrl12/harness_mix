//! Integration tests for C# and Razor outline and read_symbol navigation.

use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{OutlineArgs, ReadSymbolArgs, SymbolView, SymbolsArgs};
use serde_json::Value;

#[path = "support/codegraph.rs"]
mod graph_fixture;

#[test]
fn explore_resolves_indexed_razor_section_from_current_source() {
    let temp = tempfile::tempdir().unwrap();
    let db = graph_fixture::fixture(temp.path());
    let source =
        "@model VehicleModel\n@section Scripts {\n<script>const label = 'Vehicle';</script>\n}\n";
    std::fs::write(temp.path().join("Index.cshtml"), source).unwrap();
    db.execute(
        "INSERT INTO nodes VALUES('razor-section','Scripts','section','Index.cshtml',2)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO files VALUES('Index.cshtml',?1,NULL)",
        [harness_domain::hash(source.as_bytes())],
    )
    .unwrap();
    drop(db);
    let app = adapters::application(temp.path()).unwrap();
    let args = serde_json::from_value(serde_json::json!({"query":"Scripts","filePath":"Index.cshtml","requireMatchingFiles":true})).unwrap();
    let result = app.execute(harness_domain::contracts::Request::Explore(args));
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(
        result["data"]["definitions"][0]["symbol"]["name"], "Scripts",
        "{result}"
    );
    assert!(
        result["data"]["text"]
            .as_str()
            .unwrap()
            .contains("const label = 'Vehicle'")
    );
    assert!(
        !result
            .to_string()
            .contains("Indexed candidate not resolved")
    );
}

fn outline(app: &NativeApplication, path: &str, query: Option<&str>) -> Value {
    app.execute(
        SymbolsArgs::Outline(OutlineArgs {
            external_scope: None,
            path: path.into(),
            query: query.map(Into::into),
            max_chars: Some(16000),
            session: None,
        })
        .into_request(),
    )
}

fn read_symbol(
    app: &NativeApplication,
    path: &str,
    name: &str,
    container: Option<&str>,
    id: Option<&str>,
    signature: bool,
) -> Value {
    app.execute(
        SymbolsArgs::Read(ReadSymbolArgs {
            external_scope: None,
            path: path.into(),
            symbol: name.into(),
            container: container.map(Into::into),
            symbol_id: id.map(Into::into),
            view: signature.then_some(SymbolView::Signature),
            session: None,
        })
        .into_request(),
    )
}

#[test]
fn test_csharp_outline_and_read_symbol() {
    let temp = tempfile::tempdir().unwrap();
    let cs_code = r#"
namespace MyApp.Controllers
{
    /// <summary>
    /// User management controller.
    /// </summary>
    public class UserController : ControllerBase
    {
        public string ConnectionString { get; set; }

        public UserController()
        {
        }

        /// <summary>
        /// Retrieves a user by their unique ID.
        /// </summary>
        public async Task<IActionResult> GetUserById(int id)
        {
            var user = await _service.Find(id);
            return Ok(user);
        }
    }
}
"#;
    std::fs::write(temp.path().join("UserController.cs"), cs_code).unwrap();
    let app = adapters::application(temp.path()).unwrap();

    let res = outline(&app, "UserController.cs", None);
    assert_eq!(res["state"], "ok");
    let symbols = res["data"]["items"].as_array().unwrap();

    assert!(
        symbols
            .iter()
            .any(|s| s["name"] == "UserController" && s["kind"] == "class")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s["name"] == "ConnectionString" && s["kind"] == "property")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s["name"] == "UserController" && s["kind"] == "constructor")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s["name"] == "GetUserById" && s["kind"] == "method")
    );

    let read_res = read_symbol(
        &app,
        "UserController.cs",
        "GetUserById",
        Some("UserController"),
        None,
        false,
    );
    assert_eq!(read_res["state"], "ok");
    assert!(
        read_res["data"]["text"]
            .as_str()
            .unwrap()
            .contains("var user = await _service.Find(id);")
    );
}

#[test]
fn test_cshtml_outline_and_read_symbol() {
    let temp = tempfile::tempdir().unwrap();
    let cshtml_code = r#"
@page
@model IndexModel
@{
    ViewData["Title"] = "Home page";
}

<h1>Welcome</h1>

@code {
    public string Message { get; set; }

    public void OnGet()
    {
        Message = "Hello from Razor code block!";
    }
}
"#;
    std::fs::write(temp.path().join("Index.cshtml"), cshtml_code).unwrap();
    let app = adapters::application(temp.path()).unwrap();

    let res = outline(&app, "Index.cshtml", None);
    assert_eq!(res["state"], "ok");
    let symbols = res["data"]["items"].as_array().unwrap();

    assert!(
        symbols
            .iter()
            .any(|s| s["name"] == "Message" && s["kind"] == "property")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s["name"] == "OnGet" && s["kind"] == "method")
    );

    let read_res = read_symbol(&app, "Index.cshtml", "OnGet", None, None, false);
    assert_eq!(read_res["state"], "ok");
    assert!(
        read_res["data"]["text"]
            .as_str()
            .unwrap()
            .contains("Message = \"Hello from Razor code block!\";")
    );
}

#[test]
fn razor_directives_preserve_names_signatures_and_complete_bodies() {
    let temp = tempfile::tempdir().unwrap();
    let source = concat!(
        "\u{feff}@model IEnumerable<MyApp.Vehicle>\r\n",
        "@* @section Hidden { ignored } *@\r\n",
        "@@section Escaped { ignored }\r\n",
        "@section Scripts {\r\n<script>const s = \"}\"; /* } */</script>\r\n<p>Driver's car</p>\r\n}\r\n",
        "@helper Vehicle(string label = \"}\") {\r\n<span>@label</span>\r\n}\r\n",
        "@functions { public string Name() { return \"}\"; } }\r\n",
        "@sectional Fake { ignored }\r\n",
    );
    std::fs::write(temp.path().join("Vehicles.cshtml"), source).unwrap();
    let app = adapters::application(temp.path()).unwrap();
    let result = outline(&app, "Vehicles.cshtml", None);
    assert_eq!(result["state"], "ok", "{result}");
    let symbols = result["data"]["items"].as_array().unwrap();
    for (name, kind) in [
        ("IEnumerable<MyApp.Vehicle>", "razor_model"),
        ("Scripts", "razor_section"),
        ("Vehicle", "razor_helper"),
        ("Name", "method"),
    ] {
        assert!(
            symbols
                .iter()
                .any(|s| s["name"] == name && s["kind"] == kind),
            "{result}"
        );
    }
    assert!(!symbols.iter().any(|s| {
        ["Hidden", "Escaped", "Fake"]
            .iter()
            .any(|name| s["name"] == *name)
    }));
    let section = read_symbol(
        &app,
        "Vehicles.cshtml",
        "Scripts",
        Some("@section"),
        None,
        false,
    );
    assert_eq!(
        section["data"]["text"],
        "@section Scripts {\r\n<script>const s = \"}\"; /* } */</script>\r\n<p>Driver's car</p>\r\n}"
    );
    let helper = read_symbol(&app, "Vehicles.cshtml", "Vehicle", None, None, true);
    assert_eq!(
        helper["data"]["symbol"]["signature"],
        "@helper Vehicle(string label = \"}\")"
    );
    let member = read_symbol(
        &app,
        "Vehicles.cshtml",
        "Name",
        Some("@functions"),
        None,
        false,
    );
    assert_eq!(member["state"], "ok", "{member}");
}

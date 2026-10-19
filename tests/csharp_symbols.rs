//! Integration tests for C# and Razor outline and read_symbol navigation.

use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{OutlineArgs, ReadSymbolArgs, SymbolView, SymbolsArgs};
use serde_json::Value;

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

    assert!(symbols.iter().any(|s| s["name"] == "UserController" && s["kind"] == "class"));
    assert!(symbols.iter().any(|s| s["name"] == "ConnectionString" && s["kind"] == "property"));
    assert!(symbols.iter().any(|s| s["name"] == "UserController" && s["kind"] == "constructor"));
    assert!(symbols.iter().any(|s| s["name"] == "GetUserById" && s["kind"] == "method"));

    let read_res = read_symbol(&app, "UserController.cs", "GetUserById", Some("UserController"), None, false);
    assert_eq!(read_res["state"], "ok");
    assert!(read_res["data"]["text"].as_str().unwrap().contains("var user = await _service.Find(id);"));
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

    assert!(symbols.iter().any(|s| s["name"] == "Message" && s["kind"] == "property"));
    assert!(symbols.iter().any(|s| s["name"] == "OnGet" && s["kind"] == "method"));

    let read_res = read_symbol(&app, "Index.cshtml", "OnGet", None, None, false);
    assert_eq!(read_res["state"], "ok");
    assert!(read_res["data"]["text"].as_str().unwrap().contains("Message = \"Hello from Razor code block!\";"));
}

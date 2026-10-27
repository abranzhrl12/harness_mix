use super::super::extract;
use harness_domain::execution::OperationContext;
use serde_json::Value;

fn extract_graph(source: &str) -> Value {
    extract("Orders.cs", source, &OperationContext::without_deadline()).unwrap()
}

fn route<'a>(graph: &'a Value, method: &str) -> Option<&'a Value> {
    graph["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["kind"] == "method" && node["name"] == method)
        .and_then(|node| node.get("httpRoutes"))
}

#[test]
fn extracts_literal_routes_and_class_prefix_semantics() {
    let source = r#"
[ApiController]
[Route("api/orders")]
public class OrdersController : ControllerBase
{
    [HttpGet("recent")]
    public object Recent() { return null; }

    [HttpPost]
    [Route("validate")]
    public object Validate() { return null; }

    [NonAction]
    public object Helper() { return null; }

    public object Index() { return null; }
}
"#;
    let graph = extract_graph(source);
    assert_eq!(
        route(&graph, "Recent").unwrap()[0],
        serde_json::json!({"path":"/api/orders/recent","method":"GET"})
    );
    assert_eq!(
        route(&graph, "Validate").unwrap()[0],
        serde_json::json!({"path":"/api/orders/validate","method":"POST"})
    );
    assert!(route(&graph, "Helper").is_none());
    assert_eq!(
        route(&graph, "Index").unwrap()[0],
        serde_json::json!({"path":"/api/orders","method":null})
    );
    let recent = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "Recent")
        .unwrap();
    assert!(
        recent["decorators"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str().unwrap_or("").starts_with("harness:http:"))
    );
}

#[test]
fn extracts_conventions_inline_attributes_and_empty_http_verb() {
    let source = r#"
public class OrdersController : Controller
{
    [HttpGet()] public object Recent() { return null; } [HttpPost("save")] public object Save() { return null; }
    [HttpGet("api:v1")] public object Colon() { return null; }
    [HttpGet("mailto:bar")] public object UriLike() { return null; }
    [HttpGet("{id}")] public object Dynamic() { return null; }
    public object Index() { return null; }
}
"#;
    let graph = extract_graph(source);
    assert_eq!(
        route(&graph, "Recent").unwrap()[0],
        serde_json::json!({"path":"/Orders/Recent","method":"GET"})
    );
    assert_eq!(
        route(&graph, "Save").unwrap()[0],
        serde_json::json!({"path":"/save","method":"POST"})
    );
    assert_eq!(
        route(&graph, "Colon").unwrap()[0],
        serde_json::json!({"path":"/api:v1","method":"GET"})
    );
    assert!(route(&graph, "UriLike").is_none());
    assert!(route(&graph, "Dynamic").is_none());
    assert_eq!(
        route(&graph, "Index").unwrap()[0],
        serde_json::json!({"path":"/Orders/Index","method":null})
    );
}

#[test]
fn supports_attribute_suffix_absolute_override_and_patch() {
    let source = r#"
[Route("api/root")]
public class PathsController : ControllerBase
{
    [HttpGetAttribute("~/absolute")]
    public object Absolute() { return null; }

    [HttpPatch("changes")]
    public object Patch() { return null; }
}
"#;
    let graph = extract_graph(source);
    assert_eq!(
        route(&graph, "Absolute").unwrap()[0],
        serde_json::json!({"path":"/absolute","method":"GET"})
    );
    assert_eq!(
        route(&graph, "Patch").unwrap()[0],
        serde_json::json!({"path":"/api/root/changes","method":"PATCH"})
    );
}

#[test]
fn ignores_non_controller_types() {
    let source = r#"
public class Ordinary
{
    [HttpGet("items")]
    public object Get() { return null; }
}
"#;
    let graph = extract_graph(source);
    assert!(route(&graph, "Get").is_none());
}

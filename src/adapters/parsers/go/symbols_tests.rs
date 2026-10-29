use super::*;

#[test]
fn collects_go_receiver_methods_types_and_imports() {
    let source = "package demo\nimport `fmt`\ntype Box[T any] struct { Value T }\ntype Alias = Box[string]\ntype Runner interface { Run() error }\nfunc (b *Box[T]) Run() error { fmt.Println(b.Value); return nil }\nfunc Café() {}\n";
    let digest = hash(source.as_bytes());
    let symbols = collect(source, &digest, &OperationContext::default()).unwrap();
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Box" && s.kind == "struct")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Runner" && s.kind == "interface")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Alias" && s.kind == "type_alias")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Run" && s.kind == "method" && s.container == "Box")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Run" && s.kind == "method" && s.container == "Runner")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Value" && s.kind == "field" && s.container == "Box")
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Café" && s.kind == "function")
    );
    assert!(symbols.iter().any(|s| s.kind == "import"));
    assert!(
        symbols
            .iter()
            .any(|s| s.kind == "import" && s.name == "fmt")
    );
}

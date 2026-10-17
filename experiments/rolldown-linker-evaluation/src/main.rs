#[tokio::main]
async fn main() {
    let root = tempfile::tempdir().expect("temporary probe directory");
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let supplied: Option<Vec<(String, String)>> = match arguments.as_slice() {
        [] => None,
        [flag, json] if flag == "--fixture-json" && json.len() <= 256 * 1024 => {
            Some(serde_json::from_str(json).expect("synthetic fixture pairs"))
        }
        _ => panic!("usage: probe [--fixture-json <pairs>]"),
    };
    let supplied_refs = supplied.as_ref().map(|pairs| {
        pairs
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str()))
            .collect::<Vec<_>>()
    });
    let defaults = [
        (
            "leaf.ts",
            "export function run(){ return 'LEAF_SENTINEL'; }",
        ),
        ("barrel.ts", "export * from './leaf';"),
        (
            "app.ts",
            "import {run} from './barrel'; export const result = run();",
        ),
    ];
    let report = harness_rolldown_linker_evaluation::evaluate(
        root.path(),
        supplied_refs.as_deref().unwrap_or(&defaults),
    )
    .await
    .expect("probe setup");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if supplied.is_none() {
        assert_eq!(report["status"], "ok");
    }
}

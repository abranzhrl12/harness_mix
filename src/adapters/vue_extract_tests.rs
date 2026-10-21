use super::*;

const SOURCE: &str = "<script setup lang=\"ts\">\nimport Icon from './Icon.vue'\nconst props = withDefaults(defineProps<{ title: string }>(), { title: 'Hi' })\nconst emit = defineEmits(['save'])\nfunction save() { emit('save') }\n</script>\n<template>\n  <main class=\"page\">\n    <section class=\"card\"><h2 class=\"card__title\">Hello</h2></section>\n    <aside class=\"other\">Other</aside>\n  </main>\n</template>\n<style scoped lang=\"scss\">\n.card { color: red; &__title { font-weight: bold; } }\n.other { color: blue; }\n</style>\n";

fn preview(source: &str) -> (tempfile::TempDir, VueExtractPreviewArgs) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Parent.vue"), source).unwrap();
    let document = parse(
        source,
        &hash(source.as_bytes()),
        &OperationContext::default(),
    )
    .unwrap();
    let selected = document
        .elements
        .iter()
        .find(|e| e.tag == "section")
        .unwrap();
    let args = VueExtractPreviewArgs {
        path: "Parent.vue".into(),
        expected_hash: hash(source.as_bytes()),
        symbol_id: selected.id.clone(),
        target_path: "CardView.vue".into(),
        component_name: "CardView".into(),
        session: None,
    };
    (root, args)
}

#[test]
fn preview_moves_exclusive_bem_as_a_whole_and_never_writes() {
    let (root, args) = preview(SOURCE);
    let project = Project::new(root.path()).unwrap();
    let result = extract_preview(&project, &args, &OperationContext::default()).unwrap();
    assert_eq!(result["writesPerformed"], false);
    assert_eq!(result["reviewRequired"], false);
    assert_eq!(result["styles"]["movedRules"], json!([".card"]));
    let files = &result["_detail"]["files"];
    let parent = files[0]["content"].as_str().unwrap();
    let child = files[1]["content"].as_str().unwrap();
    assert!(parent.contains("import CardView from \"./CardView.vue\""));
    assert!(parent.contains("<CardView />"));
    assert!(!parent.contains(".card {"));
    assert!(parent.contains(".other {"));
    assert!(child.contains("<section class=\"card\""));
    assert!(child.contains("&__title"));
    assert!(child.contains("<style scoped lang=\"scss\">"));
    assert_eq!(
        std::fs::read_to_string(root.path().join("Parent.vue")).unwrap(),
        SOURCE
    );
    assert!(!root.path().join("CardView.vue").exists());
}

#[test]
fn dynamic_expressions_shared_rules_slots_refs_and_models_require_review() {
    let source = "<template><main><section class=\"card\" :class=\"variant\"><input ref=\"field\" v-model=\"value\"/><slot>{{ title }}</slot></section><aside class=\"card\"/></main></template><style scoped lang=\"scss\">.card { color: red; }</style>";
    let (root, args) = preview(source);
    let result = extract_preview(
        &Project::new(root.path()).unwrap(),
        &args,
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(result["reviewRequired"], true);
    assert!(
        result["interfaceProposal"]["props"]
            .as_array()
            .unwrap()
            .contains(&json!("title"))
    );
    assert_eq!(result["styles"]["movedRules"], json!([]));
    assert_eq!(result["styles"]["retainedRules"], json!([".card"]));
    let unresolved = result["unresolved"].to_string();
    for text in ["v-model", "ref", "Slots", "Dynamic classes"] {
        assert!(unresolved.contains(text), "{unresolved}");
    }
}

#[test]
fn style_contexts_and_interpolated_selectors_are_not_moved_as_exclusive() {
    let source = "<template><section class=\"card\">Hello</section></template><style lang=\"scss\">$tone: red; .card { color: $tone; } @media (width > 10px) { .card { color: blue; } } .#{$name} { color: red; }</style>";
    let (root, args) = preview(source);
    let doc = parse(source, &args.expected_hash, &OperationContext::default()).unwrap();
    let structure = doc.structure(source);
    assert!(
        structure["styles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["associationCertainty"] == "unknown")
    );
    let result = extract_preview(
        &Project::new(root.path()).unwrap(),
        &args,
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(result["styles"]["movedRules"], json!([]));
    assert_eq!(result["reviewRequired"], true);
}

#[test]
fn stale_sources_existing_or_protected_targets_and_branch_splits_are_refused() {
    let (root, mut args) = preview(SOURCE);
    let project = Project::new(root.path()).unwrap();
    args.expected_hash = "0".repeat(64);
    assert_eq!(
        extract_preview(&project, &args, &OperationContext::default())
            .unwrap_err()
            .0,
        "STALE"
    );
    args.expected_hash = hash(SOURCE.as_bytes());
    std::fs::write(root.path().join("CardView.vue"), "keep").unwrap();
    assert_eq!(
        extract_preview(&project, &args, &OperationContext::default())
            .unwrap_err()
            .0,
        "TARGET_EXISTS"
    );
    args.target_path = "../Outside.vue".into();
    assert!(extract_preview(&project, &args, &OperationContext::default()).is_err());
    args.target_path = ".agents/CardView.vue".into();
    assert_eq!(
        extract_preview(&project, &args, &OperationContext::default())
            .unwrap_err()
            .0,
        "PATH"
    );
    let source = "<template><main><section v-if=\"show\">A</section><aside v-else>B</aside></main></template>";
    let (root, args) = preview(source);
    assert_eq!(
        extract_preview(
            &Project::new(root.path()).unwrap(),
            &args,
            &OperationContext::default()
        )
        .unwrap_err()
        .0,
        "VUE_BRANCH_BOUNDARY"
    );
}

#[test]
fn quoted_tags_comments_unicode_crlf_and_extraction_preserve_ranges() {
    let source = "<!-- 😀 <script>fake</script> -->\r\n<script setup>const label = 'á😀'</script>\r\n<template><section title=\">\">{{ label + '</template>' }}<!-- <x/> --><br/></section></template>";
    let (root, args) = preview(source);
    let result = extract_preview(
        &Project::new(root.path()).unwrap(),
        &args,
        &OperationContext::default(),
    )
    .unwrap();
    let child = result["_detail"]["files"][1]["content"].as_str().unwrap();
    assert!(child.contains("\r\n"));
    assert!(child.contains("{{ label + '</template>' }}"));
}

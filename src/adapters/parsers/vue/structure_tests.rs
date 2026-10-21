use super::*;

const SOURCE: &str = "<script setup lang=\"ts\">\nimport Icon from './Icon.vue'\nconst props = withDefaults(defineProps<{ title: string }>(), { title: 'Hi' })\nconst emit = defineEmits(['save'])\nfunction save() { emit('save') }\n</script>\n<template>\n  <main class=\"page\">\n    <section class=\"card\"><h2 class=\"card__title\">Hello</h2></section>\n    <aside class=\"other\">Other</aside>\n  </main>\n</template>\n<style scoped lang=\"scss\">\n.card { color: red; &__title { font-weight: bold; } }\n.other { color: blue; }\n</style>\n";

#[test]
fn structure_reports_exact_script_ranges_template_hierarchy_and_nested_bem() {
    let document = parse(
        SOURCE,
        &hash(SOURCE.as_bytes()),
        &OperationContext::default(),
    )
    .unwrap();
    let structure = document.structure(SOURCE);
    assert_eq!(structure["blocks"][0]["setup"], true);
    assert_eq!(structure["blocks"][0]["lang"], "ts");
    assert_eq!(structure["blocks"][2]["scoped"], true);
    assert_eq!(structure["template"][1]["parent"], 0);
    assert!(document.symbols.iter().any(|s| s.kind == "vue_import"));
    for name in ["defineProps", "withDefaults", "defineEmits", "save"] {
        let item = document.symbols.iter().find(|s| s.name == name).unwrap();
        assert!(SOURCE[item.bytes.clone()].contains(name));
        assert_eq!(item.start, line(SOURCE, item.bytes.start));
    }
    assert_eq!(
        structure["styles"][1]["resolvedSelectors"],
        json!([".card__title"])
    );
    assert_eq!(structure["styles"][1]["classes"], json!(["card__title"]));
    assert_eq!(
        structure["styles"][1]["associationCertainty"],
        "static_class_candidate"
    );
    assert_eq!(
        structure["styles"][1]["associatedElements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn malformed_structure_and_cancellation_are_reported() {
    let source = "<!-- 😀 <script>fake</script> -->\r\n<script setup>const label = 'á😀'</script>\r\n<template><section title=\">\">{{ label + '</template>' }}<!-- <x/> --><br/></section></template>";
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(parse(source, "x", &context).err().unwrap().0, "CANCELLED");
    assert!(
        parse(
            "<template><div></template>",
            "x",
            &OperationContext::default()
        )
        .is_err()
    );
    assert!(parse("<style>.x {</style>", "x", &OperationContext::default()).is_err());
}

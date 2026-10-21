use super::*;

#[test]
fn projection_preserves_utf16_coordinates_and_line_endings() {
    let source = "😀\r\n<script>const π = 1;</script>\r\n";
    let start = source.find("const").unwrap();
    let end = source.find("</script>").unwrap();
    let projection = project(source, start..end).unwrap();
    assert_eq!(
        source.encode_utf16().count(),
        projection.encode_utf16().count()
    );
    assert_eq!(projection.matches("\r\n").count(), 2);
    let projected_start = projection.find("const").unwrap();
    assert_eq!(
        position(source, start),
        position(&projection, projected_start)
    );
    assert_eq!(position(source, 4).unwrap(), (1, 2));
    assert!(position(source, 1).is_err());
    assert!(project(source, 1..end).is_err());
}

#[test]
fn ids_are_namespaced_deterministic_and_path_specific() {
    let first = id("vue", "src/A.vue", "component", 0);
    assert!(first.starts_with("native:framework:"));
    assert_eq!(first, id("vue", "src/A.vue", "component", 0));
    assert_ne!(first, id("vue", "src/B.vue", "component", 0));
}

#[test]
fn sparse_positions_match_original_coordinate_definition_at_every_boundary() {
    for source in ["", "ascii\n", "\u{feff}😀é\r\n中文\nπ😀tail", "a\n\n😀"] {
        let positions = Positions::new(source);
        for at in 0..=source.len() + 1 {
            assert_eq!(
                positions.position(at),
                position(source, at),
                "{source:?} at {at}"
            );
        }
    }
}

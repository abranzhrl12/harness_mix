use super::normalize;
#[test]
fn normalizes_only_literal_local_paths() {
    assert_eq!(
        normalize("/Taller/NumeroOrden_Listar/?id=1#tab"),
        Some("taller/numeroorden_listar".into())
    );
    assert_eq!(
        normalize("Taller/NumeroOrden_Listar"),
        Some("taller/numeroorden_listar".into())
    );
    for url in [
        "https://external.test/a",
        "javascript:foo",
        "mailto:bar",
        "//external.test/a",
        "../a",
        "/a/{id}",
        "/",
        "/a//b",
    ] {
        assert_eq!(normalize(url), None, "{url}");
    }
}

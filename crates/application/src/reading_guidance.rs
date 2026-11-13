//! Advisory navigation hints. No I/O, graph assertions, or effect on tool outcomes.
use serde_json::Value;

#[derive(Default)]
pub(crate) struct Guidance;

impl Guidance {
    pub(crate) fn observe(&mut self, _session: &str, _kind: &str, _args: &Value, _out: &mut Value) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read(g: &mut Guidance, session: &str, kind: &str, path: &str) -> Value {
        let mut out = json!({"state":"ok","diagnostics":[],"data":{"path":path,
            "symbol":{"name":"observed","container":"","symbolId":"local-only"}}});
        g.observe(session, kind, &json!({}), &mut out);
        out
    }

    #[test]
    fn does_not_emit_explore_recommended_after_sequential_reads() {
        let mut g = Guidance::default();
        assert_eq!(
            read(&mut g, "s", "symbol", "a.rs")["diagnostics"],
            json!([])
        );
        for p in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            assert_eq!(read(&mut g, "s", "read", p)["diagnostics"], json!([]));
        }
        let mut out = json!({"state":"ok","diagnostics":[],"data":{"path":"c.rs","items":[{"name":"real","container":"Type"}]}});
        g.observe("s", "outline", &json!({}), &mut out);
        assert_eq!(out["diagnostics"], json!([]));
    }
}

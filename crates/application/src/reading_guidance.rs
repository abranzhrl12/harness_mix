//! Advisory navigation hints. No I/O, graph assertions, or effect on tool outcomes.
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};

const MAX_SESSIONS: usize = 128;
const MAX_SELECTOR_BYTES: usize = 4096;

#[derive(Default)]
struct Trail {
    files: BTreeSet<String>,
    candidate: Option<Value>,
    notified: bool,
}

#[derive(Default)]
pub(crate) struct Guidance {
    // Bounded LRU; eviction only forgets optional advice, never session data.
    sessions: VecDeque<(String, Trail)>,
}

impl Guidance {
    pub(crate) fn observe(&mut self, session: &str, kind: &str, args: &Value, out: &mut Value) {
        if kind == "explore" {
            // Even a failed exploration was an attempt: don't nag during fallback.
            self.sessions.retain(|(id, _)| id != session);
            return;
        }
        if !matches!(kind, "read" | "outline" | "symbol")
            || out["state"] != "ok"
            || args.get("externalScope").is_some_and(|v| !v.is_null())
        {
            return;
        }
        let Some(path) = out["data"]["path"].as_str() else {
            return;
        };
        if path.is_empty() || path.len() > MAX_SELECTOR_BYTES {
            return;
        }
        let path = path.replace('\\', "/");
        let index = self.sessions.iter().position(|(id, _)| id == session);
        let (_, mut trail) = index
            .and_then(|i| self.sessions.remove(i))
            .unwrap_or_else(|| (session.to_owned(), Trail::default()));
        if !trail.notified {
            // Only three distinct paths are necessary to establish the threshold.
            if trail.files.len() < 3 {
                trail.files.insert(path.clone());
            }
            let symbol = match kind {
                "symbol" => Some(&out["data"]["symbol"]),
                "outline" => out["data"]["items"]
                    .as_array()
                    .and_then(|items| items.first()),
                _ => None,
            };
            if let Some(symbol) = symbol {
                if let Some(name) = symbol["name"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= MAX_SELECTOR_BYTES)
                {
                    // Local AST symbol IDs are not graph IDs. Never copy them.
                    let mut next = json!({"filePath":path,"symbol":name,"view":"summary",
                        "maxDepth":1,"maxFiles":2,"maxChars":4000,"syncIndex":false,"session":session});
                    if let Some(container) = symbol["container"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= MAX_SELECTOR_BYTES)
                    {
                        next["container"] = json!(container);
                    }
                    trail.candidate = Some(next);
                }
            }
            if trail.files.len() >= 3 {
                if let Some(candidate) = trail.candidate.as_ref() {
                    let hint = json!({"code":"EXPLORE_RECOMMENDED","severity":"info",
                        "message":"Ya consultaste tres archivos distintos. Si necesitas consumidores o dependencias, explore puede evitar más lecturas. Sugerencia opcional basada en sintaxis local; no certifica índice ni frescura. Si el objetivo no está indexado, continúa con fuente local.",
                        "nextCall":{"tool":"harness_explore","arguments":candidate}});
                    if let Some(diagnostics) = out["diagnostics"].as_array_mut() {
                        diagnostics.push(hint);
                        trail.notified = true;
                    }
                }
            }
        }
        if self.sessions.len() >= MAX_SESSIONS {
            self.sessions.pop_front();
        }
        self.sessions.push_back((session.to_owned(), trail));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(g: &mut Guidance, session: &str, kind: &str, path: &str) -> Value {
        let mut out = json!({"state":"ok","diagnostics":[],"data":{"path":path,
            "symbol":{"name":"observed","container":"","symbolId":"local-only"}}});
        g.observe(session, kind, &json!({}), &mut out);
        out
    }
    #[test]
    fn warns_once_after_three_distinct_files_and_resets_on_explore() {
        let mut g = Guidance::default();
        assert_eq!(
            read(&mut g, "s", "symbol", "a.rs")["diagnostics"],
            json!([])
        );
        read(&mut g, "s", "read", "a.rs");
        read(&mut g, "s", "read", "b.rs");
        let out = read(&mut g, "s", "read", "c.rs");
        let next = &out["diagnostics"][0]["nextCall"]["arguments"];
        assert_eq!(next["filePath"], "a.rs");
        assert_eq!(next["symbol"], "observed");
        assert!(next.get("symbolId").is_none());
        assert!(
            serde_json::from_value::<harness_domain::contracts::ExploreArgs>(next.clone()).is_ok()
        );
        assert_eq!(read(&mut g, "s", "read", "d.rs")["diagnostics"], json!([]));
        g.observe("s", "explore", &json!({}), &mut json!({"state":"error"}));
        read(&mut g, "s", "symbol", "a.rs");
        read(&mut g, "s", "read", "b.rs");
        assert_eq!(
            read(&mut g, "s", "read", "c.rs")["diagnostics"][0]["code"],
            "EXPLORE_RECOMMENDED"
        );
    }
    #[test]
    fn sessions_details_and_errors_do_not_pollute_the_threshold() {
        let mut g = Guidance::default();
        read(&mut g, "a", "symbol", "one.rs");
        read(&mut g, "b", "read", "two.rs");
        let mut failed = json!({"state":"error","diagnostics":[],"data":{"path":"two.rs"}});
        g.observe("a", "read", &json!({}), &mut failed);
        read(&mut g, "a", "other", "two.rs");
        assert_eq!(
            read(&mut g, "a", "read", "three.rs")["diagnostics"],
            json!([])
        );
        assert_eq!(
            read(&mut g, "a", "read", "four.rs")["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn no_invented_symbol_and_late_outline_can_supply_candidate() {
        let mut g = Guidance::default();
        for p in ["a.rs", "b.rs", "c.rs"] {
            assert_eq!(read(&mut g, "s", "read", p)["diagnostics"], json!([]));
        }
        let mut out = json!({"state":"ok","diagnostics":[],"data":{"path":"c.rs","items":[{"name":"real","container":"Type"}]}});
        g.observe("s", "outline", &json!({}), &mut out);
        assert_eq!(
            out["diagnostics"][0]["nextCall"]["arguments"]["symbol"],
            "real"
        );
        assert_eq!(
            out["diagnostics"][0]["nextCall"]["arguments"]["container"],
            "Type"
        );
    }
    #[test]
    fn bounded_storage_and_external_reads_excluded() {
        let mut g = Guidance::default();
        for i in 0..200 {
            read(&mut g, &format!("s{i}"), "read", "a.rs");
        }
        assert_eq!(g.sessions.len(), MAX_SESSIONS);
        let mut out = json!({"state":"ok","diagnostics":[],"data":{"path":"outside.rs"}});
        g.observe(
            "external",
            "read",
            &json!({"externalScope":"grant"}),
            &mut out,
        );
        assert!(!g.sessions.iter().any(|(id, _)| id == "external"));
    }
}

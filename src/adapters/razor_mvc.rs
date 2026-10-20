//! Literal MVC routes. Resolution uses one immutable, bounded controller snapshot
//! per publication; no project scans, source reparses or name-only fallback.
use harness_domain::{Failure, execution::OperationContext};
use regex::Regex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Route {
    root: String,
    area: String,
    controller: String,
    action: String,
}

fn regex(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("constant MVC pattern"))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn view_context(path: &str) -> Option<Route> {
    let path = path.replace('\\', "/");
    let parts = path.split('/').collect::<Vec<_>>();
    let views = parts.iter().rposition(|p| *p == "Views")?;
    let controller = *parts.get(views + 1)?;
    if !identifier(controller) || views + 2 >= parts.len() {
        return None;
    }
    let (root_end, area) = if views >= 2 && parts[views - 2] == "Areas" {
        (views - 2, parts[views - 1])
    } else {
        (views, "")
    };
    Some(Route {
        root: parts[..root_end].join("/"),
        area: area.to_owned(),
        controller: if controller == "Shared" {
            String::new()
        } else {
            controller.to_owned()
        },
        action: String::new(),
    })
}

// Exact C# literals only: escaped/interpolated/verbatim/dynamic values defer.
fn literal(value: &str) -> Option<&str> {
    let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    (value.is_empty() || identifier(value)).then_some(value)
}

fn split_arguments(text: &str) -> Option<Vec<&str>> {
    let mut start = 0;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut args = Vec::new();
    for (at, ch) in text.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                args.push(text[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    if quoted || depth != 0 {
        return None;
    }
    args.push(text[start..].trim());
    Some(args)
}

fn call_end(source: &str, start: usize) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 1usize;
    for (offset, ch) in source[start..].char_indices().take(16_384) {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn route_values(value: &str, route: &mut Route) -> Option<()> {
    let value = value.trim();
    if value == "null" {
        return Some(());
    }
    let fields = value
        .strip_prefix("new")?
        .trim()
        .strip_prefix('{')?
        .strip_suffix('}')?;
    for field in split_arguments(fields)? {
        if field.is_empty() {
            continue;
        }
        let (name, value) = field.split_once('=')?;
        let name = name.trim();
        // Other literal route values do not choose the action. Any dynamic
        // expression may mutate routing, so this subset deliberately defers.
        if !identifier(name) {
            return None;
        }
        if name == "area" {
            route.area = literal(value)?.to_owned();
        } else if name == "controller" || name == "action" {
            return None;
        } else if literal(value).is_none() && value.trim().parse::<i64>().is_err() {
            return None;
        }
    }
    Some(())
}

fn helper_route(text: &str, html: bool, context: &Route) -> Option<Route> {
    let args = split_arguments(text)?;
    let offset = usize::from(html);
    let mut route = context.clone();
    if args
        .iter()
        .any(|arg| arg.contains(": ") || arg.starts_with("actionName:"))
    {
        return None; // Named overload binding requires the C# semantic model.
    }
    route.action = literal(args.get(offset)?)?.to_owned();
    if !identifier(&route.action) {
        return None;
    }
    if let Some(controller) = args.get(offset + 1) {
        if let Some(value) = literal(controller) {
            if !identifier(value) {
                return None;
            }
            route.controller = value.to_owned();
            if let Some(values) = args.get(offset + 2) {
                route_values(values, &mut route)?;
            }
            // Html attributes cannot override routing but must use the supported
            // overload. Url.Action's protocol/host overloads stay unresolved.
            if args.len() > offset + if html { 4 } else { 3 } {
                return None;
            }
        } else {
            if !html && *controller != "null" {
                return None;
            }
            route_values(controller, &mut route)?;
            if !html {
                if let Some(values) = args.get(offset + 2) {
                    route_values(values, &mut route)?;
                }
                if args.len() > offset + 3 {
                    return None;
                }
            } else if args.len() > offset + 3 {
                return None;
            }
        }
    }
    Some(route)
}

/// One scan per view. Preserve byte offsets while removing comments and script
/// bodies; text inside them never becomes MVC evidence.
pub(super) fn collect(
    source: &str,
    path: &str,
    file_id: &str,
    excluded: &[(std::ops::Range<usize>, &'static str)],
    context: &OperationContext,
) -> Result<Vec<Value>, Failure> {
    let Some(view) = view_context(path) else {
        return Ok(Vec::new());
    };
    static HIDDEN: OnceLock<Regex> = OnceLock::new();
    let hidden = regex(
        &HIDDEN,
        r"(?s)@\*.*?\*@|<!--.*?-->|(?i:<script\b[^>]*>.*?</script\s*>)|(?i:<style\b[^>]*>.*?</style\s*>)",
    );
    let mut visible = source.as_bytes().to_vec();
    for found in hidden.find_iter(source) {
        visible[found.range()].fill(b' ');
    }
    static SHADOW: OnceLock<Regex> = OnceLock::new();
    let shadowed = regex(
        &SHADOW,
        r"\b(?:var|dynamic|[A-Za-z_][A-Za-z0-9_.<>?]*)\s+(Html|Url)\b|\b(Html|Url)\s*=",
    )
    .is_match(std::str::from_utf8(&visible).expect("masked UTF-8"));
    for (range, _) in excluded {
        // Member blocks contain C# strings/comments that resemble markup.
        // Their semantic calls belong to the C# bridge, not the MVC scanner.
        visible[range.clone()].fill(b' ');
    }
    // Explicit C# blocks/expressions may contain arbitrary strings resembling
    // helpers or tags. They need semantic binding; only direct Razor helper
    // expressions and markup attributes are accepted by this route subset.
    let mut cursor = 0;
    while cursor + 1 < visible.len() {
        if cursor % 4096 == 0 {
            context.check()?;
        }
        if visible[cursor] == b'@' && matches!(visible[cursor + 1], b'{' | b'(') {
            let left = visible[cursor + 1];
            let right = if left == b'{' { b'}' } else { b')' };
            if let Some(end) = super::razor_syntax::delimited_end(&visible, cursor + 1, left, right)
            {
                visible[cursor..end].fill(b' ');
                cursor = end;
                continue;
            }
        }
        cursor += 1;
    }
    let visible = String::from_utf8(visible).expect("spaces preserve UTF-8");
    static HELPER: OnceLock<Regex> = OnceLock::new();
    let mut found = Vec::new();
    if !shadowed {
        for capture in
            regex(&HELPER, r"@(Html\.ActionLink|Url\.Action)\s*\(").captures_iter(&visible)
        {
            context.check()?;
            let all = capture.get(0).unwrap();
            if all.start() > 0 && visible.as_bytes()[all.start() - 1] == b'@' {
                continue;
            }
            if let Some(end) = call_end(&visible, all.end()) {
                if let Some(route) = helper_route(
                    &visible[all.end()..end],
                    &capture[1] == "Html.ActionLink",
                    &view,
                ) {
                    if identifier(&route.controller) {
                        found.push((all.start(), route));
                    }
                }
            }
        }
    }
    static TAG: OnceLock<Regex> = OnceLock::new();
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    static ATTRIBUTE_NAME: OnceLock<Regex> = OnceLock::new();
    for tag in regex(
        &TAG,
        r#"(?is)<(?:a|form|button)\b(?:[^>"']|"[^"]*"|'[^']*')*>"#,
    )
    .find_iter(&visible)
    {
        context.check()?;
        let mut attrs = BTreeMap::new();
        let mut duplicate = false;
        for attr in regex(
            &ATTRIBUTE,
            r#"(?i)\b(asp-[a-z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
        )
        .captures_iter(tag.as_str())
        {
            let key = attr[1].to_ascii_lowercase();
            let value = attr.get(2).or_else(|| attr.get(3)).unwrap().as_str();
            duplicate |= attrs.insert(key, value).is_some();
        }
        // An unquoted/dynamic routing attribute must not silently disappear
        // and cause fallback to this view's controller or area.
        if regex(&ATTRIBUTE_NAME, r"(?i)\basp-[a-z-]+\s*=")
            .find_iter(tag.as_str())
            .count()
            != attrs.len()
        {
            continue;
        }
        let Some(action) = attrs.get("asp-action") else {
            continue;
        };
        if duplicate
            || !identifier(action)
            || attrs.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "asp-route"
                        | "asp-page"
                        | "asp-page-handler"
                        | "asp-route-controller"
                        | "asp-route-action"
                        | "asp-route-area"
                )
            })
        {
            continue;
        }
        let mut route = view.clone();
        route.action = (*action).to_owned();
        if let Some(controller) = attrs.get("asp-controller") {
            if !identifier(controller) {
                continue;
            }
            route.controller = (*controller).to_owned();
        }
        if let Some(area) = attrs.get("asp-area") {
            if !area.is_empty() && !identifier(area) {
                continue;
            }
            route.area = (*area).to_owned();
        }
        if identifier(&route.controller) {
            found.push((tag.start(), route));
        }
    }
    if found.len() > 20_000 {
        return Err(("EXTRACTION_LIMIT", "Too many Razor MVC routes"));
    }
    found.sort_by_key(|(at, _)| *at);
    // Compute line/UTF-16 column in one monotonic pass, rather than repeatedly
    // rescanning the view prefix for each link.
    let (mut cursor, mut line, mut column) = (0, 1usize, 0usize);
    let mut refs = Vec::with_capacity(found.len());
    for (at, route) in found {
        context.check()?;
        for ch in source[cursor..at].chars() {
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += ch.len_utf16();
            }
        }
        cursor = at;
        refs.push(
            json!({"fromNodeId":file_id,"referenceName":serde_json::to_string(&route).unwrap(),
            "referenceKind":"mvc_route","line":line,"column":column,"filePath":path}),
        );
    }
    Ok(refs)
}

type Actions = BTreeMap<String, Option<String>>;
#[derive(Default)]
pub(super) struct Snapshot {
    controllers: BTreeMap<(String, String, String), Option<Actions>>,
    routes: BTreeMap<String, BTreeMap<String, Option<String>>>,
    route_count: usize,
    bytes: usize,
    #[cfg(test)]
    queries: usize,
    #[cfg(test)]
    parses: usize,
}

fn sql<T>(result: rusqlite::Result<T>) -> Result<T, Failure> {
    result.map_err(|_| ("INDEX_SQL", "Razor MVC index lookup failed"))
}

impl Snapshot {
    pub(super) fn resolve(
        &mut self,
        db: &Connection,
        name: &str,
        path: &str,
        context: &OperationContext,
    ) -> Result<Option<String>, Failure> {
        context.check()?;
        if let Some(target) = self.routes.get(path).and_then(|routes| routes.get(name)) {
            return Ok(target.clone());
        }
        if name.len() > 2048 || path.len() > 2048 {
            return Ok(None);
        }
        let Ok(route) = serde_json::from_str::<Route>(name) else {
            return Ok(None);
        };
        #[cfg(test)]
        {
            self.parses += 1;
        }
        let Some(view) = view_context(path) else {
            return Ok(None);
        };
        if route.root != view.root
            || !identifier(&route.controller)
            || !identifier(&route.action)
            || (!route.area.is_empty() && !identifier(&route.area))
        {
            return Ok(None);
        }
        let key = (
            if cfg!(windows) {
                route.root.to_ascii_lowercase()
            } else {
                route.root.clone()
            },
            route.area.to_ascii_lowercase(),
            route.controller.to_ascii_lowercase(),
        );
        if !self.controllers.contains_key(&key) {
            // A saturated publication defers rather than redoing uncached work.
            if self.controllers.len() >= 128 {
                return Ok(None);
            }
            let actions = self.load(db, &route, context)?;
            let bytes = 256
                + key.0.len()
                + key.1.len()
                + key.2.len()
                + actions.as_ref().map_or(0, |a| {
                    a.iter()
                        .map(|(name, id)| 128 + name.len() + id.as_ref().map_or(0, String::len))
                        .sum::<usize>()
                });
            if bytes > (4 * 1024 * 1024usize).saturating_sub(self.bytes) {
                return Ok(None);
            }
            self.bytes += bytes;
            self.controllers.insert(key.clone(), actions);
        }
        let target = self
            .controllers
            .get(&key)
            .and_then(Option::as_ref)
            .and_then(|actions| actions.get(&route.action.to_ascii_lowercase()))
            .and_then(Clone::clone);
        let bytes = 256 + path.len() + name.len() + target.as_ref().map_or(0, String::len);
        if self.route_count < 4096 && bytes <= (4 * 1024 * 1024usize).saturating_sub(self.bytes) {
            self.bytes += bytes;
            self.route_count += 1;
            self.routes
                .entry(path.to_owned())
                .or_default()
                .insert(name.to_owned(), target.clone());
        }
        Ok(target)
    }

    fn load(
        &mut self,
        db: &Connection,
        route: &Route,
        context: &OperationContext,
    ) -> Result<Option<Actions>, Failure> {
        let controller = format!("{}Controller", route.controller);
        let prefix = if route.root.is_empty() {
            String::new()
        } else {
            format!("{}/", route.root)
        };
        let directory = if route.area.is_empty() {
            format!("{prefix}Controllers/")
        } else {
            format!("{prefix}Areas/{}/Controllers/", route.area)
        };
        let mut statement = sql(db.prepare_cached("SELECT qualified_name,file_path,decorators FROM nodes WHERE lower(name)=lower(?1) AND language='csharp' AND kind='class' AND visibility='public' AND coalesce(is_static,0)=0 AND coalesce(is_abstract,0)=0 AND (type_parameters IS NULL OR type_parameters IN ('[]','null')) LIMIT 65"))?;
        let rows = sql(statement.query_map([&controller], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        }))?;
        let rows = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
        #[cfg(test)]
        {
            self.queries += 1;
        }
        if rows.len() >= 65 {
            return Ok(None);
        }
        let matches = rows
            .into_iter()
            .filter(|(_, path, decorators)| {
                let path = path.replace('\\', "/");
                let (path, directory) = if cfg!(windows) {
                    (path.to_ascii_lowercase(), directory.to_ascii_lowercase())
                } else {
                    (path, directory.clone())
                };
                path.strip_prefix(&directory)
                    .is_some_and(|tail| !tail.contains('/'))
                    && controller_attributes(decorators.as_deref(), &route.area)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Ok(None);
        }
        let (owner, path, _) = &matches[0];
        let mut statement = sql(db.prepare_cached("SELECT id,name,qualified_name,decorators,signature FROM nodes WHERE file_path=?1 COLLATE BINARY AND language='csharp' AND kind='method' AND visibility='public' AND coalesce(is_static,0)=0 AND coalesce(is_abstract,0)=0 AND (type_parameters IS NULL OR type_parameters IN ('[]','null')) LIMIT 2001"))?;
        let rows = sql(statement.query_map(params![path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        }))?;
        let rows = sql(rows.collect::<rusqlite::Result<Vec<_>>>())?;
        #[cfg(test)]
        {
            self.queries += 1;
        }
        if rows.len() > 2000 {
            return Ok(None);
        }
        let mut actions = Actions::new();
        for (id, name, qualified, decorators, signature) in rows {
            context.check()?;
            if qualified != format!("{owner}::{name}") {
                continue;
            }
            let Some(action) = action_name(&name, decorators.as_deref(), signature.as_deref())
            else {
                continue;
            };
            let entry = actions
                .entry(action.to_ascii_lowercase())
                .or_insert_with(|| Some(id.clone()));
            if entry.as_deref() != Some(&id) {
                *entry = None;
            }
        }
        Ok(Some(actions))
    }
}

fn attributes(raw: Option<&str>) -> String {
    fn strings(value: &Value, out: &mut String) {
        match value {
            Value::String(s) => {
                out.push_str(s);
                out.push(' ');
            }
            Value::Array(items) => {
                for item in items {
                    strings(item, out);
                }
            }
            Value::Object(_) => {
                out.push_str(&value.to_string());
            }
            _ => {}
        }
    }
    let raw = raw.unwrap_or("");
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        let mut out = String::new();
        strings(&value, &mut out);
        out
    } else {
        raw.to_owned()
    }
}

fn controller_attributes(raw: Option<&str>, area: &str) -> bool {
    let attrs = attributes(raw);
    if attrs.contains("NonController") {
        return false;
    }
    if !attrs.contains("Area") {
        return true;
    }
    static AREA: OnceLock<Regex> = OnceLock::new();
    let Some(capture) = regex(
        &AREA,
        r#"\bArea(?:Attribute)?\s*\(\s*"([A-Za-z0-9_]*)"\s*\)"#,
    )
    .captures(&attrs) else {
        return false;
    };
    capture[1].eq_ignore_ascii_case(area)
}

fn action_name(name: &str, raw: Option<&str>, signature: Option<&str>) -> Option<String> {
    let attrs = format!("{} {}", attributes(raw), signature.unwrap_or(""));
    if attrs.contains("NonAction") {
        return None;
    }
    if !attrs.contains("ActionName") {
        return Some(name.to_owned());
    }
    static ACTION: OnceLock<Regex> = OnceLock::new();
    let captures = regex(
        &ACTION,
        r#"\bActionName(?:Attribute)?\s*\(\s*"([A-Za-z0-9_]+)"\s*\)"#,
    )
    .captures_iter(&attrs)
    .collect::<Vec<_>>();
    let first = captures.first()?;
    captures
        .iter()
        .all(|capture| capture[1] == first[1])
        .then(|| first[1].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(source: &str, path: &str) -> Vec<Route> {
        collect(source, path, "view", &[], &OperationContext::default())
            .unwrap()
            .iter()
            .map(|r| serde_json::from_str(r["referenceName"].as_str().unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn literal_helpers_and_tag_helpers_preserve_controller_and_area() {
        let result = routes(
            r#"@Html.ActionLink("Orders", "Index", "Orders")
@Url.Action("Show")
<a asp-action="Edit" asp-controller="Orders" asp-area="">go</a>
<form asp-action='Save' asp-area='Sales'></form>
@Url.Action("List", "Orders", new { area = "Sales", id = 1 })"#,
            "App/Areas/Admin/Views/Home/Index.cshtml",
        );
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].controller, "Orders");
        assert_eq!(result[0].area, "Admin");
        assert_eq!(result[1].controller, "Home");
        assert_eq!(result[2].area, "");
        assert_eq!(result[3].area, "Sales");
        assert_eq!(result[4].area, "Sales");
    }

    #[test]
    fn dynamic_comments_shared_views_shadowed_helpers_and_route_overrides_defer() {
        for source in [
            r#"@Url.Action(action, "Home")"#,
            r#"@Url.Action("Index", controller)"#,
            r#"@Url.Action("Index", "Home", values)"#,
            r#"@* @Url.Action("Index") *@ <!-- <a asp-action="Index"> -->"#,
            r#"<script>const x = '@Url.Action("Index")';</script>"#,
            r#"<a asp-action="@Model.Action" asp-controller="Home">"#,
            r#"<a asp-action="Index" asp-controller=@Model.Controller>"#,
            r#"<a asp-action="Index" asp-route="special">"#,
            r#"<a asp-action="Index" asp-action="Other">"#,
            r#"@{ var Url = custom; } @Url.Action("Index")"#,
            r#"@Url.Action("Index", "Home", new { area = Model.Area })"#,
            r#"@@Url.Action("Index")"#,
            r#"@{ var sample = "<a asp-action='Index'>"; }"#,
        ] {
            assert!(
                routes(source, "Views/Home/Index.cshtml").is_empty(),
                "{source}"
            );
        }
        assert!(routes(r#"@Url.Action("Index")"#, "Views/Shared/_Layout.cshtml").is_empty());
        assert_eq!(
            routes(
                r#"@Url.Action("Index", "Home")"#,
                "Views/Shared/_Layout.cshtml"
            )
            .len(),
            1
        );
    }

    #[test]
    fn mvc_reference_locations_use_original_lines_and_utf16_columns() {
        let source = "@* discarded 😀 *@\n😀 @Url.Action(\"Index\")\n<a asp-action='Edit'>";
        let result = collect(
            source,
            "Views/Home/Index.cshtml",
            "view",
            &[],
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["line"], 2);
        assert_eq!(result[0]["column"], 3);
        assert_eq!(result[1]["line"], 3);
        assert_eq!(result[1]["column"], 0);
    }

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT,name TEXT,kind TEXT,qualified_name TEXT,file_path TEXT,language TEXT,visibility TEXT,is_static INTEGER,is_abstract INTEGER,decorators TEXT,signature TEXT,type_parameters TEXT); CREATE INDEX by_name ON nodes(name); CREATE INDEX idx_nodes_lower_name ON nodes(lower(name)); CREATE INDEX by_file ON nodes(file_path);").unwrap();
        db.execute("INSERT INTO nodes VALUES('c','HomeController','class','App::HomeController','Controllers/HomeController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
        db.execute("INSERT INTO nodes VALUES('index','Index','method','App::HomeController::Index','Controllers/HomeController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
        db
    }

    fn resolve(cache: &mut Snapshot, db: &Connection, action: &str) -> Option<String> {
        let name = serde_json::to_string(&Route {
            root: String::new(),
            area: String::new(),
            controller: "Home".into(),
            action: action.into(),
        })
        .unwrap();
        cache
            .resolve(
                db,
                &name,
                "Views/Home/Index.cshtml",
                &OperationContext::default(),
            )
            .unwrap()
    }

    #[test]
    fn unique_controller_actions_cache_both_hits_and_misses_per_publication() {
        let db = database();
        let mut cache = Snapshot::default();
        for _ in 0..1000 {
            assert_eq!(resolve(&mut cache, &db, "Index").as_deref(), Some("index"));
            assert_eq!(resolve(&mut cache, &db, "Missing"), None);
        }
        assert_eq!(
            cache.queries, 2,
            "one indexed class and file lookup, no per-link SQL"
        );
        assert_eq!(
            cache.parses, 2,
            "one JSON decode per distinct literal route, including misses"
        );
        db.execute("INSERT INTO nodes VALUES('overload','Index','method','App::HomeController::Index','Controllers/HomeController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
        assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
    }

    #[test]
    fn unrelated_homonyms_non_actions_and_aliases_never_bind() {
        for attribute in ["NonAction", "ActionName(\"Other\")"] {
            let db = database();
            db.execute(
                "UPDATE nodes SET decorators=?1 WHERE id='index'",
                [attribute],
            )
            .unwrap();
            assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
            if attribute.starts_with("ActionName") {
                assert_eq!(
                    resolve(&mut Snapshot::default(), &db, "Other").as_deref(),
                    Some("index")
                );
            }
        }
        let db = database();
        db.execute(
            "UPDATE nodes SET qualified_name='App::OtherController::Index' WHERE id='index'",
            [],
        )
        .unwrap();
        assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
        let context = OperationContext::default();
        context.cancel();
        assert!(
            Snapshot::default()
                .resolve(&db, "{}", "Views/Home/Index.cshtml", &context)
                .is_err()
        );
    }

    #[test]
    fn attribute_route_templates_do_not_change_literal_action_selection() {
        let db = database();
        db.execute(
            "UPDATE nodes SET decorators=?1 WHERE id='c'",
            [r#"["Route(\"[controller]/[action]\")"]"#],
        )
        .unwrap();
        db.execute(
            "UPDATE nodes SET decorators=?1 WHERE id='index'",
            [r#"["HttpGet(\"orders/{id}\")", "Route(\"all\")"]"#],
        )
        .unwrap();
        assert_eq!(
            resolve(&mut Snapshot::default(), &db, "Index").as_deref(),
            Some("index")
        );
        db.execute("UPDATE nodes SET is_abstract=1 WHERE id='c'", [])
            .unwrap();
        assert_eq!(resolve(&mut Snapshot::default(), &db, "Index"), None);
    }

    #[test]
    fn mvc_excludes_generic_static_noncontroller_and_namespaced_nonaction() {
        for update in [
            "UPDATE nodes SET type_parameters='[\"T\"]' WHERE id='index'",
            "UPDATE nodes SET type_parameters='[\"T\"]' WHERE id='c'",
            "UPDATE nodes SET is_static=1 WHERE id='index'",
            "UPDATE nodes SET decorators='Microsoft.AspNetCore.Mvc.NonControllerAttribute' WHERE id='c'",
            "UPDATE nodes SET decorators='Microsoft.AspNetCore.Mvc.NonActionAttribute' WHERE id='index'",
        ] {
            let db = database();
            db.execute(update, []).unwrap();
            assert_eq!(
                resolve(&mut Snapshot::default(), &db, "Index"),
                None,
                "{update}"
            );
        }
        let db = database();
        db.execute(
            "UPDATE nodes SET decorators=?1 WHERE id='index'",
            [r#"["Microsoft.AspNetCore.Mvc.ActionNameAttribute(\"Other\")"]"#],
        )
        .unwrap();
        assert_eq!(
            resolve(&mut Snapshot::default(), &db, "Other").as_deref(),
            Some("index")
        );
        let attrs = r#"["Microsoft.AspNetCore.Mvc.AreaAttribute(\"Admin\")"]"#;
        assert!(controller_attributes(Some(attrs), "Admin"));
        assert!(!controller_attributes(Some(attrs), "Sales"));
        assert!(!controller_attributes(Some("Area(Model.Area)"), "Admin"));
    }

    #[test]
    fn mvc_controller_and_action_case_variants_share_indexed_snapshot() {
        let db = database();
        db.execute("UPDATE nodes SET name='OrdersController',qualified_name='App::OrdersController' WHERE id='c'", []).unwrap();
        db.execute(
            "UPDATE nodes SET qualified_name='App::OrdersController::Index' WHERE id='index'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE nodes SET file_path='Controllers/OrdersController.cs'",
            [],
        )
        .unwrap();
        let refs = collect(
            r#"@Url.Action("index", "orders") @Url.Action("INDEX", "ORDERS")"#,
            "Views/Orders/Index.cshtml",
            "view",
            &[],
            &OperationContext::default(),
        )
        .unwrap();
        let mut snapshot = Snapshot::default();
        for reference in refs {
            assert_eq!(
                snapshot
                    .resolve(
                        &db,
                        reference["referenceName"].as_str().unwrap(),
                        "Views/Orders/Index.cshtml",
                        &OperationContext::default()
                    )
                    .unwrap()
                    .as_deref(),
                Some("index")
            );
        }
        assert_eq!(snapshot.queries, 2);
        let plan: String = db
            .query_row(
                "EXPLAIN QUERY PLAN SELECT id FROM nodes WHERE lower(name)=lower(?1)",
                ["ordersController"],
                |row| row.get(3),
            )
            .unwrap();
        assert!(plan.contains("idx_nodes_lower_name"), "{plan}");
        // A case-folded homonym is still ambiguous, never first-row wins.
        db.execute("INSERT INTO nodes VALUES('overload','index','method','App::OrdersController::index','Controllers/OrdersController.cs','csharp','public',0,0,NULL,NULL,NULL)", []).unwrap();
        let route = Route {
            root: String::new(),
            area: String::new(),
            controller: "orders".into(),
            action: "index".into(),
        };
        assert_eq!(
            Snapshot::default()
                .resolve(
                    &db,
                    &serde_json::to_string(&route).unwrap(),
                    "Views/Orders/Index.cshtml",
                    &OperationContext::default()
                )
                .unwrap(),
            None
        );
    }
}

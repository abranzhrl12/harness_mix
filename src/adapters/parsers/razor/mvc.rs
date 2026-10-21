//! Pure extraction of literal MVC routes from Razor source.
//! SQLite-backed route resolution lives in the index adapter.
use harness_domain::{Failure, execution::OperationContext};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(in crate::adapters) struct Route {
    pub(in crate::adapters) root: String,
    pub(in crate::adapters) area: String,
    pub(in crate::adapters) controller: String,
    pub(in crate::adapters) action: String,
}

fn regex(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("constant MVC pattern"))
}

pub(in crate::adapters) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(in crate::adapters) fn view_context(path: &str) -> Option<Route> {
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

/// Directories that can host a view referenced by name or path.
pub(in crate::adapters) struct ViewScope {
    /// Prefix before `Areas`/`Views`: the application root.
    pub(in crate::adapters) app_root: String,
    /// Prefix that owns the `Views` directory of this view.
    pub(in crate::adapters) area_root: String,
    pub(in crate::adapters) controller: String,
}

fn join(root: &str, rest: &str) -> String {
    if root.is_empty() {
        rest.to_owned()
    } else {
        format!("{root}/{rest}")
    }
}

/// Scope of a view file: its application root, its area root and its
/// controller directory.
pub(in crate::adapters) fn view_scope(path: &str) -> Option<ViewScope> {
    let path = path.replace('\\', "/");
    let parts = path.split('/').collect::<Vec<_>>();
    let views = parts.iter().rposition(|p| *p == "Views")?;
    let controller = *parts.get(views + 1)?;
    if !identifier(controller) {
        return None;
    }
    let (app_root, area_root) = if views >= 2 && parts[views - 2] == "Areas" {
        (parts[..views - 2].join("/"), parts[..views].join("/"))
    } else {
        (parts[..views].join("/"), parts[..views].join("/"))
    };
    Some(ViewScope {
        app_root,
        area_root,
        controller: controller.to_owned(),
    })
}

/// Candidate view files for a literal partial or layout name. Bare names
/// resolve inside the controller directory and then the shared directory;
/// `~/`-prefixed names resolve from the application and area roots.
pub(in crate::adapters) fn view_candidates(path: &str, name: &str) -> Vec<String> {
    let Some(scope) = view_scope(path) else {
        return Vec::new();
    };
    let literal = name.trim();
    if !view_name(literal).is_some() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if literal.contains('/') {
        let relative = literal.trim_start_matches("~/").trim_start_matches('/');
        if relative.is_empty() {
            return Vec::new();
        }
        candidates.push(join(&scope.area_root, relative));
        candidates.push(join(&scope.app_root, relative));
    } else {
        let file = if literal.ends_with(".cshtml") {
            literal.to_owned()
        } else {
            format!("{literal}.cshtml")
        };
        candidates.push(join(
            &scope.area_root,
            &format!("Views/{}/{}", scope.controller, file),
        ));
        candidates.push(join(&scope.area_root, &format!("Views/Shared/{file}")));
    }
    candidates.dedup();
    candidates
}

/// Literal view name: no interpolation, no escapes and no markup delimiters.
fn view_name(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 {
        return None;
    }
    if value.contains(['{', '}', '$', '\\', '"', '\n', '\r']) {
        return None;
    }
    Some(value)
}

/// Quoted literal view name.
fn view_literal(value: &str) -> Option<&str> {
    let value = value.trim();
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    view_name(inner)
}

/// MVC helper calls owned by this scanner. The C# bridge reports them as
/// plain calls; the MVC evidence is richer and replaces them.
pub(in crate::adapters) fn is_mvc_helper_call(name: &str) -> bool {
    matches!(
        name,
        "Html.ActionLink"
            | "Html.Partial"
            | "Html.RenderPartial"
            | "Html.Action"
            | "Html.RenderAction"
            | "Url.Action"
    )
}

/// Byte ranges covered by C# string, char and comment literals.
fn string_ranges(text: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut at = 0usize;
    while at < text.len() {
        match text[at] {
            b'"' | b'\'' => {
                let quote = text[at];
                let start = at;
                at += 1;
                while at < text.len() {
                    if text[at] == b'\\' {
                        at += 2;
                        continue;
                    }
                    if text[at] == quote {
                        break;
                    }
                    at += 1;
                }
                at = (at + 1).min(text.len());
                ranges.push(start..at);
            }
            b'/' if text.get(at + 1) == Some(&b'/') => {
                while at < text.len() && text[at] != b'\n' {
                    at += 1;
                }
            }
            b'/' if text.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < text.len() && !(text[at] == b'*' && text[at + 1] == b'/') {
                    at += 1;
                }
                at = (at + 2).min(text.len());
            }
            _ => at += 1,
        }
    }
    ranges
}

/// Inline C# regions of a Razor view: `@{ ... }` and `@(...)`. Legacy views
/// keep MVC helper calls inside them.
fn code_regions(text: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut regions = Vec::new();
    let mut at = 0usize;
    while at + 1 < text.len() {
        if text[at] == b'@' && matches!(text[at + 1], b'{' | b'(') {
            let left = text[at + 1];
            let right = if left == b'{' { b'}' } else { b')' };
            if let Some(end) =
                crate::adapters::parsers::razor::syntax::delimited_end(text, at + 1, left, right)
            {
                regions.push(at + 1..end.saturating_sub(1));
                at = end;
                continue;
            }
        }
        at += 1;
    }
    regions
}

/// One scan per view. Preserve byte offsets while removing comments and script
/// bodies; text inside them never becomes MVC evidence.
pub(in crate::adapters) fn collect(
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
    // Member blocks contain C# strings/comments that resemble markup. Their
    // semantic calls belong to the C# bridge, not the MVC scanner.
    let code = visible.clone();
    for (range, _) in excluded {
        visible[range.clone()].fill(b' ');
    }
    // Inline C# regions keep their helper calls: legacy views call
    // `Html.RenderPartial` or `Html.RenderAction` inside `@{ ... }` blocks.
    let mut regions = code_regions(&code);
    // Member blocks are scanned as code regions too: `@code` and `@functions`
    // may render partials or actions.
    regions.extend(excluded.iter().map(|(range, _)| range.clone()));
    // Markup evidence ignores inline C# regions: their strings may resemble
    // helpers or tags and need semantic binding.
    for region in &regions {
        visible[region.clone()].fill(b' ');
    }
    let visible = String::from_utf8(visible).expect("spaces preserve UTF-8");
    let code = String::from_utf8(code).expect("spaces preserve UTF-8");
    static HELPER: OnceLock<Regex> = OnceLock::new();
    let mut found: Vec<(usize, &'static str, String)> = Vec::new();
    if !shadowed {
        // One scan over markup and inline C# regions. Legacy views call
        // `Html.RenderPartial` or `Html.RenderAction` inside `@{ ... }` blocks
        // without the Razor transition character.
        for capture in regex(
            &HELPER,
            r"(?:@)?(Html\.(?:ActionLink|RenderPartial|Partial|RenderAction|Action)|Url\.Action)\s*\(",
        )
        .captures_iter(&code)
        {
            context.check()?;
            let all = capture.get(0).unwrap();
            let method = capture.get(1).map_or("", |m| m.as_str());
            // A call written inside a C# string is not a call.
            if let Some(region) = regions.iter().find(|range| range.contains(&all.start())) {
                let offset = all.start() - region.start;
                let literals = string_ranges(code[region.clone()].as_bytes());
                if literals.iter().any(|range| range.contains(&offset)) {
                    continue;
                }
            }
            if all.start() > 0 && code.as_bytes()[all.start() - 1] == b'@' {
                continue;
            }
            let Some(end) = call_end(&code, all.end()) else {
                continue;
            };
            let arguments = &code[all.end()..end];
            if matches!(method, "Html.Partial" | "Html.RenderPartial") {
                let Some(arguments) = split_arguments(arguments) else {
                    continue;
                };
                let Some(name) = arguments.first().and_then(|value| view_literal(value)) else {
                    continue;
                };
                found.push((all.start(), "razor_partial", name.to_owned()));
                continue;
            }
            if let Some(route) = helper_route(arguments, method == "Html.ActionLink", &view)
                && identifier(&route.controller)
            {
                found.push((
                    all.start(),
                    "mvc_route",
                    serde_json::to_string(&route).expect("route serializes"),
                ));
            }
        }
        // `Layout = "..."` selects the layout of this view.
        static LAYOUT: OnceLock<Regex> = OnceLock::new();
        for region in &regions {
            context.check()?;
            let text = &code[region.clone()];
            for capture in regex(&LAYOUT, r#"\bLayout\s*=\s*"([^"]*)""#).captures_iter(text) {
                context.check()?;
                let Some(name) = capture.get(1).and_then(|value| view_name(value.as_str())) else {
                    continue;
                };
                found.push((
                    region.start + capture.get(0).unwrap().start(),
                    "razor_layout",
                    name.to_owned(),
                ));
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
            found.push((
                tag.start(),
                "mvc_route",
                serde_json::to_string(&route).expect("route serializes"),
            ));
        }
    }
    if found.len() > 20_000 {
        return Err(("EXTRACTION_LIMIT", "Too many Razor MVC routes"));
    }
    found.sort_by_key(|(at, _, _)| *at);
    // Compute line/UTF-16 column in one monotonic pass, rather than repeatedly
    // rescanning the view prefix for each link.
    let (mut cursor, mut line, mut column) = (0, 1usize, 0usize);
    let mut refs = Vec::with_capacity(found.len());
    for (at, kind, name) in found {
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
        refs.push(json!({"fromNodeId":file_id,"referenceName":name,
            "referenceKind":kind,"line":line,"column":column,"filePath":path}));
    }
    Ok(refs)
}

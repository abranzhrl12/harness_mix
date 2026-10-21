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
            if let Some(end) = crate::adapters::parsers::razor::syntax::delimited_end(
                &visible,
                cursor + 1,
                left,
                right,
            ) {
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

use harness_domain::browser::BrowserTarget;
use url::Url;

/// Returns `true` when `url` is an `http://` or `https://` URL pointing to a loopback host
/// (`localhost`, `127.0.0.1`, `::1`, or `[::1]`).
pub(crate) fn is_localhost_url(url_str: &str) -> bool {
    let Ok(url) = Url::parse(url_str) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// Ranks targets so `localhost` / loopback tabs appear first, followed by regular web pages,
/// and internal `chrome://` / `devtools://` / `about:` tabs last.
pub(crate) fn rank_targets(targets: &mut [BrowserTarget], filter: Option<&str>) {
    let normalized_filter = filter.map(str::trim).filter(|value| !value.is_empty());
    targets.sort_by(|left, right| {
        let left_score = target_score(left, normalized_filter);
        let right_score = target_score(right, normalized_filter);
        right_score
            .cmp(&left_score)
            .then_with(|| left.url.cmp(&right.url))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Selects the best matching target from `targets`.
///
/// - When `filter` is provided (and is not `"auto"` or `"localhost"`), returns the highest-scoring
///   target that matches `filter` by exact `targetId`, port, URL substring, or title substring.
/// - When `filter` is `None`, `"auto"`, or `"localhost"`, returns the highest-ranked `localhost`
///   tab if at least one exists.
pub(crate) fn select_target<'a>(
    targets: &'a [BrowserTarget],
    filter: Option<&str>,
) -> Option<&'a BrowserTarget> {
    let trimmed = filter.map(str::trim).filter(|value| !value.is_empty());
    match trimmed {
        Some(f) if f.eq_ignore_ascii_case("auto") || f.eq_ignore_ascii_case("localhost") => {
            targets
                .iter()
                .filter(|target| target.is_localhost)
                .max_by_key(|target| target_score(target, None))
        }
        Some(f) => targets
            .iter()
            .filter_map(|target| {
                let match_score = filter_match_score(target, f);
                (match_score > 0).then_some((match_score, target_score(target, Some(f)), target))
            })
            .max_by_key(|(match_score, total_score, _)| (*match_score, *total_score))
            .map(|(_, _, target)| target),
        None => targets
            .iter()
            .filter(|target| target.is_localhost)
            .max_by_key(|target| target_score(target, None)),
    }
}

fn target_score(target: &BrowserTarget, filter: Option<&str>) -> i32 {
    let mut score = 0;
    if let Some(f) = filter {
        score += filter_match_score(target, f) * 100;
    }
    if target.is_localhost {
        score += 50;
    } else if target.url.starts_with("http://") || target.url.starts_with("https://") {
        score += 10;
    } else if target.url.starts_with("devtools://")
        || target.url.starts_with("chrome://")
        || target.url.starts_with("edge://")
        || target.url.starts_with("about:")
    {
        score -= 20;
    }
    score
}

fn filter_match_score(target: &BrowserTarget, filter: &str) -> i32 {
    let filter = filter.trim();
    if filter.is_empty() {
        return 0;
    }
    if target.id == filter || target.target_id == filter {
        return 100;
    }
    if filter.eq_ignore_ascii_case("localhost") || filter.eq_ignore_ascii_case("auto") {
        return if target.is_localhost { 40 } else { 0 };
    }

    let filter_lower = filter.to_ascii_lowercase();
    let url_lower = target.url.to_ascii_lowercase();
    let title_lower = target.title.to_ascii_lowercase();

    // Check if filter is a port number (`:3000` or `3000`) or a URL with a port
    if let Some(port) = extract_filter_port(&filter_lower) {
        if let Ok(parsed_url) = Url::parse(&target.url) {
            if parsed_url.port_or_known_default() == Some(port)
                || parsed_url.port() == Some(port)
            {
                // If filter also specifies a path, check path match for higher specificity
                if let Ok(filter_url) = Url::parse(&filter_lower) {
                    let filter_path = filter_url.path().trim_end_matches('/');
                    let target_path = parsed_url.path().trim_end_matches('/');
                    if !filter_path.is_empty() && target_path.starts_with(filter_path) {
                        return 95;
                    }
                    if !filter_path.is_empty() && !target_path.contains(filter_path) {
                        return 0;
                    }
                }
                return if target.is_localhost { 90 } else { 75 };
            }
        }
    }

    if url_lower == filter_lower
        || url_lower.trim_end_matches('/') == filter_lower.trim_end_matches('/')
    {
        return 95;
    }

    if url_lower.contains(&filter_lower) {
        return if target.is_localhost { 80 } else { 65 };
    }

    if title_lower == filter_lower {
        return 70;
    }

    if title_lower.contains(&filter_lower) {
        return 55;
    }

    0
}

fn extract_filter_port(filter: &str) -> Option<u16> {
    if let Some(stripped) = filter.strip_prefix(':') {
        return stripped
            .split('/')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|&p| p > 0);
    }
    if let Ok(port) = filter.parse::<u16>() {
        if port > 0 {
            return Some(port);
        }
    }
    if let Ok(url) = Url::parse(filter) {
        return url.port();
    }
    if let Ok(url) = Url::parse(&format!("http://{filter}")) {
        let host = url.host_str().unwrap_or_default();
        if matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]") {
            return url.port();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, title: &str, url: &str) -> BrowserTarget {
        BrowserTarget::new(id, title, url, "page", is_localhost_url(url))
    }

    #[test]
    fn detects_localhost_urls_across_loopback_variants() {
        assert!(is_localhost_url("http://localhost:3000/app"));
        assert!(is_localhost_url("https://localhost:5173/"));
        assert!(is_localhost_url("http://127.0.0.1:8080/login"));
        assert!(is_localhost_url("http://[::1]:4200/"));
        assert!(!is_localhost_url("https://example.com:3000/"));
        assert!(!is_localhost_url("chrome://inspect"));
        assert!(!is_localhost_url("devtools://devtools/bundled/inspector.html"));
        assert!(!is_localhost_url("about:blank"));
    }

    #[test]
    fn ranks_localhost_tabs_before_external_and_internal_tabs() {
        let mut tabs = vec![
            target("t-blank", "New Tab", "chrome://newtab/"),
            target("t-ext", "Docs", "https://docs.rs/tokio"),
            target("t-local-2", "Vite App", "http://127.0.0.1:5173/dashboard"),
            target("t-local-1", "Main App", "http://localhost:3000/"),
        ];

        rank_targets(&mut tabs, None);

        assert!(tabs[0].is_localhost);
        assert!(tabs[1].is_localhost);
        assert_eq!(tabs[2].id, "t-ext");
        assert_eq!(tabs[3].id, "t-blank");
    }

    #[test]
    fn selects_default_localhost_tab_when_no_filter_supplied() {
        let mut tabs = vec![
            target("t-ext", "External", "https://google.com"),
            target("t-local", "Local App", "http://localhost:5173/"),
        ];
        rank_targets(&mut tabs, None);

        let selected = select_target(&tabs, None).expect("should select localhost tab");
        assert_eq!(selected.id, "t-local");
    }

    #[test]
    fn selects_tab_by_port_substring_title_or_target_id() {
        let tabs = vec![
            target("target-3000", "Backend Admin", "http://localhost:3000/admin"),
            target("target-5173", "Frontend Portal", "http://127.0.0.1:5173/orders"),
            target("target-ext", "Production", "https://example.com/orders"),
        ];

        assert_eq!(
            select_target(&tabs, Some(":5173")).unwrap().id,
            "target-5173"
        );
        assert_eq!(
            select_target(&tabs, Some("5173")).unwrap().id,
            "target-5173"
        );
        assert_eq!(
            select_target(&tabs, Some("localhost:3000")).unwrap().id,
            "target-3000"
        );
        assert_eq!(
            select_target(&tabs, Some("Frontend Portal")).unwrap().id,
            "target-5173"
        );
        assert_eq!(
            select_target(&tabs, Some("target-ext")).unwrap().id,
            "target-ext"
        );
        assert_eq!(
            select_target(&tabs, Some("/orders")).unwrap().id,
            "target-5173",
            "localhost tab matching /orders should outrank external tab matching /orders"
        );
        assert!(select_target(&tabs, Some(":9999")).is_none());
    }
}

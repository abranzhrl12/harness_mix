use crate::adapters::{
    parsers::vue::structure::{attr, classes, element_classes, parse},
    project::Project,
};
use harness_domain::{
    Failure, contracts::VueExtractPreviewArgs, execution::OperationContext, hash, results::prefix,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, ops::Range, path::Path};

fn identifiers(text: &str) -> BTreeSet<String> {
    // Deliberately an over-approximation, never advertised as free-variable analysis.
    let mut out = BTreeSet::new();
    let b = text.as_bytes();
    let mut i = 0;
    let mut quote = None;
    while i < b.len() {
        if let Some(q) = quote {
            if b[i] == b'\\' {
                i += 1;
            } else if b[i] == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if matches!(b[i], b'\'' | b'"' | b'`') {
            quote = Some(b[i]);
            i += 1;
            continue;
        }
        if b[i].is_ascii_alphabetic() || matches!(b[i], b'_' | b'$') {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'$')) {
                i += 1;
            }
            let name = &text[start..i];
            if ![
                "true",
                "false",
                "null",
                "undefined",
                "this",
                "if",
                "else",
                "return",
                "in",
                "of",
                "typeof",
                "new",
                "const",
                "let",
                "Math",
                "JSON",
                "Number",
                "String",
                "Array",
                "Object",
                "$event",
            ]
            .contains(&name)
                && !text[..start].trim_end().ends_with('.')
            {
                out.insert(name.to_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}

pub(in crate::adapters) fn extract_preview(
    project: &Project,
    args: &VueExtractPreviewArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if args.expected_hash.len() != 64
        || !args.expected_hash.bytes().all(|b| b.is_ascii_hexdigit())
        || args.symbol_id.len() != 24
        || !args.symbol_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err((
            "ARGUMENT",
            "expectedHash SHA-256 and current template symbolId are required",
        ));
    }
    let name = &args.component_name;
    if name.len() > 100
        || !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err((
            "ARGUMENT",
            "componentName must be a PascalCase ASCII identifier",
        ));
    }
    if Path::new(&args.path).extension().and_then(|s| s.to_str()) != Some("vue")
        || Path::new(&args.target_path)
            .extension()
            .and_then(|s| s.to_str())
            != Some("vue")
    {
        return Err(("ARGUMENT", "Both source and target must be Vue files"));
    }
    super::edit_commit::target(project, &args.path)?;
    let normalized = args.target_path.replace('\\', "/").to_ascii_lowercase();
    if normalized
        .split('/')
        .any(|p| [".git", ".hg", ".svn", ".agents", ".codegraph", "agents.md"].contains(&p))
        || super::project::external_restricted(&normalized)
        || super::project::external_sensitive(&normalized)
    {
        return Err(("PATH", "Protected extraction target"));
    }
    let target = project.target_optional(&args.target_path)?;
    if target
        .try_exists()
        .map_err(|_| ("PATH", "Cannot inspect target"))?
    {
        return Err(("TARGET_EXISTS", "Extraction target already exists"));
    }
    let bytes = project.bytes_with_context(&args.path, context)?;
    let digest = hash(&bytes);
    if !digest.eq_ignore_ascii_case(&args.expected_hash) {
        return Err(("STALE", "Vue source hash changed; request a new outline"));
    }
    let source =
        std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "Vue source must be UTF-8"))?;
    let doc = parse(source, &digest, context)?;
    let selected = doc
        .elements
        .iter()
        .find(|e| e.id == args.symbol_id)
        .ok_or((
            "STALE_SYMBOL",
            "Template element ID not found; request a new outline",
        ))?;
    if doc.symbols.iter().any(|s| s.name == *name)
        || doc
            .blocks
            .iter()
            .filter(|b| b.kind == "script")
            .any(|b| identifiers(&source[b.body.clone()]).contains(name))
    {
        return Err((
            "VUE_NAME_COLLISION",
            "Component name conflicts with a script identifier",
        ));
    }
    let mut issues = doc.notices.clone();
    let inside = |r: &Range<usize>| r.start >= selected.range.start && r.end <= selected.range.end;
    let selected_elements: Vec<_> = doc.elements.iter().filter(|e| inside(&e.range)).collect();
    let selected_classes: BTreeSet<_> = selected_elements
        .iter()
        .flat_map(|e| element_classes(e))
        .collect();
    let other_classes: BTreeSet<_> = doc
        .elements
        .iter()
        .filter(|e| !inside(&e.range))
        .flat_map(element_classes)
        .collect();
    let dynamic = doc.elements.iter().any(|e| {
        e.attrs
            .iter()
            .any(|a| matches!(a.name.as_str(), ":class" | "v-bind:class" | "v-bind"))
    });
    let mut deps = BTreeSet::new();
    let mut expressions = Vec::new();
    for (range, text) in &doc.expressions {
        if inside(range) {
            deps.extend(identifiers(text));
            expressions.push(text);
        }
    }
    if !expressions.is_empty() {
        issues.insert("Template expressions need lexical-scope, prop type and writable-binding review; suggested props are token candidates only".into());
    }
    for e in &selected_elements {
        if matches!(e.tag.as_str(), "slot" | "component") {
            issues.insert("Slots and dynamic components require component-boundary review".into());
        }
        if e.tag.chars().next().is_some_and(char::is_uppercase) || e.tag.contains('-') {
            issues.insert(format!(
                "Component import/registration must be resolved: {}",
                e.tag
            ));
        }
        for a in &e.attrs {
            if a.name == "ref"
                || a.name.starts_with("v-model")
                || a.name.starts_with("v-slot")
                || a.name.starts_with('#')
                || a.name == "v-for"
                || a.name == "v-html"
                || a.name == "v-pre"
            {
                issues.insert(format!(
                    "Review component-boundary semantics for {}",
                    a.name
                ));
            }
        }
    }
    if selected
        .attrs
        .iter()
        .any(|a| a.name == "v-else" || a.name == "v-else-if")
    {
        return Err((
            "VUE_BRANCH_BOUNDARY",
            "Cannot extract a dependent conditional branch alone",
        ));
    }
    // Also reject an if branch whose following sibling depends on it.
    if doc.elements.iter().any(|e| {
        e.parent == selected.parent
            && e.range.start >= selected.range.end
            && source[selected.range.end..e.range.start].trim().is_empty()
            && e.attrs
                .iter()
                .any(|a| a.name == "v-else" || a.name == "v-else-if")
    }) {
        return Err((
            "VUE_BRANCH_BOUNDARY",
            "Conditional chain must be reviewed/extracted together",
        ));
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut moved = Vec::new();
    let mut retained = Vec::new();
    let mut child_styles = String::new();
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for (block_index, block) in doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == "style")
    {
        let mut pieces = Vec::new();
        for root in doc
            .rules
            .iter()
            .filter(|r| r.block == block_index && r.parent.is_none())
        {
            let subtree: Vec<_> = doc
                .rules
                .iter()
                .filter(|r| r.range.start >= root.range.start && r.range.end <= root.range.end)
                .collect();
            let names: BTreeSet<_> = subtree
                .iter()
                .flat_map(|r| r.resolved.iter().flat_map(|s| classes(s)))
                .collect();
            if names.is_disjoint(&selected_classes) {
                continue;
            }
            let text = &source[root.range.clone()];
            let uncertain = dynamic
                || subtree.iter().any(|r| r.uncertain)
                || text.contains('$')
                || text.contains('@')
                || text.contains("var(")
                || text.contains("v-bind(")
                || attr(&block.attrs, "module").is_some()
                || text.contains("url(")
                || text.contains("animation")
                || (attr(&block.attrs, "lang") == Some("scss")
                    && (text.contains('(') || source[block.body.clone()].contains('@')))
                || root
                    .selector
                    .split(',')
                    .any(|s| !s.trim().starts_with('.') || classes(s).is_empty());
            let exclusive = !names.is_empty()
                && names.is_subset(&selected_classes)
                && names.is_disjoint(&other_classes);
            if exclusive && !uncertain {
                pieces.push(text);
                moved.push(root.selector.clone());
                edits.push((root.range.clone(), String::new()));
            } else {
                retained.push(root.selector.clone());
                issues.insert("Shared, dynamic or context-dependent style rules remain in the parent; scoped styling across the new boundary requires review".into());
            }
        }
        if !pieces.is_empty() {
            child_styles.push_str(newline);
            child_styles.push_str(&source[block.range.start..block.body.start]);
            child_styles.push_str(newline);
            child_styles.push_str(&pieces.join(newline));
            child_styles.push_str(newline);
            child_styles.push_str("</style>");
            child_styles.push_str(newline);
        }
    }
    if dynamic {
        issues.insert("Dynamic classes prevent proving BEM rule exclusivity".into());
    }
    let parent_dir = Path::new(&args.path).parent().unwrap_or(Path::new(""));
    let target_parts: Vec<_> = Path::new(&args.target_path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let parent_parts: Vec<_> = parent_dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let common = parent_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let import_path = format!(
        "{}{}",
        "../".repeat(parent_parts.len() - common),
        target_parts[common..].join("/")
    );
    let import_path = if import_path.starts_with('.') {
        import_path
    } else {
        format!("./{import_path}")
    };
    let import = format!(
        "{newline}import {name} from {};{newline}",
        json!(import_path)
    );
    if let Some(setup) = doc
        .blocks
        .iter()
        .find(|b| b.kind == "script" && attr(&b.attrs, "setup").is_some())
    {
        if attr(&setup.attrs, "src").is_some() {
            return Err((
                "VUE_EXTERNAL_SCRIPT",
                "External setup scripts cannot be rewritten in this preview",
            ));
        }
        edits.push((setup.body.start..setup.body.start, import));
    } else {
        edits.push((0..0, format!("<script setup>{import}</script>{newline}")));
    }
    let bindings = deps
        .iter()
        .map(|d| format!(" :{d}=\"{d}\""))
        .collect::<String>();
    let replacement = format!("<{name}{bindings} />");
    edits.push((selected.range.clone(), replacement));
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut parent = source.to_owned();
    for (range, text) in edits {
        parent.replace_range(range, &text);
    }
    let props = if deps.is_empty() {
        String::new()
    } else {
        format!(
            "<script setup>{newline}defineProps({});{newline}</script>{newline}{newline}",
            json!(deps)
        )
    };
    let child = format!(
        "{props}<template>{newline}{}{newline}</template>{newline}{child_styles}",
        &source[selected.range.clone()]
    );
    crate::adapters::parsers::vue::syntax::validate(&args.path, &parent, context)?;
    crate::adapters::parsers::vue::syntax::validate(&args.target_path, &child, context)?;
    parse(&parent, &hash(parent.as_bytes()), context)?;
    parse(&child, &hash(child.as_bytes()), context)?;
    // Optimistic planning also rechecks both observations; application will need
    // its own multi-file commit/recovery design and fresh checks.
    if hash(&project.bytes_with_context(&args.path, context)?) != digest {
        return Err(("STALE", "Vue source changed during preview"));
    }
    if target
        .try_exists()
        .map_err(|_| ("PATH", "Cannot recheck target"))?
    {
        return Err(("TARGET_EXISTS", "Target appeared during preview"));
    }
    let patch = format!(
        "{}{}",
        super::edit::preview_diff(&args.path, source, &parent),
        super::edit::preview_diff(&args.target_path, "", &child)
    );
    let files = json!([{"path":args.path,"operation":"replace","baseHash":digest,"candidateHash":hash(parent.as_bytes()),"content":parent},
        {"path":args.target_path,"operation":"create","expectedAbsent":true,"candidateHash":hash(child.as_bytes()),"content":child}]);
    Ok(
        json!({"path":args.path,"baseHash":digest,"symbolId":selected.id,"targetPath":args.target_path,
        "previewOnly":true,"writesPerformed":false,"applicationSupported":false,"reviewRequired":!issues.is_empty(),
        "interfaceProposal":{"props":deps,"emits":[],"certainty":"lexical_candidates","note":"Event handlers are proposed as callback props; events, scopes, v-model, slots and refs require review"},
        "styles":{"movedRules":moved,"retainedRules":retained,"strategy":"whole exclusive static rule trees; shared and uncertain rules retained"},
        "unresolved":issues,"syntaxValidation":"structural_subset_and_embedded_scripts","patch":prefix(&patch,12000),"truncated":patch.len()>12000,
        "notice":"Read-only proposal. No automatic multi-file application or rollback is provided. Review unresolved dependencies and complete file content before applying.",
        "_detail":{"files":files,"patch":patch}}),
    )
}

#[cfg(test)]
#[path = "vue_extract_tests.rs"]
mod tests;

//! Pure candidate filtering policy for the native index resolver.
//!
//! Database lookup and language-specific proof construction stay in adapters;
//! this module only applies the ordered, deterministic selection policy.

#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    pub kind: &'a str,
    pub file_path: &'a str,
    pub qualified_name: &'a str,
    pub visibility: Option<&'a str>,
    pub is_exported: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection<'a> {
    pub allowed_kinds: &'a [&'a str],
    pub source_path: &'a str,
    pub lookup: &'a str,
    pub name: &'a str,
    pub owner: &'a str,
    pub imported_path: Option<&'a str>,
    pub local_only: bool,
    pub razor_same_view: bool,
    /// A language-specific proof already selected the exact declaration.
    pub exact_proof: bool,
    /// Rust import/module proofs permit a private cross-file declaration.
    pub rust_proof: bool,
}

/// Return the original candidate index only when the ordered policy selects a
/// single declaration. `candidate` projects adapter-owned rows without
/// cloning them or allocating an intermediate collection.
pub fn select_candidate_index<T>(
    candidates: &[T],
    selection: Selection<'_>,
    candidate: impl Fn(&T) -> Candidate<'_>,
) -> Option<usize> {
    // The SQL query returns at most 65 rows: the 65th is an ambiguity sentinel.
    // Reject it before any filters, matching the adapter's historical policy.
    if candidates.len() > 64 {
        return None;
    }

    let base_matches = |row: &T| {
        base_matches_raw(candidate(row), selection)
    };

    let exact_proof = selection.exact_proof;
    let qualified = selection.lookup.contains('.') || selection.lookup.contains("::");
    if exact_proof {
        return unique_match(candidates, base_matches).1;
    }
    if qualified {
        return unique_match(candidates, |row| {
            base_matches(row) && candidate(row).qualified_name == selection.lookup
        })
        .1;
    }
    if selection.imported_path.is_some() {
        return unique_match(candidates, base_matches).1;
    }

    let container = selection
        .owner
        .rsplit_once("::")
        .map(|(parent, _)| format!("{parent}::"))
        .or_else(|| {
            selection
                .owner
                .rsplit_once('.')
                .map(|(parent, _)| format!("{parent}."))
        });
    if let Some(container) = container {
        let (count, index) = unique_match(candidates, |row| {
            let row = candidate(row);
            base_matches_raw(row, selection)
                && row.file_path == selection.source_path
                && row.qualified_name == format!("{container}{}", selection.name)
        });
        if count != 0 {
            return index;
        }
    }

    let (local_count, local_index) = unique_match(candidates, |row| {
        let row = candidate(row);
        base_matches_raw(row, selection) && row.file_path == selection.source_path
    });
    if local_count != 0 {
        return local_index;
    }

    unique_match(candidates, base_matches).1
}

fn unique_match<T>(
    candidates: &[T],
    mut matches: impl FnMut(&T) -> bool,
) -> (usize, Option<usize>) {
    let mut found = None;
    for (index, row) in candidates.iter().enumerate() {
        if matches(row) {
            if found.is_some() {
                return (2, None);
            }
            found = Some(index);
        }
    }
    (usize::from(found.is_some()), found)
}

fn base_matches_raw(row: Candidate<'_>, selection: Selection<'_>) -> bool {
    selection.allowed_kinds.contains(&row.kind)
        && (!selection.razor_same_view || row.file_path == selection.source_path)
        && selection
            .imported_path
            .is_none_or(|path| {
                row.file_path == path
                    && row.kind == "function"
                    && row.qualified_name == selection.lookup
            })
        && (!selection.local_only
            || selection.imported_path.is_some()
            || row.file_path == selection.source_path)
        && (selection.imported_path.is_some()
            || selection.rust_proof
            || row.file_path == selection.source_path
            || row.visibility == Some("public")
            || row.is_exported)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, Selection, select_candidate_index};

    const FUNCTIONS: &[&str] = &["function", "method"];
    const TYPES: &[&str] = &["class", "struct", "interface"];

    fn row(kind: &'static str, path: &'static str, qualified_name: &'static str, visibility: Option<&'static str>, is_exported: bool) -> Candidate<'static> {
        Candidate { kind, file_path: path, qualified_name, visibility, is_exported }
    }

    fn base_selection() -> Selection<'static> {
        Selection {
            allowed_kinds: FUNCTIONS,
            source_path: "src/a.rs",
            lookup: "run",
            name: "run",
            owner: "module::caller",
            imported_path: None,
            local_only: false,
            razor_same_view: false,
            exact_proof: false,
            rust_proof: false,
        }
    }

    fn select(rows: &[Candidate<'static>], selection: Selection<'_>) -> Option<usize> {
        select_candidate_index(rows, selection, |row| *row)
    }

    /// The pre-refactor adapter policy, retained as a test oracle. It models
    /// the ordered Vec::retain operations and their non-empty subset priority.
    fn legacy_select(rows: &[Candidate<'static>], selection: Selection<'_>) -> Option<usize> {
        if rows.len() > 64 {
            return None;
        }
        let mut candidates = (0..rows.len()).collect::<Vec<_>>();
        candidates.retain(|&index| selection.allowed_kinds.contains(&rows[index].kind));
        if selection.razor_same_view {
            candidates.retain(|&index| rows[index].file_path == selection.source_path);
        }
        if let Some(path) = selection.imported_path {
            candidates.retain(|&index| {
                rows[index].file_path == path
                    && rows[index].kind == "function"
                    && rows[index].qualified_name == selection.lookup
            });
        } else if selection.local_only {
            candidates.retain(|&index| rows[index].file_path == selection.source_path);
        }
        if selection.imported_path.is_none() && !selection.rust_proof {
            candidates.retain(|&index| {
                let row = rows[index];
                row.file_path == selection.source_path
                    || row.visibility == Some("public")
                    || row.is_exported
            });
        }
        if !selection.exact_proof {
            if selection.lookup.contains('.') || selection.lookup.contains("::") {
                candidates.retain(|&index| rows[index].qualified_name == selection.lookup);
            } else if selection.imported_path.is_none() {
                let container = selection
                    .owner
                    .rsplit_once("::")
                    .map(|(parent, _)| format!("{parent}::"))
                    .or_else(|| {
                        selection
                            .owner
                            .rsplit_once('.')
                            .map(|(parent, _)| format!("{parent}."))
                    });
                if let Some(container) = container {
                    let scoped = candidates
                        .iter()
                        .copied()
                        .filter(|&index| {
                            rows[index].file_path == selection.source_path
                                && rows[index].qualified_name
                                    == format!("{container}{}", selection.name)
                        })
                        .collect::<Vec<_>>();
                    if !scoped.is_empty() {
                        candidates = scoped;
                    }
                }
                let local = candidates
                    .iter()
                    .copied()
                    .filter(|&index| rows[index].file_path == selection.source_path)
                    .collect::<Vec<_>>();
                if !local.is_empty() {
                    candidates = local;
                }
            }
        }
        (candidates.len() == 1).then(|| candidates[0])
    }

    #[test]
    fn local_private_candidate_wins_and_cross_file_private_stays_pending() {
        let rows = [
            row("function", "src/a.rs", "module::run", None, false),
            row("function", "src/b.rs", "module::run", None, false),
        ];
        assert_eq!(select(&rows, base_selection()), Some(0));

        let rows = [row("function", "src/b.rs", "module::run", None, false)];
        assert_eq!(select(&rows, base_selection()), None);
    }

    #[test]
    fn cross_file_public_candidate_and_ambiguity_match_legacy_policy() {
        let rows = [row("function", "src/b.rs", "module::run", Some("public"), false)];
        assert_eq!(select(&rows, base_selection()), Some(0));

        let rows = [
            row("function", "src/a.rs", "module::run", None, false),
            row("function", "src/a.rs", "module::run", None, false),
        ];
        assert_eq!(select(&rows, base_selection()), None);
    }

    #[test]
    fn qualified_name_requires_exact_match_and_container_precedes_other_locals() {
        let rows = [
            row("function", "src/a.rs", "other::run", None, false),
            row("function", "src/a.rs", "module::run", None, false),
        ];
        assert_eq!(select(&rows, base_selection()), Some(1));

        let mut selection = base_selection();
        selection.lookup = "module::run";
        assert_eq!(select(&rows, selection), Some(1));
    }

    #[test]
    fn imported_binding_is_exact_and_local_only_blocks_cross_file_fallback() {
        let rows = [
            row("function", "src/lib.rs", "run", Some("public"), true),
            row("method", "src/lib.rs", "run", Some("public"), true),
        ];
        let mut selection = base_selection();
        selection.imported_path = Some("src/lib.rs");
        selection.lookup = "run";
        assert_eq!(select(&rows, selection), Some(0));

        let rows = [row("function", "src/b.rs", "module::run", Some("public"), true)];
        let mut selection = base_selection();
        selection.imported_path = None;
        selection.local_only = true;
        assert_eq!(select(&rows, selection), None);
    }

    #[test]
    fn rust_proof_skips_visibility_but_not_allowed_kinds_or_ambiguity() {
        let rows = [row("function", "src/b.rs", "module::run", None, false)];
        let mut selection = base_selection();
        selection.rust_proof = true;
        selection.exact_proof = true;
        assert_eq!(select(&rows, selection), Some(0));

        let rows = [row("class", "src/b.rs", "module::run", None, false)];
        assert_eq!(select(&rows, selection), None);
    }

    #[test]
    fn razor_same_view_and_proof_selection_preserve_exact_target_rules() {
        let rows = [
            row("method", "src/Other.razor", "global::Demo.Widget.Run", Some("public"), true),
            row("method", "src/View.razor", "global::Demo.Widget.Run", Some("public"), true),
        ];
        let mut selection = base_selection();
        selection.source_path = "src/View.razor";
        selection.lookup = "global::Demo.Widget.Run";
        selection.razor_same_view = true;
        selection.exact_proof = true;
        assert_eq!(select(&rows, selection), Some(1));
    }

    #[test]
    fn sixty_fifth_candidate_is_rejected_before_filters() {
        let rows = vec![row("class", "src/a.rs", "module::run", None, false); 65];
        let mut selection = base_selection();
        selection.allowed_kinds = TYPES;
        assert_eq!(select(&rows, selection), None);
    }

    #[test]
    fn new_policy_matches_legacy_oracle_for_candidate_subsets_and_contexts() {
        let rows = [
            row("function", "src/a.rs", "module::run", None, false),
            row("function", "src/a.rs", "other::run", Some("private"), false),
            row("function", "src/lib.rs", "run", Some("public"), true),
            row("method", "src/a.rs", "module::run", Some("public"), false),
            row("class", "src/a.rs", "module::run", None, false),
            row("function", "src/b.rs", "module::run", Some("public"), true),
        ];
        let mut contexts = Vec::new();
        contexts.push(base_selection());

        let mut local_only = base_selection();
        local_only.local_only = true;
        contexts.push(local_only);

        let mut imported = base_selection();
        imported.imported_path = Some("src/lib.rs");
        imported.lookup = "run";
        contexts.push(imported);

        let mut qualified = base_selection();
        qualified.lookup = "module::run";
        contexts.push(qualified);

        let mut rust_proof = base_selection();
        rust_proof.rust_proof = true;
        rust_proof.exact_proof = true;
        contexts.push(rust_proof);

        let mut razor_proof = base_selection();
        razor_proof.source_path = "src/View.razor";
        razor_proof.razor_same_view = true;
        razor_proof.exact_proof = true;
        contexts.push(razor_proof);

        for mask in 0..(1 << rows.len()) {
            let subset = rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| (mask & (1 << index) != 0).then_some(*row))
                .collect::<Vec<_>>();
            for selection in &contexts {
                assert_eq!(
                    select(&subset, *selection),
                    legacy_select(&subset, *selection),
                    "policy mismatch for subset {mask:#08b} and context {selection:?}"
                );
            }
        }

        for size in 0..=65 {
            let expanded = (0..size).map(|index| rows[index % rows.len()]).collect::<Vec<_>>();
            for selection in &contexts {
                assert_eq!(
                    select(&expanded, *selection),
                    legacy_select(&expanded, *selection),
                    "policy mismatch for {size} candidates and context {selection:?}"
                );
            }
        }
    }
}

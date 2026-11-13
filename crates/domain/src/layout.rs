//! Project-relative locations of the native index. Pure: callers inspect the
//! filesystem and pass the facts, so no I/O lives in the domain.

/// Directory owning the database, locks, watcher status and index configuration.
pub const INDEX_DIR: &str = ".agents/codegraph";
/// Pre-`.agents` location. Existing indexes keep working in place; never migrated.
pub const LEGACY_INDEX_DIR: &str = ".codegraph";
/// Scope configuration written by `init`, inside the index directory.
pub const CONFIG_FILE: &str = ".agents/codegraph/config/codegraph.json";
/// Directory holding [`CONFIG_FILE`].
pub const CONFIG_DIR: &str = ".agents/codegraph/config";
/// Pre-`.agents` location of the scope configuration, in the project root.
pub const LEGACY_CONFIG_FILE: &str = "codegraph.json";
/// Entry `init` adds to `.gitignore`. Only the generated directory: the rest of
/// `.agents` (skills, receipts) is meant to be versioned.
pub const GIT_IGNORE_ENTRY: &str = ".agents/codegraph/";

/// Index directory for a project. The legacy directory is kept only while it is
/// the sole one present, so existing indexes are never orphaned or moved.
pub fn index_dir(legacy_exists: bool, current_exists: bool) -> &'static str {
    if legacy_exists && !current_exists {
        LEGACY_INDEX_DIR
    } else {
        INDEX_DIR
    }
}

/// Scope configuration path, with the same legacy preference rule.
pub fn config_file(legacy_exists: bool, current_exists: bool) -> &'static str {
    if legacy_exists && !current_exists {
        LEGACY_CONFIG_FILE
    } else {
        CONFIG_FILE
    }
}

/// `.gitignore` content with [`GIT_IGNORE_ENTRY`] present exactly once. `None`
/// means the file already ignores it and must not be touched. Existing line
/// endings are preserved.
pub fn with_git_ignore_entry(existing: Option<&[u8]>) -> Option<Vec<u8>> {
    let text = existing.map(String::from_utf8_lossy).unwrap_or_default();
    let ignored = text.lines().any(|line| {
        matches!(
            line.trim(),
            ".agents/codegraph" | ".agents/codegraph/" | "/.agents/codegraph" | "/.agents/codegraph/"
        )
    });
    if ignored {
        return None;
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut bytes = existing.map(<[u8]>::to_vec).unwrap_or_default();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.extend_from_slice(newline.as_bytes());
    }
    bytes.extend_from_slice(GIT_IGNORE_ENTRY.as_bytes());
    bytes.extend_from_slice(newline.as_bytes());
    Some(bytes)
}

/// Inverse of [`with_git_ignore_entry`]: drops only the line `init` added, keeping
/// everything the user wrote since. `None` means the entry is not present.
pub fn without_git_ignore_entry(existing: &[u8]) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(existing);
    let mut found = false;
    let kept: String = text
        .split_inclusive('\n')
        .filter(|line| {
            let own = line.trim() == GIT_IGNORE_ENTRY;
            found |= own;
            !own
        })
        .collect();
    found.then(|| kept.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_entry_keeps_user_rules_added_around_it() {
        let added = with_git_ignore_entry(Some(b"target/\n")).unwrap();
        let mut edited = added.clone();
        edited.extend_from_slice(b"dist/\n");
        assert_eq!(without_git_ignore_entry(&edited).unwrap(), b"target/\ndist/\n");
    }

    #[test]
    fn removing_the_entry_round_trips_to_the_original() {
        for original in [&b"target/\n"[..], b"target/\r\n", b""] {
            let added = with_git_ignore_entry(Some(original)).unwrap();
            assert_eq!(without_git_ignore_entry(&added).unwrap(), original);
        }
    }

    #[test]
    fn removing_an_absent_entry_reports_nothing_to_do() {
        assert!(without_git_ignore_entry(b"target/\n").is_none());
    }

    #[test]
    fn new_projects_use_the_agents_directory() {
        assert_eq!(index_dir(false, false), INDEX_DIR);
        assert_eq!(config_file(false, false), CONFIG_FILE);
    }

    #[test]
    fn a_lone_legacy_index_stays_in_place() {
        assert_eq!(index_dir(true, false), LEGACY_INDEX_DIR);
        assert_eq!(config_file(true, false), LEGACY_CONFIG_FILE);
    }

    #[test]
    fn the_new_directory_wins_when_both_exist() {
        assert_eq!(index_dir(true, true), INDEX_DIR);
        assert_eq!(index_dir(false, true), INDEX_DIR);
    }

    #[test]
    fn config_lives_inside_the_index_directory() {
        assert!(CONFIG_FILE.starts_with(INDEX_DIR));
        assert_eq!(CONFIG_FILE, format!("{CONFIG_DIR}/codegraph.json"));
    }

    #[test]
    fn gitignore_is_created_when_absent_or_empty() {
        assert_eq!(with_git_ignore_entry(None).unwrap(), b".agents/codegraph/\n");
        assert_eq!(with_git_ignore_entry(Some(b"")).unwrap(), b".agents/codegraph/\n");
    }

    #[test]
    fn gitignore_entry_is_appended_after_existing_rules() {
        assert_eq!(
            with_git_ignore_entry(Some(b"target/\n")).unwrap(),
            b"target/\n.agents/codegraph/\n"
        );
    }

    #[test]
    fn gitignore_without_trailing_newline_gets_one_first() {
        assert_eq!(
            with_git_ignore_entry(Some(b"target/")).unwrap(),
            b"target/\n.agents/codegraph/\n"
        );
    }

    #[test]
    fn gitignore_preserves_crlf() {
        assert_eq!(
            with_git_ignore_entry(Some(b"target/\r\n")).unwrap(),
            b"target/\r\n.agents/codegraph/\r\n"
        );
    }

    #[test]
    fn gitignore_is_untouched_when_the_entry_exists_in_any_spelling() {
        for existing in [
            ".agents/codegraph/\n",
            ".agents/codegraph\n",
            "/.agents/codegraph/\n",
            "target/\n  .agents/codegraph/  \n",
        ] {
            assert!(with_git_ignore_entry(Some(existing.as_bytes())).is_none(), "{existing:?}");
        }
    }

    #[test]
    fn gitignore_never_ignores_the_whole_agents_directory() {
        let updated = with_git_ignore_entry(Some(b"target/\n")).unwrap();
        let text = String::from_utf8(updated).unwrap();
        assert!(!text.lines().any(|line| matches!(line.trim(), ".agents" | ".agents/")));
    }

    #[test]
    fn gitignore_for_a_sibling_directory_does_not_count_as_ignored() {
        assert!(with_git_ignore_entry(Some(b".agents/codegraph-old/\n")).is_some());
    }
}

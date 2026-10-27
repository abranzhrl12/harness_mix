use super::super::super::{ignore as ignore_policy, project::Failure};
use super::super::plan::excluded;
use std::path::Path;

pub(super) fn build(
    root: &Path,
    exclude_minified: bool,
    explicit_scope: bool,
) -> Result<ignore::WalkBuilder, Failure> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .follow_links(false)
        .hidden(true)
        .parents(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(move |entry| {
            !entry.file_name().to_str().is_none_or(|name| {
                excluded(name)
                    || (exclude_minified
                        && (["vendor", "dist", "build", "coverage"]
                            .contains(&name.to_ascii_lowercase().as_str())
                            || name.ends_with(".min.js")
                            || name.ends_with(".min.css")
                            || name.ends_with(".bundle.js")
                            || name.ends_with(".bundle.min.js")
                            || name.ends_with(".map")))
            })
        });

    // Overrides prevent project ignore rules from re-enabling default exclusions.
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    overrides
        .add("!node_modules/")
        .map_err(|_| ("INTERNAL", "Invalid default exclusion"))?;
    overrides
        .add("!target/")
        .map_err(|_| ("INTERNAL", "Invalid default exclusion"))?;
    builder.overrides(
        overrides
            .build()
            .map_err(|_| ("INTERNAL", "Invalid default exclusions"))?,
    );

    if !explicit_scope {
        builder.add_custom_ignore_filename(ignore_policy::FILE);
    }
    Ok(builder)
}

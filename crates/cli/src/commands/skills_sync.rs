use std::path::Path;

pub(super) use harness_skills::{Options, Report};

pub(super) fn run(root: &Path, options: Options) -> Result<Report, String> {
    harness_skills::run(root, options)
}

pub(super) fn sync_on_startup(root: &Path) -> Result<Option<Report>, String> {
    harness_skills::sync_on_startup(root)
}

use super::Pending;
use notify::{Event, EventKind};
use std::path::Path;

pub(super) fn relevant(root: &Path, event: Event, pending: &mut Pending) {
    if event.need_rescan() {
        pending.reconcile();
        return;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    if event.paths.is_empty() {
        pending.reconcile();
    }
    for path in event.paths {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Some(relative) = relative.to_str() else {
            pending.reconcile();
            continue;
        };
        let relative = relative.replace('\\', "/");
        if relative.eq_ignore_ascii_case(".agents/harness.config.json") {
            pending.reconcile();
            continue;
        }
        if relative.len() > 1000 {
            pending.reconcile();
            continue;
        }
        if relative
            .split('/')
            .any(crate::adapters::index_inventory::excluded)
            || relative == ".agents"
            || relative.starts_with(".agents/")
            || crate::adapters::index_inventory::restricted(&relative)
        {
            continue;
        }
        if relative.is_empty()
            || relative.ends_with(".gitignore")
            || relative.ends_with(".json")
            || matches!(
                event.kind,
                EventKind::Create(notify::event::CreateKind::Folder)
                    | EventKind::Remove(notify::event::RemoveKind::Folder)
            )
            // Windows directory rename notifications may have an unknown type.
            || path.extension().is_none()
        {
            pending.reconcile();
        } else if crate::adapters::index_extract::language(&relative).is_some()
            || path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
        {
            pending.add(relative);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::ModifyKind;

    #[test]
    fn sql_file_events_are_queued_for_incremental_refresh() {
        let root = Path::new("C:/workspace");
        let path = root.join("Database/Procedures.sql");
        let mut pending = Pending::default();
        relevant(
            root,
            Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )))
            .add_path(path),
            &mut pending,
        );
        assert!(pending.paths.contains("Database/Procedures.sql"));
        assert!(!pending.full);
    }
}

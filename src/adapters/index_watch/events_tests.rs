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

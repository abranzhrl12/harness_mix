    use super::*;

    #[test]
    fn exact_bytes_guard_digest_matches() {
        let cache = RustFactsCache::default();
        let facts = Arc::new(super::super::super::rust::Facts::default());
        cache.insert(b"source one", "same-digest".into(), Arc::clone(&facts));
        assert!(Arc::ptr_eq(
            &cache.get(b"source one", "same-digest").unwrap(),
            &facts
        ));
        assert!(cache.get(b"source two", "same-digest").is_none());
    }

    #[test]
    fn entry_limit_evicts_oldest_deterministically() {
        let cache = RustFactsCache::default();
        for index in 0..=CACHE_ENTRY_LIMIT {
            cache.insert(&[index as u8], index.to_string(), Arc::new(super::super::super::rust::Facts::default()));
        }
        let state = cache.0.lock().unwrap();
        assert_eq!(state.entries.len(), CACHE_ENTRY_LIMIT);
        assert!(!state.entries.contains_key(&(ANALYZER_VERSION.into(), "0".into())));
        assert!(state.entries.contains_key(&(ANALYZER_VERSION.into(), CACHE_ENTRY_LIMIT.to_string())));
    }

    #[test]
    fn byte_limit_evicts_oldest_and_skips_oversized_entries() {
        let cache = RustFactsCache::default();
        let source = vec![7; CACHE_LIMIT / 3];
        for index in 0..3 {
            cache.insert(&source, index.to_string(), Arc::new(super::super::super::rust::Facts::default()));
        }
        {
            let state = cache.0.lock().unwrap();
            assert!(state.reserved <= CACHE_LIMIT);
            assert!(!state.entries.contains_key(&(ANALYZER_VERSION.into(), "0".into())));
            assert!(state.entries.contains_key(&(ANALYZER_VERSION.into(), "2".into())));
        }

        let too_large = vec![0; CACHE_LIMIT];
        cache.insert(&too_large, "oversized".into(), Arc::new(super::super::super::rust::Facts::default()));
        assert!(!cache
            .0
            .lock()
            .unwrap()
            .entries
            .contains_key(&(ANALYZER_VERSION.into(), "oversized".into())));
    }

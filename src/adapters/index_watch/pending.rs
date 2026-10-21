use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

pub(super) const MAX_PENDING: usize = 1024;
pub(super) const QUIET: Duration = Duration::from_millis(600);
pub(super) const MAX_WAIT: Duration = Duration::from_secs(2);
pub(super) const RECONCILE: Duration = Duration::from_secs(300);

#[derive(Default)]
pub(super) struct Pending {
    pub(super) paths: BTreeSet<String>,
    pub(super) full: bool,
    pub(super) first: Option<Instant>,
    pub(super) last: Option<Instant>,
    pub(super) stop: bool,
    pub(super) revision: u64,
}

impl Pending {
    pub(super) fn dirty(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        let now = Instant::now();
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    pub(super) fn reconcile(&mut self) {
        self.full = true;
        self.paths.clear();
        self.dirty();
    }

    pub(super) fn add(&mut self, path: String) {
        if !self.full {
            if self.paths.len() >= MAX_PENDING && !self.paths.contains(&path) {
                self.reconcile();
            } else {
                self.paths.insert(path);
            }
        }
        self.dirty();
    }

    pub(super) fn ready_at(&self) -> Option<Instant> {
        Some((self.last? + QUIET).min(self.first? + MAX_WAIT))
    }
}

use harness_domain::Failure;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub const MAX_ACTIVE: usize = 4;
pub const MAX_SEARCHES: usize = 2;

#[derive(Default)]
pub(crate) struct Admission {
    active: AtomicUsize,
    searches: AtomicUsize,
}
pub(crate) struct Permit {
    admission: Arc<Admission>,
    search: bool,
}

impl Admission {
    pub fn acquire(self: &Arc<Self>, search: bool) -> Result<Permit, Failure> {
        increment_below(&self.active, MAX_ACTIVE).map_err(|_| {
            (
                "QUEUE_FULL",
                "máximo de cuatro operaciones activas; reintentar al liberar un cupo",
            )
        })?;
        if search && increment_below(&self.searches, MAX_SEARCHES).is_err() {
            self.active.fetch_sub(1, Ordering::AcqRel);
            return Err((
                "QUEUE_FULL",
                "máximo de dos búsquedas activas; reintentar al liberar un cupo",
            ));
        }
        Ok(Permit {
            admission: self.clone(),
            search,
        })
    }
}

// compare_exchange works on the declared Rust 1.88 minimum as well as newer Rust.
fn increment_below(counter: &AtomicUsize, max: usize) -> Result<(), ()> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= max {
            return Err(());
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Ok(()),
            Err(observed) => current = observed,
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if self.search {
            self.admission.searches.fetch_sub(1, Ordering::AcqRel);
        }
        self.admission.active.fetch_sub(1, Ordering::AcqRel);
    }
}

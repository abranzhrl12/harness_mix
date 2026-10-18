//! Process-local, bounded PostgreSQL sessions on one managed Tokio runtime.
//! A lease is reusable only after its caller confirms the implicit transaction's ReadyForQuery.
use harness_domain::{Failure, execution::OperationContext};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    runtime::{Runtime, RuntimeFlavor},
    sync::Notify,
    task::JoinHandle,
};
use tokio_postgres::{Client, Statement};

pub(super) const CAPACITY: usize = 4;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LIFETIME: Duration = Duration::from_secs(300);

pub(super) struct Connection {
    pub client: Client,
    pub worker: JoinHandle<Result<(), tokio_postgres::Error>>,
    pub connector: postgres_native_tls::MakeTlsConnector,
    pub statements: HashMap<&'static str, Statement>,
    created: Instant,
    last_used: Instant,
}
impl Connection {
    pub fn new(
        client: Client,
        worker: JoinHandle<Result<(), tokio_postgres::Error>>,
        connector: postgres_native_tls::MakeTlsConnector,
    ) -> Self {
        let now = Instant::now();
        Self {
            client,
            worker,
            connector,
            statements: HashMap::new(),
            created: now,
            last_used: now,
        }
    }
    fn usable(&self, now: Instant) -> bool {
        !self.client.is_closed()
            && !self.worker.is_finished()
            && now.duration_since(self.created) < MAX_LIFETIME
            && now.duration_since(self.last_used) < IDLE_TIMEOUT
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
#[derive(Default)]
struct State {
    leased: usize,
    idle: Vec<(String, Connection)>,
}
pub(super) struct Pool {
    state: Mutex<State>,
    changed: Notify,
}
impl Pool {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: Notify::new(),
        }
    }
    pub async fn checkout(
        self: &Arc<Self>,
        key: String,
        context: &OperationContext,
    ) -> Result<Lease, Failure> {
        loop {
            context.check()?;
            // Register before inspecting capacity so returns cannot be missed.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                state.idle.retain(|(_, entry)| entry.usable(now));
                if let Some(index) = state.idle.iter().position(|(id, _)| id == &key) {
                    let (_, connection) = state.idle.swap_remove(index);
                    state.leased += 1;
                    return Ok(Lease {
                        pool: self.clone(),
                        key,
                        connection: Some(connection),
                        reusable: false,
                        reused: true,
                    });
                }
                if state.leased < CAPACITY {
                    // Evict a different identity before opening, keeping total sessions <= 4.
                    if state.leased + state.idle.len() >= CAPACITY {
                        state.idle.pop();
                    }
                    state.leased += 1;
                    return Ok(Lease {
                        pool: self.clone(),
                        key,
                        connection: None,
                        reusable: false,
                        reused: false,
                    });
                }
            }
            tokio::select! {
                _ = &mut changed => {},
                _ = tokio::time::sleep(Duration::from_millis(25)) => {},
            }
        }
    }
}

pub(super) struct Lease {
    pool: Arc<Pool>,
    key: String,
    pub connection: Option<Connection>,
    pub reusable: bool,
    pub reused: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.pool.state.lock().unwrap_or_else(|e| e.into_inner());
        state.leased -= 1;
        if let Some(mut connection) = self.connection.take()
            && self.reusable
            && connection.usable(Instant::now())
        {
            connection.last_used = Instant::now();
            state.idle.push((self.key.clone(), connection));
        }
        drop(state);
        self.pool.changed.notify_one();
    }
}

struct Service {
    runtime: Runtime,
    pool: Arc<Pool>,
}
fn service() -> Result<&'static Service, Failure> {
    static SERVICE: OnceLock<Result<Service, Failure>> = OnceLock::new();
    SERVICE
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("harness-postgres")
                .enable_all()
                .build()
                .map_err(|_| ("PG_RUNTIME", "Unable to initialize PostgreSQL executor"))?;
            let pool = Arc::new(Pool::new());
            let reap = pool.clone();
            runtime.spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(15));
                loop {
                    tick.tick().await;
                    let now = Instant::now();
                    reap.state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .idle
                        .retain(|(_, entry)| entry.usable(now));
                }
            });
            Ok(Service { runtime, pool })
        })
        .as_ref()
        .map_err(|e| *e)
}
pub(super) async fn checkout(key: String, context: &OperationContext) -> Result<Lease, Failure> {
    service()?.pool.checkout(key, context).await
}
pub(super) fn run<F, T>(future: F) -> Result<T, Failure>
where
    F: Future<Output = Result<T, Failure>> + Send,
    T: Send,
{
    let runtime = &service()?.runtime;
    match tokio::runtime::Handle::try_current() {
        Err(_) => runtime.block_on(future),
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| runtime.block_on(future))
        }
        // Library callers may use a current-thread runtime; never nest block_on there.
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| runtime.block_on(future))
                .join()
                .unwrap_or(Err(("PG_RUNTIME", "PostgreSQL operation worker panicked")))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn capacity_waits_are_cancellable_and_dropped_leases_release_reservations() {
        let pool = Arc::new(Pool::new());
        let context = OperationContext::without_deadline();
        let mut held = Vec::new();
        for _ in 0..CAPACITY {
            held.push(pool.checkout("a".into(), &context).await.unwrap());
        }
        let expired = OperationContext::with_timeout(Duration::from_millis(10));
        assert_eq!(
            pool.checkout("a".into(), &expired).await.err().unwrap().0,
            "TIMEOUT"
        );
        let cancelled_context = OperationContext::without_deadline();
        let cancelled = {
            let pool = pool.clone();
            let context = cancelled_context.clone();
            tokio::spawn(async move { pool.checkout("a".into(), &context).await })
        };
        tokio::task::yield_now().await;
        cancelled_context.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), cancelled)
                .await
                .unwrap()
                .unwrap()
                .err()
                .unwrap()
                .0,
            "CANCELLED"
        );
        let waiting = {
            let pool = pool.clone();
            let context = context.clone();
            tokio::spawn(async move { pool.checkout("a".into(), &context).await.unwrap() })
        };
        tokio::task::yield_now().await;
        held.pop();
        let lease = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
        drop(lease);
        drop(held);
        assert_eq!(pool.state.lock().unwrap().leased, 0);
    }
    #[tokio::test]
    async fn synchronous_bridge_supports_a_current_thread_runtime() {
        assert_eq!(run(async { Ok::<_, Failure>(42) }).unwrap(), 42);
    }
}

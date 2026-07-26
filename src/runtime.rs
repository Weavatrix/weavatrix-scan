#![allow(clippy::missing_const_for_thread_local)]

use crate::pool::{Job, ThreadPool};
use std::cell::RefCell;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
#[cfg(feature = "rayon")]
use std::sync::{Mutex, mpsc};
use std::time::Duration;

thread_local! {
    static ACTIVE_RUNTIMES: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// A job accepted by an embeddable [`ParallelExecutor`].
pub type ParallelJob = Box<dyn FnOnce() + Send + 'static>;

/// Adapter contract for an application-owned thread pool.
///
/// Implementations must either accept `job` exactly once or return an error
/// without retaining it. `busy_timeout` lets bounded pools reject work instead
/// of indefinitely waiting for capacity.
pub trait ParallelExecutor: Send + Sync + 'static {
    /// Maximum useful concurrent jobs for this executor.
    fn parallelism(&self) -> usize;

    /// Attempts to schedule one job.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the pool is closed, saturated past
    /// `busy_timeout`, or otherwise cannot accept the job.
    fn try_execute(&self, job: ParallelJob, busy_timeout: Option<Duration>) -> io::Result<()>;
}

/// Ready-to-use adapter for an application-owned Rayon pool.
#[cfg(feature = "rayon")]
#[derive(Clone)]
pub struct RayonExecutor {
    pool: Arc<rayon::ThreadPool>,
}

#[cfg(feature = "rayon")]
impl RayonExecutor {
    /// Wraps an existing Rayon pool.
    #[must_use]
    pub fn new(pool: Arc<rayon::ThreadPool>) -> Self {
        Self { pool }
    }

    /// Returns the wrapped pool.
    #[must_use]
    pub fn pool(&self) -> &Arc<rayon::ThreadPool> {
        &self.pool
    }
}

#[cfg(feature = "rayon")]
impl ParallelExecutor for RayonExecutor {
    fn parallelism(&self) -> usize {
        self.pool.current_num_threads().max(1)
    }

    fn try_execute(&self, job: ParallelJob, busy_timeout: Option<Duration>) -> io::Result<()> {
        let Some(timeout) = busy_timeout else {
            self.pool.spawn(job);
            return Ok(());
        };
        let pending = Arc::new(Mutex::new(Some(job)));
        let worker_pending = Arc::clone(&pending);
        let (started_sender, started_receiver) = mpsc::sync_channel(0);
        self.pool.spawn(move || {
            let job = worker_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(job) = job {
                let _ = started_sender.send(());
                job();
            }
        });
        match started_receiver.recv_timeout(timeout) {
            Ok(()) => Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let cancelled = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .is_some();
                if cancelled {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Rayon pool did not start the job before busy timeout",
                    ))
                } else {
                    Ok(())
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Rayon pool dropped the scheduled job",
            )),
        }
    }
}

enum Executor {
    Global,
    Dedicated(Arc<ThreadPool>),
    External(Arc<dyn ParallelExecutor>),
}

struct RuntimeInner {
    id: u64,
    executor: Executor,
    busy_timeout: Option<Duration>,
}

/// Selects where parallel traversal jobs execute.
///
/// The default uses the process-wide Weavatrix pool. Dedicated pools are
/// joined on last drop. External executors receive the configured busy timeout
/// and may reject submission without leaving a traversal waiting for a worker.
#[derive(Clone)]
pub struct ParallelRuntime {
    inner: Arc<RuntimeInner>,
}

impl ParallelRuntime {
    /// Returns the process-wide default runtime.
    #[must_use]
    pub fn global() -> Self {
        static RUNTIME: OnceLock<ParallelRuntime> = OnceLock::new();
        RUNTIME
            .get_or_init(|| Self::new(Executor::Global, None))
            .clone()
    }

    /// Creates an owned pool with exactly `parallelism.max(1)` workers.
    ///
    /// # Errors
    ///
    /// Returns an operating-system thread creation error.
    pub fn dedicated(parallelism: usize) -> io::Result<Self> {
        let pool = ThreadPool::with_workers(parallelism.max(1))?;
        Ok(Self::new(Executor::Dedicated(Arc::new(pool)), None))
    }

    /// Uses an application-owned executor.
    #[must_use]
    pub fn external(executor: Arc<dyn ParallelExecutor>) -> Self {
        Self::new(Executor::External(executor), None)
    }

    /// Uses an existing application-owned Rayon pool.
    ///
    /// A one-second busy timeout is enabled by default to reject nested
    /// single-thread pool starvation. Override it with
    /// [`Self::with_busy_timeout`] when the application has a stronger
    /// scheduling guarantee.
    #[cfg(feature = "rayon")]
    #[must_use]
    pub fn rayon_existing(pool: Arc<rayon::ThreadPool>) -> Self {
        Self::external(Arc::new(RayonExecutor::new(pool)))
            .with_busy_timeout(Some(Duration::from_secs(1)))
    }

    /// Creates a dedicated Rayon pool with exactly `parallelism.max(1)`
    /// workers.
    ///
    /// # Errors
    ///
    /// Returns an error when Rayon cannot create the requested worker pool.
    #[cfg(feature = "rayon")]
    pub fn rayon_new(parallelism: usize) -> io::Result<Self> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(parallelism.max(1))
            .build()
            .map_err(io::Error::other)?;
        Ok(Self::rayon_existing(Arc::new(pool)))
    }

    /// Supplies the maximum wait an external executor may use to accept work.
    #[must_use]
    pub fn with_busy_timeout(mut self, busy_timeout: Option<Duration>) -> Self {
        Arc::make_mut(&mut self.inner).busy_timeout = busy_timeout;
        self
    }

    /// Maximum useful worker count advertised by this runtime.
    #[must_use]
    pub fn parallelism(&self) -> usize {
        match &self.inner.executor {
            Executor::Global => ThreadPool::global().workers(),
            Executor::Dedicated(pool) => pool.workers(),
            Executor::External(executor) => executor.parallelism().max(1),
        }
    }

    pub(crate) fn is_worker_thread(&self) -> bool {
        ACTIVE_RUNTIMES.with(|active| active.borrow().contains(&self.inner.id))
    }

    pub(crate) fn try_execute<F>(&self, job: F) -> io::Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        let id = self.inner.id;
        let wrapped: Job = Box::new(move || {
            let _guard = ActiveRuntimeGuard::enter(id);
            job();
        });
        match &self.inner.executor {
            Executor::Global => ThreadPool::global().execute(wrapped),
            Executor::Dedicated(pool) => pool.execute(wrapped),
            Executor::External(executor) => executor.try_execute(wrapped, self.inner.busy_timeout),
        }
    }

    fn new(executor: Executor, busy_timeout: Option<Duration>) -> Self {
        static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            inner: Arc::new(RuntimeInner {
                id: NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
                executor,
                busy_timeout,
            }),
        }
    }
}

impl Default for ParallelRuntime {
    fn default() -> Self {
        Self::global()
    }
}

impl fmt::Debug for ParallelRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.inner.executor {
            Executor::Global => "global",
            Executor::Dedicated(_) => "dedicated",
            Executor::External(_) => "external",
        };
        formatter
            .debug_struct("ParallelRuntime")
            .field("kind", &kind)
            .field("parallelism", &self.parallelism())
            .field("busy_timeout", &self.inner.busy_timeout)
            .finish()
    }
}

impl Clone for RuntimeInner {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            executor: match &self.executor {
                Executor::Global => Executor::Global,
                Executor::Dedicated(pool) => Executor::Dedicated(Arc::clone(pool)),
                Executor::External(executor) => Executor::External(Arc::clone(executor)),
            },
            busy_timeout: self.busy_timeout,
        }
    }
}

struct ActiveRuntimeGuard;

impl ActiveRuntimeGuard {
    fn enter(id: u64) -> Self {
        ACTIVE_RUNTIMES.with(|active| active.borrow_mut().push(id));
        Self
    }
}

impl Drop for ActiveRuntimeGuard {
    fn drop(&mut self) {
        ACTIVE_RUNTIMES.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{ParallelExecutor, ParallelJob, ParallelRuntime};
    use std::io;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    struct Inline;

    impl ParallelExecutor for Inline {
        fn parallelism(&self) -> usize {
            1
        }

        fn try_execute(&self, job: ParallelJob, _busy_timeout: Option<Duration>) -> io::Result<()> {
            job();
            Ok(())
        }
    }

    #[test]
    fn dedicated_runtime_executes_and_joins() {
        let runtime = ParallelRuntime::dedicated(2).unwrap();
        let (sender, receiver) = mpsc::channel();
        runtime
            .try_execute(move || sender.send(9).unwrap())
            .unwrap();
        assert_eq!(receiver.recv().unwrap(), 9);
    }

    #[test]
    fn external_runtime_marks_nested_execution() {
        let runtime = ParallelRuntime::external(Arc::new(Inline));
        let nested = runtime.clone();
        let (sender, receiver) = mpsc::channel();
        runtime
            .try_execute(move || sender.send(nested.is_worker_thread()).unwrap())
            .unwrap();
        assert!(receiver.recv().unwrap());
    }

    #[cfg(feature = "rayon")]
    #[test]
    fn rayon_runtime_uses_existing_pool() {
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .build()
                .unwrap(),
        );
        let runtime = ParallelRuntime::rayon_existing(Arc::clone(&pool));
        let (sender, receiver) = mpsc::channel();
        runtime
            .try_execute(move || sender.send(11).unwrap())
            .unwrap();
        assert_eq!(receiver.recv().unwrap(), 11);
        assert_eq!(runtime.parallelism(), 2);
    }

    #[cfg(feature = "rayon")]
    #[test]
    fn rayon_busy_timeout_cancels_unstarted_job() {
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap(),
        );
        let (block_sender, block_receiver) = mpsc::sync_channel(0);
        let (release_sender, release_receiver) = mpsc::sync_channel(0);
        pool.spawn(move || {
            block_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        });
        block_receiver.recv().unwrap();

        let runtime = ParallelRuntime::rayon_existing(pool)
            .with_busy_timeout(Some(Duration::from_millis(10)));
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_executed = Arc::clone(&executed);
        let error = runtime
            .try_execute(move || {
                worker_executed.store(true, std::sync::atomic::Ordering::SeqCst);
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        release_sender.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst));
    }
}

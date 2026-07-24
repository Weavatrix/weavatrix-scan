use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;

pub(crate) type Job = Box<dyn FnOnce() + Send + 'static>;

pub(crate) struct ThreadPool {
    queue: Arc<JobQueue>,
    workers: usize,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

struct JobQueue {
    state: Mutex<JobQueueState>,
    ready: Condvar,
}

struct JobQueueState {
    jobs: VecDeque<Job>,
    shutdown: bool,
}

impl ThreadPool {
    pub(crate) fn global() -> &'static Self {
        static POOL: OnceLock<ThreadPool> = OnceLock::new();
        POOL.get_or_init(|| {
            let workers =
                std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
            Self::with_workers(workers).expect("scan worker threads can be created")
        })
    }

    pub(crate) const fn workers(&self) -> usize {
        self.workers
    }

    pub(crate) fn execute(&self, job: Job) -> io::Result<()> {
        let mut state = self
            .queue
            .state
            .lock()
            .map_err(|_| io::Error::other("scan job queue is poisoned"))?;
        if state.shutdown {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "scan thread pool is shutting down",
            ));
        }
        state.jobs.push_back(job);
        drop(state);
        self.queue.ready.notify_one();
        Ok(())
    }

    pub(crate) fn with_workers(workers: usize) -> io::Result<Self> {
        let workers = workers.max(1);
        let queue = Arc::new(JobQueue {
            state: Mutex::new(JobQueueState {
                jobs: VecDeque::new(),
                shutdown: false,
            }),
            ready: Condvar::new(),
        });
        let mut handles = Vec::with_capacity(workers);
        for index in 0..workers {
            let worker_queue = Arc::clone(&queue);
            match std::thread::Builder::new()
                .name(format!("weavatrix-scan-{index}"))
                .spawn(move || worker_loop(&worker_queue))
            {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    queue
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .shutdown = true;
                    queue.ready.notify_all();
                    for handle in handles {
                        let _ = handle.join();
                    }
                    return Err(error);
                }
            }
        }
        Ok(Self {
            queue,
            workers,
            handles: Mutex::new(handles),
        })
    }
}

fn worker_loop(queue: &JobQueue) {
    loop {
        let job = {
            let mut state = queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while state.jobs.is_empty() && !state.shutdown {
                state = queue
                    .ready
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if state.shutdown && state.jobs.is_empty() {
                return;
            }
            state.jobs.pop_front().expect("job queue is not empty")
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        self.queue
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shutdown = true;
        self.queue.ready.notify_all();
        for handle in self
            .handles
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
        {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadPool;
    use std::sync::mpsc;

    #[test]
    fn global_pool_executes_jobs() {
        let (sender, receiver) = mpsc::channel();
        ThreadPool::global()
            .execute(Box::new(move || sender.send(42).unwrap()))
            .unwrap();
        assert_eq!(receiver.recv().unwrap(), 42);
        assert!(ThreadPool::global().workers() > 0);
    }

    #[test]
    fn dedicated_pool_stops_after_finishing_queued_jobs() {
        let pool = ThreadPool::with_workers(2).unwrap();
        let (sender, receiver) = mpsc::channel();
        pool.execute(Box::new(move || sender.send(7).unwrap()))
            .unwrap();
        assert_eq!(receiver.recv().unwrap(), 7);
        drop(pool);
    }
}

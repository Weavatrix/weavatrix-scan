use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send + 'static>;

pub(crate) struct ThreadPool {
    queue: Arc<JobQueue>,
    workers: usize,
}

struct JobQueue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
}

impl ThreadPool {
    pub(crate) fn global() -> &'static Self {
        static POOL: OnceLock<ThreadPool> = OnceLock::new();
        POOL.get_or_init(Self::new)
    }

    pub(crate) const fn workers(&self) -> usize {
        self.workers
    }

    pub(crate) fn execute(&self, job: impl FnOnce() + Send + 'static) {
        self.queue
            .jobs
            .lock()
            .expect("global scan job queue is not poisoned")
            .push_back(Box::new(job));
        self.queue.ready.notify_one();
    }

    fn new() -> Self {
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let queue = Arc::new(JobQueue {
            jobs: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        });
        for index in 0..workers {
            let queue = Arc::clone(&queue);
            std::thread::Builder::new()
                .name(format!("weavatrix-scan-{index}"))
                .spawn(move || worker_loop(&queue))
                .expect("scan worker thread can be created");
        }
        Self { queue, workers }
    }
}

fn worker_loop(queue: &JobQueue) {
    loop {
        let job = {
            let mut jobs = queue
                .jobs
                .lock()
                .expect("global scan job queue is not poisoned");
            while jobs.is_empty() {
                jobs = queue
                    .ready
                    .wait(jobs)
                    .expect("global scan job queue is not poisoned");
            }
            jobs.pop_front().expect("job queue is not empty")
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadPool;
    use std::sync::mpsc;

    #[test]
    fn global_pool_executes_jobs() {
        let (sender, receiver) = mpsc::channel();
        ThreadPool::global().execute(move || sender.send(42).unwrap());
        assert_eq!(receiver.recv().unwrap(), 42);
        assert!(ThreadPool::global().workers() > 0);
    }
}

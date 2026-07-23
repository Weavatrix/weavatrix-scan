use std::sync::{Mutex, OnceLock, mpsc};

type Job = Box<dyn FnOnce() + Send + 'static>;

pub(crate) struct ThreadPool {
    sender: mpsc::Sender<Job>,
    workers: usize,
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
        self.sender
            .send(Box::new(job))
            .expect("global scan thread pool is alive");
    }

    fn new() -> Self {
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = std::sync::Arc::new(Mutex::new(receiver));
        for index in 0..workers {
            let receiver = std::sync::Arc::clone(&receiver);
            std::thread::Builder::new()
                .name(format!("weavatrix-scan-{index}"))
                .spawn(move || worker_loop(&receiver))
                .expect("scan worker thread can be created");
        }
        Self { sender, workers }
    }
}

fn worker_loop(receiver: &Mutex<mpsc::Receiver<Job>>) {
    loop {
        let job = receiver
            .lock()
            .expect("scan job queue is not poisoned")
            .recv();
        let Ok(job) = job else {
            break;
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

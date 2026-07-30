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

    let runtime =
        ParallelRuntime::rayon_existing(pool).with_busy_timeout(Some(Duration::from_millis(10)));
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

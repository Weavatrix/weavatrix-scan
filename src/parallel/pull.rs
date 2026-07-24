use super::{ParallelWalker, WalkControl, WalkEvent};
use crate::control::CancellationToken;
use crate::walker::{WalkEntry, WalkError};
use std::io;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::JoinHandle;

/// Bounded pull iterator backed by parallel traversal workers.
///
/// Entry order is intentionally unspecified. Dropping the iterator
/// cooperatively cancels traversal and joins its coordinator thread.
pub struct ParallelWalkIter {
    receiver: Option<Receiver<Result<WalkEntry, WalkError>>>,
    cancellation: CancellationToken,
    coordinator: Option<JoinHandle<()>>,
}

impl ParallelWalker {
    /// Starts parallel traversal with a bounded pull buffer.
    ///
    /// A capacity of zero is normalized to one. Workers stop producing when
    /// the buffer is full until the consumer requests another item.
    #[must_use]
    pub fn into_iter_bounded(self, capacity: usize) -> ParallelWalkIter {
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = sync_channel(capacity.max(1));
        let coordinator = std::thread::spawn(move || {
            let event_sender = sender.clone();
            let result = self.visit_with_cancellation(&worker_cancellation, move |event| {
                let item = match event {
                    WalkEvent::Entry(entry) => Ok(entry.clone()),
                    WalkEvent::Error(error) => Err(copy_walk_error(error)),
                };
                if event_sender.send(item).is_ok() {
                    WalkControl::Continue
                } else {
                    WalkControl::Quit
                }
            });
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });
        ParallelWalkIter {
            receiver: Some(receiver),
            cancellation,
            coordinator: Some(coordinator),
        }
    }
}

impl Iterator for ParallelWalkIter {
    type Item = Result<WalkEntry, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.receiver.as_ref()?.recv().ok();
        if item.is_none() {
            self.receiver.take();
            self.join_coordinator();
        }
        item
    }
}

impl Drop for ParallelWalkIter {
    fn drop(&mut self) {
        self.receiver.take();
        self.cancellation.cancel();
        self.join_coordinator();
    }
}

impl ParallelWalkIter {
    fn join_coordinator(&mut self) {
        if let Some(coordinator) = self.coordinator.take() {
            coordinator
                .join()
                .expect("parallel pull coordinator panicked");
        }
    }
}

fn copy_walk_error(error: &WalkError) -> WalkError {
    WalkError::new(
        error.path().to_path_buf(),
        error.depth(),
        error.operation(),
        io::Error::new(error.io_error().kind(), error.io_error().to_string()),
    )
}

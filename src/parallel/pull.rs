use super::ParallelWalker;
use super::dynamic;
use crate::control::CancellationToken;
use crate::walker::{WalkEntry, WalkError};
use std::io;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

pub(super) type PullItem = Result<WalkEntry, WalkError>;
pub(super) type PullBatch = Vec<PullItem>;

/// Bounded pull iterator backed by parallel traversal workers.
///
/// Entry order is intentionally unspecified. Dropping the iterator
/// cooperatively cancels traversal and joins its coordinator thread.
pub struct ParallelWalkIter {
    receiver: Option<Receiver<PullBatch>>,
    current: std::vec::IntoIter<PullItem>,
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
        let capacity = capacity.max(1);
        let batch_size = capacity.min(64);
        let queued_batches = capacity.saturating_sub(batch_size) / batch_size;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = sync_channel(queued_batches);
        let coordinator = std::thread::spawn(move || {
            let emitted_error = Arc::new(AtomicBool::new(false));
            let options = self.options.normalized();
            let event_sender = sender.clone();
            let visitor_emitted_error = Arc::clone(&emitted_error);
            let result = dynamic::stream_batched(
                &self.root,
                options,
                self.parallelism,
                &worker_cancellation,
                move |entries, errors| {
                    if !errors.is_empty() {
                        visitor_emitted_error.store(true, Ordering::Relaxed);
                    }
                    send_batches(&event_sender, batch_size, entries, errors)
                },
            );
            if let Err(error) = result
                && !emitted_error.load(Ordering::Relaxed)
            {
                let _ = sender.send(vec![Err(error)]);
            }
        });
        ParallelWalkIter {
            receiver: Some(receiver),
            current: Vec::new().into_iter(),
            cancellation,
            coordinator: Some(coordinator),
        }
    }
}

impl Iterator for ParallelWalkIter {
    type Item = PullItem;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.current.next() {
                return Some(item);
            }
            if let Ok(batch) = self.receiver.as_ref()?.recv() {
                self.current = batch.into_iter();
            } else {
                self.receiver.take();
                self.join_coordinator();
                return None;
            }
        }
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
    pub(super) fn from_coordinator(
        receiver: Receiver<PullBatch>,
        cancellation: CancellationToken,
        coordinator: JoinHandle<()>,
    ) -> Self {
        Self {
            receiver: Some(receiver),
            current: Vec::new().into_iter(),
            cancellation,
            coordinator: Some(coordinator),
        }
    }

    fn join_coordinator(&mut self) {
        if let Some(coordinator) = self.coordinator.take() {
            coordinator
                .join()
                .expect("parallel pull coordinator panicked");
        }
    }
}

fn send_batches(
    sender: &SyncSender<PullBatch>,
    batch_size: usize,
    entries: Vec<WalkEntry>,
    errors: &[WalkError],
) -> bool {
    let mut batch = Vec::with_capacity(batch_size.min(entries.len() + errors.len()));
    for item in entries
        .into_iter()
        .map(Ok)
        .chain(errors.iter().map(|error| Err(copy_walk_error(error))))
    {
        batch.push(item);
        if batch.len() == batch_size && sender.send(std::mem::take(&mut batch)).is_err() {
            return false;
        }
    }
    batch.is_empty() || sender.send(batch).is_ok()
}

pub(super) fn copy_walk_error(error: &WalkError) -> WalkError {
    WalkError::new(
        error.path().to_path_buf(),
        error.depth(),
        error.operation(),
        io::Error::new(error.io_error().kind(), error.io_error().to_string()),
    )
}

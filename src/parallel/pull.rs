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
    ///
    /// # Panics
    ///
    /// Panics if the coordinator thread cannot be created. Use
    /// [`Self::try_into_iter_bounded`] for fallible startup.
    #[must_use]
    pub fn into_iter_bounded(self, capacity: usize) -> ParallelWalkIter {
        self.try_into_iter_bounded(capacity)
            .expect("parallel pull coordinator thread can be created")
    }

    /// Fallible form of [`Self::into_iter_bounded`].
    ///
    /// This reports an operating-system thread creation failure instead of
    /// panicking before traversal starts.
    ///
    /// # Errors
    ///
    /// Returns the coordinator thread spawn error.
    pub fn try_into_iter_bounded(self, capacity: usize) -> io::Result<ParallelWalkIter> {
        let capacity = capacity.max(1);
        let batch_size = capacity.min(64);
        let queued_batches = capacity.saturating_sub(batch_size) / batch_size;
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = sync_channel(queued_batches);
        let coordinator = std::thread::Builder::new()
            .name("weavatrix-scan-pull".to_owned())
            .spawn(move || {
                let emitted_error = Arc::new(AtomicBool::new(false));
                let options = self.options.normalized();
                let event_sender = sender.clone();
                let visitor_emitted_error = Arc::clone(&emitted_error);
                let result = dynamic::stream_batched(
                    &self.root,
                    options,
                    self.parallelism,
                    &self.runtime,
                    &worker_cancellation,
                    move |mut entries, errors| {
                        if !errors.is_empty() {
                            visitor_emitted_error.store(true, Ordering::Relaxed);
                        }
                        if self.skip_stdout.is_some() {
                            entries.retain(|entry| !super::matches_stdout(entry, self.skip_stdout));
                        }
                        send_batches(&event_sender, batch_size, entries, errors)
                    },
                );
                if let Err(error) = result
                    && !emitted_error.load(Ordering::Relaxed)
                {
                    let _ = sender.send(vec![Err(error)]);
                }
            })?;
        Ok(ParallelWalkIter {
            receiver: Some(receiver),
            current: Vec::new().into_iter(),
            cancellation,
            coordinator: Some(coordinator),
        })
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

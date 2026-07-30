use super::{
    Arc, DirectoryBatch, DirectoryFrame, DirectoryTask, OrderedStatefulScheduler, PreparedItem,
    WalkError, WalkOperation, WorkerResult, read_directory,
};

impl<R, E> OrderedStatefulScheduler<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    pub(super) fn refill(&mut self) {
        while !self.cancellation.is_cancelled() && self.outstanding < self.limit {
            let Some(task) = self.queued.pop_front() else {
                break;
            };
            let root = Arc::clone(&self.root);
            let result_sender = self.result_sender.clone();
            let cancellation = self.cancellation.clone();
            let root_file_system = self.root_file_system;
            let options = self.options;
            let processor = self.processor.clone();
            let scheduled = self.runtime.try_execute(move || {
                let id = task.id;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_directory(
                        &root,
                        root_file_system,
                        options,
                        &cancellation,
                        processor.as_ref(),
                        task,
                    )
                }));
                let _ = result_sender.send(WorkerResult { id, outcome });
            });
            match scheduled {
                Ok(()) => self.outstanding += 1,
                Err(source) => {
                    self.cancellation.cancel();
                    self.queued.clear();
                    self.schedule_error = Some(WalkError::new(
                        self.root.as_ref(),
                        0,
                        WalkOperation::ScheduleWorker,
                        source,
                    ));
                    break;
                }
            }
        }
    }

    pub(super) fn wait_for(&mut self, id: u64) -> Result<Option<DirectoryBatch<R, E>>, WalkError> {
        if let Some(batch) = self.ready.remove(&id) {
            return Ok(Some(batch));
        }
        if let Some(error) = self.schedule_error.take() {
            self.cancel_and_drain();
            return Err(error);
        }
        loop {
            let Ok(result) = self.result_receiver.recv() else {
                if let Some(error) = self.schedule_error.take() {
                    return Err(error);
                }
                return Ok(None);
            };
            self.outstanding = self.outstanding.saturating_sub(1);
            match result.outcome {
                Ok(batch) if result.id == id => {
                    self.refill();
                    if let Some(error) = self.schedule_error.take() {
                        self.cancel_and_drain();
                        return Err(error);
                    }
                    return Ok(Some(batch));
                }
                Ok(batch) => {
                    self.ready.insert(result.id, batch);
                    self.refill();
                }
                Err(payload) => {
                    self.cancel_and_drain();
                    std::panic::resume_unwind(payload);
                }
            }
            if self.cancellation.is_cancelled() {
                self.cancel_and_drain();
                if let Some(error) = self.schedule_error.take() {
                    return Err(error);
                }
                return Ok(None);
            }
        }
    }

    pub(super) fn prepare_frame(&mut self, batch: DirectoryBatch<R, E>) -> DirectoryFrame<E> {
        let mut items = Vec::with_capacity(batch.entries.len());
        let mut children = Vec::new();
        for item in batch.entries {
            let child = item.as_ref().ok().and_then(|entry| {
                entry.read_children.then(|| {
                    let id = self.next_id;
                    self.next_id = self.next_id.saturating_add(1);
                    let identity = entry.entry.directory_identity();
                    let ancestors = identity.map_or_else(
                        || Arc::clone(&batch.ancestors),
                        |identity| {
                            let mut child = batch.ancestors.as_ref().clone();
                            child.insert(identity);
                            Arc::new(child)
                        },
                    );
                    children.push(DirectoryTask {
                        id,
                        path: entry.path().to_path_buf(),
                        depth: entry.depth(),
                        identity,
                        ancestors,
                        read_state: batch.child_state.clone(),
                    });
                    id
                })
            });
            items.push(PreparedItem { item, child });
        }
        for child in children.into_iter().rev() {
            self.queued.push_front(child);
        }
        self.refill();
        DirectoryFrame {
            items: items.into_iter(),
        }
    }

    pub(super) fn cancel_and_drain(&mut self) {
        self.cancellation.cancel();
        self.queued.clear();
        while self.outstanding > 0 {
            if self.result_receiver.recv().is_err() {
                break;
            }
            self.outstanding -= 1;
        }
    }
}

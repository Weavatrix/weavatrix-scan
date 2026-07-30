use super::{
    Arc, DirectoryFrame, DirectoryTask, HashSet, StatefulWalkBuilder, StatefulWalkEntry,
    StatefulWalker, WalkError, Walker,
};

impl<R, E> StatefulWalker<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    pub(super) fn new(builder: StatefulWalkBuilder<R, E>) -> Result<Self, WalkError> {
        let options = builder.options.normalized();
        let mut root_options = options;
        root_options.min_depth = 0;
        let mut walker = Walker::with_options(&builder.root, root_options)?;
        let root_file_system = walker.root_file_system;
        let root = Arc::clone(&walker.root);
        let root_entry = walker.next().expect("a validated root yields one entry")?;
        let identity = root_entry.directory_identity();
        let mut ancestors = HashSet::new();
        if let Some(identity) = identity {
            ancestors.insert(identity);
        }
        let can_descend = root_entry.is_dir() && root_entry.skip_reason().is_none();
        let root_entry = (root_entry.depth() >= options.min_depth).then(|| StatefulWalkEntry {
            read_children: can_descend,
            entry: root_entry,
            state: E::default(),
        });
        let pending = can_descend.then(|| DirectoryTask {
            path: root.as_ref().clone(),
            depth: 0,
            identity,
            ancestors,
            read_state: builder.root_read_dir_state,
        });
        Ok(Self {
            root,
            root_file_system,
            options,
            processor: builder.processor,
            root_entry,
            pending,
            frames: Vec::new(),
        })
    }

    fn read_directory(&self, mut task: DirectoryTask<R>) -> DirectoryFrame<R, E> {
        let mut worker_options = self.options;
        worker_options.error_policy = crate::walk_types::ErrorPolicy::Continue;
        worker_options.min_depth = 0;
        worker_options.max_open = 1;
        worker_options.max_depth = Some(
            self.options
                .max_depth
                .unwrap_or(task.depth.saturating_add(1))
                .min(task.depth.saturating_add(1)),
        );
        let mut walker = Walker::from_known_directory_with_ancestry(
            &self.root,
            task.path.clone(),
            task.depth,
            worker_options,
            self.root_file_system,
            task.identity,
            task.ancestors.clone(),
        );
        let mut entries = Vec::new();
        while let Some(item) = walker.next() {
            match item {
                Ok(mut entry) => {
                    if entry.is_dir()
                        && entry.skip_reason() == Some(crate::WalkSkipReason::MaxDepth)
                        && self
                            .options
                            .max_depth
                            .is_none_or(|maximum| entry.depth() < maximum)
                    {
                        entry.clear_depth_skip();
                    }
                    if entry.is_dir() {
                        walker.skip_current_dir();
                    }
                    entries.push(Ok(StatefulWalkEntry {
                        read_children: entry.is_dir() && entry.skip_reason().is_none(),
                        entry,
                        state: E::default(),
                    }));
                }
                Err(error) => entries.push(Err(error)),
            }
        }
        if let Some(processor) = self.processor.as_ref() {
            processor(task.depth, &task.path, &mut task.read_state, &mut entries);
        }
        DirectoryFrame {
            entries: entries.into_iter(),
            child_state: task.read_state,
            ancestors: task.ancestors,
        }
    }
}

impl<R, E> Iterator for StatefulWalker<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    type Item = Result<StatefulWalkEntry<E>, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(root_entry) = self.root_entry.take() {
            return Some(Ok(root_entry));
        }
        loop {
            if let Some(task) = self.pending.take() {
                let frame = self.read_directory(task);
                self.frames.push(frame);
            }
            let frame = self.frames.last_mut()?;
            let Some(item) = frame.entries.next() else {
                self.frames.pop();
                continue;
            };
            if let Ok(entry) = &item
                && entry.read_children
            {
                let identity = entry.entry.directory_identity();
                let mut ancestors = frame.ancestors.clone();
                if let Some(identity) = identity {
                    ancestors.insert(identity);
                }
                self.pending = Some(DirectoryTask {
                    path: entry.path().to_path_buf(),
                    depth: entry.depth(),
                    identity,
                    ancestors,
                    read_state: frame.child_state.clone(),
                });
            }
            let visible = item
                .as_ref()
                .map_or(true, |entry| entry.depth() >= self.options.min_depth);
            if visible {
                return Some(item);
            }
        }
    }
}

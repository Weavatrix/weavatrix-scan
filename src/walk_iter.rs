use crate::walk_types::{WalkEntry, WalkError, WalkOperation};
use crate::walker::Walker;

impl Iterator for Walker {
    type Item = Result<WalkEntry, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        'walk: loop {
            if self.finished {
                return None;
            }
            if let Some(entry) = self.deferred_entry.take() {
                return Some(Ok(entry));
            }
            if self.yield_root {
                self.yield_root = false;
                let root = self.root.as_ref().clone();
                let entry = self.visit(
                    root,
                    0,
                    Some(self.root_file_type.expect("root metadata is present")),
                    None,
                    None,
                );
                match entry {
                    Ok(entry) => {
                        if self.prepare_entry(entry) {
                            continue;
                        }
                        if let Some(entry) = self.deferred_entry.take() {
                            return Some(Ok(entry));
                        }
                        continue;
                    }
                    Err(error) => return Some(self.yield_error(error)),
                }
            }
            if let Some(error) = self.schedule_pending_directory() {
                return Some(self.yield_error(error));
            }
            loop {
                let frame = self.frames.last_mut()?;
                let depth = frame.depth + 1;
                let entry = match frame.entries.next() {
                    Some(Ok(entry)) => entry,
                    Some(Err(source)) => {
                        let error =
                            WalkError::new(&frame.path, depth, WalkOperation::ReadEntry, source);
                        return Some(self.yield_error(error));
                    }
                    None => {
                        let was_open = frame.entries.is_open();
                        let identity = frame.identity;
                        let post_entry = self.frames.pop().expect("last frame exists").post_entry;
                        if was_open {
                            self.open_handles -= 1;
                        }
                        if let Some(identity) = identity {
                            self.active_directories.remove(&identity);
                        }
                        if let Some(entry) = post_entry {
                            return Some(Ok(entry));
                        }
                        continue;
                    }
                };
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(source) => {
                        let error =
                            WalkError::new(path, depth, WalkOperation::ReadMetadata, source);
                        return Some(self.yield_error(error));
                    }
                };
                let (bytes, version) = if self.options.collect_metadata && file_type.is_file() {
                    match entry.metadata() {
                        Ok(metadata) => (
                            Some(metadata.len()),
                            Some(crate::file_version::from_metadata(&metadata)),
                        ),
                        Err(source) => {
                            let error =
                                WalkError::new(path, depth, WalkOperation::ReadMetadata, source);
                            return Some(self.yield_error(error));
                        }
                    }
                } else {
                    (None, None)
                };
                match self.visit(path, depth, Some(file_type), bytes, version) {
                    Ok(entry) => {
                        if self.prepare_entry(entry) {
                            continue 'walk;
                        }
                        continue 'walk;
                    }
                    Err(error) => return Some(self.yield_error(error)),
                }
            }
        }
    }
}

impl Walker {
    fn prepare_entry(&mut self, entry: WalkEntry) -> bool {
        let accepted = self.filter.as_ref().is_none_or(|filter| filter(&entry));
        if !accepted {
            if entry.is_dir() {
                self.skip_current_dir();
                if entry.depth() == 0 {
                    self.finished = true;
                }
            }
            return false;
        }
        if self.contents_first && entry.is_dir() && entry.skip_reason().is_none() {
            if entry.depth() >= self.options.min_depth
                && let Some(pending) = self.pending_directory.as_mut()
            {
                pending.post_entry = Some(entry);
            }
            return false;
        }
        if entry.depth() >= self.options.min_depth {
            self.deferred_entry = Some(entry);
            true
        } else {
            false
        }
    }
}

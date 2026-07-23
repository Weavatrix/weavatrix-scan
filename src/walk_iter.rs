use crate::walk_types::{WalkEntry, WalkError, WalkOperation};
use crate::walker::Walker;

impl Iterator for Walker {
    type Item = Result<WalkEntry, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        'walk: loop {
            if self.finished {
                return None;
            }
            if self.yield_root {
                self.yield_root = false;
                let root = self.root.as_ref().clone();
                let entry = self.visit(
                    root,
                    0,
                    Some(self.root_file_type.expect("root metadata is present")),
                    None,
                );
                match entry {
                    Ok(entry) if entry.depth() >= self.options.min_depth => {
                        return Some(Ok(entry));
                    }
                    Ok(_) => continue,
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
                        self.frames.pop();
                        if was_open {
                            self.open_handles -= 1;
                        }
                        if let Some(identity) = identity {
                            self.active_directories.remove(&identity);
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
                let bytes = if self.options.collect_metadata && file_type.is_file() {
                    match entry.metadata() {
                        Ok(metadata) => Some(metadata.len()),
                        Err(source) => {
                            let error =
                                WalkError::new(path, depth, WalkOperation::ReadMetadata, source);
                            return Some(self.yield_error(error));
                        }
                    }
                } else {
                    None
                };
                match self.visit(path, depth, Some(file_type), bytes) {
                    Ok(entry) if entry.depth() >= self.options.min_depth => {
                        return Some(Ok(entry));
                    }
                    Ok(_) => continue 'walk,
                    Err(error) => return Some(self.yield_error(error)),
                }
            }
        }
    }
}

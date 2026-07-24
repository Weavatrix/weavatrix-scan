use super::ScanOptions;
use crate::walker::WalkOptions;

impl ScanOptions {
    pub(crate) fn worker_count(&self, file_count: usize) -> usize {
        if file_count == 0 {
            return 1;
        }
        let requested = self.requested_content_workers();
        requested.min(file_count.div_ceil(128)).max(1)
    }

    pub(crate) fn content_visit_worker_count(&self, file_count: usize) -> usize {
        let parallelism = self.content_parallelism.unwrap_or(self.parallelism);
        let requested = if parallelism == 0 {
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .min(32)
        } else {
            parallelism
        };
        requested.min(file_count).max(1)
    }

    fn requested_content_workers(&self) -> usize {
        let parallelism = self.content_parallelism.unwrap_or(self.parallelism);
        if parallelism == 0 {
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .min(if cfg!(windows) { 8 } else { 16 })
        } else {
            parallelism
        }
    }

    pub(crate) fn uses_parallel_traversal(&self) -> bool {
        self.limits.max_entries.is_none()
            && self.walk.max_open > 1
            && match self.traversal_parallelism.unwrap_or(self.parallelism) {
                0 => {
                    std::thread::available_parallelism().is_ok_and(|available| available.get() > 1)
                }
                1 => false,
                _ => true,
            }
    }

    pub(crate) const fn traversal_workers(&self) -> usize {
        match self.traversal_parallelism {
            Some(parallelism) => parallelism,
            None => self.parallelism,
        }
    }

    pub(crate) const fn walk_options(&self) -> WalkOptions {
        let mut options = self.walk;
        options.min_depth = 0;
        options
    }

    pub(crate) fn effective_min_depth(&self) -> usize {
        self.walk.max_depth.map_or(self.walk.min_depth, |maximum| {
            self.walk.min_depth.min(maximum)
        })
    }
}

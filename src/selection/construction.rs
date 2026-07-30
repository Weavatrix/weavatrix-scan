use super::{
    Error, Path, RepositoryMatcher, Result, ScanOptions, SelectionMatcher, directory_info, fs,
};

impl SelectionMatcher {
    /// Builds a matcher with [`ScanOptions::default`].
    ///
    /// # Errors
    ///
    /// Returns an error when the root or required ignore configuration cannot
    /// be read.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(root, &ScanOptions::default())
    }

    /// Builds a matcher from the same options used by [`crate::Scanner`].
    ///
    /// # Errors
    ///
    /// Returns an error when the root or required ignore configuration cannot
    /// be read.
    pub fn with_options(root: impl AsRef<Path>, options: &ScanOptions) -> Result<Self> {
        let repository = RepositoryMatcher::with_options(root, options)?;
        let root_file_system = if options.walk.same_file_system {
            let metadata = fs::metadata(repository.root())
                .map_err(|source| Error::io(repository.root(), source))?;
            Some(
                directory_info(repository.root(), &metadata)
                    .map_err(|source| Error::io(repository.root(), source))?
                    .file_system,
            )
        } else {
            None
        };
        Ok(Self {
            repository,
            options: options.clone(),
            root_file_system,
        })
    }
}

use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidRoot(PathBuf),
    ConcurrentModification(PathBuf),
    /// A paged read targeted a snapshot that the session has already replaced.
    StaleSnapshot {
        /// Generation supplied by the page cursor.
        expected: u64,
        /// Generation of the session's current snapshot.
        actual: u64,
    },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn concurrent_modification(path: impl Into<PathBuf>) -> Self {
        Self::ConcurrentModification(path.into())
    }

    /// Builds a stale-generation error for a paged session read.
    #[must_use]
    pub const fn stale_snapshot(expected: u64, actual: u64) -> Self {
        Self::StaleSnapshot { expected, actual }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidRoot(path) => write!(formatter, "not a directory: {}", path.display()),
            Self::ConcurrentModification(path) => {
                write!(formatter, "file changed while scanning: {}", path.display())
            }
            Self::StaleSnapshot { expected, actual } => write!(
                formatter,
                "scan snapshot generation {expected} is stale; current generation is {actual}"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidRoot(_) | Self::ConcurrentModification(_) | Self::StaleSnapshot { .. } => {
                None
            }
        }
    }
}

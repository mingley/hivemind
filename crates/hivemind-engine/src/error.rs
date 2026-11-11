use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("corruption in {} at byte {position}: {reason}", path.display())]
    Corruption {
        path: PathBuf,
        position: u64,
        reason: String,
    },
    #[error("store at {} is in use by another process; stop it or connect through the daemon", .0.display())]
    Locked(PathBuf),
    #[error("unsupported store: {0}")]
    Format(String),
    #[error("could not decode event at offset {offset}: {reason}")]
    Decode { offset: u64, reason: String },
    #[error("{0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("refusing to store a likely secret ({0}); remove it and retry")]
    Secret(&'static str),
    #[error("record of {0} bytes exceeds the log record limit; store large content as an artifact")]
    RecordTooLarge(usize),
    #[error("the log failed a write and rejects writes until the store is reopened")]
    Poisoned,
    #[error("timed out: {0}")]
    Timeout(String),
}

impl Error {
    pub(crate) fn corruption(
        path: impl Into<PathBuf>,
        position: u64,
        reason: impl Into<String>,
    ) -> Self {
        Error::Corruption {
            path: path.into(),
            position,
            reason: reason.into(),
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

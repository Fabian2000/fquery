use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum FQueryError {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("glob pattern error: {0}")]
    GlobPattern(#[from] glob::PatternError),

    #[error("glob error: {0}")]
    Glob(#[from] glob::GlobError),
}

pub type Result<T> = std::result::Result<T, FQueryError>;

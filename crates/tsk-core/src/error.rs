use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TskError {
    #[error("XDG_RUNTIME_DIR is not set")]
    NoRuntimeDir,
    #[error("Hyprland is not available")]
    HyprlandUnavailable,
    #[error("hyprctl failed: {0}")]
    Hyprctl(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("config error: {0}")]
    Config(String),
    #[error("revision {rev} not found in {path}")]
    UnknownRevision { rev: String, path: PathBuf },
    #[error("no git/jj checkout to fork from at {path}")]
    NoCheckoutToFork { path: PathBuf },
    #[error("jj workspace or task checkout {name} not found in {path}")]
    UnknownForkWorkspace { name: String, path: PathBuf },
    #[error("checkout {checkout} is not part of the source repo {source_root}")]
    ForkCheckoutNotInRepo {
        checkout: PathBuf,
        source_root: PathBuf,
    },
    #[error("fork options require a linked git worktree / jj workspace")]
    ForkRequiresLinkedCheckout,
    #[error("invalid checkout suffix '{suffix}'")]
    InvalidCheckoutSuffix { suffix: String },
    #[error("task {id} is a scratch workspace; sibling checkouts require a linked git/jj repo")]
    ScratchHasNoLinkedRepo { id: String },
    #[error("no current task (run from a task checkout or switch to a taskspace)")]
    NoCurrentTask,
    #[error("not a git or jj repo: {path}")]
    NotARepo { path: PathBuf },
    #[error("checkout {path} does not match source repo folder '{label}'")]
    OwnedCheckoutNameMismatch { path: PathBuf, label: String },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, TskError>;

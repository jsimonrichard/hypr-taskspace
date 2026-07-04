//! PATH checks for the installed `tsk` binary (no symlinks).

use std::path::PathBuf;

use crate::binary::{path_tsk_binary, path_tsk_is_usable};
use crate::config::TskConfig;

/// Deprecated — use [`path_tsk_is_usable`].
pub fn path_tsk_is_rust(cfg: &TskConfig) -> (bool, String) {
    path_tsk_is_usable(cfg)
}

pub fn install_path_symlink(_cfg: &TskConfig, _rust_bin: &std::path::Path) -> Result<PathBuf, crate::error::TskError> {
    Ok(path_tsk_binary().unwrap_or_else(|| PathBuf::from("tsk")))
}

pub fn install_profile_symlink(_profile: crate::install::profile::InstallProfile, _rust_bin: &std::path::Path) -> Result<PathBuf, crate::error::TskError> {
    Ok(path_tsk_binary().unwrap_or_else(|| PathBuf::from("tsk")))
}

pub fn remove_profile_symlink(_profile: crate::install::profile::InstallProfile, _rust_bin: &std::path::Path) -> Result<(), crate::error::TskError> {
    Ok(())
}

pub fn remove_path_symlink(_rust_bin: &std::path::Path) -> Result<(), crate::error::TskError> {
    Ok(())
}

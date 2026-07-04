//! Install profile — prod vs dev integration markers and paths.

use std::path::PathBuf;

use crate::config::TskConfig;
use crate::xdg::{expand, user_bin_dir};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallProfile {
    Prod,
    Dev,
}

impl InstallProfile {
    pub fn from_config(cfg: &TskConfig) -> Self {
        profile_for_config(cfg)
    }

    pub fn manage_marker(self) -> &'static str {
        match self {
            Self::Prod => "tsk-managed",
            Self::Dev => "tsk-dev-managed",
        }
    }

    pub fn include_omarchy_unbinds(self) -> bool {
        matches!(self, Self::Dev)
    }

    pub fn include_omarchy_unbinds_for(self, omarchy_integration: bool) -> bool {
        omarchy_integration || self.include_omarchy_unbinds()
    }

    pub fn install_systemd(self) -> bool {
        matches!(self, Self::Prod)
    }

    pub fn path_link_name(self) -> &'static str {
        match self {
            Self::Prod => "tsk",
            Self::Dev => "tsk-dev",
        }
    }

    pub fn path_link(self) -> PathBuf {
        user_bin_dir().join(self.path_link_name())
    }
}

pub fn dev_share_dir() -> PathBuf {
    expand("~/.local/share/tsk-dev")
}

pub fn dev_config_path() -> PathBuf {
    expand("~/.config/tsk-dev/config.toml")
}

pub fn is_dev_share_dir(share_dir: &PathBuf) -> bool {
    share_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "tsk-dev")
}

/// True when this config represents the dev install tree (not prod pacman).
///
/// Intentionally ignores `TSK_CONFIG` in the environment so prod restore paths
/// (e.g. during `scripts/dev.sh leave`) stay correct while the shell exports
/// `TSK_CONFIG=~/.config/tsk-dev/config.toml`.
pub fn is_dev_config(cfg: &TskConfig) -> bool {
    if crate::dev_session::dev_session_active() {
        return true;
    }
    if is_dev_share_dir(&cfg.install_hypr_share_dir) {
        return true;
    }
    if cfg.container_prefix == "tsk-dev" {
        return true;
    }
    if cfg.daemon_socket.contains("tsk-dev") {
        return true;
    }
    if cfg
        .data_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "tsk-dev")
    {
        return true;
    }
    false
}

/// Directory for install manifests and integration backups (separate from shared task state).
pub fn install_metadata_dir(cfg: &TskConfig, profile: InstallProfile) -> PathBuf {
    match profile {
        InstallProfile::Dev => dev_share_dir(),
        InstallProfile::Prod => cfg.data_dir.clone(),
    }
}

pub fn profile_for_config(cfg: &TskConfig) -> InstallProfile {
    if is_dev_config(cfg) {
        InstallProfile::Dev
    } else {
        InstallProfile::Prod
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TskConfig;

    #[test]
    fn is_dev_config_ignores_tsk_config_env_for_prod_cfg() {
        let mut cfg = TskConfig::default();
        cfg.container_prefix = "tsk".into();
        cfg.daemon_socket = "~/.local/share/tsk/daemon.sock".into();
        cfg.install_hypr_share_dir = crate::xdg::expand("~/.local/share/tsk");
        std::env::set_var("TSK_CONFIG", "/home/u/.config/tsk-dev/config.toml");
        assert!(!is_dev_config(&cfg));
        std::env::remove_var("TSK_CONFIG");
    }

    #[test]
    fn install_metadata_dir_splits_dev_and_prod() {
        let mut cfg = TskConfig::default();
        cfg.data_dir = crate::xdg::expand("~/.local/share/tsk");
        assert_eq!(
            install_metadata_dir(&cfg, InstallProfile::Dev),
            dev_share_dir()
        );
        assert_eq!(
            install_metadata_dir(&cfg, InstallProfile::Prod),
            cfg.data_dir
        );
    }
}

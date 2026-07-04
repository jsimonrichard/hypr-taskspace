//! System vs user-local share tree resolution (pacman vs cargo install).

use std::path::{Path, PathBuf};

use crate::install::profile::is_dev_config;
use crate::xdg::tsk_data_dir;

pub const SYSTEM_SHARE_DIR: &str = "/usr/share/tsk";

pub fn system_share_dir() -> PathBuf {
    PathBuf::from(SYSTEM_SHARE_DIR)
}

pub fn system_share_available() -> bool {
    system_share_dir()
        .join("hypr/bindings.conf")
        .is_file()
}

pub fn system_waybar_module_path() -> PathBuf {
    system_share_dir().join("lib/libtsk_waybar.so")
}

pub fn default_prod_share_dir() -> PathBuf {
    if system_share_available() {
        system_share_dir()
    } else {
        tsk_data_dir()
    }
}

pub fn is_system_share(path: &Path) -> bool {
    path.starts_with(SYSTEM_SHARE_DIR)
}

/// Prod install with the distro package present (even if config still says `~/.local/share/tsk`).
pub fn uses_packaged_share(cfg: &crate::config::TskConfig) -> bool {
    system_share_available() && !is_dev_config(cfg)
}

/// Share tree used for templates, helpers, and the Waybar `.so`.
pub fn effective_share_dir(cfg: &crate::config::TskConfig) -> PathBuf {
    if uses_packaged_share(cfg) {
        system_share_dir()
    } else {
        cfg.install_hypr_share_dir.clone()
    }
}

pub fn uses_system_share(cfg: &crate::config::TskConfig) -> bool {
    uses_packaged_share(cfg)
}

pub fn packaged_systemd_unit_path() -> PathBuf {
    PathBuf::from("/usr/lib/systemd/user/tskd.service")
}

pub fn packaged_systemd_unit_installed() -> bool {
    packaged_systemd_unit_path().is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_prod_config;

    #[test]
    fn uses_packaged_share_when_system_tree_present() {
        if !system_share_available() {
            return;
        }
        let cfg = load_prod_config().unwrap_or_default();
        assert!(
            !crate::install::profile::is_dev_config(&cfg),
            "prod fixture must not look like dev"
        );
        assert!(uses_packaged_share(&cfg));
        assert_eq!(
            effective_share_dir(&cfg),
            system_share_dir(),
            "prod Waybar/Hypr templates should come from /usr/share/tsk when packaged"
        );
    }
}

//! CLI version / identity output (`tsk --version`).

use std::path::PathBuf;

use crate::config::{load_config, TskConfig};
use crate::error::Result;
use crate::install::profile::is_dev_config;
use crate::xdg::tsk_config_path;

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub pkg_version: String,
    pub profile: &'static str,
    pub binary: PathBuf,
    pub config_path: PathBuf,
    pub share_dir: PathBuf,
    pub daemon_socket: String,
    pub tsk_env: Option<String>,
}

pub fn version_info(pkg_version: &str) -> Result<VersionInfo> {
    let cfg = load_config()?;
    Ok(build_version_info(pkg_version, &cfg))
}

pub fn build_version_info(pkg_version: &str, cfg: &TskConfig) -> VersionInfo {
    VersionInfo {
        pkg_version: pkg_version.to_string(),
        profile: if is_dev_config(cfg) { "dev" } else { "prod" },
        binary: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tsk")),
        config_path: tsk_config_path(),
        share_dir: cfg.install_hypr_share_dir.clone(),
        daemon_socket: cfg.daemon_socket.clone(),
        tsk_env: std::env::var("TSK").ok(),
    }
}

pub fn format_version_short(info: &VersionInfo) -> String {
    format!("tsk {} ({})", info.pkg_version, info.profile)
}

pub fn format_version_long(info: &VersionInfo) -> String {
    let mut lines = vec![
        format_version_short(info),
        format!("binary: {}", info.binary.display()),
        format!("config: {}", info.config_path.display()),
        format!("share:  {}", info.share_dir.display()),
        format!("daemon: {}", info.daemon_socket),
    ];
    match &info.tsk_env {
        Some(path) => {
            let note = if std::path::Path::new(path).is_file() {
                String::new()
            } else {
                " (missing — run: unset TSK)".into()
            };
            lines.push(format!("TSK:    {path}{note}"));
        }
        None => lines.push("TSK:    (unset)".into()),
    }
    if crate::dev_session::dev_session_active() {
        lines.push(format!(
            "session: active ({})",
            crate::dev_session::dev_session_marker_path().display()
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::profile::dev_share_dir;
    use crate::TskConfig;

    #[test]
    fn version_info_marks_dev_profile() {
        let mut cfg = TskConfig::default();
        cfg.install_hypr_share_dir = dev_share_dir();
        cfg.container_prefix = "tsk-dev".into();
        cfg.daemon_socket = "~/.local/share/tsk-dev/daemon.sock".into();
        let info = build_version_info("0.1.0-test", &cfg);
        assert_eq!(info.profile, "dev");
        assert!(format_version_long(&info).contains("(dev)"));
    }
}

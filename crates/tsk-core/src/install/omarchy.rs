//! Prod Omarchy preset — binaries + Hyprland + Waybar integration.

use std::path::PathBuf;

use crate::config::TskConfig;
use crate::error::Result;
use crate::install::bins::{install_bins, InstallBinsOptions};
use crate::install::hypr::{install_hypr, InstallHyprOptions};
use crate::install::profile::InstallProfile;
use crate::install::waybar::{install_waybar, InstallWaybarOptions};

#[derive(Debug, Clone)]
pub struct OmarchyInstallOptions {
    pub dry_run: bool,
    pub workspace_root: Option<PathBuf>,
}

pub fn install_omarchy_prod(cfg: &TskConfig, options: &OmarchyInstallOptions) -> Result<Vec<String>> {
    let profile = InstallProfile::Prod;
    let mut actions = install_bins(
        cfg,
        &InstallBinsOptions {
            dry_run: options.dry_run,
            workspace_root: options.workspace_root.clone(),
            profile: Some(profile),
            omarchy_integration: true,
            skip_waybar: false,
            bundled_waybar_source: None,
        },
    )?;

    let hypr = install_hypr(
        cfg,
        &InstallHyprOptions {
            dry_run: options.dry_run,
            workspace_root: options.workspace_root.clone(),
            profile: Some(profile),
            omarchy_integration: true,
            skip_bins_install: true,
        },
    )?;
    actions.extend(hypr);

    let waybar = install_waybar(
        cfg,
        &InstallWaybarOptions {
            dry_run: options.dry_run,
            workspace_root: options.workspace_root.clone(),
            skip_module_build: true,
        },
    )?;
    actions.extend(waybar);

    Ok(actions)
}

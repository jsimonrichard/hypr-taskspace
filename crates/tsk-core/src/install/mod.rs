//! Install helpers — binaries, Waybar integration, manifests, backups.

pub mod all;
pub mod backup;
pub mod bins;
pub mod chromium;
pub mod detect;
pub mod doctor;
pub mod hypr;
pub mod jsonc;
pub mod manifest;
pub mod omarchy;
pub mod path_link;
pub mod plugin;
pub mod profile;
pub mod reload;
pub mod systemd;
pub mod walker;
pub mod waybar;

pub use all::{install_all, install_detected, InstallAllOptions};
pub use bins::{install_bins, InstallBinsOptions};
pub use chromium::{
    install_chromium, install_chromium_status, run_native_host, InstallChromiumOptions,
};
pub use detect::{
    chromium_present, detected_integrations, omarchy_desktop_present, quattro_hypr_present,
};
pub use doctor::{format_doctor_report, run_doctor_checks, DoctorCheck};
pub use hypr::{
    hypr_user_config_path, install_hypr, install_hypr_status, strip_managed_lua_block,
    strip_managed_source_lines, uninstall_hypr, InstallHyprOptions,
};
pub use omarchy::{install_omarchy_prod, OmarchyInstallOptions};
pub use plugin::{install_omarchy_plugin, uninstall_omarchy_plugin, InstallPluginOptions};
pub use profile::{
    dev_config_path, dev_share_dir, install_metadata_dir, is_dev_config, is_dev_share_dir,
    profile_for_config, InstallProfile,
};
pub use systemd::{
    install_systemd, install_systemd_status, is_systemd_unit_installed, render_service_unit,
    systemctl_is_active, systemctl_is_enabled, systemd_restart, systemd_start, systemd_stop,
    uninstall_systemd, InstallSystemdOptions,
};
pub use walker::{install_walker, install_walker_status, uninstall_walker, InstallWalkerOptions};
pub use waybar::{install_waybar, install_waybar_status, uninstall_waybar, InstallWaybarOptions};

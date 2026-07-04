//! Install helpers — binaries, Waybar integration, manifests, backups.

pub mod backup;
pub mod bins;
pub mod doctor;
pub mod hypr;
pub mod jsonc;
pub mod manifest;
pub mod omarchy;
pub mod path_link;
pub mod profile;
pub mod reload;
pub mod systemd;
pub mod waybar;

pub use bins::{install_bins, InstallBinsOptions};
pub use hypr::{
    install_hypr, install_hypr_status, strip_managed_source_lines, uninstall_hypr,
    InstallHyprOptions,
};
pub use doctor::{run_doctor_checks, DoctorCheck};
pub use profile::{
    dev_config_path, dev_share_dir, install_metadata_dir, is_dev_config, is_dev_share_dir,
    profile_for_config, InstallProfile,
};
pub use systemd::{
    install_systemd, install_systemd_status, is_systemd_unit_installed, render_service_unit,
    systemd_restart, systemd_start, systemd_stop, systemctl_is_active, systemctl_is_enabled,
    uninstall_systemd, InstallSystemdOptions,
};
pub use omarchy::{install_omarchy_prod, OmarchyInstallOptions};
pub use waybar::{install_waybar, install_waybar_status, uninstall_waybar, InstallWaybarOptions};

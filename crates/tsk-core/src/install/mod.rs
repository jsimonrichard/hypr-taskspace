//! Install helpers — Waybar integration, manifests, backups.

pub mod backup;
pub mod doctor;
pub mod hypr;
pub mod jsonc;
pub mod manifest;
pub mod path_link;
pub mod reload;
pub mod systemd;
pub mod wrapper;
pub mod waybar;

pub use hypr::{install_hypr, install_hypr_status, uninstall_hypr, InstallHyprOptions};
pub use doctor::{run_doctor_checks, DoctorCheck};
pub use systemd::{
    install_systemd, install_systemd_status, is_systemd_unit_installed, render_service_unit,
    systemd_restart, systemd_start, systemd_stop, systemctl_is_active, systemctl_is_enabled,
    uninstall_systemd, InstallSystemdOptions,
};
pub use waybar::{install_waybar, install_waybar_status, uninstall_waybar, InstallWaybarOptions};

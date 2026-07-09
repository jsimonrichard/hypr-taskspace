//! Shared TSK library — models, config, registry, Hyprland, and Waybar export.

pub mod config;
pub mod context_sync;
pub mod daemon;
pub mod dev_session;
pub mod distrobox;
pub mod error;
pub mod host;
pub mod hypr_log;
pub mod hyprland;
pub mod hyprland_events;
pub mod install;
pub mod models;
pub mod registry;
pub mod repos;
pub mod service;
pub mod task_paths;
pub mod task_repo;
pub mod vcs;
pub mod share;
pub mod state_notify;
pub mod task_cleanup;
pub mod task_env;
pub mod task_ids;
pub mod task_on_start;
pub mod taskspaces;
pub mod terminal;
pub mod trace;
pub mod binary;
pub mod version;
pub mod waybar;
pub mod workspace_nav;
pub mod workspace_slots;
pub mod window_registry;
pub mod workspaces;
pub mod xdg;

pub use config::{ensure_config, load_config, load_config_at, load_dev_config, load_prod_config, TskConfig};
pub use dev_session::{
    dev_session_active, dev_session_binary, dev_session_marker_path, reconcile_stale_dev_session,
    start_dev_session, stop_dev_session,
};
pub use daemon::{
    daemon_pid_path, daemon_request, daemon_socket_path, ensure_daemon, is_daemon_running,
    ping_daemon, ping_daemon_at, stop_daemon, DaemonClient, DaemonServer,
};
pub use error::{TskError, Result};
pub use install::{
    install_bins, install_hypr, install_hypr_status, install_systemd, install_systemd_status,
    install_waybar, install_waybar_status, is_systemd_unit_installed, render_service_unit,
    run_doctor_checks, systemd_restart, systemd_start, systemd_stop, systemctl_is_active,
    systemctl_is_enabled, uninstall_hypr, uninstall_systemd, uninstall_waybar, InstallBinsOptions,
    DoctorCheck, InstallHyprOptions, InstallProfile, InstallSystemdOptions, InstallWaybarOptions,
    dev_config_path, dev_share_dir, install_metadata_dir, install_omarchy_prod, is_dev_config,
    profile_for_config,
    OmarchyInstallOptions,
};
pub use models::{ContextMode, SessionState, Task, TaskStatus};
pub use repos::{
    collect_task_repo_paths, ensure_repo_removable, find_repo, find_repo_by_path,
    is_scratch_task, is_task_scratch_repo, load_repo_config, load_repos, normalize_repo_path,
    paths_match, register_repo, repo_bookmarks_path, repo_config_path, repo_display_path,
    repo_id_from_path, save_repo_config, task_belongs_to_repo, task_source_repo_path,
    tasks_for_repo, unregister_repo, RegisteredRepo, RepoConfig, SCRATCH_TASK_LIST_LABEL,
};
pub use task_paths::{
    is_managed_task_checkout, is_scratch_workspace_path, linked_checkout_path,
    scratch_checkout_path, task_workspace_dir, SCRATCH_DIR_NAME,
};
pub use task_repo::{
    provision_task_checkout, ResolvedTaskRepo, TaskRepoOptions, TaskRepoSetup, TaskRepoSource,
};
pub use vcs::{current_branch, detect_vcs_root, repo_label, vcs_kind_at, VcsKind};
pub use registry::Registry;
pub use service::{MenuTask, TaskService};
pub use state_notify::{
    publish as publish_state_change, publish_with_workspace, read_state_rev, state_events_socket_path,
    StateChangeKind, StateEventListener,
};
pub use window_registry::{RestoreMove, RestoreReport, restore_windows, sync_window_registry};
pub use workspace_nav::{
    clear_navigation_memory, clear_runtime_slot_cache, focus_last_workspace, set_taskspace,
    workspace_go, workspace_goto_name, workspace_next, workspace_prev,
};
pub use share::{
    default_prod_share_dir, effective_share_dir, is_system_share, packaged_systemd_unit_installed,
    packaged_systemd_unit_path, system_share_available, system_share_dir, uses_packaged_share,
    uses_system_share, SYSTEM_SHARE_DIR,
};
pub use binary::{
    command_v_login, maybe_reexec_dev_session, path_tsk_binary, path_tsk_is_usable,
    peel_tsk_wrapper, resolve_tsk_binary, resolve_tsk_command, resolve_tsk_spawn_binary,
    waybar_module_path,
};
pub use xdg::normalize_desktop_env;
pub use terminal::{launch_host_terminal, launch_task_terminal, launch_task_tui};
pub use version::{
    build_version_info, format_version_long, format_version_short, version_info, VersionInfo,
};
pub use context_sync::sync_from_workspace_name;
pub use taskspaces::visible_default_workspace_count;
pub use trace::{
    analyze_recent_latency, clear_log, enable_for_process, enabled as trace_enabled, event as trace_event,
    format_report, tail_raw, trace_path,
};
pub use hypr_log::{clear_log as clear_hypr_log, hypr_log_path, tail_raw as tail_hypr_log};
pub use workspaces::{
    allowed_workspace_names, bar_active_workspace_name, bar_occupied_names, bar_workspace_names,
    is_global_workspace_name, primary_task_workspace, workspace_display_label,
};
pub use task_ids::{lookup_task, short_task_id, unique_id_prefix, workspace_tooltip_label, TaskLookup};
pub use waybar::{
    build_all_modules, build_all_modules_for_active_name, fetch_occupied_indices,
    fetch_occupied_names, workspace_label, workspace_module_key, WaybarModuleJson,
    WaybarModulesCache, ACTIVE_WORKSPACE_ICON, WAYBAR_MODULE_COUNT, WAYBAR_SIGNAL,
};
pub use hyprland_events::{
    diagnose_socket2, is_full_refresh_event, is_monitor_focus_event, is_workspace_focus_event,
    parse_focusedmon_v2, parse_workspace_v2, socket2_path, HyprlandEventListener,
};

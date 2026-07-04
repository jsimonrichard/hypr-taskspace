//! Shared TSK library — models, config, registry, Hyprland, and Waybar export.

pub mod config;
pub mod context_sync;
pub mod daemon;
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
pub mod state_notify;
pub mod task_cleanup;
pub mod taskspaces;
pub mod terminal;
pub mod trace;
pub mod binary;
pub mod waybar;
pub mod workspace_nav;
pub mod workspace_slots;
pub mod workspaces;
pub mod xdg;

pub use config::{ensure_config, load_config, TskConfig};
pub use daemon::{
    daemon_pid_path, daemon_request, daemon_socket_path, ensure_daemon, is_daemon_running,
    ping_daemon,
    stop_daemon, DaemonClient, DaemonServer,
};
pub use error::{TskError, Result};
pub use install::{
    install_hypr, install_hypr_status, install_waybar, install_waybar_status, run_doctor_checks,
    uninstall_hypr, uninstall_waybar, DoctorCheck, InstallHyprOptions, InstallWaybarOptions,
};
pub use models::{ContextMode, SessionState, Task, TaskStatus};
pub use repos::{
    collect_task_repo_paths, ensure_repo_removable, find_repo, find_repo_by_path,
    is_task_scratch_repo, load_repo_config, load_repos, normalize_repo_path, paths_match,
    register_repo, repo_bookmarks_path, repo_config_path, repo_display_path, repo_id_from_path,
    save_repo_config, task_belongs_to_repo, task_source_repo_path, tasks_for_repo,
    unregister_repo, RegisteredRepo, RepoConfig,
};
pub use task_paths::{
    is_managed_task_checkout, linked_checkout_path, scratch_checkout_path, task_workspace_dir,
    SCRATCH_DIR_NAME,
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
pub use workspace_nav::{
    clear_navigation_memory, clear_runtime_slot_cache, focus_last_workspace, set_taskspace,
    workspace_go, workspace_goto_name, workspace_next, workspace_prev,
};
pub use binary::resolve_tsk_binary;
pub use terminal::{launch_host_terminal, launch_task_terminal, launch_task_tui};
pub use context_sync::sync_from_workspace_name;
pub use taskspaces::visible_default_workspace_count;
pub use trace::{
    analyze_recent_latency, clear_log, enable_for_process, enabled as trace_enabled, event as trace_event,
    format_report, tail_raw, trace_path,
};
pub use hypr_log::{clear_log as clear_hypr_log, hypr_log_path, tail_raw as tail_hypr_log};
pub use workspaces::{
    allowed_workspace_names, bar_active_workspace_name, bar_occupied_names, bar_workspace_names,
    workspace_display_label,
};
pub use waybar::{
    build_all_modules, build_all_modules_for_active_name, fetch_occupied_indices,
    fetch_occupied_names, workspace_label, workspace_module_key, WaybarModuleJson,
    WaybarModulesCache, ACTIVE_WORKSPACE_ICON, WAYBAR_MODULE_COUNT, WAYBAR_SIGNAL,
};
pub use hyprland_events::{
    diagnose_socket2, is_full_refresh_event, is_monitor_focus_event, is_workspace_focus_event,
    parse_focusedmon_v2, parse_workspace_v2, socket2_path, HyprlandEventListener,
};

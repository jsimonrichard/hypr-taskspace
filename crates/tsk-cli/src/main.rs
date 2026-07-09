use clap::{Parser, Subcommand};

use tsk_core::{
    allowed_workspace_names, analyze_recent_latency, build_all_modules, clear_log, daemon_socket_path,
    format_version_long, install_bins, diagnose_socket2, enable_for_process, format_report, hypr_log_path,
    hyprland, install_hypr,
    install_hypr_status, install_omarchy_prod, install_systemd_status, install_waybar,
    install_waybar_status, is_dev_config, is_daemon_running, is_systemd_unit_installed, launch_task_tui,
    load_config, load_dev_config, maybe_reexec_dev_session, normalize_desktop_env, ping_daemon,
    profile_for_config,
    run_doctor_checks, stop_daemon,
    systemd_restart, systemd_start, systemd_stop, systemctl_is_active, tail_hypr_log, tail_raw,
    trace_path, uninstall_hypr, uninstall_waybar, version_info, workspace_module_key, DaemonClient,
    DaemonServer, InstallBinsOptions, InstallHyprOptions, InstallProfile,
    InstallWaybarOptions, OmarchyInstallOptions, TskError, Registry, Result, TaskService, TaskStatus,
    TaskRepoSource, detect_vcs_root, find_repo, find_repo_by_path, load_repos, register_repo, repo_label,
    ensure_repo_removable, unregister_repo, clear_hypr_log,
};

#[derive(Parser)]
#[command(name = "tsk", about = "Hypr Taskspace", disable_version_flag = true)]
struct Cli {
    /// Print version, binary path, and active config profile (dev or prod).
    #[arg(short = 'V', long, global = true)]
    version: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Status,
    Doctor,
    /// List or restore window placement
    Windows {
        #[arg(long, help = "Filter list by task id (with `list` or bare `windows`)")]
        task: Option<String>,
        #[command(subcommand)]
        command: Option<WindowsCommands>,
    },
    /// Omarchy Hyprland & Waybar integration (see docs/install.md)
    Install {
        #[command(subcommand)]
        command: ProdInstallCommands,
    },
    /// Development / e2e integration (separate install tree under ~/.local/share/tsk-dev)
    Dev {
        #[command(subcommand)]
        command: DevCommands,
    },
    /// Show integration status for the active config profile
    Integration {
        #[command(subcommand)]
        command: IntegrationCommands,
    },
    #[command(subcommand)]
    Taskspace(TaskspaceCommands),
    #[command(subcommand)]
    Workspace(WorkspaceCommands),
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    Waybar {
        #[command(subcommand)]
        command: WaybarCommands,
    },
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },
    #[command(subcommand)]
    Daemon(DaemonCommands),
    /// Clear session navigation memory (workspace layout per monitor)
    Reset {
        #[command(subcommand)]
        command: ResetCommands,
    },
}

#[derive(Subcommand)]
enum ResetCommands {
    /// Clear last-workspace and per-monitor layout memory in state.db
    Layout,
}

#[derive(Subcommand)]
enum WindowsCommands {
    /// List open windows and their task association
    List {
        #[arg(long)]
        task: Option<String>,
    },
    /// Move all windows back to their home workspaces
    Restore {
        #[arg(long, help = "Show planned moves without changing Hyprland")]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ProdInstallCommands {
    /// Omarchy Hyprland & Waybar integration preset (prod)
    Omarchy {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum DevCommands {
    Install {
        #[command(subcommand)]
        command: Option<DevInstallCommands>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
    },
    Uninstall {
        #[command(subcommand)]
        command: DevUninstallCommands,
    },
    Status,
}

#[derive(Subcommand)]
enum DevInstallCommands {
    /// Install share templates + Waybar module only (no Hypr/Waybar config edits)
    Share {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
        /// Install to prod paths (~/.local/share/tsk); used by scripts/install-user-share.sh
        #[arg(long, hide = true)]
        prod: bool,
    },
    /// Install binaries + Hyprland + Waybar integration (no systemd)
    All {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
    },
    Hypr {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
    },
    Waybar {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum DevUninstallCommands {
    All,
    Hypr {
        #[arg(long)]
        keep_files: bool,
    },
    Waybar,
}

#[derive(Subcommand)]
enum IntegrationCommands {
    Status,
}

#[derive(Subcommand)]
enum TaskspaceCommands {
    Default,
    Current,
}

#[derive(Subcommand)]
enum WorkspaceCommands {
    Go {
        #[arg(value_parser = clap::value_parser!(i32).range(1..=10))]
        index: i32,
    },
    /// Persist the active slot after a direct hyprctl switch (used by `workspace switch`).
    Remember {
        #[arg(value_parser = clap::value_parser!(i32).range(1..=10))]
        index: i32,
    },
    /// Hyprland switch then async state sync (keybind hot path).
    Switch {
        #[arg(value_parser = clap::value_parser!(i32).range(1..=10))]
        index: i32,
    },
    /// Adaptive Hyprland switch only (keybind hot path; use with `remember`).
    Dispatch {
        #[arg(value_parser = clap::value_parser!(i32).range(1..=10))]
        index: i32,
    },
    /// Move the active window to a taskspace-scoped workspace (keybind hot path).
    MoveDispatch {
        #[arg(value_parser = clap::value_parser!(i32).range(1..=10))]
        index: i32,
    },
    Next,
    Prev,
    Goto {
        name: String,
    },
}

#[derive(Subcommand)]
enum TaskCommands {
    New {
        name: String,
        #[arg(long, help = "Do not switch into the new task after creating it")]
        no_switch: bool,
        #[arg(long, help = "Use an empty scratch workspace under the task home")]
        scratch: bool,
        #[arg(long, help = "Use a specific checkout path instead of detecting git/jj from cwd")]
        repo_path: Option<std::path::PathBuf>,
        #[arg(
            long,
            help = "Use the main repo checkout directly instead of creating a git worktree / jj workspace"
        )]
        no_worktree: bool,
    },
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, help = "Include archived tasks")]
        archived: bool,
    },
    Switch {
        name_or_id: String,
    },
    Current,
    Archive {
        name_or_id: String,
    },
    Restore {
        name_or_id: String,
    },
    Delete {
        name_or_id: String,
    },
    /// Open the task manager TUI in a terminal window (alias for tui-launch)
    Menu,
    /// Interactive task manager (ratatui)
    Tui,
    /// Open the task manager TUI in a terminal window (used by SUPER+Tab)
    #[command(name = "tui-launch")]
    TuiLaunch,
    /// Open a terminal in the current context (task checkout or ~)
    Terminal {
        #[arg(value_name = "NAME_OR_ID")]
        name_or_id: Option<String>,
        #[arg(long, help = "Open a plain host terminal (not scoped to a task)")]
        host: bool,
    },
}

#[derive(Subcommand)]
enum RepoCommands {
    /// Register a checkout (writes `.tsk/repo.toml` inside the repo)
    Add {
        #[arg(value_name = "DIR")]
        dir: Option<std::path::PathBuf>,
    },
    /// List registered repos
    List,
    /// Remove a repo from tsk bookmarks (does not modify the checkout)
    Remove {
        id_or_path: String,
    },
    /// Show the git/jj repo root for a directory (default: cwd)
    Root {
        #[arg(value_name = "DIR")]
        dir: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum WaybarCommands {
    Status,
    Module {
        #[arg(value_parser = ["task", "workspace"])]
        name: String,
        #[arg(default_value_t = 1)]
        index: usize,
    },
}

#[derive(Subcommand)]
enum DebugCommands {
    /// Trace log utilities (set TSK_TRACE=1 on Waybar for widget events)
    Trace {
        #[command(subcommand)]
        command: DebugTraceCommands,
    },
    /// Diagnose Hyprland socket2 event socket (Waybar live updates)
    #[command(name = "hyprland-socket")]
    HyprlandSocket,
    /// Hyprctl command log (enabled by default; set TSK_HYPR_LOG=0 to disable)
    Hypr {
        #[command(subcommand)]
        command: DebugHyprCommands,
    },
}

#[derive(Subcommand)]
enum DebugHyprCommands {
    #[command(subcommand, name = "log")]
    Log(DebugHyprLogCommands),
}

#[derive(Subcommand)]
enum DebugHyprLogCommands {
    /// Print the last N hyprctl log lines (default 80)
    Show {
        #[arg(long, default_value_t = 80)]
        last: usize,
    },
    Clear,
    Path,
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start the daemon in the background
    Start,
    /// Run the daemon in the foreground (used by start)
    Run,
    /// Stop the running daemon
    Stop,
    /// Stop then start the daemon
    Restart,
    /// Check whether the daemon is reachable
    Status,
}

#[derive(Subcommand)]
enum DebugTraceCommands {
    /// Print the last N trace lines (default 40)
    Show {
        #[arg(long, default_value_t = 40)]
        last: usize,
    },
    /// Analyze the most recent workspace switch
    Analyze,
    Clear,
    Path,
    /// Switch workspace with tracing and print a latency timeline
    Workspace {
        #[arg(value_parser = clap::value_parser!(i32).range(1..=10))]
        index: i32,
        #[arg(long, help = "Clear the trace log before switching")]
        clear: bool,
        #[arg(long, default_value_t = 400)]
        wait_ms: u64,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    normalize_desktop_env();
    maybe_reexec_dev_session()?;
    let cli = Cli::parse();
    if cli.version {
        return cmd_version();
    }
    let Some(command) = cli.command else {
        return Err(TskError::Other(
            "subcommand required — try `tsk --help`".into(),
        ));
    };
    match command {
        Commands::Status => cmd_status(),
        Commands::Doctor => cmd_doctor(),
        Commands::Windows { task, command } => match command {
            None => cmd_windows_list(task.as_deref()),
            Some(WindowsCommands::List { task: list_task }) => {
                cmd_windows_list(list_task.as_deref().or(task.as_deref()))
            }
            Some(WindowsCommands::Restore { dry_run }) => cmd_windows_restore(dry_run),
        },
        Commands::Install { command } => match command {
            ProdInstallCommands::Omarchy { dry_run, workspace } => {
                cmd_install_omarchy(dry_run, workspace)
            }
        },
        Commands::Dev { command } => match command {
            DevCommands::Install {
                command,
                dry_run,
                workspace,
            } => match command {
                Some(DevInstallCommands::Share {
                    dry_run: share_dry,
                    workspace: share_ws,
                    prod,
                }) => cmd_install_share(
                    share_dry || dry_run,
                    share_ws.or(workspace),
                    if prod {
                        InstallProfile::Prod
                    } else {
                        InstallProfile::Dev
                    },
                ),
                Some(DevInstallCommands::All { dry_run, workspace }) => {
                    cmd_dev_install_all(dry_run, workspace)
                }
                Some(DevInstallCommands::Hypr { dry_run, workspace }) => {
                    cmd_dev_install_hypr(dry_run, workspace)
                }
                Some(DevInstallCommands::Waybar { dry_run, workspace }) => {
                    cmd_dev_install_waybar(dry_run, workspace)
                }
                None => Err(TskError::Other(
                    "dev install requires a subcommand (share, all, hypr, waybar)".into(),
                )),
            },
            DevCommands::Uninstall { command } => match command {
                DevUninstallCommands::All => cmd_dev_uninstall_all(),
                DevUninstallCommands::Hypr { keep_files } => cmd_dev_uninstall_hypr(keep_files),
                DevUninstallCommands::Waybar => cmd_dev_uninstall_waybar(),
            },
            DevCommands::Status => cmd_integration_status(load_dev_config()?),
        },
        Commands::Integration { command } => match command {
            IntegrationCommands::Status => cmd_integration_status(load_config()?),
        },
        Commands::Taskspace(command) => match command {
            TaskspaceCommands::Default => cmd_taskspace_default(),
            TaskspaceCommands::Current => cmd_taskspace_current(),
        },
        Commands::Workspace(command) => match command {
            WorkspaceCommands::Go { index } => cmd_workspace_go(index),
            WorkspaceCommands::Remember { index } => cmd_workspace_remember(index),
            WorkspaceCommands::Switch { index } => cmd_workspace_switch(index),
            WorkspaceCommands::Dispatch { index } => cmd_workspace_dispatch(index),
            WorkspaceCommands::MoveDispatch { index } => cmd_workspace_move_dispatch(index),
            WorkspaceCommands::Next => cmd_workspace_next(),
            WorkspaceCommands::Prev => cmd_workspace_prev(),
            WorkspaceCommands::Goto { name } => cmd_workspace_goto(&name),
        },
        Commands::Task { command } => match command {
            TaskCommands::New {
                name,
                no_switch,
                scratch,
                repo_path,
                no_worktree,
            } => cmd_task_new(&name, !no_switch, scratch, repo_path.as_deref(), no_worktree),
            TaskCommands::List { json, archived } => cmd_task_list(json, archived),
            TaskCommands::Switch { name_or_id } => cmd_task_switch(&name_or_id),
            TaskCommands::Current => cmd_task_current(),
            TaskCommands::Archive { name_or_id } => cmd_task_archive(&name_or_id),
            TaskCommands::Restore { name_or_id } => cmd_task_restore(&name_or_id),
            TaskCommands::Delete { name_or_id } => cmd_task_delete(&name_or_id),
            TaskCommands::Menu | TaskCommands::TuiLaunch => cmd_task_tui_launch(),
            TaskCommands::Tui => cmd_task_tui(),
            TaskCommands::Terminal { name_or_id, host } => {
                cmd_task_terminal(name_or_id.as_deref(), host)
            }
        },
        Commands::Repo { command } => match command {
            RepoCommands::Add { dir } => cmd_repo_add(dir.as_deref()),
            RepoCommands::List => cmd_repo_list(),
            RepoCommands::Remove { id_or_path } => cmd_repo_remove(&id_or_path),
            RepoCommands::Root { dir } => cmd_repo_root(dir.as_deref()),
        },
        Commands::Waybar { command } => match command {
            WaybarCommands::Status => cmd_waybar_install_status(),
            WaybarCommands::Module { name, index } => cmd_waybar_module(&name, index),
        },
        Commands::Debug { command } => match command {
            DebugCommands::Trace { command } => match command {
                DebugTraceCommands::Show { last } => cmd_debug_trace_show(last),
                DebugTraceCommands::Analyze => cmd_debug_trace_analyze(),
                DebugTraceCommands::Clear => cmd_debug_trace_clear(),
                DebugTraceCommands::Path => cmd_debug_trace_path(),
                DebugTraceCommands::Workspace {
                    index,
                    clear,
                    wait_ms,
                } => cmd_debug_trace_workspace(index, clear, wait_ms),
            },
            DebugCommands::HyprlandSocket => cmd_debug_hyprland_socket(),
            DebugCommands::Hypr { command } => match command {
                DebugHyprCommands::Log(log_cmd) => match log_cmd {
                    DebugHyprLogCommands::Show { last } => cmd_debug_hypr_log_show(last),
                    DebugHyprLogCommands::Clear => cmd_debug_hypr_log_clear(),
                    DebugHyprLogCommands::Path => cmd_debug_hypr_log_path(),
                },
            },
        },
        Commands::Daemon(command) => match command {
            DaemonCommands::Start => cmd_daemon_start(),
            DaemonCommands::Run => cmd_daemon_run(),
            DaemonCommands::Stop => cmd_daemon_stop(),
            DaemonCommands::Restart => cmd_daemon_restart(),
            DaemonCommands::Status => cmd_daemon_status(),
        },
        Commands::Reset { command } => match command {
            ResetCommands::Layout => cmd_reset_layout(),
        },
    }
}

fn client() -> Result<DaemonClient> {
    DaemonClient::with_defaults()
}

fn cmd_version() -> Result<()> {
    let info = version_info(env!("CARGO_PKG_VERSION"))?;
    println!("{}", format_version_long(&info));
    Ok(())
}

fn cmd_status() -> Result<()> {
    let svc = client()?;
    let state = svc.load_state()?;
    let allowed = allowed_workspace_names(&state);
    let taskspace_label = state.taskspace_label();

    if !allowed.is_empty() {
        println!("Taskspace: {taskspace_label}");
        println!("Workspaces: {}", allowed.join(", "));
    } else {
        println!("Taskspace: {taskspace_label}");
    }

    if let Some(id) = &state.current_task_id {
        if let Some(task) = state.tasks.get(id) {
            println!("Task: {} ({id})", task.name);
            println!("Repo: {}", task.repo_path.display());
        }
    } else {
        println!("Task: (none — default taskspace)");
    }

    if let Some(ws) = hyprland::get_active_workspace().ok().flatten() {
        if !ws.name.is_empty() {
            println!("Active workspace: {}", ws.name);
        }
    }

    if let Ok(clients) = hyprland::get_clients() {
        if !clients.is_empty() {
            println!("Windows:");
            for w in clients {
                let task_id = state
                    .windows
                    .get(&w.address)
                    .and_then(|r| r.task_id.as_deref())
                    .unwrap_or("default");
                let ws_name = if w.workspace_name.is_empty() {
                    w.workspace.to_string()
                } else {
                    w.workspace_name.clone()
                };
                println!("  [{task_id}] {} → {ws_name}", w.title);
            }
        }
    }

    let others: Vec<_> = state
        .tasks
        .values()
        .filter(|t| t.status != TaskStatus::Archived)
        .filter(|t| state.current_task_id.as_deref() != Some(t.id.as_str()))
        .collect();
    if !others.is_empty() {
        println!("Other tasks:");
        for t in others {
            println!(
                "  {} ({}, {}-1..{}-{})",
                t.id,
                t.status.as_str(),
                t.id,
                t.id,
                t.workspace_count
            );
        }
    }

    Ok(())
}

fn cmd_doctor() -> Result<()> {
    let cfg = load_config()?;
    let checks = run_doctor_checks(&cfg)?;
    let mut ok = true;
    for check in checks {
        let mark = if check.passed { "ok" } else { "FAIL" };
        println!("[{mark}] {}: {}", check.label, check.detail);
        if !check.passed {
            ok = false;
        }
    }
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_windows_list(task_filter: Option<&str>) -> Result<()> {
    let state = client()?.load_state()?;
    for w in hyprland::get_clients()? {
        let task_id = state
            .windows
            .get(&w.address)
            .and_then(|r| r.task_id.as_deref())
            .unwrap_or("default");
        if task_filter.is_some_and(|t| t != task_id) {
            continue;
        }
        let ws_name = if w.workspace_name.is_empty() {
            w.workspace.to_string()
        } else {
            w.workspace_name.clone()
        };
        let home = state
            .windows
            .get(&w.address)
            .map(|r| r.home_workspace_name.as_str())
            .filter(|h| !h.is_empty())
            .unwrap_or("-");
        println!(
            "{} [{task_id}] {} ws={ws_name} home={home}",
            w.address,
            w.title,
        );
    }
    Ok(())
}

fn cmd_windows_restore(dry_run: bool) -> Result<()> {
    let report = TaskService::with_defaults()?.restore_windows(dry_run)?;
    if report.moves.is_empty() {
        println!(
            "No misplaced windows ({} synced, {} already home).",
            report.synced, report.already_home
        );
        return Ok(());
    }
    if dry_run {
        println!("Would restore {} window(s):", report.moves.len());
    } else {
        println!("Restored {} window(s):", report.restored);
    }
    for mv in &report.moves {
        println!("  {} \"{}\"  {} → {}", mv.address, mv.title, mv.from, mv.to);
    }
    Ok(())
}

fn bundled_waybar_cdylib() -> Option<std::path::PathBuf> {
    option_env!("TSK_WAYBAR_CDYLIB_SOURCE").map(std::path::PathBuf::from)
}

fn cmd_install_share(
    dry_run: bool,
    workspace: Option<std::path::PathBuf>,
    profile: InstallProfile,
) -> Result<()> {
    cmd_install_bins(dry_run, workspace, profile)
}

fn cmd_install_bins(
    dry_run: bool,
    workspace: Option<std::path::PathBuf>,
    profile: InstallProfile,
) -> Result<()> {
    let cfg = match profile {
        InstallProfile::Prod => load_config()?,
        InstallProfile::Dev => load_dev_config()?,
    };
    let actions = install_bins(
        &cfg,
        &InstallBinsOptions {
            dry_run,
            workspace_root: workspace,
            profile: Some(profile),
            omarchy_integration: false,
            skip_waybar: false,
            bundled_waybar_source: bundled_waybar_cdylib(),
        },
    )?;
    if dry_run {
        for line in actions {
            println!("{line}");
        }
    } else {
        for line in actions {
            println!("{line}");
        }
    }
    Ok(())
}

fn cmd_install_omarchy(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    let cfg = load_config()?;
    let actions = install_omarchy_prod(
        &cfg,
        &OmarchyInstallOptions {
            dry_run,
            workspace_root: workspace,
        },
    )?;
    if dry_run {
        for line in actions {
            println!("{line}");
        }
        return Ok(());
    }
    if !actions.is_empty() {
        println!("Applied: {}.", actions.join(", "));
    }
    println!();
    println!("Omarchy prod integration installed to {}.", cfg.install_hypr_share_dir.display());
    println!("  Hyprland: source line + Omarchy unbinds");
    println!("  Waybar: cffi/tsk module, styles, restart");
    println!("Next: scripts/install-systemd.sh  (or systemctl --user enable --now tskd.service when using the pacman package)");
    Ok(())
}

fn cmd_dev_install_all(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    cmd_dev_install_hypr(dry_run, workspace.clone())?;
    if !dry_run {
        println!();
    }
    cmd_dev_install_waybar(dry_run, workspace)
}

fn cmd_dev_install_hypr(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    let cfg = load_dev_config()?;
    let options = InstallHyprOptions {
        dry_run,
        workspace_root: workspace,
        profile: Some(InstallProfile::Dev),
        omarchy_integration: true,
        skip_bins_install: false,
    };
    let actions = install_hypr(&cfg, &options)?;
    if dry_run {
        for line in actions {
            println!("{line}");
        }
    } else {
        println!("Installed dev Hyprland integration.");
        if !actions.is_empty() {
            println!("Applied: {}.", actions.join(", "));
        }
        println!(
            "CLI: use `tsk` on PATH ({})",
            tsk_core::path_tsk_is_usable(&cfg).1,
        );
        println!("Start the dev daemon manually: scripts/dev.sh daemon");
        TaskService::with_config(cfg)?.reset_navigation_layout()?;
        println!("Reset workspace layout memory (per-monitor mappings cleared).");
    }
    Ok(())
}

fn cmd_dev_install_waybar(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    let cfg = load_dev_config()?;
    let options = InstallWaybarOptions {
        dry_run,
        workspace_root: workspace,
        skip_module_build: false,
    };
    let actions = install_waybar(&cfg, &options)?;
    if dry_run {
        for line in actions {
            println!("{line}");
        }
    } else {
        println!("Installed dev Waybar integration.");
        if !actions.is_empty() {
            println!("Applied: {}.", actions.join(", "));
        }
    }
    Ok(())
}

fn cmd_dev_uninstall_all() -> Result<()> {
    cmd_dev_uninstall_waybar()?;
    cmd_dev_uninstall_hypr(false)
}

fn cmd_dev_uninstall_hypr(keep_files: bool) -> Result<()> {
    let cfg = load_dev_config()?;
    let actions = uninstall_hypr(&cfg, keep_files)?;
    println!("Uninstalled dev Hyprland integration.");
    if !actions.is_empty() {
        println!("Applied: {}.", actions.join(", "));
    }
    Ok(())
}

fn cmd_dev_uninstall_waybar() -> Result<()> {
    let cfg = load_dev_config()?;
    let actions = uninstall_waybar(&cfg)?;
    println!("Uninstalled dev Waybar integration.");
    if !actions.is_empty() {
        println!("Applied: {}.", actions.join(", "));
    }
    Ok(())
}

fn cmd_reset_layout() -> Result<()> {
    TaskService::with_defaults()?.reset_navigation_layout()?;
    println!("Cleared last-workspace and per-monitor layout memory.");
    Ok(())
}

fn cmd_integration_status(cfg: tsk_core::TskConfig) -> Result<()> {
    let profile = profile_for_config(&cfg);
    let session_active = tsk_core::dev_session_active();
    println!("Profile: {profile:?} ({})", cfg.install_hypr_share_dir.display());
    if session_active {
        println!("Session: active (dev enter running)");
        if let Some(bin) = tsk_core::dev_session_binary() {
            println!("  binary: {}", bin.display());
        }
    } else {
        println!("Session: inactive");
    }
    let h = install_hypr_status(&cfg)?;
    let w = install_waybar_status(&cfg)?;
    if h.get("installed").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("Hyprland integration: installed");
        if let Some(p) = h.get("config_path").and_then(|v| v.as_str()) {
            println!("  config: {p}");
        }
    } else {
        println!("Hyprland integration: not installed");
    }
    if w.get("installed").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("Waybar integration: installed");
    } else {
        println!("Waybar integration: not installed");
    }
    if profile.install_systemd() {
        let s = install_systemd_status(&cfg)?;
        if s.get("installed").and_then(|v| v.as_bool()).unwrap_or(false) {
            println!("Systemd daemon service: installed");
            if let Some(p) = s.get("unit_path").and_then(|v| v.as_str()) {
                println!("  unit: {p}");
            }
            let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let active = s.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            println!("  enabled: {enabled}, active: {active}");
        } else {
            println!("Systemd daemon service: not installed");
        }
    } else {
        println!("Systemd daemon service: skipped (dev profile — run `scripts/dev.sh daemon`)");
    }
    Ok(())
}

fn echo_taskspace() -> Result<()> {
    println!("Taskspace: {}", client()?.taskspace_label()?);
    Ok(())
}

fn cmd_taskspace_default() -> Result<()> {
    client()?.context_default()?;
    echo_taskspace()
}

fn cmd_taskspace_current() -> Result<()> {
    println!("{}", client()?.taskspace_label()?);
    Ok(())
}

fn cmd_workspace_go(index: i32) -> Result<()> {
    let name = client()?
        .workspace_go(index)?
        .ok_or_else(|| TskError::Other("Workspace not available in current taskspace".into()))?;
    println!("{name}");
    Ok(())
}

fn cmd_workspace_remember(index: i32) -> Result<()> {
    client()?.remember_workspace_go(index)?;
    Ok(())
}

fn cmd_workspace_dispatch(index: i32) -> Result<()> {
    TaskService::with_defaults()?.workspace_dispatch(index)?;
    Ok(())
}

fn cmd_workspace_switch(index: i32) -> Result<()> {
    let svc = TaskService::with_defaults()?;
    svc.workspace_dispatch(index)?;
    std::thread::spawn(move || {
        let _ = TaskService::with_defaults().and_then(|s| s.remember_workspace_go(index));
    });
    Ok(())
}

fn cmd_workspace_move_dispatch(index: i32) -> Result<()> {
    TaskService::with_defaults()?.workspace_move_dispatch(index)?;
    Ok(())
}

fn cmd_workspace_next() -> Result<()> {
    if let Some(name) = client()?.workspace_next()? {
        println!("{name}");
    }
    Ok(())
}

fn cmd_workspace_prev() -> Result<()> {
    if let Some(name) = client()?.workspace_prev()? {
        println!("{name}");
    }
    Ok(())
}

fn cmd_workspace_goto(name: &str) -> Result<()> {
    let result = client()?
        .workspace_goto(name)?
        .ok_or_else(|| TskError::Other("Workspace not reachable".into()))?;
    println!("{result}");
    Ok(())
}

fn cmd_task_new(
    name: &str,
    switch: bool,
    scratch: bool,
    repo_path: Option<&std::path::Path>,
    no_worktree: bool,
) -> Result<()> {
    if scratch && repo_path.is_some() {
        return Err(TskError::Other(
            "Use either --scratch or --repo-path, not both".into(),
        ));
    }
    let repo = match (scratch, repo_path) {
        (true, None) => TaskRepoSource::Scratch,
        (false, Some(path)) => TaskRepoSource::Path(path.to_path_buf()),
        (false, None) => TaskRepoSource::Auto,
        (true, Some(_)) => unreachable!(),
    };
    let repo_options = tsk_core::TaskRepoOptions {
        create_worktree: !no_worktree,
    };
    let task = client()?.create_task(name, switch, repo, repo_options)?;
    println!(
        "Created task {} → workspaces {}-1..{}-{}",
        task.id,
        task.id,
        task.id,
        task.workspace_count
    );
    println!("Repo: {} ({})", repo_label(&task.repo_path), task.repo_path.display());
    if let Some(home) = task.repo_path.parent() {
        if home.file_name().is_some_and(|n| n == task.id.as_str()) {
            println!("Task home: {}", home.display());
        }
    }
    Ok(())
}

fn cmd_repo_add(dir: Option<&std::path::Path>) -> Result<()> {
    let start = dir
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| TskError::Other("Could not resolve directory".into()))?;
    let existing = load_repos([])?;
    let repo = register_repo(&start, &existing)?;
    println!(
        "Registered {} → {}",
        repo.name,
        repo.path.display()
    );
    println!("Settings: {}", tsk_core::repo_config_path(&repo.path).display());
    Ok(())
}

fn cmd_repo_list() -> Result<()> {
    let repos = load_repos([])?;
    if repos.is_empty() {
        println!("No repos registered — run `tsk repo add` from a checkout");
        return Ok(());
    }
    for repo in repos {
        println!(
            "{:<20}  {}  {}",
            repo.id,
            repo.name,
            repo.path.display()
        );
    }
    Ok(())
}

fn cmd_repo_remove(id_or_path: &str) -> Result<()> {
    let client = client()?;
    let active = client.list_active_tasks()?;
    let archived = client.list_archived_tasks()?;
    let all_tasks: Vec<_> = active.into_iter().chain(archived).collect();

    let repos = load_repos([])?;
    let repo = find_repo(&repos, id_or_path)
        .cloned()
        .or_else(|| {
            let path = std::path::PathBuf::from(id_or_path);
            find_repo_by_path(&repos, &path).cloned()
        })
        .ok_or_else(|| TskError::Other(format!("Unknown repo '{id_or_path}'")))?;
    ensure_repo_removable(&repo, &all_tasks)?;
    unregister_repo(&repo.path)?;
    println!("Removed {}", repo.path.display());
    Ok(())
}

fn cmd_repo_root(dir: Option<&std::path::Path>) -> Result<()> {
    match detect_vcs_root(dir) {
        Some(root) => {
            println!("{}", root.display());
            Ok(())
        }
        None => Err(TskError::Other(format!(
            "No git or jj repo found{}",
            dir.map(|d| format!(" in {}", d.display()))
                .unwrap_or_else(|| " from current directory".into())
        ))),
    }
}

fn cmd_task_archive(name_or_id: &str) -> Result<()> {
    let svc = client()?;
    let task = svc.resolve_task(name_or_id)?;
    let preview = svc.preview_task_teardown(&task.id)?;
    svc.archive_task(&task.id)?;
    println!("Archived {}", task.id);
    if preview.window_count > 0 {
        println!("Closed {} window(s).", preview.window_count);
    }
    if preview.container_exists {
        println!("Stopped container {}.", preview.container_name);
    }
    println!("Task files kept at {}.", preview.data_dir.display());
    Ok(())
}

fn cmd_task_restore(name_or_id: &str) -> Result<()> {
    let svc = client()?;
    let task = svc.resolve_task(name_or_id)?;
    let preview = svc.preview_task_teardown(&task.id)?;
    svc.restore_task(&task.id)?;
    println!("Restored {}", task.id);
    if preview.container_exists {
        println!("Started container {}.", preview.container_name);
    }
    println!("Task files at {}.", preview.data_dir.display());
    Ok(())
}

fn cmd_task_delete(name_or_id: &str) -> Result<()> {
    let svc = client()?;
    let task = svc.resolve_task(name_or_id)?;
    let preview = svc.preview_task_teardown(&task.id)?;
    svc.delete_task(&task.id)?;
    println!("Deleted {}", task.id);
    if preview.window_count > 0 {
        println!("Closed {} window(s).", preview.window_count);
    }
    if preview.container_exists {
        println!("Removed container {}.", preview.container_name);
    }
    if preview.data_dir.exists() {
        println!("Removed task data at {}.", preview.data_dir.display());
    }
    Ok(())
}

fn cmd_task_list(json: bool, include_archived: bool) -> Result<()> {
    let svc = client()?;
    if json {
        let items = svc.tasks_for_menu()?;
        if include_archived {
            let archived: Vec<_> = svc.list_archived_tasks()?.into_iter().map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "name": t.name,
                    "status": t.status.as_str(),
                    "kind": "task",
                    "current": false,
                })
            }).collect();
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "active": items,
                    "archived": archived,
                }))
                .map_err(|e| TskError::Other(e.to_string()))?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string(&items).map_err(|e| TskError::Other(e.to_string()))?
            );
        }
        return Ok(());
    }
    let tasks = svc.list_active_tasks()?;
    if tasks.is_empty() && !include_archived {
        println!("No tasks.");
        return Ok(());
    }
    for t in tasks {
        print_task_line(&t);
    }
    if include_archived {
        let archived = svc.list_archived_tasks()?;
        if !archived.is_empty() {
            println!();
            println!("Archived:");
            for t in archived {
                print_task_line(&t);
            }
        }
    }
    Ok(())
}

fn print_task_line(t: &tsk_core::Task) {
    println!(
        "{:<24} {:<8}  {}  {}",
        t.name,
        t.status.as_str(),
        t.id,
        t.repo_path.display()
    );
}

fn cmd_task_switch(name_or_id: &str) -> Result<()> {
    let svc = client()?;
    let task = svc.resolve_task(name_or_id)?;
    let switched = svc.switch_task(&task.id)?;
    let state = svc.load_state()?;
    let workspace = tsk_core::primary_task_workspace(
        &switched.id,
        state.default_workspace_count,
        &state.global_workspace_slots,
    );
    println!("Switched to task:{} → {workspace}", switched.id);
    Ok(())
}

fn cmd_task_current() -> Result<()> {
    let state = client()?.load_state()?;
    if let Some(id) = state.current_task_id {
        println!("{id}");
    } else {
        println!("(none)");
    }
    Ok(())
}

fn cmd_task_terminal(name_or_id: Option<&str>, host: bool) -> Result<()> {
    let task_id = if host {
        None
    } else if let Some(name) = name_or_id {
        Some(client()?.resolve_task(name)?.id)
    } else {
        None
    };
    client()?.open_terminal(task_id.as_deref(), host)
}

fn cmd_task_tui() -> Result<()> {
    tsk_tui::run()
}

fn cmd_task_tui_launch() -> Result<()> {
    launch_task_tui()
}

fn cmd_waybar_install_status() -> Result<()> {
    let cfg = load_config()?;
    let status = install_waybar_status(&cfg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&status).map_err(|e| TskError::Other(e.to_string()))?
    );
    Ok(())
}

fn cmd_waybar_module(name: &str, index: usize) -> Result<()> {
    let registry = Registry::with_defaults()?;
    let state = registry.load_state()?;
    let modules = build_all_modules(&state, true);
    let key = match name {
        "task" => "task".to_string(),
        "workspace" => workspace_module_key(index),
        _ => unreachable!(),
    };
    let module = modules.get(&key).cloned().unwrap_or_default();
    print!(
        "{}",
        serde_json::to_string(&module).map_err(|e| TskError::Other(e.to_string()))?
    );
    Ok(())
}

fn cmd_debug_trace_show(last: usize) -> Result<()> {
    print!("{}", tail_raw(last)?);
    Ok(())
}

fn cmd_debug_trace_analyze() -> Result<()> {
    let report = analyze_recent_latency();
    print!("{}", format_report(&report));
    Ok(())
}

fn cmd_debug_trace_clear() -> Result<()> {
    clear_log()?;
    println!("Cleared trace log.");
    Ok(())
}

fn cmd_debug_trace_path() -> Result<()> {
    match trace_path() {
        Some(path) => println!("{}", path.display()),
        None => println!("(no runtime dir — set XDG_RUNTIME_DIR)"),
    }
    Ok(())
}

fn cmd_debug_trace_workspace(index: i32, clear: bool, wait_ms: u64) -> Result<()> {
    if clear {
        clear_log()?;
    }
    eprintln!("Note: Waybar needs TSK_TRACE=1 to log widget events.\n");
    enable_for_process();
    cmd_workspace_go(index)?;
    std::thread::sleep(std::time::Duration::from_millis(wait_ms));
    print!("{}", format_report(&analyze_recent_latency()));
    Ok(())
}

fn cmd_debug_hyprland_socket() -> Result<()> {
    let d = diagnose_socket2();
    println!("Hyprland socket2 event socket");
    println!("  available: {}", d.available);
    if let Some(sig) = &d.hyprland_instance_signature {
        println!("  HYPRLAND_INSTANCE_SIGNATURE: {sig}");
    } else {
        println!("  HYPRLAND_INSTANCE_SIGNATURE: (not set)");
    }
    if let Some(runtime) = &d.xdg_runtime_dir {
        println!("  XDG_RUNTIME_DIR: {}", runtime.display());
    } else {
        println!("  XDG_RUNTIME_DIR: (not set)");
    }
    if let Some(path) = &d.path {
        println!("  path: {}", path.display());
    }
    println!("  reason: {}", d.reason);
    if !d.candidates.is_empty() {
        println!("  candidates:");
        for (path, status) in &d.candidates {
            println!("    [{}] {}", status, path.display());
        }
    }
    if !d.available {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_debug_hypr_log_show(last: usize) -> Result<()> {
    let text = tail_hypr_log(last)?;
    if text.is_empty() {
        if let Some(path) = hypr_log_path() {
            println!("No hyprctl log entries yet ({})", path.display());
        } else {
            println!("No hyprctl log path (XDG_RUNTIME_DIR unavailable).");
        }
    } else {
        println!("{text}");
    }
    Ok(())
}

fn cmd_debug_hypr_log_clear() -> Result<()> {
    clear_hypr_log()?;
    println!("Cleared hyprctl log.");
    Ok(())
}

fn cmd_debug_hypr_log_path() -> Result<()> {
    match hypr_log_path() {
        Some(path) => println!("{}", path.display()),
        None => {
            println!("(XDG_RUNTIME_DIR unavailable)");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn daemon_uses_systemd() -> Result<bool> {
    let cfg = load_config()?;
    Ok(!is_dev_config(&cfg) && is_systemd_unit_installed())
}

fn cmd_daemon_start() -> Result<()> {
    if is_daemon_running() {
        println!("Daemon already running.");
        return Ok(());
    }
    if daemon_uses_systemd()? {
        systemd_start()?;
        wait_for_daemon("started")?;
        return Ok(());
    }
    spawn_daemon_and_wait()?;
    println!("Daemon started.");
    Ok(())
}

fn cmd_daemon_restart() -> Result<()> {
    if daemon_uses_systemd()? {
        let was_running = is_daemon_running();
        systemd_restart()?;
        wait_for_daemon(if was_running { "restarted" } else { "started" })?;
        return Ok(());
    }
    let was_running = stop_daemon()?;
    if was_running {
        println!("Daemon stopped.");
    }
    spawn_daemon_and_wait()?;
    if was_running {
        println!("Daemon restarted.");
    } else {
        println!("Daemon started.");
    }
    Ok(())
}

fn wait_for_daemon(action: &str) -> Result<()> {
    for _ in 0..50 {
        if ping_daemon()? {
            println!("Daemon {action}.");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(TskError::Other(format!(
        "daemon did not become reachable after systemd {action}"
    )))
}

fn spawn_daemon_and_wait() -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| TskError::Other(e.to_string()))?;
    let mut child = std::process::Command::new(exe)
        .args(["daemon", "run"])
        .envs(std::env::vars())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| TskError::Other(format!("failed to spawn daemon: {e}")))?;

    for _ in 0..50 {
        if ping_daemon()? {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            let stderr = child
                .stderr
                .take()
                .map(|mut pipe| {
                    use std::io::Read;
                    let mut buf = String::new();
                    let _ = pipe.read_to_string(&mut buf);
                    buf.trim().to_string()
                })
                .unwrap_or_default();
            let detail = if stderr.is_empty() {
                format!("exit status {status}")
            } else {
                format!("exit status {status}: {stderr}")
            };
            return Err(TskError::Other(format!("daemon failed to start ({detail})")));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let _ = child.kill();
    let stderr = child
        .stderr
        .take()
        .map(|mut pipe| {
            use std::io::Read;
            let mut buf = String::new();
            let _ = pipe.read_to_string(&mut buf);
            buf.trim().to_string()
        })
        .unwrap_or_default();
    let mut message = "daemon process started but did not become reachable".to_string();
    if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(&stderr);
    }
    Err(TskError::Other(message))
}

fn cmd_daemon_run() -> Result<()> {
    eprintln!("Starting tsk daemon (foreground)...");
    DaemonServer::new()?.run_foreground()
}

fn cmd_daemon_stop() -> Result<()> {
    if daemon_uses_systemd()? {
        systemd_stop()?;
        println!("Daemon stopped.");
        return Ok(());
    }
    if stop_daemon()? {
        println!("Daemon stopped.");
    } else {
        println!("Daemon is not running.");
    }
    Ok(())
}

fn cmd_daemon_status() -> Result<()> {
    if is_systemd_unit_installed() {
        let active = systemctl_is_active().unwrap_or(false);
        let path = daemon_socket_path()?;
        if is_daemon_running() {
            println!("running (systemd, {})", path.display());
        } else if active {
            println!("systemd unit active but daemon not reachable ({})", path.display());
        } else {
            println!("stopped (systemd unit inactive; CLI will use direct mode)");
        }
        return Ok(());
    }
    if is_daemon_running() {
        let path = daemon_socket_path()?;
        println!("running ({})", path.display());
    } else {
        println!("stopped (CLI will use direct mode)");
    }
    Ok(())
}

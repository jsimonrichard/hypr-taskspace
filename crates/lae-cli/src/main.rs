use clap::{Parser, Subcommand};

use lae_core::{
    allowed_workspace_names, analyze_recent_latency, build_all_modules, clear_log, diagnose_socket2,
    enable_for_process, format_report, hyprland, install_hypr, install_hypr_status, install_waybar,
    install_waybar_status, load_config, refresh_modules_cache, run_doctor_checks, tail_raw,
    trace_path, uninstall_hypr, uninstall_waybar, workspace_module_key, InstallHyprOptions,
    InstallWaybarOptions, LaeError, Registry, Result, TaskService, TaskStatus,
};

#[derive(Parser)]
#[command(name = "lae", about = "Local Agentic Environment")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Status,
    Doctor,
    Windows {
        #[arg(long)]
        task: Option<String>,
    },
    Install {
        #[command(subcommand)]
        command: InstallCommands,
    },
    Uninstall {
        #[command(subcommand)]
        command: UninstallCommands,
    },
    #[command(subcommand)]
    Taskspace(TaskspaceCommands),
    #[command(subcommand, name = "context", about = "Alias for taskspace (deprecated name)")]
    Context(TaskspaceCommands),
    #[command(subcommand)]
    Workspace(WorkspaceCommands),
    #[command(subcommand, name = "desktop", about = "Alias for workspace (deprecated name)")]
    Desktop(WorkspaceCommands),
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    Waybar {
        #[command(subcommand)]
        command: WaybarCommands,
    },
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },
}

#[derive(Subcommand)]
enum InstallCommands {
    /// Install Hyprland + Waybar integrations
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
    Status,
}

#[derive(Subcommand)]
enum UninstallCommands {
    Hypr {
        #[arg(long)]
        keep_files: bool,
    },
    Waybar,
}

#[derive(Subcommand)]
enum TaskspaceCommands {
    Default,
    Global,
    Restore,
    #[command(name = "toggle-global")]
    ToggleGlobal,
    Current,
}

#[derive(Subcommand)]
enum WorkspaceCommands {
    Go {
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
        #[arg(long)]
        no_switch: bool,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Switch {
        name_or_id: String,
    },
    Current,
    Archive {
        name_or_id: String,
    },
    #[command(name = "menu-json")]
    MenuJson,
    Menu,
}

#[derive(Subcommand)]
enum WaybarCommands {
    RefreshCache,
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
    /// Trace log utilities (set LAE_TRACE=1 on Waybar for widget events)
    Trace {
        #[command(subcommand)]
        command: DebugTraceCommands,
    },
    /// Diagnose Hyprland socket2 event socket (Waybar live updates)
    #[command(name = "hyprland-socket")]
    HyprlandSocket,
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
    match Cli::parse().command {
        Commands::Status => cmd_status(),
        Commands::Doctor => cmd_doctor(),
        Commands::Windows { task } => cmd_windows(task.as_deref()),
        Commands::Install { command } => match command {
            InstallCommands::All {
                dry_run,
                workspace,
            } => cmd_install_all(dry_run, workspace),
            InstallCommands::Hypr {
                dry_run,
                workspace,
            } => cmd_install_hypr(dry_run, workspace),
            InstallCommands::Waybar {
                dry_run,
                workspace,
            } => cmd_install_waybar(dry_run, workspace),
            InstallCommands::Status => cmd_install_status(),
        },
        Commands::Uninstall { command } => match command {
            UninstallCommands::Hypr { keep_files } => cmd_uninstall_hypr(keep_files),
            UninstallCommands::Waybar => cmd_uninstall_waybar(),
        },
        Commands::Taskspace(command) | Commands::Context(command) => match command {
            TaskspaceCommands::Default => cmd_taskspace_default(),
            TaskspaceCommands::Global => cmd_taskspace_global(),
            TaskspaceCommands::Restore => cmd_taskspace_restore(),
            TaskspaceCommands::ToggleGlobal => cmd_taskspace_toggle(),
            TaskspaceCommands::Current => cmd_taskspace_current(),
        },
        Commands::Workspace(command) | Commands::Desktop(command) => match command {
            WorkspaceCommands::Go { index } => cmd_workspace_go(index),
            WorkspaceCommands::Next => cmd_workspace_next(),
            WorkspaceCommands::Prev => cmd_workspace_prev(),
            WorkspaceCommands::Goto { name } => cmd_workspace_goto(&name),
        },
        Commands::Task { command } => match command {
            TaskCommands::New { name, no_switch } => cmd_task_new(&name, !no_switch),
            TaskCommands::List { json } => cmd_task_list(json),
            TaskCommands::Switch { name_or_id } => cmd_task_switch(&name_or_id),
            TaskCommands::Current => cmd_task_current(),
            TaskCommands::Archive { name_or_id } => cmd_task_archive(&name_or_id),
            TaskCommands::MenuJson => cmd_task_menu_json(),
            TaskCommands::Menu => cmd_task_menu(),
        },
        Commands::Waybar { command } => match command {
            WaybarCommands::RefreshCache => cmd_waybar_refresh_cache(),
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
        },
    }
}

fn service() -> Result<TaskService> {
    TaskService::with_defaults()
}

fn cmd_status() -> Result<()> {
    let svc = service()?;
    let state = svc.load_state()?;
    let allowed = allowed_workspace_names(&state);
    let taskspace_label = state.taskspace_label();

    if !allowed.is_empty() {
        println!("Taskspace: {taskspace_label}");
        println!("Workspaces: {}", allowed.join(", "));
        println!("Escape: SUPER+ESCAPE for global taskspace");
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

fn cmd_windows(task_filter: Option<&str>) -> Result<()> {
    let state = service()?.load_state()?;
    for w in hyprland::get_clients()? {
        let task_id = state
            .windows
            .get(&w.address)
            .and_then(|r| r.task_id.as_deref())
            .unwrap_or("default");
        if task_filter.is_some_and(|t| t != task_id) {
            continue;
        }
        println!(
            "{} [{task_id}] {} ws={}",
            w.address,
            w.title,
            if w.workspace_name.is_empty() {
                w.workspace.to_string()
            } else {
                w.workspace_name.clone()
            }
        );
    }
    Ok(())
}

fn cmd_install_all(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    cmd_install_hypr(dry_run, workspace.clone())?;
    if !dry_run {
        println!();
    }
    cmd_install_waybar(dry_run, workspace)
}

fn cmd_install_hypr(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    let cfg = load_config()?;
    let options = InstallHyprOptions {
        dry_run,
        workspace_root: workspace,
    };
    let actions = install_hypr(&cfg, &options)?;
    if dry_run {
        for line in actions {
            println!("{line}");
        }
    } else {
        println!("Installed Hyprland integration.");
        if !actions.is_empty() {
            println!("Applied: {}.", actions.join(", "));
        }
    }
    Ok(())
}

fn cmd_install_waybar(dry_run: bool, workspace: Option<std::path::PathBuf>) -> Result<()> {
    let cfg = load_config()?;
    let options = InstallWaybarOptions {
        dry_run,
        workspace_root: workspace,
    };
    let actions = install_waybar(&cfg, &options)?;
    if dry_run {
        for line in actions {
            println!("{line}");
        }
    } else {
        println!("Installed Waybar integration.");
        if !actions.is_empty() {
            println!("Applied: {}.", actions.join(", "));
        }
    }
    Ok(())
}

fn cmd_install_status() -> Result<()> {
    let cfg = load_config()?;
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
    Ok(())
}

fn cmd_uninstall_hypr(keep_files: bool) -> Result<()> {
    let cfg = load_config()?;
    let actions = uninstall_hypr(&cfg, keep_files)?;
    println!("Uninstalled Hyprland integration.");
    if !actions.is_empty() {
        println!("Applied: {}.", actions.join(", "));
    }
    Ok(())
}

fn cmd_uninstall_waybar() -> Result<()> {
    let cfg = load_config()?;
    let actions = uninstall_waybar(&cfg)?;
    println!("Uninstalled Waybar integration.");
    if !actions.is_empty() {
        println!("Applied: {}.", actions.join(", "));
    }
    Ok(())
}

fn echo_taskspace() -> Result<()> {
    println!("Taskspace: {}", service()?.taskspace_label()?);
    Ok(())
}

fn cmd_taskspace_default() -> Result<()> {
    service()?.context_default()?;
    echo_taskspace()
}

fn cmd_taskspace_global() -> Result<()> {
    service()?.context_global()?;
    echo_taskspace()
}

fn cmd_taskspace_restore() -> Result<()> {
    service()?.context_restore()?;
    echo_taskspace()
}

fn cmd_taskspace_toggle() -> Result<()> {
    service()?.toggle_global()?;
    echo_taskspace()
}

fn cmd_taskspace_current() -> Result<()> {
    println!("{}", service()?.taskspace_label()?);
    Ok(())
}

fn cmd_workspace_go(index: i32) -> Result<()> {
    let name = service()?
        .workspace_go(index)?
        .ok_or_else(|| LaeError::Other("Workspace not available in current taskspace".into()))?;
    println!("{name}");
    Ok(())
}

fn cmd_workspace_next() -> Result<()> {
    if let Some(name) = service()?.workspace_next()? {
        println!("{name}");
    }
    Ok(())
}

fn cmd_workspace_prev() -> Result<()> {
    if let Some(name) = service()?.workspace_prev()? {
        println!("{name}");
    }
    Ok(())
}

fn cmd_workspace_goto(name: &str) -> Result<()> {
    let result = service()?
        .workspace_goto(name)?
        .ok_or_else(|| LaeError::Other("Workspace not reachable".into()))?;
    println!("{result}");
    Ok(())
}

fn cmd_task_new(name: &str, switch: bool) -> Result<()> {
    let task = service()?.create_task(name, switch)?;
    println!(
        "Created task {} → workspaces {}-1..{}-{}",
        task.id,
        task.id,
        task.id,
        task.workspace_count
    );
    println!("Task home: {}", task.repo_path.parent().unwrap_or(&task.repo_path).display());
    Ok(())
}

fn cmd_task_archive(name_or_id: &str) -> Result<()> {
    let svc = service()?;
    let task = svc.resolve_task(name_or_id)?;
    svc.archive_task(&task.id)?;
    println!("Archived {}", task.id);
    Ok(())
}

fn cmd_task_list(json: bool) -> Result<()> {
    let svc = service()?;
    if json {
        let items = svc.tasks_for_menu()?;
        println!(
            "{}",
            serde_json::to_string(&items).map_err(|e| LaeError::Other(e.to_string()))?
        );
        return Ok(());
    }
    let tasks = svc.list_active_tasks()?;
    if tasks.is_empty() {
        println!("No tasks.");
        return Ok(());
    }
    for t in tasks {
        println!(
            "{:<20} {:<8}  {}-1..{}-{}  {}",
            t.id,
            t.status.as_str(),
            t.id,
            t.id,
            t.workspace_count,
            t.repo_path.display()
        );
    }
    Ok(())
}

fn cmd_task_switch(name_or_id: &str) -> Result<()> {
    let svc = service()?;
    let task = svc.resolve_task(name_or_id)?;
    let switched = svc.switch_task(&task.id)?;
    println!(
        "Switched to task:{} → {}",
        switched.id,
        switched.main_workspace()
    );
    Ok(())
}

fn cmd_task_current() -> Result<()> {
    let state = service()?.load_state()?;
    if let Some(id) = state.current_task_id {
        println!("{id}");
    } else {
        println!("(none)");
    }
    Ok(())
}

fn cmd_task_menu_json() -> Result<()> {
    let lae = std::env::var("LAE").unwrap_or_else(|_| "lae".into());
    for item in service()?.tasks_for_menu()? {
        let action = if item.kind == "default" {
            format!("{lae} taskspace default")
        } else {
            format!("{lae} task switch {}", item.id)
        };
        let mut label = item.name.clone();
        if item.current {
            label.push_str(" (active)");
        }
        let workspaces = item.workspaces.join(", ");
        println!("{label}\t{workspaces}\t{}\t{action}", item.status);
    }
    Ok(())
}

fn cmd_task_menu() -> Result<()> {
    use std::process::Command;
    let cfg = load_config()?;
    let launcher = if Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {}", cfg.walker_launch_command))
        .output()
        .is_ok_and(|o| o.status.success())
    {
        cfg.walker_launch_command.clone()
    } else if Command::new("sh")
        .arg("-c")
        .arg("command -v walker")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        "walker".into()
    } else {
        return Err(LaeError::Other(
            "walker or omarchy-launch-walker not found on PATH".into(),
        ));
    };
    let _ = Command::new(launcher)
        .args([
            "-m",
            "menus:laetasks",
            "--width",
            "644",
            "--minheight",
            "300",
            "--maxheight",
            "630",
        ])
        .spawn()
        .map_err(|e| LaeError::Other(format!("failed to launch walker: {e}")))?;
    Ok(())
}

fn cmd_waybar_refresh_cache() -> Result<()> {
    let registry = Registry::with_defaults()?;
    refresh_modules_cache(&registry, true)?;
    Ok(())
}

fn cmd_waybar_install_status() -> Result<()> {
    let cfg = load_config()?;
    let status = install_waybar_status(&cfg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&status).map_err(|e| LaeError::Other(e.to_string()))?
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
        serde_json::to_string(&module).map_err(|e| LaeError::Other(e.to_string()))?
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
    eprintln!("Note: Waybar needs LAE_TRACE=1 to log widget events.\n");
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

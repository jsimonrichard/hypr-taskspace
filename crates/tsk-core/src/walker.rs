//! Walker / Elephant launch integration — taskspace env and launch failure notifications.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::apps::{resolve_browser_command, resolve_editor_command, BROWSER_CANDIDATES, EDITOR_CANDIDATES};
use crate::config::load_config;
use crate::context_sync;
use crate::error::{Result, TskError};
use crate::models::{ContextMode, SessionState, Task};
use crate::registry::Registry;
use crate::service::TaskService;
use crate::task_env;
use crate::terminal;

/// How long the detached `watch-launch` helper waits for an early launcher failure.
/// `uwsm app` returns quickly on both success and desktop-file validation errors.
const LAUNCH_FAIL_WATCH: Duration = Duration::from_millis(1500);
const STDERR_NOTIFY_MAX: usize = 280;

const TERMINAL_DESKTOP_IDS: &[&str] = &[
    "Alacritty",
    "org.alacritty.Alacritty",
    "kitty",
    "foot",
    "com.mitchellh.ghostty",
    "org.wezfurlong.wezterm",
    "org.gnome.Console",
    "org.gnome.Terminal",
    "xfce4-terminal",
];

const BROWSER_DESKTOP_IDS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "firefox",
    "brave-browser",
    "Brave-browser",
    "org.mozilla.firefox",
];

const EDITOR_DESKTOP_IDS: &[&str] = &[
    "cursor",
    "code",
    "codium",
    "VSCodium",
    "com.visualstudio.code",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkerIntegration {
    TaskTerminal,
    TaskBrowser,
    TaskEditor,
    Generic,
}

#[derive(Debug, Clone)]
pub struct WalkerLaunchContext {
    pub state: SessionState,
    pub env: Vec<(String, String)>,
    pub task: Option<Task>,
    pub cwd: PathBuf,
}

impl WalkerLaunchContext {
    pub fn resolve() -> Result<Self> {
        let cfg = load_config()?;
        let registry = Registry::new(None, cfg.clone())?;
        let mut state = registry.load_state()?;
        context_sync::sync_from_active_workspace(&mut state);

        let task = active_task(&state).cloned();
        let env = if let Some(ref task) = task {
            task_env::build_task_env(&state, task, &cfg.tasks_base_dir, None)
        } else {
            task_env::build_taskspace_env(&state, &cfg.tasks_base_dir)
        };
        let cwd = task
            .as_ref()
            .map(|t| t.repo_path.clone())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));

        Ok(Self {
            state,
            env,
            task,
            cwd,
        })
    }
}

#[derive(Debug, Clone)]
struct LaunchTarget {
    desktop_id: Option<String>,
    desktop: Option<DesktopEntry>,
    argv: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct DesktopEntry {
    id: String,
    /// `Name=` from the desktop file (preferred for notifications).
    name: Option<String>,
    exec: Option<String>,
    terminal: bool,
    categories: Vec<String>,
    try_exec: Option<String>,
}

/// Elephant / uwsm passes through the same argv it would give `uwsm app`.
pub fn walker_exec(args: &[&str]) -> Result<()> {
    if args.is_empty() {
        return Err(TskError::Other(
            "walker exec: missing application (desktop id or command)".into(),
        ));
    }
    let ctx = WalkerLaunchContext::resolve()?;
    let target = LaunchTarget::parse(args)?;
    match target.integration() {
        WalkerIntegration::TaskTerminal => walker_open_terminal(&ctx, &target)?,
        WalkerIntegration::TaskBrowser => walker_open_browser(&ctx, &target)?,
        WalkerIntegration::TaskEditor => walker_open_editor(&ctx, &target)?,
        WalkerIntegration::Generic => walker_launch_generic(&ctx, &target)?,
    }
    Ok(())
}

/// Run a command in a task-scoped terminal, or open an empty task terminal when no args.
pub fn walker_terminal(args: &[&str]) -> Result<()> {
    let ctx = WalkerLaunchContext::resolve()?;
    if args.is_empty() {
        return walker_open_terminal(&ctx, &LaunchTarget::empty());
    }
    let cfg = load_config()?;
    let term = terminal::resolve_terminal_command(&cfg)?;
    let (program, program_args) = split_command(args)?;
    let title = terminal_title(&ctx);
    terminal::spawn_terminal_command(
        &term,
        Path::new(&program),
        &program_args.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(&ctx.cwd),
        &title,
        "org.tsk.task-terminal",
        &ctx.env,
    )
}

fn walker_open_terminal(_ctx: &WalkerLaunchContext, target: &LaunchTarget) -> Result<()> {
    if target.opens_terminal_emulator() {
        let svc = TaskService::with_defaults()?;
        return svc.open_terminal(None, false);
    }
    walker_terminal(target.argv.iter().map(String::as_str).collect::<Vec<_>>().as_slice())
}

fn walker_open_browser(_ctx: &WalkerLaunchContext, target: &LaunchTarget) -> Result<()> {
    if target.opens_browser_app() {
        return TaskService::with_defaults()?.open_browser(None, false);
    }
    let browser = resolve_browser_command().ok_or_else(|| {
        TskError::Other("walker exec: browser not found".into())
    })?;
    let extra: Vec<&str> = target.argv.iter().skip(1).map(String::as_str).collect();
    launch_with_env(_ctx, Some(&browser), &extra)
}

fn walker_open_editor(ctx: &WalkerLaunchContext, target: &LaunchTarget) -> Result<()> {
    if target.opens_editor_app() {
        return TaskService::with_defaults()?.open_editor(None);
    }
    let editor = resolve_editor_command().ok_or_else(|| {
        TskError::Other("walker exec: editor not found".into())
    })?;
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into());
    let path = target.argv.get(1).cloned().unwrap_or(cwd);
    launch_with_env(ctx, Some(&editor), &[path.as_str()])
}

fn walker_launch_generic(ctx: &WalkerLaunchContext, target: &LaunchTarget) -> Result<()> {
    let label = launch_label(target, &target.argv);
    if let Some(uwsm) = command_v("uwsm") {
        let mut cmd = Command::new(&uwsm);
        cmd.arg("app");
        apply_launch_env(&mut cmd, ctx);
        cmd.current_dir(&ctx.cwd);
        push_uwsm_args(&mut cmd, target);
        return spawn_watched(cmd, label);
    }

    let (program, program_args) = if target.argv.is_empty() {
        return Err(TskError::Other("walker exec: empty launch target".into()));
    } else {
        split_command(target.argv.iter().map(String::as_str).collect::<Vec<_>>().as_slice())?
    };
    let mut cmd = Command::new(&program);
    apply_launch_env(&mut cmd, ctx);
    cmd.current_dir(&ctx.cwd);
    cmd.args(&program_args);
    spawn_watched(cmd, label)
}

fn launch_with_env(ctx: &WalkerLaunchContext, program: Option<&str>, args: &[&str]) -> Result<()> {
    let Some(program) = program else {
        return Err(TskError::Other("walker exec: program not found".into()));
    };
    let mut cmd = Command::new(program);
    apply_launch_env(&mut cmd, ctx);
    cmd.current_dir(&ctx.cwd);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| TskError::Other(format!("failed to launch `{program}`: {e}")))
}

fn apply_launch_env(cmd: &mut Command, ctx: &WalkerLaunchContext) {
    task_env::apply_env(cmd, &ctx.env);
}

fn push_uwsm_args(cmd: &mut Command, target: &LaunchTarget) {
    if let Some(id) = target.desktop_id.as_deref() {
        cmd.arg(id);
        return;
    }
    if !target.argv.is_empty() {
        for arg in &target.argv {
            cmd.arg(arg);
        }
    }
}

fn launch_label(target: &LaunchTarget, argv: &[String]) -> String {
    if let Some(name) = target
        .desktop
        .as_ref()
        .and_then(|d| d.name.as_deref())
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    if let Some(cmd) = format_command_label(argv) {
        return cmd;
    }
    target
        .desktop_id
        .clone()
        .unwrap_or_else(|| "application".into())
}

/// Human-readable command for notifications, e.g. `todoist --no-sandbox`.
fn format_command_label(argv: &[String]) -> Option<String> {
    if argv.is_empty() {
        return None;
    }
    let mut idx = 0;
    if argv[0] == "env" {
        idx = 1;
        while idx < argv.len() {
            let arg = &argv[idx];
            if arg.starts_with('-') {
                break;
            }
            if arg.contains('=') {
                idx += 1;
                continue;
            }
            break;
        }
    }
    let program = argv.get(idx)?;
    let base = Path::new(program)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(program.as_str());
    let mut out = base.to_string();
    for arg in argv.iter().skip(idx + 1).take(4) {
        out.push(' ');
        out.push_str(arg);
    }
    if argv.len() > idx + 1 + 4 {
        out.push_str(" …");
    }
    Some(out)
}

/// Spawn a detached helper that runs `argv` and notifies on early failure.
///
/// The helper is a separate process so it outlives this `tsk walker exec` invocation
/// (Walker/Elephant does not wait). A same-process thread would be killed on exit.
fn spawn_watched(cmd: Command, label: String) -> Result<()> {
    let program = cmd.get_program().to_owned();
    let args: Vec<_> = cmd.get_args().map(|a| a.to_owned()).collect();
    let helper = std::env::current_exe().map_err(|e| {
        TskError::Other(format!("walker exec: current_exe unavailable: {e}"))
    })?;

    let mut watch = Command::new(&helper);
    // Inherit process env, then overlay whatever was set on the prepared launch command
    // (taskspace vars from apply_launch_env).
    for (key, value) in cmd.get_envs() {
        match value {
            Some(v) => {
                watch.env(key, v);
            }
            None => {
                watch.env_remove(key);
            }
        }
    }
    if let Some(dir) = cmd.get_current_dir() {
        watch.current_dir(dir);
    }
    // uwsm/GLib log to the console; without this, piped stderr is often empty.
    watch.env("SYSTEMD_LOG_TARGET", "console");
    watch.args(["walker", "watch-launch", "--label", &label, "--"]);
    watch.arg(&program);
    watch.args(&args);
    watch
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| TskError::Other(format!("failed to launch `{label}`: {e}")))
}

/// Internal: run a launch command and notify if it fails quickly.
pub fn walker_watch_launch(label: &str, args: &[&str]) -> Result<()> {
    if args.is_empty() {
        return Err(TskError::Other("walker watch-launch: missing command".into()));
    }
    let (program, program_args) = split_command(args)?;
    let mut cmd = Command::new(&program);
    cmd.args(&program_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Prefer console logs from uwsm even if parent forgot to set it.
    cmd.env("SYSTEMD_LOG_TARGET", "console");

    let mut child = cmd
        .spawn()
        .map_err(|e| TskError::Other(format!("failed to launch `{label}`: {e}")))?;
    let stderr = child.stderr.take();
    let deadline = Instant::now() + LAUNCH_FAIL_WATCH;
    match wait_until(&mut child, deadline) {
        Some(status) if !status.success() => {
            let detail = read_stderr_snippet(stderr);
            notify_launch_failure(label, status.code(), &detail);
        }
        Some(_) => {}
        None => {
            // Still running — assume success. Dropping Child leaves the process reparented
            // to init when this helper exits (no kill).
            std::mem::forget(child);
        }
    }
    Ok(())
}

fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

fn read_stderr_snippet(stderr: Option<std::process::ChildStderr>) -> String {
    let Some(mut stderr) = stderr else {
        return String::new();
    };
    let mut buf = String::new();
    let _ = stderr.read_to_string(&mut buf);
    sanitize_launch_stderr(&buf)
}

fn sanitize_launch_stderr(raw: &str) -> String {
    let cleaned = raw
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches("<3>")
                .trim_start_matches("<4>")
                .trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if cleaned.chars().count() <= STDERR_NOTIFY_MAX {
        return cleaned;
    }
    let truncated: String = cleaned.chars().take(STDERR_NOTIFY_MAX).collect();
    format!("{truncated}…")
}

fn notify_launch_failure(label: &str, code: Option<i32>, detail: &str) {
    let code_text = code
        .map(|c| format!("exited with code {c}"))
        .unwrap_or_else(|| "exited with an error".into());
    let body = if detail.is_empty() {
        code_text
    } else {
        format!("{code_text}\n\n{detail}")
    };
    let _ = Command::new("notify-send")
        .args([
            "--urgency=critical",
            "--app-name=tsk",
            "--icon=dialog-error",
            "--category=device.error",
            &format!("Failed to launch {label}"),
            &body,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

impl LaunchTarget {
    fn empty() -> Self {
        Self {
            desktop_id: None,
            desktop: None,
            argv: Vec::new(),
        }
    }

    fn parse(args: &[&str]) -> Result<Self> {
        let first = args[0];
        if let Some(desktop) = resolve_desktop_entry(first) {
            let argv = desktop
                .exec
                .as_deref()
                .map(parse_desktop_exec)
                .transpose()?
                .unwrap_or_default();
            return Ok(Self {
                desktop_id: Some(desktop.id.clone()),
                desktop: Some(desktop),
                argv,
            });
        }

        Ok(Self {
            desktop_id: None,
            desktop: None,
            argv: args.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn integration(&self) -> WalkerIntegration {
        if self.is_terminal() {
            return WalkerIntegration::TaskTerminal;
        }
        if self.is_browser() {
            return WalkerIntegration::TaskBrowser;
        }
        if self.is_editor() {
            return WalkerIntegration::TaskEditor;
        }
        WalkerIntegration::Generic
    }

    fn is_terminal(&self) -> bool {
        if self.desktop.as_ref().is_some_and(|d| d.terminal) {
            return true;
        }
        if self
            .desktop
            .as_ref()
            .is_some_and(|d| d.categories.iter().any(|c| c == "TerminalEmulator" || c == "Terminal"))
        {
            return true;
        }
        if let Some(id) = self.desktop_id.as_deref() {
            if TERMINAL_DESKTOP_IDS
                .iter()
                .any(|term| term.eq_ignore_ascii_case(id))
            {
                return true;
            }
        }
        argv_starts_with_candidate(&self.argv, terminal::TERMINAL_FALLBACKS)
    }

    /// True when the user is launching a terminal emulator app (e.g. Alacritty.desktop),
    /// not running an arbitrary command inside a task terminal.
    fn opens_terminal_emulator(&self) -> bool {
        if self.argv.is_empty() {
            return true;
        }
        if let Some(id) = self.desktop_id.as_deref() {
            if TERMINAL_DESKTOP_IDS
                .iter()
                .any(|term| term.eq_ignore_ascii_case(id))
            {
                return true;
            }
        }
        if self.desktop.as_ref().is_some_and(|d| {
            d.categories
                .iter()
                .any(|c| c == "TerminalEmulator" || c == "Terminal")
        }) {
            return true;
        }
        self.argv.len() == 1 && argv_starts_with_candidate(&self.argv, terminal::TERMINAL_FALLBACKS)
    }

    fn is_browser(&self) -> bool {
        if self
            .desktop
            .as_ref()
            .is_some_and(|d| d.categories.iter().any(|c| c == "Network" || c == "WebBrowser"))
        {
            return true;
        }
        if let Some(id) = self.desktop_id.as_deref() {
            if BROWSER_DESKTOP_IDS
                .iter()
                .any(|browser| browser.eq_ignore_ascii_case(id))
            {
                return true;
            }
        }
        argv_starts_with_candidate(&self.argv, BROWSER_CANDIDATES)
    }

    /// Launching a browser app (Chromium, Firefox, …), not a one-off URL handler command.
    fn opens_browser_app(&self) -> bool {
        self.opens_integrated_app(
            BROWSER_DESKTOP_IDS,
            &["Network", "WebBrowser"],
            BROWSER_CANDIDATES,
        )
    }

    fn is_editor(&self) -> bool {
        if self.desktop.as_ref().is_some_and(|d| {
            d.categories
                .iter()
                .any(|c| c == "Development" || c == "IDE" || c == "TextEditor")
        }) {
            return true;
        }
        if let Some(id) = self.desktop_id.as_deref() {
            if EDITOR_DESKTOP_IDS
                .iter()
                .any(|editor| editor.eq_ignore_ascii_case(id))
            {
                return true;
            }
        }
        argv_starts_with_candidate(&self.argv, EDITOR_CANDIDATES)
    }

    /// Launching an editor app (Cursor, VS Code, …), not opening a specific file path.
    fn opens_editor_app(&self) -> bool {
        self.opens_integrated_app(
            EDITOR_DESKTOP_IDS,
            &["Development", "IDE", "TextEditor"],
            EDITOR_CANDIDATES,
        )
    }

    fn opens_integrated_app(
        &self,
        desktop_ids: &[&str],
        categories: &[&str],
        candidates: &[&str],
    ) -> bool {
        if self.argv.is_empty() {
            return true;
        }
        if let Some(id) = self.desktop_id.as_deref() {
            if desktop_ids
                .iter()
                .any(|name| name.eq_ignore_ascii_case(id))
            {
                return true;
            }
        }
        if self.desktop.as_ref().is_some_and(|d| {
            d.categories
                .iter()
                .any(|c| categories.iter().any(|cat| cat == c))
        }) {
            return true;
        }
        argv_starts_with_candidate(&self.argv, candidates)
    }
}

fn argv_starts_with_candidate(argv: &[String], candidates: &[&str]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };
    let base = Path::new(program)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(program.as_str());
    candidates
        .iter()
        .any(|c| base.eq_ignore_ascii_case(c))
}

fn active_task(state: &SessionState) -> Option<&Task> {
    if state.context_mode != ContextMode::Task {
        return None;
    }
    let id = state.current_task_id.as_deref()?;
    state.tasks.get(id)
}

fn terminal_title(ctx: &WalkerLaunchContext) -> String {
    if let Some(task) = ctx.task.as_ref() {
        format!("[{}] terminal", task.id)
    } else {
        "terminal".into()
    }
}

fn split_command(args: &[&str]) -> Result<(String, Vec<String>)> {
    let Some(program) = args.first() else {
        return Err(TskError::Other("walker: empty command".into()));
    };
    Ok((program.to_string(), args[1..].iter().map(|s| s.to_string()).collect()))
}

fn command_v(name: &str) -> Option<String> {
    crate::binary::command_v_login(name)
}

fn resolve_desktop_entry(id_or_path: &str) -> Option<DesktopEntry> {
    if id_or_path.contains('/') {
        let path = Path::new(id_or_path);
        if path.extension().is_some_and(|e| e == "desktop") && path.is_file() {
            return parse_desktop_file(path, path.file_stem()?.to_str()?);
        }
        return None;
    }

    let id = id_or_path.strip_suffix(".desktop").unwrap_or(id_or_path);
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    if let Some(home) = std::env::var_os("HOME") {
        let mut dirs = data_dirs;
        dirs.insert(0, PathBuf::from(home).join(".local/share"));
        for dir in dirs {
            let path = dir.join("applications").join(format!("{id}.desktop"));
            if path.is_file() {
                return parse_desktop_file(&path, id);
            }
        }
    }
    None
}

fn parse_desktop_file(path: &Path, id: &str) -> Option<DesktopEntry> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut entry = DesktopEntry {
        id: id.to_string(),
        ..Default::default()
    };
    let mut in_desktop_entry = false;
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        if line.starts_with("Name=") && entry.name.is_none() {
            entry.name = Some(line.trim_start_matches("Name=").to_string());
        } else if line.starts_with("Exec=") && entry.exec.is_none() {
            entry.exec = Some(line.trim_start_matches("Exec=").to_string());
        } else if line.starts_with("TryExec=") {
            entry.try_exec = Some(line.trim_start_matches("TryExec=").to_string());
        } else if line.starts_with("Terminal=") {
            entry.terminal = line.trim_start_matches("Terminal=").trim() == "true";
        } else if line.starts_with("Categories=") {
            entry.categories = line
                .trim_start_matches("Categories=")
                .split(';')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    Some(entry)
}

fn parse_desktop_exec(exec: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut escaped = false;
    let mut chars = exec.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            if !is_exec_field_code(ch) {
                current.push(ch);
            }
            escaped = false;
            continue;
        }
        if ch == '%' && !in_single {
            if let Some(&next) = chars.peek() {
                if is_exec_field_code(next) || next == '%' {
                    chars.next();
                    continue;
                }
            }
        }
        match ch {
            '\\' => escaped = true,
            '\'' if !in_single => in_single = true,
            '\'' if in_single => in_single = false,
            ' ' | '\t' if !in_single => {
                if !current.is_empty() {
                    out.push(take(&mut current));
                }
            }
            _ if !in_single || ch != '\'' => current.push(ch),
            _ => {}
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        return Err(TskError::Other(format!("invalid desktop Exec line: {exec}")));
    }
    Ok(out)
}

fn is_exec_field_code(ch: char) -> bool {
    matches!(ch, 'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm')
}

fn take(slot: &mut String) -> String {
    std::mem::take(slot)
}

// Expose for terminal module reuse in install templates.
pub fn walker_launch_prefix(tsk_cmd: &str) -> String {
    format!("{tsk_cmd} walker exec --")
}

pub fn walker_terminal_cmd(tsk_cmd: &str) -> String {
    format!("{tsk_cmd} walker terminal --")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_desktop_exec_strips_field_codes() {
        let argv = parse_desktop_exec("firefox %u").unwrap();
        assert_eq!(argv, vec!["firefox"]);
    }

    #[test]
    fn parse_desktop_exec_handles_quotes() {
        let argv = parse_desktop_exec("cursor '/some path'").unwrap();
        assert_eq!(argv, vec!["cursor", "/some path"]);
    }

    #[test]
    fn browser_detection_by_exec() {
        let target = LaunchTarget {
            desktop_id: None,
            desktop: None,
            argv: vec!["chromium".into(), "--new-window".into()],
        };
        assert_eq!(target.integration(), WalkerIntegration::TaskBrowser);
    }

    #[test]
    fn editor_detection_by_exec() {
        let target = LaunchTarget {
            desktop_id: None,
            desktop: None,
            argv: vec!["cursor".into()],
        };
        assert_eq!(target.integration(), WalkerIntegration::TaskEditor);
    }

    #[test]
    fn alacritty_desktop_opens_task_terminal_not_nested_emulator() {
        let target = LaunchTarget {
            desktop_id: Some("Alacritty".into()),
            desktop: Some(DesktopEntry {
                id: "Alacritty".into(),
                name: Some("Alacritty".into()),
                exec: Some("alacritty".into()),
                terminal: false,
                categories: vec!["System".into(), "TerminalEmulator".into()],
                try_exec: None,
            }),
            argv: vec!["alacritty".into()],
        };
        assert_eq!(target.integration(), WalkerIntegration::TaskTerminal);
        assert!(target.opens_terminal_emulator());
    }

    #[test]
    fn chromium_desktop_opens_task_browser_not_raw_exec() {
        let target = LaunchTarget {
            desktop_id: Some("chromium".into()),
            desktop: Some(DesktopEntry {
                id: "chromium".into(),
                name: Some("Chromium".into()),
                exec: Some("/usr/bin/chromium %U".into()),
                terminal: false,
                categories: vec!["Network".into(), "WebBrowser".into()],
                try_exec: None,
            }),
            argv: vec!["/usr/bin/chromium".into()],
        };
        assert_eq!(target.integration(), WalkerIntegration::TaskBrowser);
        assert!(target.opens_browser_app());
    }

    #[test]
    fn cursor_desktop_opens_task_editor_not_raw_exec() {
        let target = LaunchTarget {
            desktop_id: Some("cursor".into()),
            desktop: Some(DesktopEntry {
                id: "cursor".into(),
                name: Some("Cursor".into()),
                exec: Some("cursor %F".into()),
                terminal: false,
                categories: vec!["Development".into(), "IDE".into()],
                try_exec: None,
            }),
            argv: vec!["cursor".into()],
        };
        assert_eq!(target.integration(), WalkerIntegration::TaskEditor);
        assert!(target.opens_editor_app());
    }

    #[test]
    fn runner_command_in_terminal_is_not_emulator_only() {
        let target = LaunchTarget {
            desktop_id: None,
            desktop: None,
            argv: vec!["bash".into(), "-lc".into(), "echo hi".into()],
        };
        assert!(!target.opens_terminal_emulator());
    }

    #[test]
    fn sanitize_launch_stderr_strips_uwsm_priority_prefixes() {
        let raw = "<3>Desktop entry has conflicting args: \"%u\", \"%U\"\n";
        assert_eq!(
            sanitize_launch_stderr(raw),
            "Desktop entry has conflicting args: \"%u\", \"%U\""
        );
    }

    #[test]
    fn launch_label_prefers_desktop_name() {
        let target = LaunchTarget {
            desktop_id: Some("todoist".into()),
            desktop: Some(DesktopEntry {
                id: "todoist".into(),
                name: Some("Todoist".into()),
                exec: Some("/usr/bin/todoist".into()),
                ..Default::default()
            }),
            argv: vec!["/usr/bin/todoist".into()],
        };
        assert_eq!(launch_label(&target, &target.argv), "Todoist");
    }

    #[test]
    fn launch_label_uses_command_when_no_desktop_name() {
        let argv = vec![
            "env".into(),
            "DESKTOPINTEGRATION=false".into(),
            "/usr/bin/todoist".into(),
            "--no-sandbox".into(),
        ];
        let target = LaunchTarget {
            desktop_id: None,
            desktop: None,
            argv: argv.clone(),
        };
        assert_eq!(launch_label(&target, &argv), "todoist --no-sandbox");
    }
}

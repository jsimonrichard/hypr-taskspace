//! Walker / Elephant launch integration — taskspace env and tsk launch routing.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::apps::{resolve_browser_command, resolve_editor_command, BROWSER_CANDIDATES, EDITOR_CANDIDATES};
use crate::config::load_config;
use crate::context_sync;
use crate::error::{Result, TskError};
use crate::models::{ContextMode, SessionState, Task};
use crate::registry::Registry;
use crate::service::TaskService;
use crate::task_env;
use crate::terminal;

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
    if let Some(uwsm) = command_v("uwsm") {
        let mut cmd = Command::new(&uwsm);
        cmd.arg("app");
        apply_launch_env(&mut cmd, ctx);
        cmd.current_dir(&ctx.cwd);
        push_uwsm_args(&mut cmd, target);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        return cmd
            .spawn()
            .map(|_| ())
            .map_err(|e| TskError::Other(format!("failed to launch via uwsm: {e}")));
    }

    let (program, program_args) = if target.argv.is_empty() {
        return Err(TskError::Other("walker exec: empty launch target".into()));
    } else {
        split_command(target.argv.iter().map(String::as_str).collect::<Vec<_>>().as_slice())?
    };
    launch_with_env(ctx, Some(&program), &program_args.iter().map(String::as_str).collect::<Vec<_>>())
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
    for line in raw.lines() {
        let line = line.trim();
        if line.starts_with("Exec=") {
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
}

//! Opt-in latency tracing (`TSK_TRACE=1`).
//!
//! Maintained debugging surface:
//! - `TSK_TRACE=1` — enable in CLI or Waybar process
//! - `TSK_TRACE_FILE` — override log path (default: `$XDG_RUNTIME_DIR/tsk/trace.log`)
//! - `tsk debug trace show|analyze|clear|path`
//! - `tsk debug trace workspace N` — switch workspace and print timeline
//! - `tsk debug hyprland-socket` — socket2 availability (also in `tsk doctor`)

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static ORIGIN: OnceLock<Instant> = OnceLock::new();
static FORCED: AtomicBool = AtomicBool::new(false);
static WRITER: Mutex<()> = Mutex::new(());

fn origin() -> Instant {
    *ORIGIN.get_or_init(Instant::now)
}

pub fn enabled() -> bool {
    if FORCED.load(Ordering::Relaxed) {
        return true;
    }
    static ENV: OnceLock<bool> = OnceLock::new();
    *ENV.get_or_init(|| {
        std::env::var("TSK_TRACE")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    })
}

/// Turn tracing on for the remainder of this process (`tsk debug trace workspace`).
pub fn enable_for_process() {
    FORCED.store(true, Ordering::Relaxed);
}

pub fn trace_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("TSK_TRACE_FILE") {
        return Some(std::path::PathBuf::from(path));
    }
    crate::xdg::tsk_runtime_dir()
        .ok()
        .map(|dir| dir.join("trace.log"))
}

pub fn clear_log() -> crate::error::Result<()> {
    let Some(path) = trace_path() else {
        return Ok(());
    };
    if path.is_file() {
        fs::remove_file(&path).map_err(|source| crate::error::TskError::Write {
            path: path.clone(),
            source,
        })?;
    }
    Ok(())
}

pub fn event(component: &str, stage: &str, detail: &str) {
    if !enabled() {
        return;
    }
    let Some(path) = trace_path() else {
        return;
    };
    let wall_ms = chrono::Utc::now().timestamp_millis();
    let mono_us = origin().elapsed().as_micros();
    let pid = std::process::id();
    let line = format!(
        "{wall_ms} {mono_us:>12} pid={pid} {component}.{stage} {detail}\n"
    );
    let _guard = WRITER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// RAII span — logs `<stage>_done` with elapsed ms on drop.
pub struct Span {
    component: &'static str,
    stage: &'static str,
    start: Instant,
}

impl Span {
    pub fn begin(component: &'static str, stage: &'static str, detail: &str) -> Self {
        event(component, stage, detail);
        Self {
            component,
            stage,
            start: Instant::now(),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let ms = self.start.elapsed().as_secs_f64() * 1000.0;
        event(
            self.component,
            &format!("{}_done", self.stage),
            &format!("{ms:.3}ms"),
        );
    }
}

#[derive(Debug, Clone)]
pub struct TraceLine {
    pub wall_ms: i64,
    pub mono_us: u128,
    pub pid: u32,
    pub component: String,
    pub stage: String,
    pub detail: String,
}

pub fn read_recent_lines(limit: usize) -> crate::error::Result<Vec<TraceLine>> {
    let Some(path) = trace_path() else {
        return Ok(Vec::new());
    };
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|source| crate::error::TskError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(raw
        .lines()
        .filter_map(parse_line)
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

fn parse_line(line: &str) -> Option<TraceLine> {
    let mut parts = line.split_whitespace();
    let wall_ms = parts.next()?.parse().ok()?;
    let mono_us = parts.next()?.parse().ok()?;
    let pid_part = parts.next()?;
    let pid = pid_part.strip_prefix("pid=")?.parse().ok()?;
    let key = parts.next()?;
    let (component, stage) = key.split_once('.')?;
    let detail = parts.collect::<Vec<_>>().join(" ");
    Some(TraceLine {
        wall_ms,
        mono_us,
        pid,
        component: component.to_string(),
        stage: stage.to_string(),
        detail,
    })
}

#[derive(Debug, Default)]
pub struct LatencyReport {
    pub lines: Vec<TraceLine>,
    pub stages: Vec<(String, f64)>,
    pub gaps: Vec<(String, f64)>,
    pub note: Option<String>,
}

/// Timeline for the most recent workspace switch (needs `TSK_TRACE=1` on Waybar).
pub fn analyze_recent_latency() -> LatencyReport {
    let lines = read_recent_lines(200).unwrap_or_default();
    if lines.is_empty() {
        return LatencyReport {
            note: Some(
                "No trace log. Set TSK_TRACE=1 for Waybar and run `tsk debug trace workspace N`."
                    .into(),
            ),
            ..Default::default()
        };
    }

    let anchor_idx = lines.iter().rposition(|l| {
        l.component == "cli" && l.stage == "hyprland" && l.detail.starts_with("dispatch")
    });

    let Some(start) = anchor_idx else {
        return LatencyReport {
            lines: lines.clone(),
            note: Some(
                "No recent workspace switch in log. Run `tsk debug trace workspace N` with \
                 TSK_TRACE=1 on Waybar."
                    .into(),
            ),
            ..Default::default()
        };
    };

    let window = &lines[start..];
    let base_wall = window.first().map(|l| l.wall_ms).unwrap_or(0);

    let mut ordered: Vec<&TraceLine> = window.iter().collect();
    ordered.sort_by_key(|l| (l.wall_ms, l.mono_us));

    let stages = ordered
        .iter()
        .map(|line| {
            (
                format!("{}.{}{}", line.component, line.stage, pid_tag(line.pid)),
                (line.wall_ms - base_wall) as f64,
            )
        })
        .collect();

    let markers = [
        ("cli.hyprland dispatch", "cli", "hyprland", "dispatch"),
        ("cli.hyprland dispatch_done", "cli", "hyprland_done", ""),
        ("waybar.socket workspacev2", "waybar", "socket", "workspacev2"),
        ("waybar.flip done", "waybar", "flip", "done"),
        ("waybar.sync poll", "waybar", "sync", "hyprctl"),
    ];

    let mut gaps = Vec::new();
    let mut last_label = None;
    let mut last_wall = base_wall;
    for (label, comp, stage, prefix) in markers {
        if let Some(line) = window.iter().find(|l| {
            l.component == comp
                && l.stage == stage
                && (prefix.is_empty() || l.detail.starts_with(prefix))
        }) {
            if let Some(prev) = last_label {
                gaps.push((
                    format!("{prev} → {label}"),
                    (line.wall_ms - last_wall) as f64,
                ));
            }
            last_label = Some(label);
            last_wall = line.wall_ms;
        }
    }

    LatencyReport {
        lines: window.to_vec(),
        stages,
        gaps,
        note: diagnostic_note(window),
    }
}

fn diagnostic_note(window: &[TraceLine]) -> Option<String> {
    let has_waybar = window.iter().any(|l| l.component == "waybar");
    if !has_waybar {
        return Some(
            "No waybar events — restart Waybar with TSK_TRACE=1.".into(),
        );
    }
    let has_socket = window.iter().any(|l| l.component == "waybar" && l.stage == "socket");
    let has_sync = window.iter().any(|l| l.component == "waybar" && l.stage == "sync");
    let has_flip = window.iter().any(|l| l.component == "waybar" && l.stage == "flip");
    if has_flip && !has_socket {
        return Some(
            "waybar.flip without waybar.socket — using hyprctl poll fallback; run \
             `tsk debug hyprland-socket`."
                .into(),
        );
    }
    if has_sync && !has_socket {
        return Some(
            "Updates via hyprctl poll, not socket2 — expect lag matching Waybar refresh interval."
                .into(),
        );
    }
    None
}

fn pid_tag(pid: u32) -> String {
    if pid == std::process::id() {
        String::new()
    } else {
        format!(" [pid={pid}]")
    }
}

pub fn format_report(report: &LatencyReport) -> String {
    let mut out = String::new();
    if let Some(note) = &report.note {
        out.push_str("Diagnosis: ");
        out.push_str(note);
        out.push_str("\n\n");
    }

    if report.stages.is_empty() {
        if report.lines.is_empty() {
            out.push_str("No trace log entries.\n");
        } else {
            out.push_str("Recent log lines:\n");
            for line in &report.lines {
                out.push_str(&format_line(line));
            }
        }
        return out;
    }

    out.push_str("Workspace switch timeline (ms from hyprctl dispatch, wall clock):\n\n");
    for (label, ms) in &report.stages {
        out.push_str(&format!("  {ms:8.1}  {label}\n"));
    }

    if !report.gaps.is_empty() {
        out.push_str("\nKey gaps:\n");
        for (label, delta) in &report.gaps {
            out.push_str(&format!("  {delta:8.1} ms  {label}\n"));
        }
    }

    out
}

fn format_line(line: &TraceLine) -> String {
    format!(
        "  {} {}.{}{} {}\n",
        line.wall_ms,
        line.component,
        line.stage,
        pid_tag(line.pid),
        line.detail
    )
}

pub fn tail_raw(limit: usize) -> crate::error::Result<String> {
    let lines = read_recent_lines(limit)?;
    Ok(lines.iter().map(format_line).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trace_line() {
        let line = parse_line(
            "1710000000000 123456 pid=9999 cli.workspace_go start index=3",
        )
        .unwrap();
        assert_eq!(line.component, "cli");
        assert_eq!(line.stage, "workspace_go");
        assert_eq!(line.pid, 9999);
    }
}

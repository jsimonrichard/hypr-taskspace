//! Distrobox container helpers — create/start/stop/remove/enter.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use crate::error::{TskError, Result};

pub fn available() -> bool {
    which::which("distrobox").is_ok()
}

pub fn require_available() -> Result<()> {
    if available() {
        Ok(())
    } else {
        Err(TskError::Other(
            "distrobox not found on PATH — install distrobox (and Podman/Docker) for container isolation"
                .into(),
        ))
    }
}

pub fn container_exists(name: &str) -> bool {
    if !available() || name.is_empty() {
        return false;
    }
    let Ok(output) = Command::new("distrobox")
        .args(["list", "--no-color"])
        .output()
    else {
        return false;
    };
    // Treat a failed list as "unknown / missing" — never as a false positive.
    if !output.status.success() {
        return false;
    }
    list_contains_name(&String::from_utf8_lossy(&output.stdout), name)
}

fn list_contains_name(list_stdout: &str, name: &str) -> bool {
    list_stdout.lines().skip(1).any(|line| {
        // `distrobox list` columns: ID | NAME | STATUS | IMAGE
        let cols: Vec<_> = line.split('|').map(str::trim).collect();
        match cols.as_slice() {
            [_, name_col, ..] => *name_col == name,
            _ => false,
        }
    })
}

/// Resolve the image used for `distrobox create`, migrating obsolete refs.
pub fn resolve_create_image(image: &str) -> Result<String> {
    let trimmed = image.trim();
    if trimmed.is_empty() {
        return Err(TskError::Other(
            "no Distrobox image configured — set [distrobox].image in ~/.config/tsk/config.toml \
             (example for Arch: quay.io/toolbx/arch-toolbox:latest)"
                .into(),
        ));
    }
    Ok(crate::host::migrate_stale_distrobox_image(trimmed)
        .unwrap_or_else(|| trimmed.to_string()))
}

/// Create a Distrobox with task home as `--home` (host paths still shared).
pub fn create_container(name: &str, home: &Path, image: &str) -> Result<()> {
    create_container_with_progress(name, home, image, |_| {})
}

/// Like [`create_container`], but invokes `on_line` for each stdout/stderr line.
///
/// After `distrobox create`, runs a first `distrobox enter … true` so Distrobox's
/// one-time guest setup ("Starting container…", packages, user, skel, …) finishes
/// during this call instead of on the user's first terminal/editor launch.
pub fn create_container_with_progress(
    name: &str,
    home: &Path,
    image: &str,
    mut on_line: impl FnMut(String),
) -> Result<()> {
    require_available()?;
    if container_exists(name) {
        on_line(format!("Container `{name}` already exists — ensuring it is initialized…"));
        return initialize_container_with_progress(name, on_line);
    }
    let image = resolve_create_image(image)?;

    std::fs::create_dir_all(home).map_err(|source| TskError::Write {
        path: home.to_path_buf(),
        source,
    })?;

    on_line(format!("Creating Distrobox `{name}`"));
    on_line(format!("Image: {image}"));
    on_line(format!("Home:  {}", home.display()));

    run_distrobox_streaming(
        &[
            "create",
            "-Y",
            "--name",
            name,
            "--image",
            &image,
            "--home",
            &home.display().to_string(),
            "--no-entry",
        ],
        &mut on_line,
        |stderr| format_create_failure(name, &image, stderr),
    )?;

    if !container_exists(name) {
        return Err(TskError::Other(format!(
            "Distrobox create `{name}` reported success but container is missing"
        )));
    }
    on_line(format!("Container `{name}` created — running first-enter setup…"));
    initialize_container_with_progress(name, on_line)
}

/// Run Distrobox's first-enter guest setup (or wake a stopped box), streaming output.
pub fn initialize_container_with_progress(
    name: &str,
    mut on_line: impl FnMut(String),
) -> Result<()> {
    require_available()?;
    if !container_exists(name) {
        return Err(TskError::Other(format!(
            "distrobox container `{name}` does not exist"
        )));
    }
    on_line(format!("Initializing Distrobox `{name}` (first enter)…"));
    run_distrobox_streaming(
        &["enter", "--name", name, "--no-tty", "--", "true"],
        &mut on_line,
        |stderr| {
            format!(
                "distrobox enter `{name}` failed: {}",
                first_meaningful_line(stderr).unwrap_or(stderr.trim())
            )
        },
    )?;
    on_line(format!("Distrobox `{name}` ready."));
    Ok(())
}

fn run_distrobox_streaming(
    args: &[&str],
    on_line: &mut impl FnMut(String),
    format_err: impl FnOnce(&str) -> String,
) -> Result<()> {
    let mut child = Command::new("distrobox")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TskError::Other(format!("distrobox {} failed: {e}", args.first().unwrap_or(&"?"))))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel::<String>();

    let readers = [
        spawn_pipe_reader(stdout, tx.clone()),
        spawn_pipe_reader(stderr, tx),
    ];

    let mut collected = Vec::new();
    while let Ok(line) = rx.recv() {
        collected.push(line.clone());
        on_line(line);
    }
    for handle in readers {
        let _ = handle.join();
    }

    let status = child
        .wait()
        .map_err(|e| TskError::Other(format!("distrobox wait failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format_err(&collected.join("\n"))))
    }
}

fn spawn_pipe_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    tx: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Some(pipe) = pipe else {
            return;
        };
        let reader = BufReader::new(pipe);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// Human-readable Distrobox create failure for CLI / TUI status lines.
pub fn format_create_failure(name: &str, image: &str, stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    let detail = first_meaningful_line(stderr).unwrap_or("unknown error");

    if lower.contains("unauthorized")
        || lower.contains("access to the requested resource is not authorized")
        || lower.contains("authentication required")
    {
        return format!(
            "Distrobox: no usable image (pull unauthorized for `{image}`). \
             Set [distrobox].image to a supported Distrobox image \
             (Arch: quay.io/toolbx/arch-toolbox:latest). See https://distrobox.it/compatibility/"
        );
    }
    if lower.contains("manifest unknown")
        || lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("no such image")
        || lower.contains("repository does not exist")
    {
        return format!(
            "Distrobox: image `{image}` was not found. \
             No matching supported image is available under that name — \
             set [distrobox].image to a Distrobox-compatible image for this host \
             (see https://distrobox.it/compatibility/). Detail: {detail}"
        );
    }
    if lower.contains("connection refused")
        || lower.contains("cannot connect")
        || lower.contains("is the docker daemon running")
        || (lower.contains("podman") && lower.contains("cannot"))
    {
        return format!(
            "Distrobox: container runtime unavailable while creating `{name}` \
             (need Docker or Podman running). Detail: {detail}"
        );
    }
    if lower.contains("network")
        || lower.contains("i/o timeout")
        || lower.contains("tls")
        || lower.contains("temporary failure")
        || lower.contains("no route to host")
        || lower.contains("name resolution")
    {
        return format!(
            "Distrobox: could not download image `{image}` (network error). Detail: {detail}"
        );
    }
    if lower.contains("permission denied") || lower.contains("permission_denied") {
        return format!(
            "Distrobox: permission denied creating `{name}` with image `{image}`. Detail: {detail}"
        );
    }

    format!("Distrobox create `{name}` failed (image `{image}`): {detail}")
}

fn first_meaningful_line(stderr: &str) -> Option<&str> {
    stderr.lines().map(str::trim).find(|line| {
        !line.is_empty()
            && !line.starts_with("Trying to pull")
            && !line.starts_with("Getting image")
    })
}

/// Wake a stopped container (and finish first-enter setup if needed).
/// Distrobox has no `start` subcommand — `enter` starts it.
pub fn start_container(name: &str) -> Result<()> {
    if !available() || !container_exists(name) {
        return Ok(());
    }
    initialize_container_with_progress(name, |_| {})
}

pub fn stop_container(name: &str) -> Result<()> {
    if !available() || !container_exists(name) {
        return Ok(());
    }
    // `distrobox stop` takes a positional name (no `--name`); `-Y` skips the prompt.
    let output = Command::new("distrobox")
        .args(["stop", "-Y", name])
        .output()
        .map_err(|e| TskError::Other(format!("distrobox stop failed: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format!(
            "distrobox stop {} failed: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub fn remove_container(name: &str) -> Result<()> {
    if !available() {
        return Ok(());
    }
    if !container_exists(name) {
        return Ok(());
    }
    // Stop first — `distrobox rm` fails on a running container.
    let _ = stop_container(name);
    // `distrobox rm` takes a positional name; force with `-f` (not `-Y` / `--name`).
    let output = Command::new("distrobox")
        .args(["rm", "-f", name])
        .output()
        .map_err(|e| TskError::Other(format!("distrobox rm failed: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TskError::Other(format!(
            "distrobox rm {} failed: {}",
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Argv prefix: `distrobox enter --name <name> --`.
pub fn enter_prefix(name: &str) -> Vec<String> {
    vec![
        "distrobox".into(),
        "enter".into(),
        "--name".into(),
        name.into(),
        "--".into(),
    ]
}

/// Run a command inside the container (no TTY).
pub fn run_in_container(name: &str, program: &str, args: &[&str]) -> Result<std::process::Child> {
    require_available()?;
    if !container_exists(name) {
        return Err(TskError::Other(format!(
            "distrobox container `{name}` does not exist"
        )));
    }
    let _ = start_container(name);
    let mut cmd = Command::new("distrobox");
    cmd.args(["enter", "--name", name, "--no-tty", "--", program]);
    cmd.args(args);
    cmd.spawn()
        .map_err(|e| TskError::Other(format!("failed to enter distrobox `{name}`: {e}")))
}

/// Build a login shell that cds into `workdir` inside the container.
pub fn shell_enter_argv(name: &str, workdir: &Path) -> Vec<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let mut argv = enter_prefix(name);
    // Use the host absolute path — Distrobox shares the host filesystem.
    let script = format!(
        "cd {} && exec {}",
        shell_quote(&workdir.display().to_string()),
        shell_quote(&shell)
    );
    argv.push("bash".into());
    argv.push("-lc".into());
    argv.push(script);
    argv
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

mod which {
    use std::path::PathBuf;

    pub fn which(name: &str) -> Result<PathBuf, ()> {
        std::env::split_paths(&std::env::var_os("PATH").ok_or(())?)
            .find_map(|dir| {
                let path = dir.join(name);
                path.is_file().then_some(path)
            })
            .ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_contains_name_parses_pipe_columns() {
        let list = "\
ID           | NAME                 | STATUS             | IMAGE
7c612a060b21 | tsk-dev-te80ddb81    | Up 20 minutes      | quay.io/toolbx/arch-toolbox:latest
";
        assert!(list_contains_name(list, "tsk-dev-te80ddb81"));
        assert!(!list_contains_name(list, "tsk-dev"));
        assert!(!list_contains_name(list, "missing"));
    }

    #[test]
    fn enter_prefix_shape() {
        assert_eq!(
            enter_prefix("tsk-t1"),
            vec!["distrobox", "enter", "--name", "tsk-t1", "--"]
        );
    }

    #[test]
    fn shell_enter_includes_workdir() {
        let argv = shell_enter_argv("tsk-t1", Path::new("/tmp/repo"));
        assert_eq!(argv[0], "distrobox");
        assert!(argv.last().unwrap().contains("/tmp/repo"));
    }

    #[test]
    fn create_failure_unauthorized_mentions_no_usable_image() {
        let msg = format_create_failure(
            "tsk-t1",
            "quay.io/toolbx-images/arch-toolbox:latest",
            "Error response from daemon: unauthorized: access to the requested resource is not authorized",
        );
        assert!(msg.contains("no usable image"), "{msg}");
        assert!(msg.contains("[distrobox].image"), "{msg}");
    }

    #[test]
    fn create_failure_manifest_unknown_mentions_not_found() {
        let msg = format_create_failure(
            "tsk-t1",
            "quay.io/toolbx/arch-toolbox:missing",
            "Error response from daemon: manifest for quay.io/toolbx/arch-toolbox:missing not found: manifest unknown",
        );
        assert!(msg.contains("was not found"), "{msg}");
        assert!(msg.contains("No matching supported image"), "{msg}");
    }

    #[test]
    fn resolve_create_image_rejects_empty() {
        assert!(resolve_create_image("").is_err());
        assert!(resolve_create_image("   ").is_err());
    }

    #[test]
    fn resolve_create_image_migrates_stale_ref() {
        let image = resolve_create_image("quay.io/toolbx-images/arch-toolbox:latest").unwrap();
        assert_eq!(image, "quay.io/toolbx/arch-toolbox:latest");
    }
}

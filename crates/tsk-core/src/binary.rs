//! Installed TSK binary resolution and login-shell PATH lookup.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::TskConfig;
use crate::dev_session::{dev_session_active, dev_session_binary};
use crate::error::{Result, TskError};

/// Active dev session, then `TSK` env (when the file exists), then `PATH`.
pub fn resolve_tsk_binary(_cfg: &TskConfig) -> PathBuf {
    if dev_session_active() {
        if let Some(bin) = dev_session_binary() {
            return bin;
        }
    }
    if let Ok(path) = std::env::var("TSK") {
        let path = PathBuf::from(path);
        if path.is_file() {
            if let Some(real) = peel_tsk_wrapper(&path) {
                return real;
            }
            return path;
        }
        // Stale override — fall through to PATH / login shell.
    }
    if let Some(path) = path_tsk_binary() {
        return path;
    }
    PathBuf::from("tsk")
}

/// Rust binary to exec in a new terminal — never the dev share-tree bash wrapper.
///
/// When called from the Waybar CFFI module, `current_exe()` is the Waybar binary — not `tsk`.
pub fn resolve_tsk_spawn_binary(cfg: &TskConfig) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if is_elf_executable(&exe) && is_tsk_cli_executable(&exe) {
            return exe;
        }
    }
    let path = resolve_tsk_binary(cfg);
    peel_tsk_wrapper(&path).unwrap_or(path)
}

fn is_tsk_cli_executable(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name == "tsk")
}

/// Hyprland `$tsk` wrapper script → real executable path.
pub fn peel_tsk_wrapper(path: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(path).ok()?;
    if !content.starts_with("#!") {
        return None;
    }
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("exec \"") else {
            continue;
        };
        let end = rest.find('"')?;
        let target = PathBuf::from(&rest[..end]);
        if target.is_file() {
            return Some(target);
        }
    }
    None
}

fn is_elf_executable(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == [0x7f, b'E', b'L', b'F']
}

/// Command name or absolute path suitable for Hyprland `exec`.
///
/// Always resolves prod `tsk` on PATH. When a dev session file is present, that
/// binary reads `~/.local/share/tsk/dev-session` and switches to dev config itself.
pub fn resolve_tsk_command(_cfg: &TskConfig) -> String {
    if crate::share::system_share_available() {
        return "/usr/bin/tsk".into();
    }
    command_v_login("tsk")
        .unwrap_or_else(|| resolve_tsk_binary(_cfg).to_string_lossy().into_owned())
}

/// When a dev session file is present, re-exec the recorded dev binary so Hyprland
/// and helpers can keep calling prod `tsk` on PATH without a compositor reload.
#[cfg(unix)]
pub fn maybe_reexec_dev_session() -> Result<()> {
    use std::os::unix::process::CommandExt;

    let Some(dev_bin) = dev_session_binary() else {
        return Ok(());
    };
    let current = std::env::current_exe()
        .map_err(|source| crate::error::TskError::Other(format!("current_exe: {source}")))?;
    let same = current
        .canonicalize()
        .ok()
        .zip(dev_bin.canonicalize().ok())
        .is_some_and(|(a, b)| a == b);
    if same {
        return Ok(());
    }
    Err(crate::error::TskError::Other(format!(
        "failed to re-exec dev tsk ({}): {}",
        dev_bin.display(),
        std::process::Command::new(&dev_bin)
            .args(std::env::args().skip(1))
            .exec(),
    )))
}

#[cfg(not(unix))]
pub fn maybe_reexec_dev_session() -> Result<()> {
    Ok(())
}

/// First `tsk` executable found by walking `PATH`.
pub fn path_tsk_binary() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("tsk");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

/// Resolve a command through the user's login shell — Hyprland exec often has a stripped PATH.
pub fn command_v_login(name: &str) -> Option<String> {
    let script = format!("command -v {}", shell_quote(name));
    let output = Command::new("sh").arg("-lc").arg(script).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// Resolve an explicit program from a desktop Exec line or desktop id.
/// Absolute/relative paths that exist are used as-is; otherwise login-shell PATH.
pub fn resolve_named_command(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if (name.contains('/') || name.starts_with('.')) && Path::new(name).is_file() {
        return Some(name.to_string());
    }
    command_v_login(name)
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// True when `tsk` is available on PATH or via a valid `TSK` override.
pub fn path_tsk_is_usable(_cfg: &TskConfig) -> (bool, String) {
    if dev_session_active() {
        if let Some(bin) = dev_session_binary() {
            return (true, format!("{} (dev enter session)", bin.display()));
        }
    }
    if let Ok(path) = std::env::var("TSK") {
        let path = PathBuf::from(&path);
        if path.is_file() {
            return (true, format!("TSK={}", path.display()));
        }
        if let Some(found) = path_tsk_binary() {
            if let Some(detail) = packaged_path_mismatch(&found) {
                return (false, detail);
            }
            return (
                true,
                format!(
                    "{} (TSK={} missing — unset TSK or fix your shell config)",
                    found.display(),
                    path.display()
                ),
            );
        }
        if let Some(login) = command_v_login("tsk") {
            let found = PathBuf::from(&login);
            if let Some(detail) = packaged_path_mismatch(&found) {
                return (false, detail);
            }
            return (
                true,
                format!(
                    "{login} (TSK={} missing — unset TSK or fix your shell config)",
                    path.display()
                ),
            );
        }
        return (
            false,
            format!(
                "TSK={} is not an executable file and no tsk on PATH",
                path.display()
            ),
        );
    }

    if let Some(found) = path_tsk_binary() {
        if let Some(detail) = packaged_path_mismatch(&found) {
            return (false, detail);
        }
        return (true, found.display().to_string());
    }

    if let Some(login) = command_v_login("tsk") {
        let found = PathBuf::from(&login);
        if let Some(detail) = packaged_path_mismatch(&found) {
            return (false, detail);
        }
        return (true, login);
    }

    (
        false,
        "no tsk on PATH — install the package or put tsk on PATH".into(),
    )
}

const PACKAGED_TSK: &str = "/usr/bin/tsk";

fn packaged_path_mismatch(found: &Path) -> Option<String> {
    if !crate::share::system_share_available() {
        return None;
    }
    let packaged = Path::new(PACKAGED_TSK);
    if !packaged.is_file() {
        return None;
    }
    if same_executable(found, packaged) {
        return None;
    }
    Some(format!(
        "{} is first on PATH; expected {PACKAGED_TSK}",
        found.display()
    ))
}

fn same_executable(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => a == b,
    }
}

/// Extra `tsk` copies that sit earlier on PATH than `/usr/bin/tsk`.
pub fn packaged_path_shadow_candidates() -> Vec<PathBuf> {
    vec![
        crate::xdg::expand("~/.cargo/bin/tsk"),
        crate::xdg::expand("~/.local/bin/tsk"),
        crate::xdg::expand("~/.local/share/tsk/bin/tsk"),
    ]
}

/// When the distro package is installed, delete extra `tsk` binaries that would
/// shadow `/usr/bin/tsk` (cargo install, leftover share-tree copies).
pub fn remove_packaged_path_shadows() -> Result<Vec<String>> {
    if !crate::share::system_share_available() {
        return Ok(Vec::new());
    }
    let packaged = Path::new(PACKAGED_TSK);
    if !packaged.is_file() {
        return Ok(Vec::new());
    }
    remove_shadowing_tsk_copies(packaged, &packaged_path_shadow_candidates())
}

fn remove_shadowing_tsk_copies(packaged: &Path, candidates: &[PathBuf]) -> Result<Vec<String>> {
    let mut actions = Vec::new();
    for path in candidates {
        if !path.exists() {
            continue;
        }
        if same_executable(path, packaged) {
            continue;
        }
        fs::remove_file(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::PermissionDenied {
                TskError::Other(format!(
                    "could not remove {} (permission denied) — run: sudo rm {}",
                    path.display(),
                    path.display()
                ))
            } else {
                TskError::Write {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        actions.push(format!("removed PATH shadow {}", path.display()));
    }
    Ok(actions)
}

/// Legacy share-tree path (helpers only — not the main CLI).
pub fn legacy_share_tsk_binary(cfg: &TskConfig) -> PathBuf {
    cfg.install_hypr_share_dir.join("bin/tsk")
}

/// Waybar CFFI module next to the installed share tree.
pub fn waybar_module_path(cfg: &TskConfig) -> PathBuf {
    crate::share::effective_share_dir(cfg).join("lib/libtsk_waybar.so")
}

/// True when `path` looks like a non-empty shared library (empty files fail Waybar silently).
pub fn is_usable_cdylib(path: &Path) -> bool {
    path.is_file() && fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false)
}

/// FHS-style module path relative to an installed `tsk` binary (`../lib/libtsk_waybar.so`).
pub fn waybar_module_beside_binary(tsk_bin: &Path) -> PathBuf {
    tsk_bin
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("../lib/libtsk_waybar.so")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn resolve_tsk_command_uses_prod_path_when_session_active() {
        let _lock = env_lock();
        let mut cfg = TskConfig::default();
        cfg.install_hypr_share_dir = crate::install::profile::dev_share_dir();
        cfg.container_prefix = "tsk-dev".into();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let share = home.join(".local/share/tsk");
        std::fs::create_dir_all(&share).unwrap();
        let bin = share.join("dev-build");
        std::fs::write(&bin, b"").unwrap();
        std::env::set_var("HOME", home);
        crate::dev_session::start_dev_session(&bin).unwrap();
        let cmd = resolve_tsk_command(&cfg);
        assert!(
            cmd == "tsk" || cmd.ends_with("/tsk"),
            "expected prod CLI path, got {cmd}"
        );
        crate::dev_session::stop_dev_session().ok();
    }

    #[test]
    fn peel_tsk_wrapper_reads_exec_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("tsk-real");
        fs::write(&real, b"").unwrap();
        let wrapper = dir.path().join("tsk");
        fs::write(
            &wrapper,
            format!("#!/usr/bin/env bash\nexec \"{}\" \"$@\"\n", real.display()),
        )
        .unwrap();
        assert_eq!(peel_tsk_wrapper(&wrapper), Some(real));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn is_tsk_cli_executable_recognizes_tsk_only() {
        assert!(is_tsk_cli_executable(Path::new("/usr/bin/tsk")));
        assert!(!is_tsk_cli_executable(Path::new("/usr/bin/waybar")));
    }

    #[test]
    fn resolve_tsk_spawn_binary_ignores_non_tsk_current_exe() {
        let _lock = env_lock();
        crate::dev_session::stop_dev_session().ok();
        let dir = tempfile::tempdir().unwrap();
        let tsk = dir.path().join("tsk");
        fs::write(&tsk, b"").unwrap();

        let old_path = std::env::var_os("PATH");
        std::env::set_var("PATH", dir.path());
        std::env::remove_var("TSK");

        // Test runner's current_exe is not named "tsk" — must fall back to PATH.
        assert!(!is_tsk_cli_executable(&std::env::current_exe().unwrap()));
        let resolved = resolve_tsk_spawn_binary(&TskConfig::default());
        assert_eq!(resolved, tsk);

        if let Some(p) = old_path {
            std::env::set_var("PATH", p);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn remove_shadowing_tsk_copies_deletes_distinct_files() {
        let dir = tempfile::tempdir().unwrap();
        let packaged = dir.path().join("usr-bin-tsk");
        let cargo = dir.path().join("cargo-tsk");
        let alias = dir.path().join("alias-tsk");
        fs::write(&packaged, b"packaged").unwrap();
        fs::write(&cargo, b"stale cargo").unwrap();
        std::os::unix::fs::symlink(&packaged, &alias).unwrap();
        let actions =
            remove_shadowing_tsk_copies(&packaged, &[cargo.clone(), alias.clone()]).unwrap();
        assert_eq!(
            actions,
            vec![format!("removed PATH shadow {}", cargo.display())]
        );
        assert!(!cargo.exists());
        assert!(alias.exists());
        assert!(packaged.exists());
    }

    #[test]
    fn resolve_named_command_uses_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("code");
        fs::write(&bin, b"").unwrap();
        assert_eq!(
            resolve_named_command(&bin.to_string_lossy()),
            Some(bin.to_string_lossy().into_owned())
        );
        assert!(resolve_named_command("/no/such/editor-bin").is_none());
        assert!(resolve_named_command("").is_none());
    }
}

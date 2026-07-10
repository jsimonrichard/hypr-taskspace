//! Install tsk binaries and share templates (no Hyprland or Waybar config edits).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::binary::{resolve_tsk_binary, resolve_tsk_command, waybar_module_beside_binary, waybar_module_path};
use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::install::profile::{dev_config_path, is_dev_config, InstallProfile, profile_for_config};
use crate::install::reload;
use crate::share::{effective_share_dir, uses_packaged_share};
use crate::xdg::{ensure_parent, user_bin_dir};
use crate::task_env::{ensure_task_bin_dir, task_bin_dir};

const LIB_NAME: &str = "libtsk_waybar.so";
const TSK_SHARE_PLACEHOLDER: &str = "@TSK_SHARE@";
const TSK_CMD_PLACEHOLDER: &str = "@TSK_CMD@";

#[derive(Debug, Clone)]
pub struct InstallBinsOptions {
    pub dry_run: bool,
    pub workspace_root: Option<PathBuf>,
    pub profile: Option<InstallProfile>,
    /// Prepend Omarchy unbind sources to installed bindings (prod Omarchy preset).
    pub omarchy_integration: bool,
    pub skip_waybar: bool,
    /// Skip Hyprland/Waybar reload (caller will apply once at the end).
    pub skip_reload: bool,
    /// Suppress progress messages (scripts/dev.sh --quiet).
    pub quiet: bool,
    /// Path to a pre-built Waybar cdylib (from `cargo` artifact deps / `cargo install` build).
    pub bundled_waybar_source: Option<PathBuf>,
}

impl Default for InstallBinsOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            workspace_root: None,
            profile: None,
            omarchy_integration: false,
            skip_waybar: false,
            skip_reload: false,
            quiet: false,
            bundled_waybar_source: None,
        }
    }
}

pub fn install_bins(cfg: &TskConfig, options: &InstallBinsOptions) -> Result<Vec<String>> {
    let profile = options.profile.unwrap_or_else(|| profile_for_config(cfg));
    let share_dir = effective_share_dir(cfg);
    let system_share = uses_packaged_share(cfg);
    let deploy_user_share =
        should_deploy_user_share(cfg, system_share, options.omarchy_integration);
    let share_src = resolve_share_templates(options.workspace_root.as_deref(), profile)?;
    let tsk_cmd = resolve_tsk_command(cfg);

    if options.dry_run {
        return Ok(vec![
            format!("would verify tsk on PATH ({tsk_cmd})"),
            if deploy_user_share {
                format!(
                    "would deploy share templates from {} → {}",
                    share_src.display(),
                    cfg.install_hypr_share_dir.display()
                )
            } else if system_share {
                format!("would use system share at {}", share_dir.display())
            } else {
                format!(
                    "would copy share templates from {} → {}",
                    share_src.display(),
                    cfg.install_hypr_share_dir.display()
                )
            },
            if options.skip_waybar {
                "would skip waybar module".into()
            } else if system_share {
                format!(
                    "would verify Waybar module at {}",
                    waybar_module_path(cfg).display()
                )
            } else {
                format!(
                    "would install Waybar module at {}",
                    waybar_module_path(cfg).display()
                )
            },
            if options.omarchy_integration {
                "would include Omarchy unbind integration in bindings".into()
            } else {
                String::new()
            },
            format!(
                "would install taskspace xdg-open wrapper at {}",
                task_bin_dir(cfg).join("xdg-open").display()
            ),
            "would reload Hyprland config".into(),
            if options.skip_waybar {
                String::new()
            } else {
                "would restart Waybar".into()
            },
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect());
    }

    let (path_ok, path_detail) = crate::binary::path_tsk_is_usable(cfg);
    if !path_ok {
        return Err(TskError::Other(format!(
            "tsk not found on PATH ({path_detail}) — install the CLI first (e.g. cargo install --path crates/tsk-cli or your package manager)"
        )));
    }

    if deploy_user_share {
        let template_src = share_src.clone();
        let tsk_bin = resolve_tsk_binary(cfg);
        copy_share_tree(
            cfg,
            &template_src,
            profile,
            options.omarchy_integration,
            &resolve_tsk_command(cfg),
        )?;
        if !system_share {
            install_tsk_wrapper(cfg, &tsk_bin)?;
        }
        if !options.skip_waybar {
            if system_share {
                verify_system_share_for_waybar(cfg)?;
            } else {
                install_waybar_module(cfg, options)?;
            }
        }
    } else {
        verify_system_share(cfg, options)?;
    }
    install_xdg_open_wrapper(&share_src, &resolve_tsk_command(cfg), cfg)?;
    remove_legacy_global_xdg_open_wrapper()?;

    let mut actions = vec![
        if deploy_user_share {
            format!(
                "deployed share data to {}",
                cfg.install_hypr_share_dir.display()
            )
        } else if system_share {
            format!("using system share at {}", share_dir.display())
        } else {
            format!(
                "installed share data to {}",
                cfg.install_hypr_share_dir.display()
            )
        },
        format!("using tsk at {path_detail}"),
        format!(
            "installed taskspace xdg-open wrapper at {}",
            task_bin_dir(cfg).join("xdg-open").display()
        ),
        format!("runtime data in {}", cfg.data_dir.display()),
    ];
    if !options.skip_reload {
        actions.extend(reload::apply_after_hypr()?);
        if !options.skip_waybar {
            actions.extend(reload::apply_after_waybar());
        }
    }
    Ok(actions)
}

fn verify_system_share(cfg: &TskConfig, options: &InstallBinsOptions) -> Result<()> {
    let share_dir = effective_share_dir(cfg);
    let bindings = share_dir.join("hypr/bindings.conf");
    if !bindings.is_file() {
        return Err(TskError::Other(format!(
            "system share incomplete (missing {}) — reinstall the hypr-taskspace package",
            bindings.display()
        )));
    }
    if !options.skip_waybar {
        verify_system_share_for_waybar(cfg)?;
    }
    Ok(())
}

pub fn verify_system_share_for_waybar(cfg: &TskConfig) -> Result<()> {
    let module = waybar_module_path(cfg);
    if !crate::binary::is_usable_cdylib(&module) {
        return Err(TskError::Other(format!(
            "Waybar module missing or empty at {} — reinstall the hypr-taskspace package",
            module.display()
        )));
    }
    Ok(())
}

pub fn install_waybar_module(cfg: &TskConfig, options: &InstallBinsOptions) -> Result<PathBuf> {
    let dest = waybar_module_path(cfg);
    ensure_parent(&dest)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|source| TskError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source = locate_waybar_cdylib(cfg, options)?;
    fs::copy(&source, &dest).map_err(|source_err| TskError::Write {
        path: dest.clone(),
        source: source_err,
    })?;
    if !options.quiet {
        eprintln!(
            "installed Waybar module: {} (from {})",
            dest.display(),
            source.display()
        );
    }
    Ok(dest)
}

fn locate_waybar_cdylib(cfg: &TskConfig, options: &InstallBinsOptions) -> Result<PathBuf> {
    if let Some(path) = options.bundled_waybar_source.as_ref() {
        if crate::binary::is_usable_cdylib(path) {
            return Ok(path.clone());
        }
    }

    // Dev / cargo-from-source: workspace build wins over the distro package.
    if !uses_packaged_share(cfg) {
        if let Some(path) = find_workspace_waybar_cdylib(options.workspace_root.as_deref())? {
            return Ok(path);
        }

        if let Some(path) = waybar_module_beside_binary(&crate::binary::resolve_tsk_binary(cfg))
            .canonicalize()
            .ok()
            .filter(|p| crate::binary::is_usable_cdylib(p))
        {
            return Ok(path);
        }
    } else {
        let packaged = crate::share::system_waybar_module_path();
        if crate::binary::is_usable_cdylib(&packaged) {
            return Ok(packaged);
        }
    }

    Err(TskError::Other(
        "could not find libtsk_waybar.so — build from the repo (cargo build -p tsk-waybar --release) \
         or install via a package that ships the module"
            .into(),
    ))
}

fn find_workspace_waybar_cdylib(workspace_root: Option<&Path>) -> Result<Option<PathBuf>> {
    let workspace = workspace_root
        .map(Path::to_path_buf)
        .or_else(find_workspace_root)
        .ok_or_else(|| {
            TskError::Other(
                "could not find Cargo workspace — set TSK_WORKSPACE or run from the repo".into(),
            )
        })?;

    let target_dir = workspace.join("target");
    let release_so = target_dir.join("release").join(LIB_NAME);
    if crate::binary::is_usable_cdylib(&release_so) {
        return Ok(Some(release_so));
    }

    eprintln!("building tsk-waybar (release)...");
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "tsk-waybar",
            "--release",
            "--target-dir",
        ])
        .arg(&target_dir)
        .current_dir(&workspace)
        .status()
        .map_err(|e| TskError::Other(format!("failed to run cargo: {e}")))?;
    if !status.success() {
        return Err(TskError::Other("cargo build -p tsk-waybar failed".into()));
    }

    if crate::binary::is_usable_cdylib(&release_so) {
        Ok(Some(release_so))
    } else {
        Ok(None)
    }
}

fn install_tsk_wrapper(cfg: &TskConfig, tsk_bin: &Path) -> Result<()> {
    if !is_dev_config(cfg) {
        return Ok(());
    }
    let wrapper = cfg.install_hypr_share_dir.join("bin/tsk");
    ensure_parent(&wrapper)?;
    let config_path = dev_config_path().to_string_lossy().into_owned();
    let exec_bin = crate::dev_session::dev_session_binary().unwrap_or_else(|| tsk_bin.to_path_buf());
    let marker = crate::dev_session::dev_session_marker_path()
        .to_string_lossy()
        .into_owned();
    let body = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n\
MARKER=\"{marker}\"\n\
if [[ ! -f \"$MARKER\" ]]; then\n\
  PROD=\"$(sh -lc 'command -v tsk' 2>/dev/null || true)\"\n\
  if [[ -z \"$PROD\" && -x /usr/bin/tsk ]]; then PROD=/usr/bin/tsk; fi\n\
  if [[ -n \"$PROD\" ]]; then exec \"$PROD\" \"$@\"; fi\n\
  echo \"tsk: dev session inactive and prod CLI not found\" >&2\n\
  exit 1\n\
fi\n\
export TSK_CONFIG=\"{config_path}\"\n\
exec \"{}\" \"$@\"\n",
        exec_bin.display()
    );
    fs::write(&wrapper, &body).map_err(|source| TskError::Write {
        path: wrapper.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrapper)
            .map_err(|source| TskError::Read {
                path: wrapper.clone(),
                source,
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper, perms).map_err(|source| TskError::Write {
            path: wrapper,
            source,
        })?;
    }
    Ok(())
}

fn install_xdg_open_wrapper(share_src: &Path, tsk_cmd: &str, cfg: &TskConfig) -> Result<()> {
    let mut src = share_src.join("bin/xdg-open");
    if !src.is_file() {
        src = effective_share_dir(cfg).join("bin/xdg-open");
    }
    if !src.is_file() {
        return Ok(());
    }
    let dest = ensure_task_bin_dir(cfg)?.join("xdg-open");
    ensure_parent(&dest)?;
    let raw = fs::read_to_string(&src).map_err(|source| TskError::Read {
        path: src.clone(),
        source,
    })?;
    let body = raw.replace(TSK_CMD_PLACEHOLDER, tsk_cmd);
    fs::write(&dest, &body).map_err(|source| TskError::Write {
        path: dest.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)
            .map_err(|source| TskError::Read {
                path: dest.clone(),
                source,
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms).map_err(|source| TskError::Write {
            path: dest,
            source,
        })?;
    }
    Ok(())
}

/// Drop a legacy `~/.local/bin/xdg-open` installed by older tsk versions.
fn remove_legacy_global_xdg_open_wrapper() -> Result<()> {
    let legacy = user_bin_dir().join("xdg-open");
    if !legacy.is_file() {
        return Ok(());
    }
    let Ok(raw) = fs::read_to_string(&legacy) else {
        return Ok(());
    };
    if raw.contains("Taskspace xdg-open") || raw.contains("Taskspace-aware xdg-open") || raw.contains("tsk open") {
        fs::remove_file(&legacy).map_err(|source| TskError::Write {
            path: legacy,
            source,
        })?;
    }
    Ok(())
}

fn copy_share_tree(
    cfg: &TskConfig,
    share_src: &Path,
    profile: InstallProfile,
    omarchy_integration: bool,
    tsk_cmd: &str,
) -> Result<()> {
    let share_dir = &cfg.install_hypr_share_dir;
    let share_str = share_dir.to_string_lossy();

    copy_hypr_tree(
        &share_src.join("hypr"),
        &share_dir.join("hypr"),
        &share_str,
        profile,
        omarchy_integration,
        tsk_cmd,
    )?;
    copy_dir_files_flat(&share_src.join("waybar"), &share_dir.join("waybar"), &share_str)?;
    Ok(())
}

fn copy_hypr_tree(
    src: &Path,
    dest: &Path,
    share_str: &str,
    profile: InstallProfile,
    omarchy_integration: bool,
    tsk_cmd: &str,
) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    ensure_parent(&dest.join("_"))?;
    fs::create_dir_all(dest).map_err(|source| TskError::Write {
        path: dest.to_path_buf(),
        source,
    })?;

    for entry in fs::read_dir(src).map_err(|source| TskError::Read {
        path: src.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: src.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            copy_hypr_tree(
                &path,
                &dest.join(path.file_name().unwrap()),
                share_str,
                profile,
                omarchy_integration,
                tsk_cmd,
            )?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let raw = fs::read_to_string(&path).map_err(|source| TskError::Read {
            path: path.clone(),
            source,
        })?;
        let body = if name == "bindings.conf" {
            let mut out = substitute_share(&raw, share_str);
            out = out.replace(TSK_CMD_PLACEHOLDER, tsk_cmd);
            if profile.include_omarchy_unbinds_for(omarchy_integration) {
                let unbinds = format!(
                    "source = {}/hypr/integrations/omarchy-unbind.conf\n\n",
                    share_str
                );
                let escape_hatch = format!(
                    "\nsource = {}/hypr/integrations/omarchy-escape-hatch.conf\n",
                    share_str
                );
                out = format!("{unbinds}{out}{escape_hatch}");
            }
            remap_packaged_share_paths(&out, share_str)
        } else if raw.contains(TSK_SHARE_PLACEHOLDER) || raw.contains(TSK_CMD_PLACEHOLDER) {
            remap_packaged_share_paths(
                &substitute_share(&raw, share_str).replace(TSK_CMD_PLACEHOLDER, tsk_cmd),
                share_str,
            )
        } else {
            remap_packaged_share_paths(&raw, share_str)
        };
        let target = dest.join(path.file_name().unwrap());
        ensure_parent(&target)?;
        fs::write(&target, body).map_err(|source| TskError::Write {
            path: target,
            source,
        })?;
    }
    Ok(())
}

fn copy_dir_files_flat(src: &Path, dest: &Path, share_str: &str) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    ensure_parent(&dest.join("_"))?;
    fs::create_dir_all(dest).map_err(|source| TskError::Write {
        path: dest.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(src).map_err(|source| TskError::Read {
        path: src.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| TskError::Read {
            path: src.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|source| TskError::Read {
            path: path.clone(),
            source,
        })?;
        let body = if raw.contains(TSK_SHARE_PLACEHOLDER) {
            substitute_share(&raw, share_str)
        } else {
            raw
        };
        let target = dest.join(path.file_name().unwrap());
        fs::write(&target, body).map_err(|source| TskError::Write {
            path: target,
            source,
        })?;
    }
    Ok(())
}

fn substitute_share(content: &str, share_dir: &str) -> String {
    content.replace(TSK_SHARE_PLACEHOLDER, share_dir)
}

fn remap_packaged_share_paths(content: &str, share_dir: &str) -> String {
    if share_dir == crate::share::SYSTEM_SHARE_DIR {
        return content.to_string();
    }
    content.replace(crate::share::SYSTEM_SHARE_DIR, share_dir)
}

/// Deploy user-local Hypr/Waybar templates even when the pacman share tree exists.
fn should_deploy_user_share(
    cfg: &TskConfig,
    system_share: bool,
    omarchy_integration: bool,
) -> bool {
    !system_share
        || omarchy_integration
        || cfg.install_hypr_share_dir != effective_share_dir(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TskConfig;
    use crate::install::profile::InstallProfile;
    use crate::xdg::expand;

    #[test]
    fn should_deploy_user_share_for_omarchy_on_packaged_install() {
        if !crate::share::system_share_available() {
            return;
        }
        let mut cfg = TskConfig::default();
        cfg.install_hypr_share_dir = expand("~/.local/share/tsk");
        assert!(should_deploy_user_share(&cfg, true, true));
    }

    #[test]
    fn should_deploy_user_share_when_configured_share_dir_differs() {
        if !crate::share::system_share_available() {
            return;
        }
        let mut cfg = TskConfig::default();
        cfg.install_hypr_share_dir = expand("~/.local/share/tsk");
        assert!(should_deploy_user_share(&cfg, true, false));
    }

    #[test]
    fn bindings_include_omarchy_unbind_source_when_requested() {
        let raw = "source = @TSK_SHARE@/hypr/window-rules.conf\n$tsk = @TSK_CMD@\n";
        let share = "/home/u/.local/share/tsk";
        let mut out = substitute_share(raw, share);
        out = out.replace(TSK_CMD_PLACEHOLDER, "/usr/bin/tsk");
        if InstallProfile::Prod.include_omarchy_unbinds_for(true) {
            let unbinds = format!("source = {share}/hypr/integrations/omarchy-unbind.conf\n\n");
            let escape_hatch =
                format!("\nsource = {share}/hypr/integrations/omarchy-escape-hatch.conf\n");
            out = format!("{unbinds}{out}{escape_hatch}");
        }
        assert!(out.starts_with("source = /home/u/.local/share/tsk/hypr/integrations/omarchy-unbind.conf"));
        assert!(out.contains("source = /home/u/.local/share/tsk/hypr/integrations/omarchy-escape-hatch.conf"));
    }

    #[test]
    fn remap_packaged_share_paths_rewrites_system_tree() {
        let raw = "source = /usr/share/tsk/hypr/window-rules.conf\n";
        assert_eq!(
            remap_packaged_share_paths(raw, "/home/u/.local/share/tsk"),
            "source = /home/u/.local/share/tsk/hypr/window-rules.conf\n"
        );
    }

    #[test]
    fn resolve_share_templates_prefers_system_share_for_prod() {
        if !crate::share::system_share_available() {
            return;
        }
        let resolved = resolve_share_templates(None, InstallProfile::Prod).unwrap();
        assert_eq!(resolved, crate::share::system_share_dir());
    }
}

pub fn resolve_share_templates(
    workspace_root: Option<&Path>,
    profile: InstallProfile,
) -> Result<PathBuf> {
    if let Some(root) = workspace_root {
        return Ok(root.join("share"));
    }
    if profile == InstallProfile::Prod && crate::share::system_share_available() {
        return Ok(crate::share::system_share_dir());
    }
    if let Some(workspace) = find_workspace_root() {
        return Ok(workspace.join("share"));
    }
    if crate::share::system_share_available() {
        return Ok(crate::share::system_share_dir());
    }
    Err(TskError::Other(
        "could not find share/ templates — install the hypr-taskspace package or run from the repo".into(),
    ))
}

/// Back-compat alias for dev callers that discover templates from the repo.
pub fn find_share_root(workspace_root: Option<&Path>) -> Result<PathBuf> {
    resolve_share_templates(workspace_root, InstallProfile::Dev)
}

pub fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("TSK_WORKSPACE") {
        let p = PathBuf::from(env);
        if p.join("Cargo.toml").is_file() {
            return Some(p);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("Cargo.toml").is_file() && dir.join("share/hypr").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

// Back-compat for waybar install path.
pub fn build_and_install_waybar_module(
    cfg: &TskConfig,
    workspace_root: Option<&Path>,
    quiet: bool,
) -> Result<PathBuf> {
    install_waybar_module(
        cfg,
        &InstallBinsOptions {
            workspace_root: workspace_root.map(Path::to_path_buf),
            quiet,
            ..Default::default()
        },
    )
}

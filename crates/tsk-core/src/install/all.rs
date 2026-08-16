//! `tsk install all` — run each detected integration installer.

use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::install::chromium::{install_chromium, InstallChromiumOptions};
use crate::install::detect::{detected_integrations, DetectedIntegrations};
use crate::install::omarchy::{install_omarchy_prod, OmarchyInstallOptions};
use crate::install::plugin::ControlUi;
use crate::install::walker::{install_walker, InstallWalkerOptions};

#[derive(Debug, Clone, Default)]
pub struct InstallAllOptions {
    pub dry_run: bool,
    pub quiet: bool,
    pub workspace_root: Option<std::path::PathBuf>,
    pub control_ui: ControlUi,
}

pub fn install_all(cfg: &TskConfig, options: &InstallAllOptions) -> Result<Vec<String>> {
    install_detected(cfg, options, detected_integrations())
}

pub fn install_detected(
    cfg: &TskConfig,
    options: &InstallAllOptions,
    detected: DetectedIntegrations,
) -> Result<Vec<String>> {
    if !detected.any() {
        return Err(TskError::Other(
            "no supported integrations detected (looked for Omarchy, Chromium, Walker/Elephant)"
                .into(),
        ));
    }

    let mut actions = Vec::new();
    if detected.omarchy {
        actions.push("detected Omarchy".into());
        let omarchy = install_omarchy_prod(
            cfg,
            &OmarchyInstallOptions {
                dry_run: options.dry_run,
                workspace_root: options.workspace_root.clone(),
                control_ui: options.control_ui,
            },
        )?;
        actions.extend(omarchy);
    } else {
        actions.push("Omarchy: skipped (not detected)".into());
        if detected.walker {
            actions.push("detected Walker/Elephant".into());
            let walker = install_walker(
                cfg,
                &InstallWalkerOptions {
                    dry_run: options.dry_run,
                    quiet: options.quiet,
                    skip_if_missing: true,
                },
            )?;
            actions.extend(walker);
        } else {
            actions.push("Walker: skipped (Elephant config not found)".into());
        }
    }

    if detected.chromium {
        actions.push("detected Chromium".into());
        let chromium = install_chromium(
            cfg,
            &InstallChromiumOptions {
                dry_run: options.dry_run,
                quiet: options.quiet,
                skip_if_missing: true,
                assume_present: true,
                user_data_dir: None,
            },
        )?;
        actions.extend(chromium);
    } else {
        actions.push("Chromium: skipped (not detected)".into());
    }

    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_detected_errors_when_nothing_found() {
        let cfg = TskConfig::default();
        let err = install_detected(
            &cfg,
            &InstallAllOptions::default(),
            DetectedIntegrations::default(),
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("no supported integrations detected"));
    }

    #[test]
    fn install_detected_dry_run_skips_missing_and_notes_chromium() {
        let cfg = TskConfig::default();
        let actions = install_detected(
            &cfg,
            &InstallAllOptions {
                dry_run: true,
                ..InstallAllOptions::default()
            },
            DetectedIntegrations {
                chromium: true,
                ..DetectedIntegrations::default()
            },
        )
        .unwrap();
        assert!(actions.iter().any(|a| a.contains("Omarchy: skipped")));
        assert!(actions.iter().any(|a| a.contains("Walker: skipped")));
        assert!(actions.iter().any(|a| a.contains("detected Chromium")));
        assert!(actions
            .iter()
            .any(|a| a.contains("would install TSK Chromium extension")));
    }
}

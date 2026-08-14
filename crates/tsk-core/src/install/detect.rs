//! Detect desktop integrations that `tsk install all` can wire up.

use std::path::PathBuf;

use crate::binary::command_v_login;
use crate::install::walker::elephant_config_path;
use crate::xdg::expand;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetectedIntegrations {
    pub omarchy: bool,
    pub chromium: bool,
    pub walker: bool,
}

pub fn omarchy_dir() -> PathBuf {
    expand("~/.local/share/omarchy")
}

pub fn omarchy_desktop_present() -> bool {
    omarchy_dir().is_dir()
}

pub fn default_chromium_user_data_dir() -> PathBuf {
    expand("~/.config/chromium")
}

pub fn chromium_binary() -> Option<String> {
    command_v_login("chromium").or_else(|| command_v_login("chromium-browser"))
}

pub fn chromium_present() -> bool {
    chromium_binary().is_some() || default_chromium_user_data_dir().is_dir()
}

pub fn elephant_present() -> bool {
    elephant_config_path().is_file()
}

pub fn detected_integrations() -> DetectedIntegrations {
    DetectedIntegrations {
        omarchy: omarchy_desktop_present(),
        chromium: chromium_present(),
        walker: elephant_present(),
    }
}

impl DetectedIntegrations {
    pub fn any(self) -> bool {
        self.omarchy || self.chromium || self.walker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_detected_is_not_any() {
        assert!(!DetectedIntegrations::default().any());
    }

    #[test]
    fn any_is_true_when_one_flag_set() {
        assert!(DetectedIntegrations {
            chromium: true,
            ..DetectedIntegrations::default()
        }
        .any());
    }
}

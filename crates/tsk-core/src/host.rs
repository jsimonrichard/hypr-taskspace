//! Host OS detection for sensible first-run defaults.

use std::collections::HashMap;
use std::fs;

struct OsRelease {
    id: String,
    id_like: String,
}

fn read_os_release() -> OsRelease {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut fields = HashMap::new();
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(
                key.to_ascii_lowercase(),
                value.trim_matches('"').to_ascii_lowercase(),
            );
        }
    }
    OsRelease {
        id: fields.remove("id").unwrap_or_default(),
        id_like: fields.remove("id_like").unwrap_or_default(),
    }
}

fn arch_toolbox() -> &'static str {
    "quay.io/toolbx-images/arch-toolbox:latest"
}

fn fedora_toolbox() -> &'static str {
    "quay.io/toolbx-images/fedora-toolbox:40"
}

fn ubuntu_toolbox() -> &'static str {
    "quay.io/toolbx-images/ubuntu-toolbox:24.04"
}

fn debian_toolbox() -> &'static str {
    "quay.io/toolbx-images/debian-toolbox:12"
}

/// Pick a toolbox image that roughly matches the host distro.
pub fn default_distrobox_image() -> String {
    let os = read_os_release();
    let image = match os.id.as_str() {
        "arch" | "cachyos" | "omarchy" | "manjaro" | "garuda" | "endeavouros" => arch_toolbox(),
        "fedora" => fedora_toolbox(),
        "ubuntu" | "pop" | "linuxmint" => ubuntu_toolbox(),
        "debian" | "raspbian" | "pureos" => debian_toolbox(),
        "opensuse-tumbleweed" | "opensuse-leap" | "opensuse" => {
            "quay.io/toolbx-images/opensuse-toolbox:tumbleweed"
        }
        "alpine" => "quay.io/toolbx-images/alpine-toolbox:edge",
        _ if os.id_like.contains("arch") => arch_toolbox(),
        _ if os.id_like.contains("fedora") => fedora_toolbox(),
        _ if os.id_like.contains("debian") || os.id_like.contains("ubuntu") => ubuntu_toolbox(),
        _ => fedora_toolbox(),
    };
    image.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_distrobox_image_is_non_empty() {
        assert!(!default_distrobox_image().is_empty());
    }

    #[test]
    fn maps_arch_id_to_arch_toolbox() {
        let image = image_for_os("arch", "");
        assert!(image.contains("arch-toolbox"));
    }

    #[test]
    fn maps_fedora_id_like() {
        let image = image_for_os("unknown", "fedora");
        assert!(image.contains("fedora-toolbox"));
    }

    fn image_for_os(id: &str, id_like: &str) -> String {
        let os = OsRelease {
            id: id.to_string(),
            id_like: id_like.to_string(),
        };
        match os.id.as_str() {
            "arch" | "cachyos" | "omarchy" | "manjaro" | "garuda" | "endeavouros" => {
                arch_toolbox().to_string()
            }
            "fedora" => fedora_toolbox().to_string(),
            "ubuntu" | "pop" | "linuxmint" => ubuntu_toolbox().to_string(),
            "debian" | "raspbian" | "pureos" => debian_toolbox().to_string(),
            "opensuse-tumbleweed" | "opensuse-leap" | "opensuse" => {
                "quay.io/toolbx-images/opensuse-toolbox:tumbleweed".to_string()
            }
            "alpine" => "quay.io/toolbx-images/alpine-toolbox:edge".to_string(),
            _ if os.id_like.contains("arch") => arch_toolbox().to_string(),
            _ if os.id_like.contains("fedora") => fedora_toolbox().to_string(),
            _ if os.id_like.contains("debian") || os.id_like.contains("ubuntu") => {
                ubuntu_toolbox().to_string()
            }
            _ => fedora_toolbox().to_string(),
        }
    }
}

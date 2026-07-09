//! Host OS detection for sensible first-run Distrobox image defaults.
//!
//! Image refs follow https://distrobox.it/compatibility/ (Toolbox where available).
//! Prefer `quay.io/toolbx/*` and `registry.fedoraproject.org` / `quay.io/fedora/*` —
//! the older `quay.io/toolbx-images/*` org now returns unauthorized.

use std::collections::HashMap;
use std::fs;

struct OsRelease {
    id: String,
    id_like: String,
    version_id: String,
    version_codename: String,
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
        version_id: fields.remove("version_id").unwrap_or_default(),
        version_codename: fields.remove("version_codename").unwrap_or_default(),
    }
}

fn arch_toolbox() -> &'static str {
    // Official Toolbx image (was previously under quay.io/toolbx-images/).
    "quay.io/toolbx/arch-toolbox:latest"
}

fn ubuntu_toolbox(tag: &str) -> String {
    format!("quay.io/toolbx/ubuntu-toolbox:{tag}")
}

fn fedora_toolbox(version: &str) -> String {
    // Fedora keeps older toolbox tags on registry.fedoraproject.org and newer on quay.io/fedora.
    match version.parse::<u32>() {
        Ok(n) if n >= 41 => format!("quay.io/fedora/fedora-toolbox:{version}"),
        _ => format!("registry.fedoraproject.org/fedora-toolbox:{version}"),
    }
}

fn debian_toolbox(tag: &str) -> String {
    // toolbx-images debian-toolbox is currently unauthorized; use Distrobox-compatible library images.
    match tag {
        "11" | "bullseye" => "docker.io/library/debian:bullseye".into(),
        "13" | "trixie" => "docker.io/library/debian:trixie".into(),
        "testing" => "docker.io/library/debian:testing".into(),
        "unstable" | "sid" => "docker.io/library/debian:unstable".into(),
        _ => "docker.io/library/debian:bookworm".into(), // 12 / bookworm default
    }
}

fn opensuse_toolbox() -> &'static str {
    "registry.opensuse.org/opensuse/distrobox:latest"
}

fn alpine_toolbox() -> &'static str {
    "docker.io/library/alpine:edge"
}

/// Known Ubuntu toolbox tags from Distrobox compatibility docs.
const UBUNTU_TOOLBOX_TAGS: &[&str] = &["16.04", "18.04", "20.04", "22.04", "24.04"];

/// Known Fedora toolbox major versions (fallback chain when host version is missing).
const FEDORA_TOOLBOX_VERSIONS: &[u32] = &[43, 42, 41, 40, 39, 38];

fn pick_ubuntu_tag(version_id: &str) -> &str {
    if UBUNTU_TOOLBOX_TAGS.contains(&version_id) {
        return version_id;
    }
    // Round down to nearest known LTS for interim/derivative releases (e.g. 24.10 → 24.04).
    if let Some((major, minor)) = parse_dotted_version(version_id) {
        let mut best: Option<&str> = None;
        for tag in UBUNTU_TOOLBOX_TAGS {
            let Some((tm, tn)) = parse_dotted_version(tag) else {
                continue;
            };
            if (tm, tn) <= (major, minor) {
                best = Some(tag);
            }
        }
        if let Some(tag) = best {
            return tag;
        }
    }
    "24.04"
}

fn pick_fedora_version(version_id: &str) -> String {
    if version_id == "rawhide" {
        return "rawhide".into();
    }
    if let Ok(n) = version_id.parse::<u32>() {
        if FEDORA_TOOLBOX_VERSIONS.contains(&n) || (38..=50).contains(&n) {
            return n.to_string();
        }
    }
    FEDORA_TOOLBOX_VERSIONS[0].to_string()
}

fn pick_debian_tag(version_id: &str, codename: &str) -> &'static str {
    match version_id {
        "11" => "11",
        "12" => "12",
        "13" => "13",
        _ => match codename {
            "bullseye" => "11",
            "bookworm" => "12",
            "trixie" => "13",
            "testing" => "testing",
            "sid" | "unstable" => "unstable",
            _ => "12",
        },
    }
}

fn parse_dotted_version(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}

fn image_for_os(os: &OsRelease) -> String {
    match os.id.as_str() {
        "arch" | "cachyos" | "omarchy" | "manjaro" | "garuda" | "endeavouros" | "archcraft" => {
            arch_toolbox().into()
        }
        "fedora" | "nobara" | "ultramarine" => fedora_toolbox(&pick_fedora_version(&os.version_id)),
        "ubuntu" | "pop" | "linuxmint" | "elementary" | "zorin" | "neon" => {
            ubuntu_toolbox(pick_ubuntu_tag(&os.version_id))
        }
        "debian" | "raspbian" | "pureos" | "kali" | "devuan" => {
            debian_toolbox(pick_debian_tag(&os.version_id, &os.version_codename))
        }
        "opensuse-tumbleweed" | "opensuse-slowroll" | "opensuse-aeon" | "opensuse-kalpa" => {
            opensuse_toolbox().into()
        }
        "opensuse-leap" | "opensuse" | "sles" => {
            // Leap is close to Tumbleweed toolbox / distrobox image for Distrobox use.
            opensuse_toolbox().into()
        }
        "alpine" => alpine_toolbox().into(),
        "bazzite" => "ghcr.io/ublue-os/bazzite-arch:latest".into(),
        _ if os.id_like.contains("arch") => arch_toolbox().into(),
        _ if os.id_like.contains("fedora") || os.id_like.contains("rhel") => {
            fedora_toolbox(&pick_fedora_version(&os.version_id))
        }
        _ if os.id_like.contains("ubuntu") => ubuntu_toolbox(pick_ubuntu_tag(&os.version_id)),
        _ if os.id_like.contains("debian") => {
            debian_toolbox(pick_debian_tag(&os.version_id, &os.version_codename))
        }
        _ if os.id_like.contains("suse") => opensuse_toolbox().into(),
        _ => arch_toolbox().into(),
    }
}

/// Pick a toolbox/compatible image that matches the host distro (and version when known).
pub fn default_distrobox_image() -> String {
    image_for_os(&read_os_release())
}

/// Rewrite obsolete `quay.io/toolbx-images/*` refs that now fail with unauthorized.
pub fn migrate_stale_distrobox_image(image: &str) -> Option<String> {
    let trimmed = image.trim();
    if !trimmed.contains("quay.io/toolbx-images/") {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let replacement = if lower.contains("arch") {
        arch_toolbox().into()
    } else if lower.contains("ubuntu") {
        // Preserve tag when present.
        let tag = lower
            .rsplit_once(':')
            .map(|(_, t)| t)
            .filter(|t| UBUNTU_TOOLBOX_TAGS.contains(t))
            .unwrap_or("24.04");
        ubuntu_toolbox(tag)
    } else if lower.contains("fedora") {
        let tag = lower.rsplit_once(':').map(|(_, t)| t).unwrap_or("41");
        fedora_toolbox(tag)
    } else if lower.contains("debian") {
        let tag = lower.rsplit_once(':').map(|(_, t)| t).unwrap_or("12");
        debian_toolbox(tag)
    } else if lower.contains("alpine") {
        alpine_toolbox().into()
    } else if lower.contains("opensuse") {
        opensuse_toolbox().into()
    } else {
        default_distrobox_image()
    };
    if replacement == trimmed {
        None
    } else {
        Some(replacement)
    }
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
        let image = image_for_os(&OsRelease {
            id: "arch".into(),
            id_like: String::new(),
            version_id: String::new(),
            version_codename: String::new(),
        });
        assert_eq!(image, "quay.io/toolbx/arch-toolbox:latest");
        assert!(!image.contains("toolbx-images"));
    }

    #[test]
    fn maps_omarchy_to_arch_toolbox() {
        let image = image_for_os(&OsRelease {
            id: "omarchy".into(),
            id_like: "arch".into(),
            version_id: String::new(),
            version_codename: String::new(),
        });
        assert!(image.contains("arch-toolbox"));
    }

    #[test]
    fn maps_fedora_version() {
        let image = image_for_os(&OsRelease {
            id: "fedora".into(),
            id_like: String::new(),
            version_id: "42".into(),
            version_codename: String::new(),
        });
        assert_eq!(image, "quay.io/fedora/fedora-toolbox:42");
    }

    #[test]
    fn maps_older_fedora_to_fedoraproject_registry() {
        let image = image_for_os(&OsRelease {
            id: "fedora".into(),
            id_like: String::new(),
            version_id: "40".into(),
            version_codename: String::new(),
        });
        assert_eq!(image, "registry.fedoraproject.org/fedora-toolbox:40");
    }

    #[test]
    fn maps_ubuntu_version() {
        let image = image_for_os(&OsRelease {
            id: "ubuntu".into(),
            id_like: "debian".into(),
            version_id: "24.04".into(),
            version_codename: "noble".into(),
        });
        assert_eq!(image, "quay.io/toolbx/ubuntu-toolbox:24.04");
    }

    #[test]
    fn rounds_ubuntu_interim_down_to_lts() {
        assert_eq!(pick_ubuntu_tag("24.10"), "24.04");
        assert_eq!(pick_ubuntu_tag("22.10"), "22.04");
    }

    #[test]
    fn maps_debian_codename() {
        let image = image_for_os(&OsRelease {
            id: "debian".into(),
            id_like: String::new(),
            version_id: String::new(),
            version_codename: "bookworm".into(),
        });
        assert_eq!(image, "docker.io/library/debian:bookworm");
    }

    #[test]
    fn maps_fedora_id_like() {
        let image = image_for_os(&OsRelease {
            id: "unknown".into(),
            id_like: "fedora".into(),
            version_id: "41".into(),
            version_codename: String::new(),
        });
        assert!(image.contains("fedora-toolbox"));
    }

    #[test]
    fn migrates_stale_toolbx_images_arch() {
        assert_eq!(
            migrate_stale_distrobox_image("quay.io/toolbx-images/arch-toolbox:latest")
                .as_deref(),
            Some("quay.io/toolbx/arch-toolbox:latest")
        );
    }

    #[test]
    fn migrates_stale_ubuntu_preserving_tag() {
        assert_eq!(
            migrate_stale_distrobox_image("quay.io/toolbx-images/ubuntu-toolbox:22.04")
                .as_deref(),
            Some("quay.io/toolbx/ubuntu-toolbox:22.04")
        );
    }

    #[test]
    fn leaves_current_images_alone() {
        assert!(migrate_stale_distrobox_image("quay.io/toolbx/arch-toolbox:latest").is_none());
    }
}

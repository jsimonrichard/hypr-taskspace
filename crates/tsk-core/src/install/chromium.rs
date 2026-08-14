//! Chromium helper extension + native-messaging host install.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::binary::resolve_tsk_command;
use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::install::backup::backup_timestamp;
use crate::install::detect::{chromium_binary, chromium_present, default_chromium_user_data_dir};
use crate::install::manifest::{self, Manifest};
use crate::install::profile::{install_metadata_dir, profile_for_config};
use crate::share::{effective_share_dir, uses_packaged_share};
use crate::xdg::ensure_parent;

pub const NATIVE_HOST_NAME: &str = "org.tsk.browser";
const MANAGED_MARKER: &str = "tsk-managed";

#[derive(Debug, Clone)]
pub struct InstallChromiumOptions {
    pub dry_run: bool,
    pub quiet: bool,
    /// Skip quietly when Chromium is not on this machine (`tsk install all`).
    pub skip_if_missing: bool,
    /// Caller already detected Chromium (`tsk install all`).
    pub assume_present: bool,
    /// Override `~/.config/chromium` (tests).
    pub user_data_dir: Option<PathBuf>,
}

impl Default for InstallChromiumOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            quiet: false,
            skip_if_missing: false,
            assume_present: false,
            user_data_dir: None,
        }
    }
}

pub fn chromium_user_data_dir(options: &InstallChromiumOptions) -> PathBuf {
    options
        .user_data_dir
        .clone()
        .unwrap_or_else(default_chromium_user_data_dir)
}

pub fn external_extensions_dir(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("External Extensions")
}

pub fn native_messaging_hosts_dir(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("NativeMessagingHosts")
}

pub fn share_extension_dir(cfg: &TskConfig) -> PathBuf {
    effective_share_dir(cfg).join("chromium/extension")
}

pub fn install_chromium_status(cfg: &TskConfig) -> Result<Value> {
    let user_data = default_chromium_user_data_dir();
    let metadata_dir = install_metadata_dir(cfg, profile_for_config(cfg));
    let m = manifest::load_manifest(&metadata_dir, "chromium")?;
    let extension_id = m.as_ref().and_then(|manifest| {
        manifest.templates_installed.first().and_then(|v| {
            v.get("extension_id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
    });
    let json_path = extension_id
        .as_ref()
        .map(|id| external_extensions_dir(&user_data).join(format!("{id}.json")));
    let host_path = native_messaging_hosts_dir(&user_data).join(format!("{NATIVE_HOST_NAME}.json"));
    let json_present = json_path.as_ref().is_some_and(|p| p.is_file());
    let installed_version = json_path
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| {
            v.get("external_version")
                .and_then(|s| s.as_str())
                .map(str::to_string)
        });
    Ok(json!({
        "installed": json_present && host_path.is_file(),
        "detected": chromium_present(),
        "manifest": m.is_some(),
        "user_data_dir": user_data,
        "extension_id": extension_id,
        "extension_version": installed_version,
        "package_version": chrome_release_version(),
        "external_json": json_path,
        "external_json_present": json_present,
        "native_host": host_path,
        "native_host_present": host_path.is_file(),
        "share_extension": share_extension_dir(cfg),
    }))
}

pub fn install_chromium(cfg: &TskConfig, options: &InstallChromiumOptions) -> Result<Vec<String>> {
    let detected = options.user_data_dir.is_some() || options.assume_present || chromium_present();
    if !detected {
        if options.skip_if_missing {
            return Ok(vec!["Chromium: skipped (not detected)".into()]);
        }
        return Err(TskError::Other(
            "Chromium not detected — install chromium or create ~/.config/chromium".into(),
        ));
    }

    let user_data = chromium_user_data_dir(options);
    let share_ext = share_extension_dir(cfg);
    let work_dir = cfg.data_dir.join("chromium");
    let unpacked = work_dir.join("extension");
    let pem_path = work_dir.join("extension.pem");
    let crx_path = work_dir.join("extension.crx");
    let host_dest = work_dir.join("tsk-chromium-host");

    if options.dry_run {
        return Ok(vec![
            format!(
                "would install TSK Chromium extension into {}",
                user_data.display()
            ),
            format!("  unpacked from {}", share_ext.display()),
            format!("  CRX {}", crx_path.display()),
            format!(
                "  External Extensions JSON under {}",
                external_extensions_dir(&user_data).display()
            ),
            format!(
                "  native host {} → {}",
                NATIVE_HOST_NAME,
                host_dest.display()
            ),
        ]);
    }

    if !share_ext.join("manifest.json").is_file() {
        return Err(TskError::Other(format!(
            "Chromium extension templates missing at {} — reinstall share assets",
            share_ext.display()
        )));
    }

    copy_unpacked_extension(&share_ext, &unpacked)?;
    ensure_extension_pem(&pem_path)?;
    let spki = spki_der_from_pem(&pem_path)?;
    let extension_id = extension_id_from_spki(&spki);
    let version = install_extension_version(cfg, &work_dir)?;
    inject_manifest_key(&unpacked.join("manifest.json"), &spki)?;
    inject_manifest_version(&unpacked.join("manifest.json"), &version)?;
    pack_crx(cfg, &unpacked, &pem_path, &crx_path, options.quiet)?;
    let host_src = write_native_host_wrapper(cfg, &host_dest)?;
    let ext_dir = external_extensions_dir(&user_data);
    fs::create_dir_all(&ext_dir).map_err(|source| TskError::Write {
        path: ext_dir.clone(),
        source,
    })?;
    let json_path = ext_dir.join(format!("{extension_id}.json"));
    let spec = json!({
        "external_crx": crx_path,
        "external_version": version,
    });
    fs::write(
        &json_path,
        format!("{}\n", serde_json::to_string_pretty(&spec).unwrap()),
    )
    .map_err(|source| TskError::Write {
        path: json_path.clone(),
        source,
    })?;

    let hosts_dir = native_messaging_hosts_dir(&user_data);
    fs::create_dir_all(&hosts_dir).map_err(|source| TskError::Write {
        path: hosts_dir.clone(),
        source,
    })?;
    let host_json_path = hosts_dir.join(format!("{NATIVE_HOST_NAME}.json"));
    let host_spec = json!({
        "name": NATIVE_HOST_NAME,
        "description": "TSK taskspace browser helper",
        "path": host_src,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")],
    });
    fs::write(
        &host_json_path,
        format!("{}\n", serde_json::to_string_pretty(&host_spec).unwrap()),
    )
    .map_err(|source| TskError::Write {
        path: host_json_path.clone(),
        source,
    })?;

    let profile = profile_for_config(cfg);
    let metadata_dir = install_metadata_dir(cfg, profile);
    let backup_dir = metadata_dir
        .join("install/chromium/backups")
        .join(backup_timestamp());
    let manifest = Manifest {
        version: 1,
        integration: "chromium".into(),
        installed_at: Utc::now().to_rfc3339(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        templates_installed: vec![json!({
            "extension_id": extension_id,
            "version": version,
            "crx": crx_path,
            "marker": MANAGED_MARKER,
        })],
        user_files_backed_up: vec![],
        user_files_modified: vec![
            json!({"path": json_path, "actions": [{"type": "write", "marker": MANAGED_MARKER}]}),
            json!({"path": host_json_path, "actions": [{"type": "write", "marker": MANAGED_MARKER}]}),
        ],
        module_kind: Some(format!("{profile:?}").to_lowercase()),
    };
    manifest::save_manifest(&metadata_dir, &manifest)?;

    let mut actions = vec![
        format!("installed Chromium extension {extension_id} v{version}"),
        format!("  {}", json_path.display()),
        format!("  {}", host_json_path.display()),
    ];
    if version != chrome_release_version() {
        actions.push(
            "  (from-source revision — reinstall after extension edits; no manifest bump needed)"
                .into(),
        );
    }
    if !options.quiet {
        actions.push("restart Chromium to load the extension".into());
    }
    Ok(actions)
}

pub fn run_native_host() -> Result<()> {
    use std::io::{self, Read, Write};

    const MAX_MSG: usize = 1024 * 1024;
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    loop {
        let mut len_buf = [0u8; 4];
        match stdin.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                return Err(TskError::Other(format!("native host read: {e}")));
            }
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > MAX_MSG {
            return Err(TskError::Other(format!(
                "native host message too large ({len} bytes)"
            )));
        }
        let mut msg = vec![0u8; len];
        stdin
            .read_exact(&mut msg)
            .map_err(|e| TskError::Other(format!("native host read: {e}")))?;
        if let Ok(cfg) = crate::config::load_config() {
            if let Err(err) = crate::browser_session::ingest_native_message(&cfg, &msg) {
                eprintln!("tsk chromium-host: {err}");
            }
        }
        let reply = br#"{"ok":true}"#;
        stdout
            .write_all(&(reply.len() as u32).to_le_bytes())
            .and_then(|_| stdout.write_all(reply))
            .and_then(|_| stdout.flush())
            .map_err(|e| TskError::Other(format!("native host write: {e}")))?;
        let _ = msg;
    }
    Ok(())
}

fn write_native_host_wrapper(cfg: &TskConfig, dest: &Path) -> Result<PathBuf> {
    ensure_parent(dest)?;
    let tsk = resolve_tsk_command(cfg);
    let body = format!(
        "#!/usr/bin/env bash\nexec {} chromium-host\n",
        crate::binary::shell_quote(&tsk)
    );
    fs::write(dest, body).map_err(|source| TskError::Write {
        path: dest.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest)
            .map_err(|source| TskError::Read {
                path: dest.to_path_buf(),
                source,
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dest, perms).map_err(|source| TskError::Write {
            path: dest.to_path_buf(),
            source,
        })?;
    }
    Ok(dest.to_path_buf())
}

fn copy_unpacked_extension(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|source| TskError::Write {
            path: dest.to_path_buf(),
            source,
        })?;
    }
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
        let target = dest.join(path.file_name().unwrap());
        fs::copy(&path, &target).map_err(|source| TskError::Write {
            path: target,
            source,
        })?;
    }
    Ok(())
}

fn ensure_extension_pem(pem_path: &Path) -> Result<()> {
    if pem_path.is_file() {
        return Ok(());
    }
    ensure_parent(pem_path)?;
    let status = Command::new("openssl")
        .args(["genrsa", "-out"])
        .arg(pem_path)
        .arg("2048")
        .status()
        .map_err(|e| TskError::Other(format!("openssl genrsa: {e}")))?;
    if !status.success() {
        return Err(TskError::Other(
            "openssl genrsa failed — install openssl to pack the Chromium extension".into(),
        ));
    }
    Ok(())
}

fn spki_der_from_pem(pem_path: &Path) -> Result<Vec<u8>> {
    let output = Command::new("openssl")
        .args(["rsa", "-in"])
        .arg(pem_path)
        .args(["-pubout", "-outform", "DER"])
        .output()
        .map_err(|e| TskError::Other(format!("openssl rsa: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(TskError::Other(format!(
            "openssl rsa -pubout failed: {err}"
        )));
    }
    Ok(output.stdout)
}

/// Chrome/Chromium extension id: first 16 bytes of SHA-256(SPKI DER), mapped a–p.
pub fn extension_id_from_spki(der: &[u8]) -> String {
    let hash = Sha256::digest(der);
    hash[..16]
        .iter()
        .flat_map(|byte| {
            let hi = byte >> 4;
            let lo = byte & 0x0f;
            [char::from(b'a' + hi), char::from(b'a' + lo)]
        })
        .collect()
}

/// Workspace package version, sanitized to Chromium's `a.b.c` form.
pub fn chrome_release_version() -> String {
    chrome_release_version_from(env!("CARGO_PKG_VERSION"))
}

fn chrome_release_version_from(pkg: &str) -> String {
    let numeric = pkg.split('-').next().unwrap_or(pkg);
    let mut parts: Vec<&str> = numeric.split('.').take(3).collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

/// Packaged installs use the project version. From-source / dev installs append
/// a 4th component so Chromium treats each `tsk install chromium` as an update
/// without editing `manifest.json`.
fn install_extension_version(cfg: &TskConfig, work_dir: &Path) -> Result<String> {
    let base = chrome_release_version();
    if uses_packaged_share(cfg) {
        return Ok(base);
    }
    let revision = next_dev_revision(work_dir)?;
    Ok(format!("{base}.{revision}"))
}

fn next_dev_revision(work_dir: &Path) -> Result<u32> {
    let path = work_dir.join("dev-revision");
    ensure_parent(&path)?;
    let current = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let next = if current >= 65535 { 1 } else { current + 1 };
    fs::write(&path, format!("{next}\n")).map_err(|source| TskError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(next)
}

fn inject_manifest_string(manifest_path: &Path, field: &str, value: &str) -> Result<()> {
    let raw = fs::read_to_string(manifest_path).map_err(|source| TskError::Read {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    let mut parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| TskError::Other(format!("extension manifest: {e}")))?;
    let Some(obj) = parsed.as_object_mut() else {
        return Err(TskError::Other(
            "extension manifest is not an object".into(),
        ));
    };
    obj.insert(field.into(), Value::String(value.to_string()));
    let pretty = serde_json::to_string_pretty(&parsed)
        .map_err(|e| TskError::Other(format!("extension manifest: {e}")))?;
    fs::write(manifest_path, format!("{pretty}\n")).map_err(|source| TskError::Write {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn inject_manifest_key(manifest_path: &Path, spki: &[u8]) -> Result<()> {
    inject_manifest_string(manifest_path, "key", &base64_encode(spki))
}

fn inject_manifest_version(manifest_path: &Path, version: &str) -> Result<()> {
    inject_manifest_string(manifest_path, "version", version)
}

fn pack_crx(
    _cfg: &TskConfig,
    unpacked: &Path,
    pem_path: &Path,
    crx_path: &Path,
    quiet: bool,
) -> Result<()> {
    let browser = chromium_binary().ok_or_else(|| {
        TskError::Other("chromium not on PATH — cannot pack the helper extension".into())
    })?;
    let status = Command::new(&browser)
        .arg("--no-message-box")
        .arg(format!("--pack-extension={}", unpacked.display()))
        .arg(format!("--pack-extension-key={}", pem_path.display()))
        .status()
        .map_err(|e| TskError::Other(format!("pack extension: {e}")))?;
    if !status.success() {
        return Err(TskError::Other(format!(
            "{browser} --pack-extension failed (is Chromium already running this profile?)"
        )));
    }
    let generated = unpacked.with_extension("crx");
    if generated != *crx_path && generated.is_file() {
        if let Some(parent) = crx_path.parent() {
            fs::create_dir_all(parent).map_err(|source| TskError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::rename(&generated, crx_path)
            .or_else(|_| fs::copy(&generated, crx_path).map(|_| ()))
            .map_err(|source| TskError::Write {
                path: crx_path.to_path_buf(),
                source,
            })?;
        let _ = fs::remove_file(&generated);
    }
    if !crx_path.is_file() {
        return Err(TskError::Other(format!(
            "packed CRX missing at {}",
            crx_path.display()
        )));
    }
    let _ = quiet;
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes.get(i + 1).copied();
        let b2 = bytes.get(i + 2).copied();
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[((b0 & 0x03) << 4 | b1.unwrap_or(0) >> 4) as usize] as char);
        if b1.is_none() {
            out.push('=');
            out.push('=');
        } else {
            out.push(TABLE[((b1.unwrap() & 0x0f) << 2 | b2.unwrap_or(0) >> 6) as usize] as char);
            if b2.is_none() {
                out.push('=');
            } else {
                out.push(TABLE[(b2.unwrap() & 0x3f) as usize] as char);
            }
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_id_is_32_lowercase_a_to_p() {
        let id = extension_id_from_spki(&[0u8; 16]);
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| ('a'..='p').contains(&c)));
    }

    #[test]
    fn extension_id_stable_for_same_spki() {
        let der = b"not-a-real-spki-but-fine-for-hash";
        assert_eq!(extension_id_from_spki(der), extension_id_from_spki(der));
    }

    #[test]
    fn inject_manifest_key_adds_key_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        fs::write(&path, r#"{"manifest_version":3,"version":"0.0.0"}"#).unwrap();
        inject_manifest_key(&path, b"hello").unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["key"], "aGVsbG8=");
        assert_eq!(value["version"], "0.0.0");
    }

    #[test]
    fn release_version_strips_prerelease_and_pads() {
        assert_eq!(chrome_release_version_from("0.1.0"), "0.1.0");
        assert_eq!(chrome_release_version_from("1.2.3-alpha.1"), "1.2.3");
        assert_eq!(chrome_release_version_from("2.0"), "2.0.0");
    }

    #[test]
    fn release_version_matches_workspace_package() {
        assert_eq!(
            chrome_release_version(),
            chrome_release_version_from(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(chrome_release_version().split('.').count(), 3);
    }

    #[test]
    fn inject_manifest_version_overwrites_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        fs::write(&path, r#"{"manifest_version":3,"version":"0.0.0"}"#).unwrap();
        inject_manifest_version(&path, "0.1.0.4").unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["version"], "0.1.0.4");
    }

    #[test]
    fn dev_revision_increments_and_wraps() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(next_dev_revision(dir.path()).unwrap(), 1);
        assert_eq!(next_dev_revision(dir.path()).unwrap(), 2);
        fs::write(dir.path().join("dev-revision"), "65535\n").unwrap();
        assert_eq!(next_dev_revision(dir.path()).unwrap(), 1);
    }

    #[test]
    fn from_source_install_version_appends_revision() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = TskConfig::default();
        cfg.container_prefix = "tsk-dev".into();
        cfg.install_hypr_share_dir = crate::install::profile::dev_share_dir();
        let version = install_extension_version(&cfg, dir.path()).unwrap();
        let base = chrome_release_version();
        assert_eq!(version, format!("{base}.1"));
        let again = install_extension_version(&cfg, dir.path()).unwrap();
        assert_eq!(again, format!("{base}.2"));
    }

    #[test]
    fn base64_padding() {
        assert_eq!(base64_encode(b"h"), "aA==");
        assert_eq!(base64_encode(b"he"), "aGU=");
        assert_eq!(base64_encode(b"hel"), "aGVs");
    }
}

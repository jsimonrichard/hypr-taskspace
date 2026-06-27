use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{LaeError, Result};
use crate::xdg::ensure_parent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub integration: String,
    pub installed_at: String,
    pub backup_dir: String,
    #[serde(default)]
    pub templates_installed: Vec<Value>,
    #[serde(default)]
    pub user_files_backed_up: Vec<Value>,
    #[serde(default)]
    pub user_files_modified: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_kind: Option<String>,
}

impl Manifest {
    pub fn new_waybar(backup_dir: PathBuf) -> Self {
        Self {
            version: 1,
            integration: "waybar".into(),
            installed_at: Utc::now().to_rfc3339(),
            backup_dir: backup_dir.to_string_lossy().into(),
            templates_installed: Vec::new(),
            user_files_backed_up: Vec::new(),
            user_files_modified: Vec::new(),
            module_kind: Some("cffi".into()),
        }
    }
}

pub fn manifest_path(share_dir: &Path, integration: &str) -> PathBuf {
    share_dir.join("install").join(integration).join("manifest.json")
}

pub fn load_manifest(share_dir: &Path, integration: &str) -> Result<Option<Manifest>> {
    let path = manifest_path(share_dir, integration);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|source| LaeError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(Some(
        serde_json::from_str(&raw).map_err(|source| LaeError::Parse { path, source })?,
    ))
}

pub fn save_manifest(share_dir: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let path = manifest_path(share_dir, &manifest.integration);
    ensure_parent(&path)?;
    fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(manifest)
                .map_err(|e| LaeError::Other(e.to_string()))?
        ),
    )
    .map_err(|source| LaeError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn remove_manifest(share_dir: &Path, integration: &str) -> Result<()> {
    let path = manifest_path(share_dir, integration);
    if path.is_file() {
        fs::remove_file(&path).map_err(|source| LaeError::Write { path, source })?;
    }
    Ok(())
}

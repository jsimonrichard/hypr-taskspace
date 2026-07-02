//! Registered repository configuration (`~/.config/lae/repos.toml`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{LaeError, Result};
use crate::models::slugify;
use crate::xdg::{ensure_parent, expand, lae_config_dir};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredRepo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RepoFile {
    repos: Vec<RegisteredRepo>,
}

pub fn repos_config_path() -> PathBuf {
    lae_config_dir().join("repos.toml")
}

pub fn load_repos() -> Result<Vec<RegisteredRepo>> {
    let path = repos_config_path();
    if !path.is_file() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| LaeError::Read {
        path: path.clone(),
        source,
    })?;
    let file: RepoFile = toml::from_str(&raw).map_err(|e| LaeError::Config(e.to_string()))?;
    Ok(file.repos)
}

pub fn save_repos(repos: &[RegisteredRepo]) -> Result<()> {
    let path = repos_config_path();
    ensure_parent(&path)?;
    let body = toml::to_string_pretty(&RepoFile {
        repos: repos.to_vec(),
    })
    .map_err(|e| LaeError::Other(e.to_string()))?;
    std::fs::write(&path, body).map_err(|source| LaeError::Write { path, source })
}

pub fn unique_repo_id(repos: &[RegisteredRepo], seed: &str) -> String {
    let base = slugify(seed);
    let base = if base.is_empty() { "repo".into() } else { base };
    if !repos.iter().any(|r| r.id == base) {
        return base;
    }
    for n in 2..100 {
        let candidate = format!("{base}-{n}");
        if !repos.iter().any(|r| r.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", repos.len() + 1)
}

pub fn find_repo<'a>(repos: &'a [RegisteredRepo], id: &str) -> Option<&'a RegisteredRepo> {
    repos.iter().find(|r| r.id == id)
}

pub fn paths_match(a: &Path, b: &Path) -> bool {
    let a = expand(a);
    let b = expand(b);
    match (std::fs::canonicalize(&a), std::fs::canonicalize(&b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub fn repo_display_path(repo: &RegisteredRepo) -> PathBuf {
    expand(&repo.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_repo_id_appends_suffix() {
        let repos = vec![RegisteredRepo {
            id: "my-app".into(),
            name: "My App".into(),
            path: "/tmp/my-app".into(),
            url: None,
        }];
        assert_eq!(unique_repo_id(&repos, "My App"), "my-app-2");
    }
}

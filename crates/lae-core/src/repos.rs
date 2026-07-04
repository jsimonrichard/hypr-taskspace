//! Registered repositories — settings live in each checkout at `.lae/repo.toml`.
//!
//! `~/.config/lae/repo-bookmarks.txt` only stores checkout paths (pointers), not settings.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{LaeError, Result};
use crate::models::{slugify, Task};
use crate::task_paths::scratch_checkout_path;
use crate::vcs::{detect_vcs_root, repo_label};
use crate::xdg::{ensure_parent, expand, lae_config_dir};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredRepo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

pub fn repo_config_path(vcs_root: &Path) -> PathBuf {
    expand(vcs_root).join(".lae").join("repo.toml")
}

pub fn repo_bookmarks_path() -> PathBuf {
    lae_config_dir().join("repo-bookmarks.txt")
}

pub fn paths_match(a: &Path, b: &Path) -> bool {
    let a = expand(a);
    let b = expand(b);
    match (std::fs::canonicalize(&a), std::fs::canonicalize(&b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub fn normalize_repo_path(path: &Path) -> PathBuf {
    expand(path)
}

/// True when the task uses an isolated scratch repo under its task home.
pub fn is_task_scratch_repo(task_id: &str, repo_path: &Path, tasks_base_dir: &Path) -> bool {
    paths_match(
        repo_path,
        &scratch_checkout_path(&tasks_base_dir.join(task_id)),
    )
}

pub fn load_repo_config(vcs_root: &Path) -> Result<Option<RegisteredRepo>> {
    let path = repo_config_path(vcs_root);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| LaeError::Read {
        path: path.clone(),
        source,
    })?;
    let mut repo: RegisteredRepo =
        toml::from_str(&raw).map_err(|e| LaeError::Config(e.to_string()))?;
    repo.path = normalize_repo_path(vcs_root);
    Ok(Some(repo))
}

pub fn save_repo_config(repo: &RegisteredRepo) -> Result<()> {
    let root = normalize_repo_path(&repo.path);
    let path = repo_config_path(&root);
    ensure_parent(&path)?;
    let lae_dir = root.join(".lae");
    std::fs::create_dir_all(&lae_dir).map_err(|source| LaeError::Write {
        path: lae_dir,
        source,
    })?;
    let body = toml::to_string_pretty(repo).map_err(|e| LaeError::Other(e.to_string()))?;
    std::fs::write(&path, body).map_err(|source| LaeError::Write { path, source })
}

fn load_bookmarks() -> Result<Vec<PathBuf>> {
    let path = repo_bookmarks_path();
    if !path.is_file() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| LaeError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(PathBuf::from)
        .collect())
}

fn save_bookmarks(paths: &[PathBuf]) -> Result<()> {
    let path = repo_bookmarks_path();
    ensure_parent(&path)?;
    let mut unique: Vec<PathBuf> = Vec::new();
    for candidate in paths {
        let candidate = normalize_repo_path(candidate);
        if candidate.is_dir()
            && !unique.iter().any(|existing| paths_match(existing, &candidate))
        {
            unique.push(candidate);
        }
    }
    unique.sort_by(|a, b| a.display().to_string().cmp(&b.display().to_string()));
    let body = unique
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let body = if body.is_empty() {
        body
    } else {
        format!("{body}\n")
    };
    std::fs::write(&path, body).map_err(|source| LaeError::Write { path, source })
}

fn bookmark_key(path: &Path) -> String {
    normalize_repo_path(path).display().to_string()
}

pub fn load_repos(extra_paths: impl IntoIterator<Item = PathBuf>) -> Result<Vec<RegisteredRepo>> {
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in load_bookmarks()? {
        paths.insert(bookmark_key(&path), normalize_repo_path(&path));
    }
    for path in extra_paths {
        paths.insert(bookmark_key(&path), normalize_repo_path(&path));
    }

    let mut repos: Vec<RegisteredRepo> = Vec::new();
    for path in paths.into_values() {
        if !path.is_dir() {
            continue;
        }
        let repo = if let Some(config) = load_repo_config(&path)? {
            config
        } else {
            RegisteredRepo {
                id: slugify(&repo_label(&path)),
                name: repo_label(&path),
                path: path.clone(),
                url: None,
            }
        };
        if !repos.iter().any(|r| paths_match(&r.path, &repo.path)) {
            repos.push(repo);
        }
    }

    repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(repos)
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

pub fn find_repo_by_path<'a>(repos: &'a [RegisteredRepo], path: &Path) -> Option<&'a RegisteredRepo> {
    repos.iter().find(|r| paths_match(&r.path, path))
}

pub fn register_repo(path: &Path, existing: &[RegisteredRepo]) -> Result<RegisteredRepo> {
    let root = detect_vcs_root(Some(path)).ok_or_else(|| {
        LaeError::Other(format!(
            "No git or jj repo found at {}",
            path.display()
        ))
    })?;
    let root = normalize_repo_path(&root);

    let repo = if let Some(config) = load_repo_config(&root)? {
        config
    } else {
        RegisteredRepo {
            id: unique_repo_id(existing, &repo_label(&root)),
            name: repo_label(&root),
            path: root.clone(),
            url: None,
        }
    };
    save_repo_config(&repo)?;

    let mut bookmarks = load_bookmarks()?;
    if !bookmarks.iter().any(|p| paths_match(p, &root)) {
        bookmarks.push(root);
    }
    save_bookmarks(&bookmarks)?;
    Ok(repo)
}

pub fn unregister_repo(path: &Path) -> Result<()> {
    let root = normalize_repo_path(path);
    let bookmarks: Vec<_> = load_bookmarks()?
        .into_iter()
        .filter(|p| !paths_match(p, &root))
        .collect();
    save_bookmarks(&bookmarks)?;

    let config_path = repo_config_path(&root);
    if config_path.is_file() {
        std::fs::remove_file(&config_path).map_err(|source| LaeError::Write {
            path: config_path,
            source,
        })?;
    }
    Ok(())
}

pub fn repo_display_path(repo: &RegisteredRepo) -> PathBuf {
    normalize_repo_path(&repo.path)
}

/// Registered checkout a task was created from.
pub fn task_source_repo_path(task: &Task) -> &Path {
    task.source_repo_path
        .as_deref()
        .unwrap_or(task.repo_path.as_path())
}

pub fn task_belongs_to_repo(task: &Task, repo: &RegisteredRepo) -> bool {
    paths_match(task_source_repo_path(task), &repo_display_path(repo))
}

pub fn tasks_for_repo<'a>(
    repo: &RegisteredRepo,
    tasks: impl IntoIterator<Item = &'a Task>,
) -> Vec<&'a Task> {
    tasks
        .into_iter()
        .filter(|task| task_belongs_to_repo(task, repo))
        .collect()
}

pub fn ensure_repo_removable(repo: &RegisteredRepo, tasks: &[Task]) -> Result<()> {
    let count = tasks_for_repo(repo, tasks).len();
    if count > 0 {
        return Err(LaeError::Other(format!(
            "Cannot remove \"{}\": {count} task(s) still use this repo — delete them first",
            repo.name
        )));
    }
    Ok(())
}

pub fn collect_task_repo_paths(tasks: impl IntoIterator<Item = impl AsRef<Path>>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for path in tasks {
        let path = normalize_repo_path(path.as_ref());
        let key = bookmark_key(&path);
        if seen.insert(key) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_config_roundtrip_in_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("my-app");
        std::fs::create_dir_all(&checkout).unwrap();
        let repo = RegisteredRepo {
            id: "my-app".into(),
            name: "My App".into(),
            path: checkout.clone(),
            url: Some("https://example.com/app.git".into()),
        };
        save_repo_config(&repo).unwrap();
        let loaded = load_repo_config(&checkout).unwrap().unwrap();
        assert_eq!(loaded.id, "my-app");
        assert_eq!(loaded.url.as_deref(), Some("https://example.com/app.git"));
    }

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

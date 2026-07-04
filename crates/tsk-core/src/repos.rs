//! Registered repositories — checkout-local settings live in `.tsk/repo.toml`.
//!
//! Stable repo ids and checkout paths live in `state.db` (`repos` table).
//! Legacy `~/.config/tsk/repo-bookmarks.txt` paths are migrated into the database on first load.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{TskError, Result};
use crate::models::Task;
use crate::task_paths::scratch_checkout_path;
use crate::vcs::{detect_vcs_root, repo_label, VcsKind};
use crate::xdg::{ensure_parent, expand, tsk_config_dir, tsk_state_db};

const REPOS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS repos (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);
"#;

/// Checkout-local settings persisted in `.tsk/repo.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoConfig {
    /// Optional display name; defaults to the checkout folder name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs: Option<VcsKind>,
    /// Script path relative to the checkout root, run when a task is created from this repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_start: Option<String>,
    /// Optional Hyprland monitor name to focus before running `on_start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_start_monitor: Option<String>,
}

/// Runtime view of a registered checkout (path and id come from `state.db`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRepo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub url: Option<String>,
    pub vcs: Option<VcsKind>,
}

pub fn repo_config_path(vcs_root: &Path) -> PathBuf {
    expand(vcs_root).join(".tsk").join("repo.toml")
}

pub fn repo_bookmarks_path() -> PathBuf {
    tsk_config_dir().join("repo-bookmarks.txt")
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

pub fn repo_id_from_path(path: &Path) -> String {
    let path = normalize_repo_path(path);
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let digest = Sha256::digest(canonical.display().to_string().as_bytes());
    format!("{:x}", digest)[..12].to_string()
}

/// True when the task uses an isolated scratch repo under its task home.
pub fn is_task_scratch_repo(task_id: &str, repo_path: &Path, tasks_base_dir: &Path) -> bool {
    paths_match(
        repo_path,
        &scratch_checkout_path(&tasks_base_dir.join(task_id)),
    )
}

#[derive(Deserialize)]
struct RepoConfigFile {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    vcs: Option<VcsKind>,
    #[serde(default)]
    on_start: Option<String>,
    #[serde(default)]
    on_start_monitor: Option<String>,
    #[serde(default, rename = "id")]
    _legacy_id: Option<String>,
    #[serde(default, rename = "path")]
    _legacy_path: Option<PathBuf>,
}

pub fn load_repo_config(vcs_root: &Path) -> Result<Option<RepoConfig>> {
    let path = repo_config_path(vcs_root);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| TskError::Read {
        path: path.clone(),
        source,
    })?;
    let file: RepoConfigFile =
        toml::from_str(&raw).map_err(|e| TskError::Config(e.to_string()))?;
    let config = RepoConfig {
        name: file.name,
        url: file.url,
        vcs: file.vcs,
        on_start: file.on_start,
        on_start_monitor: file.on_start_monitor,
    };
    Ok(normalize_repo_config(vcs_root, config))
}

fn normalize_repo_config(vcs_root: &Path, mut config: RepoConfig) -> Option<RepoConfig> {
    if config.name.as_deref() == Some(repo_label(vcs_root).as_str()) {
        config.name = None;
    }
    if config == RepoConfig::default() {
        None
    } else {
        Some(config)
    }
}

pub fn save_repo_config(vcs_root: &Path, config: &RepoConfig) -> Result<()> {
    let root = normalize_repo_path(vcs_root);
    let path = repo_config_path(&root);
    let tsk_dir = root.join(".tsk");
    std::fs::create_dir_all(&tsk_dir).map_err(|source| TskError::Write {
        path: tsk_dir,
        source,
    })?;
    if config == &RepoConfig::default() {
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|source| TskError::Write {
                path: path.clone(),
                source,
            })?;
        }
        return Ok(());
    }
    ensure_parent(&path)?;
    let body = toml::to_string_pretty(config).map_err(|e| TskError::Other(e.to_string()))?;
    std::fs::write(&path, body).map_err(|source| TskError::Write { path, source })
}

fn repos_db_conn() -> Result<Connection> {
    let path = tsk_state_db();
    ensure_parent(&path)?;
    let conn = Connection::open(&path).map_err(TskError::from)?;
    conn.execute_batch(REPOS_SCHEMA)?;
    migrate_bookmarks_to_db(&conn)?;
    Ok(conn)
}

fn migrate_bookmarks_to_db(conn: &Connection) -> Result<()> {
    let bookmarks_path = repo_bookmarks_path();
    if !bookmarks_path.is_file() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&bookmarks_path).map_err(|source| TskError::Read {
        path: bookmarks_path.clone(),
        source,
    })?;
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let path = normalize_repo_path(Path::new(line));
        if path.is_dir() {
            upsert_repo_record(conn, &path)?;
        }
    }
    std::fs::remove_file(&bookmarks_path).map_err(|source| TskError::Write {
        path: bookmarks_path,
        source,
    })?;
    Ok(())
}

fn upsert_repo_record(conn: &Connection, path: &Path) -> Result<()> {
    let path = normalize_repo_path(path);
    let id = repo_id_from_path(&path);
    let path_str = path.display().to_string();
    conn.execute(
        "INSERT INTO repos (id, path) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET id = excluded.id",
        params![id, path_str],
    )
    .map_err(TskError::from)?;
    Ok(())
}

fn delete_repo_record(conn: &Connection, path: &Path) -> Result<()> {
    let path = normalize_repo_path(path);
    conn.execute(
        "DELETE FROM repos WHERE path = ?1",
        params![path.display().to_string()],
    )
    .map_err(TskError::from)?;
    Ok(())
}

fn load_repo_records() -> Result<Vec<(String, PathBuf)>> {
    let conn = repos_db_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, path FROM repos ORDER BY path")
        .map_err(TskError::from)?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let path: String = row.get(1)?;
            Ok((id, PathBuf::from(path)))
        })
        .map_err(TskError::from)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(TskError::from)
}

fn build_registered_repo(path: PathBuf, id: String, config: Option<RepoConfig>) -> RegisteredRepo {
    let config = config.unwrap_or_default();
    RegisteredRepo {
        id,
        name: config
            .name
            .clone()
            .unwrap_or_else(|| repo_label(&path)),
        path,
        url: config.url,
        vcs: config.vcs,
    }
}

pub fn load_repos(extra_paths: impl IntoIterator<Item = PathBuf>) -> Result<Vec<RegisteredRepo>> {
    let mut paths: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
    for (id, path) in load_repo_records()? {
        paths.insert(path.display().to_string(), (id, normalize_repo_path(&path)));
    }
    for path in extra_paths {
        let path = normalize_repo_path(&path);
        let key = path.display().to_string();
        paths
            .entry(key)
            .or_insert_with(|| (repo_id_from_path(&path), path));
    }

    let mut repos: Vec<RegisteredRepo> = Vec::new();
    for (id, path) in paths.into_values() {
        if !path.is_dir() {
            continue;
        }
        let config = load_repo_config(&path)?;
        let repo = build_registered_repo(path, id, config);
        if !repos.iter().any(|r| paths_match(&r.path, &repo.path)) {
            repos.push(repo);
        }
    }

    repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(repos)
}

pub fn find_repo<'a>(repos: &'a [RegisteredRepo], id: &str) -> Option<&'a RegisteredRepo> {
    repos.iter().find(|r| r.id == id)
}

pub fn find_repo_by_path<'a>(repos: &'a [RegisteredRepo], path: &Path) -> Option<&'a RegisteredRepo> {
    repos.iter().find(|r| paths_match(&r.path, path))
}

pub fn register_repo(path: &Path, _existing: &[RegisteredRepo]) -> Result<RegisteredRepo> {
    let root = detect_vcs_root(Some(path)).ok_or_else(|| {
        TskError::Other(format!(
            "No git or jj repo found at {}",
            path.display()
        ))
    })?;
    let root = normalize_repo_path(&root);

    let conn = repos_db_conn()?;
    upsert_repo_record(&conn, &root)?;

    let config = load_repo_config(&root)?;
    let id = repo_id_from_path(&root);
    let repo = build_registered_repo(root.clone(), id, config.clone());

    if repo_config_path(&root).is_file() {
        save_repo_config(&root, &config.clone().unwrap_or_default())?;
    }

    Ok(repo)
}

pub fn unregister_repo(path: &Path) -> Result<()> {
    let root = normalize_repo_path(path);
    let conn = repos_db_conn()?;
    delete_repo_record(&conn, &root)?;

    let config_path = repo_config_path(&root);
    if config_path.is_file() {
        std::fs::remove_file(&config_path).map_err(|source| TskError::Write {
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
        return Err(TskError::Other(format!(
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
        let key = path.display().to_string();
        if seen.insert(key) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_db<F: FnOnce()>(f: F) {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        f();
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn repo_config_roundtrip_in_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("my-app");
        std::fs::create_dir_all(&checkout).unwrap();
        let config = RepoConfig {
            name: Some("My App".into()),
            url: Some("https://example.com/app.git".into()),
            vcs: Some(VcsKind::Git),
            on_start: Some(".tsk/on-start.sh".into()),
            on_start_monitor: Some("eDP-1".into()),
        };
        save_repo_config(&checkout, &config).unwrap();
        let loaded = load_repo_config(&checkout).unwrap().unwrap();
        assert_eq!(loaded.name.as_deref(), Some("My App"));
        assert_eq!(loaded.url.as_deref(), Some("https://example.com/app.git"));
        assert_eq!(loaded.vcs, Some(VcsKind::Git));
        assert_eq!(loaded.on_start.as_deref(), Some(".tsk/on-start.sh"));
        assert_eq!(loaded.on_start_monitor.as_deref(), Some("eDP-1"));
    }

    #[test]
    fn repo_config_ignores_legacy_id_and_path_fields() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("legacy-app");
        std::fs::create_dir_all(checkout.join(".tsk")).unwrap();
        std::fs::write(
            repo_config_path(&checkout),
            r#"
id = "legacy-app"
name = "Custom"
path = "/tmp/legacy-app"
"#,
        )
        .unwrap();
        let loaded = load_repo_config(&checkout).unwrap().unwrap();
        assert_eq!(loaded.name.as_deref(), Some("Custom"));
    }

    #[test]
    fn repo_id_is_stable_hash_of_path() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("my-app");
        std::fs::create_dir_all(&checkout).unwrap();
        let id_a = repo_id_from_path(&checkout);
        let id_b = repo_id_from_path(&checkout);
        assert_eq!(id_a, id_b);
        assert_eq!(id_a.len(), 12);
    }

    #[test]
    fn register_repo_persists_in_database() {
        with_temp_db(|| {
            let dir = tempfile::tempdir().unwrap();
            let checkout = dir.path().join("project");
            std::fs::create_dir_all(checkout.join(".git")).unwrap();
            let repo = register_repo(&checkout, &[]).unwrap();
            assert_eq!(repo.name, "project");
            assert_eq!(repo.id, repo_id_from_path(&checkout));
            let loaded = load_repos([]).unwrap();
            assert_eq!(loaded.len(), 1);
            assert!(paths_match(&loaded[0].path, &checkout));
        });
    }
}

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::config::TskConfig;
use crate::error::{TskError, Result};
use crate::models::{
    ContextMode, generate_task_id, SessionState, Task, TaskStatus, WindowRecord,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    context_mode TEXT NOT NULL DEFAULT 'default',
    current_task_id TEXT,
    previous_context TEXT,
    previous_task_id TEXT,
    last_desktop TEXT NOT NULL DEFAULT '{}',
    default_desktop_count INTEGER NOT NULL DEFAULT 3
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    repo_url TEXT,
    repo_path TEXT NOT NULL,
    branch TEXT,
    container_name TEXT NOT NULL,
    desktop_count INTEGER NOT NULL DEFAULT 3,
    browser_profile TEXT,
    created_at TEXT NOT NULL,
    last_active_at TEXT NOT NULL,
    agent_notes_path TEXT,
    ports TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS windows (
    hypr_address TEXT PRIMARY KEY,
    task_id TEXT,
    title TEXT NOT NULL DEFAULT '',
    class TEXT NOT NULL DEFAULT '',
    workspace INTEGER NOT NULL DEFAULT 0,
    workspace_name TEXT NOT NULL DEFAULT '',
    pid INTEGER
);

CREATE TABLE IF NOT EXISTS repos (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);
"#;

pub struct Registry {
    db_path: PathBuf,
    config: TskConfig,
}

impl Registry {
    pub fn new(db_path: Option<PathBuf>, config: TskConfig) -> Result<Self> {
        let db_path = db_path.unwrap_or_else(|| config.state_db_path());
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TskError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let registry = Self { db_path, config };
        registry.init_db()?;
        Ok(registry)
    }

    pub fn with_defaults() -> Result<Self> {
        Self::new(None, crate::config::load_config()?)
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open(&self.db_path).map_err(TskError::from)
    }

    fn init_db(&self) -> Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(SCHEMA)?;
        self.migrate_schema(&conn)?;
        let exists: Option<i32> = conn
            .query_row("SELECT id FROM session WHERE id = 1", [], |row| row.get(0))
            .optional()?;
        if exists.is_none() {
            conn.execute(
                "INSERT INTO session (id, context_mode, last_desktop, default_desktop_count) VALUES (1, 'default', ?, ?)",
                params![
                    r#"{"default":1}"#,
                    self.config.workspaces_per_task as i32,
                ],
            )?;
        }
        Ok(())
    }

    fn migrate_schema(&self, conn: &Connection) -> Result<()> {
        let mut cols = table_columns(conn, "session")?;
        if cols.iter().any(|c| c == "last_workspace") && !cols.iter().any(|c| c == "last_desktop") {
            conn.execute(
                "ALTER TABLE session RENAME COLUMN last_workspace TO last_desktop",
                [],
            )?;
            cols = table_columns(conn, "session")?;
        }
        let win_cols = table_columns(conn, "windows")?;
        if !win_cols.is_empty() && !win_cols.iter().any(|c| c == "workspace_name") {
            conn.execute(
                "ALTER TABLE windows ADD COLUMN workspace_name TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        if !win_cols.is_empty() && !win_cols.iter().any(|c| c == "home_workspace_name") {
            conn.execute(
                "ALTER TABLE windows ADD COLUMN home_workspace_name TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        if !cols.iter().any(|c| c == "last_monitor_workspace") {
            conn.execute(
                "ALTER TABLE session ADD COLUMN last_monitor_workspace TEXT NOT NULL DEFAULT '{}'",
                [],
            )?;
        }
        let task_cols = table_columns(conn, "tasks")?;
        if !task_cols.iter().any(|c| c == "source_repo_path") {
            conn.execute("ALTER TABLE tasks ADD COLUMN source_repo_path TEXT", [])?;
        }
        Ok(())
    }

    pub fn load_state(&self) -> Result<SessionState> {
        let conn = self.connect()?;
        let session = conn.query_row(
            "SELECT context_mode, current_task_id, last_desktop, COALESCE(last_monitor_workspace, '{}') FROM session WHERE id = 1",
            [],
            |row| {
                Ok(SessionRow {
                    context_mode: row.get(0)?,
                    current_task_id: row.get(1)?,
                    last_desktop: row.get(2)?,
                    last_monitor_workspace: row.get(3)?,
                })
            },
        )?;

        let mut tasks = HashMap::new();
        let mut stmt = conn.prepare("SELECT * FROM tasks")?;
        let rows = stmt.query_map([], |row| {
            task_from_row(row).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    e.to_string(),
                )))
            })
        })?;
        for task in rows {
            let task = task?;
            tasks.insert(task.id.clone(), task);
        }

        let mut windows = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT hypr_address, task_id, title, class, workspace, workspace_name, COALESCE(home_workspace_name, ''), pid FROM windows",
        )?;
        let rows = stmt.query_map([], |row| {
            let address: String = row.get(0)?;
            Ok((
                address.clone(),
                WindowRecord {
                    hypr_address: address,
                    task_id: row.get(1)?,
                    title: row.get(2)?,
                    class_name: row.get(3)?,
                    workspace: row.get(4)?,
                    workspace_name: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    home_workspace_name: row.get(6)?,
                    pid: row.get::<_, Option<i32>>(7)?,
                },
            ))
        })?;
        for window in rows {
            let (k, v) = window?;
            windows.insert(k, v);
        }

        let last_workspace: HashMap<String, i32> =
            serde_json::from_str(&session.last_desktop).unwrap_or_default();
        let last_monitor_workspace: HashMap<String, HashMap<String, i32>> =
            serde_json::from_str(&session.last_monitor_workspace).unwrap_or_default();

        Ok(SessionState {
            context_mode: parse_context_mode(&session.context_mode),
            current_task_id: if session.context_mode == "global" {
                None
            } else {
                session.current_task_id
            },
            last_workspace,
            last_monitor_workspace,
            default_workspace_count: self.config.default_workspace_count,
            global_workspace_slots: self.config.global_workspace_slots.clone(),
            tasks,
            windows,
        })
    }

    pub fn save_state(&self, state: &SessionState) -> Result<()> {
        let conn = self.connect()?;
        let last_desktop = serde_json::to_string(&state.last_workspace)
            .map_err(|e| TskError::Other(e.to_string()))?;
        let last_monitor_workspace = serde_json::to_string(&state.last_monitor_workspace)
            .map_err(|e| TskError::Other(e.to_string()))?;
        conn.execute(
            "UPDATE session SET context_mode = ?, current_task_id = ?, previous_context = NULL, previous_task_id = NULL, last_desktop = ?, default_desktop_count = ?, last_monitor_workspace = ? WHERE id = 1",
            params![
                state.context_mode.as_str(),
                state.current_task_id,
                last_desktop,
                state.default_workspace_count as i32,
                last_monitor_workspace,
            ],
        )?;
        conn.execute("DELETE FROM tasks", [])?;
        for task in state.tasks.values() {
            upsert_task(&conn, task)?;
        }
        conn.execute("DELETE FROM windows", [])?;
        for window in state.windows.values() {
            conn.execute(
                "INSERT INTO windows (hypr_address, task_id, title, class, workspace, workspace_name, home_workspace_name, pid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    window.hypr_address,
                    window.task_id,
                    window.title,
                    window.class_name,
                    window.workspace,
                    window.workspace_name,
                    window.home_workspace_name,
                    window.pid,
                ],
            )?;
        }
        Ok(())
    }

    pub fn unique_task_id(&self, state: &SessionState, _name: &str) -> String {
        for _ in 0..256 {
            let candidate = generate_task_id();
            if !state.tasks.contains_key(&candidate) {
                return candidate;
            }
        }
        format!("t{}", uuid_like_suffix())
    }

    pub fn get_task<'a>(&self, state: &'a SessionState, name_or_id: &str) -> Option<&'a Task> {
        if let Some(task) = state.tasks.get(name_or_id) {
            return Some(task);
        }
        let lower = name_or_id.to_lowercase();
        state
            .tasks
            .values()
            .find(|t| t.name.to_lowercase() == lower)
    }

    pub fn touch_task(&self, task: &mut Task) {
        task.last_active_at = Utc::now();
    }
}

struct SessionRow {
    context_mode: String,
    current_task_id: Option<String>,
    last_desktop: String,
    last_monitor_workspace: String,
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(cols)
}

fn parse_context_mode(raw: &str) -> ContextMode {
    match raw {
        "task" => ContextMode::Task,
        "global" => ContextMode::Default,
        _ => ContextMode::Default,
    }
}

fn uuid_like_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

fn task_from_row(row: &rusqlite::Row<'_>) -> Result<Task> {
    let ports_raw: String = row.get(12)?;
    let ports: Vec<u16> = serde_json::from_str(&ports_raw).unwrap_or_default();
    let created_at: String = row.get(9)?;
    let last_active_at: String = row.get(10)?;
    Ok(Task {
        id: row.get(0)?,
        name: row.get(1)?,
        status: match row.get::<_, String>(2)?.as_str() {
            "idle" => TaskStatus::Idle,
            "archived" => TaskStatus::Archived,
            _ => TaskStatus::Active,
        },
        repo_url: row.get(3)?,
        repo_path: PathBuf::from(row.get::<_, String>(4)?),
        branch: row.get(5)?,
        container_name: row.get(6)?,
        workspace_count: row.get::<_, i32>(7)? as u32,
        browser_profile: row.get(8)?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        last_active_at: DateTime::parse_from_rfc3339(&last_active_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        agent_notes_path: row
            .get::<_, Option<String>>(11)?
            .map(PathBuf::from),
        ports,
        source_repo_path: row
            .get::<_, Option<String>>(13)
            .ok()
            .flatten()
            .map(PathBuf::from),
    })
}

fn upsert_task(conn: &Connection, task: &Task) -> Result<()> {
    let ports = serde_json::to_string(&task.ports).map_err(|e| TskError::Other(e.to_string()))?;
    conn.execute(
        "INSERT OR REPLACE INTO tasks (id, name, status, repo_url, repo_path, branch, container_name, desktop_count, browser_profile, created_at, last_active_at, agent_notes_path, ports, source_repo_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            task.id,
            task.name,
            task.status.as_str(),
            task.repo_url,
            task.repo_path.to_string_lossy(),
            task.branch,
            task.container_name,
            task.workspace_count as i32,
            task.browser_profile,
            task.created_at.to_rfc3339(),
            task.last_active_at.to_rfc3339(),
            task.agent_notes_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            ports,
            task.source_repo_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TskConfig;

    #[test]
    fn roundtrip_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db), TskConfig::default()).unwrap();
        let state = registry.load_state().unwrap();
        assert_eq!(state.context_mode, ContextMode::Default);
        registry.save_state(&state).unwrap();
        let again = registry.load_state().unwrap();
        assert_eq!(again.context_mode, ContextMode::Default);
    }
}

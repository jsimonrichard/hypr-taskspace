use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::config::LaeConfig;
use crate::error::{LaeError, Result};
use crate::models::{
    slugify, ContextMode, SessionState, Task, TaskStatus, WindowRecord,
};
use crate::xdg::lae_state_db;

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
"#;

pub struct Registry {
    db_path: PathBuf,
    config: LaeConfig,
}

impl Registry {
    pub fn new(db_path: Option<PathBuf>, config: LaeConfig) -> Result<Self> {
        let db_path = db_path.unwrap_or_else(lae_state_db);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| LaeError::Write {
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
        Connection::open(&self.db_path).map_err(LaeError::from)
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
        let cols = table_columns(conn, "session")?;
        if cols.iter().any(|c| c == "last_workspace") && !cols.iter().any(|c| c == "last_desktop") {
            conn.execute(
                "ALTER TABLE session RENAME COLUMN last_workspace TO last_desktop",
                [],
            )?;
        }
        let win_cols = table_columns(conn, "windows")?;
        if !win_cols.is_empty() && !win_cols.iter().any(|c| c == "workspace_name") {
            conn.execute(
                "ALTER TABLE windows ADD COLUMN workspace_name TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        Ok(())
    }

    pub fn load_state(&self) -> Result<SessionState> {
        let conn = self.connect()?;
        let session = conn.query_row("SELECT * FROM session WHERE id = 1", [], |row| {
            Ok(SessionRow {
                context_mode: row.get(1)?,
                current_task_id: row.get(2)?,
                last_desktop: row.get(5)?,
            })
        })?;

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
            "SELECT hypr_address, task_id, title, class, workspace, workspace_name, pid FROM windows",
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
                    pid: row.get::<_, Option<i32>>(6)?,
                },
            ))
        })?;
        for window in rows {
            let (k, v) = window?;
            windows.insert(k, v);
        }

        let last_workspace: HashMap<String, i32> =
            serde_json::from_str(&session.last_desktop).unwrap_or_default();

        Ok(SessionState {
            context_mode: parse_context_mode(&session.context_mode),
            current_task_id: if session.context_mode == "global" {
                None
            } else {
                session.current_task_id
            },
            last_workspace,
            default_workspace_count: self.config.default_workspace_count,
            tasks,
            windows,
        })
    }

    pub fn save_state(&self, state: &SessionState) -> Result<()> {
        let conn = self.connect()?;
        let last_desktop = serde_json::to_string(&state.last_workspace)
            .map_err(|e| LaeError::Other(e.to_string()))?;
        conn.execute(
            "UPDATE session SET context_mode = ?, current_task_id = ?, previous_context = NULL, previous_task_id = NULL, last_desktop = ?, default_desktop_count = ? WHERE id = 1",
            params![
                state.context_mode.as_str(),
                state.current_task_id,
                last_desktop,
                state.default_workspace_count as i32,
            ],
        )?;
        conn.execute("DELETE FROM tasks", [])?;
        for task in state.tasks.values() {
            upsert_task(&conn, task)?;
        }
        conn.execute("DELETE FROM windows", [])?;
        for window in state.windows.values() {
            conn.execute(
                "INSERT INTO windows (hypr_address, task_id, title, class, workspace, workspace_name, pid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    window.hypr_address,
                    window.task_id,
                    window.title,
                    window.class_name,
                    window.workspace,
                    window.workspace_name,
                    window.pid,
                ],
            )?;
        }
        Ok(())
    }

    pub fn unique_task_id(&self, state: &SessionState, name: &str) -> String {
        let base = slugify(name);
        let mut candidate = base.clone();
        let mut n = 2;
        while state.tasks.contains_key(&candidate)
            && state.tasks.get(&candidate).is_some_and(|t| t.status != TaskStatus::Archived)
        {
            candidate = format!("{base}-{n}");
            n += 1;
        }
        candidate
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
    })
}

fn upsert_task(conn: &Connection, task: &Task) -> Result<()> {
    let ports = serde_json::to_string(&task.ports).map_err(|e| LaeError::Other(e.to_string()))?;
    conn.execute(
        "INSERT OR REPLACE INTO tasks (id, name, status, repo_url, repo_path, branch, container_name, desktop_count, browser_profile, created_at, last_active_at, agent_notes_path, ports) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LaeConfig;

    #[test]
    fn roundtrip_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db), LaeConfig::default()).unwrap();
        let state = registry.load_state().unwrap();
        assert_eq!(state.context_mode, ContextMode::Default);
        registry.save_state(&state).unwrap();
        let again = registry.load_state().unwrap();
        assert_eq!(again.context_mode, ContextMode::Default);
    }
}

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::config::TskConfig;
use crate::error::{Result, TskError};
use crate::models::{generate_task_id, ContextMode, SessionState, Task, TaskStatus, WindowRecord};
use crate::task_ids::{lookup_task, TaskLookup};

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
    ports TEXT NOT NULL DEFAULT '[]',
    source_repo_path TEXT,
    container_isolation INTEGER NOT NULL DEFAULT 0,
    listed_at TEXT NOT NULL
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

/// Columns `migrate_schema` and `load_state`/`save_state` require after init.
/// `check_schema` uses this list so doctor and install agree with migrate.
const REQUIRED_COLUMNS: &[(&str, &[&str])] = &[
    (
        "session",
        &[
            "id",
            "context_mode",
            "current_task_id",
            "last_desktop",
            "default_desktop_count",
            "last_monitor_workspace",
        ],
    ),
    (
        "tasks",
        &[
            "id",
            "name",
            "status",
            "repo_path",
            "container_name",
            "created_at",
            "last_active_at",
            "source_repo_path",
            "container_isolation",
            "listed_at",
        ],
    ),
    (
        "windows",
        &["hypr_address", "workspace_name", "home_workspace_name"],
    ),
    ("repos", &["id", "path"]),
];

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
        Ok(Self { db_path, config })
    }

    pub fn with_defaults() -> Result<Self> {
        Self::new(None, crate::config::load_config()?)
    }

    /// Create tables and apply migrations. Daemon start, `tsk install`, and tests.
    pub fn ensure_schema(&self) -> Result<()> {
        self.init_db()
    }

    /// Read-only: required tables and columns exist. Does not create the file.
    pub fn check_schema(&self) -> Result<()> {
        if !self.db_path.is_file() {
            return Err(TskError::SchemaMissing {
                path: self.db_path.clone(),
            });
        }
        let conn = self.connect()?;
        for (table, required) in REQUIRED_COLUMNS {
            let existing = table_columns(&conn, table)?;
            if existing.is_empty() {
                return Err(TskError::SchemaMissing {
                    path: self.db_path.clone(),
                });
            }
            for col in *required {
                if !existing.iter().any(|c| c == col) {
                    return Err(TskError::SchemaMissing {
                        path: self.db_path.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open(&self.db_path).map_err(TskError::from)
    }

    fn schema_error(&self, source: rusqlite::Error) -> TskError {
        if source.to_string().contains("no such table") {
            TskError::SchemaMissing {
                path: self.db_path.clone(),
            }
        } else {
            TskError::Database(source)
        }
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
        let task_cols = table_columns(conn, "tasks")?;
        if !task_cols.iter().any(|c| c == "container_isolation") {
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN container_isolation INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        let task_cols = table_columns(conn, "tasks")?;
        if !task_cols.iter().any(|c| c == "listed_at") {
            conn.execute("ALTER TABLE tasks ADD COLUMN listed_at TEXT", [])?;
            conn.execute(
                "UPDATE tasks SET listed_at = created_at WHERE listed_at IS NULL OR listed_at = ''",
                [],
            )?;
        }
        Ok(())
    }

    pub fn load_state(&self) -> Result<SessionState> {
        let conn = self.connect()?;
        let session = load_session(&conn).map_err(|source| self.schema_error(source))?;
        let tasks = load_tasks(&conn)?;
        let windows = load_windows(&conn)?;

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
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        if write_session(&tx, state)? == 0 {
            return Err(TskError::SchemaMissing {
                path: self.db_path.clone(),
            });
        }
        sync_tasks(&tx, &state.tasks)?;
        sync_windows(&tx, &state.windows)?;
        tx.commit()?;
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
        match lookup_task(state, name_or_id) {
            TaskLookup::Found(task) => Some(task),
            TaskLookup::NotFound | TaskLookup::AmbiguousPrefix(_) => None,
        }
    }

    pub fn lookup_task<'a>(&self, state: &'a SessionState, name_or_id: &str) -> TaskLookup<'a> {
        lookup_task(state, name_or_id)
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

fn load_session(conn: &Connection) -> rusqlite::Result<SessionRow> {
    conn.query_row(
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
    )
}

fn load_tasks(conn: &Connection) -> Result<HashMap<String, Task>> {
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
    Ok(tasks)
}

fn load_windows(conn: &Connection) -> Result<HashMap<String, WindowRecord>> {
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
    Ok(windows)
}

fn write_session(conn: &Connection, state: &SessionState) -> Result<usize> {
    let last_desktop =
        serde_json::to_string(&state.last_workspace).map_err(|e| TskError::Other(e.to_string()))?;
    let last_monitor_workspace = serde_json::to_string(&state.last_monitor_workspace)
        .map_err(|e| TskError::Other(e.to_string()))?;
    Ok(conn.execute(
        "UPDATE session SET context_mode = ?, current_task_id = ?, previous_context = NULL, previous_task_id = NULL, last_desktop = ?, default_desktop_count = ?, last_monitor_workspace = ? WHERE id = 1",
        params![
            state.context_mode.as_str(),
            state.current_task_id,
            last_desktop,
            state.default_workspace_count as i32,
            last_monitor_workspace,
        ],
    )?)
}

fn sync_tasks(conn: &Connection, desired: &HashMap<String, Task>) -> Result<()> {
    let existing = load_tasks(conn)?;
    let keep: Vec<&String> = desired.values().map(|task| &task.id).collect();
    for task in desired.values() {
        if existing.get(&task.id) != Some(task) {
            upsert_task(conn, task)?;
        }
    }
    delete_absent(conn, "tasks", "id", keep)
}

fn sync_windows(conn: &Connection, desired: &HashMap<String, WindowRecord>) -> Result<()> {
    let existing = load_windows(conn)?;
    let keep: Vec<&String> = desired
        .values()
        .map(|window| &window.hypr_address)
        .collect();
    for window in desired.values() {
        if existing.get(&window.hypr_address) != Some(window) {
            upsert_window(conn, window)?;
        }
    }
    delete_absent(conn, "windows", "hypr_address", keep)
}

fn delete_absent<'a>(
    conn: &Connection,
    table: &str,
    pk: &str,
    keep: impl IntoIterator<Item = &'a String>,
) -> Result<()> {
    let keep: Vec<&'a String> = keep.into_iter().collect();
    let json = serde_json::to_string(&keep).map_err(|e| TskError::Other(e.to_string()))?;
    let sql = format!("DELETE FROM {table} WHERE {pk} NOT IN (SELECT value FROM json_each(?1))");
    conn.execute(&sql, params![json])?;
    Ok(())
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
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let last_active_at = DateTime::parse_from_rfc3339(&last_active_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let listed_at = row
        .get::<_, Option<String>>("listed_at")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(created_at);
    let container_isolation = row.get::<_, Option<i32>>(14).ok().flatten().unwrap_or(0) != 0;
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
        container_isolation,
        workspace_count: row.get::<_, i32>(7)? as u32,
        browser_profile: row.get(8)?,
        created_at,
        last_active_at,
        listed_at,
        agent_notes_path: row.get::<_, Option<String>>(11)?.map(PathBuf::from),
        ports,
        source_repo_path: row
            .get::<_, Option<String>>(13)
            .ok()
            .flatten()
            .map(PathBuf::from),
    })
}

fn upsert_window(conn: &Connection, window: &WindowRecord) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO windows (hypr_address, task_id, title, class, workspace, workspace_name, home_workspace_name, pid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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
    Ok(())
}

fn upsert_task(conn: &Connection, task: &Task) -> Result<()> {
    let ports = serde_json::to_string(&task.ports).map_err(|e| TskError::Other(e.to_string()))?;
    conn.execute(
        "INSERT OR REPLACE INTO tasks (id, name, status, repo_url, repo_path, branch, container_name, desktop_count, browser_profile, created_at, last_active_at, agent_notes_path, ports, source_repo_path, container_isolation, listed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            if task.container_isolation { 1 } else { 0 },
            task.listed_at.to_rfc3339(),
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
        registry.ensure_schema().unwrap();
        let state = registry.load_state().unwrap();
        assert_eq!(state.context_mode, ContextMode::Default);
        registry.save_state(&state).unwrap();
        let again = registry.load_state().unwrap();
        assert_eq!(again.context_mode, ContextMode::Default);
    }

    #[test]
    fn migrate_listed_at_from_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE session (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    context_mode TEXT NOT NULL DEFAULT 'default',
                    current_task_id TEXT,
                    previous_context TEXT,
                    previous_task_id TEXT,
                    last_desktop TEXT NOT NULL DEFAULT '{}',
                    default_desktop_count INTEGER NOT NULL DEFAULT 3
                );
                CREATE TABLE tasks (
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
                    ports TEXT NOT NULL DEFAULT '[]',
                    source_repo_path TEXT,
                    container_isolation INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE windows (
                    hypr_address TEXT PRIMARY KEY,
                    task_id TEXT,
                    title TEXT NOT NULL DEFAULT '',
                    class TEXT NOT NULL DEFAULT '',
                    workspace INTEGER NOT NULL DEFAULT 0,
                    workspace_name TEXT NOT NULL DEFAULT '',
                    pid INTEGER
                );
                INSERT INTO session (id, context_mode, last_desktop, default_desktop_count)
                VALUES (1, 'default', '{"default":1}', 3);
                INSERT INTO tasks (id, name, status, repo_path, container_name, created_at, last_active_at)
                VALUES ('t1', 'old', 'active', '/tmp', 'tsk-t1', '2020-01-01T00:00:00Z', '2020-01-02T00:00:00Z');
                "#,
            )
            .unwrap();
        }
        let registry = Registry::new(Some(db), TskConfig::default()).unwrap();
        registry.ensure_schema().unwrap();
        let state = registry.load_state().unwrap();
        let task = state.tasks.get("t1").expect("migrated task");
        assert_eq!(task.created_at.to_rfc3339(), "2020-01-01T00:00:00+00:00");
        assert_eq!(task.listed_at, task.created_at);
    }

    #[test]
    fn load_state_without_schema_is_schema_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db.clone()), TskConfig::default()).unwrap();
        let err = registry.load_state().unwrap_err();
        match err {
            TskError::SchemaMissing { path } => assert_eq!(path, db),
            other => panic!("expected SchemaMissing, got {other}"),
        }
    }

    #[test]
    fn check_schema_without_file_is_schema_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db.clone()), TskConfig::default()).unwrap();
        let err = registry.check_schema().unwrap_err();
        match err {
            TskError::SchemaMissing { path } => assert_eq!(path, db),
            other => panic!("expected SchemaMissing, got {other}"),
        }
        assert!(!db.is_file(), "check_schema must not create the database");
    }

    #[test]
    fn check_schema_passes_after_ensure() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db), TskConfig::default()).unwrap();
        registry.ensure_schema().unwrap();
        registry.check_schema().unwrap();
    }

    #[test]
    fn check_schema_fails_when_required_column_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE session (id INTEGER PRIMARY KEY);
                CREATE TABLE tasks (id TEXT PRIMARY KEY);
                CREATE TABLE windows (hypr_address TEXT PRIMARY KEY);
                CREATE TABLE repos (id TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE);
                "#,
            )
            .unwrap();
        }
        let registry = Registry::new(Some(db.clone()), TskConfig::default()).unwrap();
        let err = registry.check_schema().unwrap_err();
        match err {
            TskError::SchemaMissing { path } => assert_eq!(path, db),
            other => panic!("expected SchemaMissing, got {other}"),
        }
    }

    fn sample_task(id: &str, name: &str) -> Task {
        let now = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        Task {
            id: id.into(),
            name: name.into(),
            status: TaskStatus::Active,
            repo_url: None,
            repo_path: PathBuf::from("/tmp"),
            source_repo_path: None,
            branch: None,
            container_name: format!("tsk-{id}"),
            container_isolation: false,
            workspace_count: 3,
            browser_profile: None,
            created_at: now,
            last_active_at: now,
            listed_at: now,
            agent_notes_path: None,
            ports: vec![],
        }
    }

    fn sample_window(address: &str, title: &str) -> WindowRecord {
        WindowRecord {
            hypr_address: address.into(),
            task_id: Some("t1".into()),
            title: title.into(),
            class_name: "term".into(),
            workspace: 1,
            workspace_name: "1".into(),
            home_workspace_name: "1".into(),
            pid: Some(1),
        }
    }

    fn registry_with_schema() -> (tempfile::TempDir, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db), TskConfig::default()).unwrap();
        registry.ensure_schema().unwrap();
        (dir, registry)
    }

    #[test]
    fn save_state_updates_one_task_without_dropping_others() {
        let (_dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.tasks.insert("t1".into(), sample_task("t1", "one"));
        state.tasks.insert("t2".into(), sample_task("t2", "two"));
        registry.save_state(&state).unwrap();

        state.tasks.get_mut("t1").unwrap().name = "one-renamed".into();
        registry.save_state(&state).unwrap();

        let again = registry.load_state().unwrap();
        assert_eq!(again.tasks["t1"].name, "one-renamed");
        assert_eq!(again.tasks["t2"].name, "two");
    }

    #[test]
    fn save_state_deletes_removed_task_and_window() {
        let (_dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.tasks.insert("t1".into(), sample_task("t1", "one"));
        state.tasks.insert("t2".into(), sample_task("t2", "two"));
        state
            .windows
            .insert("0x1".into(), sample_window("0x1", "a"));
        state
            .windows
            .insert("0x2".into(), sample_window("0x2", "b"));
        registry.save_state(&state).unwrap();

        state.tasks.remove("t2");
        state.windows.remove("0x2");
        registry.save_state(&state).unwrap();

        let again = registry.load_state().unwrap();
        assert!(again.tasks.contains_key("t1"));
        assert!(!again.tasks.contains_key("t2"));
        assert!(again.windows.contains_key("0x1"));
        assert!(!again.windows.contains_key("0x2"));
    }

    #[test]
    fn save_state_without_session_row_is_schema_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let registry = Registry::new(Some(db.clone()), TskConfig::default()).unwrap();
        registry.ensure_schema().unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute("DELETE FROM session", []).unwrap();
        }
        let state = SessionState::default();
        let err = registry.save_state(&state).unwrap_err();
        match err {
            TskError::SchemaMissing { path } => assert_eq!(path, db),
            other => panic!("expected SchemaMissing, got {other}"),
        }
    }

    fn db_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("state.db")
    }

    fn task_rowids(path: &std::path::Path) -> HashMap<String, i64> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn.prepare("SELECT id, rowid FROM tasks").unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    fn window_rowids(path: &std::path::Path) -> HashMap<String, i64> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn
            .prepare("SELECT hypr_address, rowid FROM windows")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    fn rich_task() -> Task {
        let created = DateTime::parse_from_rfc3339("2020-01-01T00:00:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);
        let active = DateTime::parse_from_rfc3339("2021-06-15T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        Task {
            id: "tabc".into(),
            name: "rich".into(),
            status: TaskStatus::Archived,
            repo_url: Some("git@example.com:org/repo.git".into()),
            repo_path: PathBuf::from("/tmp/task"),
            source_repo_path: Some(PathBuf::from("/tmp/src")),
            branch: Some("feat/x".into()),
            container_name: "tsk-tabc".into(),
            container_isolation: true,
            workspace_count: 7,
            browser_profile: Some("task-tabc".into()),
            created_at: created,
            last_active_at: active,
            listed_at: created,
            agent_notes_path: Some(PathBuf::from("/tmp/notes.md")),
            ports: vec![8080, 9090],
        }
    }

    #[test]
    fn save_state_roundtrips_session_task_and_window_fields() {
        let (_dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.context_mode = ContextMode::Task;
        state.current_task_id = Some("tabc".into());
        state.last_workspace.insert("tabc".into(), 3);
        state
            .last_monitor_workspace
            .insert("tabc".into(), HashMap::from([("DP-1".into(), 2)]));
        let task = rich_task();
        state.tasks.insert(task.id.clone(), task.clone());
        state.windows.insert(
            "0xabc".into(),
            WindowRecord {
                hypr_address: "0xabc".into(),
                task_id: None,
                title: "untitled".into(),
                class_name: "chromium".into(),
                workspace: 4,
                workspace_name: "tabc-4".into(),
                home_workspace_name: "tabc-1".into(),
                pid: None,
            },
        );
        registry.save_state(&state).unwrap();

        let again = registry.load_state().unwrap();
        assert_eq!(again.context_mode, ContextMode::Task);
        assert_eq!(again.current_task_id.as_deref(), Some("tabc"));
        assert_eq!(again.last_workspace.get("tabc"), Some(&3));
        assert_eq!(again.last_monitor_workspace["tabc"].get("DP-1"), Some(&2));
        assert_eq!(again.tasks["tabc"], task);
        assert_eq!(again.windows["0xabc"].task_id, None);
        assert_eq!(again.windows["0xabc"].pid, None);
        assert_eq!(again.windows["0xabc"].home_workspace_name, "tabc-1");
        assert_eq!(again.windows["0xabc"].workspace_name, "tabc-4");
    }

    #[test]
    fn save_state_mixed_insert_update_and_delete() {
        let (_dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.tasks.insert("t1".into(), sample_task("t1", "one"));
        state.tasks.insert("t2".into(), sample_task("t2", "two"));
        state
            .windows
            .insert("0x1".into(), sample_window("0x1", "a"));
        state
            .windows
            .insert("0x2".into(), sample_window("0x2", "b"));
        registry.save_state(&state).unwrap();

        state.tasks.remove("t1");
        state.tasks.get_mut("t2").unwrap().name = "two-renamed".into();
        state.tasks.insert("t3".into(), sample_task("t3", "three"));
        state.windows.remove("0x1");
        state.windows.get_mut("0x2").unwrap().title = "b-renamed".into();
        state
            .windows
            .insert("0x3".into(), sample_window("0x3", "c"));
        registry.save_state(&state).unwrap();

        let again = registry.load_state().unwrap();
        assert_eq!(
            again
                .tasks
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["t2".into(), "t3".into()])
        );
        assert_eq!(again.tasks["t2"].name, "two-renamed");
        assert_eq!(again.tasks["t3"].name, "three");
        assert_eq!(
            again
                .windows
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["0x2".into(), "0x3".into()])
        );
        assert_eq!(again.windows["0x2"].title, "b-renamed");
        assert_eq!(again.windows["0x3"].title, "c");
    }

    #[test]
    fn save_state_clears_tasks_and_windows_when_maps_are_empty() {
        let (_dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.tasks.insert("t1".into(), sample_task("t1", "one"));
        state
            .windows
            .insert("0x1".into(), sample_window("0x1", "a"));
        registry.save_state(&state).unwrap();

        state.tasks.clear();
        state.windows.clear();
        registry.save_state(&state).unwrap();

        let again = registry.load_state().unwrap();
        assert!(again.tasks.is_empty());
        assert!(again.windows.is_empty());
    }

    #[test]
    fn save_state_skips_replace_when_loaded_rows_are_unchanged() {
        let (dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.tasks.insert("t1".into(), sample_task("t1", "one"));
        let rich = rich_task();
        state.tasks.insert(rich.id.clone(), rich);
        state
            .windows
            .insert("0x1".into(), sample_window("0x1", "a"));
        registry.save_state(&state).unwrap();

        let path = db_path(&dir);
        let tasks_before = task_rowids(&path);
        let windows_before = window_rowids(&path);

        let loaded = registry.load_state().unwrap();
        registry.save_state(&loaded).unwrap();

        assert_eq!(task_rowids(&path), tasks_before);
        assert_eq!(window_rowids(&path), windows_before);
        let again = registry.load_state().unwrap();
        assert_eq!(again.tasks["t1"].name, "one");
        assert_eq!(again.tasks["tabc"], rich_task());
        assert_eq!(again.windows["0x1"].title, "a");
    }

    #[test]
    fn save_state_keeps_rows_by_task_id_not_map_key() {
        let (_dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        let task = sample_task("treal", "real");
        state.tasks.insert("wrong-key".into(), task);
        state
            .windows
            .insert("wrong-key".into(), sample_window("0xreal", "w"));
        registry.save_state(&state).unwrap();

        let again = registry.load_state().unwrap();
        assert!(again.tasks.contains_key("treal"));
        assert!(!again.tasks.contains_key("wrong-key"));
        assert!(again.windows.contains_key("0xreal"));
        assert!(!again.windows.contains_key("wrong-key"));
    }

    #[test]
    fn save_state_rolls_back_when_a_task_upsert_fails() {
        let (dir, registry) = registry_with_schema();
        let mut state = registry.load_state().unwrap();
        state.tasks.insert("t1".into(), sample_task("t1", "one"));
        state
            .windows
            .insert("0x1".into(), sample_window("0x1", "keep"));
        registry.save_state(&state).unwrap();

        {
            let conn = Connection::open(db_path(&dir)).unwrap();
            conn.execute_batch(
                r#"
                CREATE TRIGGER abort_bad BEFORE INSERT ON tasks
                BEGIN
                    SELECT RAISE(ABORT, 'bad task') WHERE NEW.id = 'bad';
                END;
                "#,
            )
            .unwrap();
        }

        state.tasks.get_mut("t1").unwrap().name = "changed".into();
        state.tasks.insert("bad".into(), sample_task("bad", "nope"));
        state.windows.get_mut("0x1").unwrap().title = "changed".into();
        let err = registry.save_state(&state).unwrap_err();
        assert!(
            matches!(err, TskError::Database(_)),
            "expected Database error, got {err}"
        );

        let again = registry.load_state().unwrap();
        assert_eq!(again.tasks.len(), 1);
        assert_eq!(again.tasks["t1"].name, "one");
        assert!(!again.tasks.contains_key("bad"));
        assert_eq!(again.windows["0x1"].title, "keep");
    }
}

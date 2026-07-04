"""SQLite-backed task registry and session state."""

from __future__ import annotations

import json
import sqlite3
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

from tsk.core.config import TskConfig, load_config
from tsk.core.models import (
    ContextMode,
    SessionState,
    Task,
    TaskStatus,
    WindowRecord,
    slugify,
    task_from_row,
)
from tsk.util import xdg


SCHEMA = """
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
"""


class Registry:
    def __init__(self, db_path: Path | None = None, config: TskConfig | None = None):
        self.db_path = db_path or xdg.tsk_state_db()
        self.config = config or load_config()
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._init_db()

    @contextmanager
    def _conn(self):
        conn = sqlite3.connect(self.db_path)
        conn.row_factory = sqlite3.Row
        try:
            yield conn
            conn.commit()
        finally:
            conn.close()

    def _migrate_schema(self, conn: sqlite3.Connection) -> None:
        cols = {row[1] for row in conn.execute("PRAGMA table_info(session)").fetchall()}
        if "last_workspace" in cols and "last_desktop" not in cols:
            conn.execute(
                "ALTER TABLE session RENAME COLUMN last_workspace TO last_desktop"
            )
        if "default_workspace_range" in cols and "default_desktop_count" not in cols:
            conn.execute(
                "ALTER TABLE session ADD COLUMN default_desktop_count INTEGER NOT NULL DEFAULT 3"
            )

        task_cols = {row[1] for row in conn.execute("PRAGMA table_info(tasks)").fetchall()}
        if task_cols and "desktop_count" not in task_cols:
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN desktop_count INTEGER NOT NULL DEFAULT 3"
            )
        if "workspace_range" in task_cols or "workspaces" in task_cols:
            self._rebuild_tasks_table(conn)

        win_cols = {row[1] for row in conn.execute("PRAGMA table_info(windows)").fetchall()}
        if win_cols and "workspace_name" not in win_cols:
            conn.execute(
                "ALTER TABLE windows ADD COLUMN workspace_name TEXT NOT NULL DEFAULT ''"
            )

    def _rebuild_tasks_table(self, conn: sqlite3.Connection) -> None:
        rows = conn.execute("SELECT * FROM tasks").fetchall()
        conn.execute("ALTER TABLE tasks RENAME TO tasks_legacy")
        conn.execute(
            """
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
                ports TEXT NOT NULL DEFAULT '[]'
            )
            """
        )
        for row in rows:
            data = dict(row)
            import json as _json

            desktop_count = data.get("desktop_count")
            if desktop_count is None:
                legacy_ws = _json.loads(data.get("workspaces") or "{}")
                desktop_count = len(legacy_ws) if legacy_ws else 3
            conn.execute(
                """
                INSERT INTO tasks (
                    id, name, status, repo_url, repo_path, branch, container_name,
                    desktop_count, browser_profile, created_at, last_active_at,
                    agent_notes_path, ports
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    data["id"],
                    data["name"],
                    data["status"],
                    data.get("repo_url"),
                    data["repo_path"],
                    data.get("branch"),
                    data["container_name"],
                    int(desktop_count),
                    data.get("browser_profile"),
                    data["created_at"],
                    data["last_active_at"],
                    data.get("agent_notes_path"),
                    data.get("ports") or "[]",
                ),
            )
        conn.execute("DROP TABLE tasks_legacy")

    def _init_db(self) -> None:
        with self._conn() as conn:
            conn.executescript(SCHEMA)
            self._migrate_schema(conn)
            row = conn.execute("SELECT id FROM session WHERE id = 1").fetchone()
            if row is None:
                conn.execute(
                    """
                    INSERT INTO session (id, context_mode, last_desktop, default_desktop_count)
                    VALUES (1, 'default', ?, ?)
                    """,
                    (
                        json.dumps({"default": 1}),
                        self.config.workspaces_per_task,
                    ),
                )

    def load_state(self) -> SessionState:
        with self._conn() as conn:
            session = conn.execute("SELECT * FROM session WHERE id = 1").fetchone()
            tasks = {
                row["id"]: task_from_row(dict(row))
                for row in conn.execute("SELECT * FROM tasks")
            }
            windows = {
                row["hypr_address"]: WindowRecord(
                    hypr_address=row["hypr_address"],
                    task_id=row["task_id"],
                    title=row["title"],
                    **{"class": row["class"]},
                    workspace=row["workspace"],
                    workspace_name=row["workspace_name"] if "workspace_name" in row.keys() else "",
                    pid=row["pid"],
                )
                for row in conn.execute("SELECT * FROM windows")
            }

        last_key = "last_desktop" if "last_desktop" in session.keys() else "last_workspace"

        context_mode_raw = session["context_mode"]
        if context_mode_raw == "global":
            context_mode = ContextMode.default
            current_task_id = None
        else:
            context_mode = ContextMode(context_mode_raw)
            current_task_id = session["current_task_id"]

        return SessionState(
            context_mode=context_mode,
            current_task_id=current_task_id,
            last_workspace=json.loads(session[last_key]),
            default_workspace_count=self.config.default_workspace_count,
            tasks=tasks,
            windows=windows,
        )

    def save_state(self, state: SessionState) -> None:
        with self._conn() as conn:
            conn.execute(
                """
                UPDATE session SET
                    context_mode = ?,
                    current_task_id = ?,
                    previous_context = NULL,
                    previous_task_id = NULL,
                    last_desktop = ?,
                    default_desktop_count = ?
                WHERE id = 1
                """,
                (
                    state.context_mode.value,
                    state.current_task_id,
                    json.dumps(state.last_workspace),
                    state.default_workspace_count,
                ),
            )
            conn.execute("DELETE FROM tasks")
            for task in state.tasks.values():
                self._upsert_task_conn(conn, task)
            conn.execute("DELETE FROM windows")
            for window in state.windows.values():
                conn.execute(
                    """
                    INSERT INTO windows (
                        hypr_address, task_id, title, class, workspace, workspace_name, pid
                    ) VALUES (?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        window.hypr_address,
                        window.task_id,
                        window.title,
                        window.class_,
                        window.workspace,
                        window.workspace_name,
                        window.pid,
                    ),
                )

    def _upsert_task_conn(self, conn: sqlite3.Connection, task: Task) -> None:
        conn.execute(
            """
            INSERT OR REPLACE INTO tasks (
                id, name, status, repo_url, repo_path, branch, container_name,
                desktop_count, browser_profile, created_at,
                last_active_at, agent_notes_path, ports
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                task.id,
                task.name,
                task.status.value,
                task.repo_url,
                str(task.repo_path),
                task.branch,
                task.container_name,
                task.workspace_count,
                task.browser_profile,
                task.created_at.isoformat(),
                task.last_active_at.isoformat(),
                str(task.agent_notes_path) if task.agent_notes_path else None,
                json.dumps(task.ports),
            ),
        )

    def unique_task_id(self, state: SessionState, name: str) -> str:
        base = slugify(name)
        candidate = base
        n = 2
        while candidate in state.tasks and state.tasks[candidate].status != TaskStatus.archived:
            candidate = f"{base}-{n}"
            n += 1
        return candidate

    def get_task(self, state: SessionState, name_or_id: str) -> Task | None:
        if name_or_id in state.tasks:
            return state.tasks[name_or_id]
        for task in state.tasks.values():
            if task.name.lower() == name_or_id.lower():
                return task
        return None

    def touch_task(self, task: Task) -> None:
        task.last_active_at = datetime.now(timezone.utc)

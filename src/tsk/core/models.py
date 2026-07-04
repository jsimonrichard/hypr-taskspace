"""Pydantic models for tasks, windows, and session state."""

from __future__ import annotations

from datetime import datetime, timezone
from enum import Enum
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field

from tsk.core.workspaces import task_workspace_names


class ContextMode(str, Enum):
    default = "default"
    task = "task"


class TaskStatus(str, Enum):
    active = "active"
    idle = "idle"
    archived = "archived"


class Task(BaseModel):
    id: str
    name: str
    status: TaskStatus = TaskStatus.active
    repo_url: str | None = None
    repo_path: Path
    branch: str | None = None
    container_name: str
    workspace_count: int = 3
    browser_profile: str | None = None
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    last_active_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    agent_notes_path: Path | None = None
    ports: list[int] = Field(default_factory=list)

    def workspace_names(self) -> list[str]:
        return task_workspace_names(self.id, self.workspace_count)

    def main_workspace(self) -> str:
        return self.workspace_names()[0]

    def workspace_name_at(self, relative: int) -> str:
        names = self.workspace_names()
        if relative < 1 or relative > len(names):
            raise IndexError(
                f"Workspace {relative} out of range for task {self.id}"
            )
        return names[relative - 1]

    @property
    def desktop_count(self) -> int:
        return self.workspace_count


class WindowRecord(BaseModel):
    hypr_address: str
    task_id: str | None = None
    title: str = ""
    class_: str = Field(default="", alias="class")
    workspace: int = 0
    workspace_name: str = ""
    pid: int | None = None

    model_config = {"populate_by_name": True}


class SessionState(BaseModel):
    context_mode: ContextMode = ContextMode.default
    current_task_id: str | None = None
    # Relative workspace index (1-based) per taskspace key
    last_workspace: dict[str, int] = Field(default_factory=dict)
    default_workspace_count: int = 10
    tasks: dict[str, Task] = Field(default_factory=dict)
    windows: dict[str, WindowRecord] = Field(default_factory=dict)

    @property
    def last_desktop(self) -> dict[str, int]:
        return self.last_workspace

    @last_desktop.setter
    def last_desktop(self, value: dict[str, int]) -> None:
        self.last_workspace = value

    @property
    def default_desktop_count(self) -> int:
        return self.default_workspace_count

    @default_desktop_count.setter
    def default_desktop_count(self, value: int) -> None:
        self.default_workspace_count = value

    def taskspace_key(self) -> str:
        if self.context_mode == ContextMode.task and self.current_task_id:
            return f"task:{self.current_task_id}"
        return self.context_mode.value

    def taskspace_label(self) -> str:
        if self.context_mode == ContextMode.task and self.current_task_id:
            return f"task:{self.current_task_id}"
        return self.context_mode.value

    def context_key(self) -> str:
        return self.taskspace_key()

    def context_label(self) -> str:
        return self.taskspace_label()


def slugify(name: str) -> str:
    import re

    slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return slug or "task"


def task_from_row(row: dict[str, Any]) -> Task:
    import json

    workspace_count = row.get("workspace_count")
    if workspace_count is None:
        workspace_count = row.get("desktop_count")
    if workspace_count is None:
        # Migrate legacy rows that stored workspace_range + workspaces dict
        legacy_ws = json.loads(row.get("workspaces") or "{}")
        if isinstance(legacy_ws, dict):
            workspace_count = len(legacy_ws) or 3
        else:
            workspace_count = len(legacy_ws) if legacy_ws else 3

    return Task(
        id=row["id"],
        name=row["name"],
        status=TaskStatus(row["status"]),
        repo_url=row.get("repo_url"),
        repo_path=Path(row["repo_path"]),
        branch=row.get("branch"),
        container_name=row["container_name"],
        workspace_count=int(workspace_count),
        browser_profile=row.get("browser_profile"),
        created_at=datetime.fromisoformat(row["created_at"]),
        last_active_at=datetime.fromisoformat(row["last_active_at"]),
        agent_notes_path=Path(row["agent_notes_path"]) if row.get("agent_notes_path") else None,
        ports=json.loads(row.get("ports") or "[]"),
    )

"""Task and context orchestration."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

from lae.util import subprocess as sp

from lae.core.config import LaeConfig, load_config
from lae.core.models import ContextMode, SessionState, Task, TaskStatus
from lae.core.registry import Registry
from lae.core.workspaces import default_taskspace_workspace_names
from lae.daemon import workspace_nav
from lae.daemon.waybar_export import write_waybar_file
from lae.integrations import distrobox, git, hyprland
from lae.util import xdg


class TaskService:
    def __init__(
        self,
        registry: Registry | None = None,
        config: LaeConfig | None = None,
    ):
        self.config = config or load_config()
        self.registry = registry or Registry(config=self.config)
        self._state: SessionState | None = None

    def get_state(self) -> SessionState:
        # Always reload — Rust CLI and Waybar CFFI write state.db directly.
        self._state = self.registry.load_state()
        return self._state

    def save_state(self, state: SessionState | None = None) -> None:
        state = state or self.get_state()
        self.registry.save_state(state)
        self._write_runtime_files(state)

    def _write_runtime_files(self, state: SessionState) -> None:
        self._write_context_file(state)
        write_waybar_file(state)

    def _write_context_file(self, state: SessionState) -> None:
        try:
            runtime = xdg.lae_runtime_dir()
            runtime.mkdir(parents=True, exist_ok=True)
            xdg.lae_context_file().write_text(state.taskspace_label())
        except RuntimeError:
            pass

    def initialize(self) -> None:
        state = self.get_state()
        state.default_workspace_count = self.config.default_workspace_count
        if hyprland.available() and self.config.hyprland_enabled:
            workspace_nav.setup_default_taskspace_workspaces(
                self.config.default_workspace_count
            )
            for task in state.tasks.values():
                if task.status != TaskStatus.archived:
                    workspace_nav.setup_task_workspaces(
                        task, slot_count=state.default_workspace_count
                    )
        self.save_state(state)

    def create_task(
        self,
        name: str,
        *,
        repo_url: str | None = None,
        branch: str | None = None,
        switch: bool = True,
    ) -> Task:
        state = self.get_state()
        task_id = self.registry.unique_task_id(state, name)

        task_home = self.config.tasks_base_dir / task_id
        repo_path = task_home / "repo"
        agent_dir = task_home / ".lae"
        agent_dir.mkdir(parents=True, exist_ok=True)
        notes_path = agent_dir / "agent-notes.md"
        if not notes_path.exists():
            notes_path.write_text(f"# {name}\n\nTask notes for agent and human.\n")

        if repo_url:
            git.clone_repo(repo_url, repo_path, branch)
        else:
            git.init_repo(repo_path)

        container_name = f"{self.config.container_prefix}-{task_id}"
        if distrobox.available():
            if not distrobox.container_exists(container_name):
                distrobox.create_container(
                    container_name,
                    task_home,
                    self.config.distrobox_image,
                )

        task = Task(
            id=task_id,
            name=name,
            status=TaskStatus.active,
            repo_url=repo_url,
            repo_path=repo_path,
            branch=branch,
            container_name=container_name,
            workspace_count=self.config.default_workspace_count,
            agent_notes_path=notes_path,
        )
        state.tasks[task_id] = task
        self.registry.touch_task(task)

        if hyprland.available() and self.config.hyprland_enabled:
            workspace_nav.setup_task_workspaces(
                task, slot_count=state.default_workspace_count
            )

        if switch:
            self.switch_task(task_id)
        else:
            self.save_state(state)
        return task

    def switch_task(self, task_id: str) -> Task:
        state = self.get_state()
        task = state.tasks.get(task_id)
        if task is None:
            raise ValueError(f"Unknown task: {task_id}")
        if task.status == TaskStatus.archived:
            raise ValueError(f"Task is archived: {task_id}")
        task.status = TaskStatus.active
        self.registry.touch_task(task)
        workspace_nav.set_taskspace(state, ContextMode.task, task_id)
        state.last_workspace[f"task:{task_id}"] = state.last_workspace.get(
            f"task:{task_id}", 1
        )
        if hyprland.available():
            workspace_nav.setup_task_workspaces(
                task, slot_count=state.default_workspace_count
            )
            hyprland.switch_workspace(task.main_workspace())
        self.save_state(state)
        return task

    def context_default(self) -> None:
        state = self.get_state()
        workspace_nav.set_taskspace(state, ContextMode.default)
        self.save_state(state)

    def archive_task(self, task_id: str) -> None:
        from lae.core import task_cleanup

        state = self.get_state()
        task = state.tasks.get(task_id)
        if task is None:
            raise ValueError(f"Unknown task: {task_id}")
        if task.status == TaskStatus.archived:
            raise ValueError(f"Task is already archived: {task_id}")

        if state.current_task_id == task_id:
            workspace_nav.set_taskspace(state, ContextMode.default)
            state.current_task_id = None

        task_cleanup.close_task_windows(task)
        task_cleanup.stop_task_container(task)
        task_cleanup.purge_task_windows(state, task_id)
        task.status = TaskStatus.archived
        self.save_state(state)

    def delete_task(self, task_id: str) -> None:
        from lae.core import task_cleanup

        state = self.get_state()
        task = state.tasks.get(task_id)
        if task is None:
            raise ValueError(f"Unknown task: {task_id}")

        if state.current_task_id == task_id:
            workspace_nav.set_taskspace(state, ContextMode.default)
            state.current_task_id = None

        task_cleanup.close_task_windows(task)
        task_cleanup.remove_task_container(task)
        task_cleanup.remove_task_data_dir(self.config.tasks_base_dir, task)
        task_cleanup.purge_task_windows(state, task_id)
        task_cleanup.purge_task_session_keys(state, task_id)
        del state.tasks[task_id]
        self.save_state(state)

    def open_terminal(
        self,
        task_id: str | None = None,
        *,
        host: bool = False,
    ) -> None:
        state = self.get_state()
        if host:
            self._launch_host_terminal(None)
            return

        tid = task_id or state.current_task_id
        if not tid:
            raise RuntimeError(
                "No task context. Run `lae task switch <name>` or pass a task name."
            )
        task = state.tasks.get(tid)
        if task is None:
            raise ValueError(f"Unknown task: {tid}")

        inner = f"cd ~/repo && exec ${{'SHELL:-/bin/bash'}}"
        enter_argv = distrobox.enter_command(task.container_name, inner)
        title = f"[{task.id}] terminal"
        self._launch_terminal(task, enter_argv, title)

    def _launch_terminal(
        self, task: Task | None, command: list[str], title: str
    ) -> None:
        term = self.config.terminal_command
        if not sp.which(term):
            raise RuntimeError(f"Terminal emulator not found: {term}")

        argv = [term]
        if self.config.terminal_title_flag:
            argv.extend([self.config.terminal_title_flag, title])
        if task:
            argv.extend(["--working-directory", str(task.repo_path)])
        argv.extend(["--", *command])
        subprocess.Popen(argv, env=os.environ.copy())

    def _launch_host_terminal(self, cwd: Path | None) -> None:
        term = self.config.terminal_command
        if not sp.which(term):
            raise RuntimeError(f"Terminal emulator not found: {term}")
        argv = [term]
        if cwd:
            argv.extend(["--working-directory", str(cwd)])
        subprocess.Popen(argv, env=os.environ.copy())

    def resolve_task(self, name_or_id: str) -> Task:
        state = self.get_state()
        task = self.registry.get_task(state, name_or_id)
        if task is None:
            raise ValueError(f"Unknown task: {name_or_id}")
        return task

    def list_active_tasks(self) -> list[Task]:
        state = self.get_state()
        return [t for t in state.tasks.values() if t.status != TaskStatus.archived]

    def tasks_for_menu(self) -> list[dict]:
        state = self.get_state()
        items: list[dict] = []

        in_default = state.context_mode == ContextMode.default
        items.append(
            {
                "id": "default",
                "name": "default",
                "kind": "default",
                "workspaces": default_taskspace_workspace_names(
                    state.default_workspace_count
                ),
                "current": in_default,
                "status": "system",
            }
        )

        for task in self.list_active_tasks():
            items.append(
                {
                    "id": task.id,
                    "name": task.name,
                    "kind": "task",
                    "workspaces": task.workspace_names(),
                    "current": (
                        state.context_mode == ContextMode.task
                        and state.current_task_id == task.id
                    ),
                    "status": task.status.value,
                }
            )
        return items

    def status_summary(self) -> dict:
        state = self.get_state()
        active_ws = hyprland.get_active_workspace() if hyprland.available() else None
        current_task = (
            state.tasks.get(state.current_task_id) if state.current_task_id else None
        )
        allowed = workspace_nav.allowed_workspaces(state)
        branch = git.current_branch(current_task.repo_path) if current_task else None
        container_running = (
            distrobox.is_running(current_task.container_name) if current_task else False
        )
        return {
            "taskspace": state.taskspace_label(),
            "context": state.taskspace_label(),
            "allowed_workspaces": allowed,
            "active_workspace": active_ws.name if active_ws else None,
            "active_workspace_id": active_ws.id if active_ws else None,
            "current_task": current_task.model_dump(mode="json") if current_task else None,
            "branch": branch,
            "container_running": container_running,
            "tasks": [t.model_dump(mode="json") for t in self.list_active_tasks()],
            "windows": [
                w.model_dump(mode="json", by_alias=True) for w in state.windows.values()
            ],
        }

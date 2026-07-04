"""Task tear-down helpers for archive and delete."""

from __future__ import annotations

import shutil
from pathlib import Path

from tsk.core.models import SessionState, Task
from tsk.integrations import distrobox, hyprland


def task_data_dir(base_dir: Path, task_id: str) -> Path:
    return base_dir / task_id


def is_active_task_context(state: SessionState, task: Task) -> bool:
    if state.current_task_id == task.id:
        return True
    if not hyprland.available():
        return False
    active = hyprland.get_active_workspace()
    if active is None:
        return False
    return active.name in set(task.workspace_names())


def clients_for_task(task: Task) -> list:
    if not hyprland.available():
        return []
    workspace_names = set(task.workspace_names())
    title_prefix = f"[{task.id}]"
    return [
        client
        for client in hyprland.get_clients()
        if client.workspace_name in workspace_names
        or client.title.startswith(title_prefix)
    ]


def close_task_windows(task: Task) -> int:
    clients = clients_for_task(task)
    for client in clients:
        hyprland.close_window(client.address)
    return len(clients)


def stop_task_container(task: Task) -> None:
    distrobox.stop_container(task.container_name)


def remove_task_container(task: Task) -> None:
    distrobox.stop_container(task.container_name)
    distrobox.remove_container(task.container_name)


def purge_task_windows(state: SessionState, task_id: str) -> None:
    stale = [addr for addr, record in state.windows.items() if record.task_id == task_id]
    for addr in stale:
        del state.windows[addr]


def purge_task_session_keys(state: SessionState, task_id: str) -> None:
    state.last_workspace.pop(f"task:{task_id}", None)
    if state.current_task_id == task_id:
        state.current_task_id = None
        from tsk.core.models import ContextMode

        state.context_mode = ContextMode.default


def remove_task_data_dir(base_dir: Path, task: Task) -> None:
    task_home = task_data_dir(base_dir, task.id)
    if task_home.is_dir():
        try:
            shutil.rmtree(task_home)
        except OSError as err:
            import sys

            print(
                f"tsk: delete task {task.id}: remove data dir: {err}",
                file=sys.stderr,
            )

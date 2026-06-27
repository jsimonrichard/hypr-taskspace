"""Named Hyprland workspace helpers within a taskspace."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from lae.core.models import SessionState, Task


def default_taskspace_workspace_name(relative: int) -> str:
    """Default taskspace uses plain digits (1..10) — same as Omarchy Hyprland ids."""
    return str(relative)


def default_taskspace_workspace_names(count: int) -> list[str]:
    return [default_taskspace_workspace_name(n) for n in range(1, count + 1)]


def is_default_taskspace_workspace_name(name: str, workspace_count: int = 3) -> bool:
    if not name.isdigit():
        return False
    n = int(name)
    return 1 <= n <= workspace_count


def task_workspace_name(task_id: str, relative: int) -> str:
    return f"{task_id}-{relative}"


def task_workspace_names(task_id: str, count: int) -> list[str]:
    return [task_workspace_name(task_id, n) for n in range(1, count + 1)]


def relative_from_name(name: str, task_id: str | None = None) -> int | None:
    if is_default_taskspace_workspace_name(name, workspace_count=99):
        return int(name)
    if task_id and name.startswith(f"{task_id}-"):
        suffix = name[len(task_id) + 1 :]
        return int(suffix) if suffix.isdigit() else None
    parts = name.rsplit("-", 1)
    if len(parts) == 2 and parts[1].isdigit():
        return int(parts[1])
    return None


def task_for_workspace_name(state: SessionState, name: str) -> Task | None:
    if is_default_taskspace_workspace_name(name, state.default_workspace_count):
        return None
    for task in state.tasks.values():
        if task.status.value == "archived":
            continue
        if name.startswith(f"{task.id}-"):
            return task
    return None


def allowed_workspace_names(
    state: SessionState, *, workspace_count: int = 3
) -> list[str]:
    from lae.core.models import ContextMode

    if state.context_mode == ContextMode.global_:
        names = default_taskspace_workspace_names(workspace_count)
        for task in state.tasks.values():
            if task.status.value != "archived":
                names.extend(task.workspace_names())
        return names

    if state.context_mode == ContextMode.task and state.current_task_id:
        task = state.tasks.get(state.current_task_id)
        if task:
            return task.workspace_names()

    return default_taskspace_workspace_names(workspace_count)


# Deprecated aliases
default_desktop_name = default_taskspace_workspace_name
default_desktop_names = default_taskspace_workspace_names
is_default_workspace_name = is_default_taskspace_workspace_name
task_desktop_name = task_workspace_name
task_desktop_names = task_workspace_names

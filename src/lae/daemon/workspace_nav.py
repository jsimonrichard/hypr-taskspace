"""Taskspace-scoped Hyprland workspace navigation."""

from __future__ import annotations

from lae.core.models import ContextMode, SessionState, Task
from lae.core.workspaces import (
    allowed_workspace_names,
    default_taskspace_workspace_names,
    task_workspace_names,
)
from lae.integrations import hyprland


def allowed_workspaces(state: SessionState) -> list[str]:
    return allowed_workspace_names(
        state, workspace_count=state.default_workspace_count
    )


def _relative_to_name(state: SessionState, relative: int) -> str | None:
    names = allowed_workspaces(state)
    if not names:
        return None
    idx = relative - 1
    if idx < 0 or idx >= len(names):
        return None
    return names[idx]


def _remember_workspace(state: SessionState, relative: int) -> None:
    state.last_workspace[state.taskspace_key()] = relative


def _active_relative(state: SessionState) -> int | None:
    active = hyprland.get_active_workspace()
    if not active or not active.name:
        return None
    names = allowed_workspaces(state)
    if active.name in names:
        return names.index(active.name) + 1
    return None


def workspace_go(state: SessionState, relative: int) -> str | None:
    name = _relative_to_name(state, relative)
    if name is None:
        return None
    hyprland.switch_workspace(name)
    _remember_workspace(state, relative)
    return name


def workspace_next(state: SessionState) -> str | None:
    names = allowed_workspaces(state)
    if not names:
        return None
    current_rel = _active_relative(state)
    if current_rel is not None:
        next_rel = (current_rel % len(names)) + 1
    else:
        next_rel = 1
    return workspace_go(state, next_rel)


def workspace_prev(state: SessionState) -> str | None:
    names = allowed_workspaces(state)
    if not names:
        return None
    current_rel = _active_relative(state)
    if current_rel is not None:
        prev_rel = current_rel - 1 if current_rel > 1 else len(names)
    else:
        prev_rel = len(names)
    return workspace_go(state, prev_rel)


def workspace_goto_name(state: SessionState, name: str) -> str | None:
    allowed = allowed_workspaces(state)
    if name not in allowed:
        return None
    hyprland.switch_workspace(name)
    if name in allowed:
        _remember_workspace(state, allowed.index(name) + 1)
    return name


def focus_last_workspace(state: SessionState) -> str | None:
    key = state.taskspace_key()
    relative = state.last_workspace.get(key, 1)
    name = _relative_to_name(state, relative)
    if name is None:
        names = allowed_workspaces(state)
        if not names:
            return None
        name = names[0]
        relative = 1
    hyprland.switch_workspace(name)
    _remember_workspace(state, relative)
    return name


def set_taskspace(
    state: SessionState,
    mode: ContextMode,
    task_id: str | None = None,
) -> None:
    if mode == ContextMode.task:
        if not task_id:
            raise ValueError("task_id required for task taskspace")
        if task_id not in state.tasks:
            raise ValueError(f"Unknown task: {task_id}")
        state.context_mode = ContextMode.task
        state.current_task_id = task_id
    elif mode == ContextMode.default:
        state.context_mode = ContextMode.default
        state.current_task_id = None
    focus_last_workspace(state)


def setup_task_workspaces(task: Task, *, slot_count: int | None = None) -> None:
    count = slot_count if slot_count is not None else task.workspace_count
    hyprland.ensure_workspaces(task_workspace_names(task.id, count))


def setup_default_taskspace_workspaces(count: int) -> None:
    hyprland.ensure_workspaces(default_taskspace_workspace_names(count))


# Deprecated aliases
set_context = set_taskspace
focus_last_desktop = focus_last_workspace
desktop_go = workspace_go
desktop_next = workspace_next
desktop_prev = workspace_prev
desktop_goto_name = workspace_goto_name
setup_default_workspaces = setup_default_taskspace_workspaces

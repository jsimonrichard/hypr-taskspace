"""Sync taskspace from the active Hyprland workspace name."""

from __future__ import annotations

from tsk.core.models import ContextMode, SessionState
from tsk.core.workspaces import (
    allowed_workspace_names,
    is_default_taskspace_workspace_name,
    resolve_bar_workspace_name,
    task_for_workspace_name,
)
from tsk.integrations import hyprland


def sync_from_workspace_name(state: SessionState, name: str) -> bool:
    """Align tsk taskspace + active workspace index with a Hyprland workspace name."""
    if not name:
        return False

    allowed = allowed_workspace_names(
        state, workspace_count=state.default_workspace_count
    )
    resolved = resolve_bar_workspace_name(name, allowed)
    if resolved is None:
        return False

    changed = False

    if is_default_taskspace_workspace_name(resolved, state.default_workspace_count):
        if state.context_mode != ContextMode.default or state.current_task_id is not None:
            state.context_mode = ContextMode.default
            state.current_task_id = None
            changed = True
    else:
        task = task_for_workspace_name(state, resolved)
        if task:
            if (
                state.context_mode != ContextMode.task
                or state.current_task_id != task.id
            ):
                state.context_mode = ContextMode.task
                state.current_task_id = task.id
                changed = True

    if resolved in allowed:
        rel = allowed.index(resolved) + 1
        key = state.taskspace_key()
        if state.last_workspace.get(key) != rel:
            state.last_workspace[key] = rel
            changed = True

    return changed


def sync_from_active_workspace(state: SessionState) -> bool:
    """Align tsk taskspace + active workspace index with Hyprland focus."""
    if not hyprland.available():
        return False

    active = hyprland.get_active_workspace()
    if not active or not active.name:
        return False

    return sync_from_workspace_name(state, active.name)

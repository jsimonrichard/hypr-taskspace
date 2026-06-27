"""Sync taskspace from the active Hyprland workspace name."""

from __future__ import annotations

from lae.core.models import ContextMode, SessionState
from lae.core.workspaces import (
    allowed_workspace_names,
    is_default_taskspace_workspace_name,
    task_for_workspace_name,
)
from lae.integrations import hyprland


def sync_from_active_workspace(state: SessionState) -> bool:
    """Align lae taskspace + active workspace index with Hyprland focus."""
    if not hyprland.available():
        return False

    active = hyprland.get_active_workspace()
    if not active or not active.name:
        return False

    changed = False
    name = active.name

    if state.context_mode != ContextMode.global_:
        if is_default_taskspace_workspace_name(name, state.default_workspace_count):
            if state.context_mode != ContextMode.default or state.current_task_id is not None:
                state.context_mode = ContextMode.default
                state.current_task_id = None
                changed = True
        else:
            task = task_for_workspace_name(state, name)
            if task:
                if (
                    state.context_mode != ContextMode.task
                    or state.current_task_id != task.id
                ):
                    state.context_mode = ContextMode.task
                    state.current_task_id = task.id
                    changed = True

    names = allowed_workspace_names(
        state, workspace_count=state.default_workspace_count
    )
    if name in names:
        rel = names.index(name) + 1
        key = state.taskspace_key()
        if state.last_workspace.get(key) != rel:
            state.last_workspace[key] = rel
            changed = True

    return changed

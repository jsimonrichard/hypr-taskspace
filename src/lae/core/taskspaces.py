"""Taskspace visibility helpers for Waybar and navigation."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from lae.core.models import SessionState

DEFAULT_MIN_VISIBLE_WORKSPACES = 5


def visible_default_workspace_count(
    state: SessionState,
    allowed: list[str],
    active_rel: int,
    occupied: set[int],
) -> int:
    """How many workspace slots to show in Waybar for the current taskspace."""
    from lae.core.models import ContextMode

    total = len(allowed)
    if state.context_mode == ContextMode.task:
        return total

    highest_occupied = max(occupied) if occupied else 0
    visible = max(active_rel, highest_occupied, DEFAULT_MIN_VISIBLE_WORKSPACES)
    return min(visible, total, 10)

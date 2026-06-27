"""Waybar status export for taskspace-scoped workspace indicators."""

from __future__ import annotations

import json
from typing import Any

from lae.core.models import ContextMode, SessionState
from lae.core.taskspaces import visible_default_workspace_count
from lae.daemon import context_sync, workspace_nav
from lae.integrations import hyprland
from lae.util import xdg

# Same glyph as Omarchy hyprland/workspaces format-icons.active
ACTIVE_WORKSPACE_ICON = "󱓻"
WAYBAR_MODULE_COUNT = 10


def build_waybar_data(state: SessionState, *, sync: bool = True) -> dict[str, Any]:
    if sync:
        context_sync.sync_from_active_workspace(state)
    allowed = workspace_nav.allowed_workspaces(state)
    active = hyprland.get_active_workspace() if hyprland.available() else None
    active_name = active.name if active else None

    active_rel = state.last_workspace.get(state.taskspace_key(), 1)
    if active_name and active_name in allowed:
        active_rel = allowed.index(active_name) + 1

    task = (
        state.tasks.get(state.current_task_id)
        if state.current_task_id
        else None
    )

    occupied = occupied_relative_indices(allowed)
    visible = visible_default_workspace_count(
        state, allowed, active_rel, occupied
    )

    return {
        "taskspace": state.taskspace_label(),
        "context_mode": state.context_mode.value,
        "task_id": state.current_task_id,
        "task_name": task.name if task else None,
        "workspaces": allowed,
        "workspace_count": len(allowed),
        "visible_workspace_count": visible,
        "occupied_workspace_indices": sorted(occupied),
        "active_workspace": active_rel,
        "active_workspace_name": active_name,
        "global_mode": state.context_mode == ContextMode.global_,
    }


def occupied_relative_indices(allowed: list[str]) -> set[int]:
    if not hyprland.available():
        return set()
    occupied: set[int] = set()
    for client in hyprland.get_clients():
        name = client.workspace_name
        if name in allowed:
            occupied.add(allowed.index(name) + 1)
    return occupied


def build_all_modules(state: SessionState, *, sync: bool = True) -> dict[str, dict[str, Any]]:
    data = build_waybar_data(state, sync=sync)
    modules: dict[str, dict[str, Any]] = {"task": _task_module(data)}
    for index in range(1, WAYBAR_MODULE_COUNT + 1):
        modules[f"workspace-{index}"] = _workspace_module(data, index)
    return modules


def write_waybar_file(state: SessionState) -> None:
    try:
        runtime = xdg.lae_runtime_dir()
        runtime.mkdir(parents=True, exist_ok=True)
        data = build_waybar_data(state, sync=True)
        xdg.lae_waybar_file().write_text(json.dumps(data) + "\n")
        from lae.waybar_cache import write_modules_cache

        write_modules_cache(build_all_modules(state, sync=False), notify=True)
    except RuntimeError:
        pass


def module_json(state: SessionState, module: str) -> dict[str, Any]:
    data = build_waybar_data(state, sync=True)

    if module == "task":
        return _task_module(data)
    if module.startswith("workspace-"):
        index = int(module.split("-", 1)[1])
        return _workspace_module(data, index)
    if module.startswith("desktop-"):
        index = int(module.split("-", 1)[1])
        return _workspace_module(data, index)
    raise ValueError(f"Unknown waybar module: {module}")


# Deprecated alias
build_waybar_state = build_waybar_data


def _task_module(data: dict[str, Any]) -> dict[str, Any]:
    if data["global_mode"]:
        return {
            "text": "󰌾 all",
            "tooltip": "Global taskspace — all Hyprland workspaces reachable",
            "class": "global",
        }
    if data["task_id"]:
        name = data["task_name"] or data["task_id"]
        tip = f"Task: {name}\nWorkspaces: {', '.join(data['workspaces'])}"
        return {"text": f"󱓝 {name}", "tooltip": tip, "class": "task"}
    return {
        "text": "󰣇 default",
        "tooltip": f"Default taskspace workspaces: {', '.join(data['workspaces'])}",
        "class": "default",
    }


def _workspace_label(index: int) -> str:
    return "0" if index == 10 else str(index)


def _workspace_module(data: dict[str, Any], index: int) -> dict[str, Any]:
    if index < 1 or index > WAYBAR_MODULE_COUNT:
        return {"text": "", "class": "hidden"}
    if index > data["workspace_count"]:
        return {"text": "", "class": "hidden"}

    is_active = data["active_workspace"] == index
    if index > data["visible_workspace_count"] and not is_active:
        return {"text": "", "class": "hidden"}

    workspace_name = (
        data["workspaces"][index - 1]
        if index <= len(data["workspaces"])
        else str(index)
    )
    occupied = set(data.get("occupied_workspace_indices", []))
    classes: list[str] = []
    if is_active:
        classes.append("active")
    elif index not in occupied:
        classes.append("empty")
    if data["global_mode"]:
        classes.append("global")

    return {
        "text": ACTIVE_WORKSPACE_ICON if is_active else _workspace_label(index),
        "tooltip": workspace_name,
        "class": " ".join(classes) if classes else "idle",
    }

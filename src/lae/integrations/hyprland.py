"""Hyprland hyprctl wrapper."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from lae.util import subprocess as sp


class HyprlandError(RuntimeError):
    pass


def available() -> bool:
    return sp.which("hyprctl") is not None and _has_instance()


def _has_instance() -> bool:
    import os

    sig = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    return bool(sig and runtime)


def hyprctl_json(*args: str) -> Any:
    if not available():
        raise HyprlandError("Hyprland is not available (hyprctl or instance signature missing)")
    try:
        return sp.run_json(["hyprctl", "-j", *args])
    except sp.CommandError as exc:
        raise HyprlandError(str(exc)) from exc


def dispatch(*cmd: str) -> None:
    if not available():
        return
    sp.run(["hyprctl", "dispatch", *cmd], check=False)


def keyword(*args: str) -> None:
    if not available():
        return
    sp.run(["hyprctl", "keyword", *args], check=False)


@dataclass
class HyprWindow:
    address: str
    title: str
    class_: str
    workspace: int
    workspace_name: str
    pid: int | None


@dataclass
class Workspace:
    id: int
    name: str


def _workspace_fields(raw: Any) -> tuple[int, str]:
    if isinstance(raw, dict):
        return int(raw.get("id") or 0), str(raw.get("name") or "")
    return int(raw or 0), ""


def get_workspaces() -> list[Workspace]:
    data = hyprctl_json("workspaces") or []
    return [
        Workspace(id=int(item.get("id", 0)), name=str(item.get("name") or ""))
        for item in data
    ]


def get_clients() -> list[HyprWindow]:
    data = hyprctl_json("clients") or []
    windows: list[HyprWindow] = []
    for item in data:
        ws_id, ws_name = _workspace_fields(item.get("workspace", {}))
        windows.append(
            HyprWindow(
                address=item.get("address", ""),
                title=item.get("title", ""),
                class_=item.get("class", ""),
                workspace=ws_id,
                workspace_name=ws_name,
                pid=item.get("pid"),
            )
        )
    return windows


def get_active_workspace() -> Workspace | None:
    if not available():
        return None
    data = hyprctl_json("activeworkspace")
    if not data:
        return None
    return Workspace(id=int(data.get("id", 0)), name=str(data.get("name") or ""))


def switch_workspace(name: str) -> None:
    """Focus workspace. Plain-digit names use Hyprland id (Omarchy-compatible)."""
    if name.isdigit():
        dispatch("workspace", name)
    else:
        dispatch("workspace", f"name:{name}")


def switch_workspace_id(ws_id: int) -> None:
    dispatch("workspace", str(ws_id))


def move_window_to_workspace(address: str, workspace_name: str) -> None:
    if workspace_name.isdigit():
        dispatch("movetoworkspace", f"{workspace_name},address:{address}")
    else:
        dispatch("movetoworkspace", f"name:{workspace_name},address:{address}")


def rename_workspace(ws_id: int, name: str) -> None:
    keyword("workspace", f"{ws_id},name:{name}")


def ensure_workspaces(names: list[str]) -> None:
    """Ensure workspaces exist and default ids 1..N carry matching names."""
    active = get_active_workspace()
    previous = active.name if active and active.name else None
    for name in names:
        if name.isdigit():
            ws_id = int(name)
            rename_workspace(ws_id, name)
        switch_workspace(name)
    if previous:
        switch_workspace(previous)


def reload_config() -> None:
    dispatch("reload")

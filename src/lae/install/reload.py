"""Reload desktop components after install / uninstall."""

from __future__ import annotations

from lae.integrations import hyprland
from lae.util import subprocess as sp


def reload_hypr() -> bool:
    if not hyprland.available():
        return False
    hyprland.reload_config()
    return True


def restart_walker() -> bool:
    if not sp.which("omarchy-restart-walker"):
        return False
    sp.run(["omarchy-restart-walker"], check=False)
    return True


def restart_waybar() -> bool:
    if not sp.which("omarchy-restart-waybar"):
        return False
    sp.run(["omarchy-restart-waybar"], check=False)
    return True


def apply_after_hypr() -> list[str]:
    actions: list[str] = []
    if reload_hypr():
        actions.append("reloaded Hyprland config")
    elif sp.which("hyprctl"):
        actions.append("Hyprland not active — run `hyprctl reload` after login")
    if restart_walker():
        actions.append("restarted Walker")
    elif sp.which("elephant"):
        actions.append("run `omarchy-restart-walker` to refresh the task menu")
    return actions


def apply_after_waybar() -> list[str]:
    actions: list[str] = []
    if restart_waybar():
        actions.append("restarted Waybar")
    elif sp.which("waybar"):
        actions.append("run `omarchy-restart-waybar` to apply Waybar changes")
    return actions

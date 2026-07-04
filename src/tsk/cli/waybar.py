"""Waybar module output (called by Waybar exec)."""

from __future__ import annotations

import json
import sys

import typer

app = typer.Typer(help="Waybar module helpers.", hidden=True)


def _module_key(name: str, index: int | None = None) -> str:
    if name in ("task",):
        return "task"
    if name in ("workspace", "desktop") and index is not None:
        return f"workspace-{index}"
    raise typer.BadParameter(f"Unknown waybar target: {name}")


def _emit(module: str) -> None:
    from tsk.waybar_cache import emit_module

    sys.stdout.write(json.dumps(emit_module(module), separators=(",", ":")))


@app.command("refresh-cache")
def refresh_cache() -> None:
    """Rebuild Waybar module cache (one heavy pass for all modules)."""
    from tsk.waybar_cache import refresh_modules_cache

    refresh_modules_cache(notify=True)


@app.command("task")
def waybar_task() -> None:
    """JSON for custom/tsk-task module."""
    _emit("task")


@app.command("workspace")
def waybar_workspace(
    index: int = typer.Argument(..., min=1, max=10),
) -> None:
    """JSON for custom/tsk-workspace-N module."""
    _emit(f"workspace-{index}")


@app.command("desktop")
def waybar_desktop(
    index: int = typer.Argument(..., min=1, max=10),
) -> None:
    """Deprecated alias for waybar workspace."""
    _emit(f"workspace-{index}")


def main() -> None:
    app()


if __name__ == "__main__":
    main()

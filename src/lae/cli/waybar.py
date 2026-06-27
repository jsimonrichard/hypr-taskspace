"""Waybar module output (called by Waybar exec)."""

from __future__ import annotations

import json
import sys

import typer

app = typer.Typer(help="Waybar module helpers.", hidden=True)


@app.command("task")
def waybar_task() -> None:
    """JSON for custom/lae-task module."""
    _emit("task")


@app.command("workspace")
def waybar_workspace(
    index: int = typer.Argument(..., min=1, max=10),
) -> None:
    """JSON for custom/lae-workspace-N module."""
    _emit(f"workspace-{index}")


@app.command("desktop")
def waybar_desktop(
    index: int = typer.Argument(..., min=1, max=10),
) -> None:
    """Deprecated alias for waybar workspace."""
    _emit(f"workspace-{index}")


def _emit(module: str) -> None:
    from lae.daemon.service import TaskService
    from lae.daemon.waybar_export import module_json

    service = TaskService()
    state = service.get_state()
    payload = module_json(state, module)
    service.save_state(state)
    sys.stdout.write(json.dumps(payload))


def main() -> None:
    app()


if __name__ == "__main__":
    main()

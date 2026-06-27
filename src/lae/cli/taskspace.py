"""Taskspace switching — default, task-scoped, or global."""

from __future__ import annotations

import typer

app = typer.Typer(help="Taskspace: default host, task-scoped, or global escape hatch.")


def _echo_taskspace() -> None:
    from lae.cli.client import call

    summary = call("status")
    label = summary.get("taskspace") or summary.get("context")
    typer.echo(f"Taskspace: {label}")


@app.command("default")
def taskspace_default() -> None:
    """Switch to the default host taskspace."""
    from lae.cli.client import call

    call("set_context", {"mode": "default"})
    _echo_taskspace()


@app.command("global")
def taskspace_global() -> None:
    """Enter global taskspace (all Hyprland workspaces reachable)."""
    from lae.cli.client import call

    call("set_context", {"mode": "global"})
    _echo_taskspace()


@app.command("restore")
def taskspace_restore() -> None:
    """Exit global taskspace and restore the previous scoped taskspace."""
    from lae.cli.client import call

    call("restore_context")
    _echo_taskspace()


@app.command("escape")
def taskspace_escape() -> None:
    """Alias for global escape hatch."""
    taskspace_global()


@app.command("toggle-global")
def taskspace_toggle_global() -> None:
    """Toggle global taskspace (bound to SUPER+ESCAPE)."""
    from lae.cli.client import call

    call("toggle_global")
    _echo_taskspace()


@app.command("current")
def taskspace_current() -> None:
    """Print current taskspace label."""
    from lae.cli.client import call

    summary = call("status")
    typer.echo(summary.get("taskspace") or summary.get("context"))

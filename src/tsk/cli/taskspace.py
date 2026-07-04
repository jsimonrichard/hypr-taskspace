"""Taskspace switching — default or task-scoped."""

from __future__ import annotations

import typer

app = typer.Typer(help="Taskspace: default host or task-scoped.")


def _echo_taskspace() -> None:
    from tsk.cli.client import call

    summary = call("status")
    label = summary.get("taskspace") or summary.get("context")
    typer.echo(f"Taskspace: {label}")


@app.command("default")
def taskspace_default() -> None:
    """Switch to the default host taskspace."""
    from tsk.cli.client import call

    call("set_context", {"mode": "default"})
    _echo_taskspace()


@app.command("current")
def taskspace_current() -> None:
    """Print current taskspace label."""
    from tsk.cli.client import call

    summary = call("status")
    typer.echo(summary.get("taskspace") or summary.get("context"))

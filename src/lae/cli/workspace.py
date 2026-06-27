"""Workspace navigation within the current taskspace."""

from __future__ import annotations

import typer

app = typer.Typer(
    help="Navigate Hyprland workspaces within the active taskspace (keybind target).",
)


@app.command("go")
def workspace_go(
    index: int = typer.Argument(
        ..., min=1, max=10, help="Workspace index within current taskspace"
    ),
) -> None:
    """Go to workspace N (e.g. auth-fix-2 when in the auth-fix taskspace)."""
    from lae.cli.client import call

    result = call("workspace_go", {"relative": index})
    if result.get("workspace") is None:
        typer.echo("Workspace not available in current taskspace.", err=True)
        raise typer.Exit(1)
    typer.echo(result["workspace"])


@app.command("next")
def workspace_next() -> None:
    """Next workspace within current taskspace."""
    from lae.cli.client import call

    call("workspace_next")


@app.command("prev")
def workspace_prev() -> None:
    """Previous workspace within current taskspace."""
    from lae.cli.client import call

    call("workspace_prev")


@app.command("goto")
def workspace_goto(
    name: str = typer.Argument(..., help="Named Hyprland workspace (e.g. auth-fix-2 or 4)"),
) -> None:
    """Jump to a named Hyprland workspace."""
    from lae.cli.client import call

    result = call("workspace_goto", {"name": name})
    if result.get("workspace") is None:
        typer.echo("Workspace not reachable.", err=True)
        raise typer.Exit(1)

"""Daemon control subcommands."""

from __future__ import annotations

import os
import subprocess
import sys

import typer

app = typer.Typer(help="Background daemon control plane.")


@app.command("start")
def daemon_start(
    foreground: bool = typer.Option(False, "--foreground", "-f", help="Run in foreground"),
) -> None:
    """Start the tsk daemon."""
    if foreground:
        from tsk.daemon.server import run_daemon_foreground

        typer.echo("Starting tsk daemon (foreground)...")
        run_daemon_foreground()
        return

    from tsk.daemon.server import is_daemon_running

    if is_daemon_running():
        typer.echo("Daemon already running.")
        raise typer.Exit(0)

    cmd = [sys.executable, "-m", "tsk.cli.daemon", "start", "--foreground"]
    subprocess.Popen(
        cmd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        env=os.environ.copy(),
    )
    typer.echo("Daemon started.")


@app.command("stop")
def daemon_stop() -> None:
    """Stop the tsk daemon (removes socket; send SIGTERM to foreground process manually)."""
    from tsk.util import xdg

    sock = xdg.tsk_daemon_socket()
    if sock.exists():
        sock.unlink()
        typer.echo("Daemon socket removed.")
    else:
        typer.echo("Daemon is not running.")


@app.command("status")
def daemon_status() -> None:
    """Check if daemon is reachable."""
    from tsk.daemon.server import is_daemon_running
    from tsk.util import xdg

    if is_daemon_running():
        typer.echo(f"running ({xdg.tsk_daemon_socket()})")
    else:
        typer.echo("stopped (CLI will use direct mode)")


if __name__ == "__main__":
    app()

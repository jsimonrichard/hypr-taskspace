"""Typer CLI entry point."""

from __future__ import annotations

import typer

app = typer.Typer(
    name="lae",
    help="Local Agentic Environment — task-centric Hyprland + Distrobox control plane.",
    no_args_is_help=True,
)

uninstall_app = typer.Typer(help="Remove lae integrations.")


def _register_commands() -> None:
    """Import subcommand modules only when the CLI starts."""
    from lae.cli import context, daemon, desktop, install, task, taskspace, waybar, workspace

    app.add_typer(taskspace.app, name="taskspace")
    app.add_typer(context.app, name="context", hidden=True)
    app.add_typer(daemon.app, name="daemon")
    app.add_typer(workspace.app, name="workspace")
    app.add_typer(desktop.app, name="desktop", hidden=True)
    app.add_typer(task.app, name="task")
    app.add_typer(install.app, name="install")
    app.add_typer(waybar.app, name="waybar")
    app.add_typer(uninstall_app, name="uninstall")


@app.callback()
def _root() -> None:
    """Local Agentic Environment control plane."""


@uninstall_app.command("hypr")
def uninstall_hypr_root(
    keep_files: bool = typer.Option(False, "--keep-files"),
) -> None:
    """Restore Hyprland config from backup."""
    from lae.install.hypr import uninstall_hypr

    actions = uninstall_hypr(keep_files=keep_files)
    typer.echo("Uninstalled Hyprland integration.")
    if actions:
        typer.echo(f"Applied: {', '.join(actions)}.")


@uninstall_app.command("waybar")
def uninstall_waybar_root() -> None:
    """Restore Waybar config from backup."""
    from lae.install.waybar import uninstall_waybar

    actions = uninstall_waybar()
    typer.echo("Uninstalled Waybar integration.")
    if actions:
        typer.echo(f"Applied: {', '.join(actions)}.")


@app.command()
def status() -> None:
    """Show current taskspace, task, container, and windows."""
    from lae.cli.client import call

    summary = call("status")
    taskspace_label = summary.get("taskspace") or summary["context"]
    allowed = summary["allowed_workspaces"]
    if allowed:
        typer.echo(f"Taskspace: {taskspace_label}")
        typer.echo(f"Workspaces: {', '.join(allowed)}")
        typer.echo("Escape: SUPER+ESCAPE for global taskspace")
    else:
        typer.echo(f"Taskspace: {taskspace_label}")
    current = summary.get("current_task")
    if current:
        typer.echo(f"Task: {current['name']} ({current['id']})")
        typer.echo(f"Container: {current['container_name']} ({'running' if summary['container_running'] else 'stopped'})")
        typer.echo(f"Repo: {current['repo_path']}" + (f" @ {summary['branch']}" if summary.get("branch") else ""))
    else:
        typer.echo("Task: (none — default taskspace)")
    if summary.get("active_workspace"):
        typer.echo(f"Active workspace: {summary['active_workspace']}")
    windows = summary.get("windows") or []
    if windows:
        typer.echo("Windows:")
        for w in windows:
            tid = w.get("task_id") or "default"
            ws = w.get("workspace_name") or w.get("workspace")
            typer.echo(f"  [{tid}] {w.get('title', '')} → {ws}")
    others = [t for t in summary.get("tasks", []) if not current or t["id"] != current["id"]]
    if others:
        typer.echo("Other tasks:")
        for t in others:
            count = t.get("workspace_count", t.get("desktop_count", 3))
            typer.echo(f"  {t['id']} ({t['status']}, {t['id']}-1..{t['id']}-{count})")


@app.command()
def windows(
    task_name: str | None = typer.Option(None, "--task", help="Filter by task"),
) -> None:
    """List windows correlated with tasks."""
    from lae.cli.client import call

    summary = call("status")
    for w in summary.get("windows") or []:
        if task_name and w.get("task_id") != task_name:
            continue
        tid = w.get("task_id") or "default"
        typer.echo(f"{w.get('hypr_address')} [{tid}] {w.get('title')} ws={w.get('workspace')}")


@app.command()
def doctor() -> None:
    """Verify lae installation and daemon health."""
    from lae.daemon.server import is_daemon_running
    from lae.install.hypr import doctor_checks as hypr_doctor
    from lae.install.waybar import install_status as waybar_status

    ok = True
    for label, passed, detail in hypr_doctor():
        mark = "ok" if passed else "FAIL"
        typer.echo(f"[{mark}] {label}: {detail}")
        if not passed:
            ok = False

    if not is_daemon_running():
        typer.echo(
            "[WARN] Daemon not running — Waybar will refresh a shared cache "
            "instead of per-poll DB writes; run `lae daemon start` for best performance"
        )

    w = waybar_status()
    wb_ok = w["lae_modules_present"]
    typer.echo(
        f"[{'ok' if wb_ok else 'FAIL'}] Waybar lae workspace modules: {w['config_path']}"
    )
    if not wb_ok:
        ok = False

    if not ok:
        raise typer.Exit(code=1)


def main() -> None:
    _register_commands()
    app()


if __name__ == "__main__":
    main()

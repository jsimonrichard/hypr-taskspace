"""Task subcommands."""

from __future__ import annotations

import json

import typer

app = typer.Typer(help="Task lifecycle: create, list, switch, archive.")


@app.command("new")
def task_new(
    name: str = typer.Argument(..., help="Task name (used for slug/id)"),
    repo: str | None = typer.Option(None, "--repo", help="Git repository URL"),
    branch: str | None = typer.Option(None, "--branch", help="Branch to clone"),
    no_switch: bool = typer.Option(False, "--no-switch", help="Do not switch after create"),
) -> None:
    """Create a new task with Distrobox container and repo clone."""
    from lae.cli.client import call

    task = call(
        "create_task",
        {
            "name": name,
            "repo_url": repo,
            "branch": branch,
            "switch": not no_switch,
        },
    )
    count = task.get("workspace_count", task.get("desktop_count", 3))
    typer.echo(f"Created task {task['id']} → workspaces {task['id']}-1..{task['id']}-{count}")
    typer.echo(f"Repo: {task['repo_path']}")
    typer.echo(f"Container: {task['container_name']}")


@app.command("list")
def task_list(
    as_json: bool = typer.Option(False, "--json", help="JSON output"),
) -> None:
    """List all tasks."""
    from lae.cli.client import call

    if as_json:
        typer.echo(json.dumps(call("tasks_for_menu")))
        return

    summary = call("status")
    if not summary.get("tasks"):
        typer.echo("No tasks.")
        return
    for t in summary["tasks"]:
        count = t.get("workspace_count", t.get("desktop_count", 3))
        typer.echo(
            f"{t['id']:20} {t['status']:8}  {t['id']}-1..{t['id']}-{count}  {t['repo_path']}"
        )


@app.command("switch")
def task_switch(
    name_or_id: str = typer.Argument(..., help="Task name or id"),
) -> None:
    """Switch to a task (enters its taskspace and focuses the main workspace)."""
    from lae.cli.client import call
    from lae.daemon.service import TaskService

    service = TaskService()
    task = service.resolve_task(name_or_id)
    call("switch_task", {"task_id": task.id})
    typer.echo(f"Switched to task:{task.id} → {task.main_workspace()}")


@app.command("current")
def task_current() -> None:
    """Print the current task id, if any."""
    from lae.cli.client import call

    summary = call("status")
    current = summary.get("current_task")
    if current:
        typer.echo(current["id"])
    else:
        typer.echo("(none)")


@app.command("terminal")
def task_terminal(
    name_or_id: str | None = typer.Argument(None, help="Task (default: current)"),
    host: bool = typer.Option(False, "--host", help="Launch host shell instead of Distrobox"),
) -> None:
    """Open a terminal in the current task's Distrobox container."""
    from lae.cli.client import call
    from lae.daemon.service import TaskService

    task_id = None
    if name_or_id:
        service = TaskService()
        task_id = service.resolve_task(name_or_id).id
    call("open_terminal", {"task_id": task_id, "host": host})
    typer.echo("Terminal launched.")


@app.command("archive")
def task_archive(
    name_or_id: str = typer.Argument(..., help="Task to archive"),
) -> None:
    """Archive a task (closes windows, stops container; keeps files)."""
    from lae.cli.client import call
    from lae.daemon.service import TaskService

    task = TaskService().resolve_task(name_or_id)
    call("archive_task", {"task_id": task.id})
    typer.echo(f"Archived {task.id}")


@app.command("delete")
def task_delete(
    name_or_id: str = typer.Argument(..., help="Task to delete permanently"),
) -> None:
    """Delete a task and remove its data directory."""
    from lae.cli.client import call
    from lae.daemon.service import TaskService

    task = TaskService().resolve_task(name_or_id)
    call("delete_task", {"task_id": task.id})
    typer.echo(f"Deleted {task.id}")

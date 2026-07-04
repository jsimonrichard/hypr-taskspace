"""CLI ↔ daemon bridge with direct fallback."""

from __future__ import annotations

from typing import Any


def call(method: str, params: dict[str, Any] | None = None) -> Any:
    params = params or {}
    from tsk.daemon.server import daemon_request, is_daemon_running

    if is_daemon_running():
        response = daemon_request(method, params)
        if not response.get("ok"):
            raise RuntimeError(response.get("error", "daemon error"))
        return response.get("result")

    return _direct(method, params)


def _direct(method: str, params: dict[str, Any]) -> Any:
    from tsk.core.models import ContextMode
    from tsk.daemon import workspace_nav
    from tsk.daemon.service import TaskService
    service = TaskService()

    if method == "get_state":
        return service.get_state().model_dump(mode="json")

    if method == "create_task":
        task = service.create_task(
            params["name"],
            repo_url=params.get("repo_url"),
            branch=params.get("branch"),
            switch=params.get("switch", True),
        )
        return task.model_dump(mode="json")

    if method == "switch_task":
        return service.switch_task(params["task_id"]).model_dump(mode="json")

    if method == "archive_task":
        service.archive_task(params["task_id"])
        return {"archived": params["task_id"]}

    if method == "delete_task":
        service.delete_task(params["task_id"])
        return {"deleted": params["task_id"]}

    if method == "set_context":
        mode = params["mode"]
        if mode in ("default", "global"):
            service.context_default()
        elif mode == ContextMode.task.value:
            service.switch_task(params["task_id"])
        else:
            raise ValueError(f"Unknown context mode: {mode}")
        return {"context": service.get_state().context_label()}

    if method in ("workspace_go", "desktop_go"):
        state = service.get_state()
        ws = workspace_nav.workspace_go(state, int(params["relative"]))
        service.save_state(state)
        return {"workspace": ws}

    if method in ("workspace_next", "desktop_next"):
        state = service.get_state()
        ws = workspace_nav.workspace_next(state)
        service.save_state(state)
        return {"workspace": ws}

    if method in ("workspace_prev", "desktop_prev"):
        state = service.get_state()
        ws = workspace_nav.workspace_prev(state)
        service.save_state(state)
        return {"workspace": ws}

    if method in ("workspace_goto", "desktop_goto"):
        state = service.get_state()
        ws = workspace_nav.workspace_goto_name(state, str(params["name"]))
        service.save_state(state)
        return {"workspace": ws}

    if method == "open_terminal":
        service.open_terminal(params.get("task_id"), host=params.get("host", False))
        return {"launched": True}

    if method == "tasks_for_menu":
        return service.tasks_for_menu()

    if method == "status":
        return service.status_summary()

    raise ValueError(f"Unknown method: {method}")

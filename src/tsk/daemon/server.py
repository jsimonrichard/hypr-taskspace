"""UNIX socket IPC server for tsk daemon."""

from __future__ import annotations

import json
import os
import signal
import socket
import threading
from pathlib import Path
from typing import Any

from tsk.core.models import ContextMode
from tsk.daemon.service import TaskService
from tsk.daemon.window_router import WindowRouter
from tsk.integrations.hyprland_events import HyprlandEventListener, make_window_handler
from tsk.util import xdg


class DaemonServer:
    def __init__(self, service: TaskService | None = None):
        self.service = service or TaskService()
        self.socket_path = xdg.tsk_daemon_socket()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._listener: HyprlandEventListener | None = None
        self.router = WindowRouter(
            self.service.get_state,
            self.service.save_state,
            auto_move=self.service.config.auto_move_tagged_windows,
        )

    def start(self, *, foreground: bool = False) -> None:
        self.service.initialize()
        self.router.reconcile()
        handler = make_window_handler(self.router)
        self._listener = HyprlandEventListener(handler)
        self._listener.start()

        self._cache_thread = threading.Thread(
            target=self._waybar_cache_loop, name="tsk-waybar-cache", daemon=True
        )
        self._cache_thread.start()

        xdg.tsk_runtime_dir().mkdir(parents=True, exist_ok=True)
        if self.socket_path.exists():
            self.socket_path.unlink()

        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(str(self.socket_path))
        server.listen(5)
        os.chmod(self.socket_path, 0o600)

        if foreground:
            self._serve(server)
        else:
            self._thread = threading.Thread(target=self._serve, args=(server,), daemon=True)
            self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._listener:
            self._listener.stop()
        if self.socket_path.exists():
            self.socket_path.unlink()

    def _waybar_cache_loop(self) -> None:
        """Keep module cache warm for exec fallback; CFFI module owns live updates."""
        while not self._stop.is_set():
            try:
                from tsk.waybar_cache import refresh_modules_cache

                refresh_modules_cache(notify=False)
            except Exception:
                pass
            self._stop.wait(1.0)

    def _serve(self, server: socket.socket) -> None:
        server.settimeout(1.0)
        while not self._stop.is_set():
            try:
                conn, _ = server.accept()
            except TimeoutError:
                continue
            except OSError:
                break
            threading.Thread(target=self._handle_client, args=(conn,), daemon=True).start()
        server.close()

    def _handle_client(self, conn: socket.socket) -> None:
        with conn:
            try:
                data = conn.recv(65536).decode("utf-8").strip()
                if not data:
                    return
                request = json.loads(data)
                response = self._dispatch(request)
                conn.sendall((json.dumps(response) + "\n").encode("utf-8"))
            except (json.JSONDecodeError, OSError):
                pass

    def _dispatch(self, request: dict[str, Any]) -> dict[str, Any]:
        method = request.get("method", "")
        params = request.get("params") or {}
        try:
            result = self._call(method, params)
            return {"ok": True, "result": result}
        except Exception as exc:
            return {"ok": False, "error": str(exc)}

    def _call(self, method: str, params: dict[str, Any]) -> Any:
        from tsk.daemon import workspace_nav

        state = self.service.get_state()

        if method == "get_state":
            return json.loads(state.model_dump_json())

        if method == "create_task":
            task = self.service.create_task(
                params["name"],
                repo_url=params.get("repo_url"),
                branch=params.get("branch"),
                switch=params.get("switch", True),
            )
            return task.model_dump(mode="json")

        if method == "switch_task":
            task = self.service.switch_task(params["task_id"])
            return json.loads(task.model_dump_json())

        if method == "archive_task":
            self.service.archive_task(params["task_id"])
            return {"archived": params["task_id"]}

        if method == "delete_task":
            self.service.delete_task(params["task_id"])
            return {"deleted": params["task_id"]}

        if method == "set_context":
            mode = params["mode"]
            if mode in ("default", "global"):
                self.service.context_default()
            elif mode == ContextMode.task.value:
                self.service.switch_task(params["task_id"])
            else:
                raise ValueError(f"Unknown context mode: {mode}")
            return {"context": self.service.get_state().context_label()}

        if method in ("workspace_go", "desktop_go"):
            ws = workspace_nav.workspace_go(state, int(params["relative"]))
            self.service.save_state(state)
            return {"workspace": ws}

        if method in ("workspace_next", "desktop_next"):
            ws = workspace_nav.workspace_next(state)
            self.service.save_state(state)
            return {"workspace": ws}

        if method in ("workspace_prev", "desktop_prev"):
            ws = workspace_nav.workspace_prev(state)
            self.service.save_state(state)
            return {"workspace": ws}

        if method in ("workspace_goto", "desktop_goto"):
            ws = workspace_nav.workspace_goto_name(state, str(params["name"]))
            self.service.save_state(state)
            return {"workspace": ws}

        if method == "open_terminal":
            self.service.open_terminal(
                params.get("task_id"),
                host=params.get("host", False),
            )
            return {"launched": True}

        if method == "tasks_for_menu":
            return self.service.tasks_for_menu()

        if method == "status":
            return self.service.status_summary()

        if method == "reconcile_windows":
            self.router.reconcile()
            return {"ok": True}

        raise ValueError(f"Unknown method: {method}")


def is_daemon_running() -> bool:
    try:
        path = xdg.tsk_daemon_socket()
    except RuntimeError:
        return False
    return path.exists()


def daemon_request(method: str, params: dict[str, Any] | None = None, timeout: float = 5.0) -> dict:
    path = xdg.tsk_daemon_socket()
    if not path.exists():
        raise ConnectionError("tsk daemon is not running")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(timeout)
        sock.connect(str(path))
        payload = json.dumps({"method": method, "params": params or {}})
        sock.sendall((payload + "\n").encode("utf-8"))
        response_data = b""
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            response_data += chunk
            if b"\n" in response_data:
                break
        return json.loads(response_data.decode("utf-8").split("\n")[0])


def run_daemon_foreground() -> None:
    server = DaemonServer()
    server.start(foreground=True)

    def _shutdown(_signum, _frame):
        server.stop()
        raise SystemExit(0)

    signal.signal(signal.SIGINT, _shutdown)
    signal.signal(signal.SIGTERM, _shutdown)

    try:
        signal.pause()
    except AttributeError:
        import time

        while True:
            time.sleep(3600)

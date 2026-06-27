"""Hyprland event socket listener."""

from __future__ import annotations

import os
import socket
import threading
from collections.abc import Callable
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from lae.daemon.window_router import WindowRouter


EventHandler = Callable[[str, str], None]


class HyprlandEventListener:
    def __init__(self, handler: EventHandler):
        self._handler = handler
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self._sock: socket.socket | None = None

    def socket_path(self) -> str | None:
        sig = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")
        runtime = os.environ.get("XDG_RUNTIME_DIR")
        if not sig or not runtime:
            return None
        return f"{runtime}/hypr/{sig}/.socket2.sock"

    def start(self) -> bool:
        path = self.socket_path()
        if not path or not os.path.exists(path):
            return False
        self._stop.clear()
        self._thread = threading.Thread(target=self._run, args=(path,), daemon=True)
        self._thread.start()
        return True

    def stop(self) -> None:
        self._stop.set()
        if self._sock:
            try:
                self._sock.close()
            except OSError:
                pass
        if self._thread:
            self._thread.join(timeout=2)

    def _run(self, path: str) -> None:
        while not self._stop.is_set():
            try:
                self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self._sock.connect(path)
                buf = ""
                while not self._stop.is_set():
                    chunk = self._sock.recv(4096)
                    if not chunk:
                        break
                    buf += chunk.decode("utf-8", errors="replace")
                    while "\n" in buf:
                        line, buf = buf.split("\n", 1)
                        self._parse_line(line.strip())
            except OSError:
                if self._stop.is_set():
                    break
                self._stop.wait(1)
            finally:
                if self._sock:
                    try:
                        self._sock.close()
                    except OSError:
                        pass
                    self._sock = None

    def _parse_line(self, line: str) -> None:
        if not line or ">>" not in line:
            return
        event, payload = line.split(">>", 1)
        self._handler(event.strip(), payload.strip())


def make_window_handler(router: WindowRouter) -> EventHandler:
    def handler(event: str, payload: str) -> None:
        if event == "openwindow":
            router.on_open_window(payload)
        elif event == "closewindow":
            router.on_close_window(payload)
        elif event == "activewindow":
            router.on_active_window(payload)
        elif event == "workspace":
            router.on_workspace(payload)

    return handler

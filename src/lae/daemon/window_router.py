"""Window-to-task correlation."""

from __future__ import annotations

import re

from lae.core.models import ContextMode, SessionState, WindowRecord
from lae.core.workspaces import is_default_workspace_name
from lae.integrations import hyprland

TITLE_PREFIX = re.compile(r"^\[([a-z0-9-]+)\]")


class WindowRouter:
    def __init__(self, get_state, save_state, *, auto_move: bool = True):
        self._get_state = get_state
        self._save_state = save_state
        self.auto_move = auto_move

    def reconcile(self) -> None:
        if not hyprland.available():
            return
        state = self._get_state()
        clients = hyprland.get_clients()
        seen: set[str] = set()
        for client in clients:
            seen.add(client.address)
            task_id = self._task_from_title(client.title) or self._task_from_workspace(
                client.workspace_name
            )
            record = state.windows.get(client.address)
            if record is None:
                record = WindowRecord(
                    hypr_address=client.address,
                    task_id=task_id,
                    title=client.title,
                    **{"class": client.class_},
                    workspace=client.workspace,
                    workspace_name=client.workspace_name,
                    pid=client.pid,
                )
                state.windows[client.address] = record
            else:
                record.title = client.title
                record.class_ = client.class_
                record.workspace = client.workspace
                record.workspace_name = client.workspace_name
                record.pid = client.pid
                if task_id:
                    record.task_id = task_id
            if task_id and self.auto_move:
                task = state.tasks.get(task_id)
                if task:
                    main_ws = task.main_workspace()
                    if client.workspace_name != main_ws:
                        hyprland.move_window_to_workspace(client.address, main_ws)
        stale = [addr for addr in state.windows if addr not in seen]
        for addr in stale:
            del state.windows[addr]
        self._save_state(state)

    def on_open_window(self, address: str) -> None:
        if not hyprland.available():
            return
        state = self._get_state()
        clients = {c.address: c for c in hyprland.get_clients()}
        client = clients.get(f"0x{address}" if not address.startswith("0x") else address)
        if client is None:
            for c in clients.values():
                if c.address.endswith(address):
                    client = c
                    break
        if client is None:
            return

        task_id = self._task_from_title(client.title)
        if not task_id and state.context_mode == ContextMode.task and state.current_task_id:
            task_id = state.current_task_id

        record = WindowRecord(
            hypr_address=client.address,
            task_id=task_id,
            title=client.title,
            **{"class": client.class_},
            workspace=client.workspace,
            workspace_name=client.workspace_name,
            pid=client.pid,
        )
        state.windows[client.address] = record

        if task_id and self.auto_move:
            task = state.tasks.get(task_id)
            if task:
                main_ws = task.main_workspace()
                hyprland.move_window_to_workspace(client.address, main_ws)
                record.workspace_name = main_ws
        self._save_state(state)

    def on_close_window(self, address: str) -> None:
        state = self._get_state()
        key = f"0x{address}" if not address.startswith("0x") else address
        state.windows.pop(key, None)
        for addr in list(state.windows):
            if addr.endswith(address):
                del state.windows[addr]
        self._save_state(state)

    def on_active_window(self, _payload: str) -> None:
        pass

    def on_workspace(self, payload: str) -> None:
        state = self._get_state()
        from lae.daemon import context_sync

        context_sync.sync_from_active_workspace(state)
        self._save_state(state)

    @staticmethod
    def _task_from_title(title: str) -> str | None:
        match = TITLE_PREFIX.match(title.strip())
        return match.group(1) if match else None

    @staticmethod
    def _task_from_workspace(workspace_name: str) -> str | None:
        if is_default_workspace_name(workspace_name, 9):
            return None
        parts = workspace_name.rsplit("-", 1)
        if len(parts) != 2 or not parts[1].isdigit():
            return None
        return parts[0]

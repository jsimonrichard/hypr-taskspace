"""XDG path helpers."""

from __future__ import annotations

import os
from pathlib import Path


def expand(path: str | Path) -> Path:
    return Path(os.path.expanduser(str(path))).resolve()


def config_home() -> Path:
    return expand(os.environ.get("XDG_CONFIG_HOME", "~/.config"))


def data_home() -> Path:
    return expand(os.environ.get("XDG_DATA_HOME", "~/.local/share"))


def runtime_dir() -> Path:
    base = os.environ.get("XDG_RUNTIME_DIR")
    if not base:
        raise RuntimeError("XDG_RUNTIME_DIR is not set")
    return Path(base)


def tsk_config_dir() -> Path:
    return config_home() / "tsk"


def tsk_config_path() -> Path:
    return tsk_config_dir() / "config.toml"


def tsk_data_dir() -> Path:
    return data_home() / "tsk"


def tsk_state_db() -> Path:
    return tsk_data_dir() / "state.db"


def tsk_runtime_dir() -> Path:
    return runtime_dir() / "tsk"


def resolve_daemon_socket(configured: str) -> Path:
    """Resolve `[daemon].socket` from config to an absolute path."""
    value = configured.strip()
    if value.startswith("~") or value.startswith("/"):
        return expand(value)
    if value in {"daemon.sock", "tsk/daemon.sock"}:
        return tsk_data_dir() / "daemon.sock"
    return tsk_data_dir() / value


def tsk_daemon_socket() -> Path:
    from tsk.core.config import load_config

    return resolve_daemon_socket(load_config().daemon_socket)


def tsk_context_file() -> Path:
    return tsk_runtime_dir() / "context"


def tsk_waybar_file() -> Path:
    return tsk_runtime_dir() / "waybar.json"


def share_dir() -> Path:
    """Shipped static templates (repo share/ or installed package)."""
    pkg = Path(__file__).resolve().parent.parent
    candidate = pkg.parent.parent / "share"
    if candidate.is_dir():
        return candidate
    import importlib.resources as ir

    try:
        ref = ir.files("tsk").joinpath("../../share")
        resolved = Path(str(ref))
        if resolved.is_dir():
            return resolved
    except (TypeError, FileNotFoundError):
        pass
    return tsk_data_dir()

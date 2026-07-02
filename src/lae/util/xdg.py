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


def lae_config_dir() -> Path:
    return config_home() / "lae"


def lae_config_path() -> Path:
    return lae_config_dir() / "config.toml"


def lae_data_dir() -> Path:
    return data_home() / "lae"


def lae_state_db() -> Path:
    return lae_data_dir() / "state.db"


def lae_runtime_dir() -> Path:
    return runtime_dir() / "lae"


def resolve_daemon_socket(configured: str) -> Path:
    """Resolve `[daemon].socket` from config to an absolute path."""
    value = configured.strip()
    if value.startswith("~") or value.startswith("/"):
        return expand(value)
    if value in {"daemon.sock", "lae/daemon.sock"}:
        return lae_data_dir() / "daemon.sock"
    return lae_data_dir() / value


def lae_daemon_socket() -> Path:
    from lae.core.config import load_config

    return resolve_daemon_socket(load_config().daemon_socket)


def lae_context_file() -> Path:
    return lae_runtime_dir() / "context"


def lae_waybar_file() -> Path:
    return lae_runtime_dir() / "waybar.json"


def share_dir() -> Path:
    """Shipped static templates (repo share/ or installed package)."""
    pkg = Path(__file__).resolve().parent.parent
    candidate = pkg.parent.parent / "share"
    if candidate.is_dir():
        return candidate
    import importlib.resources as ir

    try:
        ref = ir.files("lae").joinpath("../../share")
        resolved = Path(str(ref))
        if resolved.is_dir():
            return resolved
    except (TypeError, FileNotFoundError):
        pass
    return lae_data_dir()

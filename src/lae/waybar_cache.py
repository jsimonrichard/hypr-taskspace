"""Fast Waybar module cache — avoids spawning heavy lae imports on every poll."""

from __future__ import annotations

import fcntl
import json
import os
import time
from pathlib import Path
from typing import Any

# Daemon refreshes every 1s; Waybar polls every 1s. Allow headroom to avoid stampede.
CACHE_MAX_AGE_S = 5.0


def _runtime_dir() -> Path:
    base = os.environ.get("XDG_RUNTIME_DIR")
    if not base:
        raise RuntimeError("XDG_RUNTIME_DIR is not set")
    return Path(base) / "lae"


def modules_cache_path() -> Path:
    return _runtime_dir() / "waybar-modules.json"


def refresh_lock_path() -> Path:
    return _runtime_dir() / "waybar.refresh.lock"


def cache_age_s() -> float | None:
    path = modules_cache_path()
    if not path.is_file():
        return None
    return time.time() - path.stat().st_mtime


def cache_is_stale() -> bool:
    age = cache_age_s()
    return age is None or age > CACHE_MAX_AGE_S


def read_full_cache(*, allow_stale: bool = True) -> dict[str, Any] | None:
    path = modules_cache_path()
    if not path.is_file():
        return None
    if not allow_stale and cache_is_stale():
        return None
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None
    return data if isinstance(data, dict) else None


def read_cached_module(module: str, *, allow_stale: bool = True) -> dict[str, Any] | None:
    data = read_full_cache(allow_stale=allow_stale)
    if not data:
        return None
    payload = data.get(module)
    return payload if isinstance(payload, dict) else None


def write_modules_cache(
    modules: dict[str, dict[str, Any]], *, notify: bool = False
) -> None:
    runtime = _runtime_dir()
    runtime.mkdir(parents=True, exist_ok=True)
    tmp = modules_cache_path().with_suffix(".tmp")
    tmp.write_text(json.dumps(modules, separators=(",", ":")) + "\n")
    tmp.replace(modules_cache_path())
    if notify:
        from lae.integrations.waybar_notify import notify_waybar

        notify_waybar()


def refresh_modules_cache(*, notify: bool = False) -> bool:
    """Rebuild all Waybar module JSON in one pass. Returns True if state changed."""
    from lae.daemon import context_sync
    from lae.daemon.service import TaskService
    from lae.daemon.waybar_export import build_all_modules

    service = TaskService()
    state = service.get_state()
    changed = context_sync.sync_from_active_workspace(state)
    modules = build_all_modules(state, sync=False)
    previous = read_full_cache(allow_stale=True)
    if modules != previous:
        write_modules_cache(modules, notify=notify)
    if changed:
        service.save_state(state)
    return changed


def ensure_fresh_cache() -> None:
    """Refresh stale cache once; concurrent callers wait for the refresh to finish."""
    if not cache_is_stale():
        return

    try:
        runtime = _runtime_dir()
    except RuntimeError:
        refresh_modules_cache()
        return

    runtime.mkdir(parents=True, exist_ok=True)
    lock_path = refresh_lock_path()
    lock = lock_path.open("w")
    try:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            owns_refresh = True
        except BlockingIOError:
            owns_refresh = False

        if owns_refresh:
            if cache_is_stale():
                refresh_modules_cache()
        else:
            # Another module is refreshing — wait, then read the updated cache.
            fcntl.flock(lock, fcntl.LOCK_SH)
    finally:
        fcntl.flock(lock, fcntl.LOCK_UN)
        lock.close()
        try:
            lock_path.unlink(missing_ok=True)
        except OSError:
            pass


def emit_module(module: str) -> dict[str, Any]:
    """Return module JSON, serving stale cache rather than blanking modules mid-refresh."""
    fallback = read_cached_module(module, allow_stale=True)
    if fallback and not cache_is_stale():
        return fallback
    if cache_is_stale():
        ensure_fresh_cache()
    return read_cached_module(module, allow_stale=True) or fallback or {
        "text": "",
        "class": "hidden",
    }


# Deprecated alias
refresh_modules_cache_with_lock = ensure_fresh_cache

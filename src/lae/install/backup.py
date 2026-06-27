"""Timestamped full-file config backups."""

from __future__ import annotations

import shutil
from datetime import datetime, timezone
from pathlib import Path


def backup_timestamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H%M%S")


def backup_file(source: Path, backup_root: Path) -> Path:
    backup_root.mkdir(parents=True, exist_ok=True)
    dest = backup_root / source.name
    shutil.copy2(source, dest)
    return dest


def restore_file(backup: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(backup, destination)

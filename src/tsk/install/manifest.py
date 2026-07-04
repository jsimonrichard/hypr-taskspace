"""Install manifest read/write."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass
class Manifest:
    version: int = 1
    integration: str = "hypr"
    installed_at: str = field(
        default_factory=lambda: datetime.now(timezone.utc).isoformat()
    )
    backup_dir: str = ""
    templates_installed: list[dict[str, str]] = field(default_factory=list)
    user_files_backed_up: list[dict[str, str]] = field(default_factory=list)
    user_files_modified: list[dict[str, Any]] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "version": self.version,
            "integration": self.integration,
            "installed_at": self.installed_at,
            "backup_dir": self.backup_dir,
            "templates_installed": self.templates_installed,
            "user_files_backed_up": self.user_files_backed_up,
            "user_files_modified": self.user_files_modified,
        }

    @classmethod
    def from_dict(cls, data: dict) -> Manifest:
        return cls(
            version=int(data.get("version", 1)),
            integration=str(data.get("integration", "hypr")),
            installed_at=str(data.get("installed_at", "")),
            backup_dir=str(data.get("backup_dir", "")),
            templates_installed=list(data.get("templates_installed", [])),
            user_files_backed_up=list(data.get("user_files_backed_up", [])),
            user_files_modified=list(data.get("user_files_modified", [])),
        )


def manifest_path(share_dir: Path, integration: str = "hypr") -> Path:
    return share_dir / "install" / integration / "manifest.json"


def load_manifest(share_dir: Path, integration: str = "hypr") -> Manifest | None:
    path = manifest_path(share_dir, integration)
    if not path.exists():
        return None
    return Manifest.from_dict(json.loads(path.read_text()))


def save_manifest(share_dir: Path, manifest: Manifest) -> Path:
    path = manifest_path(share_dir, manifest.integration)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest.to_dict(), indent=2) + "\n")
    return path

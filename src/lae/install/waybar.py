"""Waybar integration install / uninstall."""

from __future__ import annotations

import json
import re
import shutil
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from lae.core.config import LaeConfig, load_config
from lae.install import backup, manifest
from lae.install.reload import apply_after_waybar
from lae.util import xdg

WAYBAR_WORKSPACE_SLOTS = 10
LAE_TASK_MODULE = "custom/lae-task"
LAE_WORKSPACE_MODULES = [
    f"custom/lae-workspace-{n}" for n in range(1, WAYBAR_WORKSPACE_SLOTS + 1)
]
LAE_MODULES_LEFT = [LAE_TASK_MODULE] + LAE_WORKSPACE_MODULES


@dataclass
class WaybarInstallPlan:
    templates: list[tuple[Path, Path]]
    config_path: Path
    backup_dir: Path
    modules_left_before: list[str] | None = None
    reload_actions: list[str] | None = None


def _repo_share() -> Path:
    root = Path(__file__).resolve().parents[3]
    share = root / "share" / "waybar"
    if share.is_dir():
        return share
    return xdg.lae_data_dir() / "waybar"


def _default_waybar_config() -> Path:
    return xdg.config_home() / "waybar" / "config.jsonc"


def _load_jsonc(path: Path) -> dict:
    text = path.read_text()
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    text = re.sub(r"//.*?$", "", text, flags=re.MULTILINE)
    return json.loads(text)


def _dump_jsonc(data: dict) -> str:
    return json.dumps(data, indent=2) + "\n"


def _is_lae_module(name: str) -> bool:
    return name == LAE_TASK_MODULE or name.startswith(
        ("custom/lae-desktop-", "custom/lae-workspace-")
    )


def plan_install(cfg: LaeConfig | None = None) -> WaybarInstallPlan:
    cfg = cfg or load_config()
    share_src = _repo_share()
    share_dest = cfg.install_hypr_share_dir / "waybar"
    templates: list[tuple[Path, Path]] = []
    for src in share_src.glob("*"):
        if src.is_file():
            templates.append((src, share_dest / src.name))

    config_path = _default_waybar_config()
    backup_dir = (
        cfg.install_hypr_share_dir / "install" / "waybar" / "backups" / backup.backup_timestamp()
    )
    before = None
    if config_path.exists():
        data = _load_jsonc(config_path)
        before = list(data.get("modules-left", []))

    return WaybarInstallPlan(
        templates=templates,
        config_path=config_path,
        backup_dir=backup_dir,
        modules_left_before=before,
    )


def _patch_config(config_path: Path) -> bool:
    data = _load_jsonc(config_path)

    for key in list(data.keys()):
        if key.startswith("custom/lae-desktop-") or key.startswith("custom/lae-workspace-"):
            del data[key]

    modules_left: list[str] = [
        m for m in data.get("modules-left", []) if not _is_lae_module(m)
    ]
    insert_at = 0
    if modules_left and modules_left[0].startswith("custom/omarchy"):
        insert_at = 1
    modules_left[insert_at:insert_at] = LAE_MODULES_LEFT
    data["modules-left"] = modules_left

    modules_path = _repo_share() / "modules.jsonc"
    if modules_path.is_file():
        extra = _load_jsonc(modules_path)
        data.update(extra)

    config_path.write_text(_dump_jsonc(data))
    return True


def _patch_style(config_dir: Path, share_dest: Path) -> None:
    style_path = config_dir / "style.css"
    marker = "/* lae-waybar */"
    snippet_path = share_dest / "lae-style.css"
    if not snippet_path.is_file() or not style_path.is_file():
        return
    content = style_path.read_text()
    snippet = snippet_path.read_text().strip()
    if marker in content:
        content = content[: content.index(marker)].rstrip()
    style_path.write_text(content + f"\n\n{marker}\n{snippet}\n")


def install_waybar(*, dry_run: bool = False, cfg: LaeConfig | None = None) -> WaybarInstallPlan:
    cfg = cfg or load_config()
    plan = plan_install(cfg)
    if dry_run:
        return plan

    share_dest = cfg.install_hypr_share_dir / "waybar"
    for src, dest in plan.templates:
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)

    if plan.config_path.exists():
        backup.backup_file(plan.config_path, plan.backup_dir)
        style_path = plan.config_path.parent / "style.css"
        if style_path.exists():
            backup.backup_file(style_path, plan.backup_dir)
        _patch_config(plan.config_path)
        _patch_style(plan.config_path.parent, share_dest)
    else:
        raise RuntimeError(f"Waybar config not found: {plan.config_path}")

    m = manifest.Manifest(
        integration="waybar",
        backup_dir=str(plan.backup_dir),
        templates_installed=[{"from": str(s), "to": str(d)} for s, d in plan.templates],
        user_files_backed_up=[
            {"path": str(plan.config_path), "backup": plan.config_path.name},
            {"path": str(plan.config_path.parent / "style.css"), "backup": "style.css"},
        ],
        user_files_modified=[
            {
                "path": str(plan.config_path),
                "actions": [{"type": "replace_hyprland_workspaces", "with": LAE_MODULES_LEFT}],
            }
        ],
    )
    manifest.save_manifest(cfg.install_hypr_share_dir, m)
    plan.reload_actions = apply_after_waybar()
    return plan


def uninstall_waybar(*, cfg: LaeConfig | None = None) -> list[str]:
    cfg = cfg or load_config()
    m = manifest.load_manifest(cfg.install_hypr_share_dir, "waybar")
    if m is None:
        raise RuntimeError("No lae Waybar installation found")

    backup_root = Path(m.backup_dir)
    for entry in m.user_files_backed_up:
        src = backup_root / entry["backup"]
        dest = Path(entry["path"]).expanduser()
        if src.exists():
            backup.restore_file(src, dest)

    manifest_path = cfg.install_hypr_share_dir / "install" / "waybar" / "manifest.json"
    if manifest_path.exists():
        manifest_path.unlink()

    return apply_after_waybar()


def install_status(cfg: LaeConfig | None = None) -> dict:
    cfg = cfg or load_config()
    m = manifest.load_manifest(cfg.install_hypr_share_dir, "waybar")
    config_path = _default_waybar_config()
    has_modules = False
    if config_path.exists():
        data = _load_jsonc(config_path)
        left = data.get("modules-left", [])
        has_modules = all(x in left for x in LAE_MODULES_LEFT)
    return {
        "installed": m is not None,
        "manifest": m.to_dict() if m else None,
        "lae_modules_present": has_modules,
        "config_path": str(config_path),
    }

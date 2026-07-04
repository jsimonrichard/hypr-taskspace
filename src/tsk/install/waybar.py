"""Waybar integration install / uninstall."""

from __future__ import annotations

import json
import re
import shutil
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from tsk.core.config import TskConfig, load_config
from tsk.install import backup, manifest
from tsk.install.reload import apply_after_waybar
from tsk.util import xdg

WAYBAR_WORKSPACE_SLOTS = 10
TSK_TASK_MODULE = "custom/tsk-task"
TSK_WORKSPACE_MODULES = [
    f"custom/tsk-workspace-{n}" for n in range(1, WAYBAR_WORKSPACE_SLOTS + 1)
]
TSK_MODULES_LEFT = [TSK_TASK_MODULE] + TSK_WORKSPACE_MODULES


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
    return xdg.tsk_data_dir() / "waybar"


def _default_waybar_config() -> Path:
    return xdg.config_home() / "waybar" / "config.jsonc"


def _load_jsonc(path: Path) -> dict:
    text = path.read_text()
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    text = re.sub(r"//.*?$", "", text, flags=re.MULTILINE)
    return json.loads(text)


def _dump_jsonc(data: dict) -> str:
    return json.dumps(data, indent=2) + "\n"


def _is_tsk_module(name: str) -> bool:
    return name == TSK_TASK_MODULE or name.startswith("custom/tsk-workspace-")


def _config_has_tsk(data: dict) -> bool:
    if any(_is_tsk_module(m) for m in data.get("modules-left", [])):
        return True
    return any(
        key == TSK_TASK_MODULE or key.startswith("custom/tsk-workspace-")
        for key in data
    )


def _pristine_backup_dir(cfg: TskConfig) -> Path:
    return cfg.install_hypr_share_dir / "install" / "waybar" / "backups" / "pristine"


def _seed_pristine_from_oldest_backup(cfg: TskConfig) -> bool:
    """One-time bootstrap when pristine was never saved (e.g. reinstall overwrote backups)."""
    pristine = _pristine_backup_dir(cfg)
    if (pristine / "config.jsonc").exists():
        return False

    backups_root = cfg.install_hypr_share_dir / "install" / "waybar" / "backups"
    if not backups_root.is_dir():
        return False

    candidates = sorted(
        d
        for d in backups_root.iterdir()
        if d.is_dir() and d.name not in {"pristine", "original"}
    )
    for backup_dir in candidates:
        config_backup = backup_dir / "config.jsonc"
        if not config_backup.is_file():
            continue
        try:
            data = _load_jsonc(config_backup)
        except (json.JSONDecodeError, OSError):
            continue
        if _config_has_tsk(data):
            continue
        pristine.mkdir(parents=True, exist_ok=True)
        shutil.copy2(config_backup, pristine / "config.jsonc")
        style_backup = backup_dir / "style.css"
        if style_backup.is_file():
            shutil.copy2(style_backup, pristine / "style.css")
        return True
    return False


def _ensure_pristine_backup(
    config_path: Path, style_path: Path, cfg: TskConfig
) -> None:
    pristine = _pristine_backup_dir(cfg)
    if (pristine / config_path.name).exists():
        return
    pristine.mkdir(parents=True, exist_ok=True)
    backup.backup_file(config_path, pristine)
    if style_path.is_file():
        backup.backup_file(style_path, pristine)


def plan_install(cfg: TskConfig | None = None) -> WaybarInstallPlan:
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
        if key.startswith("custom/tsk-workspace-"):
            del data[key]

    modules_left: list[str] = [
        m for m in data.get("modules-left", []) if not _is_tsk_module(m)
    ]
    insert_at = 0
    if modules_left and modules_left[0].startswith("custom/omarchy"):
        insert_at = 1
    modules_left[insert_at:insert_at] = TSK_MODULES_LEFT
    data["modules-left"] = modules_left

    modules_path = _repo_share() / "modules.jsonc"
    if modules_path.is_file():
        extra = _load_jsonc(modules_path)
        data.update(extra)

    config_path.write_text(_dump_jsonc(data))
    return True


def _patch_style(config_dir: Path, share_dest: Path) -> None:
    style_path = config_dir / "style.css"
    marker = "/* tsk-waybar */"
    snippet_path = share_dest / "tsk-style.css"
    if not snippet_path.is_file() or not style_path.is_file():
        return
    content = style_path.read_text()
    snippet = snippet_path.read_text().strip()
    if marker in content:
        content = content[: content.index(marker)].rstrip()
    style_path.write_text(content + f"\n\n{marker}\n{snippet}\n")


def _unpatch_config(config_path: Path) -> bool:
    """Remove tsk modules and put hyprland/workspaces back on the bar."""
    if not config_path.is_file():
        return False

    data = _load_jsonc(config_path)
    changed = False

    for key in list(data.keys()):
        if key == TSK_TASK_MODULE or key.startswith("custom/tsk-workspace-"):
            del data[key]
            changed = True

    modules_left = [m for m in data.get("modules-left", []) if not _is_tsk_module(m)]
    if "hyprland/workspaces" not in modules_left:
        insert_at = (
            1
            if modules_left and modules_left[0].startswith("custom/omarchy")
            else 0
        )
        modules_left.insert(insert_at, "hyprland/workspaces")
        changed = True

    if modules_left != data.get("modules-left"):
        data["modules-left"] = modules_left
        changed = True

    if changed:
        config_path.write_text(_dump_jsonc(data))
    return changed


def _unpatch_style(config_dir: Path) -> bool:
    style_path = config_dir / "style.css"
    marker = "/* tsk-waybar */"
    if not style_path.is_file():
        return False
    content = style_path.read_text()
    if marker not in content:
        return False
    style_path.write_text(content[: content.index(marker)].rstrip() + "\n")
    return True


def _restore_from_pristine(cfg: TskConfig, config_path: Path) -> bool:
    pristine = _pristine_backup_dir(cfg)
    config_backup = pristine / config_path.name
    if not config_backup.is_file():
        return False
    backup.restore_file(config_backup, config_path)
    style_backup = pristine / "style.css"
    style_path = config_path.parent / "style.css"
    if style_backup.is_file() and style_path.parent.exists():
        backup.restore_file(style_backup, style_path)
    return True


def install_waybar(*, dry_run: bool = False, cfg: TskConfig | None = None) -> WaybarInstallPlan:
    cfg = cfg or load_config()
    plan = plan_install(cfg)
    if dry_run:
        return plan

    share_dest = cfg.install_hypr_share_dir / "waybar"
    for src, dest in plan.templates:
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)

    if plan.config_path.exists():
        style_path = plan.config_path.parent / "style.css"
        data = _load_jsonc(plan.config_path)
        if not _config_has_tsk(data):
            backup.backup_file(plan.config_path, plan.backup_dir)
            if style_path.exists():
                backup.backup_file(style_path, plan.backup_dir)
            _ensure_pristine_backup(plan.config_path, style_path, cfg)
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
                "actions": [{"type": "replace_hyprland_workspaces", "with": TSK_MODULES_LEFT}],
            }
        ],
    )
    manifest.save_manifest(cfg.install_hypr_share_dir, m)
    plan.reload_actions = apply_after_waybar()
    return plan


def uninstall_waybar(*, cfg: TskConfig | None = None) -> list[str]:
    cfg = cfg or load_config()
    config_path = _default_waybar_config()
    m = manifest.load_manifest(cfg.install_hypr_share_dir, "waybar")

    _seed_pristine_from_oldest_backup(cfg)

    restored = _restore_from_pristine(cfg, config_path)
    if not restored and m is not None:
        backup_root = Path(m.backup_dir)
        for entry in m.user_files_backed_up:
            src = backup_root / entry["backup"]
            dest = Path(entry["path"]).expanduser()
            if src.is_file():
                backup.restore_file(src, dest)

    if config_path.is_file():
        _unpatch_config(config_path)
    _unpatch_style(config_path.parent)

    manifest_path = cfg.install_hypr_share_dir / "install" / "waybar" / "manifest.json"
    if manifest_path.exists():
        manifest_path.unlink()

    if m is None and not restored:
        has_tsk = False
        if config_path.is_file():
            try:
                has_tsk = _config_has_tsk(_load_jsonc(config_path))
            except (json.JSONDecodeError, OSError):
                pass
        if not has_tsk:
            raise RuntimeError("No tsk Waybar installation found")

    return apply_after_waybar()


def install_status(cfg: TskConfig | None = None) -> dict:
    cfg = cfg or load_config()
    m = manifest.load_manifest(cfg.install_hypr_share_dir, "waybar")
    config_path = _default_waybar_config()
    has_modules = False
    if config_path.exists():
        data = _load_jsonc(config_path)
        left = data.get("modules-left", [])
        has_modules = all(x in left for x in TSK_MODULES_LEFT)
    return {
        "installed": m is not None,
        "manifest": m.to_dict() if m else None,
        "tsk_modules_present": has_modules,
        "config_path": str(config_path),
    }

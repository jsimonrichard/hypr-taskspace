"""Hyprland integration install / uninstall."""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from lae.core.config import LaeConfig, load_config
from lae.install import backup, manifest
from lae.install.reload import apply_after_hypr
from lae.install.reload import apply_after_hypr
from lae.install.wrapper import bin_dir, write_lae_wrapper, write_menu_helper, write_waybar_helper
from lae.integrations import hyprland
from lae.util import xdg


@dataclass
class InstallPlan:
    templates: list[tuple[Path, Path]]
    elephant_menu: tuple[Path, Path] | None
    elephant_symlink: Path | None
    config_path: Path
    source_line: str
    backup_dir: Path
    already_installed: bool
    reload_actions: list[str] | None = None


def _repo_share() -> Path:
    root = Path(__file__).resolve().parents[3]
    share = root / "share"
    if share.is_dir():
        return share
    return xdg.share_dir()


def _elephant_paths(cfg: LaeConfig) -> tuple[tuple[Path, Path], Path]:
    src = _repo_share() / "elephant" / "lae_tasks.lua"
    dest = cfg.install_hypr_share_dir / "elephant" / "lae_tasks.lua"
    link = Path.home() / ".config/elephant/menus/lae_tasks.lua"
    return (src, dest), link


def plan_install(cfg: LaeConfig | None = None) -> InstallPlan:
    cfg = cfg or load_config()
    share_src = _repo_share() / "hypr"
    share_dest = cfg.install_hypr_share_dir / "hypr"
    templates: list[tuple[Path, Path]] = []
    for src in share_src.glob("*"):
        if src.is_file():
            templates.append((src, share_dest / src.name))

    elephant_copy, elephant_link = _elephant_paths(cfg)
    elephant_menu = elephant_copy if elephant_copy[0].is_file() else None

    ts = backup.backup_timestamp()
    backup_dir = cfg.install_hypr_share_dir / "install" / "hypr" / "backups" / ts
    existing = manifest.load_manifest(cfg.install_hypr_share_dir, "hypr")
    source_line = (
        f"source = {xdg.expand(cfg.install_hypr_source_line)}  "
        f"# lae-managed (installed {datetime.now(timezone.utc).date().isoformat()})"
    )
    return InstallPlan(
        templates=templates,
        elephant_menu=elephant_menu,
        elephant_symlink=elephant_link,
        config_path=cfg.install_hypr_config_path,
        source_line=source_line,
        backup_dir=backup_dir,
        already_installed=existing is not None,
    )


def _install_elephant_menu(cfg: LaeConfig) -> list[dict[str, str]]:
    installed: list[dict[str, str]] = []
    (src, dest), link = _elephant_paths(cfg)
    if not src.is_file():
        return installed
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dest)
    installed.append({"from": str(src), "to": str(dest)})
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.is_symlink() or link.exists():
        link.unlink()
    link.symlink_to(dest)
    installed.append({"symlink": str(link), "target": str(dest)})
    return installed


def install_hypr(*, dry_run: bool = False, cfg: LaeConfig | None = None) -> InstallPlan:
    cfg = cfg or load_config()
    plan = plan_install(cfg)

    if dry_run:
        return plan

    for src, dest in plan.templates:
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dest)

    elephant_installed = _install_elephant_menu(cfg)
    write_menu_helper(cfg)
    write_waybar_helper(cfg)
    lae_wrapper = write_lae_wrapper(cfg)

    config_path = plan.config_path
    backed_up: list[dict[str, str]] = []
    modified = False

    if config_path.exists():
        backup.backup_file(config_path, plan.backup_dir)
        backed_up.append({"path": str(config_path), "backup": config_path.name})
        content = config_path.read_text()
        if "lae-managed" not in content:
            with config_path.open("a") as f:
                f.write("\n" + plan.source_line + "\n")
            modified = True
    else:
        config_path.parent.mkdir(parents=True, exist_ok=True)
        config_path.write_text(plan.source_line + "\n")
        modified = True

    templates_installed = [{"from": str(s), "to": str(d)} for s, d in plan.templates]
    templates_installed.extend(elephant_installed)
    templates_installed.append({"generated": str(lae_wrapper)})
    templates_installed.append({"generated": str(bin_dir(cfg) / "lae-task-menu-json")})
    templates_installed.append({"generated": str(bin_dir(cfg) / "lae-waybar-json")})

    m = manifest.Manifest(
        integration="hypr",
        backup_dir=str(plan.backup_dir),
        templates_installed=templates_installed,
        user_files_backed_up=backed_up,
        user_files_modified=(
            [
                {
                    "path": str(config_path),
                    "actions": [{"type": "append", "line": plan.source_line}],
                }
            ]
            if modified
            else []
        ),
    )
    manifest.save_manifest(cfg.install_hypr_share_dir, m)

    plan.reload_actions = apply_after_hypr()

    return plan


def uninstall_hypr(*, keep_files: bool = False, cfg: LaeConfig | None = None) -> list[str]:
    cfg = cfg or load_config()
    m = manifest.load_manifest(cfg.install_hypr_share_dir, "hypr")
    if m is None:
        raise RuntimeError("No lae Hyprland installation found")

    backup_root = Path(m.backup_dir)
    for entry in m.user_files_backed_up:
        src = backup_root / entry["backup"]
        dest = Path(entry["path"]).expanduser()
        if src.exists():
            backup.restore_file(src, dest)

    elephant_link = Path.home() / ".config/elephant/menus/lae_tasks.lua"
    if elephant_link.is_symlink():
        elephant_link.unlink()

    if not keep_files:
        hypr_dir = cfg.install_hypr_share_dir / "hypr"
        if hypr_dir.exists():
            shutil.rmtree(hypr_dir)
        elephant_dir = cfg.install_hypr_share_dir / "elephant"
        if elephant_dir.exists():
            shutil.rmtree(elephant_dir)

    manifest_path = cfg.install_hypr_share_dir / "install" / "hypr" / "manifest.json"
    if manifest_path.exists():
        manifest_path.unlink()

    return apply_after_hypr()


def install_status(cfg: LaeConfig | None = None) -> dict:
    cfg = cfg or load_config()
    m = manifest.load_manifest(cfg.install_hypr_share_dir, "hypr")
    bindings = cfg.install_hypr_share_dir / "hypr" / "bindings.conf"
    elephant = cfg.install_hypr_share_dir / "elephant" / "lae_tasks.lua"
    elephant_link = Path.home() / ".config/elephant/menus/lae_tasks.lua"
    config_path = cfg.install_hypr_config_path
    has_source = False
    if config_path.exists():
        has_source = "lae-managed" in config_path.read_text()
    return {
        "installed": m is not None,
        "manifest": m.to_dict() if m else None,
        "bindings_exist": bindings.exists(),
        "elephant_menu_exist": elephant.exists(),
        "elephant_symlink": elephant_link.is_symlink(),
        "source_line_present": has_source,
        "config_path": str(config_path),
        "bindings_path": str(bindings),
    }


def doctor_checks(cfg: LaeConfig | None = None) -> list[tuple[str, bool, str]]:
    cfg = cfg or load_config()
    checks: list[tuple[str, bool, str]] = []
    status = install_status(cfg)

    checks.append(
        (
            "Hyprland bindings installed",
            status["bindings_exist"],
            str(cfg.install_hypr_share_dir / "hypr" / "bindings.conf"),
        )
    )
    checks.append(
        (
            "Walker task menu (elephant) installed",
            status["elephant_menu_exist"] and status["elephant_symlink"],
            str(Path.home() / ".config/elephant/menus/lae_tasks.lua"),
        )
    )
    checks.append(
        (
            "hyprland.conf contains lae source line",
            status["source_line_present"],
            str(cfg.install_hypr_config_path),
        )
    )

    backup_ok = False
    backup_msg = "no manifest"
    if status["manifest"]:
        backup_dir = Path(status["manifest"]["backup_dir"])
        backup_ok = backup_dir.is_dir() and any(backup_dir.iterdir())
        backup_msg = str(backup_dir)
    checks.append(("Install backup exists", backup_ok, backup_msg))

    from lae.daemon.server import is_daemon_running

    checks.append(
        (
            "Daemon reachable",
            is_daemon_running(),
            str(xdg.lae_daemon_socket()),
        )
    )
    checks.append(
        (
            "SUPER+1 runs lae (not Omarchy workspace)",
            _super_one_is_lae(),
            "hyprctl binds -j",
        )
    )
    return checks


def _super_one_is_lae() -> bool:
    if not hyprland.available():
        return False
    try:
        binds = hyprland.hyprctl_json("binds") or []
    except hyprland.HyprlandError:
        return False
    lae_binds = [
        b
        for b in binds
        if b.get("keycode") == 10
        and b.get("modmask") == 64
        and "lae workspace go" in str(b.get("arg", ""))
    ]
    omarchy_binds = [
        b
        for b in binds
        if b.get("keycode") in range(10, 20)
        and b.get("modmask") == 64
        and str(b.get("arg", "")).strip().isdigit()
        and "lae workspace go" not in str(b.get("arg", ""))
    ]
    return bool(lae_binds) and not omarchy_binds

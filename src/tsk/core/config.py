"""Load ~/.config/tsk/config.toml."""

from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from pathlib import Path

import tomli_w

from tsk.util import xdg


def _read_os_release() -> tuple[str, str]:
    try:
        content = Path("/etc/os-release").read_text(encoding="utf-8")
    except OSError:
        return "", ""
    fields: dict[str, str] = {}
    for line in content.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        fields[key.strip().lower()] = value.strip().strip('"').lower()
    return fields.get("id", ""), fields.get("id_like", "")


def default_distrobox_image() -> str:
    distro_id, id_like = _read_os_release()
    if distro_id in {
        "arch",
        "cachyos",
        "omarchy",
        "manjaro",
        "garuda",
        "endeavouros",
    } or "arch" in id_like:
        return "quay.io/toolbx-images/arch-toolbox:latest"
    if distro_id == "fedora" or "fedora" in id_like:
        return "quay.io/toolbx-images/fedora-toolbox:40"
    if distro_id in {"ubuntu", "pop", "linuxmint"}:
        return "quay.io/toolbx-images/ubuntu-toolbox:24.04"
    if distro_id in {"debian", "raspbian", "pureos"}:
        return "quay.io/toolbx-images/debian-toolbox:12"
    if distro_id in {"opensuse-tumbleweed", "opensuse-leap", "opensuse"}:
        return "quay.io/toolbx-images/opensuse-toolbox:tumbleweed"
    if distro_id == "alpine":
        return "quay.io/toolbx-images/alpine-toolbox:edge"
    if "debian" in id_like or "ubuntu" in id_like:
        return "quay.io/toolbx-images/ubuntu-toolbox:24.04"
    return "quay.io/toolbx-images/fedora-toolbox:40"


def default_daemon_socket_config_value() -> str:
    return "~/.local/share/tsk/daemon.sock"


def default_config_contents() -> str:
    return f"""\
[default]
workspace_count = 10

[tasks]
base_dir = "~/tsk-tasks"
workspaces_per_task = 3
max_tasks = 9

[distrobox]
image = "{default_distrobox_image()}"
container_prefix = "tsk"

[terminal]
command = "kitty"
title_flag = "--title"

[hyprland]
enabled = true
auto_move_tagged_windows = true
switch_task_on_window_focus = false

[daemon]
socket = "{default_daemon_socket_config_value()}"

[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"
share_dir = "~/.local/share/tsk"
source_line = "~/.local/share/tsk/hypr/bindings.conf"
require_sourced_last = true
allow_user_file_comments = false
"""


@dataclass
class TskConfig:
    default_workspace_count: int = 10
    tasks_base_dir: Path = field(default_factory=lambda: xdg.expand("~/tsk-tasks"))
    workspaces_per_task: int = 3
    max_tasks: int = 9
    distrobox_image: str = field(default_factory=default_distrobox_image)
    container_prefix: str = "tsk"
    terminal_command: str = "kitty"
    terminal_title_flag: str = "--title"
    hyprland_enabled: bool = True
    auto_move_tagged_windows: bool = True
    switch_task_on_window_focus: bool = False
    daemon_socket: str = field(default_factory=default_daemon_socket_config_value)
    install_hypr_config_path: Path = field(
        default_factory=lambda: xdg.expand("~/.config/hypr/hyprland.conf")
    )
    install_hypr_share_dir: Path = field(default_factory=xdg.tsk_data_dir)
    install_hypr_source_line: str = "~/.local/share/tsk/hypr/bindings.conf"
    install_hypr_require_sourced_last: bool = True
    install_hypr_allow_user_file_comments: bool = False

    @property
    def default_desktop_count(self) -> int:
        return self.default_workspace_count

    @property
    def default_workspace_range(self) -> tuple[int, int]:
        """Legacy compat — desktop count drives named workspaces now."""
        return (1, self.default_workspace_count)


def _parse_config(data: dict) -> TskConfig:
    default = data.get("default", {})
    tasks = data.get("tasks", {})
    distrobox = data.get("distrobox", {})
    terminal = data.get("terminal", {})
    hyprland = data.get("hyprland", {})
    daemon = data.get("daemon", {})
    install_hypr = data.get("install", {}).get("hypr", {})

    workspace_count = int(
        default.get("workspace_count", default.get("desktop_count", 10))
    )
    if "workspace_range" in default and "workspace_count" not in default and "desktop_count" not in default:
        ws_range = default["workspace_range"]
        workspace_count = int(ws_range[1]) - int(ws_range[0]) + 1

    per_task = int(tasks.get("workspaces_per_task", workspace_count))

    return TskConfig(
        default_workspace_count=workspace_count,
        tasks_base_dir=xdg.expand(tasks.get("base_dir", "~/tsk-tasks")),
        workspaces_per_task=per_task,
        max_tasks=int(tasks.get("max_tasks", 9)),
        distrobox_image=str(
            distrobox.get("image", default_distrobox_image())
        ),
        container_prefix=str(distrobox.get("container_prefix", "tsk")),
        terminal_command=str(terminal.get("command", "kitty")),
        terminal_title_flag=str(terminal.get("title_flag", "--title")),
        hyprland_enabled=bool(hyprland.get("enabled", True)),
        auto_move_tagged_windows=bool(hyprland.get("auto_move_tagged_windows", True)),
        switch_task_on_window_focus=bool(
            hyprland.get("switch_task_on_window_focus", False)
        ),
        daemon_socket=str(daemon.get("socket", default_daemon_socket_config_value())),
        install_hypr_config_path=xdg.expand(
            install_hypr.get("config_path", "~/.config/hypr/hyprland.conf")
        ),
        install_hypr_share_dir=xdg.expand(
            install_hypr.get("share_dir", "~/.local/share/tsk")
        ),
        install_hypr_source_line=str(
            install_hypr.get(
                "source_line", "~/.local/share/tsk/hypr/bindings.conf"
            )
        ),
        install_hypr_require_sourced_last=bool(
            install_hypr.get("require_sourced_last", True)
        ),
        install_hypr_allow_user_file_comments=bool(
            install_hypr.get("allow_user_file_comments", False)
        ),
    )


def ensure_config() -> Path:
    path = xdg.tsk_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        path.write_text(default_config_contents())
    return path


def load_config() -> TskConfig:
    path = ensure_config()
    with path.open("rb") as f:
        data = tomllib.load(f)
    return _parse_config(data)


def config_to_dict(cfg: TskConfig) -> dict:
    return {
        "default": {"workspace_count": cfg.default_workspace_count},
        "tasks": {
            "base_dir": str(cfg.tasks_base_dir),
            "workspaces_per_task": cfg.workspaces_per_task,
            "max_tasks": cfg.max_tasks,
        },
        "distrobox": {
            "image": cfg.distrobox_image,
            "container_prefix": cfg.container_prefix,
        },
        "terminal": {
            "command": cfg.terminal_command,
            "title_flag": cfg.terminal_title_flag,
        },
        "hyprland": {
            "enabled": cfg.hyprland_enabled,
            "auto_move_tagged_windows": cfg.auto_move_tagged_windows,
            "switch_task_on_window_focus": cfg.switch_task_on_window_focus,
        },
        "daemon": {"socket": cfg.daemon_socket},
        "install": {
            "hypr": {
                "config_path": str(cfg.install_hypr_config_path),
                "share_dir": str(cfg.install_hypr_share_dir),
                "source_line": cfg.install_hypr_source_line,
                "require_sourced_last": cfg.install_hypr_require_sourced_last,
                "allow_user_file_comments": cfg.install_hypr_allow_user_file_comments,
            }
        },
    }

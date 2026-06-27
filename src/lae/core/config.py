"""Load ~/.config/lae/config.toml."""

from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from pathlib import Path

import tomli_w

from lae.util import xdg


DEFAULT_CONFIG = """\
[default]
workspace_count = 10

[tasks]
base_dir = "~/lae-tasks"
workspaces_per_task = 3
max_tasks = 9

[distrobox]
image = "quay.io/toolbx-images/fedora-toolbox:40"
container_prefix = "lae"

[terminal]
command = "kitty"
title_flag = "--title"

[walker]
launch_command = "omarchy-launch-walker"
menu_width = 644
menu_height = 300

[hyprland]
enabled = true
auto_move_tagged_windows = true
switch_task_on_window_focus = false

[daemon]
socket = "lae/daemon.sock"

[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"
share_dir = "~/.local/share/lae"
source_line = "~/.local/share/lae/hypr/bindings.conf"
require_sourced_last = true
allow_user_file_comments = false
"""


@dataclass
class LaeConfig:
    default_workspace_count: int = 10
    tasks_base_dir: Path = field(default_factory=lambda: xdg.expand("~/lae-tasks"))
    workspaces_per_task: int = 3
    max_tasks: int = 9
    distrobox_image: str = "quay.io/toolbx-images/fedora-toolbox:40"
    container_prefix: str = "lae"
    terminal_command: str = "kitty"
    terminal_title_flag: str = "--title"
    walker_launch_command: str = "omarchy-launch-walker"
    walker_menu_width: int = 644
    walker_menu_height: int = 300
    hyprland_enabled: bool = True
    auto_move_tagged_windows: bool = True
    switch_task_on_window_focus: bool = False
    daemon_socket: str = "lae/daemon.sock"
    install_hypr_config_path: Path = field(
        default_factory=lambda: xdg.expand("~/.config/hypr/hyprland.conf")
    )
    install_hypr_share_dir: Path = field(default_factory=xdg.lae_data_dir)
    install_hypr_source_line: str = "~/.local/share/lae/hypr/bindings.conf"
    install_hypr_require_sourced_last: bool = True
    install_hypr_allow_user_file_comments: bool = False

    @property
    def default_desktop_count(self) -> int:
        return self.default_workspace_count

    @property
    def default_workspace_range(self) -> tuple[int, int]:
        """Legacy compat — desktop count drives named workspaces now."""
        return (1, self.default_workspace_count)


def _parse_config(data: dict) -> LaeConfig:
    default = data.get("default", {})
    tasks = data.get("tasks", {})
    distrobox = data.get("distrobox", {})
    terminal = data.get("terminal", {})
    walker = data.get("walker", {})
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

    return LaeConfig(
        default_workspace_count=workspace_count,
        tasks_base_dir=xdg.expand(tasks.get("base_dir", "~/lae-tasks")),
        workspaces_per_task=per_task,
        max_tasks=int(tasks.get("max_tasks", 9)),
        distrobox_image=str(
            distrobox.get("image", "quay.io/toolbx-images/fedora-toolbox:40")
        ),
        container_prefix=str(distrobox.get("container_prefix", "lae")),
        terminal_command=str(terminal.get("command", "kitty")),
        terminal_title_flag=str(terminal.get("title_flag", "--title")),
        walker_launch_command=str(walker.get("launch_command", "omarchy-launch-walker")),
        walker_menu_width=int(walker.get("menu_width", 644)),
        walker_menu_height=int(walker.get("menu_height", 300)),
        hyprland_enabled=bool(hyprland.get("enabled", True)),
        auto_move_tagged_windows=bool(hyprland.get("auto_move_tagged_windows", True)),
        switch_task_on_window_focus=bool(
            hyprland.get("switch_task_on_window_focus", False)
        ),
        daemon_socket=str(daemon.get("socket", "lae/daemon.sock")),
        install_hypr_config_path=xdg.expand(
            install_hypr.get("config_path", "~/.config/hypr/hyprland.conf")
        ),
        install_hypr_share_dir=xdg.expand(
            install_hypr.get("share_dir", "~/.local/share/lae")
        ),
        install_hypr_source_line=str(
            install_hypr.get(
                "source_line", "~/.local/share/lae/hypr/bindings.conf"
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
    path = xdg.lae_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        path.write_text(DEFAULT_CONFIG)
    return path


def load_config() -> LaeConfig:
    path = ensure_config()
    with path.open("rb") as f:
        data = tomllib.load(f)
    return _parse_config(data)


def config_to_dict(cfg: LaeConfig) -> dict:
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
        "walker": {
            "launch_command": cfg.walker_launch_command,
            "menu_width": cfg.walker_menu_width,
            "menu_height": cfg.walker_menu_height,
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

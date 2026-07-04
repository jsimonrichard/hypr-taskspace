"""Distrobox container management."""

from __future__ import annotations

from pathlib import Path

from tsk.util import subprocess as sp


def available() -> bool:
    return sp.which("distrobox") is not None


def container_exists(name: str) -> bool:
    if not available():
        return False
    result = sp.run(
        ["distrobox", "list", "--no-color"],
        check=False,
    )
    return name in (result.stdout or "")


def create_container(name: str, home: Path, image: str) -> None:
    if not available():
        raise RuntimeError(
            "distrobox is not installed. Install distrobox or use `tsk task terminal --host`."
        )
    home.mkdir(parents=True, exist_ok=True)
    sp.run(
        [
            "distrobox",
            "create",
            "-Y",
            "--name",
            name,
            "--image",
            image,
            "--home",
            str(home),
        ]
    )


def enter_command(name: str, cmd: str) -> list[str]:
    return ["distrobox", "enter", name, "--", "bash", "-lc", cmd]


def is_running(name: str) -> bool:
    if not available():
        return False
    result = sp.run(["distrobox", "list", "--no-color"], check=False)
    for line in (result.stdout or "").splitlines():
        if name in line and "running" in line.lower():
            return True
    return False


def stop_container(name: str) -> None:
    if not available() or not container_exists(name):
        return
    sp.run(["distrobox", "stop", "--name", name], check=False)


def remove_container(name: str) -> None:
    if not available() or not container_exists(name):
        return
    sp.run(["distrobox", "rm", "--name", name, "-Y"], check=False)

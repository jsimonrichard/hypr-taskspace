"""Git clone/init on host."""

from __future__ import annotations

from pathlib import Path

from lae.util import subprocess as sp


def clone_repo(url: str, dest: Path, branch: str | None = None) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    cmd = ["git", "clone", url, str(dest)]
    if branch:
        cmd[2:2] = ["--branch", branch]
    sp.run(cmd)


def init_repo(dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    if not (dest / ".git").exists():
        sp.run(["git", "init", str(dest)])


def current_branch(repo_path: Path) -> str | None:
    if not (repo_path / ".git").exists():
        return None
    result = sp.run(
        ["git", "-C", str(repo_path), "branch", "--show-current"],
        check=False,
    )
    branch = (result.stdout or "").strip()
    return branch or None

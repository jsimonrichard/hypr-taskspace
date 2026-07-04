"""Safe subprocess helpers."""

from __future__ import annotations

import json
import shutil
import subprocess
from typing import Any


class CommandError(RuntimeError):
    def __init__(self, cmd: list[str], returncode: int, stderr: str):
        self.cmd = cmd
        self.returncode = returncode
        self.stderr = stderr
        super().__init__(f"Command failed ({returncode}): {' '.join(cmd)}\n{stderr}")


def which(name: str) -> str | None:
    return shutil.which(name)


def run(
    cmd: list[str],
    *,
    check: bool = True,
    capture: bool = True,
    text: bool = True,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        cmd,
        check=False,
        capture_output=capture,
        text=text,
        env=env,
    )
    if check and result.returncode != 0:
        raise CommandError(cmd, result.returncode, result.stderr or "")
    return result


def run_json(cmd: list[str], *, check: bool = True) -> Any:
    result = run(cmd, check=check)
    if not result.stdout.strip():
        return None
    return json.loads(result.stdout)

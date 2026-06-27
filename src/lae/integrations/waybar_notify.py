"""Notify Waybar to re-run lae custom module exec hooks immediately."""

from __future__ import annotations

from lae.util import subprocess as sp

# Omarchy uses 7–10; lae modules share 11 (SIGRTMIN+11).
WAYBAR_SIGNAL = 11


def notify_waybar() -> None:
    if not sp.which("waybar"):
        return
    sp.run(["pkill", f"-RTMIN+{WAYBAR_SIGNAL}", "waybar"], check=False)

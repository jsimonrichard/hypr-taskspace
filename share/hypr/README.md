# Hyprland integration

Shipped templates install to `@TSK_SHARE@/hypr/` via the **pacman package**, **`scripts/install-user-share.sh`**, or **`scripts/dev.sh`**.

Default binds (see `bindings.conf`):

| Action | Binding |
|--------|---------|
| Host / default taskspace | `SUPER+H` |
| Workspace 1–10 | `SUPER+1..9`, `SUPER+0` |
| Move window to workspace 1–10 | `SUPER+Shift+1..9 / 0` |
| Prev / next workspace | `SUPER+[` / `SUPER+]`, trackpad swipe |
| Task manager | `SUPER+Tab` |
| Task-aware terminal | `SUPER+Return` |
| Editor / browser | `SUPER+E` / `SUPER+B` |

## Manual prod install

1. Install share assets (`makepkg -si` or `scripts/install-user-share.sh`).
2. Add **as the last line** of `~/.config/hypr/hyprland.conf`:

   ```ini
   source = ~/.local/share/tsk/hypr/bindings.conf
   # pacman: source = /usr/share/tsk/hypr/bindings.conf
   ```

   Or use **`tsk install omarchy`** to do this automatically (Omarchy only).

3. Resolve keybind conflicts your way:
   - **Omarchy**: `tsk install omarchy` comments out native `gesture = …, workspace` lines in `~/.config/hypr/input.conf` and sources tsk swipe gestures (`tsk workspace prev/next`) from `bindings.conf`. Omarchy unbinds for keybinds are applied automatically.
   - **Emergency terminal**: `SUPER+Ctrl+Return` opens a plain `xdg-terminal-exec` shell (no tsk) via `integrations/omarchy-escape-hatch.conf` — use when tsk or tskd is broken.

Because Hyprland uses the **last** matching bind, sourcing `bindings.conf` last overrides earlier workspace keys.

For the daemon, use **`scripts/install-systemd.sh`** (see [docs/install.md](../../docs/install.md)).

## Dev install

Use `scripts/dev.sh enter` — it installs to `~/.local/share/tsk-dev/`, applies Omarchy unbinds automatically, patches Waybar, and **does not** install the systemd unit. The foreground daemon is started by `enter`; or run `scripts/dev.sh daemon` alone.

After code changes, rebuild dev share assets with `scripts/dev.sh install share`. Full details: [docs/dev.md](../../docs/dev.md).

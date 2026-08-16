# Hyprland integration

Shipped templates install to `@TSK_SHARE@/hypr/` via the **pacman package**, **`scripts/install-user-share.sh`**, or **`scripts/dev.sh`**.

Default binds (see `omarchy.lua` on Omarchy, `bindings.conf` otherwise):

| Action | Binding |
|--------|---------|
| Host / default taskspace | `SUPER+H` |
| Workspace 1–10 | `SUPER+1..9`, `SUPER+0` |
| Move window to workspace 1–10 | `SUPER+Shift+1..9 / 0` |
| Prev / next workspace | `SUPER+[` / `SUPER+]`, trackpad swipe |
| Task manager | `SUPER+Tab` |
| Task-aware terminal | `SUPER+Return` |
| Editor / browser | `SUPER+E` / `SUPER+B` |
| Chromium (`tsk launch`) | `SUPER+Shift+B`, `SUPER+Shift+Return` |
| Chromium private | `SUPER+Shift+Alt+B` |

## Omarchy (Quattro)

`tsk install omarchy` appends a marked block to `~/.config/hypr/bindings.lua` (loaded last by `hyprland.lua`):

```lua
-- tsk-managed begin
dofile("/usr/share/tsk/hypr/omarchy.lua")
hl.unbind("SUPER + TAB")
o.bind("SUPER + TAB", "Task manager", "/usr/bin/tsk task tui-launch")
-- tsk-managed end
```

It does **not** edit `hyprland.conf`. After edits: `hyprctl reload` then `hyprctl configerrors`.

`omarchy.lua` unbinds Omarchy workspace digits, SUPER+Tab, SUPER+Return, mouse scroll, and browser keys, then binds tsk commands. Browser keys call `tsk launch chromium.desktop` (never `omarchy-launch-browser`). **SUPER+Space** Apps go through a cloned `omarchy.menu` whose launch prefix is also `tsk launch`. Menu updates may require re-applying that patch.

The bar is the `tsk.taskspace` plugin. **SUPER+Tab** and the task label run `tsk task tui-launch`, which opens the overlay after `tsk install omarchy` or the floating TUI after `tsk install omarchy --tui`. Waybar CFFI remains available for non-Omarchy Hyprland.

## Manual prod install (legacy `.conf`)

1. Install share assets (`makepkg -si` or `scripts/install-user-share.sh`).
2. Add **as the last line** of `~/.config/hypr/hyprland.conf`:

   ```ini
   source = ~/.local/share/tsk/hypr/bindings.conf
   # pacman: source = /usr/share/tsk/hypr/bindings.conf
   ```

   Or use **`tsk install omarchy`** on Omarchy (Lua path above).

3. Resolve keybind conflicts your way:
   - **Omarchy**: unbinds live in `omarchy.lua`. Native workspace swipe gestures in `input.lua` should stay commented so tsk 3-finger swipes apply.
   - **Emergency terminal**: on `.conf` installs, `SUPER+Ctrl+Return` opens a plain `xdg-terminal-exec` shell via `integrations/omarchy-escape-hatch.conf`.

Because Hyprland uses the **last** matching bind, sourcing `bindings.conf` last (or `dofile` last in `bindings.lua`) overrides earlier workspace keys.

For the daemon, use **`scripts/install-systemd.sh`** (see [docs/install.md](../../docs/install.md)).

## Dev install

Use `scripts/dev.sh enter` — it installs to `~/.local/share/tsk-dev/`, applies Lua (or `.conf`) bindings, copies the Omarchy plugin, and **does not** install the systemd unit. The foreground daemon is started by `enter`; or run `scripts/dev.sh daemon` alone.

After code changes, rebuild dev share assets with `scripts/dev.sh install share`. Full details: [docs/dev.md](../../docs/dev.md).

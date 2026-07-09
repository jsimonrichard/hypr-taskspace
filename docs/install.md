# Install (production)

Production setup is **manual** for Hyprland and Waybar integration because keybinds depend on your existing config — except on **Omarchy**, where an automated preset is available.

Static files (templates, Waybar `.so`, systemd unit) come from **your package manager** or **repo scripts** — not from `tsk install` (except `install omarchy`, which patches your Hypr/Waybar configs).

## Arch Linux (pacman)

```bash
cd packaging/arch && makepkg -si
systemctl --user enable --now tskd.service
```

Templates live under `/usr/share/tsk/`; runtime data (`state.db`, `daemon.sock`) stays in `~/.local/share/tsk/`. Wire Hyprland and Waybar yourself.

See **[docs/packaging.md](packaging.md)** for paths and manual integration steps.

---

## Omarchy (automated prod install)

If you use Omarchy and its default Hyprland/Waybar layout:

```bash
# Ensure share assets exist first (pacman, or scripts/install-user-share.sh)
tsk install omarchy
scripts/install-systemd.sh
tsk doctor
```

This patches Hyprland and Waybar (source line, Omarchy unbinds, `cffi/tsk` module, styles, restart) with config backups for rollback. Share assets must already be installed (pacman or `scripts/install-user-share.sh`).

Dry-run: `tsk install omarchy --dry-run`

---

## Manual install (non-Omarchy or custom keybinds)

### 1. Install the CLI

```bash
cargo install --path crates/tsk-cli
# or: cd packaging/arch && makepkg -si
```

### 2. Install share assets

**Pacman:** skip this step (files are in `/usr/share/tsk/`).

**Cargo / from source:**

```bash
scripts/install-user-share.sh
```

This copies `share/` templates to `~/.local/share/tsk/`, builds `libtsk_waybar.so`, and reloads Hyprland/Waybar.

| Runtime data | `~/.local/share/tsk/` (`state.db`, `daemon.sock`) |
| CLI | on `PATH` (`~/.cargo/bin/tsk` or `/usr/bin/tsk`) |
| Waybar module | `~/.local/share/tsk/lib/` (cargo) or `/usr/share/tsk/lib/` (pacman) |
| Hypr templates | same share tree as the module |

### Waybar module (`.so`)

Waybar loads the module from a path under the share tree (see `share/waybar/cffi-module.jsonc`). The install script or package places `libtsk_waybar.so` there.

## 3. Hyprland integration

Add **as the last line** of `~/.config/hypr/hyprland.conf`:

```ini
# cargo / user-local install:
source = ~/.local/share/tsk/hypr/bindings.conf
# pacman:
# source = /usr/share/tsk/hypr/bindings.conf
```

Resolve keybind conflicts your way. Omarchy users may source `…/hypr/integrations/omarchy-unbind.conf` before `bindings.conf`.

Run `hyprctl reload` after editing.

## 4. Waybar integration

Merge the CFFI snippet from your share tree (`waybar/cffi-module.jsonc`) into `~/.config/waybar/config.jsonc`. Append `waybar/tsk-style.css` to your Waybar `style.css`.

## 5. Daemon (systemd)

```bash
scripts/install-systemd.sh
```

Pacman installs the unit to `/usr/lib/systemd/user/tskd.service`; the script enables it. Cargo users get a copy in `~/.config/systemd/user/`.

Manage with `systemctl --user status tskd.service` or `tsk daemon start|stop|restart`.

## 6. Verify

```bash
tsk doctor
tsk integration status
tsk daemon status
```

## Update after pulling

```bash
scripts/install-user-share.sh          # cargo / source: refresh share + .so
tsk install omarchy                    # Omarchy: re-patch configs
# pacman: cd packaging/arch && makepkg -si
```

## Uninstall

Remove Hypr/Waybar integration manually, then:

```bash
systemctl --user disable --now tskd.service
# pacman: pacman -R hypr-taskspace
# optional: rm -rf ~/.local/share/tsk/   # removes state.db
```

For automated rollback (dev install only), see [dev.md](dev.md).

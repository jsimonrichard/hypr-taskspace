# Install (production)

Production setup is **manual** for Hyprland and Waybar integration because keybinds depend on your existing config — except on **Omarchy**, where an automated preset is available.

Static files (templates, Waybar `.so`, systemd unit) come from **your package manager** or **repo scripts** — not from `tsk install` (except the `install` subcommands, which patch user configs).

## Arch Linux (pacman)

```bash
cd packaging/arch && makepkg -si
systemctl --user enable --now tskd.service
```

Templates live under `/usr/share/tsk/`; runtime data (`state.db`, `daemon.sock`) stays in `~/.local/share/tsk/`. Wire Hyprland and Waybar yourself.

See **[packaging.md](packaging.md)** for paths and manual integration steps.

---

## Detected integrations (`tsk install all`)

```bash
tsk install all
tsk install all --dry-run
```

This looks at the machine and runs the matching installers:

| Detected | Command run |
|---|---|
| `~/.local/share/omarchy` | `tsk install omarchy` (Hypr, Waybar, Walker, systemd) |
| Chromium on PATH or `~/.config/chromium` | `tsk install chromium` |
| `~/.config/elephant/elephant.toml` (and no Omarchy) | `tsk install walker` |

Omarchy already installs Walker, so Elephant is not patched twice.

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

## Chromium helper extension

Orthogonal to Omarchy. Registers a packed helper extension and a native-messaging host in the **host** Chromium profile (`~/.config/chromium`):

```bash
tsk install chromium
```

Restart Chromium afterwards (fully quit first). The extension is loaded via `External Extensions/<id>.json` (not by writing into `Default/Extensions/`).

The CRX version comes from the workspace package version in `Cargo.toml`. Bump that for a release — do not edit `share/chromium/extension/manifest.json`. From-source installs append a revision (`0.1.0.3`, …) so `tsk install chromium` updates Chromium without a version bump.

Test the helper without archiving a task:

```bash
tsk chromium status      # live tabs + per-task snapshot
# close the Chromium window, then open Chromium from Walker
# or: tsk task browser / tsk chromium restore
```

The helper writes `~/tsk-tasks/<id>/.tsk/browser-session.json` automatically as tabs change. Archiving freezes that snapshot; restoring a task leaves it pending. The first Chromium launch in that taskspace (Walker or `tsk task browser`) reopens the tabs.

`status` is the first thing to check: if `live-windows.json` is missing, the extension is not reaching the native host.

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

| | Path |
|---|------|
| Runtime data | `~/.local/share/tsk/` (`state.db`, `daemon.sock`) |
| CLI | on `PATH` (`/usr/bin/tsk` when packaged — it must be first if a cargo copy is also installed) |
| Waybar module | `~/.local/share/tsk/lib/` (cargo) or `/usr/share/tsk/lib/` (pacman) |
| Hypr templates | same share tree as the module |

### Waybar module (`.so`)

Waybar loads the module from a path under the share tree (see `share/waybar/cffi-module.jsonc`). The install script or package places `libtsk_waybar.so` there.

### 3. Hyprland integration

`bindings.conf` defines the default Hyprland keybinds documented in the [README](../README.md#keybindings-hyprland) (**SUPER+Tab**, **SUPER+H**, workspace digits, etc.). `tsk install omarchy` sources this file for you; on other setups you add it yourself and remap as needed.

Add **as the last line** of `~/.config/hypr/hyprland.conf`:

```ini
# cargo / user-local install:
source = ~/.local/share/tsk/hypr/bindings.conf
# pacman:
# source = /usr/share/tsk/hypr/bindings.conf
```

Resolve keybind conflicts your way. The shipped `bindings.conf` sources `hypr/integrations/omarchy-unbind.conf` before the tsk binds.

Run `hyprctl reload` after editing.

### 4. Waybar integration

Merge the CFFI snippet from your share tree (`waybar/cffi-module.jsonc`) into `~/.config/waybar/config.jsonc`. Append `waybar/tsk-style.css` to your Waybar `style.css`.

### 5. Daemon (systemd)

```bash
scripts/install-systemd.sh
```

Pacman installs the unit to `/usr/lib/systemd/user/tskd.service`; the script enables it. Cargo users get a copy in `~/.config/systemd/user/`.

Manage with `systemctl --user status tskd.service` or `tsk daemon start|stop|restart`.

### 6. Verify

```bash
tsk doctor
tsk integration status
tsk daemon status
```

## Suggested config

On first run, `tsk` creates `~/.config/tsk/config.toml`. For pacman installs, set share paths explicitly (or copy `/usr/share/tsk/config.toml.example`):

```toml
[data]
dir = "~/.local/share/tsk"

[install.hypr]
share_dir = "/usr/share/tsk"
source_line = "/usr/share/tsk/hypr/bindings.conf"
```

## Update after pulling

```bash
# pacman
cd packaging/arch && makepkg -si
systemctl --user restart tskd.service
# If Hyprland still reports missing tsk source files: hyprctl reload

# cargo / source
scripts/install-user-share.sh          # refresh share + .so
# tsk install omarchy                  # only to re-patch Hypr/Waybar user configs
systemctl --user restart tskd.service
```

## Migrating from a legacy cargo install

If you previously copied everything into `~/.local/share/tsk/` and switch to pacman:

```bash
scripts/cleanup-legacy-install.sh    # removes duplicate templates; keeps state.db
```

## Uninstall

Remove Hypr/Waybar integration manually, then:

```bash
systemctl --user disable --now tskd.service
# pacman: pacman -R hypr-taskspace
# optional: rm -rf ~/.local/share/tsk/   # removes state.db
```

For automated rollback of **dev** integration only, see [dev.md](dev.md).

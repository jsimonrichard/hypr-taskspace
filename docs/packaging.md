# Arch Linux packaging

Build and install from the repo:

```bash
cd packaging/arch
makepkg -si
```

This installs:

| Path | Purpose |
|------|---------|
| `/usr/bin/tsk` | CLI |
| `/usr/share/tsk/hypr/` | Hyprland Lua + `.conf` bindings + window rules |
| `/usr/share/tsk/omarchy-plugin/` | Omarchy bar-widget + overlay (`tsk.taskspace`) |
| `/usr/share/tsk/waybar/` | Waybar CFFI snippet + styles |
| `/usr/share/tsk/chromium/` | Helper extension source (packed by `tsk install chromium`) |
| `/usr/share/tsk/bin/tsk-chromium-host` | Native-messaging wrapper |
| `/usr/share/tsk/bin/xdg-open` | Taskspace `xdg-open` wrapper (installed into `~/.local/share/tsk/task-bin`) |
| `/usr/share/tsk/bin/tsk-open` | URL opener for `$BROWSER` / Cursor `workbench.externalBrowser` |
| `/usr/share/tsk/lib/libtsk_waybar.so` | Waybar module |
| `/usr/lib/systemd/user/tskd.service` | User daemon unit |
| `/usr/share/tsk/config.toml.example` | Suggested user config |
| `/usr/share/libalpm/hooks/90-hypr-taskspace-reload.hook` | Reloads Hyprland, restarts `tskd`, and refreshes `~/.config/omarchy/plugins/tsk.taskspace` after share files or `/usr/bin/tsk` are replaced |

Runtime data always lives under **`~/.local/share/tsk/`** (`state.db`, `daemon.sock`). The package does not write there.

## Manual integration (no `tsk install omarchy`)

### Omarchy (Quattro)

Prefer `tsk install omarchy` (or `tsk install omarchy --tui`). It `dofile`s `/usr/share/tsk/hypr/omarchy.lua` from `~/.config/hypr/bindings.lua` and copies the bar plugin. Do not edit `hyprland.conf`.

### Hyprland (legacy `.conf`)

Add as the **last** line of `~/.config/hypr/hyprland.conf`:

```ini
source = /usr/share/tsk/hypr/bindings.conf
```

Resolve keybind conflicts with your existing config. Omarchy users may source `/usr/share/tsk/hypr/integrations/omarchy-unbind.conf` first.

### Waybar

Merge `/usr/share/tsk/waybar/cffi-module.jsonc` into `~/.config/waybar/config.jsonc` — replace `hyprland/workspaces` with `cffi/tsk` in `modules-left`.

Append `/usr/share/tsk/waybar/tsk-style.css` to your Waybar `style.css`.

### Config

On first run, `tsk` creates `~/.config/tsk/config.toml`. For pacman installs, set:

```toml
[data]
dir = "~/.local/share/tsk"

[install.hypr]
share_dir = "/usr/share/tsk"
source_line = "/usr/share/tsk/hypr/bindings.conf"
config_path = "~/.config/hypr/bindings.lua"
```

Or copy `/usr/share/tsk/config.toml.example` to `~/.config/tsk/config.toml`.

### Daemon

```bash
systemctl --user enable --now tskd.service
```

The packaged unit uses `ExecStart=/usr/bin/tsk daemon run`. You can also enable it with `scripts/install-systemd.sh`.

### Verify

```bash
tsk doctor
```

## Updating

```bash
cd packaging/arch && makepkg -si
systemctl --user restart tskd.service
# restart Waybar after package updates the .so
```

Pacman replaces `/usr/share/tsk` with remove-then-add. Hyprland sources those files directly and can auto-reload while they are missing. A PostTransaction hook runs `hyprctl reload`, restarts `tskd`, and (when already installed) copies `/usr/share/tsk/omarchy-plugin/` into `~/.config/omarchy/plugins/tsk.taskspace/` then `omarchy-shell shell rescanPlugins`. If you still see “could not find file” source errors, `hyprctl reload` is enough. You do not need `tsk install omarchy` for plugin code updates after the first install — only when changing user config integration (Lua bindings, `--tui` vs overlay, menu launch prefix).

## Cargo install (non-pacman)

Users without the package still run:

```bash
cargo install --path crates/tsk-cli
scripts/install-user-share.sh
```

That copies templates to `~/.local/share/tsk/` instead of `/usr/share/tsk`. See [install.md](install.md).

## AUR publish

One PKGBUILD covers in-tree `makepkg -si` (empty `source`, builds this checkout) and the AUR (GitHub tag tarball). Helpers (`install-share.sh`, hooks) stay in the tarball, so the AUR git repo only needs `PKGBUILD`, `.SRCINFO`, and `hypr-taskspace.install`.

1. Match `pkgver` to `[workspace.package] version` in `Cargo.toml`.
2. Tag and push `v$pkgver` (`git tag v0.1.1 && git push origin v0.1.1`).
3. From a machine with `makepkg`:

```bash
packaging/arch/publish-aur.sh
# first time:
git clone ssh://aur@aur.archlinux.org/hypr-taskspace.git /tmp/aur-hypr-taskspace
packaging/arch/publish-aur.sh --sync /tmp/aur-hypr-taskspace --push
```

You need an [AUR account](https://aur.archlinux.org) with an SSH key. The script fills `_aur_sha256`, writes `.SRCINFO`, and copies those three files into the AUR clone. It does not push unless you pass `--push`.

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
| `/usr/share/tsk/hypr/` | Hyprland bindings + window rules |
| `/usr/share/tsk/waybar/` | Waybar CFFI snippet + styles |
| `/usr/share/tsk/lib/libtsk_waybar.so` | Waybar module |
| `/usr/lib/systemd/user/tskd.service` | User daemon unit |
| `/usr/share/tsk/config.toml.example` | Suggested user config |

Runtime data always lives under **`~/.local/share/tsk/`** (`state.db`, `daemon.sock`). The package does not write there.

## Manual integration (no `tsk install omarchy`)

### Hyprland

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
```

Or copy `/usr/share/tsk/config.toml.example` to `~/.config/tsk/config.toml`.

### Daemon

```bash
systemctl --user enable --now tskd.service
```

The packaged unit uses `ExecStart=/usr/bin/tsk daemon run`. Enable it with `systemctl --user enable --now tskd.service` or `scripts/install-systemd.sh`.

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

## Cargo install (non-pacman)

Users without the package still run:

```bash
cargo install --path crates/tsk-cli
scripts/install-user-share.sh
```

That copies templates to `~/.local/share/tsk/` instead of `/usr/share/tsk`.

## AUR publish

1. Tag a release and set `source` + `sha512sums` in `PKGBUILD`.
2. Copy `packaging/arch/PKGBUILD` to the AUR repo.
3. Run `makepkg --printsrcinfo > .SRCINFO`.

See also [docs/install.md](../install.md) for integration details.

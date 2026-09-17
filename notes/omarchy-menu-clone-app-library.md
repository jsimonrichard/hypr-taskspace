# Cloned omarchy.menu app list (2026-09-17)

## Status

Implemented. Super+Space Apps were empty after Omarchy 4.0.4 because the cloned
menu's `open()` runs with `root.shell === null`.

## What broke

Omarchy now wraps third-party plugins in `PluginShellApi` instead of handing them
the host shell. `jsimonrichard.menu` (cloned from `omarchy.menu` by
`tsk install omarchy`) is third-party. On this machine the scoped shell object is
never assigned, so `root.appLibrary` is null. Stock `mergeAppRows()` returns
immediately: the Apps submenu opens as "Nothing here yet".

First-party `omarchy.menu` is disabled while the clone is active;
`omarchy-menu toggle` still resolves to the clone.

Not done: fixing Omarchy's `item.shell = pluginShellFor(...)` injection (lives in
`/usr/share/omarchy/shell/shell.qml`). The clone workaround uses Quickshell
`DesktopEntries`, the same singleton `AppLibrary.qml` reads.

## What tsk does

`install_menu_launch_prefix` refreshes `Menu.qml` from packaged `omarchy.menu`,
then patches:

1. `mergeAppRows` / icon lookup (`tsk-managed-apps`)
2. app activate → `tsk launch` (`tsk-managed-launch`)

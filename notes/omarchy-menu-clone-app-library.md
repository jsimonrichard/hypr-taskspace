# Cloned omarchy.menu app list (2026-09-17)

## Status

**Superseded 2026-09-17:** the QML patch from `stkxrotn` is still required, but
it does not take effect until omarchy-shell is fully restarted. `rescanPlugins`
and inotify reload leave the keepLoaded clone on the pre-patch `Menu.qml`.
`tsk install omarchy` now runs `omarchy restart shell` after writing the clone
(including the already-patched path, so a second `tsk install all` recovers a
stale running shell).

Live diagnosis on this machine after installing AUR 0.2.0: on-disk `Menu.qml`
had `tsk-managed-apps`, `root.shell` was a `PluginShellApi` with
`appLibrary === null`, and `DesktopEntries.applications.values.length` was 92.
The Apps list stayed empty because the running keepLoaded instance never loaded
that file.

## What broke

Omarchy wraps third-party plugins in `PluginShellApi` instead of handing them
the host shell. `jsimonrichard.menu` (cloned from `omarchy.menu` by
`tsk install omarchy`) is third-party. `root.appLibrary` is null even when
`root.shell` is assigned (`PluginShellApi.appLibrary` is not populated for this
clone). Stock `mergeAppRows()` returns immediately: the Apps submenu opens as
"Nothing here yet".

First-party `omarchy.menu` is disabled while the clone is active;
`omarchy-menu toggle` still resolves to the clone.

The AUR PostTransaction hook does **not** run `tsk install all`. It copies
`tsk.taskspace` and restarts the shell. Menu-clone patches only happen in
`tsk install omarchy`.

Not done: fixing Omarchy's `PluginShellApi.appLibrary` injection (lives in
`/usr/share/omarchy/shell/shell.qml`). The clone workaround uses Quickshell
`DesktopEntries`, the same singleton `AppLibrary.qml` reads. Also not done:
making the package hook run `tsk install omarchy` as the logged-in user.

## What tsk does

`install_menu_launch_prefix` refreshes `Menu.qml` from packaged `omarchy.menu`,
then patches:

1. `mergeAppRows` / icon lookup (`tsk-managed-apps`)
2. app activate → `tsk launch` (`tsk-managed-launch`)
3. `omarchy restart shell` so the keepLoaded clone actually loads that file

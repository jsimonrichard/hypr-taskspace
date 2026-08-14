# Chromium helper extension

Installed by `tsk install chromium` (or `tsk install all` when Chromium is detected). Orthogonal to Omarchy.

On Linux, Chromium loads **external** extensions from:

- user: `~/.config/chromium/External Extensions/<id>.json`
- system (pacman, later): `/usr/share/chromium/extensions/<id>.json`

Do not copy files into `Default/Extensions/` — Chromium owns that tree.

The native messaging host is `~/.config/chromium/NativeMessagingHosts/org.tsk.browser.json`, pointing at `tsk-chromium-host` in the share tree.

## Versioning (dev vs release)

`manifest.json` `version` is a `0.0.0` placeholder. Do not edit it.
`tsk install chromium` stamps the version from `[workspace.package]` in
`Cargo.toml`.

| Situation | What to bump | What Chromium sees |
|-----------|--------------|--------------------|
| Extension JS/source while developing | nothing — run `tsk install chromium` | `0.1.0.N` (`N` auto-increments) |
| Releasing the project | `[workspace.package] version` only | exact `0.1.0` (packaged `/usr/share/tsk`) |

Fully quit Chromium after install (the helper snapshots on tab changes or
about once a minute). Do not Remove the extension in `chrome://extensions`
(that blocklists the id). To drop a high `0.1.0.N` back to a lower version,
quit Chromium, delete `~/.config/chromium/External Extensions/<id>.json`,
start Chromium once and quit, delete `~/.local/share/tsk/chromium/dev-revision`,
then reinstall. Agents: `.cursor/rules/chromium-extension-version.mdc`.

## Test without archive/restore

1. Rebuild `tsk` and run `tsk install chromium` (from-source installs bump a
   revision so Chromium picks up JS changes).
2. Fully quit Chromium, then open it on a task workspace and load a tab.
3. `tsk chromium status` — `~/.local/share/tsk/chromium/live-windows.json`
   should list that tab. If it says missing, the extension is not talking to
   the native host yet.
4. `tsk chromium status` should show a **Saved session** for the current task
   (written automatically from the live snapshot). `tsk chromium snapshot`
   forces the same write.
5. Close that Chromium window, then open Chromium from Walker (or
   `tsk task browser`). The first launch reopens the saved tabs.

Archiving freezes the last snapshot and closes those windows. Restoring the
task does **not** open Chromium. The first Walker / `tsk task browser` launch
in that taskspace reopens the saved URLs.

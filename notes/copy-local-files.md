# Copy-local files via `.tsk/repo.toml`

**Status (2026-09-21):** Implemented in `tsk-core`. Opt-in `copy_local` list;
default behavior unchanged (no copies).

---

## Goal

When creating a linked task checkout or a sibling checkout, copy an explicit
list of repo-relative paths (typically gitignored files like `.env`) from a
local working tree into the new checkout, driven by `.tsk/repo.toml` — not by
gitignore discovery and not by a silent default.

## Principles

- **Opt-in only.** Empty / unset `copy_local` means no copies. Never invent a
  default that includes `.env`.
- **Config stays on the registered root.** Load the list from `source_root`'s
  `.tsk/repo.toml` (same place hooks / `[browser]` already live).
- **Copy source follows the fork WC when one exists:**
  - `ForkFrom::Current` / `Workspace` / `checkout add` (default current) →
    live fork checkout path
  - `ForkFrom::Default` / `Revision` → registered `source_root`
- **Fail closed on bad paths.** Absolute paths, empty entries, and `..` /
  `.` components → error naming the path. Missing source files → skip.
  Destination already present → skip (do not clobber).
- **Logic in `tsk-core`.** One helper used by both `task new` and
  `checkout add`. Seed **before** `on_create`.

## `.env` safety

Same-user local copy between checkouts on one machine is the intended use and
is reasonable. The list stays explicit so secrets are never auto-propagated.
Do not commit these files; treat shared/CI machines as higher risk.

## Reuse survey

Extending:

- `RepoConfig` / `load_repo_config` in `crates/tsk-core/src/repos.rs`
- `ForkFrom` in `crates/tsk-core/src/task_repo.rs` (`resolve_local_files_root`)
- `TaskService::create_task` / `add_sibling_checkout`
- Docs in `docs/cursor.md` + commented `.tsk/repo.toml`

Not extending: gitignore parsing, archive restore, TUI.

## Numbered work sections

1. Schema — `copy_local: Vec<String>` on `RepoConfig`
2. Helper — `copy_local_files` / `seed_copy_local`
3. Wire — after `create_linked_checkout` in task new + sibling add
4. Tests + docs

## Out of scope

- Globs / gitignore-driven discovery
- Overwriting existing dest paths
- Copying on archive restore
- Remote/clone flows
- Auto-including `.env` without config

## Success criteria

1. With `copy_local = [".env"]`, default `tsk task new` copies from
   `source_root` when present.
2. `--from-current` / `--from-workspace` / `checkout add` copy from the live
   fork checkout.
3. Missing listed paths skipped; absolute/`..` entries error.
4. Existing dest paths left untouched.
5. Unset `copy_local` preserves today's behavior.

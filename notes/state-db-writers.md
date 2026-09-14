# Session DB: schema owner, incremental saves

## Goal

Stop `database is locked` on taskspace switch, then stop rewriting the whole
`state.db` on every persist. First slice: only the daemon (and tests) run DDL.

## Limbo / Turso

[Limbo](https://turso.tech/blog/introducing-limbo-a-complete-rewrite-of-sqlite-in-rust)
(now Turso) is a Rust rewrite that keeps SQLite’s **language and file format**.
That format is still one writer per database file. It does not give us safe
multi-process writers, and it does not replace `save_state`’s delete-all +
reinsert. Switching engines would not fix this race and is out of scope.

The comment on `daemon/mod.rs` already claims a single writer. Make that true
for DDL now; make writes incremental later. WAL + `busy_timeout` can wait if
readers still collide with a writer — they are not a substitute for fewer
writers.

## Principles

- Fail closed: a missing schema is `TskError::SchemaMissing { path }`, not an
  implicit `CREATE TABLE` on a bar poll or SUPER+N.
- One constructor opens the file; `ensure_schema` is a required, explicit call
  for schema owners (daemon start, `tsk install`, tests). `tsk doctor` checks
  the same columns without writing. Hosts do not branch on “which integration”
  — every install entry that sets up the machine calls the same helper.
- `save_state` rewriting every table is a separate concern (section 3).

## 1. Schema only on daemon start (landed)

`Registry::new` no longer runs `init_db`. `Registry::ensure_schema` is the only
DDL entry. `TaskService::initialize` (daemon-only today) calls it first.
`test_service` and registry tests call `ensure_schema` on empty temp DBs.

Bar status, chromium-host, workspace remember, and other CLI `with_defaults`
paths become open + read/write without `CREATE TABLE IF NOT EXISTS`.

## 1b. Install and doctor (this change)

`install::ensure_session_schema` is the install-side owner. Every user-facing
install entry (`install_detected`, `install_bins`, walker, chromium) calls that
helper. Same `Registry::ensure_schema` — not a second migration. Dry-run
reports the path and does not open the DB.

`Registry::check_schema` is read-only: missing file or missing required columns
is `SchemaMissing`. `tsk doctor` reports it. Doctor must not create `state.db`.

## 2. Later: WAL + busy_timeout

If a reader still hits a writer, wait instead of `SQLITE_BUSY` at timeout 0.
Does not require Limbo.

## 3. Later: incremental persist

Replace wholesale `DELETE FROM tasks/windows` + reinsert with per-row updates
(or a single transaction around a narrower write). Needed before concurrent
reads of a half-rewritten snapshot are safe. See `notes/daemon-lock-contention.md`
Tier 5.

## Out of scope

- Adopting Limbo/Turso/libSQL
- Bar `workspacev2` polling (separate)
- Changing `save_state` in this changeset

## Success criteria

1. `Registry::new` does not execute schema DDL.
2. A fresh temp DB cannot `load_state` until `ensure_schema`.
3. Daemon `initialize` still creates/migrates the prod DB.
4. Existing registry/service tests pass.
5. User-facing install entries call `ensure_session_schema` (skipped on
   dry-run).
6. `check_schema` fails closed on a missing file without creating one.
7. `tsk doctor` includes a session-schema check.

## Reuse survey

Extending `Registry::ensure_schema` / `init_db` / `migrate_schema`. Install
calls the same method via `install::ensure_session_schema` from the public
install entries — not a per-integration migration.
Doctor uses `Registry::check_schema` (read-only), not a second column list
scattered in QML or the CLI.

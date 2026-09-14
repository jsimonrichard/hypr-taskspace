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
  at process start for the process that owns migrations (daemon). Tests call
  the same method. Hosts do not branch on “which CLI command” — they either
  migrate or they do not.
- `save_state` rewriting every table is a separate concern (section 3).

## 1. Schema only on daemon start (this change)

`Registry::new` no longer runs `init_db`. `Registry::ensure_schema` is the only
DDL entry. `TaskService::initialize` (daemon-only today) calls it first.
`test_service` and registry tests call `ensure_schema` on empty temp DBs.

Bar status, chromium-host, workspace remember, and other CLI `with_defaults`
paths become open + read/write without `CREATE TABLE IF NOT EXISTS`.

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

## Reuse survey

Extending `crates/tsk-core/src/registry.rs` (`init_db` / `migrate_schema`) and
the existing daemon-only `TaskService::initialize`. Not a second migration
path. Tests already go through `Registry::new` on a temp file — they call
`ensure_schema` instead of implicit init.

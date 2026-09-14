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

## Embedded engines (if we outgrow SQLite WAL)

Recorded 2026-09-14. Concurrent **reads** are not a reason to leave SQLite.
[WAL mode](https://www.sqlite.org/wal.html) already allows many reader processes
plus one writer, with snapshot isolation. Our `database is locked` was writers
colliding (`init_db` + `save_state`) at `busy_timeout` 0, not missing readers.
Section 2 is still the first concurrency step.

Almost every established embedded store is still **one writer**. What changes is
whether readers block that writer.

| Engine | Concurrent reads | Multi-process | Notes |
|--------|------------------|---------------|--------|
| **SQLite WAL** (stay) | Yes, snapshot | Yes (same host; not network FS) | SQL, `rusqlite`, one file. Default rollback journal *does* block readers during a write. |
| **LMDB** (`heed`) | Yes, MVCC mmap | Yes — designed for it | Battle-tested KV (OpenLDAP et al.). No SQL; set map size; one writer. |
| **RocksDB** (`rust-rocksdb`) | Yes, snapshots | Possible, heavier | LSM, Meta-maintained. Directory of files, C++/cross-compile cost. Built for TB write load, not a 50KB session. |
| **redb** | Yes, MVCC (threads) | Experimental feature | Pure Rust, LMDB-inspired. Less established than SQLite/LMDB. |
| **DuckDB** | Yes (analytics) | Process-local | Wrong shape: OLAP engine, not a session registry. |

Not candidates: Limbo/libSQL (same SQLite writer model), sled (maintenance
stalled). Berkeley DB is concurrent and old; nobody new should pick it over
SQLite or LMDB.

If we ever switch, LMDB is the only “established embedded” that is *more*
multi-process-native than SQLite WAL. We would give up SQL, `tsk`’s existing
migrations, and `sqlite3` debugging for a KV we have to schema ourselves. Do
that only if WAL + incremental `save_state` still cannot keep bar/CLI reads
off the writer. A daemon-owned write socket (readers never open the file for
write) is the application-level version of the same idea and does not need a
new engine.

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

## 3. Incremental persist (this change)

`save_state` runs in one SQLite transaction: update the session row, upsert
tasks/windows that differ from the current rows, delete keys that left the
snapshot. Readers never see an empty `tasks`/`windows` table mid-save.

Still one `SessionState` snapshot from the service — not per-field SQL APIs
(that is the rest of Tier 5). WAL + `busy_timeout` stay section 2.

## Out of scope

- Adopting Limbo/Turso/libSQL
- Bar `workspacev2` polling (separate)
- WAL / `busy_timeout` (section 2)
- Per-operation SQL instead of load → mutate → save

## Success criteria

1. `Registry::new` does not execute schema DDL.
2. A fresh temp DB cannot `load_state` until `ensure_schema`.
3. Daemon `initialize` still creates/migrates the prod DB.
4. Existing registry/service tests pass.
5. User-facing install entries call `ensure_session_schema` (skipped on
   dry-run).
6. `check_schema` fails closed on a missing file without creating one.
7. `tsk doctor` includes a session-schema check.
8. `save_state` does not `DELETE FROM tasks/windows` then reinsert the table.
9. Removing one task or window leaves the others; a missing session row fails
   closed.

## Reuse survey

§1–1b: `Registry::ensure_schema` / `init_db` / `migrate_schema` and
`install::ensure_session_schema`.

§3: same `Registry::save_state` and the existing `upsert_task` helper.
`load_state` and the diff read share `load_tasks` / `load_windows`. Not a
second persist API on `TaskService`.

# AGENTS.md

Hypr Taskspace (`tsk`) is a task-centric Hyprland control plane. `crates/tsk-core`
is the shared library; `tsk-cli`, `tsk-tui` (ratatui), and `tsk-waybar` (CFFI) are
thin frontends over it. Runtime state lives in `~/.local/share/tsk/`; templates
under `/usr/share/tsk/` (pacman) or `~/.local/share/tsk/` (from source).

Daily use and layout: `README.md`. Dev setup: `docs/dev.md`. Packaging:
`docs/packaging.md`.

## House rules

These apply to **every** change, in every language. A narrower rule in
`.cursor/rules/` may override one of them; nothing else does. They exist because
each has come up repeatedly in review — following them up front saves a round.

### 1. No silent fallbacks

- If a state is impossible, **throw**. A fallback that hides a broken invariant
  is a defect even when it makes a test pass.
- If a permission, scope, or identity is unclear, **deny** — fail closed.
- A guard for a dangerous capability is a **required** argument, never an
  optional one with a safe-looking default.
- An error a user can hit must reach the **UI**, not only the log or console.
- Expected failures — validation, authorization, not-found, business rules — are
  not exceptions. Return them as data and display them.

Before writing `?? fallback`, an empty-collection return, or a `catch` that
swallows: decide whether the condition means an invariant broke. If it does,
throw instead.

### 2. Read upstream before writing a workaround

- Check what the framework, SDK, or platform already provides, and prefer its
  supported mechanism — **even at the cost of deleting local code that works**.
- If a workaround is genuinely needed, name the upstream mechanism you checked
  and why it did not work, in the commit body or PR description.
- Derive values from their authoritative source. A version, path, or expected
  value restated in a second place is a bug while it is still correct.
- Reach for a type guard before a cast, and for a documented API before
  coordinate math or DOM probing.
- A cross-cutting problem (release scripting, lockfiles, a Result type, a CI
  gate) may already be solved in a sibling repo. Port that solution rather than
  inventing a second one.

### 3. Generalize; do not special-case

- When a fix applies to one case, check whether the general rule holds for all
  cases and unify on **one path**. Two code paths where one would do is a defect
  even when both are correct.
- A host never branches on the identity of a plugin, extension, or backend. If
  it needs to know *which* one it is handling, the contract is missing a hook —
  add the optional hook so every implementation can opt in.
- After a general fix lands, **delete** the special case or fallback it
  replaced. Leaving both is the most common way this rule half-lands.

### 4. One concern per change

- One concern per commit and per PR. Never batch unrelated modules.
- Migrating a pattern across N modules is N changes, not one.
- Stop at each step of a multi-step feature and hand back for review rather than
  running the whole plan.
- A change that is narrower than asked but complete beats a wider one that needs
  unpicking. If scope has to grow, say so and stop.

### 5. Plan before building anything substantial

Write the plan to a file first, in this shape:

- **Goal** — one paragraph.
- **Principles** — the decisions that constrain everything else, including what
  backward compatibility (if any) is actually required.
- **Numbered work sections** — in shipping order, one concern each.
- **Out of scope** — explicit non-goals.
- **Success criteria** — numbered and observable.

Get the plan reviewed before writing code. Correcting a plan is cheaper than
correcting an implementation.

### 6. State what is not done

- Report gaps, deferrals, and residual assumptions with the same specificity as
  the work. Never let a summary imply coverage the change does not have.
- If a check was skipped, say it was skipped. If a test fails, quote the output.
- Establish which checks were already failing **before** you started. Report an
  inherited failure as inherited; do not silently fix it inside an unrelated
  change, and do not let it mask yours.
- When touching docs, verify each claim against current behavior rather than
  against the surrounding prose. Correct stale claims and date the
  reconciliation.
- A plan or note that reality has overtaken gets a status banner saying what
  superseded it — not a quiet edit.

### Required outputs

Two things are part of the deliverable, not optional extras.

1. **Reuse survey — before writing code.** Name the existing modules, tables,
   routes, and helpers that already touch this area, and say which you are
   extending. If you are adding rather than extending, say why the existing
   abstraction did not fit.
2. **Diff shape — at handoff.** Report files changed and lines added/removed,
   and say what the change let you delete. A diff that adds far more than it
   removes is a signal to re-check, not a sign of progress.

## Where these bite in this repo

- **Layer first.** `crates/tsk-core` holds models, config, registry, Hyprland
  and service logic. `tsk-cli`, `tsk-tui`, and `tsk-waybar` are thin frontends.
  A frontend never grows logic a second frontend would need.
- **Rule 1** — add a `TskError` variant that carries the path, id, or key rather
  than an `Other(String)` or a swallowed error. A crashing task application
  reports its message to the user (overlay/TUI), not just the daemon log.
- **Rule 2** — Hyprland is upstream: prefer `hyprctl` and socket2 events over
  polling or inferring state from a compositor-adjacent source that does not
  notify. Prefer XDG conventions over hardcoded paths.
- **Rule 2 (derive)** — do not bump
  `share/chromium/extension/manifest.json`; `tsk install chromium` stamps the
  version at pack time. See `.cursor/rules/chromium-extension-version.mdc`.
- **Rule 3** — one workspace/window resolution path for default and task
  taskspaces, not a branch per case.
- **Rule 6** — design and status notes go in `notes/`; user-facing docs in
  `docs/` and `README.md`. A superseded plan gets an implementation-status
  banner (see `notes/poc-plan.md`).

Gate before every push: `cargo fmt --all`, `cargo clippy --all-targets`,
`cargo test --workspace`, `cargo build --release`. There is no CI in this repo,
so these are the only gate — confirm the set is right before relying on it.

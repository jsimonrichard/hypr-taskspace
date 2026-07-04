#!/usr/bin/env bash
# Development / e2e entrypoint — installs to ~/.local/share/tsk-dev (separate from prod).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export TSK_WORKSPACE="$ROOT"

PROD_DATA="${XDG_DATA_HOME:-$HOME/.local/share}/tsk"
DEV_DATA="${XDG_DATA_HOME:-$HOME/.local/share}/tsk-dev"
PROD_DB="$PROD_DATA/state.db"
DEV_DB="$DEV_DATA/state.db"
DEV_BUILD="$ROOT/target/release/tsk"
SESSION_FILE="$PROD_DATA/dev-session"
HYPR_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/hypr/hyprland.conf"

PROD_TSKD_WAS_ACTIVE=false
TEARDOWN_DONE=false

stop_prod_tskd() {
  if systemctl --user is-active --quiet tskd.service 2>/dev/null; then
    PROD_TSKD_WAS_ACTIVE=true
    echo "Stopping prod tskd.service for dev session..."
    systemctl --user stop tskd.service
  fi
}

start_prod_tskd() {
  if $PROD_TSKD_WAS_ACTIVE; then
    echo "Restarting prod tskd.service..."
    systemctl --user start tskd.service 2>/dev/null || true
  fi
}

cmd="${1:-enter}"
shift || true

build_dev_binary() {
  echo "Building dev tsk + tsk-waybar (release)..."
  if [[ ! -x "$DEV_BUILD" ]] \
    || [[ -z "$(find "$ROOT/target/release/build" -path '*/libsqlite3-sys-*/out/libsqlite3.a' -print -quit 2>/dev/null)" ]]; then
    echo "Note: bundled SQLite compiles from C on cold builds — libsqlite3-sys(build) may sit ~1 min with no progress bar."
    echo "      Let it finish once; interrupting forces a full recompile on the next dev enter."
  fi
  cargo build -p tsk-cli -p tsk-waybar --release --target-dir "$ROOT/target"
  if [[ ! -x "$DEV_BUILD" ]]; then
    echo "Dev build missing: $DEV_BUILD" >&2
    exit 1
  fi
}

run_cli() {
  if [[ -x "$DEV_BUILD" ]]; then
    env TSK_WORKSPACE="$ROOT" "$DEV_BUILD" "$@"
  else
    cargo run -p tsk-cli --release -- "$@"
  fi
}

dev_integration_installed() {
  [[ -f "$HYPR_CONFIG" ]] && grep -q "tsk-dev-managed" "$HYPR_CONFIG"
}

# Dev uses prod task/session state so existing windows and tasks stay visible.
# Set TSK_DEV_ISOLATED=1 to keep a separate dev state.db (CI/e2e).
link_dev_state_db() {
  if [[ "${TSK_DEV_ISOLATED:-}" == "1" ]]; then
    echo "TSK_DEV_ISOLATED=1 — using separate dev state.db"
    return 0
  fi

  mkdir -p "$PROD_DATA" "$DEV_DATA"

  if [[ -e "$DEV_DB" && ! -L "$DEV_DB" ]]; then
    local backup="$DEV_DATA/state.db.local-$(date +%Y%m%d-%H%M%S).bak"
    echo "Backing up existing dev state.db → $backup"
    mv "$DEV_DB" "$backup"
  fi

  ln -sfn "$PROD_DB" "$DEV_DB"
  echo "Linked dev state.db → $PROD_DB"
}

start_dev_session() {
  mkdir -p "$PROD_DATA"
  echo "$DEV_BUILD" >"$SESSION_FILE"
  rm -f "$DEV_DATA/.session-active"
  echo "Dev session active → $DEV_BUILD"
  echo "Session file: $SESSION_FILE"
}

stop_dev_session() {
  rm -f "$SESSION_FILE" "$DEV_DATA/.session-active"
}

prepare_dev_session() {
  build_dev_binary
  start_dev_session
}

uninstall_dev_integration() {
  if [[ -x "$DEV_BUILD" ]]; then
    env TSK_WORKSPACE="$ROOT" "$DEV_BUILD" dev uninstall all
  else
    env TSK_WORKSPACE="$ROOT" cargo run -p tsk-cli --release -- dev uninstall all
  fi
}

# Full dev teardown — integration, session file, prod daemon.
teardown_dev_session() {
  if $TEARDOWN_DONE; then
    return 0
  fi
  TEARDOWN_DONE=true

  if dev_integration_installed || [[ -f "$SESSION_FILE" ]]; then
    echo "Leaving dev mode — restoring prod integration..."
    uninstall_dev_integration || echo "Warning: dev uninstall had errors (continuing cleanup)" >&2
  fi

  stop_dev_session
  rm -f "$DEV_DATA/bin/tsk"
  start_prod_tskd
}

maybe_teardown_stale_dev() {
  if [[ -f "$SESSION_FILE" || -f "$DEV_DATA/.session-active" ]]; then
    echo "Stale dev session file found (daemon not running) — cleaning up..."
    teardown_dev_session
    return 0
  fi
  if dev_integration_installed; then
    echo "Stale dev integration found (no active session) — cleaning up..."
    teardown_dev_session
  fi
}

start_dev_daemon() {
  if [[ ! -x "$DEV_BUILD" ]]; then
    echo "Dev tsk not found — run: $ROOT/scripts/dev.sh enter" >&2
    exit 1
  fi

  stop_prod_tskd

  cleanup_on_exit() {
    teardown_dev_session
  }
  trap cleanup_on_exit EXIT INT TERM

  echo "Starting dev daemon (Ctrl+C to exit and restore prod)..."
  "$DEV_BUILD" daemon run
}

case "$cmd" in
  enter)
    maybe_teardown_stale_dev
    link_dev_state_db
    prepare_dev_session
    stop_prod_tskd
    run_cli dev install all "$@"
    echo
    echo "Dev share: ${XDG_DATA_HOME:-$HOME/.local/share}/tsk-dev"
    echo "Config:   ${XDG_CONFIG_HOME:-$HOME/.config}/tsk-dev/config.toml"
    echo "State:    $DEV_DB → $PROD_DB"
    echo "Session:  $SESSION_FILE (prod tsk + Hyprland helpers read this — no env vars)"
    echo "Session:  active (Ctrl+C or scripts/dev.sh leave restores prod)"
    echo
    start_dev_daemon
    ;;
  leave|exit)
    teardown_dev_session
    echo "Dev mode disabled."
    ;;
  install)
    if [[ "${1:-}" == "share" ]]; then
      prepare_dev_session
      shift
    fi
    run_cli dev install "$@"
    ;;
  daemon)
    maybe_teardown_stale_dev
    link_dev_state_db
    prepare_dev_session
    start_dev_daemon
    ;;
  uninstall)
    run_cli dev uninstall all "$@"
    stop_dev_session
    rm -f "$DEV_DATA/bin/tsk"
    ;;
  status)
    run_cli dev status "$@"
    if [[ -f "$SESSION_FILE" ]]; then
      echo "Session: active ($(cat "$SESSION_FILE"))"
      echo "Session file: $SESSION_FILE"
    else
      echo "Session: inactive"
    fi
    if dev_integration_installed; then
      echo "Integration: dev Hypr source line still present (run scripts/dev.sh leave)"
    fi
    ;;
  *)
    echo "Usage: scripts/dev.sh {enter|leave|install|daemon|uninstall|status}" >&2
    echo "  enter                 — build, write dev-session, install all, start daemon" >&2
    echo "  leave                 — uninstall dev integration and restore prod" >&2
    echo "  install               — build + dev-session + dev install subcommand" >&2
    echo "  daemon                — build, dev-session, start dev daemon" >&2
    echo "  TSK_DEV_ISOLATED=1    — skip prod state.db symlink (CI/e2e)" >&2
    exit 1
    ;;
esac

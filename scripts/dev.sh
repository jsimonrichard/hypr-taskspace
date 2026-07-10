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
PROD_TSKD_MARKER="$PROD_DATA/.dev-prod-tskd-was-active"
HYPR_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}/hypr/hyprland.conf"

PROD_TSKD_WAS_ACTIVE=false
TEARDOWN_DONE=false
DEV_DAEMON_PID=""
QUIET=true

section() {
  echo
  echo "== $* =="
}

note() {
  if ! $QUIET; then
    echo "$@"
  fi
}

stop_prod_tskd() {
  if systemctl --user is-active --quiet tskd.service 2>/dev/null; then
    PROD_TSKD_WAS_ACTIVE=true
    echo "1" >"$PROD_TSKD_MARKER"
    echo "Stopping prod tskd.service for dev session..."
    systemctl --user stop tskd.service
  else
    rm -f "$PROD_TSKD_MARKER"
  fi
}

stop_socket_listener() {
  local sock="$1"
  [[ -e "$sock" ]] || return 0
  if command -v fuser >/dev/null 2>&1; then
    fuser -k -TERM "$sock" 2>/dev/null || true
    sleep 0.2
    fuser -k -KILL "$sock" 2>/dev/null || true
  fi
  rm -f "$sock"
}

stop_pidfile_process() {
  local pidfile="$1"
  [[ -f "$pidfile" ]] || return 0
  local pid
  pid="$(tr -d '[:space:]' <"$pidfile")"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null || true
    sleep 0.2
    kill -KILL "$pid" 2>/dev/null || true
  fi
  rm -f "$pidfile"
}

stop_dev_daemon() {
  local pid="${DEV_DAEMON_PID:-}"
  if [[ -z "$pid" && -f "$DEV_DATA/daemon.pid" ]]; then
    pid="$(tr -d '[:space:]' <"$DEV_DATA/daemon.pid")"
  fi
  if [[ -x "$DEV_BUILD" ]]; then
    env TSK_WORKSPACE="$ROOT" "$DEV_BUILD" daemon stop >/dev/null 2>&1 || true
  fi
  stop_pidfile_process "$DEV_DATA/daemon.pid"
  stop_socket_listener "$DEV_DATA/daemon.sock"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null || true
    sleep 0.2
    kill -KILL "$pid" 2>/dev/null || true
  fi
  DEV_DAEMON_PID=""
}

cleanup_prod_daemon_orphans() {
  if systemctl --user is-active --quiet tskd.service 2>/dev/null; then
    return 0
  fi
  stop_pidfile_process "$PROD_DATA/daemon.pid"
  stop_socket_listener "$PROD_DATA/daemon.sock"
}

stop_orphan_tsk_daemons() {
  local systemd_pid=""
  if systemctl --user is-active --quiet tskd.service 2>/dev/null; then
    systemd_pid="$(systemctl --user show -p MainPID --value tskd.service 2>/dev/null || true)"
  fi
  local line pid cmd
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    pid="${line%% *}"
    cmd="${line#* }"
    [[ -z "$pid" || "$pid" == "$systemd_pid" ]] && continue
    if [[ "$cmd" == *"tsk daemon run"* || "$cmd" == *"tsk daemon"* ]]; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  done < <(pgrep -af '[/ ]tsk( |$).*daemon' 2>/dev/null || true)
  sleep 0.2
}

start_prod_tskd() {
  local should_start=false
  if [[ -f "$PROD_TSKD_MARKER" ]]; then
    should_start=true
    rm -f "$PROD_TSKD_MARKER"
  elif $PROD_TSKD_WAS_ACTIVE; then
    should_start=true
  elif systemctl --user is-enabled --quiet tskd.service 2>/dev/null \
    && ! systemctl --user is-active --quiet tskd.service 2>/dev/null; then
    # dev leave from another shell never saw stop_prod_tskd — restore prod if unit is installed
    should_start=true
  fi
  # Drop orphaned prod socket/pid left when tskd was stopped abruptly (e.g. interrupted teardown).
  cleanup_prod_daemon_orphans
  if $should_start; then
    echo "Starting prod tskd.service..."
    if command -v timeout >/dev/null 2>&1; then
      timeout 10 systemctl --user start tskd.service 2>/dev/null || true
    else
      systemctl --user start tskd.service 2>/dev/null || true
    fi
  fi
}

cmd="${1:-enter}"
shift || true

while [[ $# -gt 0 ]]; do
  case "$1" in
    --verbose|-v)
      QUIET=false
      shift
      ;;
    *)
      break
      ;;
  esac
done

build_dev_binary() {
  local cargo_q=()
  if $QUIET; then
    cargo_q=(-q)
  fi
  if ! $QUIET; then
    echo "Building dev tsk + tsk-waybar (release)..."
    if [[ ! -x "$DEV_BUILD" ]] \
      || [[ -z "$(find "$ROOT/target/release/build" -path '*/libsqlite3-sys-*/out/libsqlite3.a' -print -quit 2>/dev/null)" ]]; then
      echo "Note: bundled SQLite compiles from C on cold builds — libsqlite3-sys(build) may sit ~1 min with no progress bar."
      echo "      Let it finish once; interrupting forces a full recompile on the next dev enter."
    fi
  fi
  cargo build -p tsk-cli -p tsk-waybar --release --target-dir "$ROOT/target" "${cargo_q[@]}"
  if [[ ! -x "$DEV_BUILD" ]]; then
    echo "Dev build missing: $DEV_BUILD" >&2
    exit 1
  fi
}

run_cli() {
  local args=("$@")
  if $QUIET; then
    args+=(--quiet)
  fi
  if [[ -x "$DEV_BUILD" ]]; then
    env TSK_WORKSPACE="$ROOT" "$DEV_BUILD" "${args[@]}"
  else
    cargo run -p tsk-cli --release -- "${args[@]}"
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
  note "Linked dev state.db → $PROD_DB"
}

start_dev_session() {
  mkdir -p "$PROD_DATA"
  echo "$DEV_BUILD" >"$SESSION_FILE"
  rm -f "$DEV_DATA/.session-active"
  note "Dev session active → $DEV_BUILD"
  note "Session file: $SESSION_FILE"
}

stop_dev_session() {
  rm -f "$SESSION_FILE" "$DEV_DATA/.session-active"
}

dev_daemon_reachable() {
  [[ -S "$DEV_DATA/daemon.sock" ]] || return 1
  if [[ -x "$DEV_BUILD" ]]; then
    env TSK_WORKSPACE="$ROOT" "$DEV_BUILD" daemon status 2>/dev/null | grep -q '^running'
    return
  fi
  return 1
}

ensure_dev_not_running() {
  if dev_daemon_reachable; then
    echo "Dev session already running ($DEV_DATA/daemon.sock is in use)." >&2
    echo "  Stop it first: Ctrl+C in the dev enter terminal, or run: scripts/dev.sh leave" >&2
    exit 1
  fi
}

ensure_dev_trap() {
  trap teardown_dev_session EXIT INT TERM
}

prepare_dev_session() {
  build_dev_binary
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

  stop_dev_daemon

  # Ignore further Ctrl+C while cleanup runs (a second interrupt used to abort mid-teardown).
  trap '' INT TERM

  if dev_integration_installed || [[ -f "$SESSION_FILE" ]]; then
    echo "Leaving dev mode — restoring prod integration..."
    uninstall_dev_integration || echo "Warning: dev uninstall had errors (continuing cleanup)" >&2
  fi

  # Drop the session marker before restarting prod so systemd does not re-exec the dev build.
  stop_dev_session
  rm -f "$DEV_DATA/bin/tsk"
  cleanup_prod_daemon_orphans
  stop_orphan_tsk_daemons
  start_prod_tskd
  trap - INT TERM
}

maybe_teardown_stale_dev() {
  if [[ -f "$SESSION_FILE" || -f "$DEV_DATA/.session-active" ]]; then
    if dev_daemon_reachable; then
      return 0
    fi
    echo "Stale dev session file found (daemon not running) — cleaning up..."
    teardown_dev_session
    return 0
  fi
  if dev_integration_installed; then
    echo "Stale dev integration found (no active session) — cleaning up..."
    teardown_dev_session
  fi
}

require_dev_not_running() {
  maybe_teardown_stale_dev
  ensure_dev_not_running
}

start_dev_daemon() {
  if [[ ! -x "$DEV_BUILD" ]]; then
    echo "Dev tsk not found — run: $ROOT/scripts/dev.sh enter" >&2
    exit 1
  fi

  stop_prod_tskd
  ensure_dev_trap
  start_dev_session

  note "Dev daemon (Ctrl+C or scripts/dev.sh leave restores prod)"
  export TSK_QUIET=1
  "$DEV_BUILD" daemon run &
  DEV_DAEMON_PID=$!
  wait "$DEV_DAEMON_PID"
  local status=$?
  DEV_DAEMON_PID=""
  return "$status"
}

case "$cmd" in
  enter)
    require_dev_not_running
    ensure_dev_trap
    section "State"
    link_dev_state_db
    section "Build"
    prepare_dev_session
    section "Integration"
    stop_prod_tskd
    run_cli dev install all "$@"
    if $QUIET; then
      echo
      echo "Dev paths:"
      echo "  share:  ${XDG_DATA_HOME:-$HOME/.local/share}/tsk-dev"
      echo "  config: ${XDG_CONFIG_HOME:-$HOME/.config}/tsk-dev/config.toml"
      echo "  state:  $DEV_DB → $PROD_DB"
      echo "  leave:  scripts/dev.sh leave"
    else
      echo
      echo "Dev share: ${XDG_DATA_HOME:-$HOME/.local/share}/tsk-dev"
      echo "Config:   ${XDG_CONFIG_HOME:-$HOME/.config}/tsk-dev/config.toml"
      echo "State:    $DEV_DB → $PROD_DB"
      echo "Session:  $SESSION_FILE (prod tsk + Hyprland helpers read this — no env vars)"
      echo "Session:  active (Ctrl+C or scripts/dev.sh leave restores prod)"
      echo
    fi
    section "Daemon"
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
    require_dev_not_running
    ensure_dev_trap
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
    echo "  enter --verbose       — same, with full cargo/tsk install output" >&2
    echo "  leave                 — uninstall dev integration and restore prod" >&2
    echo "  install               — build + dev-session + dev install subcommand" >&2
    echo "  install all           — Hyprland + Waybar + Walker (Elephant) integration" >&2
    echo "  daemon                — build, dev-session, start dev daemon" >&2
    echo "  TSK_DEV_ISOLATED=1    — skip prod state.db symlink (CI/e2e)" >&2
    exit 1
    ;;
esac

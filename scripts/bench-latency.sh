#!/usr/bin/env bash
# Ad hoc latency benchmarks for tsk spawn + daemon RPC paths.
# Uses TERMINAL=/bin/false where possible so no real windows open; still cleans up on exit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLEANUP="$ROOT/scripts/close-test-terminals.sh"

cleanup() {
  "$CLEANUP" 2>/dev/null || true
}
trap cleanup EXIT

TSK="${TSK:-$ROOT/target/release/tsk}"
if [[ ! -x "$TSK" ]]; then
  TSK="$(command -v tsk)"
fi

bench5() {
  local label="$1"
  shift
  local sum=0 i t
  for i in 1 2 3 4 5; do
    t=$( { TIMEFORMAT='%R'; time "$@" >/dev/null 2>&1; } 2>&1 )
    sum=$(awk -v a="$sum" -v b="$t" 'BEGIN{print a+b}')
  done
  awk -v l="$label" -v s="$sum" 'BEGIN{printf "%-34s %.3f sec avg\n", l, s/5}'
}

echo "=== Daemon RPC (unix socket, 1 new connection per call) ==="
echo "Note: this microbench includes per-connection setup; CLI read/spawn paths often skip the daemon."
python3 -c "
import json, socket, time
from pathlib import Path
sock = Path.home() / '.local/share/tsk/daemon.sock'
if not sock.exists():
    print('daemon socket not found — skipping RPC benchmarks')
    raise SystemExit(0)
for method in ['ping', 'get_state']:
    times = []
    for _ in range(20):
        t0 = time.perf_counter()
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(5)
        s.connect(str(sock))
        s.sendall((json.dumps({'method': method, 'params': {}}) + '\n').encode())
        s.recv(65536)
        s.close()
        times.append((time.perf_counter() - t0) * 1000)
    print(f'{method:12s} avg={sum(times)/len(times):.2f}ms min={min(times):.2f}ms max={max(times):.2f}ms')
"

echo
echo "=== CLI paths (TERMINAL=/bin/false — no real windows) ==="
bench5 "task terminal" env TERMINAL=/bin/false "$TSK" task terminal
bench5 "task terminal --host" env TERMINAL=/bin/false "$TSK" task terminal --host
bench5 "tui-launch" env TERMINAL=/bin/false "$TSK" task tui-launch
bench5 "status" "$TSK" status

echo
echo "=== Optional: real terminal spawn (set BENCH_REAL_TERMINAL=1) ==="
if [[ "${BENCH_REAL_TERMINAL:-}" == "1" ]]; then
  bench5 "task terminal (real)" "$TSK" task terminal
  bench5 "tui-launch (real)" "$TSK" task tui-launch
fi

echo
echo "Done. Cleaning up any tsk test terminals..."

#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# vite-port.test.sh — tests for scripts/vite-port.sh.
#
# The failure being guarded: `task dev` pointed at port 6000 (on Chromium's
# blocked-ports list) gave a window that flickered forever with no explanation.
# docs/specs/SPEC_DEV_VITE_UNSAFE_PORT_GUARD_2026_10_02.md
#
# Usage: bash scripts/vite-port.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUT="$HERE/vite-port.sh"
PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); }
bad() { FAIL=$((FAIL + 1)); echo "FAIL: $1"; }

expect_status() { # <want 0|1> <label> <args...>
  local want="$1" label="$2"
  shift 2
  bash "$SUT" "$@" >/dev/null 2>&1
  local got=$?
  if [ "$got" = "$want" ]; then ok; else bad "$label (wanted exit $want, got $got)"; fi
}

# 1. Every listed port is blocked; the neighbours of an isolated entry are not.
for p in 1719 1720 1723 2049 3659 4045 4190 5060 5061 6000 6566 6665 6666 6667 6668 6669 6679 6697 10080; do
  expect_status 0 "blocked $p" blocked "$p"
done
for p in 5999 6001 5059 5062 3658 3660 6565 6567 6664 6670 10079 10081; do
  expect_status 1 "not blocked $p" blocked "$p"
done

# 2. check: refuses bad values and blocked ports, accepts ordinary ones.
for v in "" abc 12x 0 80 1023 65536 99999 -5 6000 6666; do
  expect_status 1 "check rejects '$v'" check "$v"
done
for v in 1024 5173 5300 5372 8080 65535; do
  expect_status 0 "check accepts $v" check "$v"
done

# 3. The error for a blocked port names the port and the way out.
msg="$(bash "$SUT" check 6000 2>&1)"
case "$msg" in *6000*ERR_UNSAFE_PORT*) ok ;; *) bad "blocked-port message names the port and the error: $msg" ;; esac
case "$msg" in *"vite-port.sh pick"*) ok ;; *) bad "blocked-port message shows the fix: $msg" ;; esac

# 4. Every port Taskfile.yml can pick on its own (5173 + 0..199) is usable.
auto_bad=""
for off in $(seq 0 199); do
  bash "$SUT" check $((5173 + off)) >/dev/null 2>&1 || auto_bad="$auto_bad $((5173 + off))"
done
if [ -z "$auto_bad" ]; then ok; else bad "automatic ports refused:$auto_bad"; fi

# 5. pick: never returns a blocked port, starting right below one.
got="$(bash "$SUT" pick 5999 2>/dev/null)"
if [ -n "$got" ] && ! bash "$SUT" blocked "$got"; then ok; else bad "pick 5999 returned '$got'"; fi
if [ "$got" != "6000" ]; then ok; else bad "pick 5999 returned the blocked 6000"; fi
got="$(bash "$SUT" pick 2>/dev/null)"
if [ -n "$got" ] && [ "$got" -ge 5300 ]; then ok; else bad "pick default start returned '$got'"; fi

# 6. pick and listening see a port that is really in use.
PY="$(command -v python3 || command -v python || true)"
if [ -n "$PY" ]; then
  FREE="$(bash "$SUT" pick 5300)"
  "$PY" - "$FREE" <<'EOF' &
import socket, sys, time
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))
s.listen(1)
time.sleep(20)
EOF
  LISTENER=$!
  for _ in $(seq 1 50); do
    bash "$SUT" listening "$FREE" && break
    sleep 0.2
  done
  expect_status 0 "listening sees $FREE" listening "$FREE"
  got="$(bash "$SUT" pick "$FREE")"
  if [ "$got" != "$FREE" ] && [ -n "$got" ]; then ok; else bad "pick returned the busy port $FREE (got '$got')"; fi
  kill "$LISTENER" 2>/dev/null
  wait "$LISTENER" 2>/dev/null
else
  echo "note: no python on PATH — skipped the live-listener checks"
fi

# 7. With no probe tool on PATH, `pick` and `listening` fail loudly instead of
#    reporting every port as free (ReAgent P2 on #4214). An empty PATH hides
#    lsof, ss and netstat alike; `blocked` and `check` need none of them.
EMPTY="$(mktemp -d)"
out="$(env PATH="$EMPTY" "$BASH" "$SUT" pick 2>&1)"; st=$?
if [ "$st" != 0 ]; then ok; else bad "pick without a probe tool succeeded: $out"; fi
case "$out" in *"cannot tell which ports are in use"*) ok ;; *) bad "pick without a probe tool says why: $out" ;; esac
env PATH="$EMPTY" "$BASH" "$SUT" listening 5300 >/dev/null 2>&1; st=$?
if [ "$st" != 0 ]; then ok; else bad "listening without a probe tool reported success"; fi
env PATH="$EMPTY" "$BASH" "$SUT" check 5300 >/dev/null 2>&1 && ok || bad "check should not need a probe tool"
rmdir "$EMPTY" 2>/dev/null

echo "vite-port.test.sh: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

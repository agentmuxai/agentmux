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
WAIT="$HERE/dev-vite-wait.sh"
# Claims go to a folder of their own, never the real ~/.agentmux/dev-port-claims.
export AGENTMUX_DEV_PORT_CLAIMS="$(mktemp -d)"
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
# Leading zeros are the same port to Vite and Chromium.
for p in 06000 006666 05060; do
  expect_status 0 "blocked $p (leading zeros)" blocked "$p"
done
for p in 5999 6001 5059 5062 3658 3660 6565 6567 6664 6670 10079 10081; do
  expect_status 1 "not blocked $p" blocked "$p"
done

# 2. check: refuses bad values and blocked ports, accepts ordinary ones.
for v in "" abc 12x 0 80 1023 65536 99999 -5 6000 6666   06000 006666 05300 0 00000 99999999999999999999 123456 " 6000" "6000 "; do
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
#    reporting every port as free (muxreview P2 on #4214). An empty PATH hides
#    lsof, ss and netstat alike; `blocked` and `check` need none of them.
EMPTY="$(mktemp -d)"
out="$(env PATH="$EMPTY" "$BASH" "$SUT" pick 2>&1)"; st=$?
if [ "$st" != 0 ]; then ok; else bad "pick without a probe tool succeeded: $out"; fi
case "$out" in *"cannot tell which ports are in use"*) ok ;; *) bad "pick without a probe tool says why: $out" ;; esac
env PATH="$EMPTY" "$BASH" "$SUT" listening 5300 >/dev/null 2>&1; st=$?
if [ "$st" != 0 ]; then ok; else bad "listening without a probe tool reported success"; fi
env PATH="$EMPTY" "$BASH" "$SUT" check 5300 >/dev/null 2>&1 && ok || bad "check should not need a probe tool"
rmdir "$EMPTY" 2>/dev/null

# 8. Claims: a pick reserves its port. The failure: two `task dev` runs picked 5301
#    within seconds of each other, then waited for the build slot together, and both
#    started Vite there (docs/retro/RETRO_DEV_VITE_PORT_RACE_COUPLED_INSTANCES_2026_11_06.md).
PICKS="$(mktemp -d)"
for i in $(seq 1 12); do
  AGENTMUX_DEV_PORT_OWNER="clone$i" bash "$SUT" pick 5400 >"$PICKS/$i" 2>/dev/null &
done
wait
distinct="$(cat "$PICKS"/* | sort -u | wc -l | tr -d ' ')"
if [ "$distinct" = 12 ]; then ok; else bad "12 simultaneous picks gave $distinct different ports, not 12"; fi
rm -rf "$PICKS"

CLAIM_PORT=5450
AGENTMUX_DEV_PORT_OWNER=clone-a bash "$SUT" claim "$CLAIM_PORT" >/dev/null 2>&1 && ok || bad "first claim refused"
AGENTMUX_DEV_PORT_OWNER=clone-a bash "$SUT" claim "$CLAIM_PORT" >/dev/null 2>&1 && ok || bad "a clone's own claim must renew"
got="$(AGENTMUX_DEV_PORT_OWNER=clone-a bash "$SUT" pick "$CLAIM_PORT" 2>/dev/null)"
if [ "$got" = "$CLAIM_PORT" ]; then ok; else bad "a clone's own pick should give its claimed port back (got '$got')"; fi
out="$(AGENTMUX_DEV_PORT_OWNER=clone-b AGENTMUX_AGENT_ID=other bash "$SUT" claim "$CLAIM_PORT" 2>&1)"; st=$?
if [ "$st" = 1 ]; then ok; else bad "another clone's claim on a held port exited $st"; fi
case "$out" in *clone-a*) ok ;; *) bad "the refusal names the holder: $out" ;; esac
got="$(AGENTMUX_DEV_PORT_OWNER=clone-b bash "$SUT" pick "$CLAIM_PORT" 2>/dev/null)"
if [ -n "$got" ] && [ "$got" != "$CLAIM_PORT" ]; then ok; else bad "another clone's pick returned the claimed port (got '$got')"; fi
# Only the holder can let go.
AGENTMUX_DEV_PORT_OWNER=clone-b bash "$SUT" release "$CLAIM_PORT" >/dev/null 2>&1
AGENTMUX_DEV_PORT_OWNER=clone-b bash "$SUT" claim "$CLAIM_PORT" >/dev/null 2>&1 && bad "another clone released the claim" || ok
AGENTMUX_DEV_PORT_OWNER=clone-a bash "$SUT" release "$CLAIM_PORT" >/dev/null 2>&1
AGENTMUX_DEV_PORT_OWNER=clone-b bash "$SUT" claim "$CLAIM_PORT" >/dev/null 2>&1 && ok || bad "the port was not free after its holder released it"
# A claim nobody renewed (its run is gone) is taken over.
sleep 2
AGENTMUX_DEV_PORT_CLAIM_TTL=1 AGENTMUX_DEV_PORT_OWNER=clone-c bash "$SUT" claim "$CLAIM_PORT" >/dev/null 2>&1 && ok || bad "a lapsed claim was not taken over"
# A takeover that was killed half way leaves its lock; that must not wedge the port for good.
sleep 2
mkdir "$AGENTMUX_DEV_PORT_CLAIMS/$CLAIM_PORT.lock"
touch -d '5 minutes ago' "$AGENTMUX_DEV_PORT_CLAIMS/$CLAIM_PORT.lock" 2>/dev/null || touch -t "$(date -d '5 minutes ago' +%Y%m%d%H%M 2>/dev/null || date -v-5M +%Y%m%d%H%M)" "$AGENTMUX_DEV_PORT_CLAIMS/$CLAIM_PORT.lock"
AGENTMUX_DEV_PORT_CLAIM_TTL=1 AGENTMUX_DEV_PORT_OWNER=clone-d bash "$SUT" claim "$CLAIM_PORT" >/dev/null 2>&1 && ok || bad "an abandoned takeover lock blocked a lapsed claim"
if [ ! -d "$AGENTMUX_DEV_PORT_CLAIMS/$CLAIM_PORT.lock" ]; then ok; else bad "a takeover left its lock behind"; fi
# claim rejects what is not a port.
expect_status 2 "claim rejects a non-port" claim abc

# 9. dev-vite-wait.sh: ours or refuse. The failure: a Vite that lost the bind exited
#    quietly and `curl` passed against the winner, so the window loaded its frontend.
NODE="$(command -v node || true)"
if [ -n "$NODE" ]; then
  WPORT="$(bash "$SUT" pick 5600)"
  "$NODE" -e '
    require("http").createServer((q, r) => {
      if (q.url === "/__agentmux_dev_instance") {
        r.setHeader("content-type", "application/json");
        r.end(JSON.stringify({ token: process.argv[1], owner: "other-agent", commit: "abc1234" }));
      } else { r.statusCode = 404; r.end(); }
    }).listen(Number(process.argv[2]));
  ' held-token "$WPORT" &
  SERVER=$!
  for _ in $(seq 1 50); do
    bash "$SUT" listening "$WPORT" && break
    sleep 0.2
  done
  bash "$WAIT" "$WPORT" held-token 5 >/dev/null 2>&1 && ok || bad "wait refused the server that has our token"
  out="$(bash "$WAIT" "$WPORT" my-token 5 2>&1)"; st=$?
  if [ "$st" = 1 ]; then ok; else bad "wait accepted another run's server (exit $st)"; fi
  case "$out" in *"not with this run's Vite"*other-agent*) ok ;; *) bad "the refusal names the other run: $out" ;; esac
  # It must fail at once, not after the whole wait: our own Vite cannot answer with another token.
  start=$SECONDS
  bash "$WAIT" "$WPORT" my-token 20 >/dev/null 2>&1
  if [ $((SECONDS - start)) -le 5 ]; then ok; else bad "wait kept waiting on another run's server for $((SECONDS - start))s"; fi
  kill "$SERVER" 2>/dev/null
  wait "$SERVER" 2>/dev/null
  # Nothing answering at all is a failure too, after the wait runs out.
  out="$(bash "$WAIT" "$(bash "$SUT" pick 5700)" my-token 2 2>&1)"; st=$?
  if [ "$st" = 1 ]; then ok; else bad "wait on a silent port exited $st"; fi
  case "$out" in *"did not answer"*) ok ;; *) bad "silent-port message: $out" ;; esac
else
  echo "note: no node on PATH — skipped the dev-vite-wait checks"
fi
bash "$WAIT" >/dev/null 2>&1; [ "$?" = 2 ] && ok || bad "dev-vite-wait.sh with no arguments should exit 2"

rm -rf "$AGENTMUX_DEV_PORT_CLAIMS"
echo "vite-port.test.sh: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]

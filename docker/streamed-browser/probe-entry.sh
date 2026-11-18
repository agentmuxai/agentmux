#!/bin/sh
# Start a headless srv with the streamed browser on, then run the probe against it. Prints the probe's JSON.
set -eu
KEY=/tmp/srv-key
head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$KEY"
agentmux-srv --headless --web-port 8190 --ws-port 8191 --auth-key-file "$KEY" > /tmp/srv.log 2>&1 &
SRV=$!
for _ in $(seq 1 60); do
  node -e "require('net').connect(8190,'127.0.0.1').on('connect',()=>process.exit(0)).on('error',()=>process.exit(1))" && break
  sleep 0.5
done
set +e
if [ "${GUARD:-}" = "1" ]; then
  # The address proxy and the blocked destinations (spec section 12), instead of the timing probe.
  node /app/guard-probe.mjs --port 8190 --ws-port 8191 --key-file "$KEY" ${EXTRA_DENIED:+--extra-denied "$EXTRA_DENIED"}
else
  node /app/probe.mjs --port 8190 --key-file "$KEY" --runs "${RUNS:-5}"
fi
CODE=$?
if [ "${GUARD:-}" = "1" ]; then
  # What Chromium itself was started with (spec section 12): no variable of srv's or a credential in its environment,
  # the proxy and the switches that close every other way out, and no TCP debugging port.
  PID=$(ps -eo pid,args | grep -- '--remote-debugging-pipe' | grep -v -- '--type=' | grep -v grep | awk '{print $1}' | head -1)
  if [ -z "$PID" ]; then
    echo "guard: FAIL no Chromium main process to inspect" >&2
    CODE=1
  else
    NAMES=$(tr '\0' '\n' < "/proc/$PID/environ" | cut -d= -f1)
    echo "guard: Chromium environment names: $(echo "$NAMES" | tr '\n' ' ')" >&2
    if echo "$NAMES" | grep -Eq '^(AGENTMUX|AMX_|AWS_)|KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL'; then
      echo "guard: FAIL Chromium's environment holds a variable of srv's or a credential" >&2
      CODE=1
    else
      echo "guard: ok Chromium's environment holds none of srv's variables or a credential" >&2
    fi
    ARGS=$(tr '\0' '\n' < "/proc/$PID/cmdline")
    for want in '--disable-quic' '--force-webrtc-ip-handling-policy=disable_non_proxied_udp' '--dns-prefetch-disable' '--proxy-server=socks5://127.0.0.1:' '--host-resolver-rules=MAP * ~NOTFOUND' '--remote-debugging-pipe'; do
      if echo "$ARGS" | grep -qF -- "$want"; then echo "guard: ok flag $want" >&2; else echo "guard: FAIL missing flag $want" >&2; CODE=1; fi
    done
    if echo "$ARGS" | grep -q -- '--remote-debugging-port'; then echo "guard: FAIL a TCP debugging port is open" >&2; CODE=1; fi
  fi
fi
# One Chromium serves every pane: its main process is the one with the pipe and no `--type=`.
echo "chromium main processes: $(ps -eo args | grep -- '--remote-debugging-pipe' | grep -v -- '--type=' | grep -vc grep)" >&2
grep -i 'streamed' /tmp/srv.log >&2 || true
kill "$SRV" 2>/dev/null
[ "$CODE" -ne 0 ] && tail -40 /tmp/srv.log >&2
exit "$CODE"

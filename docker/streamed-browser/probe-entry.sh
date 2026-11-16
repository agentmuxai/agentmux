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
node /app/probe.mjs --port 8190 --key-file "$KEY" --runs "${RUNS:-5}"
CODE=$?
# One Chromium serves every pane: its main process is the one with the pipe and no `--type=`.
echo "chromium main processes: $(ps -eo args | grep -- '--remote-debugging-pipe' | grep -v -- '--type=' | grep -vc grep)" >&2
grep -i 'streamed' /tmp/srv.log >&2 || true
kill "$SRV" 2>/dev/null
[ "$CODE" -ne 0 ] && tail -40 /tmp/srv.log >&2
exit "$CODE"

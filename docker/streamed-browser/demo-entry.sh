#!/bin/sh
# Starts the demo: a headless srv with its own web UI and the streamed browser on, a small server for the demo pages,
# and a forwarder so the one published port reaches srv's loopback. See README.md.
set -eu
node /app/pages-server.js &
agentmux-srv --headless --web-port 8190 --ws-port 8191 --frontend-dir /app/web > /tmp/srv.log 2>&1 &
# srv binds loopback only; this makes it reachable on the container's address for the one published port.
socat TCP-LISTEN:18190,fork,reuseaddr TCP:127.0.0.1:8190 &
echo "streamed browser demo is up: srv on 127.0.0.1:8190 (forwarded on 18190), pages on 127.0.0.1:8200" >&2
wait

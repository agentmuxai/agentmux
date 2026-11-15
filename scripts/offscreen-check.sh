#!/usr/bin/env bash
# Fail closed before a test launch that must never show a window on a real display.
#
#   scripts/offscreen-check.sh <launcher-or-host-binary>...
#
# A build from before AGENTMUX_TEST_OFFSCREEN ignores the variable and opens its window on the
# screen. Each binary given must carry the variable's name (the host and the launcher both read
# it); one that does not is refused with exit status 1, and the caller must not start it. Run it
# on every binary of the build under test, then start the build with AGENTMUX_TEST_OFFSCREEN=1:
#
#   scripts/offscreen-check.sh ./agentmux-launcher ./runtime/agentmux-cef-host \
#     && AGENTMUX_TEST_OFFSCREEN=1 ./agentmux-launcher
#
# After the start, the host's log says "test window mode: every window is created off-screen"
# (a dev or local build) or "AGENTMUX_TEST_OFFSCREEN=1 ignored" (a published release, which
# never honours it: do not use a release for a test that must stay off the screen).
set -u

NAME=AGENTMUX_TEST_OFFSCREEN

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <binary>..." >&2
  exit 2
fi

status=0
for bin in "$@"; do
  if [ ! -f "$bin" ]; then
    echo "offscreen-check: $bin: no such file; do not launch" >&2
    status=1
  elif ! grep -aq -- "$NAME" "$bin"; then
    echo "offscreen-check: $bin does not support $NAME (an older build): it would open a window on the screen; do not launch" >&2
    status=1
  fi
done
exit "$status"

#!/bin/sh
# Start a headless srv with the streamed browser on, then run the probe against it. Prints the probe's JSON.
set -eu
KEY=/tmp/srv-key
head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$KEY"
if [ "${GUARD:-}" = "1" ]; then
  # Names a hosted task's environment really has, plus one of srv's own and an arbitrary secret-looking one. None of
  # them may reach Chromium. (Dummy values: nothing here is a credential.)
  CANARIES="AWS_CONTAINER_CREDENTIALS_RELATIVE_URI ECS_CONTAINER_METADATA_URI_V4 AWS_EXECUTION_ENV AGENTMUX_CANARY_VALUE DEPLOY_SECRET_CANARY"
  export AWS_CONTAINER_CREDENTIALS_RELATIVE_URI=/canary AWS_EXECUTION_ENV=canary AGENTMUX_CANARY_VALUE=canary DEPLOY_SECRET_CANARY=canary
  export ECS_CONTAINER_METADATA_URI_V4=http://169.254.170.2/v4/canary
fi
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
  # What Chromium itself was started with (spec section 12). Chromium is started through Debian's /usr/bin/chromium
  # wrapper, which exports a few fixed names of its own and may add flags from /etc/chromium.d; both are judged here
  # on the real process: the environment against an allow-list, the command line against flags that would weaken ours.
  PID=$(ps -eo pid,args | grep -- '--remote-debugging-pipe' | grep -v -- '--type=' | grep -v grep | awk '{print $1}' | head -1)
  if [ -z "$PID" ]; then
    echo "guard: FAIL no Chromium main process to inspect" >&2
    CODE=1
  else
    SRV_NAMES=$(tr '\0' '\n' < "/proc/$SRV/environ" | cut -d= -f1)
    for c in $CANARIES; do
      echo "$SRV_NAMES" | grep -qx "$c" || { echo "guard: FAIL canary $c is not in srv's own environment (the check would prove nothing)" >&2; CODE=1; }
    done
    NAMES=$(tr '\0' '\n' < "/proc/$PID/environ" | cut -d= -f1)
    echo "guard: Chromium environment names: $(echo "$NAMES" | tr '\n' ' ')" >&2
    # srv passes these on (chromium.rs ENV_KEEP); the wrapper adds the rest by exact name; the shell adds PWD and friends.
    ALLOWED="PATH HOME USER LOGNAME LANG LANGUAGE LC_ALL LC_CTYPE TZ TMPDIR FONTCONFIG_FILE FONTCONFIG_PATH PWD OLDPWD SHLVL _ CHROME_DESKTOP CHROME_WRAPPER CHROME_VERSION_EXTRA CHROMIUM_FLAGS GOOGLE_API_KEY GOOGLE_DEFAULT_CLIENT_ID GOOGLE_DEFAULT_CLIENT_SECRET"
    BAD=""
    for n in $NAMES; do
      case " $ALLOWED " in *" $n "*) ;; *) BAD="$BAD $n" ;; esac
    done
    for c in $CANARIES; do
      echo "$NAMES" | grep -qx "$c" && BAD="$BAD $c"
    done
    if [ -n "$BAD" ]; then
      echo "guard: FAIL Chromium's environment holds names outside the allow-list:$BAD" >&2
      CODE=1
    else
      echo "guard: ok Chromium's environment holds nothing of srv's or the platform's (canaries absent, every name allow-listed)" >&2
    fi
    ARGS=$(tr '\0' '\n' < "/proc/$PID/cmdline")
    for want in '--disable-quic' '--force-webrtc-ip-handling-policy=disable_non_proxied_udp' '--dns-prefetch-disable' '--proxy-server=socks5://127.0.0.1:' '--host-resolver-rules=MAP * ~NOTFOUND' '--remote-debugging-pipe'; do
      if echo "$ARGS" | grep -qF -- "$want"; then echo "guard: ok flag $want" >&2; else echo "guard: FAIL missing flag $want" >&2; CODE=1; fi
    done
    # The final command line, after the wrapper: ours exactly once, and nothing that undoes them.
    for once in '--proxy-server=' '--host-resolver-rules='; do
      N=$(echo "$ARGS" | grep -c -- "^$once")
      if [ "$N" = "1" ]; then echo "guard: ok $once appears once" >&2; else echo "guard: FAIL $once appears $N times" >&2; CODE=1; fi
    done
    for never in '--remote-debugging-port' '--remote-debugging-address' '--enable-quic' '--no-proxy-server' '--proxy-pac-url' '--proxy-auto-detect' '--remote-allow-origins' '--enable-features=.*Quic' '--origin-to-force-quic-on'; do
      if echo "$ARGS" | grep -Eq -- "^$never"; then echo "guard: FAIL the final command line carries $never" >&2; CODE=1; fi
    done
    echo "guard: Chromium command line has $(echo "$ARGS" | wc -l) arguments" >&2
  fi
fi
# One Chromium serves every pane: its main process is the one with the pipe and no `--type=`.
echo "chromium main processes: $(ps -eo args | grep -- '--remote-debugging-pipe' | grep -v -- '--type=' | grep -vc grep)" >&2
grep -i 'streamed' /tmp/srv.log >&2 || true
kill "$SRV" 2>/dev/null
[ "$CODE" -ne 0 ] && tail -40 /tmp/srv.log >&2
exit "$CODE"

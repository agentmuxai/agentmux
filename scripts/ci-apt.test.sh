#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# ci-apt.test.sh — tests for ci-apt.sh, with sudo, timeout, apt-get, getent and sleep replaced by stubs.
#
# Usage: bash scripts/ci-apt.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/ci-apt.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
mkdir -p "$BIN"
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# sudo runs the command, but keeps the apt config out of /etc.
cat > "$BIN/sudo" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = tee ]; then exec tee "$STUB_DIR/aptconf"; fi
exec "$@"
EOF
# timeout runs the command, or reports 124 for the first N calls.
cat > "$BIN/timeout" <<'EOF'
#!/usr/bin/env bash
shift
n=$(cat "$STUB_DIR/timeout-calls" 2>/dev/null || echo 0)
echo $((n + 1)) > "$STUB_DIR/timeout-calls"
if [ "$n" -lt "${STUB_TIMEOUTS:-0}" ]; then exit 124; fi
exec "$@"
EOF
# apt-get consumes one outcome per call from APT_UPDATE_SEQ / APT_INSTALL_SEQ (the last one repeats).
cat > "$BIN/apt-get" <<'EOF'
#!/usr/bin/env bash
kind=$1
n=$(cat "$STUB_DIR/calls-$kind" 2>/dev/null || echo 0)
echo $((n + 1)) > "$STUB_DIR/calls-$kind"
if [ "$kind" = update ]; then seq=${APT_UPDATE_SEQ:-ok}; else seq=${APT_INSTALL_SEQ:-ok}; fi
set -- $seq
if [ "$n" -ge "$#" ]; then n=$(($# - 1)); fi
outcome=${@:$((n + 1)):1}
case "$outcome" in
    ok) echo "Get:1 http://archive.ubuntu.com/ubuntu noble InRelease"; exit 0 ;;
    bad) echo "Err:2 https://security.ubuntu.com/ubuntu noble InRelease"; echo "  Temporary failure resolving 'azure.archive.ubuntu.com'"; exit 100 ;;
    warn) echo "W: Failed to fetch mirror+file:/etc/apt/apt-mirrors.txt/dists/noble/InRelease  Temporary failure resolving 'azure.archive.ubuntu.com'"; exit 0 ;;
    third) echo "E: Failed to fetch https://dl.google.com/linux/chrome/deb/dists/stable/InRelease  Hash Sum mismatch"; exit 100 ;;
    fail) echo "E: Failed to fetch https://security.ubuntu.com/ubuntu/pool/main/x.deb"; exit 100 ;;
esac
EOF
# getent fails for the first STUB_DNS_DOWN calls.
cat > "$BIN/getent" <<'EOF'
#!/usr/bin/env bash
n=$(cat "$STUB_DIR/getent-calls" 2>/dev/null || echo 0)
echo $((n + 1)) > "$STUB_DIR/getent-calls"
if [ "$n" -lt "${STUB_DNS_DOWN:-0}" ]; then exit 2; fi
echo "203.0.113.7 $2"
EOF
cat > "$BIN/sleep" <<'EOF'
#!/usr/bin/env bash
echo "$1" >> "$STUB_DIR/sleeps"
EOF
chmod +x "$BIN"/*

n=0
# run <name> <env assignments...> -- <ci-apt.sh arguments...>; leaves $got, $out, $dir.
run() {
    n=$((n+1)); dir="$TMP/case$n"; mkdir -p "$dir"
    local envs=()
    while [ "$1" != "--" ]; do envs+=("$1"); shift; done
    shift
    out="$(env STUB_DIR="$dir" PATH="$BIN:$PATH" CI_APT_PAUSE=7 CI_APT_DNS_WAIT=30 ${envs[@]+"${envs[@]}"} bash "$SCRIPT" "$@" 2>&1)"; got=$?
}
calls() { cat "$dir/calls-$1" 2>/dev/null || echo 0; }
check() {  # check <name> <condition result 0/1> <detail>
    if [ "$2" = 0 ]; then ok "$1"; else bad "$1" "$3"; fi
}

echo "ci-apt.sh update"
run APT_UPDATE_SEQ="ok" -- update
check "clean first try: one update, exit 0" "$([ $got = 0 ] && [ "$(calls update)" = 1 ]; echo $?)" "exit $got, calls $(calls update): $out"
check "writes apt's retry settings" "$(grep -q 'Acquire::Retries "5"' "$dir/aptconf"; echo $?)" "$(cat "$dir/aptconf" 2>&1)"

run APT_UPDATE_SEQ="bad bad ok" -- update
check "two Ubuntu failures then success: three updates, exit 0" "$([ $got = 0 ] && [ "$(calls update)" = 3 ]; echo $?)" "exit $got, calls $(calls update): $out"
check "the pause grows with the attempt (7 s then 14 s)" "$([ "$(paste -sd, "$dir/sleeps")" = "7,14" ]; echo $?)" "sleeps: $(paste -sd, "$dir/sleeps")"

run APT_UPDATE_SEQ="warn ok" -- update --strict
check "the quiet 'W: Failed to fetch' form is a failure too" "$([ $got = 0 ] && [ "$(calls update)" = 2 ]; echo $?)" "exit $got, calls $(calls update): $out"

run APT_UPDATE_SEQ="bad" -- update
check "never recovers, not strict: warns and exits 0 after 3 tries" "$([ $got = 0 ] && [ "$(calls update)" = 3 ] && echo "$out" | grep -q 'continuing'; echo $?)" "exit $got, calls $(calls update): $out"

run APT_UPDATE_SEQ="bad" -- update --strict
check "never recovers, strict: exit 1 with an error line" "$([ $got = 1 ] && echo "$out" | grep -q '::error::apt-get update failed against the Ubuntu archive'; echo $?)" "exit $got: $out"

run APT_UPDATE_SEQ="third" -- update --strict
check "a broken third-party index alone is not a failure (one update, exit 0)" "$([ $got = 0 ] && [ "$(calls update)" = 1 ]; echo $?)" "exit $got, calls $(calls update): $out"

run STUB_TIMEOUTS=1 APT_UPDATE_SEQ="ok" -- update --strict
check "an update killed by its time limit (124) is retried" "$([ $got = 0 ] && echo "$out" | grep -q 'ran past'; echo $?)" "exit $got: $out"

echo "ci-apt.sh install"
run APT_INSTALL_SEQ="fail fail ok" -- install -y libfoo-dev
check "two failures then success: three installs, two list refreshes between, exit 0" "$([ $got = 0 ] && [ "$(calls install)" = 3 ] && [ "$(calls update)" = 2 ]; echo $?)" "exit $got, installs $(calls install), updates $(calls update): $out"

run APT_INSTALL_SEQ="ok" -- install -y libfoo-dev
check "clean first try: one install, no refresh" "$([ $got = 0 ] && [ "$(calls install)" = 1 ] && [ "$(calls update)" = 0 ]; echo $?)" "exit $got: $out"

run APT_INSTALL_SEQ="fail" -- install -y libfoo-dev
check "never succeeds: exit 1 after 3 installs, with an error line" "$([ $got = 1 ] && [ "$(calls install)" = 3 ] && echo "$out" | grep -q '::error::apt-get install -y libfoo-dev failed after 3 attempts'; echo $?)" "exit $got, installs $(calls install): $out"

run CI_APT_ATTEMPTS=1 APT_INSTALL_SEQ="fail" -- install -y libfoo-dev
check "CI_APT_ATTEMPTS bounds the tries" "$([ $got = 1 ] && [ "$(calls install)" = 1 ]; echo $?)" "exit $got, installs $(calls install)"

run -- install
check "install with nothing to install is a usage error (2)" "$([ $got = 2 ]; echo $?)" "exit $got: $out"

echo "name resolution"
run STUB_DNS_DOWN=2 APT_UPDATE_SEQ="ok" -- update
check "waits for the archive hosts to resolve, then goes ahead" "$([ $got = 0 ] && echo "$out" | grep -q 'do not resolve yet; waiting' && ! echo "$out" | grep -q 'trying apt anyway'; echo $?)" "exit $got: $out"

run STUB_DNS_DOWN=999 APT_UPDATE_SEQ="ok" -- update
check "gives up waiting after CI_APT_DNS_WAIT, says so, and still tries apt" "$([ $got = 0 ] && echo "$out" | grep -q 'trying apt anyway' && [ "$(calls update)" = 1 ]; echo $?)" "exit $got: $out"

run -- frobnicate
check "an unknown command is a usage error (2)" "$([ $got = 2 ]; echo $?)" "exit $got"

echo
echo "passed: $pass  failed: $fail"
[ "$fail" = 0 ]

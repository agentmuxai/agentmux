#!/usr/bin/env bash
# agentmux-install-update.test.sh -- tests for agentmux-install-update.sh, the root helper that
# installs an AgentMux .deb or .rpm, with the package tools, curl and /proc replaced by fakes.
# No network, no root. Run: bash scripts/agentmux-install-update.test.sh (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HELPER="$HERE/agentmux-install-update.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"; [ -n "${SLEEPER:-}" ] && kill "$SLEEPER" 2>/dev/null' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

BIN="$TMP/bin"; mkdir -p "$BIN"
fake() { printf '#!/bin/sh\n%s\n' "$2" > "$BIN/$1"; chmod +x "$BIN/$1"; }
fake uname 'echo x86_64'
fake curl 'case "$*" in */download/*) case "$*" in *" -L "*" --proto-redir =https "*) cat "$FAKE_DOWNLOAD";; esac;; *) cat "$FAKE_LISTING";; esac'
fake dpkg-query 'case "$1" in -S) [ "$FAKE_OWNER" = deb ];; -W) printf %s "$FAKE_INSTALLED";; esac'
fake dpkg-deb 'case "$3" in Package) echo "$FAKE_NAME";; Version) echo "$FAKE_VERSION";; Architecture) echo "${FAKE_ARCH:-amd64}";; esac'
fake dpkg '[ "$1" = --compare-versions ] && [ "$3" = gt ] && [ "$(printf "%s\n%s\n" "$2" "$4" | sort -V | tail -1)" = "$2" ] && [ "$2" != "$4" ]'
fake rpm 'case "$1" in
  -qf) [ "$FAKE_OWNER" = rpm ];;
  -qp) case "$3" in *NAME*) printf %s "$FAKE_NAME";; *VERSION*) printf %s "$FAKE_VERSION";; *ARCH*) printf %s "${FAKE_ARCH:-x86_64}";; esac;;
  -q) printf %s "$FAKE_INSTALLED";;
  --eval) v=$(echo "$2" | sed "s/.*vercmp(.\(.*\)., .\(.*\).)).*/\1 \2/"); set -- $v
          if [ "$1" = "$2" ]; then echo 0; elif [ "$(printf "%s\n%s\n" "$1" "$2" | sort -V | tail -1)" = "$1" ]; then echo 1; else echo -1; fi;;
esac'
fake apt-get 'echo "apt-get $*" >> "$FAKE_LOG"'
fake dnf 'echo "dnf $*" >> "$FAKE_LOG"'
fake sleep 'exec /bin/sleep 0.05'

# A release listing in GitHub's pretty-printed form, listing one file with its digest.
listing() { # name sha256
    cat > "$TMP/listing.json" <<JSON
{
  "tag_name": "v0.60.2",
  "assets": [
    {
      "id": 1,
      "name": "other.AppImage",
      "uploader": { "login": "x" },
      "size": 5,
      "digest": "sha256:$(printf '0%.0s' $(seq 64))"
    },
    {
      "id": 2,
      "name": "$1",
      "uploader": { "login": "x" },
      "size": 5,
      "digest": "sha256:$2"
    }
  ]
}
JSON
}

run() { # runs the helper with the fakes; args are the helper's. Output in $TMP/out, status in $?
    : > "$TMP/log"
    env -i HOME="$TMP" AGENTMUX_HELPER_TEST=1 AGENTMUX_HELPER_PATH="$BIN" AGENTMUX_QUIT_WAIT="${QUIT_WAIT:-2}" \
        AGENTMUX_CACHE_DIR="$TMP/cache" AGENTMUX_STATE_DIR="$TMP/state" AGENTMUX_INSTALL_ROOT="$TMP/opt" AGENTMUX_PROC_DIR="$TMP/proc" \
        AGENTMUX_DOWNLOAD_BASE=https://example.test/download FAKE_DOWNLOAD="$TMP/download" FAKE_LISTING="$TMP/listing.json" FAKE_LOG="$TMP/log" FAKE_OWNER="${FAKE_OWNER:-deb}" FAKE_INSTALLED="${FAKE_INSTALLED:-0.60.1}" \
        FAKE_NAME="${FAKE_NAME:-agentmux}" FAKE_VERSION="${FAKE_VERSION:-0.60.2}" FAKE_ARCH="${FAKE_ARCH:-}" \
        sh "$HELPER" "$@" > "$TMP/out" 2> "$TMP/err"
}
mkdir -p "$TMP/proc"

deb="$TMP/updates/AgentMux_0.60.2_amd64.deb"; rpm="$TMP/updates/AgentMux-0.60.2-1.x86_64.rpm"
mkdir -p "$TMP/updates"; printf 'deb bytes' > "$deb"; printf 'rpm bytes' > "$rpm"
sha() { sha256sum < "$1" | awk '{print $1}'; }
dead_pid=999999

listing "AgentMux_0.60.2_amd64.deb" "$(sha "$deb")"
FAKE_OWNER=deb run "$deb" 0.60.2 $dead_pid; st=$?
if [ $st = 0 ] && grep -q '^READY$' "$TMP/out" && grep -q 'apt-get install -y --no-install-recommends' "$TMP/log" && grep -q 'to=0.60.2' "$TMP/state/last-upgrade" && [ "$(stat -c %a "$TMP/state/last-upgrade")" = 644 ] && [ "$(stat -c %a "$TMP/state")" = 755 ]; then ok "a good .deb: checked, READY, installed through apt-get, recorded where the launcher can read it, not where anyone can write"; else bad "a good .deb" "status $st: $(cat "$TMP/out" "$TMP/err" "$TMP/log")"; fi

refused() { # name; expects a non-zero status, no READY and no install
    if [ "$st" != 0 ] && ! grep -q READY "$TMP/out" && [ ! -s "$TMP/log" ]; then ok "$1 ($(tail -1 "$TMP/err" | sed 's/agentmux-install-update: //'))"; else bad "$1" "status $st out=$(cat "$TMP/out") err=$(cat "$TMP/err") log=$(cat "$TMP/log")"; fi
}

printf x > "$TMP/updates/other.deb"
FAKE_OWNER=deb run "$TMP/updates/other.deb" 0.60.2 $dead_pid; st=$?; refused "a file that isn't the release's name"
ln -sf "$deb" "$TMP/updates/link"; cp "$deb" "$TMP/real.deb"; ln -sf "$TMP/real.deb" "$TMP/updates/AgentMux_0.60.3_amd64.deb"
FAKE_OWNER=deb run "$TMP/updates/AgentMux_0.60.3_amd64.deb" 0.60.3 $dead_pid; st=$?; refused "a symlink"
printf 'tampered' > "$TMP/updates/AgentMux_0.60.2_amd64.deb"
FAKE_OWNER=deb run "$TMP/updates/AgentMux_0.60.2_amd64.deb" 0.60.2 $dead_pid; st=$?; refused "a file that doesn't match GitHub's checksum"
printf 'deb bytes' > "$deb"
FAKE_OWNER=deb FAKE_VERSION=0.60.9 run "$deb" 0.60.2 $dead_pid; st=$?; refused "a package whose version isn't the one asked for"
FAKE_OWNER=deb FAKE_NAME=other run "$deb" 0.60.2 $dead_pid; st=$?; refused "a package that isn't agentmux"
FAKE_OWNER=deb FAKE_ARCH=arm64 run "$deb" 0.60.2 $dead_pid; st=$?; refused "a package for another architecture"
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 run "$deb" 0.60.2 $dead_pid; st=$?; refused "the version that is already installed"
FAKE_OWNER=deb FAKE_INSTALLED=0.61.0 run "$deb" 0.60.2 $dead_pid; st=$?; refused "an older version than the installed one"
FAKE_OWNER=rpm run "$deb" 0.60.2 $dead_pid; st=$?; refused "a .deb on an install owned by rpm"
run "$deb" 0.60.2 abc; st=$?; refused "a process id that isn't a number"
run "relative.deb" 0.60.2 $dead_pid; st=$?; refused "a relative path"
FAKE_OWNER=none run "$deb" 0.60.2 $dead_pid; st=$?; refused "an install neither dpkg nor rpm owns"
listing "AgentMux_0.60.9_amd64.deb" "$(sha "$deb")"
FAKE_OWNER=deb run "$deb" 0.60.2 $dead_pid; st=$?; refused "a release whose listing doesn't name the file"
listing "AgentMux_0.60.2_amd64.deb" "$(sha "$deb")"

# AgentMux is still running: from the launcher the helper was told about, or from the install by anyone.
/bin/sleep 30 & SLEEPER=$!
QUIT_WAIT=1 FAKE_OWNER=deb run "$deb" 0.60.2 $SLEEPER; st=$?
if [ $st != 0 ] && grep -q READY "$TMP/out" && [ ! -s "$TMP/log" ] && grep -q "didn't quit" "$TMP/err"; then ok "a launcher that doesn't quit in time: READY, then nothing installed"; else bad "the launcher never quits" "status $st: $(cat "$TMP/out" "$TMP/err" "$TMP/log")"; fi
kill $SLEEPER 2>/dev/null; SLEEPER=
mkdir -p "$TMP/proc/4242"; ln -sf "$TMP/opt/bin/agentmux-cef" "$TMP/proc/4242/exe"
QUIT_WAIT=1 FAKE_OWNER=deb run "$deb" 0.60.2 $dead_pid; st=$?
if [ $st != 0 ] && [ ! -s "$TMP/log" ] && grep -q "still running" "$TMP/err"; then ok "another user's AgentMux still running from the install blocks it"; else bad "a process still runs from the install" "status $st: $(cat "$TMP/out" "$TMP/err" "$TMP/log")"; fi
rm -rf "$TMP/proc/4242"

# An .rpm, through dnf.
listing "AgentMux-0.60.2-1.x86_64.rpm" "$(sha "$rpm")"
FAKE_OWNER=rpm run "$rpm" 0.60.2 $dead_pid; st=$?
if [ $st = 0 ] && grep -q '^READY$' "$TMP/out" && grep -q '^dnf install -y ' "$TMP/log"; then ok "a good .rpm: checked and installed through dnf"; else bad "a good .rpm" "status $st: $(cat "$TMP/out" "$TMP/err" "$TMP/log")"; fi
FAKE_OWNER=rpm FAKE_INSTALLED=0.60.2 run "$rpm" 0.60.2 $dead_pid; st=$?; refused "an .rpm that isn't newer (rpm's own comparison)"

# Going back: only to the version the last upgrade replaced, once, downloaded by the helper itself.
old_deb="$TMP/old.deb"; printf 'old deb bytes' > "$old_deb"; old_rpm="$TMP/old.rpm"; printf 'old rpm bytes' > "$old_rpm"
record() { mkdir -p "$TMP/state"; printf 'from=%s\nto=%s\n' "$1" "$2" > "$TMP/state/last-upgrade"; }
listing "AgentMux_0.60.1_amd64.deb" "$(sha "$old_deb")"; cp "$old_deb" "$TMP/download"
record 0.60.1 0.60.2
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1 $dead_pid; st=$?
if [ $st = 0 ] && grep -q '^READY$' "$TMP/out" && grep -q 'apt-get install -y --allow-downgrades' "$TMP/log" && [ ! -e "$TMP/state/last-upgrade" ]; then ok "going back (.deb): downloaded, checked, installed with a downgrade, record used up"; else bad "go back .deb" "status $st: $(cat "$TMP/out" "$TMP/err" "$TMP/log")"; fi
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1 $dead_pid; st=$?; refused "going back twice (the record is used up)"
record 0.60.0 0.60.2
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1 $dead_pid; st=$?; refused "going back to a version the last upgrade didn't replace"
record 0.60.1 0.60.2
FAKE_OWNER=deb FAKE_INSTALLED=0.60.3 FAKE_VERSION=0.60.1 run --go-back 0.60.1 $dead_pid; st=$?; refused "going back from a version the last upgrade didn't install"
printf 'tampered' > "$TMP/download"
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1 $dead_pid; st=$?; refused "a download that doesn't match GitHub's checksum"
cp "$old_deb" "$TMP/download"
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1 abc; st=$?; refused "going back with a process id that isn't a number"
FAKE_OWNER=deb FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1; st=$?; refused "going back without a process id"
listing "AgentMux-0.60.1-1.x86_64.rpm" "$(sha "$old_rpm")"; cp "$old_rpm" "$TMP/download"
FAKE_OWNER=rpm FAKE_INSTALLED=0.60.2 FAKE_VERSION=0.60.1 run --go-back 0.60.1 $dead_pid; st=$?
if [ $st = 0 ] && grep -q '^dnf downgrade -y ' "$TMP/log"; then ok "going back (.rpm): installed through dnf downgrade"; else bad "go back .rpm" "status $st: $(cat "$TMP/out" "$TMP/err" "$TMP/log")"; fi
[ -z "$(ls -A "$TMP/cache" 2>/dev/null)" ] && ok "the root-only work folder is removed after every run" || bad "the work folder is left behind" "$(ls "$TMP/cache")"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]

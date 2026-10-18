#!/usr/bin/env bash
# check-long-jump.sh — bring copies of an old AgentMux's data to this build's
# schema in one start, as a user who skipped many versions gets, and check
# the result (docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §3,
# "Skipping versions").
#
# Usage:
#   scripts/check-long-jump.sh <agentmux-srv> <name> <shared store.db | -> <channel db dir> [<name> <store.db | -> <dir>]...
#
#   <agentmux-srv>     the srv binary under test (cargo build -p agentmux-srv)
#   <name>             a label for the case
#   <shared store.db>  the old ~/.agentmux/shared/store.db, or - for none
#   <channel db dir>   a folder with the old objects.db (and filestore.db,
#                      sagas.db if there are any)
#
# Good sources on a machine that has run AgentMux for a while: the backups srv
# takes before every migration batch (~/.agentmux/shared/backups/
# pre-migration-<version>-<ts>/, a store.db and an objects.db) and the
# snapshots of older channel data (~/.agentmux/snapshots/*.bak/). They hold
# the user's own data: keep the copies, and this script's output, local.
#
# Each case runs in a temp home of its own. AGENTMUX_HOME_OVERRIDE and
# AGENTMUX_SETTINGS_DIR point srv there, and an `agents/` folder in it keeps
# the legacy-folder migration from reading the real home, so the real
# ~/.agentmux is never written. Each case prints the migrate run's summary and
# `migrate --verify`'s; exit status 1 when any case fails either.
set -u

if [ $# -lt 4 ] || [ $(( ($# - 1) % 3 )) -ne 0 ]; then
    sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
fi
SRV="$1"
shift
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
failed=0

while [ $# -gt 0 ]; do
    name="$1" shared="$2" channel="$3"
    shift 3
    home="$WORK/$name"
    data="$home/channels/stable/versions/9.9.9/data"
    mkdir -p "$home/shared" "$home/agents" "$home/settings" "$data/db"
    [ "$shared" != "-" ] && cp "$shared" "$home/shared/store.db"
    for f in objects.db filestore.db sagas.db; do
        [ -f "$channel/$f" ] && cp "$channel/$f" "$data/db/$f"
    done
    env=(AGENTMUX_HOME_OVERRIDE="$home" AGENTMUX_SETTINGS_DIR="$home/settings" AGENTMUX_CHANNEL=stable
        AGENTMUX_AUTH_KEY=long-jump AGENTMUX_DATA_HOME="$data")
    echo "=== $name"
    if env "${env[@]}" "$SRV" --wavedata "$data" migrate > "$home/migrate.log" 2>&1; then
        grep -o '"applied":[0-9]*,"skipped":[0-9]*' "$home/migrate.log" | tail -1 | sed 's/^/migrate: ok, /'
    else
        echo "migrate: FAILED"
        tail -5 "$home/migrate.log"
        failed=1
        continue
    fi
    env "${env[@]}" "$SRV" --wavedata "$data" migrate --verify > "$home/verify.log" 2>&1
    code=$?
    grep -E "^verify: [0-9]+ applied checked" "$home/verify.log" | sed 's/ (data dir.*//'
    grep -E "\[(MISMATCH|error)\]" "$home/verify.log"
    [ "$code" -ne 0 ] && failed=1
done
exit "$failed"

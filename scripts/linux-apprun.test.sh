#!/usr/bin/env bash
# linux-apprun.test.sh — tests for the extract-once cache in scripts/linux-apprun.sh.
#
# Covers two real defects:
#
# 1. The cache used to be keyed on VERSION alone. `task package` deliberately
#    does not bump the version, so every local build of a version shared one
#    extraction dir — and since the guard is "does a launcher already exist
#    there", the FIRST build extracted won permanently. Later builds silently
#    re-exec'd the older binary along with the per-build data-dir channel baked
#    into it. Reproduced with two local 0.56.3 builds: the second launch ran the
#    first's binary in the first's channel, so a fix that had just been built
#    was never actually exercised — and looked like "works in dev, broken in the
#    portable", indefinitely, across rebuilds.
#
# 2. Keying per build makes these dirs turn over on every package, which turns
#    the pre-existing "keep the 2 most recent" prune into a hazard: it would
#    delete the ~400MB tree a RUNNING instance is still lazily reading its
#    binaries, .pak files and libcef.so out of. Pruning must skip anything a
#    live process is executing from.
#
# Usage: bash scripts/linux-apprun.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APPRUN="$HERE/linux-apprun.sh"
TMP="$(mktemp -d)"
cleanup() { [ -n "${SLEEPER_PID:-}" ] && kill "$SLEEPER_PID" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
pass=0; fail=0

ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# ── 1. The cache key is the build, not the version ──────────────────────
# Asserted against the script's source: two AppDirs with the same VERSION but
# different BUILD_ID must not resolve to the same extraction dir.
# Resolve the key by sourcing the SCRIPT'S OWN lines, not a copy of them —
# an earlier revision of this test reimplemented the resolution here and
# therefore passed against the very code it was meant to reject.
KEYBLOCK="$TMP/keyblock.sh"
sed -n '/^VERSION="\$(cat "\$this_dir/,/^EXTRACT_DIR=/p' "$APPRUN" > "$KEYBLOCK"
[ -s "$KEYBLOCK" ] || { echo "  FAIL  could not extract key block from $APPRUN"; exit 1; }

key_for() { # $1 = version, $2 = build id ("" for none) -> prints EXTRACT_DIR
    local d="$TMP/appdir-$1-${2:-none}-$RANDOM/usr/share/agentmux"
    mkdir -p "$d"
    printf '%s' "$1" > "$d/VERSION"
    [ -n "$2" ] && printf '%s' "$2" > "$d/BUILD_ID"
    (
        set +u
        this_dir="${d%/usr/share/agentmux}"
        HOME="$TMP/home"
        # shellcheck disable=SC1090
        . "$KEYBLOCK"
        printf '%s' "$EXTRACT_DIR"
    )
}

a="$(key_for 0.56.3 '0.56.3+gAAAAAAA.20260917T010101.111')"
b="$(key_for 0.56.3 '0.56.3+gBBBBBBB.20260917T020202.222')"
if [ "$a" != "$b" ]; then
    ok "two builds of the same version extract to different dirs"
else
    bad "two builds of the same version extract to different dirs" "both resolved to '$a'"
fi

r="$(key_for 0.56.3 '')"
case "$r" in
    */0.56.3) ok "a release build with no BUILD_ID falls back to VERSION" ;;
    *) bad "a release build with no BUILD_ID falls back to VERSION" "got '$r'" ;;
esac

# The script must actually read BUILD_ID — guards against the fallback being
# silently dropped back to VERSION-only keying.
if grep -q 'BUILD_ID="\$(cat "\$this_dir/usr/share/agentmux/BUILD_ID"' "$APPRUN" \
   && grep -q 'CACHE_KEY="\${BUILD_ID:-\$VERSION}"' "$APPRUN"; then
    ok "linux-apprun.sh keys EXTRACT_DIR on BUILD_ID with a VERSION fallback"
else
    bad "linux-apprun.sh keys EXTRACT_DIR on BUILD_ID with a VERSION fallback" \
        "expected BUILD_ID read + CACHE_KEY fallback in $APPRUN"
fi

# ── 1b. An inherited AGENTMUX_EXTRACTED_RUN must not short-circuit ──────
# The running app exports this into its own terminal panes, so anything
# launched from a pane inherits it. As a bare "1" it made a freshly built
# AppImage skip extraction and run mounted, silently reusing nothing and
# checking nothing.
if grep -q 'AGENTMUX_EXTRACTED_RUN:-}" = "\$this_dir"' "$APPRUN"; then
    ok "the extracted-run marker is compared against this_dir, not truthiness"
else
    bad "the extracted-run marker is compared against this_dir, not truthiness" \
        "a bare =1 check is inherited by every child process"
fi
if grep -q 'export AGENTMUX_EXTRACTED_RUN="\$EXTRACT_DIR"' "$APPRUN"; then
    ok "the marker is exported as a path so a stale inherited one is ignored"
else
    bad "the marker is exported as a path so a stale inherited one is ignored" \
        "expected export AGENTMUX_EXTRACTED_RUN=\"\$EXTRACT_DIR\""
fi

# ── 2. Pruning must never delete a tree that is in use ───────────────────
# Extract the real prune block out of the script so this cannot drift.
PRUNE="$TMP/prune-block.sh"
awk '/^            \($/,/^            \) \|\| true$/' "$APPRUN" | sed 's/^            //' > "$PRUNE"
if [ ! -s "$PRUNE" ]; then
    bad "prune block extracted from linux-apprun.sh" "awk found no block"
else
    base="$TMP/extracted"
    mkdir -p "$base"/{build-OLDEST,build-INUSE,build-PREV,build-NEW}
    EXTRACT_DIR="$base/build-NEW"
    CACHE_KEY=build-NEW
    # Pre-fix block referenced $VERSION; define it so that revision RUNS
    # (and deletes build-INUSE) instead of aborting on an unbound variable
    # and passing this test for the wrong reason.
    VERSION=build-NEW

    # A genuinely live process whose *exe* is inside build-INUSE. Note bash is
    # used rather than `sleep`: coreutils' multi-call binary refuses to run
    # under another name, and `bash -c "sleep N"` exec's sleep and replaces
    # itself — both make the process's exe point somewhere else and the test
    # would pass for the wrong reason.
    cp /bin/bash "$base/build-INUSE/runner"
    "$base/build-INUSE/runner" -c "sleep 30; :" &
    SLEEPER_PID=$!
    sleep 0.3

    touch -d "-40 minutes" "$base/build-OLDEST"
    touch -d "-30 minutes" "$base/build-INUSE"
    touch -d "-10 minutes" "$base/build-PREV"
    touch "$base/build-NEW"

    # shellcheck disable=SC1090
    . "$PRUNE"

    [ -d "$base/build-INUSE" ] \
        && ok "a dir a live process is executing from survives pruning" \
        || bad "a dir a live process is executing from survives pruning" "build-INUSE was deleted"
    [ -d "$base/build-NEW" ] && [ -d "$base/build-PREV" ] \
        && ok "the two most recent dirs are kept" \
        || bad "the two most recent dirs are kept" "one of build-NEW/build-PREV was deleted"
    [ ! -d "$base/build-OLDEST" ] \
        && ok "a stale, unused dir is pruned" \
        || bad "a stale, unused dir is pruned" "build-OLDEST survived; cache would grow ~400MB per build"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]

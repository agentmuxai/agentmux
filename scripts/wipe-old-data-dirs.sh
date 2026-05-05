#!/usr/bin/env bash
# wipe-old-data-dirs.sh — torch legacy AgentMux data on this single-user dev machine.
#
# Per docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md §3.4 and §6,
# we don't migrate; we restart fresh. This script removes:
#
#   1. Tauri-era app data (ai.agentmux.app.*, com.a5af.agentmux*,
#      com.agentmuxhq.agentmux*) — completely dead since the CEF migration.
#   2. CEF-era data (ai.agentmux.cef.*) for any version that is NOT currently
#      running on this machine. Running instances are detected via tasklist.
#   3. ~/.agentmux/<old-version>/ subdirs that no current portable references.
#
# It does NOT touch:
#   - Anything under a running portable's path (Get-CimInstance lookup).
#   - The current source tree or build artifacts.
#
# Usage:
#   scripts/wipe-old-data-dirs.sh           # dry-run, lists what would be deleted
#   scripts/wipe-old-data-dirs.sh --yes     # actually delete
#
# Exit codes: 0 success, 1 args/preflight failure.

set -euo pipefail

# Empty globs should expand to nothing rather than the literal pattern.
# Without this, a glob with no matches loops once over the literal
# pattern, which then fails downstream (e.g. `du -sm <literal>` returns
# empty stdout, making `$((TOTAL + SIZE))` a parse error under set -e).
shopt -s nullglob

DRY_RUN=1
for arg in "$@"; do
    case "$arg" in
        --yes) DRY_RUN=0 ;;
        --help|-h)
            sed -n '2,/^$/p' "$0"
            exit 0
            ;;
        *)
            echo "unknown arg: $arg" >&2
            exit 1
            ;;
    esac
done

# ── Detect running paths to preserve ───────────────────────────────────────
# Two sources, both unioned:
#   (a) ExecutablePath parent dir — covers portable's <root>/data (since
#       portable srv lives in <root>/runtime/, parent dir = <root>).
#   (b) `--wavedata <path>` argument from any running srv command line —
#       covers installed mode, where the exe lives in Program Files but
#       the data dir is %LOCALAPPDATA%/ai.agentmux.cef.v<v>/ (Codex P1
#       on PR #694: without this, --yes would wipe an active installed
#       instance's profile).

# Codex P1 round-2 on PR #694: `|| true` would silently swallow any
# powershell failure (missing, restricted, broken), making the script
# treat that as "no running instances" and happily wipe live state.
# We require powershell discovery to succeed; fail fast if it doesn't.
PS_OUTPUT=$(
    powershell.exe -NoProfile -Command "
        \$procs = Get-CimInstance Win32_Process -Filter \"Name LIKE 'agentmux%'\"
        \$exeParents = \$procs | Select-Object -ExpandProperty ExecutablePath |
            Where-Object { \$_ } | ForEach-Object { Split-Path -Parent \$_ }
        \$dataDirs = \$procs | Select-Object -ExpandProperty CommandLine |
            Where-Object { \$_ -match '--wavedata\s+\"?([^\"]+?)\"?(\s|$)' } |
            ForEach-Object {
                if (\$_ -match '--wavedata\s+\"?([^\"]+?)\"?(\s|$)') { \$matches[1] }
            }
        @(\$exeParents + \$dataDirs) | Where-Object { \$_ } | Sort-Object -Unique
    "
) || {
    echo "ERROR: PowerShell process-discovery failed (exit $?)." >&2
    echo "  Without this we cannot tell which directories are in use." >&2
    echo "  Refusing to proceed — re-run after PowerShell is restored," >&2
    echo "  or set AGENTMUX_WIPE_FORCE=1 to override." >&2
    if [ "${AGENTMUX_WIPE_FORCE:-}" != "1" ]; then
        exit 1
    fi
    echo "  AGENTMUX_WIPE_FORCE=1 — proceeding with empty preserve list." >&2
}
# An empty stdout from a successful run means no agentmux processes
# are running — that's a valid state, not an error.
RUNNING_PATHS=$(echo "$PS_OUTPUT" | tr -d '\r' | grep -v '^$' || true)

echo "Running AgentMux folders (preserved):"
if [ -z "$RUNNING_PATHS" ]; then
    echo "  (none)"
else
    echo "$RUNNING_PATHS" | sed 's/^/  /'
fi
echo

# Helper: convert Windows path to MSYS for `rm`.
to_msys() {
    local p="$1"
    p="${p//\\//}"
    if [[ "$p" =~ ^[A-Za-z]: ]]; then
        # Drive letter → /c/...
        local drive="${p:0:1}"
        drive="${drive,,}"
        p="/$drive/${p:3}"
    fi
    echo "$p"
}

is_running_path() {
    local target="$1"
    local p
    while IFS= read -r p; do
        [ -z "$p" ] && continue
        local mp
        mp=$(to_msys "$p")
        # If target is the running path or a parent of it, preserve it.
        case "$mp/" in
            "$target"/*|"$target") return 0 ;;
        esac
    done <<< "$RUNNING_PATHS"
    return 1
}

# ── Build delete list ───────────────────────────────────────────────────────

DELETE_LIST=()

# Resolve home base in MSYS form. On Windows, $HOME is /c/Users/<name>;
# fall back to converting $USERPROFILE if HOME isn't set.
HOME_DIR="${HOME:-}"
if [ -z "$HOME_DIR" ] && [ -n "${USERPROFILE:-}" ]; then
    HOME_DIR=$(to_msys "$USERPROFILE")
fi
if [ -z "$HOME_DIR" ]; then
    echo "ERROR: cannot resolve home directory ($HOME and $USERPROFILE both unset)" >&2
    exit 1
fi
LOCALAPPDATA_DIR="$HOME_DIR/AppData/Local"
ROAMING_DIR="$HOME_DIR/AppData/Roaming"
DOTAGENTMUX_DIR="$HOME_DIR/.agentmux"

# 1. Tauri-era and bundle-id-era AppData
for d in \
    "$LOCALAPPDATA_DIR"/ai.agentmux.app.* \
    "$ROAMING_DIR"/ai.agentmux.app.* \
    "$LOCALAPPDATA_DIR"/com.a5af.agentmux* \
    "$ROAMING_DIR"/com.a5af.agentmux* \
    "$LOCALAPPDATA_DIR"/com.agentmuxhq.agentmux* \
    "$ROAMING_DIR"/com.agentmuxhq.agentmux* \
; do
    [ -e "$d" ] && DELETE_LIST+=("$d")
done

# 2. CEF-era data. Running installed instances are now correctly preserved
# via the --wavedata extraction in RUNNING_PATHS above, so this glob is
# safe to apply broadly — anything currently in use will be filtered out
# by is_running_path() below.
for d in \
    "$LOCALAPPDATA_DIR"/ai.agentmux.cef.v* \
    "$ROAMING_DIR"/ai.agentmux.cef.v* \
    "$LOCALAPPDATA_DIR"/ai.agentmux.cef.dev \
    "$ROAMING_DIR"/ai.agentmux.cef.dev \
; do
    [ -e "$d" ] && DELETE_LIST+=("$d")
done

# 3. ~/.agentmux/<version>/ subdirs (CLI shell config, agent workspaces).
# Per CLAUDE.md "Multiple Instances Run in Parallel" the running portables
# all write to <portable-root>/data, so ~/.agentmux/<v>/ is per-version
# CLI config that's only ever appended to. Safe to wipe — torch & restart.
if [ -d "$DOTAGENTMUX_DIR" ]; then
    for d in "$DOTAGENTMUX_DIR"/*/; do
        # Strip trailing slash for grep
        DELETE_LIST+=("${d%/}")
    done
fi

# ── Apply preserve filter ───────────────────────────────────────────────────

FILTERED=()
for d in "${DELETE_LIST[@]}"; do
    if is_running_path "$d"; then
        echo "PRESERVE (running): $d"
    else
        FILTERED+=("$d")
    fi
done

# ── Report + execute ────────────────────────────────────────────────────────

echo
echo "Delete list (${#FILTERED[@]} entries):"
TOTAL=0
for d in "${FILTERED[@]}"; do
    SIZE=$(du -sm "$d" 2>/dev/null | awk '{print $1}')
    SIZE="${SIZE:-0}"
    TOTAL=$((TOTAL + SIZE))
    printf "  %5dM  %s\n" "$SIZE" "$d"
done
echo
echo "Total: ${TOTAL}M (~$((TOTAL / 1024))G)"
echo

if [ "$DRY_RUN" = 1 ]; then
    echo "DRY RUN — re-run with --yes to actually delete."
    exit 0
fi

echo "Proceeding to delete…"
for d in "${FILTERED[@]}"; do
    rm -rf -- "$d" 2>&1 | head -5 || true
done
echo "Done."

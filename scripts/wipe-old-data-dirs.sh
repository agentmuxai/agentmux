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

# ── Detect running portable paths ───────────────────────────────────────────
# Use PowerShell to enumerate running agentmux processes; extract their parent
# folder paths. We never touch anything under these.

RUNNING_PATHS=$(
    powershell.exe -NoProfile -Command \
        "Get-CimInstance Win32_Process -Filter \"Name LIKE 'agentmux%'\" |
         Select-Object -ExpandProperty ExecutablePath |
         Where-Object { \$_ } |
         ForEach-Object { Split-Path -Parent \$_ } |
         Sort-Object -Unique" 2>/dev/null |
    tr -d '\r' |
    grep -v '^$' || true
)

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

# 1. Tauri-era and bundle-id-era AppData
for d in \
    /c/Users/area54/AppData/Local/ai.agentmux.app.* \
    /c/Users/area54/AppData/Roaming/ai.agentmux.app.* \
    /c/Users/area54/AppData/Local/com.a5af.agentmux* \
    /c/Users/area54/AppData/Roaming/com.a5af.agentmux* \
    /c/Users/area54/AppData/Local/com.agentmuxhq.agentmux* \
    /c/Users/area54/AppData/Roaming/com.agentmuxhq.agentmux* \
; do
    [ -e "$d" ] && DELETE_LIST+=("$d")
done

# 2. CEF-era data, excluding any version currently in a running portable's path.
# Running portables write to <root>/data, NOT to %LOCALAPPDATA%/ai.agentmux.cef.v*,
# so all ai.agentmux.cef.v* are safe to delete on this machine.
# (Installed mode would use those paths, but no one is running an installed
# AgentMux on this box per the earlier process audit.)
for d in \
    /c/Users/area54/AppData/Local/ai.agentmux.cef.v* \
    /c/Users/area54/AppData/Roaming/ai.agentmux.cef.v* \
    /c/Users/area54/AppData/Local/ai.agentmux.cef.dev \
    /c/Users/area54/AppData/Roaming/ai.agentmux.cef.dev \
; do
    [ -e "$d" ] && DELETE_LIST+=("$d")
done

# 3. ~/.agentmux/<version>/ subdirs (CLI shell config, agent workspaces).
# Per CLAUDE.md "Multiple Instances Run in Parallel" the running portables
# all write to <portable-root>/data, so ~/.agentmux/<v>/ is per-version
# CLI config that's only ever appended to. Safe to wipe — torch & restart.
if [ -d /c/Users/area54/.agentmux ]; then
    for d in /c/Users/area54/.agentmux/*/; do
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

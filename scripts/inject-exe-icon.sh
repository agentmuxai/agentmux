#!/usr/bin/env bash
# Set the AgentMux icon on a Windows host exe.
#
# Why: the Phase 3 Windows sandbox host (#1633) is CEF's bootstrap.exe (renamed
# to the host exe), which ships Chrome's icon — so Explorer / Task Manager /
# Alt-Tab show Chrome instead of AgentMux. (The running WINDOW icon is fixed
# separately at runtime via set_window_icon → WM_SETICON; this fixes the EXE
# FILE's own icon resource.) winres can't edit an already-built PE, so we rewrite
# the icon resource with electron's `rcedit`. Idempotent — safe to run on the
# non-sandbox raw bin too (it just re-sets the same icon).
#
# Usage: bash scripts/inject-exe-icon.sh <path-to-exe>
set -euo pipefail

EXE="${1:?usage: inject-exe-icon.sh <exe>}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ICO="$REPO_ROOT/agentmux-cef/resources/win/agentmux.ico"

[ -f "$EXE" ] || { echo "inject-exe-icon: exe not found: $EXE" >&2; exit 1; }
[ -f "$ICO" ] || { echo "inject-exe-icon: icon not found: $ICO" >&2; exit 1; }

# rcedit is a BUILD tool (not shipped) — cache it under the account-wide build
# tools dir, pinned by sha256.
RCEDIT_URL="https://github.com/electron/rcedit/releases/download/v2.0.0/rcedit-x64.exe"
RCEDIT_SHA="3e7801db1a5edbec91b49a24a094aad776cb4515488ea5a4ca2289c400eade2a"
CACHE_DIR="${AGENTMUX_BUILD_TOOLS:-$HOME/.agentmux/build-tools}"
RCEDIT="$CACHE_DIR/rcedit-x64.exe"
mkdir -p "$CACHE_DIR"

if [ ! -f "$RCEDIT" ] || [ "$(sha256sum "$RCEDIT" | cut -d' ' -f1)" != "$RCEDIT_SHA" ]; then
    echo "  [icon] downloading rcedit (v2.0.0)…"
    curl -fsSL "$RCEDIT_URL" -o "$RCEDIT"
    got="$(sha256sum "$RCEDIT" | cut -d' ' -f1)"
    if [ "$got" != "$RCEDIT_SHA" ]; then
        echo "inject-exe-icon: rcedit sha256 mismatch (expected $RCEDIT_SHA got $got)" >&2
        rm -f "$RCEDIT"
        exit 1
    fi
fi

# rcedit is a native Windows exe — feed it Windows paths.
win() { cygpath -w "$1" 2>/dev/null || echo "$1"; }
win_exe="$(win "$EXE")"
win_ico="$(win "$ICO")"

# rcedit can transiently fail with "Unable to commit changes" when a scanner
# (Defender / Search indexer) still holds a handle on the just-copied exe — the
# same handle race that bites CEF extraction. Retry, then degrade to a warning:
# a missed exe-file icon stamp must NOT break packaging (the runtime WINDOW icon
# is set independently at startup via set_window_icon → WM_SETICON).
err="$(mktemp)"
attempt=0
max=6
# Also stamp the version-info strings. The host exe is CEF's bootstrap.exe, whose
# FileDescription is "CEF Bootstrap application" — that is what Explorer's
# Properties, Task Manager's Details, and the TASKBAR right-click (jump list)
# header show. Overwrite them so the user sees "AgentMux" everywhere. Same rcedit
# call as the icon (one PE rewrite, one retry loop).
until "$RCEDIT" "$win_exe" --set-icon "$win_ico" \
    --set-version-string "FileDescription" "AgentMux" \
    --set-version-string "ProductName" "AgentMux" \
    --set-version-string "CompanyName" "AgentMux Corp" \
    --set-version-string "InternalName" "AgentMux" \
    --set-version-string "OriginalFilename" "agentmux.exe" 2>"$err"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge "$max" ]; then
        echo "  [icon] WARNING: rcedit failed after $max attempts on $(basename "$EXE") —" >&2
        echo "         shipping without the stamped exe-file icon (runtime window icon unaffected):" >&2
        sed 's/^/           /' "$err" >&2
        rm -f "$err"
        exit 0
    fi
    echo "  [icon] rcedit busy (attempt $attempt/$max) — retrying in 2s…" >&2
    sleep 2
done
rm -f "$err"
echo "  [icon] set AgentMux icon + version strings (FileDescription/ProductName=AgentMux) on $(basename "$EXE")"

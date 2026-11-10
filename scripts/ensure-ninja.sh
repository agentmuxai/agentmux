#!/bin/sh
# ensure-ninja.sh: make `ninja` usable for the CEF native build, or say how.
#
# cef-dll-sys builds CEF's C wrapper through CMake, and CMake needs a `ninja`
# it can actually start. Presence on PATH is not enough: WinGet installs Ninja
# as a "Links" alias, a symlink/reparse point that Git Bash lists but native
# programs cannot start, so `task dev` used to die deep inside cef-dll-sys with
# exit status 58 and no mention of ninja (Kanban #31442). Visual Studio ships a
# real ninja.exe that is not put on PATH.
#
# A CMake configure that already failed pinned the alias in
# target/<profile>/build/cef-dll-sys-*/out/build/CMakeCache.txt. CMake reuses
# that cached CMAKE_MAKE_PROGRAM, so fixing PATH alone changes nothing until
# the cache goes (Camper's finding on #31442). The default mode deletes exactly
# those caches whose make program no longer runs.
#
# Contract:
#   ensure-ninja.sh           -> stdout is empty when the ninja on PATH is fine,
#                                or one line: the directory holding a real ninja,
#                                for the caller to prepend to PATH.
#   ensure-ninja.sh --check   -> human-readable status (for check-toolchain.sh).
#                                Read-only: it never removes a cache.
#   exit 0 when a usable ninja is available (after the PATH fix), 1 with
#   instructions when none is.
#
# Env:
#   AGENTMUX_NINJA          an explicit ninja executable to use; it wins over
#                           the ninja on PATH and over every search location
#   AGENTMUX_NINJA_SEARCH   colon-separated directories to search for
#                           ninja.exe, replacing the built-in WinGet/Visual
#                           Studio/Scoop/Chocolatey locations (unusual
#                           installs; also keeps tests host-independent)
#   AGENTMUX_TARGET_DIR     the cargo target dir to inspect for stale
#                           cef-dll-sys CMake caches (default: <repo>/target)
set -u

# No `dirname`: this runs before any tool check, and a PATH that holds only the
# tools being tested must not turn into a "dirname: not found" warning.
case "$0" in
    */*) SCRIPT_DIR=${0%/*} ;;
    *) SCRIPT_DIR=. ;;
esac
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd -P)

CHECK=0
case "${1:-}" in
    --check) CHECK=1 ;;
    "") ;;
    *)
        echo "usage: $0 [--check]" >&2
        exit 2
        ;;
esac

# A usable ninja starts and prints its version. Anything else (missing, a
# broken alias, a file that is not ninja) is not usable.
usable() {
    [ -n "${1:-}" ] || return 1
    [ -f "$1" ] || return 1
    "$1" --version >/dev/null 2>&1
}

version_of() {
    "$1" --version 2>/dev/null
}

case "$(uname -s 2>/dev/null || echo unknown)" in
    MINGW* | MSYS* | CYGWIN*) IS_WINDOWS=1 ;;
    *) IS_WINDOWS=0 ;;
esac

PATH_NINJA="$(command -v ninja 2>/dev/null || true)"
PATH_OK=0
PATH_ALIAS=0
if usable "$PATH_NINJA"; then PATH_OK=1; fi
if [ -n "$PATH_NINJA" ] && [ -L "$PATH_NINJA" ]; then PATH_ALIAS=1; fi

# Remove a CMakeCache.txt that pinned a make program which cannot run. CMake
# keeps CMAKE_MAKE_PROGRAM in the cache and reuses it on the next configure,
# so a fixed PATH alone would still fail. Only affected caches are touched.
invalidate_stale_cef_caches() {
    target_dir="${AGENTMUX_TARGET_DIR:-$REPO_ROOT/target}"
    [ -d "$target_dir" ] || return 0
    find "$target_dir" -type f -path '*cef-dll-sys*/out/build/CMakeCache.txt' 2>/dev/null |
    while IFS= read -r cache; do
        prog=$(sed -n 's/^CMAKE_MAKE_PROGRAM:FILEPATH=//p' "$cache" 2>/dev/null | head -1)
        [ -n "$prog" ] || continue
        prog_posix=$(printf '%s' "$prog" | tr '\\' '/')
        # On Windows a symlink is the WinGet "Links" alias: Git Bash can run
        # it, so `usable` says yes, but native CMake cannot start it. A cache
        # that pinned the alias is stale even though it answers --version.
        if usable "$prog_posix" && { [ "$IS_WINDOWS" -eq 0 ] || [ ! -L "$prog_posix" ]; }; then
            continue
        fi
        rm -f "$cache"
        printf '[ensure-ninja] removed stale CMake cache (CMAKE_MAKE_PROGRAM=%s cannot run): %s\n' "$prog" "$cache" >&2
    done
}

REAL=""

# The first usable path that is not itself an alias.
first_usable() {
    for p in "$@"; do
        [ -n "$p" ] || continue
        [ -f "$p" ] || continue
        [ -L "$p" ] && continue
        if usable "$p"; then
            # Callers prepend ${REAL%/*} to PATH, which needs forward slashes.
            REAL=$(printf '%s' "$p" | tr '\\' '/')
            return 0
        fi
    done
    return 1
}

# AGENTMUX_NINJA is an explicit override and the strongest signal: it is
# honoured on every OS, before following a PATH alias and before the
# AGENTMUX_NINJA_SEARCH locations. If it is set but does not start, say so
# rather than silently using a different ninja.
if [ -n "${AGENTMUX_NINJA:-}" ]; then
    if usable "$AGENTMUX_NINJA"; then
        # Git Bash accepts a backslash path, but ${REAL%/*} needs forward
        # slashes to find the directory.
        REAL=$(printf '%s' "$AGENTMUX_NINJA" | tr '\\' '/')
    else
        printf '[ensure-ninja] warning: AGENTMUX_NINJA=%s is not usable; ignoring it\n' "$AGENTMUX_NINJA" >&2
    fi
fi

# A ninja on PATH that starts and is not an alias is exactly what the build
# wants. Leave it where it is: searching for and prepending a different ninja
# would contradict "keeps a working PATH ninja as-is".
if [ -z "$REAL" ] && [ "$PATH_OK" -eq 1 ] && { [ "$IS_WINDOWS" -eq 0 ] || [ "$PATH_ALIAS" -eq 0 ]; }; then
    if [ "$CHECK" -eq 1 ]; then
        echo "  OK   ninja — $(version_of "$PATH_NINJA")"
    else
        invalidate_stale_cef_caches
    fi
    exit 0
fi

# AGENTMUX_NINJA_SEARCH replaces the built-in locations when set, so a test or
# an unusual install can say exactly where to look.
if [ -z "$REAL" ] && [ -n "${AGENTMUX_NINJA_SEARCH:-}" ]; then
    OLD_IFS=$IFS
    IFS=:
    set -- $AGENTMUX_NINJA_SEARCH
    IFS=$OLD_IFS
    for dir in "$@"; do
        [ -n "$dir" ] || continue
        first_usable "$dir/ninja.exe" "$dir/ninja" "$dir" && break
    done
elif [ -z "$REAL" ] && [ "$IS_WINDOWS" -eq 1 ]; then
    LOCAL_HOME="${HOME:-${USERPROFILE:-}}"
    LOCAL_APPDATA="${LOCALAPPDATA:-}"
    # Git Bash reports Windows-style env vars; only use one that is a POSIX path.
    case "$LOCAL_APPDATA" in
        /*) ;;
        *) LOCAL_APPDATA="" ;;
    esac

    # The alias's own target first: it is the same binary, addressed directly.
    if [ -n "$PATH_NINJA" ] && [ "$PATH_ALIAS" -eq 1 ] && command -v readlink >/dev/null 2>&1; then
        TARGET="$(readlink -f "$PATH_NINJA" 2>/dev/null || true)"
        if [ -n "$TARGET" ] && usable "$TARGET"; then REAL="$TARGET"; fi
    fi

    if [ -z "$REAL" ]; then
        first_usable \
            "$LOCAL_HOME"/AppData/Local/Microsoft/WinGet/Packages/Ninja-build.Ninja_*/ninja.exe \
            "$LOCAL_APPDATA"/Microsoft/WinGet/Packages/Ninja-build.Ninja_*/ninja.exe \
            "/c/Program Files/Microsoft Visual Studio/"*/*/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja/ninja.exe \
            "/c/Program Files (x86)/Microsoft Visual Studio/"*/*/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja/ninja.exe \
            "$LOCAL_HOME"/scoop/apps/ninja/current/ninja.exe \
            "$LOCAL_HOME"/scoop/shims/ninja.exe \
            "/c/ProgramData/chocolatey/bin/ninja.exe" || true
    fi
fi

if [ -n "$REAL" ]; then
    if [ "$CHECK" -eq 1 ]; then
        echo "  OK   ninja — $REAL ($(version_of "$REAL"))"
        if [ -n "$PATH_NINJA" ] && [ "$PATH_ALIAS" -eq 1 ]; then
            echo "           the ninja on PATH ($PATH_NINJA) is a symlink alias; task dev will use the real one"
        fi
    else
        invalidate_stale_cef_caches
        # The directory, for the caller to prepend to PATH.
        echo "${REAL%/*}"
    fi
    exit 0
fi

# A Windows alias that starts (unusual, but possible) is better than nothing:
# keep it rather than breaking a setup that works today.
if [ "$PATH_OK" -eq 1 ]; then
    if [ "$CHECK" -eq 1 ]; then
        echo "  OK   ninja — $(version_of "$PATH_NINJA")"
        echo "           note: $PATH_NINJA is a symlink; if the CEF build cannot start it, put a real ninja on PATH (see BUILD.md)"
    else
        invalidate_stale_cef_caches
    fi
    exit 0
fi

if [ -n "$PATH_NINJA" ]; then
    if [ "$CHECK" -eq 1 ]; then
        echo "  UNUSABLE  ninja — '$PATH_NINJA --version' failed"
    fi
else
    if [ "$CHECK" -eq 1 ]; then
        echo "  MISSING  ninja — not on PATH"
    fi
fi

{
    echo
    echo "The CEF native build (cef-dll-sys -> CMake) needs a ninja that starts."
    if [ -n "$PATH_NINJA" ]; then
        echo "  on PATH: $PATH_NINJA"
        echo "  'ninja --version' failed there. On Windows this is usually the WinGet"
        echo "  'Links' alias: a symlink Git Bash lists but native programs cannot start."
    fi
    echo
    if [ "$IS_WINDOWS" -eq 1 ]; then
        echo "Put a real ninja on PATH. Visual Studio ships one (use BuildTools, or"
        echo "Community/Professional/Enterprise if that is what you installed):"
        echo '  cp "/c/Program Files (x86)/Microsoft Visual Studio/"*"/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja/ninja.exe" /c/tools/bin/'
        echo
        echo "Or point AGENTMUX_NINJA at an existing ninja.exe."
    else
        echo "Install ninja:  macOS: brew install ninja    Linux: sudo apt install ninja-build"
    fi
    echo "See BUILD.md, Prerequisites, 'Ninja must be on PATH'."
} >&2
exit 1

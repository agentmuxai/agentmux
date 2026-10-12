#!/usr/bin/env bash
# Verify a macOS "Chromium Embedded Framework.framework" carries AgentMux's
# BeginWindowDrag patch and the agentmuxai/cef#11 ThemeService fix.
#
# Why
# ---
# Native window drag / floating-pane edge-resize on macOS calls
# CefWindow::BeginWindowDrag() via raw FFI (crates/cef/src/ui_tasks.rs). That
# struct slot exists ONLY in a framework built from the agentmuxai/cef fork
# (branch agentmux/7778-drag-rightclick-and-transparency). Bundle the upstream
# prebuilt CEF (the cef-dll-sys cargo cache) instead and the native drag path
# silently no-ops — which is exactly why the macOS floater drag/resize has had to
# fall back to JS-polled get/set_window_position workarounds. This is the build/
# release-time guard that catches a regressed (unpatched) framework before it ships
# in a DMG.
#
# Detection
# ---------
# The patch compiles in the symbol `CefWindowImpl::BeginWindowDrag`. On macOS the
# mangled name is `__ZN13CefWindowImpl15BeginWindowDragEv` and it is emitted as a
# LOCAL symbol (nm type `t`, lowercase) — NOT exported. So `nm -gU` (external/
# defined-only) will MISS it; we must read the FULL symbol table with plain `nm`.
# Local symbols live in the unstripped build output and are GONE after `strip`, so
# this check is only meaningful on the UNSTRIPPED framework (the cef-build tree, or
# the freshly-downloaded release asset), BEFORE package-macos.sh strips it.
#
# The cef#11 fix (CefWidgetImpl observes each ThemeService once) adds no named
# function, but its `observed_theme_services_` map instantiates std::map code
# keyed by raw_ptr<ThemeService>, also emitted as local symbols. No other code
# in the framework has such a map (0 matches on the 154 build without the fix,
# 2 with it). Without the fix, a browser pane whose Profile differs from its
# window's (any default-jar pane in a second window, including a torn-off tab)
# aborts the host: "Observers can only be added once!". A framework built from
# the fork before 2026-10-10 has BeginWindowDrag but not this, so checking
# BeginWindowDrag alone passed it.
#
# Usage:  verify-cef-framework-darwin.sh <dir-containing-framework | framework path | framework binary>
#   Accepts any of:
#     - a directory that contains "Chromium Embedded Framework.framework"
#     - the "…/Chromium Embedded Framework.framework" bundle itself
#     - the inner Mach-O binary directly
#
# Exit:   0  both present
#         1  symbol table present but a patch missing: no BeginWindowDrag slot
#            (the upstream prebuilt CEF; native drag will silently no-op), or
#            no cef#11 (a fork build older than it; second-window panes crash)
#         2  cannot verify — stripped binary, missing file, or no nm available
#
# Tests: scripts/verify-cef-framework-darwin.test.sh (fake nm, runs anywhere).
#
# NOTE: no `set -o pipefail` — `grep -q` short-circuits on first match and SIGPIPEs
# the symbol reader, which pipefail would surface as a spurious pipeline failure.
set -eu

FW_NAME="Chromium Embedded Framework.framework"
BIN_NAME="Chromium Embedded Framework"

arg="${1:-}"
if [ -z "$arg" ]; then
    echo "usage: verify-cef-framework-darwin.sh <dir | framework | binary>" >&2
    exit 2
fi

# Resolve <arg> to the inner Mach-O binary. Three accepted shapes:
#   1. a directory holding the .framework  → <arg>/<FW_NAME>/<BIN_NAME>
#   2. the .framework bundle               → <arg>/<BIN_NAME>
#   3. the Mach-O binary itself            → <arg>
if [ -d "$arg/$FW_NAME" ]; then
    BIN="$arg/$FW_NAME/$BIN_NAME"
elif [ -d "$arg" ] && [ "$(basename "$arg")" = "$FW_NAME" ]; then
    BIN="$arg/$BIN_NAME"
else
    BIN="$arg"
fi

# The framework binary is a Versions/Current symlink chain; nm follows it.
if [ ! -e "$BIN" ]; then
    echo "verify-cef-framework-darwin: framework binary not found at $BIN" >&2
    exit 2
fi

if ! command -v nm >/dev/null 2>&1; then
    echo "verify-cef-framework-darwin: nm not available — cannot verify $BIN" >&2
    exit 2
fi

DRAG_MARKER='BeginWindowDrag'
THEME_MARKER='__value_typeIN4base7raw_ptrI12ThemeService'

# Plain nm = full symbol table (local + external). REQUIRED: the patch symbols
# are local symbols; nm -gU would not see them. -p (symbol-table order, no sort)
# and a fixed-string grep that stops at the first match: about 1 s per marker on
# the ~1.2M-symbol unstripped framework, against ~5 s for one sorted nm.
has_sym() { nm -p "$BIN" 2>/dev/null | LC_ALL=C grep -qF -- "$1" && echo 1 || echo 0; }
has_drag="$(has_sym "$DRAG_MARKER")"
has_theme="$(has_sym "$THEME_MARKER")"

if [ "$has_drag" = 1 ] && [ "$has_theme" = 1 ]; then
    echo "verify-cef-framework-darwin: ✓ $BIN carries the BeginWindowDrag patch and the cef#11 ThemeService fix" >&2
    exit 0
fi

# A patch is missing. Distinguish "has symbols but unpatched" from "stripped".
#
# `strip` on macOS removes LOCAL symbols and keeps the external/dynamic ones, so
# a stripped framework still prints thousands of nm lines. Testing for "any
# output at all" therefore misidentifies every stripped binary as UNPATCHED:
# measured on a correctly-patched 148.23.25 build, the bundled framework has
# 4354 symbols and 0 local ones, against 1232824 / 1228470 unstripped. Before
# this fix, running the script against a shipped .app printed "This is the
# UNPATCHED upstream CEF" and exited 0 -- a confident, alarming, wrong answer
# about a correct build.
#
# The patch symbols are local, so the real question is whether ANY local symbols
# survive. nm marks local symbols with a lowercase type letter.
if nm -p "$BIN" 2>/dev/null | LC_ALL=C grep -qE '^[0-9a-f]+ [a-z] '; then
    if [ "$has_drag" != 1 ]; then
        echo "verify-cef-framework-darwin: ✗ $BIN has a symbol table but NO BeginWindowDrag slot." >&2
        echo "                  This is the UNPATCHED upstream CEF — native window drag /" >&2
        echo "                  floating-pane resize will silently no-op." >&2
    fi
    if [ "$has_theme" != 1 ]; then
        echo "verify-cef-framework-darwin: ✗ $BIN lacks the agentmuxai/cef#11 ThemeService fix." >&2
        echo "                  A browser pane in a second window (or a torn-off tab holding" >&2
        echo "                  one) aborts the host: \"Observers can only be added once!\"." >&2
        echo "                  A fork build from before 2026-10-10 looks like this." >&2
    fi
    echo "                  Use the pinned runtime (CEF_MACOS_RUNTIME_TAG in" >&2
    echo "                  scripts/cef-build/cef-runtime-pins.sh, an agentmuxai/cef release)" >&2
    echo "                  or a framework built from the fork at 6fba204 or later" >&2
    echo "                  (docs/cef-build/build-patched-framework-macos.md): stage it at" >&2
    echo "                  ~/cef-build/darwin/aarch64 or point AGENTMUX_CEF_RUNTIME_DIR_DARWIN at it." >&2
    exit 1
fi

echo "verify-cef-framework-darwin: ? $BIN is STRIPPED (no local symbols) — cannot verify" >&2
echo "                  the BeginWindowDrag patch by symbol. This is expected for a framework" >&2
echo "                  bundled into a .app: package-macos.sh strips at bundle time." >&2
echo "" >&2
echo "                  Verify the UNSTRIPPED framework instead (the cef-build tree, or a" >&2
echo "                  freshly-extracted release asset) — the patch check must precede the" >&2
echo "                  packaging strip." >&2
echo "" >&2
echo "                  To check a stripped framework against a known-good unstripped one," >&2
echo "                  compare executable code directly — stripping does not touch it:" >&2
echo "" >&2
echo "                    otool -s __TEXT __text <binary> | tail -n +3 | shasum" >&2
echo "" >&2
echo "                  Equal hashes prove identical code. Confirmed on the 148.23.25 build:" >&2
echo "                  the bundled and build-tree binaries hash the same despite differing" >&2
echo "                  by 195 MB of symbol table." >&2
exit 2

#!/usr/bin/env bash
# Verify a macOS "Chromium Embedded Framework.framework" carries AgentMux's
# BeginWindowDrag patch.
#
# Why
# ---
# Native window drag / floating-pane edge-resize on macOS calls
# CefWindow::BeginWindowDrag() via raw FFI (agentmux-cef/src/ui_tasks.rs). That
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
# Usage:  verify-cef-framework-darwin.sh <dir-containing-framework | framework path | framework binary>
#   Accepts any of:
#     - a directory that contains "Chromium Embedded Framework.framework"
#     - the "…/Chromium Embedded Framework.framework" bundle itself
#     - the inner Mach-O binary directly
#
# Exit:   0  patch present (symbol found)
#         1  UNPATCHED — symbol table present but no BeginWindowDrag slot (the
#            upstream prebuilt CEF; native drag will silently no-op)
#         2  cannot verify — stripped binary, missing file, or no nm available
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

PATTERN='BeginWindowDrag'

# Plain nm = full symbol table (local + external). REQUIRED: the patch symbol is
# a local symbol; nm -gU would not see it.
read_syms() { nm "$BIN" 2>/dev/null; }

if read_syms | grep -qE "$PATTERN"; then
    echo "verify-cef-framework-darwin: ✓ $BIN carries the BeginWindowDrag patch" >&2
    exit 0
fi

# No match. Distinguish "has symbols but unpatched" from "stripped (no symtab)".
if read_syms | grep -q .; then
    echo "verify-cef-framework-darwin: ✗ $BIN has a symbol table but NO BeginWindowDrag slot." >&2
    echo "                  This is the UNPATCHED upstream CEF — native window drag /" >&2
    echo "                  floating-pane resize will silently no-op. Use the patched" >&2
    echo "                  framework (docs/cef-build/build-patched-framework-macos.md), or" >&2
    echo "                  point AGENTMUX_CEF_RUNTIME_DIR_DARWIN at a patched Release_GN_arm64." >&2
    exit 1
fi

echo "verify-cef-framework-darwin: ? $BIN is stripped (no symbol table) — cannot verify" >&2
echo "                  the BeginWindowDrag patch by symbol. Run the check on the UNSTRIPPED" >&2
echo "                  framework (the patch check must precede the packaging strip)." >&2
exit 2

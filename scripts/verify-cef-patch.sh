#!/usr/bin/env bash
# Verify a Linux libcef.so carries AgentMux's BeginWindowDrag patch.
#
# Why
# ---
# Left-click window drag on Linux calls CefWindow::BeginWindowDrag() via raw FFI
# (agentmux-cef/src/ui_tasks.rs). That struct slot exists ONLY in a libcef.so built
# from the a5af/cef fork (branch agentmux/7680-…). Bundle the upstream prebuilt CEF
# instead and drag silently no-ops — the runtime ABI guard catches it, but only
# after the user clicks the title bar and nothing happens. This is the build/
# release-time guard that catches a regressed (unpatched) runtime before shipping.
#
# Detection
# ---------
# The patch compiles in the symbols `CefWindowImpl::BeginWindowDrag` and the CppToC
# wrapper `window_begin_window_drag_<apiver>`. Their (mangled) names contain the
# literal strings "BeginWindowDrag" / "begin_window_drag" and live in `.symtab` —
# present in the UNSTRIPPED build output, GONE after `strip --strip-debug/-all`.
# So this check is only meaningful on the unstripped runtime (the cef-build tree, or
# dist/cef/libcef.so freshly copied by `task bundle:linux`), BEFORE packaging strips
# it. A stripped .so has no symbol table and cannot be verified this way (exit 2).
#
# Usage:  verify-cef-patch.sh <runtime-dir | libcef.so path>
# Exit:   0  patch present (symbol found)
#         1  UNPATCHED — symbol table present but no BeginWindowDrag slot (the
#            upstream prebuilt CEF; drag will silently no-op)
#         2  cannot verify — stripped .so, missing file, or no nm/readelf available
#
# NOTE: no `set -o pipefail` — `grep -q` short-circuits on first match and SIGPIPEs
# the symbol reader, which pipefail would surface as a spurious pipeline failure.
set -eu

arg="${1:-}"
if [ -z "$arg" ]; then
    echo "usage: verify-cef-patch.sh <runtime-dir | libcef.so>" >&2
    exit 2
fi
if [ -d "$arg" ]; then SO="$arg/libcef.so"; else SO="$arg"; fi
if [ ! -f "$SO" ]; then
    echo "verify-cef-patch: libcef.so not found at $SO" >&2
    exit 2
fi

PATTERN='BeginWindowDrag|begin_window_drag'

# Pick a symbol reader: nm preferred, readelf -sW as fallback. Both emit the
# (mangled) .symtab names that still contain the literal patch symbol.
if command -v nm >/dev/null 2>&1; then
    read_syms() { nm "$SO" 2>/dev/null; }
elif command -v readelf >/dev/null 2>&1; then
    read_syms() { readelf -sW "$SO" 2>/dev/null; }
else
    echo "verify-cef-patch: neither nm nor readelf available — cannot verify $SO" >&2
    exit 2
fi

if read_syms | grep -qE "$PATTERN"; then
    echo "verify-cef-patch: ✓ $SO carries the BeginWindowDrag patch" >&2
    exit 0
fi

# No match. Distinguish "has symbols but unpatched" from "stripped (no symtab)".
if read_syms | grep -q .; then
    echo "verify-cef-patch: ✗ $SO has a symbol table but NO BeginWindowDrag slot." >&2
    echo "                  This is the UNPATCHED upstream CEF — left-click window drag" >&2
    echo "                  will silently no-op. Build the patched libcef" >&2
    echo "                  (docs/cef-build/build-patched-libcef.md), or point" >&2
    echo "                  AGENTMUX_CEF_RUNTIME_DIR at a patched Release_GN_x64." >&2
    exit 1
fi

echo "verify-cef-patch: ? $SO is stripped (no symbol table) — cannot verify the" >&2
echo "                  BeginWindowDrag patch by symbol. Run the check on the UNSTRIPPED" >&2
echo "                  build output (the patch check must precede the packaging strip)." >&2
exit 2

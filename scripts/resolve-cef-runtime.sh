#!/usr/bin/env bash
# Print the absolute path of the directory that holds libcef.so + the CEF
# runtime files (paks, snapshot data, GL libs, locales, etc.) that the
# AgentMux build pipeline should bundle.
#
# Why this script exists
# ----------------------
# Linux AgentMux needs a libcef.so built from `agentmux/7680-drag-rightclick-and-transparency`
# (a5af/cef fork) — it adds `CefWindow::BeginWindowDrag()` which AgentMux PR #663 calls
# via raw FFI for left-click window drag on Wayland. The cef-dll-sys cargo cache holds
# the upstream prebuilt libcef.so, which lacks that slot — when bundled, runtime falls
# back to no-op (drag silently broken). We need a way to inject the patched libcef.so
# into `task bundle:linux` without forking cef-dll-sys.
#
# Resolution order
# ----------------
#   1. $AGENTMUX_CEF_RUNTIME_DIR — explicit override (when set + non-empty + readable).
#   2. $HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64 — the standard
#      cef-build layout documented in docs/cef-build/build-patched-libcef.md.
#   3. Cargo cef-dll-sys cache: the first match of
#      <repo>/target/{debug,release}/build/cef-dll-sys-*/out/cef_linux_x86_64.
#
# Each candidate is validated by checking for libcef.so + icudtl.dat (necessary
# minimum for a usable CEF runtime).
#
# Sanity check (warning, not failure)
# -----------------------------------
# If the chosen libcef.so is suspiciously large (>1 GB), it is almost certainly the
# unstripped upstream debug build from cef-dll-sys, which means it lacks our
# BeginWindowDrag patch. The runtime ABI guard in agentmux-cef will catch this and
# log a warning at runtime, but we surface the issue at build time too so it's
# obvious before the user clicks the binary and finds drag silently broken.
#
# Output
# ------
# stdout: absolute path of the chosen directory.
# stderr: progress info + the size warning (if applicable) + actionable errors.
# exit 0 on success, 1 if no candidate had libcef.so + icudtl.dat.
#
# Used by Taskfile.yml::bundle:linux. See docs/specs/patched-libcef-bundling-2026-05-08.md
# for the full design.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

candidates=()

# 1. Explicit override.
if [ -n "${AGENTMUX_CEF_RUNTIME_DIR:-}" ]; then
    candidates+=("$AGENTMUX_CEF_RUNTIME_DIR")
fi

# 2. Standard cef-build layout under $HOME.
candidates+=("$HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64")

# 3. Cargo cef-dll-sys cache. Find first match in either debug or release.
while IFS= read -r d; do
    candidates+=("$d")
done < <(find "$REPO_ROOT/target" -maxdepth 6 -type d -name 'cef_linux_x86_64' 2>/dev/null)

for dir in "${candidates[@]}"; do
    if [ -f "$dir/libcef.so" ] && [ -f "$dir/icudtl.dat" ]; then
        # Heuristic: warn on >1 GB libcef (likely unpatched upstream debug build).
        size_bytes=$(stat -c %s "$dir/libcef.so" 2>/dev/null || stat -f %z "$dir/libcef.so" 2>/dev/null || echo 0)
        size_mb=$((size_bytes / 1024 / 1024))
        if [ "$size_bytes" -gt 1073741824 ]; then
            echo "WARNING: libcef.so at $dir is ${size_mb} MB (>1 GB) — likely unpatched upstream debug build." >&2
            echo "         AgentMux's BeginWindowDrag FFI override will silently no-op (left-click drag broken)." >&2
            echo "         Build the patched libcef per docs/cef-build/build-patched-libcef.md, then either" >&2
            echo "         (a) move the result to ~/cef-build/chromium_git/chromium/src/out/Release_GN_x64, or" >&2
            echo "         (b) export AGENTMUX_CEF_RUNTIME_DIR=/path/to/your/Release_GN_x64." >&2
        fi
        echo "$dir"
        exit 0
    fi
done

echo "ERROR: could not find libcef.so + icudtl.dat in any of these locations:" >&2
for c in "${candidates[@]}"; do echo "  - $c" >&2; done
echo "Build the patched libcef (see docs/cef-build/build-patched-libcef.md) or run \`cargo build -p agentmux-cef\` to populate the cef-dll-sys fallback cache." >&2
exit 1

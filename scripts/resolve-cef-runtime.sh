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
# Diagnostics (warning vs. info, never failure)
# ---------------------------------------------
# The real risk is resolving to the cef-dll-sys cargo cache: that's the upstream
# prebuilt CEF, which lacks our BeginWindowDrag patch regardless of file size, so
# left-click window drag silently no-ops. We emit a WARNING whenever we fall through
# to that candidate. The runtime ABI guard in agentmux-cef also catches it at
# runtime, but surfacing it at build time makes it obvious before the user clicks.
#
# Size is NOT a reliable patched/unpatched signal: the patched dev build is ~1.5 GB
# UNSTRIPPED (the AppImage packager strips it to ~260 MB at bundle time), so on the
# cef-build/override trees a >1 GB libcef.so is expected — we emit a soft INFO there,
# not the unpatched-provenance alarm.
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

# `validate_dir <dir>` — print to stdout and exit 0 if `<dir>` has libcef.so +
# icudtl.dat. Returns 1 (no output) if either file is missing.
# When the directory provides both files, emit a diagnostic keyed on <kind>:
#   cargo-cache → the cef-dll-sys prebuilt download. It NEVER carries AgentMux's
#                 BeginWindowDrag patch (regardless of size), so resolving here
#                 means left-click window drag silently no-ops — the real alarm,
#                 and it's about provenance, not file size.
#   override | cef-build → an operator/dev-built tree. A >1 GB libcef.so here is
#                 just an unstripped build (the dev tree keeps symbols); the
#                 AppImage packager strips it to ~260 MB at bundle time. That is
#                 NORMAL — emit at most a soft INFO, never the upstream alarm.
#                 (Pre-fix this size check false-alarmed "likely unpatched upstream"
#                 on the correct official build — see the v0.45.0 slim-libcef work.)
validate_dir() {
    local dir="$1" kind="${2:-cef-build}"
    if [ ! -f "$dir/libcef.so" ] || [ ! -f "$dir/icudtl.dat" ]; then
        return 1
    fi
    if [ "$kind" = "cargo-cache" ]; then
        echo "WARNING: resolved libcef.so to the cef-dll-sys cargo cache at $dir." >&2
        echo "         This is the upstream prebuilt CEF — it lacks AgentMux's BeginWindowDrag" >&2
        echo "         patch, so left-click window drag will silently no-op. Build the patched" >&2
        echo "         libcef (docs/cef-build/build-patched-libcef.md), then either" >&2
        echo "         (a) place it at ~/cef-build/chromium_git/chromium/src/out/Release_GN_x64, or" >&2
        echo "         (b) export AGENTMUX_CEF_RUNTIME_DIR=/path/to/your/Release_GN_x64." >&2
    else
        local size_bytes
        size_bytes=$(stat -c %s "$dir/libcef.so" 2>/dev/null || stat -f %z "$dir/libcef.so" 2>/dev/null || echo 0)
        if [ "$size_bytes" -gt 1073741824 ]; then
            local size_mb=$((size_bytes / 1024 / 1024))
            echo "INFO: libcef.so at $dir is ${size_mb} MB — an unstripped build; the AppImage" >&2
            echo "      packager strips it to ~260 MB at bundle time (expected, not a problem)." >&2
        fi
    fi
    echo "$dir"
    return 0
}

# 1. Explicit override. STRICT: if the env var is set, we trust the operator
#    knew what they wanted. A typo or missing file is a hard error — do NOT
#    silently fall through to the cef-dll-sys cache, because in CI / release
#    packaging the env var is the only signal that the patched libcef should
#    be used. Falling through would yield a successful but drag-broken bundle.
#    (Codex P2 on PR #743.)
if [ -n "${AGENTMUX_CEF_RUNTIME_DIR:-}" ]; then
    if validate_dir "$AGENTMUX_CEF_RUNTIME_DIR" override; then
        exit 0
    fi
    echo "ERROR: AGENTMUX_CEF_RUNTIME_DIR=$AGENTMUX_CEF_RUNTIME_DIR" >&2
    echo "       is set but does not contain libcef.so + icudtl.dat." >&2
    echo "       Treating an explicit override as a hard requirement so a typo" >&2
    echo "       in CI / release packaging doesn't silently regress to the upstream" >&2
    echo "       cef-dll-sys fallback. Fix the path or unset the env var to use" >&2
    echo "       auto-detection (~/cef-build → cef-dll-sys cache)." >&2
    exit 1
fi

# 2. Standard cef-build layout under $HOME (your patched, dev-built tree).
CEF_BUILD_DIR="$HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64"
if validate_dir "$CEF_BUILD_DIR" cef-build; then
    exit 0
fi

# 3. Cargo cef-dll-sys cache — the upstream prebuilt fallback (no BeginWindowDrag).
#    Find first match in either debug or release.
cargo_candidates=()
while IFS= read -r d; do
    cargo_candidates+=("$d")
done < <(find "$REPO_ROOT/target" -maxdepth 6 -type d -name 'cef_linux_x86_64' 2>/dev/null)

for dir in "${cargo_candidates[@]}"; do
    if validate_dir "$dir" cargo-cache; then
        exit 0
    fi
done

echo "ERROR: could not find libcef.so + icudtl.dat in any of these locations:" >&2
echo "  - $CEF_BUILD_DIR" >&2
for c in "${cargo_candidates[@]}"; do echo "  - $c" >&2; done
echo "Build the patched libcef (see docs/cef-build/build-patched-libcef.md) or run \`cargo build -p agentmux-cef\` to populate the cef-dll-sys fallback cache." >&2
exit 1

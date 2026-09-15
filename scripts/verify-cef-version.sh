#!/usr/bin/env bash
# Verify a bundled libcef.dll's MAJOR version matches the `cef` crate the host
# links against. Guards against shipping a portable/install whose CEF runtime is
# stale relative to the linked bindings — the exact failure that made v0.42.0
# show "Request for unsupported CEF API version 14800" (splash, then no window).
#
# See docs/specs/SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03.md.
#
# Usage: verify-cef-version.sh <dir-containing-libcef.dll>
# Exit: 0 = match (or undeterminable → warn-only, never block on tooling hiccup)
#       1 = confirmed mismatch (hard fail — do NOT ship)
#
# Runs in real bash (invoke as `bash scripts/verify-cef-version.sh ...`), not the
# Taskfile Go-coreutils shell. Reads the DLL version via pwsh (Windows build dep).
set -uo pipefail

dir="${1:?usage: verify-cef-version.sh <dir-containing-libcef.dll>}"
dll="$dir/libcef.dll"

if [ ! -f "$dll" ]; then
  echo "❌ verify-cef-version: libcef.dll not found in $dir" >&2
  exit 1
fi

# Expected MAJOR = resolved version of agentmux-cef's `cef` dependency
# (agentmux-cef/Cargo.toml), via `cargo metadata --filter-platform`.
#
# NOT "grep the first 'name = \"cef\"' entry in Cargo.lock" — filter-platform
# resolves the dependency graph exactly as Windows sees it, which is more
# robust than a lockfile-ordering assumption even now that there's only one
# `cef` entry to find (SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md
# Phase E collapsed the temporary per-platform `cef_win`/`cef_unix` split —
# see agentmux-cef/Cargo.toml — back to a single dependency named plain
# `cef` for every platform; this script's lookup key follows that).
metadata="$(cargo metadata --filter-platform x86_64-pc-windows-msvc --format-version 1 2>/dev/null)"
cef_pkg_id="$(printf '%s' "$metadata" | jq -r '
  .resolve.nodes[] | select(.id | startswith("path+file://") and contains("/agentmux-cef#"))
  | .deps[] | select(.name == "cef") | .pkg
')"
expected_full="$(printf '%s' "$metadata" | jq -r --arg pkg "$cef_pkg_id" '
  .packages[] | select(.id == $pkg) | .version
')"
expected_major="$(printf '%s' "$expected_full" | sed -n 's/^\([0-9][0-9]*\).*/\1/p')"
if [ -z "$expected_major" ]; then
  echo "⚠ verify-cef-version: could not resolve cef's version via 'cargo metadata' — skipping check" >&2
  exit 0
fi

# Actual MAJOR = ProductVersion of the bundled DLL, e.g. "146.0.9+g3ca6a87..." → 146
winpath="$(cygpath -w "$dll" 2>/dev/null || echo "$dll")"
actual_ver="$(pwsh -NoProfile -Command "(Get-Item -LiteralPath '$winpath').VersionInfo.ProductVersion" 2>/dev/null | tr -d '\r\n')"
actual_major="$(printf '%s' "$actual_ver" | sed -n 's/^\([0-9][0-9]*\).*/\1/p')"
if [ -z "$actual_major" ]; then
  echo "⚠ verify-cef-version: could not read libcef.dll ProductVersion — skipping check" >&2
  exit 0
fi

if [ "$expected_major" != "$actual_major" ]; then
  echo "❌ CEF version mismatch: bundled libcef.dll is ${actual_ver} (major ${actual_major}) but the host links cef crate major ${expected_major}." >&2
  echo "   The runtime in '${dir}' is stale relative to the linked bindings — shipping it yields" >&2
  echo "   'Request for unsupported CEF API version' at startup (splash, then no window)." >&2
  echo "   Fix:  task clean:cef && task build:host   (re-materializes the matching CEF runtime)" >&2
  echo "   See:  docs/specs/SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03.md" >&2
  exit 1
fi

echo "✓ CEF runtime OK: libcef.dll ${actual_ver} matches linked cef crate major ${expected_major}"
exit 0

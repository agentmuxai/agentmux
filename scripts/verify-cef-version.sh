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

# Expected MAJOR = resolved version of Windows's specific `cef_win` alias
# (agentmux-cef/Cargo.toml), via `cargo metadata --filter-platform`.
#
# NOT "grep the first 'name = \"cef\"' entry in Cargo.lock" — as of the
# staged per-platform 152 rollout (SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_
# 2026_09_07.md Phase C), Cargo.lock can carry TWO "cef" package entries
# simultaneously (cef_win + macOS/Linux's cef_unix, e.g. 152.x and 148.x).
# "First" is a lockfile-ordering accident, not a target selection — it
# would silently validate this build's DLL against the WRONG platform's
# resolved version, exactly the class of bug this script exists to catch.
# --filter-platform resolves the dependency graph as Windows actually sees
# it, so only cef_win is in scope, unambiguously.
metadata="$(cargo metadata --filter-platform x86_64-pc-windows-msvc --format-version 1 2>/dev/null)"
cef_win_pkg_id="$(printf '%s' "$metadata" | jq -r '
  .resolve.nodes[] | select(.id | startswith("path+file://") and contains("/agentmux-cef#"))
  | .deps[] | select(.name == "cef_win") | .pkg
')"
expected_full="$(printf '%s' "$metadata" | jq -r --arg pkg "$cef_win_pkg_id" '
  .packages[] | select(.id == $pkg) | .version
')"
expected_major="$(printf '%s' "$expected_full" | sed -n 's/^\([0-9][0-9]*\).*/\1/p')"
if [ -z "$expected_major" ]; then
  echo "⚠ verify-cef-version: could not resolve cef_win's version via 'cargo metadata' — skipping check" >&2
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

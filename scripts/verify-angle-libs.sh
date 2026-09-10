#!/usr/bin/env bash
# Verify that ANGLE's EGL/GLESv2 shared libraries in a CEF runtime directory
# are genuine builds, not empty/stub artifacts from a broken or partial
# Chromium build.
#
# A real ANGLE libEGL/libGLESv2 for any recent Chromium milestone is several
# MB and exports eglGetProcAddress / glGetString by name. An interrupted or
# misconfigured local rebuild has been observed silently producing ~460KB
# placeholder files that pass a plain `-f` existence check, get bundled, and
# only fail at runtime: the GPU process dies at init with "eglGetProcAddress
# not found", falls through the SwiftShader software path too, and Chromium
# disables GPU entirely — the status bar's GFX indicator reads "off" instead
# of "HW"/"SW" with no build-time signal pointing at the cause.
#
# Usage: verify-angle-libs.sh <dir> <egl-lib-filename> <glesv2-lib-filename>
# Exit 0 if both libs (that exist) look like real ANGLE builds. A missing
# file is not this script's concern — the caller already decides whether
# absence is fatal — so a missing lib is skipped, not flagged.
# Exit 1 on a lib that exists but looks like a stub (hard fail — do NOT ship).
set -uo pipefail

dir="${1:?usage: verify-angle-libs.sh <dir> <egl-lib-filename> <glesv2-lib-filename>}"
egl_name="${2:?usage: verify-angle-libs.sh <dir> <egl-lib-filename> <glesv2-lib-filename>}"
gles_name="${3:?usage: verify-angle-libs.sh <dir> <egl-lib-filename> <glesv2-lib-filename>}"

# Real ANGLE shared libraries are multi-MB; the broken stubs observed so far
# were ~460KB. 1MB leaves comfortable margin either way.
min_bytes=1000000

ok=0

check_one() {
  local path="$1" symbol="$2" size
  [ -f "$path" ] || return 0

  size=$(wc -c <"$path" 2>/dev/null | tr -d ' ')
  if [ -z "$size" ] || [ "$size" -lt "$min_bytes" ]; then
    echo "❌ verify-angle-libs: $path is only ${size:-0} bytes — too small to be a real ANGLE build (expect several MB)." >&2
    return 1
  fi
  if ! grep -a -q "$symbol" "$path"; then
    echo "❌ verify-angle-libs: $path does not export '$symbol' — not a genuine ANGLE build." >&2
    return 1
  fi
  return 0
}

check_one "$dir/$egl_name" "eglGetProcAddress" || ok=1
check_one "$dir/$gles_name" "glGetString" || ok=1

if [ "$ok" -ne 0 ]; then
  echo "" >&2
  echo "⚠️  ANGLE GL libraries in '$dir' look like broken/stub build output, not a real ANGLE build." >&2
  echo "    Shipping these produces 'GFX off' at runtime with no build-time warning: the GPU" >&2
  echo "    process fails at init ('eglGetProcAddress not found'), the SwiftShader software" >&2
  echo "    fallback fails too, and Chromium disables GPU entirely." >&2
  echo "    Fix: rebuild ANGLE in the source CEF tree (a full rebuild, not an incremental one" >&2
  echo "    that only touched a subset of targets), then re-run bundling." >&2
  exit 1
fi

echo "✓ ANGLE GL libraries OK: $egl_name / $gles_name in '$dir' look like real builds"
exit 0

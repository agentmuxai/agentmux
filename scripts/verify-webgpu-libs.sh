#!/usr/bin/env bash
# Verify that the WebGPU shader compiler libraries are in a Windows runtime
# directory and are real builds.
#
# Why this exists: Chromium's WebGPU (Dawn, D3D12 backend) opens dxcompiler.dll
# when a page asks for a device. When it is missing, navigator.gpu.requestAdapter()
# still succeeds and requestDevice() then fails ("DynamicLib.Open: dxcompiler.dll"),
# in every page: a widget frame, a Browser pane. WebGL keeps working, so nothing
# else shows the gap. The bundling step used to skip these files on purpose
# ("WebGPU: not used") and no check noticed when they were absent.
#
# Usage: verify-webgpu-libs.sh <dir> <lib-filename>...
# Exit 0 when every named library exists in <dir>, is not a stub, and exports
# what the real one exports. Exit 1 on a missing, tiny or stub-shaped library
# (hard fail: do NOT ship). Unlike verify-angle-libs.sh a missing file IS fatal:
# this check is what keeps the file in the bundle.
set -uo pipefail

dir="${1:?usage: verify-webgpu-libs.sh <dir> <lib-filename>...}"
shift
[ "$#" -ge 1 ] || { echo "usage: verify-webgpu-libs.sh <dir> <lib-filename>..." >&2; exit 2; }

# A real dxcompiler.dll is tens of MB; this floor only catches an empty or
# truncated copy.
min_sane_bytes=100000
ok=0

symbol_for() {
  case "$1" in
    dxcompiler.dll) echo "DxcCreateInstance" ;;
    dxil.dll) echo "DxcCreateInstance" ;;
    *) echo "" ;;
  esac
}

check_one() {
  local name="$1" path size symbol
  path="$dir/$name"
  if [ ! -f "$path" ]; then
    echo "❌ verify-webgpu-libs: $path is missing; WebGPU would find an adapter but get no device." >&2
    return 1
  fi
  size=$(wc -c <"$path" 2>/dev/null | tr -d ' ')
  if [ -z "$size" ] || [ "$size" -lt "$min_sane_bytes" ]; then
    echo "❌ verify-webgpu-libs: $path is only ${size:-0} bytes; too small to be a real build." >&2
    return 1
  fi
  symbol="$(symbol_for "$name")"
  if [ -n "$symbol" ] && ! grep -a -q "$symbol" "$path"; then
    echo "❌ verify-webgpu-libs: $path ($size bytes) does not export '$symbol'; not a genuine build." >&2
    return 1
  fi
  return 0
}

for name in "$@"; do
  check_one "$name" || ok=1
done

if [ "$ok" -ne 0 ]; then
  echo "" >&2
  echo "⚠️  The WebGPU shader compiler libraries in '$dir' are missing or broken." >&2
  echo "    Shipping this leaves WebGPU with an adapter and no device in every page." >&2
  echo "    Fix: use a CEF runtime that includes them (docs/cef-build/build-patched-cef-windows.md)." >&2
  exit 1
fi

echo "✓ WebGPU shader compiler libraries OK in '$dir': $*"
exit 0

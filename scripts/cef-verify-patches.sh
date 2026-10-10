#!/usr/bin/env bash
# cef-verify-patches.sh — prove the no-symbol CEF patches are in a built artifact.
#
# docs/cef-build/CEF_FORK_MAINTENANCE.md §7.2b
#
# THE PROBLEM: two of the three Chromium-side patches add no symbol.
# agentmux_process_requirement rewrites the body of an existing function and
# rwhv_background_opaque_check changes one call inside one, so `nm` cannot see
# either and §7.2's symbol probes cannot answer whether they reached the build.
#
# THE METHOD: three compiles, isolating one variable at a time.
#   A  working-tree source at its ORIGINAL path  -> must equal the shipped object
#   B  working-tree content from a synthetic path
#   C  pristine upstream content from the SAME synthetic path
#   A == shipped  proves the artifact was built from what is checked out.
#   B != C        proves the patch materially changes codegen, which A alone
#                 cannot show (a patch compiling to upstream's bytes would pass).
#
# Why B and C share a path: under symbol_level=1 the compiler embeds the source
# path in DWARF, so comparing the shipped object against a /tmp-compiled pristine
# build always differs -- on the path alone, patched or not. An earlier version
# did exactly that and its "shipped != unpatched" leg could never fire.
#
# Why all three keep the object's own output path: Linux builds use split DWARF,
# so each object also embeds its .dwo name, derived from `-o`. An A compiled to
# /tmp/A.o could never equal the shipped object, and B.o and C.o always differed
# by name, so the no-op check could never fire. Each compile therefore runs its
# command unchanged from a scratch sibling of the build dir (same depth, so the
# `../../` source paths resolve the same; `gen/` and the rest symlinked; its own
# `obj/`), and its output is copied out before the next one.
#
# Usage:
#   scripts/cef-verify-patches.sh --build-dir out/Release_GN_arm64
#   scripts/cef-verify-patches.sh --build-dir <dir> --pair <src>:<obj>   (repeatable)
#
# Exit 0 = every pair verified. Never writes to the build directory: it compiles
# in a temporary sibling of it, removed on exit.

set -euo pipefail

BUILD_DIR="" ; PAIRS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --build-dir) BUILD_DIR="${2:-}"; shift 2 ;;
    --pair)      PAIRS+=("${2:-}"); shift 2 ;;
    -h|--help)   sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "cef-verify-patches: unknown argument '$1'" >&2; exit 1 ;;
  esac
done
[ -n "$BUILD_DIR" ] || { echo "cef-verify-patches: --build-dir is required" >&2; exit 1; }
[ -d "$BUILD_DIR" ] || { echo "cef-verify-patches: no such build dir: $BUILD_DIR" >&2; exit 1; }

if [ "${#PAIRS[@]}" -eq 0 ]; then
  # agentmux_process_requirement patches a macOS-only file.
  [ "$(uname -s)" = Darwin ] && PAIRS+=("base/apple/mach_port_rendezvous_mac.cc:obj/base/base/mach_port_rendezvous_mac.o")
  PAIRS+=("content/browser/renderer_host/render_widget_host_view_base.cc:obj/content/browser/browser/render_widget_host_view_base.o")
fi

BUILD="$(cd "$BUILD_DIR" && pwd)"
TMP="$(mktemp -d)"
# The scratch build dir: a sibling of the real one, everything but obj/ symlinked.
SCRATCH="$(mktemp -d "$(dirname "$BUILD")/.cef-verify-patches.XXXXXX")"
trap 'rm -rf "$TMP" "$SCRATCH"' EXIT
for e in "$BUILD"/* "$BUILD"/.[!.]*; do
  [ -e "$e" ] || continue
  [ "$(basename "$e")" = obj ] || ln -s "$e" "$SCRATCH/$(basename "$e")"
done
mkdir "$SCRATCH/obj"
ok=0; fail=0

# Run the object's compile command, unchanged but for `sub` (a sed expression
# applied to it, or empty), in the scratch dir, and copy the result to `dest`.
compile_to() {
  local cmd="$1" sub="$2" dest="$3"
  [ -n "$sub" ] && cmd="$(printf '%s' "$cmd" | sed "$sub")"
  ( cd "$SCRATCH" && eval "$cmd" )
  cp "$SCRATCH/$OBJ" "$dest"
  rm -f "$SCRATCH/$OBJ"
}

cd "$BUILD"
for pair in "${PAIRS[@]}"; do
  SRC="${pair%%:*}" ; OBJ="${pair##*:}"
  [ -f "$OBJ" ] || { echo "FAIL $SRC: no such object $OBJ"; fail=$((fail+1)); continue; }

  CMD="$(ninja -C . -t commands "$OBJ" | tail -1)"

  # The command runs as-is from $SCRATCH, so its relative `-o` lands in the
  # scratch obj/. Refuse anything that could still reach the real build dir: an
  # absolute path to it, or a scratch obj/ that isn't its own directory.
  case "$CMD" in *"$BUILD"*)
    echo "ABORT $SRC: the compile command names the build dir itself - refusing to run"; exit 1 ;;
  esac
  { [ -d "$SCRATCH/obj" ] && [ ! -L "$SCRATCH/obj" ]; } || { echo "ABORT: scratch obj/ is not a plain directory"; exit 1; }
  mkdir -p "$SCRATCH/$(dirname "$OBJ")"

  PROBE="s#\.\./\.\./$SRC#$TMP/probe.cc#g"
  compile_to "$CMD" "" "$TMP/A.o"
  cp "../../$SRC" "$TMP/probe.cc" ; compile_to "$CMD" "$PROBE" "$TMP/B.o"
  git -C ../.. show "HEAD:$SRC" > "$TMP/probe.cc" ; compile_to "$CMD" "$PROBE" "$TMP/C.o"

  bad=0
  cmp -s "$TMP/A.o" "$OBJ"     || { echo "FAIL $SRC: shipped object does not match the working tree"; bad=1; }
  cmp -s "$TMP/B.o" "$TMP/C.o" && { echo "FAIL $SRC: patch is a no-op - nothing distinguishes it from upstream"; bad=1; }
  if [ "$bad" = 0 ]; then echo "OK   $SRC"; ok=$((ok+1)); else fail=$((fail+1)); fi
  rm -f "$TMP/A.o" "$TMP/B.o" "$TMP/C.o" "$TMP/probe.cc"
done

echo "--- $ok verified, $fail failed ---"
[ "$fail" -eq 0 ]

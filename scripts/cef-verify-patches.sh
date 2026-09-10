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
# Usage:
#   scripts/cef-verify-patches.sh --build-dir out/Release_GN_arm64
#   scripts/cef-verify-patches.sh --build-dir <dir> --pair <src>:<obj>   (repeatable)
#
# Exit 0 = every pair verified. Never writes to the build directory.

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
  PAIRS=(
    "base/apple/mach_port_rendezvous_mac.cc:obj/base/base/mach_port_rendezvous_mac.o"
    "content/browser/renderer_host/render_widget_host_view_base.cc:obj/content/browser/browser/render_widget_host_view_base.o"
  )
fi

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
ok=0; fail=0

cd "$BUILD_DIR"
for pair in "${PAIRS[@]}"; do
  SRC="${pair%%:*}" ; OBJ="${pair##*:}"
  [ -f "$OBJ" ] || { echo "FAIL $SRC: no such object $OBJ"; fail=$((fail+1)); continue; }

  CMD="$(ninja -C . -t commands "$OBJ" | tail -1)"

  # ${VAR//a/b} and sed's /g are BOTH mandatory: the object path appears TWICE
  # in a Chromium compile line (`-MF <obj>.d` and `-o <obj>`). A non-global
  # replace redirects only the depfile and leaves `-o` on the REAL object -- it
  # does not error, it OVERWRITES the build output. The guard below refuses to
  # run any command that could still do that.
  A_CMD="${CMD//$OBJ/$TMP/A.o}"
  B_CMD="$(printf '%s' "$CMD" | sed "s#$OBJ#$TMP/B.o#g; s#\.\./\.\./$SRC#$TMP/probe.cc#g")"
  C_CMD="$(printf '%s' "$CMD" | sed "s#$OBJ#$TMP/C.o#g; s#\.\./\.\./$SRC#$TMP/probe.cc#g")"
  for c in "$A_CMD" "$B_CMD" "$C_CMD"; do
    case "$c" in *"-o $OBJ"*)
      echo "ABORT $SRC: a rewritten command still targets $OBJ - refusing to run"; exit 1 ;;
    esac
  done

  eval "$A_CMD"
  cp "../../$SRC" "$TMP/probe.cc" ; eval "$B_CMD"
  git -C ../.. show "HEAD:$SRC" > "$TMP/probe.cc" ; eval "$C_CMD"

  bad=0
  cmp -s "$TMP/A.o" "$OBJ"     || { echo "FAIL $SRC: shipped object does not match the working tree"; bad=1; }
  cmp -s "$TMP/B.o" "$TMP/C.o" && { echo "FAIL $SRC: patch is a no-op - nothing distinguishes it from upstream"; bad=1; }
  if [ "$bad" = 0 ]; then echo "OK   $SRC"; ok=$((ok+1)); else fail=$((fail+1)); fi
  rm -f "$TMP/A.o" "$TMP/B.o" "$TMP/C.o" "$TMP/probe.cc"
done

echo "--- $ok verified, $fail failed ---"
[ "$fail" -eq 0 ]

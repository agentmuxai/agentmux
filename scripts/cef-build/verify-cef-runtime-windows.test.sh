#!/usr/bin/env bash
# verify-cef-runtime-windows.test.sh — tests for verify-cef-runtime-windows.sh.
#
# The guard's job is to make a tracer-on libcef.dll fail the build instead of
# shipping a runtime that deadlocks later (2026-09-22 renderer, 2026-09-23 host).
# Every case below is a way it could wrongly pass, plus the ones it must pass.
#
# The real pinned libcef.dll is ~300 MB, so the "pinned" case runs the guard
# against a copy of it whose pin file names a small fixture's hash instead:
# the hash comparison is what's under test, not the specific value.
#
# Usage: bash scripts/cef-build/verify-cef-runtime-windows.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# A copy of the guard next to a pin file we control.
GUARD_DIR="$TMP/guard"; mkdir -p "$GUARD_DIR"
cp "$HERE/verify-cef-runtime-windows.sh" "$GUARD_DIR/"
printf 'known-good libcef bytes\n' > "$TMP/good.dll"
GOOD_SHA="$(sha256sum "$TMP/good.dll" | cut -d' ' -f1)"
cat > "$GUARD_DIR/windows-runtime-pin.sh" <<EOF
CEF_WINDOWS_RELEASE_TAG="cef-windows-x86_64-test-r9"
CEF_WINDOWS_ASSET="cef-windows-x86_64-test-r9.zip"
CEF_WINDOWS_LIBCEF_SHA256="$GOOD_SHA"
EOF
GUARD="$GUARD_DIR/verify-cef-runtime-windows.sh"

runtime() {  # runtime <name> <libcef-content-file|-> [args.gn line]
  local d="$TMP/rt-$1"; mkdir -p "$d"
  [ "$2" = "-" ] || cp "$2" "$d/libcef.dll"
  # A finished local gn build: configured (args.gn + build.ninja) first, DLL
  # built afterwards.
  if [ -n "${3:-}" ]; then
    printf '%s\n' "$3" > "$d/args.gn"
    : > "$d/build.ninja"
    touch -d '2026-01-01 00:00' "$d/args.gn" "$d/build.ninja"
  fi
  echo "$d"
}
expect() {  # expect <want-exit> <name> <dir> [env...]
  local want="$1" name="$2" dir="$3"; shift 3
  out="$(env "$@" bash "$GUARD" "$dir" 2>&1)"; got=$?
  if [ "$got" = "$want" ]; then ok "$name"; else bad "$name" "exit $got, want $want: $out"; fi
}

printf 'stale tracer-on libcef bytes\n' > "$TMP/stale.dll"

echo "verify-cef-runtime-windows.sh"
expect 0 "pinned libcef.dll passes" "$(runtime pinned "$TMP/good.dll")"
expect 1 "any other libcef.dll fails (the 09-23 stale cached runtime)" "$(runtime stale "$TMP/stale.dll")"
expect 1 "missing libcef.dll fails" "$(runtime empty -)"

out="$(bash "$GUARD" "$TMP/rt-stale" 2>&1)"
case "$out" in *"gh auth login"*"AGENTMUX_CEF_RUNTIME_DIR_WINDOWS"*"args-windows.gn"*) ok "failure names every fix";; *) bad "failure names every fix" "$out";; esac
case "$out" in *"sha256 $(sha256sum "$TMP/stale.dll" | cut -c1-12)"*) ok "failure shows the hash it found";; *) bad "failure shows the hash it found" "$out";; esac

# No local-build exception: args.gn text can't prove what the DLL contains.
# Each shape below was accepted by an earlier revision and broken in review
# (Codex, reagentx on #3615); a non-pinned DLL must fail under all of them.
lb_bypass() {  # lb_bypass <name> <args.gn content>
  expect 1 "local build tree with a non-pinned DLL fails: $1" "$(runtime "lb-$1" "$TMP/stale.dll" "$2")"
}
lb_bypass "tracer=false" 'enable_backup_ref_ptr_instance_tracer=false'
lb_bypass "tracer=false, trailing comment" 'enable_backup_ref_ptr_instance_tracer = false  # 09-22'
lb_bypass "tracer=falsey_nonsense" 'enable_backup_ref_ptr_instance_tracer=falsey_nonsense'
lb_bypass "conditional reassignment" "$(printf 'enable_backup_ref_ptr_instance_tracer=false\nif (is_win) {\n  enable_backup_ref_ptr_instance_tracer=true\n}')"
lb_bypass "import of a .gni reassigning it" "$(printf 'enable_backup_ref_ptr_instance_tracer=false\nimport("//tracer_on.gni")')"
d="$(runtime lb-reconf "$TMP/stale.dll" 'enable_backup_ref_ptr_instance_tracer=false')"; touch -d '2025-06-01 00:00' "$d/libcef.dll"
expect 1 "local build tree with a non-pinned DLL fails: reconfigured, not rebuilt" "$d"
d="$(runtime lb-nobn "$TMP/stale.dll" 'enable_backup_ref_ptr_instance_tracer=false')"; rm "$d/build.ninja"
expect 1 "local build tree with a non-pinned DLL fails: args.gn without build.ninja" "$d"
expect 0 "a local build tree whose DLL is the pinned build passes, on its hash" "$(runtime lb-pinned "$TMP/good.dll" 'is_official_build=true')"

expect 0 "AGENTMUX_ALLOW_UNVERIFIED_CEF=1 overrides a failure" "$TMP/rt-stale" AGENTMUX_ALLOW_UNVERIFIED_CEF=1
out="$(AGENTMUX_ALLOW_UNVERIFIED_CEF=1 bash "$GUARD" "$TMP/rt-stale" 2>&1)"
case "$out" in *"continuing with this runtime anyway"*) ok "override still warns";; *) bad "override still warns" "$out";; esac
expect 1 "AGENTMUX_ALLOW_UNVERIFIED_CEF=true is not the opt-out" "$TMP/rt-stale" AGENTMUX_ALLOW_UNVERIFIED_CEF=true

echo "wiring"
# shellcheck source=windows-runtime-pin.sh
( source "$HERE/windows-runtime-pin.sh"
  [[ "$CEF_WINDOWS_LIBCEF_SHA256" =~ ^[0-9a-f]{64}$ ]] && [ "$CEF_WINDOWS_ASSET" = "$CEF_WINDOWS_RELEASE_TAG.zip" ] ) \
  && ok "pin file is well-formed" || bad "pin file is well-formed"
grep -q 'source "$(dirname "${BASH_SOURCE\[0\]}")/windows-runtime-pin.sh"' "$HERE/fetch-patched-cef-windows.sh" \
  && ! grep -Eq '^RELEASE_TAG="cef-' "$HERE/fetch-patched-cef-windows.sh" \
  && ok "fetch script takes its pin from windows-runtime-pin.sh" || bad "fetch script takes its pin from windows-runtime-pin.sh"
tf="$(tr -d '\r' < "$REPO/Taskfile.yml")"
v=$(grep -n 'verify-cef-version.sh "$cefDir"' <<<"$tf" | head -1 | cut -d: -f1)
g=$(grep -n 'verify-cef-runtime-windows.sh "$cefDir" || exit 1' <<<"$tf" | head -1 | cut -d: -f1)
[ -n "$v" ] && [ -n "$g" ] && [ "$g" -gt "$v" ] && ok "bundle:windows runs the guard after the version check" \
  || bad "bundle:windows runs the guard after the version check" "version@$v guard@$g"

echo "$pass passed, $fail failed"
[ "$fail" = 0 ]

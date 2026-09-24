#!/usr/bin/env bash
# verify-cef-runtime-windows.sh <runtime-dir> — refuse a CEF runtime that
# isn't known to be built with the raw_ptr instance tracer off.
#
# With the tracer on, any process that loads libcef.dll can deadlock on the
# tracer's global mutex: a renderer on 2026-09-22, the host UI thread on
# 2026-09-23 (INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md).
# Upstream CEF's Windows release builds, the stock cef-dll-sys runtime and every
# AgentMux runtime before r2 have it on. A build that bundles one succeeds and
# then hangs at some later moment, so bundle:windows checks here instead.
#
# Accepts when:
#   1. libcef.dll's SHA-256 is the pinned tracer-off build
#      (windows-runtime-pin.sh), or
#   2. <runtime-dir> is a local Chromium build tree whose args.gn sets
#      enable_backup_ref_ptr_instance_tracer=false.
# AGENTMUX_ALLOW_UNVERIFIED_CEF=1 downgrades a failure to a warning.
#
# Exit: 0 = accepted (or overridden), 1 = refused.
# Spec: docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md
set -uo pipefail

# shellcheck source=windows-runtime-pin.sh
source "$(dirname "${BASH_SOURCE[0]}")/windows-runtime-pin.sh"

dir="${1:?usage: verify-cef-runtime-windows.sh <runtime-dir>}"
libcef="$dir/libcef.dll"

refuse() {
  echo "" >&2
  echo "❌ CEF runtime check failed: $1" >&2
  echo "   Runtime: $dir" >&2
  echo "   A libcef.dll built with the raw_ptr instance tracer can deadlock the app at a random" >&2
  echo "   later moment (INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md)." >&2
  echo "   Expected $CEF_WINDOWS_RELEASE_TAG (libcef.dll sha256 ${CEF_WINDOWS_LIBCEF_SHA256:0:12}…). Fix one of:" >&2
  echo "     - let the build fetch it: 'gh auth login', or run with GH_TOKEN=<token>" >&2
  echo "     - point AGENTMUX_CEF_RUNTIME_DIR_WINDOWS at that runtime" >&2
  echo "     - rebuild CEF with scripts/cef-build/args-windows.gn (tracer off)" >&2
  if [ "${AGENTMUX_ALLOW_UNVERIFIED_CEF:-}" = "1" ]; then
    echo "   ⚠️  AGENTMUX_ALLOW_UNVERIFIED_CEF=1 — continuing with this runtime anyway." >&2
    echo "" >&2
    exit 0
  fi
  echo "   (Deliberate exception: AGENTMUX_ALLOW_UNVERIFIED_CEF=1.)" >&2
  echo "" >&2
  exit 1
}

[ -f "$libcef" ] || refuse "no libcef.dll in the runtime directory"

actual="$(sha256sum "$libcef" | cut -d' ' -f1)"
if [ "$actual" = "$CEF_WINDOWS_LIBCEF_SHA256" ]; then
  echo "CEF runtime verified: $CEF_WINDOWS_RELEASE_TAG (tracer off)"
  exit 0
fi

# A local Chromium compile. args.gn is configuration, not proof of what the
# DLL contains, so accept only when (Codex + reagentx on #3615):
#   - the flag is assigned exactly once, to exactly `false` (a trailing
#     comment is fine; `falsey`, a second or conditional assignment is not);
#   - it is a real gn build tree (build.ninja present -- args.gn alone next to
#     a copied-in DLL ties nothing to that config), and
#   - libcef.dll is newer than args.gn and build.ninja (which `gn gen`
#     rewrites on every args change): a tree reconfigured with the fixed args
#     but not yet (or not successfully) rebuilt still holds the old DLL.
if [ -f "$dir/args.gn" ]; then
  [ -f "$dir/build.ninja" ] || refuse "args.gn without build.ninja isn't a gn build tree — only the pinned build is accepted here"
  args="$(tr -d '\r' < "$dir/args.gn")"
  mentions="$(grep -Ec '^[^#]*enable_backup_ref_ptr_instance_tracer' <<<"$args")"
  if [ "$mentions" != 1 ] || ! grep -Eq '^[[:space:]]*enable_backup_ref_ptr_instance_tracer[[:space:]]*=[[:space:]]*false[[:space:]]*(#.*)?$' <<<"$args"; then
    refuse "local CEF build whose args.gn doesn't set enable_backup_ref_ptr_instance_tracer=false exactly once"
  fi
  for cfg in args.gn build.ninja; do
    if [ "$dir/$cfg" -nt "$libcef" ]; then
      refuse "local CEF build reconfigured after libcef.dll was built ($cfg is newer) — finish the rebuild"
    fi
  done
  echo "CEF runtime is a local build with enable_backup_ref_ptr_instance_tracer=false, built after its config — accepted"
  exit 0
fi

refuse "libcef.dll is not the pinned tracer-off build (sha256 ${actual:0:12}…)"

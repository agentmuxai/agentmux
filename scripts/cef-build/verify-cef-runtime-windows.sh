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
# Accepts only when libcef.dll's SHA-256 is the pinned tracer-off build
# (windows-runtime-pin.sh). AGENTMUX_ALLOW_UNVERIFIED_CEF=1 downgrades a
# failure to a warning -- the way to try a local CEF build before its hash is
# pinned.
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
  echo "     - testing your own CEF build (args-windows.gn, tracer off)? use the opt-out below," >&2
  echo "       and pin its libcef.dll hash in scripts/cef-build/windows-runtime-pin.sh once published" >&2
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

# Deliberately no "local CEF build" exception. An earlier revision accepted
# a build tree whose args.gn set the tracer off; review (Codex, reagentx on
# #3615) found four ways that text can disagree with the DLL: a malformed
# value, a second or conditional assignment, a tree reconfigured but not
# rebuilt, an args.gn without its gn tree, and an import()ed .gni reassigning
# the flag. GN's own effective value (`gn args --list`) needs the full CEF
# build environment, which a build-time check can't count on. Only the bytes
# prove it: a local build is accepted once its hash is pinned, or knowingly via
# AGENTMUX_ALLOW_UNVERIFIED_CEF=1 while testing it.
refuse "libcef.dll is not the pinned tracer-off build (sha256 ${actual:0:12}…)"

# shellcheck shell=bash
# The one pin for the Windows CEF runtime local builds use. Sourced (not run)
# by fetch-patched-cef-windows.sh (what to download) and
# verify-cef-runtime-windows.sh (what bundle:windows accepts).
#
# The runtime must be built with enable_backup_ref_ptr_instance_tracer=false:
# with the tracer on, any process that loads libcef.dll can deadlock on its
# global mutex (INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md).
#
# Bumping the runtime: change all three lines here AND the Windows tag in
# cef-runtime-pins.sh in the same PR (release-core.yml fails if they differ). CEF_WINDOWS_LIBCEF_SHA256 is the SHA-256 of libcef.dll inside
# the release zip (not the zip's own checksum). See
# docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md.
CEF_WINDOWS_RELEASE_TAG="cef-windows-x86_64-154.0.8037.98-r1"
CEF_WINDOWS_ASSET="cef-windows-x86_64-154.0.8037.98-r1.zip"
CEF_WINDOWS_LIBCEF_SHA256="c8ead7a4a79e007acd0df12430d31db0aaacecb6b53305e84b7d740c7a8cbead"

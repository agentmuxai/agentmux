# shellcheck shell=bash
# The one pin for the Windows CEF runtime local builds use. Sourced (not run)
# by fetch-patched-cef-windows.sh (what to download) and
# verify-cef-runtime-windows.sh (what bundle:windows accepts).
#
# The runtime must be built with enable_backup_ref_ptr_instance_tracer=false:
# with the tracer on, any process that loads libcef.dll can deadlock on its
# global mutex (INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md).
#
# Bumping the runtime: change all three lines here AND release.yml's WIN_TAG in
# the same PR. CEF_WINDOWS_LIBCEF_SHA256 is the SHA-256 of libcef.dll inside
# the release zip (not the zip's own checksum). See
# docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md.
CEF_WINDOWS_RELEASE_TAG="cef-windows-x86_64-152.0.7977.83-r2"
CEF_WINDOWS_ASSET="cef-windows-x86_64-152.0.7977.83-r2.zip"
CEF_WINDOWS_LIBCEF_SHA256="cb652dd7ae5c1724ea7f2f1efcd4d72630685684aa0381c0570020a436881085"

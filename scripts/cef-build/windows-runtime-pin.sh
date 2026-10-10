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
# cef-runtime-pins.sh in the same PR (release.yml fails if they differ). CEF_WINDOWS_LIBCEF_SHA256 is the SHA-256 of libcef.dll inside
# the release zip (not the zip's own checksum). See
# docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md.
CEF_WINDOWS_RELEASE_TAG="cef-windows-x86_64-154.0.8037.58-r2"
CEF_WINDOWS_ASSET="cef-windows-x86_64-154.0.8037.58-r2.zip"
CEF_WINDOWS_LIBCEF_SHA256="173757b4a4ce7fd89c15a73f62acee929bdd70dd8be7301f72e60fd3a434f8c2"

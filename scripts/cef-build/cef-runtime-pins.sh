# shellcheck shell=bash
# The one pin for the CEF runtime each platform's CI build fetches. Sourced
# (not run) by release-core.yml's cef-runtime-pins job and by the blank-tag path of
# build-windows.yml, build-linux.yml and build-macos.yml, so a nightly or a
# direct dispatch without an explicit cef-runtime-tag builds exactly what a
# release would, never "whatever agentmuxai/cef published last". Publishing a
# new runtime therefore changes nothing until a PR moves these pins.
#
# Bumping the runtime: change all three lines here in ONE PR, built from ONE
# agentmuxai/cef commit (docs/cef-build/CEF_FORK_MAINTENANCE.md §8), together
# with windows-runtime-pin.sh (the Windows tag must match its
# CEF_WINDOWS_RELEASE_TAG; release-core.yml fails the release otherwise).
CEF_WINDOWS_RUNTIME_TAG="cef-windows-x86_64-154.0.8037.58-r4"
# r5 is r4's fork commit rebuilt against Chromium's Debian sysroot (glibc 2.31 floor), so it moves alone; the
# archive's sha256 is checked by verify-runtime-archive.sh when a build downloads it.
CEF_LINUX_RUNTIME_TAG="cef-linux-x86_64-154.0.8037.58-codecs-r5"
CEF_LINUX_RUNTIME_SHA256="5aae8898bf44f86f7a5da84ca62d221abf930cff6dafcd06557339859cc30930"
CEF_MACOS_RUNTIME_TAG="cef-macos-arm64-154.0.8037.98-codecs-r1"

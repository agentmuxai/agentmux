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
CEF_WINDOWS_RUNTIME_TAG="cef-windows-x86_64-156.0.8078.88-r1"
# The Linux runtime is built against Chromium's Debian sysroot (glibc 2.31 floor); the archive's sha256 is
# checked by verify-runtime-archive.sh when a build downloads it.
CEF_LINUX_RUNTIME_TAG="cef-linux-x86_64-156.0.8078.88-codecs-r1"
CEF_LINUX_RUNTIME_SHA256="a070f5d48810561b044128b07ad9e88a7727f585b9f07366340242839b8ff616"
CEF_MACOS_RUNTIME_TAG="cef-macos-arm64-156.0.8078.88-codecs-r1"

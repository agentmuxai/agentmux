#!/usr/bin/env bash
# Idempotently ensure the codec-enabled (patched) Windows CEF runtime exists at
# the standard cef-build location, downloading it from the pre-built
# agentmuxai/cef GitHub release if it isn't already there.
#
# Why this exists
# ----------------
# docs/cef-build/build-patched-cef-windows.md documents a ~3-6 hour local
# Chromium compile to produce a codec-enabled libcef.dll. That's only needed
# ONCE per CEF version bump — the resulting artifact is already published as a
# GitHub release on agentmuxai/cef (see RELEASE_TAG below), so every other
# machine/agent can just download it instead of rebuilding. Without this
# script, `task package`/`task dev` silently fall back to the stock
# (no-codec) CEF whenever ~/cef-build isn't set up locally, which is easy to
# miss (docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md).
#
# Non-fatal by design: this is a convenience auto-fetch, not a hard
# requirement. Any failure (gh missing/unauthenticated, network, extraction)
# prints a clear reason and exits 1 -- the caller (Taskfile.yml's
# bundle:windows) treats that exactly like "cef-build not present" and falls
# through to the existing advisory-warning + stock-CEF behavior. It never
# blocks a build.
#
# Usage: bash scripts/cef-build/fetch-patched-cef-windows.sh <target-dir>
#   <target-dir> is normally $HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64
#   (Taskfile.yml's $cefBuildDefault).
# Exit: 0 = target-dir now has RELEASE_TAG's libcef.dll + icudtl.dat (already
#           did, or just fetched them), or is a local CEF build left untouched.
#           A fetched runtime from any other release is moved aside to
#           <target-dir>.stale-<timestamp> and replaced.
#       1 = could not ensure it (see stderr) -- caller should fall back
set -uo pipefail

# Pin explicitly, don't resolve "latest" -- must match the CEF major linked in
# Cargo.lock (currently 152, see scripts/verify-cef-version.sh). Bump the pin
# in windows-runtime-pin.sh when a new patched build is cut per
# build-patched-cef-windows.md's "Package + upload as a GitHub release" section.
# Codex P1 on PR #3231: this was still pinned to 148 after agentmux-cef's
# Cargo 152 collapse landed -- `task dev`/`task package` on a machine with no
# existing ~/cef-build tree would auto-fetch the WRONG runtime and fail
# Taskfile.yml's version guard.
RELEASE_REPO="agentmuxai/cef"
# The pin lives in windows-runtime-pin.sh, shared with
# verify-cef-runtime-windows.sh so what is fetched and what bundle:windows
# accepts can't disagree.
# shellcheck source=windows-runtime-pin.sh
source "$(dirname "${BASH_SOURCE[0]}")/windows-runtime-pin.sh"
RELEASE_TAG="$CEF_WINDOWS_RELEASE_TAG"
ASSET_PATTERN="$CEF_WINDOWS_ASSET"

target_dir="${1:?usage: fetch-patched-cef-windows.sh <target-dir>}"

# Which release a fetched runtime came from. "libcef.dll exists" is not
# enough: bumping RELEASE_TAG used to change nothing on a machine that had
# already fetched an older runtime, so the tracer-off r2 pin (#3561) never
# reached local builds. A local 0.56.13 package shipped the 2026-09-15
# tracer-on libcef.dll and its UI thread deadlocked on InstanceTracer's mutex
# (INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md §3a,
# same lock at libcef+0x11656b38, 2026-09-23).
stamp_path="$target_dir/.agentmux-cef-release"
replace_stale=0
if [ -f "$target_dir/libcef.dll" ] && [ -f "$target_dir/icudtl.dat" ]; then
  if [ "$(cat "$stamp_path" 2>/dev/null)" = "$RELEASE_TAG" ]; then
    echo "fetch-patched-cef-windows: $RELEASE_TAG already present at $target_dir" >&2
    exit 0
  fi
  if [ -f "$target_dir/build.ninja" ] || [ -f "$target_dir/args.gn" ]; then
    # A local Chromium compile, not a fetched runtime: never replace it.
    echo "fetch-patched-cef-windows: $target_dir is a local CEF build; leaving it." >&2
    echo "  It must be built with scripts/cef-build/args-windows.gn (raw_ptr instance tracer off)." >&2
    exit 0
  fi
  echo "fetch-patched-cef-windows: runtime at $target_dir is not $RELEASE_TAG" \
       "(stamp: $(cat "$stamp_path" 2>/dev/null || echo none)) -- replacing it" >&2
  replace_stale=1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "fetch-patched-cef-windows: gh CLI not found -- cannot auto-fetch" >&2
  exit 1
fi

# No `gh auth status` pre-check: it exits 1 whenever ANY stored login is
# invalid, even with a working GH_TOKEN it reports as logged in, so it
# refused downloads that would have succeeded (2026-09-23). The download's
# own failure is the real test.
work_dir="$(mktemp -d)"
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT

echo "fetch-patched-cef-windows: downloading $ASSET_PATTERN from $RELEASE_REPO@$RELEASE_TAG ..." >&2
if ! gh release download "$RELEASE_TAG" --repo "$RELEASE_REPO" \
    --pattern "$ASSET_PATTERN" --dir "$work_dir" >&2; then
  echo "fetch-patched-cef-windows: download failed -- is gh authenticated ('gh auth login' or GH_TOKEN)?" >&2
  echo "  Or set up $target_dir manually per docs/cef-build/build-patched-cef-windows.md" >&2
  exit 1
fi

zip_path="$work_dir/$ASSET_PATTERN"
if [ ! -f "$zip_path" ]; then
  echo "fetch-patched-cef-windows: expected asset not found after download: $zip_path" >&2
  exit 1
fi

echo "fetch-patched-cef-windows: extracting ..." >&2
extract_dir="$work_dir/extracted"
# pwsh needs native Windows paths -- mktemp -d produces a Unix-style /tmp/...
# path in Git Bash that Expand-Archive can't resolve (confirmed live).
zip_path_win="$(cygpath -w "$zip_path")"
extract_dir_win="$(cygpath -w "$extract_dir")"
if ! pwsh -NoProfile -Command "Expand-Archive -Path '$zip_path_win' -DestinationPath '$extract_dir_win' -Force" >&2; then
  echo "fetch-patched-cef-windows: extraction failed" >&2
  exit 1
fi

# Codex P1 on PR #3231: don't assume a fixed wrapper directory name inside
# the zip -- the 148 release was packaged with a `cef_windows_x86_64/` root
# (this script's old ZIP_ROOT_DIR), but the 152 release
# (docs/cef-build/build-patched-cef-windows.md §7's Compress-Archive
# invocation) has the files at the zip's top level instead, with no wrapper
# directory at all. Search for libcef.dll and use its own containing
# directory, matching the already-robust technique
# .github/workflows/build-windows.yml's CI download step uses for the same
# ambiguity.
libcef_found="$(find "$extract_dir" -iname 'libcef.dll' -print -quit)"
if [ -z "$libcef_found" ]; then
  echo "fetch-patched-cef-windows: libcef.dll not found anywhere inside the extracted archive -- release layout may have changed" >&2
  exit 1
fi
src_dir="$(dirname "$libcef_found")"

# Swap only now that the new runtime is in hand: every failure above leaves
# the old runtime in place, which the caller would use anyway (falling back
# to stock CEF instead would be no safer -- it carries the same tracer).
# Moved aside rather than deleted, and into a fresh directory rather than
# copied over, so no file from the old runtime survives into the new one.
if [ "$replace_stale" = 1 ]; then
  stale_dir="$target_dir.stale-$(date +%Y%m%d%H%M%S)"
  if ! mv "$target_dir" "$stale_dir"; then
    echo "fetch-patched-cef-windows: could not move the old runtime aside (is a running AgentMux dev build using it?)." >&2
    echo "  Close it and re-run, or delete $target_dir by hand. The OLD runtime is still in place." >&2
    exit 1
  fi
  echo "fetch-patched-cef-windows: old runtime kept at $stale_dir" >&2
fi

mkdir -p "$target_dir"
cp -rf "$src_dir/." "$target_dir/"

if [ -f "$target_dir/libcef.dll" ] && [ -f "$target_dir/icudtl.dat" ]; then
  printf '%s\n' "$RELEASE_TAG" > "$stamp_path"
  echo "fetch-patched-cef-windows: ✓ installed $RELEASE_TAG to $target_dir" >&2
  exit 0
fi

echo "fetch-patched-cef-windows: copy completed but libcef.dll/icudtl.dat still missing in $target_dir" >&2
exit 1

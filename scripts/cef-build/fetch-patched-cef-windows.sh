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
# Exit: 0 = target-dir now has a valid libcef.dll + icudtl.dat (already did,
#           or just fetched them)
#       1 = could not ensure it (see stderr) -- caller should fall back
set -uo pipefail

# Pin explicitly, don't resolve "latest" -- must match the CEF major linked in
# Cargo.lock (currently 148, see scripts/verify-cef-version.sh). Bump this
# when a new patched build is cut per build-patched-cef-windows.md's
# "Package + upload as a GitHub release" section.
RELEASE_REPO="agentmuxai/cef"
RELEASE_TAG="cef-windows-x86_64-148.0.7778.180"
ASSET_PATTERN="cef-windows-x86_64-148.0.7778.180.zip"
# The asset's zip root -- see the release's own package step in
# build-patched-cef-windows.md.
ZIP_ROOT_DIR="cef_windows_x86_64"

target_dir="${1:?usage: fetch-patched-cef-windows.sh <target-dir>}"

if [ -f "$target_dir/libcef.dll" ] && [ -f "$target_dir/icudtl.dat" ]; then
  echo "fetch-patched-cef-windows: already present at $target_dir" >&2
  exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "fetch-patched-cef-windows: gh CLI not found -- cannot auto-fetch" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "fetch-patched-cef-windows: gh is not authenticated -- cannot auto-fetch" >&2
  echo "  run 'gh auth login', or set up $target_dir manually per" >&2
  echo "  docs/cef-build/build-patched-cef-windows.md" >&2
  exit 1
fi

work_dir="$(mktemp -d)"
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT

echo "fetch-patched-cef-windows: downloading $ASSET_PATTERN from $RELEASE_REPO@$RELEASE_TAG ..." >&2
if ! gh release download "$RELEASE_TAG" --repo "$RELEASE_REPO" \
    --pattern "$ASSET_PATTERN" --dir "$work_dir" >&2; then
  echo "fetch-patched-cef-windows: download failed" >&2
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

src_dir="$extract_dir/$ZIP_ROOT_DIR"
if [ ! -d "$src_dir" ]; then
  echo "fetch-patched-cef-windows: expected $ZIP_ROOT_DIR/ not found inside archive -- release layout may have changed" >&2
  exit 1
fi

mkdir -p "$target_dir"
cp -rf "$src_dir/." "$target_dir/"

if [ -f "$target_dir/libcef.dll" ] && [ -f "$target_dir/icudtl.dat" ]; then
  echo "fetch-patched-cef-windows: ✓ installed to $target_dir" >&2
  exit 0
fi

echo "fetch-patched-cef-windows: copy completed but libcef.dll/icudtl.dat still missing in $target_dir" >&2
exit 1

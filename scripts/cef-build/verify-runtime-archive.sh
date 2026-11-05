#!/usr/bin/env bash
# The Linux CEF runtime archive a build downloaded is the one the pin names.
#
#   verify-runtime-archive.sh <archive> <tag> [pins-file]
#
# cef-runtime-pins.sh carries the archive's sha256 next to its tag (CEF_LINUX_RUNTIME_SHA256). A build of the
# pinned tag must match it; a build of another tag (an explicit cef-runtime-tag input) has no pinned hash and
# is not checked. Exit 0 = fine, 1 = the archive is not the pinned one, 2 = bad arguments.
set -euo pipefail

archive="${1:-}"
tag="${2:-}"
pins="${3:-scripts/cef-build/cef-runtime-pins.sh}"
if [ -z "$archive" ] || [ -z "$tag" ]; then
  echo "usage: verify-runtime-archive.sh <archive> <tag> [pins-file]" >&2
  exit 2
fi
if [ ! -f "$archive" ]; then
  echo "ERROR: $archive is not a file" >&2
  exit 2
fi

# shellcheck source=/dev/null
source "$pins"
if [ "$tag" != "${CEF_LINUX_RUNTIME_TAG:-}" ]; then
  echo "$tag is not the pinned Linux runtime (${CEF_LINUX_RUNTIME_TAG:-none}): hash not checked"
  exit 0
fi
want="${CEF_LINUX_RUNTIME_SHA256:-}"
if [ -z "$want" ]; then
  echo "ERROR: $pins pins $tag but carries no CEF_LINUX_RUNTIME_SHA256" >&2
  exit 1
fi
# From stdin, never `sha256sum "$archive"`: a file name that starts with a backslash changes the output.
got="$(sha256sum < "$archive" | cut -d' ' -f1)"
if [ "$got" != "$want" ]; then
  echo "ERROR: $tag's archive has sha256 $got, the pin says $want" >&2
  exit 1
fi
echo "$tag: archive sha256 matches the pin"

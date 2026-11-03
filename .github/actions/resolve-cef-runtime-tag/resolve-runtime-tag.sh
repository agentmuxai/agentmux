#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# The agentmuxai/cef release tag a build-*.yml fetches its CEF runtime from, printed on stdout.
#
#   resolve-runtime-tag.sh <linux|macos|windows> <pins-file> [tag]
#
# <pins-file> is the checked-out ref's scripts/cef-build/cef-runtime-pins.sh. This script itself comes from main
# (the build workflows use the action at @main, so a ref that predates it still builds). A tag given as the third
# argument wins. Blank means this ref's committed pin in cef-runtime-pins.sh, never
# "latest published": a nightly or a direct dispatch must not pair a newly published runtime with this ref's
# binding (SPEC_CEF_MILESTONE_UPGRADE_152_TO_154_2026_09_27.md section 4). A deliberate non-pinned build names its
# cef-runtime-tag explicitly. Run by this action for the three build workflows.
set -euo pipefail

platform="${1:-}"
pins="${2:-}"
tag="${3:-}"

if [ -z "$tag" ]; then
    # shellcheck source=/dev/null
    source "$pins"
    case "$platform" in
        linux) tag="${CEF_LINUX_RUNTIME_TAG:-}" ;;
        macos) tag="${CEF_MACOS_RUNTIME_TAG:-}" ;;
        windows) tag="${CEF_WINDOWS_RUNTIME_TAG:-}" ;;
        *)
            echo "usage: resolve-runtime-tag.sh <linux|macos|windows> <pins-file> [tag]" >&2
            exit 2
            ;;
    esac
fi

if [ -z "$tag" ] || [ "$tag" = "null" ]; then
    echo "ERROR: no $platform CEF runtime tag (input blank and no pin in $pins)" >&2
    exit 1
fi

printf '%s\n' "$tag"

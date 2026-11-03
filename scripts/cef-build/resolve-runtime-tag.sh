#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# The agentmuxai/cef release tag a build-*.yml fetches its CEF runtime from, printed on stdout.
#
#   resolve-runtime-tag.sh <linux|macos|windows> [tag]
#
# A tag given as the second argument wins. Blank means this ref's committed pin in cef-runtime-pins.sh, never
# "latest published": a nightly or a direct dispatch must not pair a newly published runtime with this ref's
# binding (SPEC_CEF_MILESTONE_UPGRADE_152_TO_154_2026_09_27.md section 4). A deliberate non-pinned build names its
# cef-runtime-tag explicitly. Run by .github/actions/resolve-cef-runtime-tag for the three build workflows.
set -euo pipefail

platform="${1:-}"
tag="${2:-}"

if [ -z "$tag" ]; then
    # shellcheck source=/dev/null
    source "$(dirname "$0")/cef-runtime-pins.sh"
    case "$platform" in
        linux) tag="${CEF_LINUX_RUNTIME_TAG:-}" ;;
        macos) tag="${CEF_MACOS_RUNTIME_TAG:-}" ;;
        windows) tag="${CEF_WINDOWS_RUNTIME_TAG:-}" ;;
        *)
            echo "usage: resolve-runtime-tag.sh <linux|macos|windows> [tag]" >&2
            exit 2
            ;;
    esac
fi

if [ -z "$tag" ] || [ "$tag" = "null" ]; then
    echo "ERROR: no $platform CEF runtime tag (input blank and no pin in cef-runtime-pins.sh)" >&2
    exit 1
fi

printf '%s\n' "$tag"

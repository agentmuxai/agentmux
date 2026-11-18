#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# Install go-task (`task`) on a CI runner from its GitHub release, never from a package feed.
#
#   scripts/ci-install-go-task.sh [<install dir>]
#
# Why: the Windows release build took go-task from the Chocolatey community feed; the feed answered 504, the step
# passed anyway, and the build died minutes later with "task: command not found". This script
#   1. takes the version and the SHA-256 of each archive from scripts/go-task.pins (one place),
#   2. downloads with CI_GO_TASK_ATTEMPTS (3: the first try and two more) tries on any download error, pausing between them,
#   3. refuses an archive whose hash is not the pinned one (and a platform with no pin),
#   4. runs `task --version` from the install dir and fails unless it names the pinned version,
#   5. puts the install dir on the later steps' PATH (GITHUB_PATH) when running in GitHub Actions.
# The install dir defaults to $RUNNER_TEMP/go-task (else a temp dir).
#
# Test and tuning knobs: CI_GO_TASK_PINS (another pins file), CI_GO_TASK_BASE_URL (another release URL base),
# CI_GO_TASK_PLATFORM (e.g. linux_amd64), CI_GO_TASK_PAUSE (seconds between tries, 10).

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PINS="${CI_GO_TASK_PINS:-$HERE/go-task.pins}"
ATTEMPTS="${CI_GO_TASK_ATTEMPTS:-3}"
PAUSE="${CI_GO_TASK_PAUSE:-10}"

die() { printf 'ci-install-go-task: %s\n' "$*" >&2; exit 1; }

[ -f "$PINS" ] || die "pins file not found: $PINS"
VERSION="$(awk '$1 == "version" { print $2; exit }' "$PINS")"
[ -n "$VERSION" ] || die "no 'version' line in $PINS"
BASE="${CI_GO_TASK_BASE_URL:-https://github.com/go-task/task/releases/download/v$VERSION}"

platform() {
    if [ -n "${CI_GO_TASK_PLATFORM:-}" ]; then echo "$CI_GO_TASK_PLATFORM"; return; fi
    local os arch
    case "$(uname -s)" in
        Linux) os=linux ;;
        Darwin) os=darwin ;;
        MINGW*|MSYS*|CYGWIN*) os=windows ;;
        *) die "unsupported OS: $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch=amd64 ;;
        aarch64|arm64) arch=arm64 ;;
        *) die "unsupported CPU: $(uname -m)" ;;
    esac
    echo "${os}_${arch}"
}

PLATFORM="$(platform)"
case "$PLATFORM" in windows_*) EXT=zip ;; *) EXT=tar.gz ;; esac
FILE="task_${PLATFORM}.${EXT}"
WANT="$(awk -v f="$FILE" '$2 == f { print $1; exit }' "$PINS")"
[ -n "$WANT" ] || die "no pinned hash for $FILE in $PINS (add the line from the release's task_checksums.txt)"

DEST="${1:-${RUNNER_TEMP:-$(mktemp -d)}/go-task}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$DEST" || die "cannot create $DEST"

sha256_of() {
    # Hash from stdin: given a Windows path, coreutils prefixes the hash with a backslash.
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum < "$1" | cut -d' ' -f1
    else
        shasum -a 256 < "$1" | cut -d' ' -f1
    fi
}

got=""
n=1
while [ "$n" -le "$ATTEMPTS" ]; do
    rm -f "$WORK/$FILE"
    if curl --fail --silent --show-error --location --connect-timeout 20 --max-time 180 \
        --output "$WORK/$FILE" "$BASE/$FILE"; then
        got="$(sha256_of "$WORK/$FILE")"
        [ "$got" = "$WANT" ] && break
        # A wrong hash is not a network blip: a changed or tampered file stays wrong on every try.
        die "$FILE is not the pinned release: its SHA-256 is $got, the pin says $WANT"
    fi
    echo "ci-install-go-task: download of $FILE failed (try $n of $ATTEMPTS)" >&2
    n=$((n + 1))
    [ "$n" -le "$ATTEMPTS" ] && sleep "$PAUSE"
done
[ "$got" = "$WANT" ] || die "could not download $BASE/$FILE after $ATTEMPTS tries"

case "$EXT" in
    zip)
        if command -v unzip > /dev/null 2>&1; then unzip -o -q "$WORK/$FILE" task.exe -d "$DEST" || die "unzip failed"
        elif command -v 7z > /dev/null 2>&1; then 7z x -y -o"$DEST" "$WORK/$FILE" task.exe > /dev/null || die "7z failed"
        else die "neither unzip nor 7z is on PATH"; fi
        BIN="$DEST/task.exe" ;;
    *)
        tar -xzf "$WORK/$FILE" -C "$DEST" task || die "tar failed"
        chmod +x "$DEST/task"
        BIN="$DEST/task" ;;
esac

SAID="$("$BIN" --version 2>&1)" || die "$BIN --version failed: $SAID"
case "$SAID" in
    *"$VERSION"*) ;;
    *) die "$BIN says '$SAID', expected version $VERSION" ;;
esac

echo "ci-install-go-task: go-task $VERSION installed in $DEST ($FILE, SHA-256 matched)"
if [ -n "${GITHUB_PATH:-}" ]; then
    # GITHUB_PATH wants a path the runner understands; on Windows that is the native form.
    if command -v cygpath > /dev/null 2>&1; then cygpath -w "$DEST" >> "$GITHUB_PATH"; else echo "$DEST" >> "$GITHUB_PATH"; fi
fi

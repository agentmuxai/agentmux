#!/usr/bin/env bash
# check-taskfile-dev-cargo-env.sh — CI grep gate: `task dev` never runs a full-LTO release build.
#
# THE RULE: `task dev` and `task dev:standalone` build with the release profile (so every {{.TARGET_DIR}}/release/... path the
# dev flow reads is unchanged) but with the settings in DEV_CARGO_ENV: LTO off, 16 codegen units, opt-level 1, incremental. Full
# LTO with one codegen unit takes 15+ minutes cold and several GB of commit to link, and on a shared host it holds the build slot
# for everyone (#48337). The settings reach cargo through `{{.CARGO_ENV}}`, written in front of each cargo build line; a cargo
# build line without it, or a dev task that stops passing DEV_CARGO_ENV down, silently turns the dev build back into a release
# build, and nothing on screen says so (the build slot's queue shows the same `cargo build --release` either way).
#
# Three checks:
#   1. DEV_CARGO_ENV exists and turns LTO off (CARGO_PROFILE_RELEASE_LTO=off) and raises codegen units.
#   2. Every `cargo build` on a non-comment line carries `{{.CARGO_ENV}}`, so a caller can choose the settings.
#   3. The dev tasks (dev, dev:standalone, dev:serve) pass CARGO_ENV: '{{.DEV_CARGO_ENV}}' to what they build.
# Release builds are not in the Taskfile's cargo lines: the packagers run cargo themselves and take no CARGO_ENV.
#
# Usage:
#   bash scripts/check-taskfile-dev-cargo-env.sh [Taskfile.yml]
# Exit 0 = clean, exit 1 = one of the three checks failed.

set -euo pipefail

FILE="${1:-Taskfile.yml}"
if [ ! -f "$FILE" ]; then
    echo "check-taskfile-dev-cargo-env: $FILE does not exist." >&2
    exit 1
fi

fail=0

# 1. The dev settings.
dev_line="$(grep -E '^[[:space:]]*DEV_CARGO_ENV:' "$FILE" | grep -vE '^[[:space:]]*#' || true)"
if [ -z "$dev_line" ]; then
    echo "check-taskfile-dev-cargo-env: $FILE has no DEV_CARGO_ENV variable." >&2
    fail=1
else
    case "$dev_line" in
        *CARGO_PROFILE_RELEASE_LTO=off*) ;;
        *) echo "check-taskfile-dev-cargo-env: DEV_CARGO_ENV must set CARGO_PROFILE_RELEASE_LTO=off." >&2; fail=1 ;;
    esac
    case "$dev_line" in
        *CARGO_PROFILE_RELEASE_CODEGEN_UNITS=*) ;;
        *) echo "check-taskfile-dev-cargo-env: DEV_CARGO_ENV must set CARGO_PROFILE_RELEASE_CODEGEN_UNITS." >&2; fail=1 ;;
    esac
fi

# 2. Every cargo build line carries the caller's settings.
bare="$(grep -nE 'cargo build' "$FILE" | grep -vE '^[0-9]+:[[:space:]]*#' | grep -vF '{{.CARGO_ENV}}' || true)"
if [ -n "$bare" ]; then
    echo "check-taskfile-dev-cargo-env: a cargo build line in $FILE has no {{.CARGO_ENV}} in front (a dev build would run full LTO):" >&2
    printf '%s\n' "$bare" | cut -c1-200 >&2
    fail=1
fi

# 3. The dev tasks hand DEV_CARGO_ENV down.
block() { # block <task name>: the task's lines, from its header (4-space indent) to the next task header
    awk -v n="$1" '
        $0 ~ "^    " n ":[[:space:]]*$" { on = 1; next }
        on && /^    [^ #][^ ]*:/ { exit }
        on { print }
    ' "$FILE"
}
for t in dev dev:standalone dev:serve; do
    body="$(block "$t")"
    if [ -z "$body" ]; then
        echo "check-taskfile-dev-cargo-env: task $t not found in $FILE." >&2
        fail=1
    elif ! printf '%s\n' "$body" | grep -vE '^[[:space:]]*#' | grep -qF "CARGO_ENV: '{{.DEV_CARGO_ENV}}'"; then
        echo "check-taskfile-dev-cargo-env: task $t does not pass CARGO_ENV: '{{.DEV_CARGO_ENV}}' to what it builds." >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "check-taskfile-dev-cargo-env: $FILE builds dev without full LTO"

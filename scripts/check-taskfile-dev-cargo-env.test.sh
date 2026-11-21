#!/usr/bin/env bash
# Self-test for scripts/check-taskfile-dev-cargo-env.sh: it passes the repo's Taskfile and fails on each way it could be wrong,
# so the gate cannot pass vacuously. Runs anywhere (fixtures in a temp dir).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$REPO_ROOT/scripts/check-taskfile-dev-cargo-env.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

failures=0
expect() { # expect <pass|fail> <name> <file>
    local want="$1" name="$2" file="$3" got
    if bash "$GATE" "$file" >/dev/null 2>&1; then got=pass; else got=fail; fi
    if [ "$got" = "$want" ]; then
        echo "ok   - $name"
    else
        echo "FAIL - $name (wanted $want, got $got)" >&2
        failures=$((failures + 1))
    fi
}

# A fixture: DEV_CARGO_ENV, the three dev tasks, and one build task. Arguments change one thing each.
fixture() { # fixture <file> <dev env line> <build line> <dev:serve passes? 1|0>
    local out="$1" envline="$2" build="$3" serve="$4" pass
    pass="                  CARGO_ENV: '{{.DEV_CARGO_ENV}}'"
    {
        printf 'version: "3"\nvars:\n%s\ntasks:\n' "$envline"
        printf '    dev:\n        cmds:\n            - task: build:backend\n              vars:\n%s\n' "$pass"
        printf '    dev:standalone:\n        cmds:\n            - task: build:backend\n              vars:\n%s\n' "$pass"
        printf '    dev:serve:\n        cmds:\n            - task: build:host\n'
        if [ "$serve" = 1 ]; then printf '              vars:\n%s\n' "$pass"; fi
        printf '    build:backend:\n        cmds:\n            %s\n' "$build"
    } > "$out"
}

GOOD_ENV="    DEV_CARGO_ENV: 'export CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_PROFILE_RELEASE_OPT_LEVEL=1; '"
GOOD_BUILD="- '{{.CARGO_ENV}}cargo build --release -p agentmux-srv'"

# The repo's own Taskfile.
expect pass "the repo's Taskfile.yml" "$REPO_ROOT/Taskfile.yml"

fixture "$TMP/clean.yml" "$GOOD_ENV" "$GOOD_BUILD" 1
expect pass "a clean fixture" "$TMP/clean.yml"

# A comment may name a bare cargo build.
{ cat "$TMP/clean.yml"; printf '            # a bare cargo build --release would be full LTO\n'; } > "$TMP/comment.yml"
expect pass "a comment naming a bare cargo build" "$TMP/comment.yml"

fixture "$TMP/bare.yml" "$GOOD_ENV" "- cargo build --release -p agentmux-srv" 1
expect fail "a cargo build line without {{.CARGO_ENV}}" "$TMP/bare.yml"

fixture "$TMP/no-env.yml" "    OTHER: x" "$GOOD_BUILD" 1
expect fail "no DEV_CARGO_ENV variable" "$TMP/no-env.yml"

fixture "$TMP/lto-on.yml" "    DEV_CARGO_ENV: 'export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16; '" "$GOOD_BUILD" 1
expect fail "DEV_CARGO_ENV that does not turn LTO off" "$TMP/lto-on.yml"

fixture "$TMP/one-cgu.yml" "    DEV_CARGO_ENV: 'export CARGO_PROFILE_RELEASE_LTO=off; '" "$GOOD_BUILD" 1
expect fail "DEV_CARGO_ENV without codegen units" "$TMP/one-cgu.yml"

fixture "$TMP/serve-drops.yml" "$GOOD_ENV" "$GOOD_BUILD" 0
expect fail "dev:serve that stops passing DEV_CARGO_ENV" "$TMP/serve-drops.yml"

# A dev task that is missing altogether.
grep -v 'dev:standalone' "$TMP/clean.yml" > "$TMP/no-standalone.yml"
expect fail "a missing dev:standalone task" "$TMP/no-standalone.yml"

expect fail "a missing file" "$TMP/does-not-exist.yml"

if [ "$failures" -ne 0 ]; then
    echo "check-taskfile-dev-cargo-env self-test: $failures failure(s)" >&2
    exit 1
fi
echo "check-taskfile-dev-cargo-env self-test: all cases behave"

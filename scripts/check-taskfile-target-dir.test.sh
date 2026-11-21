#!/usr/bin/env bash
# Self-test for scripts/check-taskfile-target-dir.sh: it passes the repo's Taskfile and fails on each way it could be wrong,
# so the gate cannot pass vacuously. Runs anywhere (fixtures in a temp dir).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$REPO_ROOT/scripts/check-taskfile-target-dir.sh"
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

GOOD_VAR="    TARGET_DIR: '{{if .CARGO_TARGET_DIR}}{{.CARGO_TARGET_DIR | toSlash}}{{else}}target{{end}}'"

# The repo's own Taskfile.
expect pass "the repo's Taskfile.yml" "$REPO_ROOT/Taskfile.yml"

# A clean fixture: the variable, a step through it, and a comment that names the literal path.
cat > "$TMP/clean.yml" <<EOF
vars:
$GOOD_VAR
tasks:
  a:
    cmds:
      # the packagers read target/release on purpose
      - cp "{{.TARGET_DIR}}/release/agentmux-srv.exe" dist/
EOF
expect pass "a step through TARGET_DIR and a comment naming target/release" "$TMP/clean.yml"

# A literal path in a pwsh step.
cat > "$TMP/literal-pwsh.yml" <<EOF
vars:
$GOOD_VAR
tasks:
  a:
    cmds:
      - cmd: pwsh -Command 'Copy-Item target/release/agentmux-srv.exe dist/bin/x.exe -Force'
EOF
expect fail "a literal target/release in a pwsh step" "$TMP/literal-pwsh.yml"

# A literal path in a bash step, and a debug one, and a backslash one.
n=0
for lit in 'cp target/release/agentmux-mcp.exe "$DEV/"' 'cp target/debug/agentmux-mcp.exe "$DEV/"' 'copy target\release\x.exe dist'; do
    n=$((n + 1))
    printf 'vars:\n%s\ntasks:\n  a:\n    cmds:\n      - %s\n' "$GOOD_VAR" "$lit" > "$TMP/lit$n.yml"
    expect fail "a literal path: $lit" "$TMP/lit$n.yml"
done

# The variable without toSlash (a Windows D:\x would reach bash with backslashes).
cat > "$TMP/no-toslash.yml" <<EOF
vars:
    TARGET_DIR: '{{if .CARGO_TARGET_DIR}}{{.CARGO_TARGET_DIR}}{{else}}target{{end}}'
tasks:
  a:
    cmds:
      - cp "{{.TARGET_DIR}}/release/x" dist/
EOF
expect fail "TARGET_DIR without toSlash" "$TMP/no-toslash.yml"

# A missing file must not pass.
expect fail "a missing file" "$TMP/does-not-exist.yml"

if [ "$failures" -ne 0 ]; then
    echo "check-taskfile-target-dir self-test: $failures failure(s)" >&2
    exit 1
fi
echo "check-taskfile-target-dir self-test: all cases behave"

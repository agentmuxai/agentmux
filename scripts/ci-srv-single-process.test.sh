#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# ci-srv-single-process.test.sh — tests for ci-srv-single-process.sh: it runs the srv test binary from the srv crate's
# folder with one thread and passes its exit code on, finds the binary through `cargo nextest list` (a stub cargo
# here), and says so when that call compiled something.
#
# Usage: bash scripts/ci-srv-single-process.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/ci-srv-single-process.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0
fail=0
ok() { pass=$((pass + 1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# A stand-in test binary: prints where it was run and with what, exits with $WANT.
BIN="$TMP/srv-tests"
cat > "$BIN" <<'STUB'
#!/usr/bin/env bash
echo "ran in $(basename "$PWD") with: $*"
exit "${WANT:-0}"
STUB
chmod +x "$BIN"

out=$(CI_SRV_BINARY="$BIN" bash "$SCRIPT" 2>&1); got=$?
if [ "$got" = 0 ] && grep -q "ran in srv with: --test-threads=1" <<<"$out"; then ok "runs the binary from crates/srv with one thread"; else bad "runs the binary from crates/srv with one thread" "exit $got: $out"; fi

out=$(WANT=101 CI_SRV_BINARY="$BIN" bash "$SCRIPT" 2>&1); got=$?
if [ "$got" = 101 ] && grep -q "exit 101" <<<"$out"; then ok "passes a failing exit code on"; else bad "passes a failing exit code on" "exit $got: $out"; fi

# A stub cargo that answers `nextest list` the way cargo-nextest does.
mkdir -p "$TMP/bin"
cat > "$TMP/bin/cargo" <<STUB
#!/usr/bin/env bash
[ "\$1 \$2" = "nextest list" ] || { echo "unexpected: \$*" >&2; exit 2; }
[ -n "\${STUB_COMPILING:-}" ] && echo "   Compiling agentmux-srv v0.0.0" >&2
printf '{"rust-binaries":{"agentmux-srv-core::lib":{"binary-id":"agentmux-srv-core::lib","binary-name":"agentmux-srv-core","kind":"lib","binary-path":"/nonexistent"},"agentmux-srv::bin/agentmux-srv":{"binary-id":"agentmux-srv::bin/agentmux-srv","binary-name":"agentmux-srv","kind":"bin","binary-path":"$BIN"}}}'
STUB
chmod +x "$TMP/bin/cargo"

out=$(PATH="$TMP/bin:$PATH" RUNNER_TEMP="$TMP" bash "$SCRIPT" 2>&1); got=$?
if [ "$got" = 0 ] && grep -q "ran in srv with: --test-threads=1" <<<"$out" && ! grep -q "::warning::" <<<"$out"; then ok "finds the srv binary through nextest list, quietly when nothing was compiled"; else bad "finds the srv binary through nextest list" "exit $got: $out"; fi

out=$(STUB_COMPILING=1 PATH="$TMP/bin:$PATH" RUNNER_TEMP="$TMP" bash "$SCRIPT" 2>&1); got=$?
if [ "$got" = 0 ] && grep -q "::warning::.*not reused" <<<"$out"; then ok "warns when the list call compiled something"; else bad "warns when the list call compiled something" "exit $got: $out"; fi

printf '%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" = 0 ]

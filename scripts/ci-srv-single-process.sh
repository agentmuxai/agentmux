#!/usr/bin/env bash
# The agentmux-srv binary's tests in ONE process, one thread: how the nightly runs them (`cargo test
# --workspace -- --test-threads=1`), which PR CI's cargo-nextest (one process per test) never does. A test that
# shares a process-wide value with another (the reactive handler, the attachment store, an environment variable,
# a temp path) passes on every PR and fails every night; run here, it fails on the PR that adds it (the nightly's
# four red nights of 2026-11-21 to 11-24).
#
# It builds nothing. The nextest step before it (scripts/ci-nextest-run.sh) built the binary; `cargo nextest list`
# with the same package list resolves to that build and only prints where the binary is, and the binary is run
# directly from the package's own folder, as cargo and nextest run it. A call of `cargo test -p agentmux-srv` here
# would resolve the dependencies' features for a different package set and compile the core and the server layer
# again on a runner that has no room for it (see 4. in ci-nextest-run.sh).
#
#   scripts/ci-srv-single-process.sh
#
# CI_SRV_BINARY=<path> runs that binary instead of looking one up (for scripts/ci-srv-single-process.test.sh).
set -euo pipefail

# Keep this list identical to CRATES in ci-nextest-run.sh: it is what makes `list` resolve to that build.
CRATES="-p agentmux-common -p agentmux-srv-core -p agentmux-srv -p agentmux-launcher -p agentmux-bashwrap -p agentmux-mcp -p agentmux-remote -p agentmux-procstats"

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="${CI_SRV_BINARY:-}"
if [ -z "$bin" ]; then
    log="${RUNNER_TEMP:-/tmp}/srv-single-process-list.log"
    # shellcheck disable=SC2086 # CRATES is a list of arguments
    json="$(cd "$repo" && cargo nextest list --profile ci $CRATES --list-type binaries-only --message-format json 2> "$log")" || {
        cat "$log" >&2
        echo "::error::cargo nextest list failed"
        exit 1
    }
    if grep -q "Compiling" "$log"; then
        # Not an error: the run is right either way. It says the build was not reused, which is what this step's cost depends on.
        echo "::warning::the single-process srv step compiled $(grep -c Compiling "$log") crate(s): the nextest build was not reused"
    fi
    bin="$(printf '%s' "$json" | python3 -c '
import json, sys
bins = json.load(sys.stdin)["rust-binaries"]
hit = [v["binary-path"] for v in bins.values() if v.get("binary-name") == "agentmux-srv" and v.get("kind") == "bin"]
if len(hit) != 1:
    sys.exit("expected one agentmux-srv test binary, found %d" % len(hit))
print(hit[0])
')"
fi
[ -x "$bin" ] || { echo "::error::no test binary at $bin"; exit 1; }

start=$(date +%s)
cd "$repo/crates/srv"
set +e
"$bin" --test-threads=1
status=$?
set -e
echo "srv tests in one process: exit $status after $(( $(date +%s) - start )) s"
exit "$status"

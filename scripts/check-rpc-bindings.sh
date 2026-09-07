#!/usr/bin/env bash
# check-rpc-bindings.sh — CI gate for the generated RPC type bindings.
#
# THE RULE: `frontend/types/rpc/*.ts` is generated from the Rust types in
# `agentmux-srv/src/backend/rpc_types/` by `ts-rs`, and what is committed must
# match what the generator currently produces.
#
# WHY THIS EXISTS
#
# docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md §2.1 counted 334
# "keep in sync with the agentmux-srv RPC" comments across 284 hand-written
# stubs and a 2,819-line hand-maintained `gotypes.d.ts`, plus one drift found
# by hand (the default layout tree, PR #2988 — whose own comment claimed it
# was in sync). A comment cannot fail a build. This can.
#
# Scope today is deliberately small: only the types reachable from commands
# that have been migrated to `RpcEngine::register_typed` carry
# `#[derive(ts_rs::TS)]`, so only those are generated and checked. The set
# grows as the migration proceeds (spec §3.4 step 2); the gate does not need
# to change as it does.
#
# HOW IT WORKS
#
# `#[ts(export)]` makes ts-rs emit one `.ts` per type as a side effect of
# `cargo test` — the crate's `export_bindings_*` tests. So: run those, then
# ask git whether anything under the generated directory changed. A dirty
# tree means the committed bindings are stale.
#
# Usage:
#   bash scripts/check-rpc-bindings.sh
#
# Exit 0 when the committed bindings match the generator, 1 when they drift
# (or when regeneration itself fails).

set -uo pipefail

GEN_DIR="frontend/types/rpc"

cd "$(dirname "$0")/.." || exit 1

# Refuse to run against a tree that is already dirty in the generated
# directory: a pre-existing edit there would be indistinguishable from drift
# this run produced, and the diff below would blame the generator for it.
if ! git diff --quiet -- "$GEN_DIR" 2>/dev/null; then
    echo "check-rpc-bindings: $GEN_DIR has uncommitted changes before regenerating." >&2
    echo "  Commit or stash them first — otherwise this check cannot tell your" >&2
    echo "  edits apart from generator drift." >&2
    exit 1
fi

echo "check-rpc-bindings: regenerating $GEN_DIR ..."
if ! cargo test -p agentmux-srv export_bindings >/dev/null 2>&1; then
    echo "check-rpc-bindings: regeneration FAILED — the export tests did not pass." >&2
    echo "  Run: cargo test -p agentmux-srv export_bindings" >&2
    exit 1
fi

if git diff --quiet -- "$GEN_DIR"; then
    n=$(find "$GEN_DIR" -name '*.ts' 2>/dev/null | wc -l | tr -d ' ')
    echo "check-rpc-bindings: ok ($n generated type(s) current)"
    exit 0
fi

echo "check-rpc-bindings: generated RPC bindings are STALE." >&2
echo "  A Rust type behind a migrated RPC command changed without the" >&2
echo "  committed TypeScript being regenerated." >&2
echo "" >&2
echo "  Fix: cargo test -p agentmux-srv export_bindings && git add $GEN_DIR" >&2
echo "" >&2
echo "  Difference (committed vs regenerated):" >&2
git --no-pager diff --stat -- "$GEN_DIR" >&2
git --no-pager diff -- "$GEN_DIR" | head -60 >&2
exit 1

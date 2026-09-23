#!/usr/bin/env bash
# gen-docs-index.sh — regenerate the machine-maintained half of docs/specs/INDEX.md.
#
# The generator is scripts/gen-docs-index.mjs; this wrapper keeps every
# existing invocation (Taskfile, CI, docs, muscle memory) working:
#
#   bash scripts/gen-docs-index.sh              # rewrite the generated section
#   bash scripts/gen-docs-index.sh --check      # CI: assert, only when this branch touches docs/specs
#   bash scripts/gen-docs-index.sh --check-all  # assert against the whole tree
#
# It was a bash script until 2026-09-23. Ported to Node because it took ~200 s
# per run on Windows (about ten process starts per spec under Git Bash's fork
# emulation), needed bash >= 4 (stock macOS ships 3.2), and could produce
# different output per platform. See
# docs/specs/SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md.
#
# Builtins only: no `dirname` process, so the wrapper adds no Windows spawn cost.
# Either separator: invoked with a native Windows path, BASH_SOURCE carries
# backslashes (C:\...\scripts\gen-docs-index.sh).

src=${BASH_SOURCE[0]:-$0}
case "$src" in
    */* | *\\*) dir=${src%[/\\]*} ;;
    *)          dir=. ;;
esac

if ! command -v node >/dev/null 2>&1; then
    echo "gen-docs-index: node is required but was not found on PATH" >&2
    exit 127
fi

exec node "$dir/gen-docs-index.mjs" "$@"

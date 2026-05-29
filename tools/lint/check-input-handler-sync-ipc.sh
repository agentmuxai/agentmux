#!/usr/bin/env bash
# Synchronous-IPC guardrail for input-handler hot paths.
#
# Enforces invariant I2 from the input-first execution plan (follow-up to
# discussion #1161, built on SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md):
#   "No synchronous IPC on any input path, ever."
#
# A keystroke handler may *dispatch* host work (fire-and-forget
# `invokeCommand(...)` / `getApi()....()` — async, returns immediately) but
# must never *block* on it before the frame paints. The two blocking shapes:
#
#   1. `await invokeCommand(...)` / `await fetch(...)` inside an input handler
#      — stalls the handler on a renderer->host round-trip.
#   2. An `async` keyDownHandler (returns a Promise) — the dispatch layer in
#      store/keymodel.ts treats the return as a synchronous boolean; making it
#      async both breaks that contract and invites awaited IPC.
#   3. Synchronous XHR (`new XMLHttpRequest()` ... `.open(..., false)`) — always
#      blocking; never acceptable on any path, banned repo-wide here.
#
# Audit baseline (2026-05-29): the input path is CLEAN — keyDownHandlers are
# synchronous boolean-returning functions, fire-and-forget invokeCommand is
# compliant, no synchronous XHR. This guard locks that in.
# See docs/analysis/ANALYSIS_KEYDOWN_IPC_AUDIT_2026_05_29.md.
#
# Scope (the input-dispatch surface):
#   - frontend/app/store/keymodel.ts                       — central appHandleKeyDown dispatch
#   - frontend/app/view/term/termViewModel.ts              — terminal keyDownHandler
#   - frontend/app/view/launcher/launcher.tsx              — launcher keyDownHandler
#   - frontend/app/view/agent/components/AgentFooter.tsx   — agent composer
# Plus a repo-wide ban on synchronous XHR.
#
# Escape hatch: any line containing `perf:allow-input-ipc` is skipped, with a
# justification on the same line, e.g.:
#   await invokeCommand("x"); // perf:allow-input-ipc — not on a keystroke path
#
# Exit codes: 0 clean · 1 violations · 2 usage/script error
#
# v1: grep + awk text scan, file-level scope. Coarse but fast (<1s). Escalate
# to an AST-aware ESLint rule if false-positive volume warrants.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Input-dispatch files in scope for the awaited-IPC / async-handler checks.
SCOPE=(
  frontend/app/store/keymodel.ts
  frontend/app/view/term/termViewModel.ts
  frontend/app/view/launcher/launcher.tsx
  frontend/app/view/agent/components/AgentFooter.tsx
)

EXISTING_SCOPE=()
for f in "${SCOPE[@]}"; do
  [[ -f "$f" ]] && EXISTING_SCOPE+=("$f")
done

if [[ ${#EXISTING_SCOPE[@]} -eq 0 ]]; then
  echo "check-input-handler-sync-ipc: no files in scope (paths moved?)" >&2
  exit 2
fi

# Awaited IPC: `await invokeCommand(` / `await fetch(`
AWAIT_IPC='\bawait[[:space:]]+(invokeCommand|fetch)[[:space:]]*\('
# Async key handler: `async keyDownHandler` or `keyDownHandler(...): Promise`
ASYNC_HANDLER='(async[[:space:]]+keyDownHandler|keyDownHandler[^=]*:[[:space:]]*Promise)'
SCOPED_BANNED="($AWAIT_IPC|$ASYNC_HANDLER)"

# Synchronous XHR — repo-wide (frontend).
SYNC_XHR='\.open\([^)]*,[[:space:]]*false[[:space:]]*[,)]'

# awk filter: drop comment lines and escape-hatched lines.
filter_awk='
{
  file = $1; line = $2
  content = ""
  for (i = 3; i <= NF; i++) content = content (i == 3 ? "" : ":") $i
  if (content ~ /perf:allow-input-ipc/) next
  stripped = content
  sub(/^[ \t]+/, "", stripped)
  if (stripped ~ /^\/\//) next
  if (stripped ~ /^\*/)   next
  if (stripped ~ /^\/\*/) next
  print file ":" line ":" content
}'

scoped_hits=$(grep -nHE "$SCOPED_BANNED" "${EXISTING_SCOPE[@]}" 2>/dev/null | awk -F: "$filter_awk" || true)
xhr_hits=$(grep -rnHE "$SYNC_XHR" frontend/app --include='*.ts' --include='*.tsx' 2>/dev/null | awk -F: "$filter_awk" || true)

violations=""
[[ -n "$scoped_hits" ]] && violations+="$scoped_hits"$'\n'
[[ -n "$xhr_hits" ]] && violations+="$xhr_hits"$'\n'
violations="$(printf '%s' "$violations" | sed '/^$/d')"

if [[ -z "$violations" ]]; then
  echo "✓ check-input-handler-sync-ipc: clean (${#EXISTING_SCOPE[@]} dispatch files + repo-wide sync-XHR scan)"
  exit 0
fi

cat >&2 <<'EOF'
✗ check-input-handler-sync-ipc: synchronous-IPC violations on the input path.

Invariant I2 (input-first execution plan): "No synchronous IPC on any input path, ever."
A keystroke handler may DISPATCH host work, but must never BLOCK on it before paint.

Violations:
EOF
echo "$violations" >&2
cat >&2 <<'EOF'

Fixes:
  - Replace `await invokeCommand(...)` with fire-and-forget `invokeCommand(...)`
    (it returns immediately; let the result update state asynchronously).
  - Keep keyDownHandler synchronous (return boolean). Move any async work into a
    dispatched task; don't make the handler `async`.
  - Never use synchronous XHR. Use `fetch` (async) or `invokeCommand`.
  - Genuinely off the keystroke path? Add `// perf:allow-input-ipc — <why>`.

EOF
exit 1

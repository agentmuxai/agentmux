#!/usr/bin/env bash
# Layout-read guardrail for input-handler hot paths.
#
# Enforces rule #3 from docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md:
#   "Never read layout after touching style on the keystroke path."
#
# Scope (per spec §3.1 + §3.2):
#   - frontend/app/view/term/*.{ts,tsx}    — terminal pane (xterm.js)
#   - frontend/app/view/agent/components/AgentFooter.tsx  — agent composer
#
# Banned identifiers (any property access or call):
#   scrollHeight / scrollTop / scrollWidth / scrollLeft
#   offsetHeight / offsetTop / offsetWidth / offsetLeft
#   clientHeight / clientTop / clientWidth / clientLeft
#   getBoundingClientRect() / getClientRects()
#   getComputedStyle()
#
# Why: each of these forces a synchronous reflow if the style was just
# touched (the textbook layout-thrashing pattern). The 22 ms agent
# typing lag fixed in docs/analysis/agent-typing-lag-trace-2026-04-12.md
# was caused by exactly this. Catching the regression at PR time is
# cheaper than chasing it in a trace later.
#
# Escape hatch: any line containing `perf:allow-layout-read` is skipped.
# Use sparingly, with a justification comment on the same line:
#   const h = el.scrollHeight; // perf:allow-layout-read — runs in RAF, not handler
#
# Exit codes:
#   0 — clean
#   1 — violations found (printed to stderr)
#   2 — usage / script error
#
# v1 implementation: grep + awk text scan, file-level scope. Coarse but
# fast (<1s). If false-positive volume becomes annoying, escalate to an
# AST-aware ESLint rule per the execution plan §4 v2 design.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Files in scope. Glob-expanded by the shell.
SCOPE=(
  frontend/app/view/term/*.ts
  frontend/app/view/term/*.tsx
  frontend/app/view/agent/components/AgentFooter.tsx
)

# Filter to existing files only (the glob would otherwise include
# literal patterns if no matches).
EXISTING_SCOPE=()
for f in "${SCOPE[@]}"; do
  [[ -f "$f" ]] && EXISTING_SCOPE+=("$f")
done

if [[ ${#EXISTING_SCOPE[@]} -eq 0 ]]; then
  echo "check-input-handler-layout-reads: no files in scope (directories moved?)" >&2
  exit 2
fi

# Property accesses: \.NAME (followed by a word boundary or open paren)
PROP_PATTERN='\.(scrollHeight|scrollTop|scrollWidth|scrollLeft|offsetHeight|offsetTop|offsetWidth|offsetLeft|clientHeight|clientTop|clientWidth|clientLeft|getBoundingClientRect|getClientRects)\b'
# Function calls: getComputedStyle(
CALL_PATTERN='\bgetComputedStyle[[:space:]]*\('

BANNED="($PROP_PATTERN|$CALL_PATTERN)"

# Collect candidate matches: file:line:content
candidates=$(grep -nHE "$BANNED" "${EXISTING_SCOPE[@]}" 2>/dev/null || true)

if [[ -z "$candidates" ]]; then
  echo "✓ check-input-handler-layout-reads: clean (${#EXISTING_SCOPE[@]} files scanned)"
  exit 0
fi

# Filter:
#   - lines whose content (post leading whitespace) starts with //, *, or /*
#     (comment lines, including JSDoc blocks)
#   - lines containing `perf:allow-layout-read` (escape hatch)
violations=$(echo "$candidates" | awk -F: '
{
  file = $1
  line = $2
  # Reconstruct content (everything after the second colon).
  content = ""
  for (i = 3; i <= NF; i++) content = content (i == 3 ? "" : ":") $i

  # Escape hatch
  if (content ~ /perf:allow-layout-read/) next

  # Strip leading whitespace for comment detection
  stripped = content
  sub(/^[ \t]+/, "", stripped)
  if (stripped ~ /^\/\//) next
  if (stripped ~ /^\*/)   next
  if (stripped ~ /^\/\*/) next

  print file ":" line ":" content
}
')

if [[ -z "$violations" ]]; then
  echo "✓ check-input-handler-layout-reads: clean (${#EXISTING_SCOPE[@]} files scanned; all matches were comments or escape-hatched)"
  exit 0
fi

cat >&2 <<EOF
✗ check-input-handler-layout-reads: layout-read violations in input-handler scope.

Rule #3 from SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md §4:
  "Never read layout after touching style on the keystroke path."

Each of these reads forces a synchronous reflow. In an input handler,
that translates directly to per-keystroke lag (the 22 ms incident).

Violations:
EOF
echo "$violations" >&2
cat >&2 <<EOF

Fixes:
  - For auto-grow textareas: CSS \`field-sizing: content\` (already
    in use in AgentFooter — keep it). No JS scrollHeight read needed.
  - For scroll-to-bottom: \`scrollTo({ top: Number.MAX_SAFE_INTEGER })\`
    — the browser clamps. No \`scrollHeight\` read needed.
  - For sizing in non-handler contexts: read inside a
    \`requestAnimationFrame\` callback that runs a frame AFTER any
    style write, and add \`// perf:allow-layout-read — <why>\` so
    this check skips it.

EOF
exit 1

#!/usr/bin/env bash
# check-scrollbar-cursor.sh — CI grep gate for the scrollbar cursor rule.
#
# docs/retro/retro-scrollbar-cursor-regression-2026-06-17.md
#
# THE RULE: a `::-webkit-scrollbar*` (or OverlayScrollbars `.os-scrollbar*`)
# selector must never set `cursor` to a link/text value. A scrollbar pseudo-
# element INHERITS the `cursor` of its scroll-host element, so a scrollbar over
# the conversation (`cursor: text`) or a clickable tool block (`cursor: pointer`)
# shows the wrong cursor unless the arrow is pinned explicitly. The canonical
# arrow is pinned once in `frontend/app/app.scss` via `cursor: var(--cursor-default)`.
#
# This gate FAILS the build if any scrollbar/os-scrollbar selector block sets
#   cursor: pointer | text | var(--cursor-interactive) | var(--cursor-text)
# (the values that caused the regression). It ALLOWS `cursor: default` /
# `cursor: var(--cursor-default)` (the fix) and allows scrollbar blocks that set
# no cursor at all (they inherit the pinned arrow from app.scss).
#
# Why a grep gate and not stylelint: the rule is BOTH selector-scoped (only
# scrollbar selectors) AND value-scoped (only the link/text values) AND positive
# (the arrow is required, not forbidden). stylelint's
# `rule-selector-property-disallowed-list` can only ban a property name wholesale
# — which is exactly the inverted guard that blocked the fix in the first place.
#
# Usage:
#   bash scripts/check-scrollbar-cursor.sh
# Exit 0 = clean, exit 1 = a forbidden scrollbar cursor was found.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

SEARCH_DIR="frontend"

# Collect every SCSS/CSS file that even mentions a scrollbar selector — no point
# scanning the rest.
files="$(grep -rEl --include='*.scss' --include='*.css' '\-webkit-scrollbar|os-scrollbar' "$SEARCH_DIR" 2>/dev/null || true)"

if [[ -z "$files" ]]; then
    echo "OK: no scrollbar selectors found (nothing to check)."
    exit 0
fi

# awk scanner: track brace depth + whether each depth is inside a scrollbar
# selector (inherited downward into nested rules), buffering tokens between
# block boundaries so multi-line selectors and declarations are seen whole.
# Reports any cursor declaration with a forbidden value while in scrollbar
# context. Emits "file:line: <decl>" per violation to stdout.
report="$(awk '
    function trim(s){ gsub(/^[ \t]+|[ \t]+$/,"",s); return s }
    function checkDecl(   d, val){
        d = trim(buf)
        if (d ~ /^cursor[ \t]*:/ && depth > 0 && ctx[depth]) {
            val = d
            sub(/^cursor[ \t]*:[ \t]*/, "", val)
            # Forbidden: the link hand / text bar, by keyword or by token.
            if (val ~ /pointer/ || val ~ /interactive/ || val ~ /text/) {
                printf "%s:%d: %s\n", FILENAME, FNR, d
            }
        }
    }
    FNR == 1 { depth = 0; buf = ""; for (k in ctx) delete ctx[k] }
    {
        line = $0
        sub(/\/\/.*/, "", line)            # strip // line comments
        gsub(/\/\*.*\*\//, "", line)       # strip single-line /* */ comments
        L = length(line)
        for (i = 1; i <= L; i++) {
            c = substr(line, i, 1)
            if (c == "{") {
                isScroll = (buf ~ /-webkit-scrollbar/ || buf ~ /os-scrollbar/) ? 1 : 0
                if (depth > 0 && ctx[depth]) isScroll = 1   # inherit into nested rules
                depth++
                ctx[depth] = isScroll
                buf = ""
            } else if (c == "}") {
                checkDecl()
                if (depth > 0) { ctx[depth] = 0; depth-- }
                buf = ""
            } else if (c == ";") {
                checkDecl()
                buf = ""
            } else {
                buf = buf c
            }
        }
        buf = buf " "                      # keep tokens separated across lines
    }
' $files || true)"

if [[ -n "$report" ]]; then
    echo "ERROR: forbidden cursor on scrollbar selector(s)." >&2
    echo "" >&2
    echo "A scrollbar is a scroll affordance, not a link or text — it must show" >&2
    echo "the default arrow. Use 'cursor: var(--cursor-default)' (or remove the" >&2
    echo "declaration to inherit the arrow pinned in frontend/app/app.scss);" >&2
    echo "never 'pointer'/'text' (or the --cursor-interactive/--cursor-text tokens)." >&2
    echo "" >&2
    echo "$report" | sed 's/^/  - /' >&2
    echo "" >&2
    echo "See docs/retro/retro-scrollbar-cursor-regression-2026-06-17.md" >&2
    exit 1
fi

echo "OK: no forbidden cursor on any scrollbar/os-scrollbar selector."
exit 0

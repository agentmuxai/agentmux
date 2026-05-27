#!/usr/bin/env bash
# check-menu-positioning.sh — CI grep gate for the menu positioning framework.
#
# SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20 §6.2.
#
# Every DOM menu must route its positioning through `useMenuPosition` /
# `computeMenuPosition` (frontend/app/util/menu-position.ts) so it offsets away
# from BOTH window edges and native browser-pane child windows. This gate
# FAILS the build if a raw floating-ui `computePosition(` call appears in
# `frontend/app/**` outside the sanctioned positioning files — forcing any new
# menu through the shared primitive instead of re-rolling viewport math (the
# inconsistency the spec exists to kill).
#
# Sanctioned files:
#   - util/menu-position.ts       the primitive itself
#   - element/flyoutmenu.tsx      migrated (Phase 2)
#   - element/popover.tsx         migrated (Phase 2)
#   - element/tooltip.tsx         spec Q3 — already correct, migration optional
#
# Grandfathered (NOT menus — pre-date the spec, out of the menu framework's
# scope per spec §3's surface inventory; they are autocomplete/search-result
# dropdowns bound to a text input, not anchored pop-up menus):
#   - suggestion/suggestion.tsx
#   - element/search.tsx
#
# Usage:
#   bash scripts/check-menu-positioning.sh
# Exit 0 = clean, exit 1 = a stray computePosition call was found.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

SEARCH_DIR="frontend/app"

# Files allowed to call computePosition directly. Paths are repo-relative.
ALLOWLIST=(
    "frontend/app/util/menu-position.ts"
    "frontend/app/element/flyoutmenu.tsx"
    "frontend/app/element/popover.tsx"
    "frontend/app/element/tooltip.tsx"
    "frontend/app/suggestion/suggestion.tsx"
    "frontend/app/element/search.tsx"
)

is_allowed() {
    local f="$1"
    for allowed in "${ALLOWLIST[@]}"; do
        [[ "$f" == "$allowed" ]] && return 0
    done
    return 1
}

# Collect every file under frontend/app/** containing a `computePosition(` call.
# grep is the right tool here — this script IS the grep gate.
matches="$(grep -rln --include='*.ts' --include='*.tsx' 'computePosition(' "$SEARCH_DIR" 2>/dev/null || true)"

violations=()
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    # Normalize ./ prefix and backslashes (Windows git-bash).
    f="${f#./}"
    f="${f//\\//}"
    if ! is_allowed "$f"; then
        violations+=("$f")
    fi
done <<< "$matches"

if [[ ${#violations[@]} -gt 0 ]]; then
    echo "ERROR: stray computePosition() call(s) outside the sanctioned menu" >&2
    echo "       positioning files. New menus must route through" >&2
    echo "       useMenuPosition / computeMenuPosition (frontend/app/util/menu-position.ts)." >&2
    echo "" >&2
    for v in "${violations[@]}"; do
        echo "  - $v" >&2
        grep -n 'computePosition(' "$v" 2>/dev/null | sed 's/^/      /' >&2 || true
    done
    echo "" >&2
    echo "If this is a genuinely new menu surface: migrate it to useMenuPosition." >&2
    echo "If it is intentionally exempt: add it to ALLOWLIST in this script with a" >&2
    echo "comment explaining why (see SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20 §6.2)." >&2
    exit 1
fi

echo "OK: no stray computePosition() calls — all menu positioning routes through the framework."
exit 0

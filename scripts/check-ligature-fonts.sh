#!/usr/bin/env bash
# check-ligature-fonts.sh — CI grep gate against shipping/naming ligature fonts.
#
# docs/retro/retro-terminal-consecutive-period-input-loss-2026-09-15.md
#
# THE RULE: no CSS font stack may NAME a programming-ligature font, and no such
# font may be bundled in public/fonts/.
#
# WHY: coding fonts implement ligatures like `..` `::` `==` `->` by substituting
# one character for a BLANK glyph (no contours) and the other for a composed
# glyph drawn leftward, keeping the monospace cell grid intact. They guard this
# to runs of 2-3 with no-op contextual rules placed ahead of the substituting
# ones. CEF 152's shaper stopped honoring those guards and applied the
# substitution across whole runs, so holding "." rendered as a stretch of blank
# cells with a single dot at the end ("            .") in every text surface,
# while the underlying value kept every character.
#
# Deleting the bundled woff2 is NOT sufficient on its own, which is the specific
# trap this gate exists for (reagent P1 on PR #3247): a CSS stack that still
# names "JetBrains Mono" resolves against a SYSTEM-INSTALLED copy, and it is one
# of the most commonly installed developer fonts. The name must go too.
#
# Disabling `calt` in CSS is NOT an acceptable alternative: the `font` shorthand
# resets font-variant-ligatures/font-feature-settings, this codebase has ~19
# `font:` shorthand declarations, and each one re-enables the feature on its
# element at a specificity a global override cannot beat. Two attempts at that
# failed review before the font itself was removed.
#
# If you need ligatures back, do it as an explicit opt-in setting (VS Code's
# `editor.fontLigatures` model) that a user turns on for a font they chose —
# not as a default baked into a fallback stack.
#
# Usage:
#   bash scripts/check-ligature-fonts.sh
# Exit 0 = clean, exit 1 = a ligature font is named in CSS or bundled.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$REPO_ROOT"

# Families whose shipped builds carry programming ligatures via `calt`/`liga`.
# Hack (our bundled mono) has no `calt` table at all and is deliberately absent.
LIGATURE_FONTS='JetBrains Mono|Fira Code|Cascadia Code|Iosevka|Victor Mono|Monoid|Hasklig'

fail=0

# 1) CSS/SCSS must not name one in a font stack.
#
#    Comments are excluded so the explanatory notes that cite these fonts by
#    name (including the one in tailwindsetup.css warning you not to add them)
#    stay legal. That exclusion must handle MULTI-LINE block comments: an
#    earlier version only skipped lines *beginning* with `//`, `/*` or `*`,
#    which false-positived on a continuation line inside a `/* … */` block --
#    caught when this gate failed on its own documentation.
#
#    Comments are blanked out rather than deleted so reported line numbers
#    still match the real file.
strip_comments() {
    perl -0777 -pe '
        s{/\*.*?\*/}{ my $c = $&; $c =~ s/[^\n]/ /g; $c }ges;   # block comments
        s{(^|[^:])//[^\n]*}{ my $c = $&; $c =~ s{//[^\n]*}{ " " x length($&) }e; $c }gme;
    ' "$1"
}

css_hits=""
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    hits="$(strip_comments "$f" | grep -nE "$LIGATURE_FONTS" || true)"
    [[ -n "$hits" ]] && css_hits+="$(echo "$hits" | sed "s|^|$f:|")"$'\n'
done < <(find frontend -type f \( -name '*.scss' -o -name '*.css' \) 2>/dev/null)

css_hits="$(echo "$css_hits" | sed '/^$/d')"

if [[ -n "$css_hits" ]]; then
    echo "FAIL: a programming-ligature font is named in a CSS font stack."
    echo "      These resolve against a system-installed copy even when we do"
    echo "      not bundle the font, reintroducing the CEF 152 blank-glyph bug."
    echo
    echo "$css_hits"
    echo
    fail=1
fi

# 2) None may be bundled.
font_hits="$(find public/fonts -type f \( -iname '*jetbrains*' -o -iname '*firacode*' -o -iname '*fira-code*' \
    -o -iname '*cascadia*' -o -iname '*iosevka*' -o -iname '*victormono*' -o -iname '*monoid*' -o -iname '*hasklig*' \) 2>/dev/null || true)"

if [[ -n "$font_hits" ]]; then
    echo "FAIL: a programming-ligature font is bundled in public/fonts/:"
    echo "$font_hits"
    echo
    fail=1
fi

if [[ "$fail" -ne 0 ]]; then
    echo "check-ligature-fonts: FAILED"
    echo "see docs/retro/retro-terminal-consecutive-period-input-loss-2026-09-15.md"
    exit 1
fi

echo "check-ligature-fonts: ok"

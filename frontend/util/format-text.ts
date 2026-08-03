// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared string-truncation display formatting — consolidates 3 independently-
 * duplicated implementations found across `block/autotitle.ts` (dead code,
 * removed outright — see below), `view/drone/drone-view.tsx`, and
 * `AgentFooter.tsx`'s `abbreviateArg`.
 *
 * `autotitle.ts`'s `truncate` turned out to be genuinely unused (no call
 * sites anywhere in that file) — deleted rather than migrated.
 *
 * The remaining two disagreed on more than just the ellipsis character:
 * `AgentFooter.tsx`'s `abbreviateArg` left-truncates any string containing
 * `/`/`\` to preserve a trailing filename — the right call for its actual
 * inputs (tool args, overwhelmingly file paths), but NOT automatically safe
 * to apply to `drone-view.tsx`'s inputs (URLs, boolean expressions,
 * templates), where a `/` is often incidental and left-truncating would hide
 * the more useful prefix. `pathAware` is therefore an explicit opt-in
 * (default `false`, matching `drone-view.tsx`'s original plain-truncate
 * behavior) rather than an automatic per-string heuristic — the earlier
 * draft of this function auto-detected `/`/`\` and would have silently
 * changed `drone-view.tsx`'s URL truncation direction.
 *
 * See docs/specs/SPEC_TOKEN_STATS_NUMBER_FORMATTING_2026_08_02.md §7.2.
 */

/**
 * Truncate `s` to at most `max` characters (including the ellipsis), using a
 * real "…" character (both prior duplicates that used a literal "..." are
 * gone — one was dead code, the other never shipped that bug). With
 * `pathAware: true`, left-truncates instead of right-truncating so a
 * trailing filename survives — opt in only for inputs that are actually
 * file paths.
 */
export function abbreviateText(s: string, max: number, opts?: { pathAware?: boolean }): string {
    if (s.length <= max) return s;
    if (opts?.pathAware) {
        return "…" + s.slice(-(max - 1));
    }
    return s.slice(0, max - 1) + "…";
}

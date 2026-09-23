// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared resolution of a terminal's font family — the CSS font-family
 * string `term.tsx` and (once wired) `AgentShellSubblock.tsx` hand xterm.js.
 *
 * Mirrors `termscrollback.ts`/`termscrollsensitivity.ts`: pulled out so the
 * value can be recomputed reactively (`createMemo`) for a live-apply effect,
 * not just resolved once inside `onMount` the way it was before
 * SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md §6.1.
 *
 * Precedence is `term:fontfamily` setting, THEN a per-connection default,
 * THEN the hardcoded fallback — preserved exactly as `term.tsx` already
 * had it (`ts?.["term:fontfamily"] ?? connFontFamily ?? "Hack"`).
 * Deliberately NOT reconciled with `term:fontsize`'s own precedence
 * (per-block meta, then connection, then setting — the opposite order):
 * that's a pre-existing inconsistency between the two settings, and
 * fixing it is a real behavior change unrelated to "make this live",
 * out of scope here.
 */
export const DEFAULT_TERM_FONT_FAMILY = "Hack";

/** A bag of `term:*` settings, as returned by `getSettingsPrefixAtom("term")`. */
export type TermSettingsLike = Record<string, unknown> | null | undefined;

export function resolveTermFontFamily(termSettings: TermSettingsLike, connFontFamily?: string | null): string {
    const fromSettings = termSettings?.["term:fontfamily"];
    if (typeof fromSettings === "string") return fromSettings;
    if (typeof connFontFamily === "string") return connFontFamily;
    return DEFAULT_TERM_FONT_FAMILY;
}

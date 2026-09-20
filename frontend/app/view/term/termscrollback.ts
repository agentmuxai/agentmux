// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared resolution of a terminal's scrollback depth.
 *
 * Every xterm surface in the app must size its scrollback the same way, or the
 * `term:scrollback` setting silently means different things in different panes.
 * That drift is exactly what this module exists to prevent: the agent Shell
 * drawer (`AgentShellSubblock.tsx`) was built as a spike with a hardcoded
 * `scrollback: 2000` and never read the setting at all, so a user who raised
 * `term:scrollback` got a deeper buffer in terminal panes while the drawer
 * stayed capped — and a long agent session lost the top of its own history
 * with no indication why.
 *
 * Note the unit: xterm counts scrollback in *display rows*, not logical lines.
 * A narrow surface soft-wraps one logical line across several rows, so the same
 * row budget yields noticeably less history in a narrow drawer than in a wide
 * pane. Trimming is also per-row, which is why an overrun leaves the topmost
 * visible line cut through its middle rather than at a clean line boundary.
 */

/** Used when neither the settings value nor a per-block override applies. */
export const DEFAULT_TERM_SCROLLBACK = 2000;

/**
 * Upper bound applied after the setting and any override are resolved.
 *
 * Preserves the clamp `term.tsx` has always applied. Note this is lower than
 * the 100000 the settings UI (`terminal-section.tsx`) accepts — a value above
 * this is silently reduced. Deliberately left as-is here so that centralizing
 * the logic changes no existing behavior; raising it is a memory-footprint
 * decision, not a refactor.
 */
export const MAX_TERM_SCROLLBACK = 50000;

/** A bag of `term:*` settings, as returned by `getSettingsPrefixAtom("term")`. */
export type TermSettingsLike = Record<string, unknown> | null | undefined;

/** A block's `meta`, which may carry a per-block `term:scrollback` override. */
export type BlockMetaLike = Record<string, unknown> | null | undefined;

function readPositiveInt(source: TermSettingsLike | BlockMetaLike): number | null {
    const raw = source?.["term:scrollback"];
    // Guard the type rather than relying on truthiness: a malformed settings
    // file can hold a string here, and `Math.floor("abc")` is NaN — which xterm
    // accepts and then misbehaves on. 0 and negatives fall through to the
    // previous value, matching the original truthiness check.
    if (typeof raw !== "number" || !Number.isFinite(raw) || raw <= 0) return null;
    return Math.floor(raw);
}

/**
 * Resolve scrollback depth in display rows.
 *
 * Precedence: per-block `meta` override > `term:scrollback` setting > default.
 */
export function resolveTermScrollback(termSettings: TermSettingsLike, blockMeta?: BlockMetaLike): number {
    const fromSettings = readPositiveInt(termSettings);
    const fromMeta = readPositiveInt(blockMeta);
    const resolved = fromMeta ?? fromSettings ?? DEFAULT_TERM_SCROLLBACK;
    return Math.max(0, Math.min(resolved, MAX_TERM_SCROLLBACK));
}

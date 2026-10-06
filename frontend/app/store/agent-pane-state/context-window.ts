// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Model-name context windows: the SEED for the agent-pane context meter, used
 * only until Claude Code reports the real window.
 *
 * The authoritative window is the CLI's own: every `result` frame carries
 * `modelUsage[<model id>].contextWindow` (verified on CLI 2.1.288 — e.g.
 * `claude-sonnet-5-5` → 1,000,000). It accounts for things a model name
 * cannot (the Sonnet 4.x 1M beta, a `[1m]` model, the API provider), so it
 * always wins — see context-reading.ts, which combines the two. This table
 * covers the interval before the pane's first `result` (and replayed history
 * from a CLI old enough to omit the field). `system/init` names the model but
 * carries no window.
 *
 * The table is not uniform within a family: Opus/Fable/Mythos are 1M, Haiku is
 * 200K, Sonnet 5+ is 1M, and Sonnet 4.x is 200K unless the 1M beta is on. For
 * the beta-gated 4.x models we seed conservatively and LEARN upward: a prompt
 * can never exceed the real window, so a larger observed prompt proves the
 * next known tier.
 *
 * Spec: docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md §5 P1;
 * docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md §3.
 */

/** Known Claude window tiers, ascending — the high-water upgrade promotes to these. */
export const KNOWN_CONTEXT_WINDOW_TIERS: readonly number[] = [200_000, 1_000_000];

/** The largest known tier. A prompt larger than this with no reported window
 *  is not a believable context size (see context-reading.ts). */
export const MAX_KNOWN_CONTEXT_WINDOW = KNOWN_CONTEXT_WINDOW_TIERS[KNOWN_CONTEXT_WINDOW_TIERS.length - 1];

/** Usable buffer below the raw window before the CLI auto-compacts (~33K):
 *  threshold ≈ effectiveWindow − 13K, effectiveWindow = window − min(maxOutput, 20K). */
const COMPACTION_BUFFER = 33_000;

/**
 * Seed window from a resolved model id/alias. Returns `undefined` for models we
 * don't recognise (non-Claude, or future ids) — the window is then unknown
 * until the CLI reports one. Sonnet 4.x and earlier seed CONSERVATIVELY at
 * 200K; the high-water upgrade promotes to 1M the moment context exceeds 200K
 * (its beta-gated ceiling). Sonnet 5+ has no beta gate — it seeds at 1M
 * directly. A `[1m]` suffix (Claude Code's own spelling of the 1M variant of a
 * model) is 1M whatever the family.
 */
export function contextWindowForModel(model: string | null | undefined): number | undefined {
    if (!model) return undefined;
    const m = model.toLowerCase();
    if (m.endsWith("[1m]")) return 1_000_000;
    if (m.includes("haiku")) return 200_000;
    if (m.includes("opus") || m.includes("fable") || m.includes("mythos")) return 1_000_000;
    if (m.includes("sonnet")) return sonnetMajorVersion(m) >= 5 ? 1_000_000 : 200_000;
    return undefined;
}

/**
 * Major version number following "sonnet" in a model id or alias, e.g.
 * "claude-sonnet-5" → 5, "claude-sonnet-4-6" → 4. The bare family alias
 * ("sonnet") carries no version digits and returns 0 — treated as pre-5
 * (conservative) since the CLI resolves it to whatever the current pin is,
 * and we can't tell which from the string alone. The meter never needs the
 * alias in practice: it is fed resolved ids from the stream.
 */
function sonnetMajorVersion(m: string): number {
    const match = m.match(/sonnet-(\d+)/);
    return match ? parseInt(match[1], 10) : 0;
}

/**
 * Smallest known tier strictly greater than `n`, or `undefined` when `n` is
 * above every tier. A prompt of `n` tokens was accepted, so the real window is
 * at least this. Never `n` itself: promoting the window to an observed prompt
 * size would let one bad reading set the window to that reading forever
 * ("17m / 17m"). context-reading.ts records what it proves per model.
 */
export function nextTierAbove(n: number): number | undefined {
    for (const t of KNOWN_CONTEXT_WINDOW_TIERS) if (t > n) return t;
    return undefined;
}

/**
 * Approximate auto-compaction threshold for a window. We band the meter against
 * this (not the raw window) so "full" means "about to compact" — the number the
 * user actually cares about.
 */
export function compactionThreshold(window: number): number {
    return Math.max(1, window - COMPACTION_BUFFER);
}

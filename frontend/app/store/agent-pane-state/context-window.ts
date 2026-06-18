// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Per-model context-window resolution for the agent-pane context meter.
 *
 * The window is NOT a per-provider constant: the Claude provider spans
 * Opus/Sonnet (1M) and Haiku (200K), and Sonnet itself is 200K by default but
 * 1M with the `context-1m-2025-08-07` beta. The CLI never reports the effective
 * window (verified — `system/init` and `result` carry `model` but no window
 * field), so we (a) SEED from the resolved model id the stream reports and
 * (b) LEARN upward from observed usage: a prompt can never exceed the real
 * window, so if it ever does, promote to the next known tier (this is what
 * catches Sonnet-1M).
 *
 * Spec: docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md §5 P1.
 * Follow-ups (not here): seed the catalog from the Anthropic Models API on CLI
 * install; calibrate the exact window at the compaction boundary.
 */

/** Known Claude window tiers, ascending — the high-water upgrade promotes to these. */
const TIERS = [200_000, 1_000_000];

/** Usable buffer below the raw window before the CLI auto-compacts (~33K):
 *  threshold ≈ effectiveWindow − 13K, effectiveWindow = window − min(maxOutput, 20K). */
const COMPACTION_BUFFER = 33_000;

/**
 * Seed window from a resolved model id/alias. Returns `undefined` for models we
 * don't recognise (non-Claude, or future ids) — the caller falls back to the
 * provider's static window. Sonnet seeds CONSERVATIVELY at 200K; the high-water
 * upgrade promotes it to 1M the moment context exceeds 200K (its beta-gated ceiling).
 */
export function contextWindowForModel(model: string | null | undefined): number | undefined {
    if (!model) return undefined;
    const m = model.toLowerCase();
    if (m.includes("haiku")) return 200_000;
    if (m.includes("opus") || m.includes("fable") || m.includes("mythos")) return 1_000_000;
    if (m.includes("sonnet")) return 200_000; // conservative seed; learns up to 1M
    return undefined;
}

/** Smallest known tier strictly greater than `n`; `n` itself if above all tiers. */
function nextTierAbove(n: number): number {
    for (const t of TIERS) if (t > n) return t;
    return n;
}

/**
 * Resolve the window after observing a prompt of `observed` tokens for `model`.
 * Learn-up-only: never shrinks within a session (model/beta are fixed per
 * session; a new session re-seeds). Returns `undefined` only when we have
 * neither a prior value nor a recognised model (caller uses the provider fallback).
 */
export function learnContextWindow(
    prev: number | null | undefined,
    observed: number,
    model: string | null | undefined,
): number | undefined {
    let w = prev ?? contextWindowForModel(model);
    if (w == null) return undefined;
    // A prompt can't exceed the real window: if it did, our assumption is too low.
    if (observed > w) w = nextTierAbove(observed);
    return w;
}

/**
 * Approximate auto-compaction threshold for a window. We band the meter against
 * this (not the raw window) so "full" means "about to compact" — the number the
 * user actually cares about.
 */
export function compactionThreshold(window: number): number {
    return Math.max(1, window - COMPACTION_BUFFER);
}

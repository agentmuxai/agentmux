// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How full the context window is and what the strip says about it: the fill band, the tooltip, and
 * the countdown to auto-compaction. Pure functions of the reading.
 */

import { compactionThreshold } from "@/app/store/agent-pane-state/context-window";
import type { AutoCompactPoint } from "@/app/store/agent-pane-state/auto-compact";
import { formatCompactNumber, formatExactNumber } from "@/util/format-count";

export type CtxBand = "low" | "mid" | "high" | "critical";

/** The auto-compact point to measure against: the one given, else the CLI's
 *  default for the window (window − 33K), assumed. */
function pointFor(contextWindow: number, point: AutoCompactPoint | null | undefined): AutoCompactPoint {
    return point ?? { kind: "at", tokens: compactionThreshold(contextWindow), source: "assumed" };
}

/** How close the reading is to auto-compacting; against the window itself when
 *  auto-compaction is off (the CLI then stops at the window). */
export function ctxBand(tokens: number, contextWindow: number, point?: AutoCompactPoint | null): CtxBand {
    const p = pointFor(contextWindow, point);
    const fraction = tokens / (p.kind === "at" ? p.tokens : contextWindow);
    if (fraction >= 0.9) return "critical";
    if (fraction >= 0.75) return "high";
    if (fraction >= 0.5) return "mid";
    return "low";
}

export function contextTitle(
    tokens: number,
    contextWindow: number | undefined,
    note: string | undefined,
    point?: AutoCompactPoint | null,
): string {
    const tail = note ? `\n${note}` : "";
    if (contextWindow == null) {
        return `Context: ${formatExactNumber(tokens)} tokens${tail}`;
    }
    const pct = ((tokens / contextWindow) * 100).toFixed(1);
    const p = pointFor(contextWindow, point);
    const compactLine =
        p.kind === "off"
            ? `Auto-compaction is off for this session; a manual /compact still works.`
            : `Auto-compacts ${p.source === "reported" ? "at" : "around"} ${formatExactNumber(p.tokens)} tokens ` +
              `(≈${formatExactNumber(Math.max(0, p.tokens - tokens))} tokens left)` +
              `${p.source === "reported" ? ", as the CLI reports it" : ""}.\n` +
              `Applies to auto-compaction only — a manual /compact can happen at any fill level.`;
    return (
        `Context window: ${formatExactNumber(tokens)} / ${formatExactNumber(contextWindow)} tokens (${pct}%)\n` +
        `This is the conversation sent to the model on its latest call.\n` +
        compactLine +
        tail
    );
}

/**
 * Tier 3 of docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md
 * §7 — explicit countdown language, surfaced inline (not hover-gated) once
 * the fill level is worth calling out (mid band and above). Predicts only
 * the `auto` trigger: `compactionThreshold(window) − tokens` is the
 * distance to the CLI's own auto-compact point, computable every turn
 * (Tier 0's groundwork) — a manual `/compact` can happen at any fill level
 * and is fundamentally unpredictable, so this must never be read as "the"
 * time compaction will happen, only as "no sooner than."
 */
export function compactionCountdownText(tokens: number, window: number, point?: AutoCompactPoint | null): string | null {
    const p = pointFor(window, point);
    if (p.kind === "off") return null;
    const band = ctxBand(tokens, window, p);
    if (band === "low") return null;
    const remaining = Math.max(0, p.tokens - tokens);
    return `~${formatCompactNumber(remaining)} to auto-compact`;
}

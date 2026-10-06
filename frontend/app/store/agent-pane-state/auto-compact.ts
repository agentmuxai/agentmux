// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the context meter counts down to: the prompt size at which Claude Code
 * auto-compacts.
 *
 * The CLI reports it (srv asks `get_context_usage` at spawn and every turn
 * boundary and publishes `agentcontextusage`, crates/srv/src/backend/
 * agent_context_usage.rs). It differs from the default window − 33K under
 * `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, the `autoCompactWindow` setting or
 * `CLAUDE_CODE_DISABLE_1M_CONTEXT`, and auto-compaction can be off. Until the
 * CLI has answered for the reading's model, the default is assumed and said to
 * be. docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md §9.1.
 */

import type { ContextReading } from "./context-reading";
import { compactionThreshold } from "./context-window";

/** The `agentcontextusage` event, as the meter uses it. */
export interface AutoCompactReport {
    /** The model the CLI answered for (a raw id, e.g. `claude-haiku-4-5-20251001`). */
    model: string | null;
    /** The window auto-compaction is measured against. */
    window: number | null;
    /** The prompt size that triggers auto-compaction; null when it is off. */
    threshold: number | null;
    enabled: boolean;
}

/** Where auto-compaction happens for a reading. */
export type AutoCompactPoint =
    | { kind: "at"; tokens: number; source: "reported" | "assumed" }
    /** The CLI said auto-compaction is off: there is nothing to count down to. */
    | { kind: "off" };

function positiveInt(v: unknown): number | null {
    return typeof v === "number" && Number.isInteger(v) && v > 0 ? v : null;
}

/** Parse an `agentcontextusage` event's data; null if it isn't one. */
export function parseAutoCompactReport(data: unknown): AutoCompactReport | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    if (typeof d.auto_compact_enabled !== "boolean") return null;
    return {
        model: typeof d.model === "string" && d.model ? d.model : null,
        window: positiveInt(d.auto_compact_window),
        threshold: d.auto_compact_enabled ? positiveInt(d.auto_compact_threshold) : null,
        enabled: d.auto_compact_enabled,
    };
}

/** A model id without case, a `[1m]` suffix or a `-YYYYMMDD` date stamp: the CLI
 *  names `claude-haiku-4-5-20251001` where the API reply says `claude-haiku-4-5`. */
function modelKey(model: string): string {
    return model
        .toLowerCase()
        .replace(/\[1m\]$/, "")
        .replace(/-\d{8}$/, "");
}

/** Whether two model ids name the same model. */
export function sameModel(a: string | null | undefined, b: string | null | undefined): boolean {
    return !!a && !!b && modelKey(a) === modelKey(b);
}

/**
 * The auto-compact point for `reading`: the CLI's own, when its report is for
 * the model the reading was measured on; else window − 33K, assumed. None when
 * the window is unknown, including after a model switch (the new model's
 * process reports at its spawn).
 */
export function autoCompactPoint(
    reading: ContextReading | null,
    report: AutoCompactReport | null,
): AutoCompactPoint | null {
    if (!reading || reading.switchedTo != null) return null;
    if (report && sameModel(report.model, reading.model)) {
        if (!report.enabled) return { kind: "off" };
        if (report.threshold != null) return { kind: "at", tokens: report.threshold, source: "reported" };
    }
    if (reading.window == null) return null;
    return { kind: "at", tokens: compactionThreshold(reading.window), source: "assumed" };
}

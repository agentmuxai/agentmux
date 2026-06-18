// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ToolCallEvent, ToolResultEvent } from "../types";

/**
 * Shared tool-call ↔ tool-result correlation for the stream translators.
 *
 * Every provider's wire protocol splits a tool invocation from its later
 * result, and the result frame typically carries only the call id — not
 * the tool name. So each translator has to remember `id → name` when the
 * call arrives, look it up when the result arrives, and emit the
 * provider-agnostic `tool_call` / `tool_result` events in an identical
 * shape. That bookkeeping (the map, its reset, and the two event
 * literals) was copy-pasted across the codex, gemini, kimi and acp
 * translators. Collecting it here makes "the tool name didn't resolve" a
 * single-place fix; the genuinely provider-specific part — pulling the
 * name / id / params out of each wire format — stays in the translators.
 *
 * See docs/analysis/ANALYSIS_CODEBASE_ARCHITECTURE_AUDIT_2026_06_18.md (A7).
 */
export class ToolCorrelator {
    private nameById = new Map<string, string>();

    /** Remember `id → name` and build the agnostic `tool_call` event. */
    call(name: string, id: string, params: Record<string, any>): ToolCallEvent {
        this.nameById.set(id, name);
        return { type: "tool_call", tool: name, id, params };
    }

    /**
     * Build the agnostic `tool_result` event, resolving the tool name
     * from the id remembered at call time. `fallbackName` covers a
     * result whose call was never seen (e.g. it predates a reset /
     * reconnect, or the provider also carries the name on the result).
     */
    result(
        id: string,
        status: ToolResultEvent["status"],
        result: any,
        fallbackName = "unknown",
    ): ToolResultEvent {
        return {
            type: "tool_result",
            tool: this.nameById.get(id) ?? fallbackName,
            id,
            status,
            result,
        };
    }

    /** Forget all correlations (between sessions). */
    reset(): void {
        this.nameById.clear();
    }
}

/**
 * Normalise a tool's output into the `result` record used by
 * `tool_result`: a string is wrapped as `{ output }`; anything else
 * (already-structured output) is passed through unchanged.
 */
export function wrapOutput(output: any): any {
    return typeof output === "string" ? { output } : output;
}

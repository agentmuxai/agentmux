// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The notice for memory delivered through Claude Code's `SessionStart` hook
 * (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2). The sidecar
 * writes one persisted `agentmux_memory_injected` frame into the pane's
 * output once every part of a delivery reached the CLI
 * (`server/memory_delivery_handlers.rs`); it names each entry and its size
 * (owner decision D11). The model already has the content — the frame is
 * only the label, rendered as the same `MemoryReinjectionNode` the hidden
 * reinjection produces.
 *
 * Shared by the live stream (`useAgentStream.ts`) and history replay
 * (`parseHistoryLines.ts`): the node id is the frame's own `id`, stable for
 * the delivery, so a live frame and a replay of the same line render once.
 */

import { FALLBACK_CONTEXT_WINDOW, memorySizeBand } from "./memory-reinjection";
import type { MemoryReinjectionNode } from "./types";

export const MEMORY_INJECTED_SUBTYPE = "agentmux_memory_injected";

export function isMemoryInjectedFrame(rawEvent: unknown): boolean {
    if (!rawEvent || typeof rawEvent !== "object") return false;
    const e = rawEvent as Record<string, unknown>;
    return e.type === "system" && e.subtype === MEMORY_INJECTED_SUBTYPE;
}

/**
 * The notice node for an `agentmux_memory_injected` frame, or `null` when
 * `rawEvent` isn't one. `contextWindow` bands Personal Memory's size the way
 * the hidden reinjection's notice does; replay passes the fallback window.
 * `now` stands in when the frame carries no parseable `timestamp`.
 */
export function buildMemoryInjectedNode(
    rawEvent: unknown,
    opts: { contextWindow?: number; now: number },
): MemoryReinjectionNode | null {
    if (!isMemoryInjectedFrame(rawEvent)) return null;
    const e = rawEvent as Record<string, unknown>;

    const perEntryTokens: MemoryReinjectionNode["perEntryTokens"] = (Array.isArray(e.entries) ? e.entries : [])
        .filter((x): x is Record<string, unknown> => !!x && typeof x === "object")
        .map((x) => ({
            label: typeof x.label === "string" ? x.label : "",
            source: x.source === "personal" ? ("personal" as const) : ("global" as const),
            tokens: typeof x.tokens === "number" ? x.tokens : 0,
            sizeBytes: typeof x.size_bytes === "number" ? x.size_bytes : 0,
        }));

    const of = (source: "global" | "personal") => perEntryTokens.filter((p) => p.source === source);
    const sum = (list: typeof perEntryTokens, pick: (p: (typeof perEntryTokens)[number]) => number) =>
        list.reduce((total, p) => total + pick(p), 0);
    const [global, personal] = [of("global"), of("personal")];

    const timestamp = typeof e.timestamp === "string" ? e.timestamp : null;
    const parsed = timestamp != null ? Date.parse(timestamp) : NaN;
    const at = Number.isNaN(parsed) ? opts.now : parsed;

    return {
        type: "memory_reinjection",
        id: typeof e.id === "string" && e.id.length > 0 ? e.id : `memory-injected-${timestamp ?? "notime"}`,
        globalMemoryCount: global.length,
        personalMemoryCount: personal.length,
        estimatedTokens: sum(perEntryTokens, (p) => p.tokens),
        perEntryTokens,
        totalSizeBytes: { global: sum(global, (p) => p.sizeBytes), personal: sum(personal, (p) => p.sizeBytes) },
        sizeBand: memorySizeBand(sum(personal, (p) => p.tokens), opts.contextWindow ?? FALLBACK_CONTEXT_WINDOW),
        at,
    };
}

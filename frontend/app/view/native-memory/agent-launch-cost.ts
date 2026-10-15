// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent starts with (`memory:launchcost`), grouped and measured
 * against its context window (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.3, §5.4).
 * A window is shown only when it is known: the reading the agent's open pane
 * mirrors to block meta (`agent:context`). It is never guessed.
 */

import { contextReadingFromMeta, plausibleReading } from "@/app/store/agent-pane-state/context-reading";
import { compactionThreshold } from "@/app/store/agent-pane-state/context-window";
import { DEFAULT_MEMORY_SIZE_FRACTION, memorySizeBand, type SizeBand } from "@/app/view/agent/memory-reinjection";
import type { LaunchCost } from "@/types/rpc/LaunchCost";
import type { LaunchCostPart } from "@/types/rpc/LaunchCostPart";
import { formatCompactNumber } from "@/util/format-count";
import { shareLabel } from "./memory-tokens";

/** The bar's groups, in launch order. */
export const LAUNCH_GROUPS: { key: string; label: string; kinds: string[] }[] = [
    { key: "operator", label: "Operator Config", kinds: ["operator_config"] },
    { key: "workspace", label: "Workspace", kinds: ["workspace_global"] },
    { key: "bundles", label: "Bundles", kinds: ["bundle"] },
    { key: "agent", label: "Agent", kinds: ["soul", "agentmd", "agent_memory", "skills_index", "formatting"] },
    { key: "personal", label: "Personal Memory", kinds: ["personal"] },
];

/** `cost`'s tokens by group, leaving out the empty ones. */
export function launchGroups(cost: LaunchCost): { key: string; label: string; tokens: number }[] {
    return LAUNCH_GROUPS.map((g) => ({
        key: g.key,
        label: g.label,
        tokens: cost.parts.filter((p) => g.kinds.includes(p.kind)).reduce((n, p) => n + p.tokens_est, 0),
    })).filter((g) => g.tokens > 0);
}

/** How a part reaches the agent. */
export function viaLabel(part: Pick<LaunchCostPart, "via">): string {
    switch (part.via) {
        case "session_hook":
            return "Session hook";
        case "cli":
            return "Loaded by Claude Code";
        default:
            return "Startup file";
    }
}

/** An agent's context window and the model it was reported for. */
export interface AgentWindow {
    window: number;
    model: string | null;
}

/** The window an agent's open panes last reported, from their block meta;
 *  null when none has a reading with a window. */
export function windowFromMetas(metas: (Record<string, unknown> | null | undefined)[]): AgentWindow | null {
    for (const meta of metas) {
        const reading = plausibleReading(contextReadingFromMeta(meta?.["agent:context"]));
        if (reading?.window) return { window: reading.window, model: reading.model };
    }
    return null;
}

/** The words a budget band warrants beside its bar; the colour only adds to them. */
export function bandWords(band: SizeBand): string {
    if (band === "critical") return "very large: trim it or split it";
    if (band === "high") return "large";
    return "";
}

/** What a launch of `total` tokens takes of `window`: its share ("6% of 200k"),
 *  band, how full the budget the band measures against is (`fill`, 0–1), and
 *  the room left before auto-compaction. */
export function launchBudget(
    total: number,
    window: number
): { share: string; band: SizeBand; fill: number; room: number } {
    return {
        share: `${shareLabel(total, window) || "0%"} of ${formatCompactNumber(window)}`,
        band: memorySizeBand(total, window),
        fill: Math.min(1, total / (window * DEFAULT_MEMORY_SIZE_FRACTION)),
        room: Math.max(0, compactionThreshold(window) - total),
    };
}

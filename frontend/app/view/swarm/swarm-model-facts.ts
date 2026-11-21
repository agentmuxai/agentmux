// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model facts of a Swarm row's agent, from the window's one runtime store (the row it already reads for stopped
 * and parked agents), keyed by the agent id in its pane's meta. A row without the fields, an agent without a pane or a
 * store that is not there gives null: the row has no model tag, and nothing is guessed
 * (docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §4.3).
 */

import { readModelFacts, type ModelFacts } from "@/app/agent-model/model-tag";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";

export function agentIdOfMeta(meta: Record<string, unknown> | undefined): string | null {
    const id = meta?.["agentId"];
    return typeof id === "string" && id ? id : null;
}

export function modelFactsForRow(meta: Record<string, unknown> | undefined): ModelFacts | null {
    const agentId = agentIdOfMeta(meta);
    if (!agentId) return null;
    try {
        const row = sharedAgentRuntime()
            .rows()
            .find((r) => r.agent_id === agentId);
        return row ? readModelFacts(row) : null;
    } catch {
        return null;
    }
}

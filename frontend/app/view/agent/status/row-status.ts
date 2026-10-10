// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The working row's ranks 0 and 1 (present-status.ts): which of "needs you"
 * and the held statuses wins, from the shared ranking (store/agent-status.ts),
 * and the row's words for it. Ranks 2 and below are the presenter's own.
 */

import { rankAgentStatus } from "@/app/store/agent-status";
import { formatCountdownCompact } from "@/util/format-time";
import { formatPhaseLabel, type LaunchPhase } from "../flows/launch-phase";

export interface RowStatusFacts {
    /** The row is in its live form: a turn in flight or between two passes,
     *  or held by a compaction or a reconnect. */
    live: boolean;
    /** A question or approval waiting on the user, in words, or null. */
    needsYou: string | null;
    reconnecting: boolean;
    compacting: boolean;
    stopping: boolean;
    waitingReason?: "rate_limited" | null;
    /** Until the provider's next retry (Retry-After), when it said. */
    retryAfterMs?: number | null;
    launchPhase?: LaunchPhase | null;
}

/** At most one of the two is set: the one the ranking picked. */
export function rowStatusWords(facts: RowStatusFacts, nowMs: number): { needsYou: string | null; held: string | null } {
    const launchLabel = formatPhaseLabel(facts.launchPhase, nowMs);
    const status = rankAgentStatus({
        live: facts.live,
        needsYou: facts.needsYou != null,
        held: {
            reconnecting: facts.reconnecting,
            compacting: facts.compacting,
            stopping: facts.stopping,
            "rate-limited": facts.waitingReason === "rate_limited",
            launching: launchLabel != null,
        },
    });
    if (status.kind === "needs-you") return { needsYou: facts.needsYou, held: null };
    if (status.kind !== "held") return { needsYou: null, held: null };
    switch (status.reason) {
        case "reconnecting":
            return { needsYou: null, held: "Reconnecting…" };
        case "compacting":
            return { needsYou: null, held: "Compacting…" };
        case "stopping":
            return { needsYou: null, held: "Stopping…" };
        case "rate-limited":
            return {
                needsYou: null,
                held:
                    facts.retryAfterMs != null
                        ? `Rate limited — retrying in ${formatCountdownCompact(facts.retryAfterMs)}`
                        : "Rate limited — retrying…",
            };
        case "launching":
            return { needsYou: null, held: launchLabel };
    }
}

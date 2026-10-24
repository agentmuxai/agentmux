// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the Swarm shows an agent doing, derived from its turn phase (moved out of swarm-view.tsx). */
import { getBlockTurnPhase } from "@/app/store/agentActivity";

export type AgentDisplayStatus =
    "working" | "tools" | "stopping" | "idle" | "error" | "disconnected" | "unknown" | "interrupted";

export function phaseToDisplayStatus(blockId: string, fallback: "running" | "idle"): AgentDisplayStatus {
    const phaseAccessor = getBlockTurnPhase(blockId);
    // A block only has a registered TurnPhase while its pane is mounted in
    // THIS renderer (registerActivity() in agentActivity.ts runs from the
    // mounted agent-pane component). A subagent running in an unmounted
    // pane, a background tab, or another workspace has no entry here — that
    // does NOT mean it's idle, it means we have no visibility into it from
    // here. Conflating the two used to render active agents as flatly
    // "idle" ("nothing in progress" when something clearly was) — see
    // docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §1.4/§3.5.
    //
    // `fallback` is `node.agentStatus` — backend-verified via
    // `GetControllerStatus`/`ControllerStatus` (swarm-model.ts's
    // `derivedRunningStatus`), independent of whether this renderer has a
    // mounted pane for the block. For agent panes it's turn-precise (keyed
    // off `turn_active`, not raw process-alive `shellprocstatus` — see
    // docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    // Finding 1), so it's safe to trust here instead of falling through to
    // "unknown" for an unmounted/background agent.
    if (!phaseAccessor) return fallback === "running" ? "working" : "idle";
    const phase = phaseAccessor();
    switch (phase.kind) {
        case "Submitting":
            return "working";
        case "Streaming":
            return phase.toolsActive > 0 ? "tools" : "working";
        case "Interrupting":
            return "stopping";
        case "Done":
            return phase.outcome === "errored" ? "error" : "idle";
        case "Disconnected":
            return "disconnected";
        default:
            return "idle";
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


/** The working row under the transcript: what the agent is doing now, or how the last turn went. */

import { Show, type JSX } from "solid-js";

import { useRowLiveEffects } from "./working-row-effects";
import { useRowMetrics } from "./working-row-metrics";
import type { AgentWorkingRowProps } from "./working-row-model";
import { createRowState } from "./working-row-state";
import { useRowStatus } from "./working-row-status";
import { useRowSummary } from "./working-row-summary";
import { LiveLine, WorkedLine } from "./working-row-view";

// ── AgentWorkingRow ───────────────────────────────────────────────────────────
// Rendered immediately below AgentDocumentView in the conversation area.
// Shows spinner + "Working… · Ns" while loading, "✓ Worked · 42s" on completion.
// Returns null when neither loading nor has stats — no idle placeholder.
// Stays visible as a turn delimiter until the user sends the next message.

/**
 * The row is a state object (working-row-state.ts) and a run of hook groups that act on it, called in the
 * order their computations and effects must be created: the live status and its type-out, the clocks, the
 * phrase effects, then the worked line and readout.
 */
export const AgentWorkingRow = (props: AgentWorkingRowProps): JSX.Element => {
    const s = createRowState(props);
    const { settling, live, leftText } = useRowStatus(s);
    const metrics = useRowMetrics(s);
    useRowLiveEffects(s, live);
    const summary = useRowSummary(s, metrics);

    // No "Running in background" state here: the ActivityDock's running row IS the indicator for a live
    // attached task (user feedback 2026-08-10; the reducer axis stays, for the watchdog/Swarm consumers).
    return (
        <Show
            when={live() || !!props.compacting || !!props.reconnecting}
            fallback={<WorkedLine summary={summary} />}
        >
            <LiveLine s={s} settling={settling} leftText={leftText} metrics={metrics} summary={summary} />
        </Show>
    );
};

AgentWorkingRow.displayName = "AgentWorkingRow";

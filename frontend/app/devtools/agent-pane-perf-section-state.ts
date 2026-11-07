// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent-pane perf section's state: the latest perf-store snapshot, refreshed once a second while the section is
 * mounted, and whether it holds anything worth showing.
 */

import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { agentPerfStore, type AgentPerfSnapshot } from "@/app/agent-model/perf-probe";

const POLL_INTERVAL_MS = 1000;

const EMPTY_SNAPSHOT: AgentPerfSnapshot = {
    rowMountByKind: new Map(),
    estimatorMissRateByKind: new Map(),
    recentEstimatorMisses: [],
    recentLayoutShifts: [],
    dispatchByKind: new Map(),
    transcriptCursors: [],
};

export type PerfSnapshotState = {
    snapshot: Accessor<AgentPerfSnapshot>;
    hasData: () => boolean;
};

/** Polls the perf store at 1 Hz from mount until cleanup. */
export function createPerfSnapshot(): PerfSnapshotState {
    const [snapshot, setSnapshot] = createSignal<AgentPerfSnapshot>(EMPTY_SNAPSHOT);
    let pollHandle: number | undefined;

    const poll = (): void => {
        setSnapshot(agentPerfStore.snapshot());
    };

    onMount(() => {
        poll();
        pollHandle = window.setInterval(poll, POLL_INTERVAL_MS);
    });
    onCleanup(() => {
        if (pollHandle != null) window.clearInterval(pollHandle);
    });

    const hasData = (): boolean => {
        const s = snapshot();
        return s.rowMountByKind.size > 0
            || s.recentEstimatorMisses.length > 0
            || s.recentLayoutShifts.length > 0
            || s.dispatchByKind.size > 0
            || s.transcriptCursors.length > 0;
    };

    return { snapshot, hasData };
}

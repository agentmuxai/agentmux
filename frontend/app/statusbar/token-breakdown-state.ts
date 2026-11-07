// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the token usage popover holds (TokenBreakdownPopover.tsx lays it out): the agent rows from the usage store,
// listed by agent id so a turn landing does not replace a row under the pointer, the ambient row, the totals, and
// the reset and focus calls.

import { createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { PopoverAnchor } from "@/app/element/anchored-popover";
import { revealBlock } from "@/app/util/reveal-block";
import { AMBIENT_OPEN_MS, watchAmbient } from "@/app/store/ambient-health";
import {
    getAgentBreakdown,
    getCacheHitRate,
    getTotal,
    resetSession,
    tokenUsageState,
    type AgentUsage,
} from "@/store/token-usage";

/** What names an agent's row from one breakdown to the next: its pane, else its name. */
const rowKey = (r: AgentUsage): string => r.blockId ?? r.agentName;

export interface TokenBreakdownPopoverProps {
    /** The TokenUsageIndicator button. */
    anchor: PopoverAnchor;
    onClose: () => void;
}

export function createTokenBreakdown(props: TokenBreakdownPopoverProps) {
    const [confirmingReset, setConfirmingReset] = createSignal(false);
    const [ambientExpanded, setAmbientExpanded] = createSignal(false);
    // The counts come fast for as long as the popover is open, whether or not the part that shows
    // them is (no internal tokens yet, or its row collapsed).
    onMount(() => onCleanup(watchAmbient(AMBIENT_OPEN_MS)));

    // Trigger reactivity on the store so breakdown re-renders when
    // a new turn lands while the popover is open.
    const agentRows = createMemo((): AgentUsage[] => {
        void tokenUsageState.byAgent;
        return getAgentBreakdown();
    });
    const realAgentRows = createMemo(() => agentRows().filter((r) => !r.isAmbient));
    // The rows are listed by agent id and each reads its data from the latest breakdown: a turn that
    // lands rebuilds the breakdown, and rows keyed by their data objects would be replaced under the
    // pointer, losing the click (SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md P1.4).
    const realAgentIds = createMemo(
        () => realAgentRows().map(rowKey),
        [] as string[],
        { equals: (a, b) => a.length === b.length && a.every((id, i) => id === b[i]) }
    );
    const rowById = createMemo(() => new Map(realAgentRows().map((r) => [rowKey(r), r])));
    const ambientRow = createMemo(() => agentRows().find((r) => r.isAmbient) ?? null);
    const total = createMemo(() => {
        void tokenUsageState.byService;
        return getTotal();
    });
    // null until at least one turn has reported a cache breakdown (see
    // getCacheHitRate's doc comment) — render nothing rather than a
    // misleading "0%" in that window.
    const cacheHitRate = createMemo(() => {
        void tokenUsageState.byService;
        return getCacheHitRate();
    });

    const handleResetClick = () => setConfirmingReset(true);

    const handleConfirmReset = () => {
        resetSession();
        setConfirmingReset(false);
        props.onClose();
    };

    const handleAgentClick = (row: AgentUsage) => {
        if (!row.blockId) return;
        void revealBlock(row.blockId, { source: "tokens" });
        props.onClose();
    };
    return { confirmingReset, setConfirmingReset, ambientExpanded, setAmbientExpanded, agentRows, realAgentIds, rowById, ambientRow, total, cacheHitRate, handleResetClick, handleConfirmReset, handleAgentClick };
}

export type TokenBreakdownState = ReturnType<typeof createTokenBreakdown>;

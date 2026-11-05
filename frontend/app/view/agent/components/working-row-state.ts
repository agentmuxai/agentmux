// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Working row's state: the signals it owns and the fields that carry a memory from one memo run to the
 * next. Created first in the component; the hook groups that follow add the computations in their original order.
 */

import { createSignal } from "solid-js";

import { useTick } from "@/app/hook/useTick";
import type { StatusMemory } from "../status/present-status";
import type { AgentWorkingRowProps } from "./working-row-model";
import { pickThinkingPhrase } from "./status-formatters";

export function createRowState(props: AgentWorkingRowProps) {
    const [phrase, setPhrase] = createSignal(pickThinkingPhrase());
    const [lastPhrase, setLastPhrase] = createSignal(pickThinkingPhrase());
    const tick = useTick(1000);
    // tick() re-runs this memo every second so a phase's "up to Ys" countdown
    // (formatPhaseLabel) stays live — see useTick.ts's "always-on tick" pattern.
    const [loadStartMs, setLoadStartMs] = createSignal<number | null>(null);
    // How much of the line is typed out; Infinity once it is all there, so a
    // counter that grows inside the same line shows in full.
    const [revealed, setRevealed] = createSignal(Number.POSITIVE_INFINITY);

    return {
        props,
        phrase,
        setPhrase,
        lastPhrase,
        setLastPhrase,
        tick,
        loadStartMs,
        setLoadStartMs,
        revealed,
        setRevealed,
        // A turn's first line types out too (operator, 2026-10-10; it showed whole
        // since 2026-08-10). Going live forgets the presenter's memory, so that line
        // is new to it and its dwell counts from the end of its own type-out.
        wasLive: false,
        // The live status (status/present-status.ts): what to say, and when, from
        // what the agent is doing and what the row showed last.
        statusMemory: null as StatusMemory | null,
        // The turn's output so far, never shown going down within one turn (a
        // pass's exact count can land under its streamed estimate).
        shownOutput: { turnId: null as number | null, output: 0 },
    };
}

export type RowState = ReturnType<typeof createRowState>;

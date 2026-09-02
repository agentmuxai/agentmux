// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useScrollToNode — signal-based jump command that AgentDocumentView
 * reacts to via createEffect.
 *
 * Step 9 of docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Replaces the mutable `let scrollToNodeFn: ((id: string) => void) | null`
 * pattern in agent-view.tsx. Callers (search) invoke `jumpTo`;
 * AgentDocumentView reads `command()` in an effect and runs the actual
 * DOM scroll inside its own scroll container.
 *
 * Every call to `jumpTo` increments a monotonic `seq` counter so that
 * consecutive jumps to the same nodeId still fire a new effect run —
 * otherwise SolidJS would see the signal value as unchanged and skip
 * the effect.
 */

import { createSignal, type Accessor } from "solid-js";

export interface ScrollCommand {
    nodeId: string;
    seq: number;
}

export interface UseScrollToNode {
    /**
     * The latest jump request. `null` until the first `jumpTo` call.
     * Consumers should read this inside a `createEffect` and perform
     * their own DOM scroll.
     */
    command: Accessor<ScrollCommand | null>;
    /**
     * Request a jump to the given document node id. Safe to call from
     * any code path — fires the effect on the next microtask.
     */
    jumpTo: (nodeId: string) => void;
}

export function useScrollToNode(): UseScrollToNode {
    const [command, setCommand] = createSignal<ScrollCommand | null>(null);
    let seq = 0;
    const jumpTo = (nodeId: string) => {
        seq += 1;
        setCommand({ nodeId, seq });
    };
    return { command, jumpTo };
}

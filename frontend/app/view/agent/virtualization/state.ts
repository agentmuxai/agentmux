// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentViewState — scroll-and-streaming state for the virtualized
 * agent pane, kept in a Solid store so it's the single source of truth
 * (DOM scrollTop is a projection of this, never the other way around).
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md
 * §"Scroll state in data".
 *
 * Phase 1: store + transitions only. The view layer in Phase 2 will
 * subscribe to these signals and project them into the virtualizer.
 */

import { createMemo, createSignal, type Accessor, type Setter } from "solid-js";
import type { DocumentNode } from "../types";
import type { SignalPair } from "../state";
import type { ScrollAnchor } from "./anchor";

export interface AgentViewState {
    /** Current document — pass-through from the existing documentAtom. */
    nodes: Accessor<readonly DocumentNode[]>;

    /**
     * O(1) id → index lookup. Recomputed when `nodes` changes; tested
     * for memo-stability so consumers don't pay quadratic costs on
     * frequent re-reads.
     */
    nodeIndex: Accessor<ReadonlyMap<string, number>>;

    /**
     * True when the user is anchored to the latest message and any new
     * tail content should auto-scroll into view. Set false when the
     * user scrolls up; restored to true on jumpToBottom or when the
     * user scrolls back near the bottom.
     */
    stickToBottom: Accessor<boolean>;

    /**
     * Engage stick-to-bottom AND clear any captured head anchor.
     * Atomic — these always go together because a stale anchor would
     * later restore scroll to the wrong place after a remount.
     * (codex P2 on PR #783.)
     */
    engageStickToBottom: () => void;

    /**
     * Disengage stick-to-bottom without touching the anchor. Used when
     * the user scrolls up but hasn't yet crossed the near-top threshold
     * that triggers anchor capture.
     */
    disengageStickToBottom: () => void;

    /**
     * Captured anchor for restoring scroll position after a prepend
     * (history pagination) or remount (tab switch). null when sticky
     * to bottom or when no anchor has been captured.
     */
    headAnchor: Accessor<ScrollAnchor | null>;
    /**
     * Capture an anchor and flip stickToBottom off — these always go
     * together: capturing an anchor means "user wants to stay where
     * they are", which is the opposite of stick-to-bottom semantics.
     */
    captureHeadAnchor: (anchor: ScrollAnchor) => void;
    clearHeadAnchor: () => void;

    /**
     * Id of the node currently receiving streaming content. Pinned
     * out of the virtualizer's recycle range so its measurement isn't
     * invalidated mid-stream. null when no stream is active.
     */
    streamingNodeId: Accessor<string | null>;
    setStreamingNodeId: Setter<string | null>;

    /** Convenience: lookup index by node id, or -1. */
    indexOf: (nodeId: string) => number;
}

/**
 * Construct a fresh AgentViewState bound to a document signal pair.
 * Call once per agent ViewModel instance — same lifetime as the
 * existing AgentAtoms (see ../state.ts).
 */
export function createAgentViewState(documentAtom: SignalPair<DocumentNode[]>): AgentViewState {
    // Renamed from `document` to avoid shadowing the browser global
    // (reagent P2 on PR #783).
    const [docSignal] = documentAtom;

    const nodeIndex = createMemo<ReadonlyMap<string, number>>(() => {
        const docs = docSignal();
        const map = new Map<string, number>();
        for (let i = 0; i < docs.length; i++) {
            map.set(docs[i].id, i);
        }
        return map;
    });

    const [stickToBottom, setStickToBottom] = createSignal(true);
    const [headAnchor, setHeadAnchor] = createSignal<ScrollAnchor | null>(null);
    const [streamingNodeId, setStreamingNodeId] = createSignal<string | null>(null);

    const captureHeadAnchor = (anchor: ScrollAnchor) => {
        setHeadAnchor(anchor);
        setStickToBottom(false);
    };

    const clearHeadAnchor = () => setHeadAnchor(null);

    // Engaging stick MUST clear any captured head anchor — otherwise
    // a later remount would restore from the stale anchor instead of
    // sticking to the latest. Atomic.
    const engageStickToBottom = () => {
        setHeadAnchor(null);
        setStickToBottom(true);
    };

    const disengageStickToBottom = () => {
        setStickToBottom(false);
    };

    const indexOf = (nodeId: string): number => {
        return nodeIndex().get(nodeId) ?? -1;
    };

    return {
        nodes: docSignal,
        nodeIndex,
        stickToBottom,
        engageStickToBottom,
        disengageStickToBottom,
        headAnchor,
        captureHeadAnchor,
        clearHeadAnchor,
        streamingNodeId,
        setStreamingNodeId,
        indexOf,
    };
}

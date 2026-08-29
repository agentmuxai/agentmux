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

import { batch, createMemo, createSignal, type Accessor, type Setter } from "solid-js";
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
     * True once this pane's content has overflowed its viewport at least
     * once (`scrollHeight > clientHeight`). A pane that has never
     * overflowed cannot have a legitimate "user scrolled away" state —
     * there is nowhere to have scrolled away to yet — so the view layer
     * uses this to force-pin the very first overflow transition instead
     * of trusting whatever geometry a scroll event reports at that
     * instant. See docs/specs/SPEC_AGENT_PANE_FIRST_OVERFLOW_SCROLL_PIN_FIX_2026_08_29.md.
     */
    hasOverflowedOnce: Accessor<boolean>;
    /** Idempotent — latches true and stays true for the pane's lifetime. */
    markOverflowedOnce: () => void;

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

    /**
     * True once the initial history load has completed (HistoryLoaded /
     * HistoryRestored / empty-document fast-exit / InitFailed). Used by
     * AgentDocumentVirtualList to gate the new-message enter animation:
     * rows that mount before this point are historical, rows that mount
     * after it are live-streamed. Set by useHistoryPagination via
     * markHistoryReady. See PR #1212.
     */
    historyReady: Accessor<boolean>;
    markHistoryReady: () => void;
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
    const [hasOverflowedOnce, setHasOverflowedOnce] = createSignal(false);
    const markOverflowedOnce = () => setHasOverflowedOnce(true);
    const [headAnchor, setHeadAnchor] = createSignal<ScrollAnchor | null>(null);
    const [streamingNodeId, setStreamingNodeId] = createSignal<string | null>(null);
    const [historyReady, setHistoryReady] = createSignal(false);
    const markHistoryReady = () => setHistoryReady(true);

    // Both two-signal mutations wrap in batch() so subscribers don't
    // observe the inconsistent intermediate state — without batch, an
    // effect listening to `headAnchor` would fire while
    // `stickToBottom` was still its old value, breaking the documented
    // atomic-pair invariant. (reagent P1 on PR #783 fix push.)
    const captureHeadAnchor = (anchor: ScrollAnchor) => {
        batch(() => {
            setHeadAnchor(anchor);
            setStickToBottom(false);
        });
    };

    const clearHeadAnchor = () => setHeadAnchor(null);

    // Engaging stick MUST clear any captured head anchor — otherwise
    // a later remount would restore from the stale anchor instead of
    // sticking to the latest. Atomic via batch().
    const engageStickToBottom = () => {
        batch(() => {
            setHeadAnchor(null);
            setStickToBottom(true);
        });
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
        hasOverflowedOnce,
        markOverflowedOnce,
        headAnchor,
        captureHeadAnchor,
        clearHeadAnchor,
        streamingNodeId,
        setStreamingNodeId,
        indexOf,
        historyReady,
        markHistoryReady,
    };
}

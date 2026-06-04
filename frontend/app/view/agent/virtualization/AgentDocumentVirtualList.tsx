// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentVirtualList — hybrid virtualized + streaming-buffer
 * renderer for the agent pane document. Replaces the old monolithic
 * `<For each={document()}>` block in AgentDocumentView.
 *
 * Architecture (from docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md):
 *
 *  ┌─ scrollRef (.agent-document) ─────────────────────────┐
 *  │  ┌─ virtualized region ─────────────────────────────┐ │
 *  │  │  height = layoutView().totalSize (slice)         │ │
 *  │  │  position: relative                              │ │
 *  │  │  rows: position: absolute, translateY(start)     │ │
 *  │  │  measure ResizeObserver on each row              │ │
 *  │  └──────────────────────────────────────────────────┘ │
 *  │  ┌─ streaming buffer ───────────────────────────────┐ │
 *  │  │  trailing N nodes, normal flex flow              │ │
 *  │  │  no virtualization → no measurement lag during   │ │
 *  │  │  token streams                                   │ │
 *  │  └──────────────────────────────────────────────────┘ │
 *  └────────────────────────────────────────────────────────┘
 *
 * Scroll state (`stickToBottom`, `headAnchor`) lives in
 * AgentViewState, never in scrollRef.scrollTop. Caller drives
 * scrollToBottom / scrollToNode via props; this component projects
 * the state into the DOM but never reads it back.
 */

import { createEffect, createMemo, createSignal, Index, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";
import { trail } from "@/log/render-trail";
import { Key } from "@solid-primitives/keyed";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import { agentPerfStore, startAgentLayoutShiftObserver } from "./perf-probe";
import {
    captureTopmostAnchor,
    isNearBottom,
    isNearTop,
    restoreScrollFromAnchor,
} from "./anchor";
import { DocumentRow } from "./DocumentRow";
import { estimateNode, estimateNodeForState } from "./renderers";
import { currentExpansion } from "./expansion-source";
import type { AgentViewState } from "./state";
import {
    dispatchIfRegistered as dispatchLayoutIfRegistered,
    snapshot as snapshotLayout,
    type LayoutView,
    type RowPosition,
} from "@/app/store/agent-pane-layout-store";
import { computeLayoutView } from "@/app/store/agent-pane-layout/reducer";
import { inFlowState } from "@/app/store/agent-pane-layout/types";
import {
    initialStickyFrontierId,
    partitionForVirtualization,
    STREAMING_BUFFER_SIZE,
} from "./streaming-buffer";

export interface AgentDocumentVirtualListProps {
    viewState: AgentViewState;
    documentState: Accessor<DocumentState>;
    bookmarkedNodeIds?: Accessor<Set<string>>;
    onBookmark?: (node: DocumentNode) => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
    onLoadOlder?: () => Promise<void>;
    loadingOlder?: Accessor<boolean>;
    highlightNodeId?: Accessor<string | null>;
    scrollCommand?: Accessor<ScrollCommand | null>;
    /** Wire-up callback so the parent (AgentFooter via AgentDocumentView)
     *  can invoke jumpToBottom on user keystroke. */
    scrollToBottomRef?: (fn: () => void) => void;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    /**
     * Live per-pane zoom factor (the same value applied as CSS `zoom` on
     * `.agent-view` in agent-view.tsx). Used to normalize the measure RO + the
     * Scrolled / scrollToNode math into unzoomed CSS px so slice positions don't
     * double-count zoom and overlap — see
     * SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01.
     * Defaults to 1 (no zoom) when not supplied.
     */
    zoomFactor?: Accessor<number>;
    /**
     * Content rendered inside the scroll container, above the
     * virtualized region. Used by AgentDocumentView for the
     * auth-url box and the loading-older banner. Affects scroll math
     * via scrollMargin (read from virtualContainerRef.offsetTop).
     */
    headerSlot?: JSX.Element;
    /**
     * blockId for the agent-pane-layout slice. When present, the list feeds the
     * slice (nodes / estimates / expansion / viewport / measurements) and the
     * measure RO dispatches `RowMeasured` keyed by the row's current expansion
     * state (INV-3). Required for the slice-driven render path (Phase 3).
     */
    blockId?: string;
    /**
     * Derived layout view from the agent-pane-layout slice (Phase 3). When
     * present, rows render from `layoutView().rows` (prefix-sum positions)
     * instead of TanStack's virtual items.
     */
    layoutView?: Accessor<LayoutView | null>;
}

export function AgentDocumentVirtualList(props: AgentDocumentVirtualListProps): JSX.Element {
    let scrollRef!: HTMLDivElement;
    let virtualContainerRef: HTMLDivElement | undefined;
    // Guard against concurrent older-history fetches triggered by scroll.
    let loadingOlderInFlight = false;

    // Sticky frontier id — once the document crosses
    // STREAMING_BUFFER_SIZE, this freezes to the id of the first node
    // in the streaming buffer. After that, every appended node lands
    // in `streamingNodes` and the virtualized head stays fixed; no
    // node ever migrates from one subtree to the other on a simple
    // append. That migration was the root cause of the SolidJS
    // `replaceChild` crash on send-message
    // (docs/analysis/AGENT_PANE_REPLACECHILD_CRASH_ON_SEND_2026_05_27.md).
    //
    // Cleared whenever the anchor node is truncated away (e.g.,
    // history reset / pane re-mount); the next partition recompute
    // re-anchors against the new tail.
    //
    // Plain `let` rather than a Solid signal: mutating the variable
    // must NOT trigger another memo run while we're inside one.
    let stickyFrontierId: string | null = null;

    // Gate for the new-message enter animation. Starts false.
    // Flipped to true by a createEffect (below) once viewState.historyReady()
    // fires, but only AFTER a forced browser style resolution via
    // `void scrollRef.scrollTop`. The forced reflow commits the history rows'
    // first style resolution WITHOUT [data-animate] present; subsequent
    // mounts (new streaming rows) get [data-animate] at their first resolution
    // and animate via @starting-style + transition. Without the reflow,
    // @starting-style fires on the history rows' first resolution (which
    // happens at paint time, after all microtasks, including any queueMicrotask
    // defer), so they cascade regardless. (reagent P1 on #1212.)
    const [animateEnabled, setAnimateEnabled] = createSignal(false);

    // Memoized — partition is read inside the slice-feeding effect,
    // nodeById, scrollToNode / older-history, and the streaming buffer
    // <Index>. Without createMemo, every read re-slices the document
    // (O(n)) on every token of streaming, defeating the streaming
    // buffer's purpose. (reagent P1 on #784.)
    const partition = createMemo(() => {
        const nodes = props.viewState.nodes();

        // First time the document exceeds the streaming buffer, lock
        // the frontier. Subsequent appends grow `streamingNodes`.
        if (stickyFrontierId == null) {
            stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
        }

        let result = partitionForVirtualization(
            nodes,
            STREAMING_BUFFER_SIZE,
            stickyFrontierId,
        );

        // Stale frontier (anchor node was truncated). Re-anchor and
        // recompute once. The re-anchor is the ONLY place a node
        // crosses subtrees, and it happens only on
        // truncate/clear/reset — never during normal streaming.
        if (result.splitIndex === -1) {
            stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
            result = partitionForVirtualization(
                nodes,
                STREAMING_BUFFER_SIZE,
                stickyFrontierId,
            );
        }

        // Crash-trace: every partition recompute is a candidate trigger
        // for the reconciler's <For> reconcile pass. The render-trail
        // ring buffer dump from the boundary will show how many of
        // these landed just before the throw.
        trail("agent:virt:partition", {
            virtCount: result.virtualizedNodes.length,
            streamCount: result.streamingNodes.length,
            frontier: stickyFrontierId?.slice(0, 8) ?? null,
        });
        return result;
    });

    // Phase 3: feed the layout slice from the VIRTUALIZED partition only.
    // The slice models the prefix-summed region; the streaming buffer is
    // normal-flow and out of scope. Dispatches NodesChanged + per-node
    // EstimateSet (both states, INV-3) + ExpansionResolved so positions /
    // effectiveHeight are authoritative. No-op when blockId is absent.
    if (props.blockId) {
        // nodeId → JSON(Expansion) last pushed (expansion diff cache).
        const pushedExpansion = new Map<string, string>();
        // nodeIds with EstimateSet pushed for both states; pruned in lockstep
        // with the slice so a removed-then-re-added node gets fresh values.
        const estimatesPushed = new Set<string>();
        createEffect(() => {
            const blockId = props.blockId!;
            const vnodes = partition().virtualizedNodes;
            const docState = props.documentState();
            const ids = vnodes.map((n) => n.id);
            dispatchLayoutIfRegistered(blockId, { type: "NodesChanged", orderedIds: ids });
            const idSet = new Set(ids);
            for (const k of pushedExpansion.keys()) if (!idSet.has(k)) pushedExpansion.delete(k);
            for (const k of estimatesPushed) if (!idSet.has(k)) estimatesPushed.delete(k);
            for (const node of vnodes) {
                if (!estimatesPushed.has(node.id)) {
                    estimatesPushed.add(node.id);
                    dispatchLayoutIfRegistered(blockId, {
                        type: "EstimateSet",
                        nodeId: node.id,
                        state: "collapsed",
                        cssPx: estimateNodeForState(node, "collapsed", docState),
                    });
                    dispatchLayoutIfRegistered(blockId, {
                        type: "EstimateSet",
                        nodeId: node.id,
                        state: "expanded",
                        cssPx: estimateNodeForState(node, "expanded", docState),
                    });
                }
                const next = currentExpansion(node, docState);
                const key = JSON.stringify(next);
                if (pushedExpansion.get(node.id) !== key) {
                    pushedExpansion.set(node.id, key);
                    dispatchLayoutIfRegistered(blockId, {
                        type: "ExpansionResolved",
                        nodeId: node.id,
                        to: next,
                    });
                }
            }
        });
    }

    // ── Phase 3 Step 2: feed the layout slice's VIEWPORT inputs ─────────
    // Shadow wiring — the slice records scrollTop / viewport / scrollMargin /
    // zoom, but nothing renders from them until the Step-3 cutover. All
    // blockId-gated and reducer-deduped (an unchanged value returns the same
    // state ref). Slice positions are unzoomed CSS px, so scroll + viewport
    // are normalized by the live zoom — the same ÷zoom basis as measureElement.
    // (scrollTop ÷zoom under CSS `zoom` is pending CDP confirmation at zoom
    // 0.5 / 2 per the Phase-3 plan.)
    let lastScrollMarginPx = -1;
    const dispatchScrollMargin = (): void => {
        if (!props.blockId) return;
        const px = virtualContainerRef?.offsetTop ?? 0;
        if (px === lastScrollMarginPx) return; // skip redundant dispatches
        lastScrollMarginPx = px;
        dispatchLayoutIfRegistered(props.blockId, { type: "ScrollMarginChanged", px });
    };
    if (props.blockId) {
        // Mirror the live zoom into the slice (INV-2: stored, never relayouts).
        createEffect(() => {
            const blockId = props.blockId!;
            const zoom = props.zoomFactor?.() ?? 1;
            dispatchLayoutIfRegistered(blockId, { type: "ZoomChanged", zoom });
        });
        // scrollMargin = the virtualized container's offsetTop (the header
        // above it). Re-read on every region reflow; handleScroll + the dedup
        // cover the header-shift cases (auth box / loading-older banner).
        onMount(() => {
            if (!virtualContainerRef) return;
            dispatchScrollMargin();
            const ro = new ResizeObserver(() => dispatchScrollMargin());
            ro.observe(virtualContainerRef);
            onCleanup(() => ro.disconnect());
        });
    }

    // Phase 3 Step 6: the TanStack virtualizer is gone. Positions, windowing,
    // and measurement now come entirely from the agent-pane-layout slice
    // (rendered above from layoutView + the measure RO). Heights are keyed by
    // (nodeId, state) in the slice, so a row that scrolls out and back keeps
    // its measured height — the recycle-stale-estimate class is gone by
    // construction, and prefix-sum positions make overlap impossible (INV-1).

    // Layout-shift observer scoped to .agent-document. Idempotent —
    // subsequent mounts are no-ops. Production: short-circuits.
    onMount(() => {
        startAgentLayoutShiftObserver();
    });

    // Flip animateEnabled once history loading is done.
    //
    // `historyReady()` is set by useHistoryPagination after
    // HistoryLoaded/HistoryRestored (or immediately for empty documents).
    // By the time it fires, the history rows are already in the DOM.
    //
    // CRITICAL: we must force a browser style resolution for those rows
    // BEFORE adding [data-animate], otherwise @starting-style applies to
    // them on their first resolution (which happens at paint time, after
    // all pending microtasks drain — including any queueMicrotask defers).
    // Reading a layout property (scrollRef.scrollTop) triggers a
    // synchronous style recalc/layout flush, so the history rows' first
    // style resolution is committed WITHOUT [data-animate]. Subsequent
    // mounts (new streaming rows) see [data-animate] at their first
    // resolution and animate correctly. (reagent #1212 P1.)
    //
    // For empty conversations historyReady() fires immediately with no
    // rows in the DOM yet — the forced reflow is a no-op and the first
    // streaming row mounts with [data-animate] already present. (codex P2.)
    createEffect(() => {
        if (animateEnabled() || !props.viewState.historyReady()) return;
        void scrollRef?.scrollTop; // forced synchronous style/layout flush
        setAnimateEnabled(true);
    });

    // Project stickToBottom into scroll position. When the document
    // grows AND we're sticky, scroll the streaming buffer's last item
    // into view. Streaming-buffer items live in normal flow at the
    // bottom of the scroll container, so MAX_SAFE_INTEGER scrollTo
    // works reliably for them (no virtualizer reflow needed).
    createEffect(() => {
        // Track length changes — Solid will re-run when nodes() emits.
        const _len = props.viewState.nodes().length;
        if (props.viewState.stickToBottom() && scrollRef) {
            // queueMicrotask so the new content has rendered before we scroll.
            queueMicrotask(() => {
                if (!scrollRef) return;
                scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
            });
        }
    });

    // Re-apply sticky-bottom on hidden → visible. An inactive tab is
    // display:none (0×0), so the nodes()-driven scroll above is a no-op
    // while hidden and nothing re-issues it on reactivation. Watch the
    // container's clientHeight 0 → non-zero transition; re-scroll only
    // when already sticky (never engages stick, so a user who scrolled
    // up stays put).
    onMount(() => {
        let wasHidden = scrollRef.clientHeight === 0;
        const ro = new ResizeObserver(() => {
            const h = scrollRef.clientHeight;
            if (h > 0 && wasHidden && props.viewState.stickToBottom()) {
                scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
            }
            // Phase 3 Step 2 (shadow): the scroll container resizing changes the
            // viewport the slice windows against — feed it (covers hidden→visible
            // 0→N and pane resize). blockId-gated, reducer-deduped.
            if (props.blockId && h > 0) {
                const zoom = props.zoomFactor?.() ?? 1;
                dispatchLayoutIfRegistered(props.blockId, {
                    type: "Scrolled",
                    scrollTop: scrollRef.scrollTop / (zoom || 1),
                    viewportPx: h / (zoom || 1),
                });
            }
            wasHidden = h === 0;
        });
        ro.observe(scrollRef);
        onCleanup(() => ro.disconnect());
    });

    // jumpToBottom: forced scroll-to-bottom that re-engages stick.
    // Exposed to parent so AgentFooter can call it on keystroke.
    const jumpToBottom = (): void => {
        if (!scrollRef) return;
        props.viewState.engageStickToBottom();
        scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
    };
    if (props.scrollToBottomRef) props.scrollToBottomRef(jumpToBottom);

    // scrollToNode: jump the named node into view. Disengages stick
    // (user wants to stay where they jumped). Uses the slice's row
    // position if the target is in the virtualized head, else the
    // streaming buffer's natural DOM offset.
    const scrollToNode = (nodeId: string): void => {
        if (!scrollRef) return;
        const idx = props.viewState.indexOf(nodeId);
        if (idx < 0) return;
        const p = partition();
        if (idx < p.splitIndex) {
            // Virtualized region — center the row from the slice's position
            // (Step 5). row.start/height are unzoomed CSS px; convert the
            // target back to the scroll element's zoomed scrollTop (×zoom).
            const v = props.layoutView?.();
            const row = v?.rows[idx];
            if (row) {
                const zoom = props.zoomFactor?.() ?? 1;
                const viewportPx = scrollRef.clientHeight / (zoom || 1);
                const center = row.start - viewportPx / 2 + row.height / 2;
                scrollRef.scrollTo({ top: Math.max(0, center * (zoom || 1)), behavior: "smooth" });
            }
        } else {
            // Streaming buffer — node is mounted; query DOM and scroll directly.
            const el = scrollRef.querySelector(`[data-node-id="${nodeId}"]`) as HTMLElement | null;
            if (!el) return;
            const elTop = el.offsetTop;
            const center = elTop - scrollRef.clientHeight / 2 + el.clientHeight / 2;
            scrollRef.scrollTo({ top: Math.max(0, center), behavior: "smooth" });
        }
        props.viewState.disengageStickToBottom();
    };

    // React to jump commands from the parent's useScrollToNode hook.
    createEffect(() => {
        const cmd = props.scrollCommand?.();
        if (cmd) scrollToNode(cmd.nodeId);
    });

    const handleScroll = (): void => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;

        // Phase 3 Step 2 (shadow): push scroll position + viewport into the
        // slice as unzoomed CSS px. Reducer-deduped; blockId-gated.
        if (props.blockId) {
            const zoom = props.zoomFactor?.() ?? 1;
            dispatchLayoutIfRegistered(props.blockId, {
                type: "Scrolled",
                scrollTop: scrollTop / (zoom || 1),
                viewportPx: clientHeight / (zoom || 1),
            });
            dispatchScrollMargin();
        }

        // Engage stick when user scrolls back near bottom; disengage
        // otherwise. Engaging clears any captured headAnchor (atomic).
        if (isNearBottom(scrollTop, scrollHeight, clientHeight)) {
            if (!props.viewState.stickToBottom()) {
                props.viewState.engageStickToBottom();
            }
        } else {
            if (props.viewState.stickToBottom()) {
                props.viewState.disengageStickToBottom();
            }
        }

        // Older-history pagination — capture anchor, fetch, restore.
        if (
            props.onLoadOlder &&
            isNearTop(scrollTop) &&
            !loadingOlderInFlight &&
            !(props.loadingOlder?.())
        ) {
            // Capture anchor from the topmost visible node. Two cases:
            //   1) Document is large enough to virtualize → anchor on the
            //      topmost visible row from the slice's window. row.start
            //      already includes scrollMargin, so it's read directly
            //      without re-adding the margin (same basis as the old
            //      TanStack v3 virtualItem.start).
            //   2) Document fits entirely in the streaming buffer
            //      (≤ STREAMING_BUFFER_SIZE nodes) → the window is empty, so
            //      fall back to the streaming buffer's first node via DOM.
            //      This is the common initial-pagination case (codex P2 #784).
            const v = props.layoutView?.();
            const topRow = v && v.window.endIndex >= v.window.startIndex
                ? v.rows[v.window.startIndex]
                : undefined;
            let anchorId: string | null = null;
            let anchorOffsetPx = 0;
            if (topRow) {
                anchorId = topRow.nodeId;
                anchorOffsetPx = topRow.start;
            } else if (partition().streamingNodes.length > 0) {
                anchorId = partition().streamingNodes[0].id;
                const el = scrollRef.querySelector(
                    `[data-node-id="${anchorId}"]`,
                ) as HTMLElement | null;
                anchorOffsetPx = el?.offsetTop ?? 0;
            }
            if (anchorId != null) {
                const anchor = captureTopmostAnchor(
                    [{ id: anchorId, offsetPx: anchorOffsetPx }],
                    scrollTop,
                );
                if (anchor) props.viewState.captureHeadAnchor(anchor);
            }

            loadingOlderInFlight = true;
            props.onLoadOlder().then(() => {
                requestAnimationFrame(() => {
                    // Restore from anchor by id — partition reflects the
                    // new prepended nodes, so the anchor's new index
                    // gives a fresh measured offset.
                    //
                    // Use the same `partition()` memo the DOM is
                    // rendered from. Recomputing via the raw
                    // `partitionForVirtualization()` here would use a
                    // count-based split that ignores the sticky
                    // frontier, so an anchor that's actually in the
                    // streaming buffer could be misclassified as
                    // virtualized and read a slice row index past the
                    // virtualized region, and the restore would
                    // silently fail. Codex P2 on PR #1101.
                    const anchor = props.viewState.headAnchor();
                    if (anchor != null && scrollRef) {
                        const newIdx = props.viewState.indexOf(anchor.nodeId);
                        if (newIdx >= 0) {
                            const newP = partition();
                            if (newIdx < newP.splitIndex) {
                                // Still virtualized — read the new offset from a
                                // FRESH slice snapshot (the prepend shifted every
                                // position). Synchronous, so no projection-timing
                                // dependency. start includes scrollMargin — same
                                // basis as the captured anchorOffsetPx.
                                const snap = props.blockId ? snapshotLayout(props.blockId) : null;
                                const offset = snap ? computeLayoutView(snap).rows[newIdx]?.start : undefined;
                                if (offset != null) {
                                    const target = restoreScrollFromAnchor(anchor, offset);
                                    scrollRef.scrollTo({ top: target, behavior: "auto" });
                                }
                            } else {
                                // Now in streaming buffer — query DOM.
                                const el = scrollRef.querySelector(
                                    `[data-node-id="${anchor.nodeId}"]`,
                                ) as HTMLElement | null;
                                if (el) {
                                    const target = restoreScrollFromAnchor(anchor, el.offsetTop);
                                    scrollRef.scrollTo({ top: target, behavior: "auto" });
                                }
                            }
                        }
                    }
                    loadingOlderInFlight = false;
                });
            }).catch(() => {
                loadingOlderInFlight = false;
            });
        }
    };

    // ── Phase 3 Step 3+4: render from the slice + a standalone measure RO ──
    // Rows render from the slice's prefix-sum positions (computeLayoutView →
    // the layoutView signal), keyed by stable nodeId so the fresh RowPosition
    // objects each recompute produces don't remount rows. Heights come from a
    // single ResizeObserver that dispatches RowMeasured keyed by (nodeId,
    // state) — replacing measureElement + the data-index dance.

    // The visible rows, sliced from the slice's full positions array. Empty
    // window (endIndex < startIndex) → render nothing.
    const windowedRows = createMemo<RowPosition[]>(() => {
        const v = props.layoutView?.();
        if (!v || v.window.endIndex < v.window.startIndex) return [];
        return v.rows.slice(v.window.startIndex, v.window.endIndex + 1);
    });

    // nodeId → live DocumentNode for the virtualized partition, so a row
    // renders the CURRENT node at its id (streaming updates propagate without
    // remount). A windowed row whose node isn't in the partition this frame is
    // skipped — the same recoverable drop the old getVirtualItems guard did.
    const nodeById = createMemo(() => {
        const m = new Map<string, DocumentNode>();
        for (const n of partition().virtualizedNodes) m.set(n.id, n);
        return m;
    });

    // One measure RO for all virtualized rows; el→nodeId lets the callback
    // dispatch RowMeasured keyed by the row's current in-flow state, ÷zoom to
    // stay in unzoomed CSS px (INV-2/3). Rows observe on mount, unobserve on
    // unmount (the Key child's onCleanup).
    const elNodeId = new WeakMap<Element, string>();
    const measureRO = typeof ResizeObserver !== "undefined"
        ? new ResizeObserver((entries) => {
            const blockId = props.blockId;
            if (!blockId) return;
            const zoom = props.zoomFactor?.() ?? 1;
            const snap = snapshotLayout(blockId);
            const nodes = nodeById();
            for (const entry of entries) {
                const nodeId = elNodeId.get(entry.target);
                if (!nodeId) continue;
                const cssPx = entry.target.getBoundingClientRect().height / (zoom || 1);
                const node = nodes.get(nodeId);
                if (node) {
                    agentPerfStore.recordEstimatorMeasurement(
                        node.type,
                        estimateNode(node, props.documentState()),
                        cssPx,
                    );
                }
                dispatchLayoutIfRegistered(blockId, {
                    type: "RowMeasured",
                    nodeId,
                    state: inFlowState(snap?.expansion.get(nodeId)),
                    cssPx,
                });
            }
        })
        : undefined;
    onCleanup(() => measureRO?.disconnect());
    const observeRow = (el: HTMLElement, nodeId: string): void => {
        elNodeId.set(el, nodeId);
        measureRO?.observe(el);
    };
    const unobserveRow = (el: HTMLElement): void => {
        measureRO?.unobserve(el);
        elNodeId.delete(el);
    };

    // Render: scroll container holds the optional header, the
    // virtualized head, and the streaming buffer. headerSlot offsets
    // the rows; the slice's scrollMargin (fed above) accounts for it.
    return (
        <div class="agent-document" ref={scrollRef} onScroll={handleScroll}>
            {props.headerSlot}
            {/* Virtualized head — only present when document > buffer size */}
            <div
                ref={(el) => { virtualContainerRef = el; }}
                class="agent-document-virtualizer"
                style={{
                    height: `${props.layoutView?.()?.totalSize ?? 0}px`,
                    position: "relative",
                    width: "100%",
                }}
            >
                <Key each={windowedRows()} by={(r) => r.nodeId}>
                    {(row) => {
                        let rowEl: HTMLElement | undefined;
                        // The live node at this row's id; skip a frame if it's
                        // momentarily absent from the partition (recoverable,
                        // like the old getVirtualItems undefined guard).
                        const node = (): DocumentNode | undefined => nodeById().get(row().nodeId);
                        onCleanup(() => { if (rowEl) unobserveRow(rowEl); });
                        return (
                            <Show when={node()}>
                                {(n) => (
                                    <DocumentRow
                                        node={n}
                                        documentState={props.documentState}
                                        bookmarkedNodeIds={props.bookmarkedNodeIds}
                                        onBookmark={props.onBookmark}
                                        onSubagentClick={props.onSubagentClick}
                                        highlightNodeId={props.highlightNodeId}
                                        onToggleCollapse={props.onToggleCollapse}
                                        onTogglePin={props.onTogglePin}
                                        // Step 4: ref carries the measure RO
                                        // (keyed by nodeId), not measureElement —
                                        // no data-index, no measure race.
                                        ref={(el) => { rowEl = el; observeRow(el, row().nodeId); }}
                                        style={{
                                            position: "absolute",
                                            top: "0",
                                            left: "0",
                                            width: "100%",
                                            transform: `translateY(${row().start - (virtualContainerRef?.offsetTop ?? 0)}px)`,
                                        }}
                                    />
                                )}
                            </Show>
                        );
                    }}
                </Key>
            </div>

            {/* Streaming buffer — always-mounted trailing nodes.
                Uses <Index> not <For> because Solid's <For> reconciles
                by item REFERENCE: each streaming token replaces the
                active node's object (immutable update preserves id but
                gives a new ref), and <For> would treat that as a new
                item and unmount/remount the row on every token —
                exactly the regression we wanted the streaming buffer
                to prevent. <Index> keys by position and passes the
                item as a Solid signal accessor, so the same
                DocumentRow stays mounted while its `props.node()`
                reactively re-reads the current value. (reagent P1 on
                #784.) */}
            <div class="agent-document-streaming-buffer" data-animate={animateEnabled() || undefined}>
                <Index each={partition().streamingNodes as DocumentNode[]}>
                    {(nodeAccessor) => (
                        <DocumentRow
                            node={nodeAccessor}
                            documentState={props.documentState}
                            bookmarkedNodeIds={props.bookmarkedNodeIds}
                            onBookmark={props.onBookmark}
                            onSubagentClick={props.onSubagentClick}
                            highlightNodeId={props.highlightNodeId}
                            onToggleCollapse={props.onToggleCollapse}
                            onTogglePin={props.onTogglePin}
                        />
                    )}
                </Index>
            </div>
        </div>
    );
}

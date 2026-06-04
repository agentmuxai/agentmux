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
 *  │  │  height = virtualizer.getTotalSize()             │ │
 *  │  │  position: relative                              │ │
 *  │  │  rows: position: absolute, translateY(start)     │ │
 *  │  │  measureElement on each row                      │ │
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

import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, createSignal, For, Index, onCleanup, onMount, type Accessor, type JSX } from "solid-js";
import { trail } from "@/log/render-trail";
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
} from "@/app/store/agent-pane-layout-store";
import { effectiveHeight } from "@/app/store/agent-pane-layout/reducer";
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
     * `.agent-view` in agent-view.tsx). Used to normalize `measureElement`
     * into unzoomed CSS px so virtualizer row offsets don't double-count zoom
     * and overlap — see SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01.
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
     * blockId for the agent-pane-layout slice (Phase 2+). When present,
     * `estimateSize` reads `effectiveHeight` from the slice (measured >
     * estimate > default) and `measureElement` dispatches `RowMeasured`
     * keyed by the row's current expansion state (INV-3).
     */
    blockId?: string;
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

    // Memoized — partition is read inside virtualizer's reactive
    // count getter, estimateSize, getItemKey, the streaming buffer
    // <For>, and each virtual item's nodeAccessor. Without
    // createMemo, every read re-slices the document (O(n)) on every
    // token of streaming, defeating the streaming buffer's purpose.
    // (reagent P1 on #784.)
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

    // Virtualizer over the virtualized head only — streaming buffer is
    // rendered separately as a normal flex list. Per-kind estimators
    // come from the renderer registry; measureElement settles each row
    // to actual height after first paint.
    const virtualizer = createVirtualizer({
        get count() { return partition().virtualizedNodes.length; },
        getScrollElement: () => scrollRef,
        estimateSize: (index) => {
            const node = partition().virtualizedNodes[index];
            if (!node) return 32;
            // Phase 2: prefer the slice's effective height (measured > estimate >
            // DEFAULT_ROW_PX) over the direct per-kind estimator. This is the core
            // INV-3 fix: when a row switches expansion state the slice immediately
            // returns the cached height for the NEW state instead of the wrong-state
            // value TanStack would have kept in its cache.
            if (props.blockId) {
                const s = snapshotLayout(props.blockId);
                if (s) return effectiveHeight(s, node.id);
            }
            return estimateNode(node, props.documentState());
        },
        overscan: 5,
        getItemKey: (index) => partition().virtualizedNodes[index]?.id ?? index,
        // scrollMargin: any content rendered above the virtualizer
        // container (loading-older banner, auth-url box from the
        // parent) shifts the virtual coordinate system. Read offsetTop
        // as a getter so it's evaluated each tick.
        get scrollMargin() {
            return virtualContainerRef?.offsetTop ?? 0;
        },
        // Phase 3 perf probe: validate the per-kind estimator against
        // the measured size after every measurement. The HUD surfaces
        // any kind whose miss-rate stays high so we recalibrate.
        // No-op in production builds (agentPerfStore short-circuits).
        measureElement: (element) => {
            // Normalize OUT the per-pane CSS `zoom`: getBoundingClientRect is
            // zoom-scaled (verified cef-146: css×zoom), but estimateSize returns
            // fixed unzoomed CSS px. Without this, the two units disagree at
            // zoom≠1 and the cumulative translateY offsets double-count zoom →
            // rows overlap (zoom<1) or gap (zoom>1). Dividing by the live zoom
            // factor keeps the virtualizer entirely in zoom-independent CSS px.
            // SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01 §4.1.
            const zoom = props.zoomFactor?.() ?? 1;
            const measured = element.getBoundingClientRect().height / (zoom || 1);
            const indexAttr = element.getAttribute("data-index");
            if (indexAttr != null) {
                const idx = Number(indexAttr);
                const node = partition().virtualizedNodes[idx];
                if (node) {
                    const estimated = estimateNode(node, props.documentState());
                    agentPerfStore.recordEstimatorMeasurement(node.type, estimated, measured);
                    // Phase 2: dispatch RowMeasured keyed by the row's CURRENT
                    // expansion state (INV-3 — an expanded measurement must land in
                    // the expanded slot, not the collapsed slot, and vice versa).
                    // `dispatchLayoutIfRegistered` is a no-op if the pane isn't
                    // wired (e.g. during tests), so this is always-safe.
                    if (props.blockId) {
                        const s = snapshotLayout(props.blockId);
                        const expState = inFlowState(s?.expansion.get(node.id));
                        dispatchLayoutIfRegistered(props.blockId, {
                            type: "RowMeasured",
                            nodeId: node.id,
                            state: expState,
                            cssPx: measured,
                        });
                    }
                }
            }
            return measured;
        },
    });

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
    // (user wants to stay where they jumped). Routes through the
    // virtualizer if the target is in the virtualized head, else uses
    // the streaming buffer's natural DOM offset.
    const scrollToNode = (nodeId: string): void => {
        if (!scrollRef) return;
        const idx = props.viewState.indexOf(nodeId);
        if (idx < 0) return;
        const p = partition();
        if (idx < p.splitIndex) {
            // Virtualized region — virtualizer handles ensure-visible + scroll.
            virtualizer.scrollToIndex(idx, { align: "center", behavior: "smooth" });
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
            //   1) Document is large enough to virtualize → use the
            //      topmost virtual item. Note: in TanStack Virtual v3
            //      `virtualItem.start` ALREADY includes scrollMargin
            //      so we read it directly without re-adding the margin.
            //   2) Document fits entirely in the streaming buffer
            //      (≤ STREAMING_BUFFER_SIZE nodes) → getVirtualItems()
            //      is empty, so fall back to the streaming buffer's
            //      first node via DOM. This is the common initial-
            //      pagination case (codex P2 on #784).
            const items = virtualizer.getVirtualItems();
            let anchorId: string | null = null;
            let anchorOffsetPx = 0;
            if (items.length > 0) {
                anchorId = partition().virtualizedNodes[items[0].index]?.id ?? null;
                anchorOffsetPx = items[0].start;
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
                    // streaming buffer could be classified as
                    // virtualized, then `getOffsetForIndex(newIdx)`
                    // would be called with an index past the
                    // virtualizer's count and the restore would
                    // silently fail. Codex P2 on PR #1101.
                    const anchor = props.viewState.headAnchor();
                    if (anchor != null && scrollRef) {
                        const newIdx = props.viewState.indexOf(anchor.nodeId);
                        if (newIdx >= 0) {
                            const newP = partition();
                            if (newIdx < newP.splitIndex) {
                                // Still virtualized — recompute via virtualizer's offsetForIndex.
                                // The returned offset already includes scrollMargin in v3,
                                // so don't add virtualContainerRef.offsetTop again.
                                const offset = virtualizer.getOffsetForIndex(newIdx, "start");
                                if (offset != null) {
                                    const target = restoreScrollFromAnchor(anchor, offset[0]);
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

    // Render: scroll container holds the optional header, the
    // virtualized head, and the streaming buffer. headerSlot offsets
    // the virtualizer; scrollMargin (above) handles that automatically.
    return (
        <div class="agent-document" ref={scrollRef} onScroll={handleScroll}>
            {props.headerSlot}
            {/* Virtualized head — only present when document > buffer size */}
            <div
                ref={(el) => { virtualContainerRef = el; }}
                class="agent-document-virtualizer"
                style={{
                    height: `${virtualizer.getTotalSize()}px`,
                    position: "relative",
                    width: "100%",
                }}
            >
                <For each={virtualizer.getVirtualItems()}>
                    {(virtualItem) => {
                        // Defensive: during the reflow that follows a tool
                        // expand/collapse height change, getVirtualItems() can
                        // transiently yield an undefined entry. Without this
                        // guard the `virtualItem.index` reads below throw
                        // "Cannot read properties of undefined (reading 'index')"
                        // *during render*, which the agent-pane error boundary
                        // catches by tearing down the ENTIRE virtualized list.
                        // Dropping one transient row for a frame is recoverable;
                        // crashing the list is not. (Observed live under rapid
                        // expand/collapse churn.)
                        if (!virtualItem) return null;
                        const nodeAccessor = (): DocumentNode => {
                            return partition().virtualizedNodes[virtualItem.index];
                        };
                        return (
                            <DocumentRow
                                node={nodeAccessor}
                                documentState={props.documentState}
                                bookmarkedNodeIds={props.bookmarkedNodeIds}
                                onBookmark={props.onBookmark}
                                onSubagentClick={props.onSubagentClick}
                                highlightNodeId={props.highlightNodeId}
                                onToggleCollapse={props.onToggleCollapse}
                                onTogglePin={props.onTogglePin}
                                // data-index is also bound reactively below
                                // (dataIndex prop), but that binding is a Solid
                                // render-effect that RACES this ref callback. If
                                // the ref wins, TanStack's measureElement reads a
                                // null data-index and returns *before*
                                // observer.observe(node) — so the row is never
                                // observed and stays pinned at estimateSize
                                // forever, overlapping its neighbors. Setting the
                                // attribute synchronously here, right before
                                // measureElement, closes that race. Verified via
                                // live CDP: rows mounting on the losing side of
                                // the race were stuck at the 32px estimate.
                                ref={(el) => {
                                    el.setAttribute("data-index", String(virtualItem.index));
                                    virtualizer.measureElement(el);
                                }}
                                dataIndex={virtualItem.index}
                                style={{
                                    position: "absolute",
                                    top: "0",
                                    left: "0",
                                    width: "100%",
                                    transform: `translateY(${virtualItem.start - (virtualContainerRef?.offsetTop ?? 0)}px)`,
                                }}
                            />
                        );
                    }}
                </For>
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

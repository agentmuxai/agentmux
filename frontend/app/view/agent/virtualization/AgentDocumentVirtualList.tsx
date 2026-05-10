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
import { createEffect, createMemo, For, Index, onMount, type Accessor, type JSX } from "solid-js";
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
import { estimateNode } from "./renderers";
import type { AgentViewState } from "./state";
import { partitionForVirtualization } from "./streaming-buffer";

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
     * Content rendered inside the scroll container, above the
     * virtualized region. Used by AgentDocumentView for the
     * auth-url box and the loading-older banner. Affects scroll math
     * via scrollMargin (read from virtualContainerRef.offsetTop).
     */
    headerSlot?: JSX.Element;
}

export function AgentDocumentVirtualList(props: AgentDocumentVirtualListProps): JSX.Element {
    let scrollRef!: HTMLDivElement;
    let virtualContainerRef: HTMLDivElement | undefined;
    // Guard against concurrent older-history fetches triggered by scroll.
    let loadingOlderInFlight = false;

    // Memoized — partition is read inside virtualizer's reactive
    // count getter, estimateSize, getItemKey, the streaming buffer
    // <For>, and each virtual item's nodeAccessor. Without
    // createMemo, every read re-slices the document (O(n)) on every
    // token of streaming, defeating the streaming buffer's purpose.
    // (reagent P1 on #784.)
    const partition = createMemo(() => {
        return partitionForVirtualization(props.viewState.nodes());
    });

    // Virtualizer over the virtualized head only — streaming buffer is
    // rendered separately as a normal flex list. Per-kind estimators
    // come from the renderer registry; measureElement settles each row
    // to actual height after first paint.
    const virtualizer = createVirtualizer({
        get count() { return partition().virtualizedNodes.length; },
        getScrollElement: () => scrollRef,
        estimateSize: (index) => {
            const node = partition().virtualizedNodes[index];
            return node ? estimateNode(node, props.documentState()) : 32;
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
            const measured = element.getBoundingClientRect().height;
            const indexAttr = element.getAttribute("data-index");
            if (indexAttr != null) {
                const idx = Number(indexAttr);
                const node = partition().virtualizedNodes[idx];
                if (node) {
                    const estimated = estimateNode(node, props.documentState());
                    agentPerfStore.recordEstimatorMeasurement(node.type, estimated, measured);
                }
            }
            // Return the measured height. (reagent P2 on #785: dead
            // ternary removed — both branches returned `measured`.)
            return measured;
        },
    });

    // Layout-shift observer scoped to .agent-document. Idempotent —
    // subsequent mounts are no-ops. Production: short-circuits.
    onMount(() => {
        startAgentLayoutShiftObserver();
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
                    const anchor = props.viewState.headAnchor();
                    if (anchor != null && scrollRef) {
                        const newIdx = props.viewState.indexOf(anchor.nodeId);
                        if (newIdx >= 0) {
                            const newP = partitionForVirtualization(props.viewState.nodes());
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
                                ref={virtualizer.measureElement}
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
            <div class="agent-document-streaming-buffer">
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

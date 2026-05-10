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
import { createEffect, For, type Accessor, type JSX } from "solid-js";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
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

    const partition = (): ReturnType<typeof partitionForVirtualization> => {
        return partitionForVirtualization(props.viewState.nodes());
    };

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
            // Capture anchor from the topmost virtualized item — its
            // offsetPx is virtualItem.start + scrollMargin.
            const items = virtualizer.getVirtualItems();
            const anchorId = items.length > 0
                ? partition().virtualizedNodes[items[0].index]?.id
                : null;
            if (anchorId != null) {
                const items0 = items[0];
                const anchorOffsetPx = (items0.start) + (virtualContainerRef?.offsetTop ?? 0);
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
                                const offset = virtualizer.getOffsetForIndex(newIdx, "start");
                                if (offset != null) {
                                    const target = restoreScrollFromAnchor(
                                        anchor,
                                        offset[0] + (virtualContainerRef?.offsetTop ?? 0),
                                    );
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

            {/* Streaming buffer — always-mounted trailing nodes */}
            <div class="agent-document-streaming-buffer">
                <For each={partition().streamingNodes as DocumentNode[]}>
                    {(node) => {
                        const nodeAccessor = (): DocumentNode => node;
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
                            />
                        );
                    }}
                </For>
            </div>
        </div>
    );
}

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

import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, untrack, type Accessor, type JSX } from "solid-js";
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
    /** Hold a tool expanded after it completes live on screen (ToolBlock calls
     *  this on the active→inactive transition). */
    onHoldToolOpen?: (id: string) => void;
    /** Release a held tool once its row has scrolled off the top (latched
     *  collapse). Invoked by the scroll-off scan in `handleScroll`. */
    onReleaseToolOpen?: (id: string) => void;
    /**
     * Live per-pane zoom factor (the same value applied as CSS `zoom` on
     * `.agent-view` in agent-view.tsx). Normalizes the SINGLE zoomed read —
     * the measure RO's getBoundingClientRect — into unzoomed CSS px so slice
     * positions don't double-count zoom and overlap (Phase 4). scrollTop /
     * clientHeight / offsetTop are already unzoomed under CSS `zoom`, so they
     * feed the slice raw — see
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

    // Sticky frontier id — set once when the document first crosses
    // STREAMING_BUFFER_SIZE; advanced whenever the buffer exceeds the
    // cap (see below) to keep streamingNodes.length ≤ STREAMING_BUFFER_SIZE.
    //
    // Plain `let` rather than a Solid signal: mutating this variable must
    // NOT trigger another memo run while we're inside one.
    //
    // Cleared when the anchor node is truncated (e.g., history reset /
    // pane re-mount); the next partition recompute re-anchors to the new
    // tail.
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
    // <Key>. Without createMemo, every read re-slices the document
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
        // recompute once.
        if (result.splitIndex === -1) {
            stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
            result = partitionForVirtualization(
                nodes,
                STREAMING_BUFFER_SIZE,
                stickyFrontierId,
            );
        }

        // Cap: across multi-turn sessions the streaming buffer grows
        // unbounded because the sticky frontier never advances. Fix:
        // advance the frontier to pin streamingNodes at exactly
        // STREAMING_BUFFER_SIZE items. The cap runs inside createMemo
        // before any render effect reads the result — the intermediate
        // 51-item state is never observable by <Index>; reconcileArrays
        // always sees a fixed-size 50-item array (50 → 50, not 50 → 51)
        // and never encounters a growing array. (#1302)
        //
        // WHY <Index> NOT <Key>: rapid consecutive cap-advances cause
        // <Key>'s keyArray to dispose slots synchronously (removing their
        // inner DOM content) while reconcileArrays still holds stale
        // element refs → "replaceChild: node not a child" crash (§7.4,
        // SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md).
        // <Index> avoids reconcileArrays DOM rearrangements entirely at
        // steady-state 50 items: position-slot signals update in place,
        // no DOM moves. ToolBlock guards against same-slot state leakage
        // by resetting prevStatus when the node id changes. (#1317)
        if (result.streamingNodes.length > STREAMING_BUFFER_SIZE) {
            stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
            result = partitionForVirtualization(nodes, STREAMING_BUFFER_SIZE, stickyFrontierId);
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

    // ── Phase 3: feed the layout slice's VIEWPORT inputs ────────────────
    // The slice records scrollTop / viewport / scrollMargin / zoom and the
    // render path (windowing + prefix-sum row positions) reads them back. All
    // blockId-gated and reducer-deduped (an unchanged value returns the same
    // state ref). Slice positions are unzoomed CSS px; scrollTop / clientHeight /
    // offsetTop are ALSO unzoomed under the ancestor CSS `zoom` (Phase 4 —
    // CDP-confirmed at zoom 0.5/2: ratio 1.0; only getBoundingClientRect is
    // zoomed), so they feed the slice raw — the lone ÷zoom is at the measure RO.
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
            // The header (auth box / loading-older banner) sits ABOVE the
            // container; it shifts the container's offsetTop WITHOUT resizing
            // the container itself, so the RO above never fires for it. The
            // header moves offsetTop two ways, and we watch both:
            //   • add/remove (a Show toggles the banner/auth box) → childList
            //     on scrollRef.
            //   • internal resize (AuthUrlBox grows when its paste-result
            //     appears inside the existing box) → no childList mutation, so
            //     a ResizeObserver on the header element itself catches it.
            // headerRO observes only the 0-2 header elements that precede the
            // container (never the virtualized rows), so there's no scroll-time
            // reflow storm. Keeping scrollMarginPx in sync matters because row
            // starts include it while the render path, windowing, and
            // scroll-to-node subtract the live offsetTop.
            const headerRO = new ResizeObserver(() => dispatchScrollMargin());
            const observeHeaders = (): void => {
                for (const child of Array.from(scrollRef.children)) {
                    if (child === virtualContainerRef) break;
                    headerRO.observe(child); // idempotent per element
                }
            };
            observeHeaders();
            const mo = new MutationObserver(() => {
                dispatchScrollMargin();
                observeHeaders();
            });
            mo.observe(scrollRef, { childList: true });
            onCleanup(() => { ro.disconnect(); headerRO.disconnect(); mo.disconnect(); });
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
                // Re-check: user may have disengaged stickToBottom between
                // when this microtask was queued and when it fires.
                if (!props.viewState.stickToBottom()) return;
                scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
                // New content may have pushed a held-open tool above the top
                // without a user scroll event — collapse it now (pinned to
                // bottom, so no visible jump).
                collapseScrolledOffTools();
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
            // Phase 3: the scroll container resizing changes the viewport the
            // slice windows against — feed it (covers hidden→visible 0→N and
            // pane resize). blockId-gated, reducer-deduped. scrollTop / h are
            // unzoomed CSS px (Phase 4 — only the measure RO ÷zooms).
            if (props.blockId && h > 0) {
                dispatchLayoutIfRegistered(props.blockId, {
                    type: "Scrolled",
                    scrollTop: scrollRef.scrollTop,
                    viewportPx: h,
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
            // (Step 5). row.start/height and the scroll element's clientHeight /
            // scrollTop are all unzoomed CSS px (Phase 4 — CSS `zoom` leaves them
            // in layout px), so no zoom conversion is needed here.
            const v = props.layoutView?.();
            const row = v?.rows[idx];
            if (row) {
                const viewportPx = scrollRef.clientHeight;
                const center = row.start - viewportPx / 2 + row.height / 2;
                scrollRef.scrollTo({ top: Math.max(0, center), behavior: "smooth" });
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
    // untrack the scroll so this effect depends ONLY on the command signal:
    // scrollToNode reads layoutView / partition, both of which change on
    // every scroll, and useScrollToNode holds the last command rather than
    // clearing it — tracking them would re-fire the jump on each scroll and pin
    // the user on the old target.
    createEffect(() => {
        const cmd = props.scrollCommand?.();
        if (cmd) untrack(() => scrollToNode(cmd.nodeId));
    });

    // Release (latched-collapse) any held-open tool whose row has scrolled fully
    // above the viewport top. DOM-based so it works uniformly for virtualized
    // and streaming-buffer rows and is zoom-safe (both rects share the zoomed
    // space). `expandedTools` is tiny (a few recently-completed tools), so this
    // is a handful of lookups per scroll. Pinned tools are skipped — pin wins.
    const collapseScrolledOffTools = (): void => {
        const release = props.onReleaseToolOpen;
        if (!release || !scrollRef) return;
        const ds = props.documentState();
        if (ds.expandedTools.size === 0) return;
        const containerTop = scrollRef.getBoundingClientRect().top;
        for (const id of ds.expandedTools) {
            if (ds.pinnedNodes.has(id)) continue;
            const el = scrollRef.querySelector(
                `[data-node-id="${id}"]`,
            ) as HTMLElement | null;
            // No element → not rendered → already off-screen (unmounted): release
            // as a safety net. Otherwise release once it's fully above the top.
            if (!el || el.getBoundingClientRect().bottom <= containerTop) {
                release(id);
            }
        }
    };

    const handleScroll = (): void => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;

        // Phase 4: scrollTop / clientHeight are already unzoomed CSS px under the
        // ancestor CSS `zoom` (CDP-confirmed: ratio 1.0 at zoom 0.5/2 — only
        // getBoundingClientRect is zoomed), so push them raw. The lone ÷zoom is
        // at the measure RO. Reducer-deduped; blockId-gated.
        if (props.blockId) {
            dispatchLayoutIfRegistered(props.blockId, {
                type: "Scrolled",
                scrollTop,
                viewportPx: clientHeight,
            });
            dispatchScrollMargin();
        }

        // Collapse held-open tools that have scrolled off the top (latched).
        // Gated on stick-to-bottom: collapsing a row above the fold shrinks
        // layout height above scrollTop, which is invisible while pinned to the
        // bottom (no jump) — the primary live-streaming case. The scrolled-up /
        // anchor-compensated case is a documented Phase 2 follow-up.
        if (props.viewState.stickToBottom()) {
            collapseScrolledOffTools();
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
            {/* Virtualized head — only present when document > buffer size.
                Uses <Key by={r => r.nodeId}> (identity-keyed) so each DOM
                slot is permanently tied to one nodeId. The old <Index>
                (position-keyed) approach required an extra createEffect to
                keep elNodeId current whenever the window shifted a different
                node into the same position slot; without it measureRO would
                dispatch RowMeasured for the wrong nodeId. <Key> eliminates
                that complexity: each slot's lifecycle matches its node's
                lifecycle exactly. (#1319, #1326) */}
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
                        // momentarily absent from the partition (recoverable).
                        const node = (): DocumentNode | undefined => nodeById().get(row().nodeId);
                        onCleanup(() => { if (rowEl) unobserveRow(rowEl); });
                        return (
                            <Show when={node()}>
                                {(n) => (
                                    <DocumentRow
                                        node={n}
                                        documentState={props.documentState}
                                        onSubagentClick={props.onSubagentClick}
                                        highlightNodeId={props.highlightNodeId}
                                        onToggleCollapse={props.onToggleCollapse}
                                        onTogglePin={props.onTogglePin}
                                        onHoldToolOpen={props.onHoldToolOpen}
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
                Uses <Key by={n => n.id}> rather than <For> or <Index>:
                  - <For> reconciles by item REFERENCE: each streaming
                    token replaces the active node's object (immutable
                    update preserves id but gives a new ref), so <For>
                    would unmount/remount the row on every token.
                    (reagent P1 on #784.)
                  - <Index> is position-keyed: slot state (prevStatus,
                    expanded flag in ToolBlock) leaks to whatever node
                    lands at the same position after a cap-advance
                    migration. Also still crashes if the array grows
                    (reconcileArrays sentinel corruption) — the keying
                    strategy doesn't matter; growth does. (#1326)
                  - <Key by={n => n.id}> keys each slot by stable node
                    id. Token updates (new object ref, same id) fire the
                    slot's accessor without remount. Cap-advance migrations
                    dispose the departing slot by id cleanly — no position
                    state leaks. The array never grows because the
                    partition memo caps streamingNodes at
                    STREAMING_BUFFER_SIZE before any render effect sees
                    it, so reconcileArrays always sees a fixed-size array.
                    (#1302, #1317, #1326)

                <Show when={partition()}> guards against the secondary
                crash: if the memo is read after its reactive scope is
                disposed (mid-error-boundary teardown) the <Show> catches
                the falsy result before it reaches <Key>, preventing
                "Cannot read properties of undefined (reading
                'streamingNodes')". The narrowed `p()` isolates <Key>
                in its own child scope so it is disposed before the outer
                memo can produce a stale read.
                (SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §3.3) */}
            <Show when={partition()}>
                {(p) => (
                    <div class="agent-document-streaming-buffer" data-animate={animateEnabled() || undefined}>
                        <Key each={p().streamingNodes as DocumentNode[]} by={(n) => n.id}>
                            {(nodeAccessor) => (
                                <DocumentRow
                                    node={nodeAccessor}
                                    documentState={props.documentState}
                                    onSubagentClick={props.onSubagentClick}
                                    highlightNodeId={props.highlightNodeId}
                                    onToggleCollapse={props.onToggleCollapse}
                                    onTogglePin={props.onTogglePin}
                                    onHoldToolOpen={props.onHoldToolOpen}
                                />
                            )}
                        </Key>
                    </div>
                )}
            </Show>
        </div>
    );
}

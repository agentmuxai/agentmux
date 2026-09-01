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

import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, Show, untrack, type Accessor, type JSX } from "solid-js";
import { trail } from "@/log/render-trail";
import { Key } from "@solid-primitives/keyed";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { AgentDispatch } from "../../swarm/swarm-model";
import type { DocumentNode, DocumentState } from "../types";
import { agentPerfStore, startAgentLayoutShiftObserver } from "./perf-probe";
import {
    captureTopmostAnchor,
    isNearBottom,
    isNearTop,
    restoreScrollFromAnchor,
} from "./anchor";
import { DocumentRow } from "./DocumentRow";
import { ShrinkTrace, formatAttribution } from "./shrink-trace";
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
    /** Re-run the provider login flow — forwarded to each row so an inline
     *  auth-error node can offer a "Login Again" CTA (SPEC_REAUTH_FROM_AUTH_ERROR §7). */
    onAgentErrorLogin?: () => void;
    /** Open/focus the Agent History tab — forwarded to each row so a
     *  `history_link` synthetic node can act on click. See
     *  SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2. */
    onOpenHistory?: () => void;
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
    /**
     * AgentWorkingRow's current rendered height in px (0 when hidden) —
     * agent-view.tsx measures it via ResizeObserver and uses the same value
     * to size .agent-document's bottom padding (the row floats over this
     * scroll container as an overlay; see
     * docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md
     * §3.2). Tracked by the stick-to-bottom effect below for the same
     * reason nodes().length and layoutView().totalSize are: when this
     * grows while pinned to bottom (a turn starts, tool-name text
     * widens the row), .agent-document's effective content height grows
     * too, and without re-pinning, the newly-taller overlay can end up
     * covering the previously-visible tail of the message list.
     */
    workingRowHeight?: Accessor<number>;
    /** Ordinal-matched tool_use_id -> live dispatch, for this pane's
     *  Agent/Task/Workflow tool nodes. See `activity/dispatch-correlation.ts`. */
    dispatchMatches?: Accessor<Map<string, AgentDispatch>>;
}

export function AgentDocumentVirtualList(props: AgentDocumentVirtualListProps): JSX.Element {
    let scrollRef!: HTMLDivElement;
    let virtualContainerRef: HTMLDivElement | undefined;
    // Streaming-buffer's own element — observed by the content-resize pin
    // below (never used for anything else, so it's fine that it's only set
    // once the buffer first mounts).
    let streamingBufferRef: HTMLDivElement | undefined;
    // Guard against concurrent older-history fetches triggered by scroll.
    let loadingOlderInFlight = false;
    // RAF-coalesced scroll handling (task #39): native `scroll` events can
    // fire dozens of times per drag/wheel gesture; without coalescing, each
    // one dispatches a layout `Scrolled` command and re-derives the window.
    // Mirrors the `scheduleFlush`/`flushRafId` idiom useAgentStream.ts uses to
    // batch stream chunks into one dispatch per animation frame — same
    // "guard with a nullable rAF handle" pattern, applied to scroll here.
    // `handleScrollNow` reads scrollRef's LIVE values (not anything captured
    // from the event), so it's safe to defer to the next frame: whichever
    // scroll position is current when the frame runs is the one applied.
    let scrollRafId: number | null = null;

    // Set immediately before every programmatic scrollTo() call below
    // (pin-to-bottom effect, ResizeObserver re-pin, jumpToBottom), consumed
    // by the very next handleScrollNow() call. A `scrollTo()` call itself
    // fires a native `scroll` event, coalesced by scrollRafId into the same
    // handleScrollNow() batch as any other scroll activity in that frame —
    // this flag lets that call distinguish "this scroll event resulted from
    // OUR OWN auto-scroll" from a genuine user scroll landing in the same
    // batch, so the disengage branch below doesn't misattribute it. Third
    // documented attempt at the "scroll-follow silently stops" class of bug
    // (docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md,
    // docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md §3/§5)
    // — this is the "input-gating race" both prior passes deferred pending
    // live reports continuing, which they did.
    let pendingProgrammaticScroll = false;
    // Last scrollHeight observed at a pin-correction call — diagnostic only,
    // see docs/analysis/ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md.
    // A shrink between two consecutive calls means content that was already
    // laid out at true bottom got shorter while pinned (a tool-call
    // running->terminal swap, a spinner-run reclassification, a markdown
    // re-highlight reflow, ...). By the time THIS function runs, the
    // browser's own scrollTop auto-clamp for that shrink has already
    // happened synchronously as part of the layout pass that produced it —
    // there is nothing left here to animate away (confirmed against
    // PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md §2's "H1" math, which
    // established the same clamp-is-already-synchronous fact for the grow
    // case). This log exists purely so a live repro
    // (`muxlog fe grep wave-scroll-shrink`) can identify WHICH content
    // change produced a given visible "scroll went backward" report — the
    // real fix has to happen upstream, at whatever DOM swap caused the
    // shrink (e.g. a FLIP-style freeze-then-ease on that element), not here.
    // Keeps isOverflowing() current in both directions from live geometry.
    // MUST run unconditionally from every observation point (handleScrollNow,
    // both ResizeObservers, the itemized effect) — NOT only from paths gated
    // on stickToBottom() already being true. A pane can collapse to
    // non-overflowing (or grow past it) while the user is scrolled away
    // reading history — a `/clear` wiping the transcript, or the documented
    // whole-pane scrollHeight->0px collapse, mid-history-read — and every
    // stickToBottom()-gated call site (all of RO #1/#2's re-pin, the
    // itemized effect's re-pin, and scrollToTrueBottom itself) would
    // otherwise never observe it, leaving isOverflowing() stale until the
    // user manually re-engages. This was reagent P1's finding on the first
    // review-fix pass (PR #2834) — the original version of this file only
    // updated the flag from inside scrollToTrueBottom, which is exactly one
    // of those gated call sites.
    function syncOverflowState(): void {
        if (!scrollRef) return;
        const overflowing = scrollRef.scrollHeight > scrollRef.clientHeight;
        if (overflowing !== props.viewState.isOverflowing()) {
            if (overflowing) {
                props.viewState.markOverflowing();
            } else {
                props.viewState.markNotOverflowing();
            }
        }
    }

    // Per-node shrink attribution for the `[wave-scroll-shrink]` line below.
    // Fed by `shrinkRO` (streaming-buffer rows), drained when a pane shrink is
    // detected. Step 1 of SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md.
    const shrinkTrace = new ShrinkTrace();

    let lastKnownScrollHeight = 0;
    function scrollToTrueBottom(): void {
        if (!scrollRef) return;
        const h = scrollRef.scrollHeight;
        if (lastKnownScrollHeight > 0 && h < lastKnownScrollHeight - 1) {
            const paneDelta = lastKnownScrollHeight - h;
            console.info(
                "[wave-scroll-shrink]",
                `pane=${props.blockId?.slice(0, 7) ?? "?"}`,
                `scrollHeight ${lastKnownScrollHeight}px -> ${h}px (delta=${paneDelta}px)`,
                // Which row(s) actually got shorter, and how much of the pane
                // delta they fail to explain — see shrink-trace.ts. Without
                // this suffix the line above is a bare net number, which is
                // precisely what limited every conclusion in the 08-21/08-22
                // findings docs.
                formatAttribution(shrinkTrace.attribute(paneDelta, performance.now())),
            );
        }
        lastKnownScrollHeight = h;
        syncOverflowState();
        pendingProgrammaticScroll = true;
        scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
    }

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
            // `dispatchLayoutIfRegistered` synchronously calls the slot's
            // `proj.layout(view)` callback — which IS `setLayoutView`, a real
            // Solid signal setter (registerLayoutPane wires it directly). Each
            // of the dispatches below (NodesChanged, then up to 2×N
            // EstimateSet, then up to N ExpansionResolved) can independently
            // produce a DIFFERENT LayoutView and fire it unbatched — so
            // `windowedRows()` (and the `<Key>` it drives) could re-render
            // several times per logical document update, each against a
            // PARTIALLY-updated slice (e.g. `NodesChanged` landing with a
            // node's estimate/expansion not yet pushed). On a small
            // incremental change (the common live-streaming case) the
            // intermediate views are usually equivalent enough that nothing
            // visibly breaks — but a LARGE simultaneous batch (every node
            // changing identity at once, e.g. AgentHistoryView's wholesale
            // reparse-on-loadOlder, SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md)
            // hits this glitch window hard enough to crash `reconcileArrays`
            // ("replaceChild: node not a child" — a stale intermediate
            // `windowedRows()` row still referencing a DOM node an even-more-
            // intermediate pass had already removed). `batch()` defers every
            // signal write started inside it until the callback returns, so
            // `windowedRows()` observes exactly ONE consistent LayoutView per
            // logical update instead of N glitching ones — the standard Solid
            // fix for exactly this class of mid-transaction inconsistency.
            batch(() => {
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
    //
    // Also tracks layoutView().totalSize (below) — not just nodes().length.
    // A row's rendered height can change WITHOUT the node count changing
    // (a tool panel collapsing/expanding, code re-highlighting, an image
    // loading) — measureRO dispatches RowMeasured for that independently of
    // this effect, so without this second dependency, an off-screen row's
    // height change silently desyncs scrollTop from true bottom: nothing
    // here re-runs, CSS overflow-anchor doesn't reliably cover it either
    // (rows are absolutely positioned, outside anchor-candidate selection),
    // and stickToBottom itself never flips — the drop is invisible in state
    // inspection. See docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md §2.
    //
    // Also tracks workingRowHeight (below) — reagent P1 on #2292: that same
    // spec's §3.2 overlay (AgentWorkingRow floating over this scroll
    // container's bottom edge, sized via .agent-document's padding-bottom)
    // introduced an identical gap. When the row appears or grows while
    // pinned to bottom (a turn starts, tool-name text widens it),
    // .agent-document's effective content height changes without any
    // node-count or layoutView change, so without this dependency the
    // newly-taller overlay could end up covering the message that was
    // previously visible at the bottom.
    createEffect(() => {
        // Track length changes — Solid will re-run when nodes() emits.
        const _len = props.viewState.nodes().length;
        const _totalSize = props.layoutView?.()?.totalSize;
        const _workingRowHeight = props.workingRowHeight?.();
        // Unconditional — a node-count drop (e.g. /clear) can collapse this
        // pane to non-overflowing while scrolled away reading history; see
        // syncOverflowState's own doc comment.
        syncOverflowState();
        if (props.viewState.stickToBottom() && scrollRef) {
            // queueMicrotask so the new content has rendered before we scroll.
            queueMicrotask(() => {
                if (!scrollRef) return;
                // Re-check: user may have disengaged stickToBottom between
                // when this microtask was queued and when it fires.
                if (!props.viewState.stickToBottom()) return;
                scrollToTrueBottom();
                // New content may have pushed a held-open tool above the top
                // without a user scroll event — collapse it now (pinned to
                // bottom, so no visible jump).
                collapseScrolledOffTools();
            });
        }
    });

    // Re-apply sticky-bottom on ANY clientHeight change to this scroll
    // container — not just hidden → visible. Originally only handled the
    // inactive-tab case (display:none is 0×0, so the nodes()-driven scroll
    // above is a no-op while hidden and nothing re-issues it on
    // reactivation). Broadened per REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md's
    // scroll-follow-drift investigation: a *sibling* below this scroll
    // region resizing (the retry bar, AgentDecisionPanel, AgentQuestionPanel,
    // or PendingMessagesPanel appearing/growing mid-turn — all normal-flow,
    // flex-shrink:0 rows per SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md
    // §3.3's own "deferred as rare/transient" admission) shrinks THIS
    // container's clientHeight via pure CSS reflow — no scroll event fires
    // (it's a resize, not a scroll), and neither of the pin effect's tracked
    // deps (nodes().length, layoutView().totalSize, workingRowHeight) change
    // either, so `stickToBottom` stays silently true while the view falls
    // short of the new, smaller max scroll position. Rather than adding a
    // tracked height signal per interposing panel (the whack-a-mole pattern
    // the 07-24 fix already had to extend once), just re-pin on every resize
    // this observer reports — idempotent when already at true bottom, and
    // still gated on `stickToBottom()` so a user who deliberately scrolled
    // away is never fought.
    onMount(() => {
        const ro = new ResizeObserver(() => {
            const h = scrollRef.clientHeight;
            // Unconditional — see syncOverflowState's own doc comment.
            syncOverflowState();
            if (h > 0 && props.viewState.stickToBottom()) {
                scrollToTrueBottom();
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
        });
        ro.observe(scrollRef);
        onCleanup(() => ro.disconnect());
    });

    // Content-resize-driven pin. The effect above only re-pins when one of
    // three hand-picked signals changes (nodes().length, layoutView().totalSize,
    // workingRowHeight) — any OTHER source of the content growing taller is
    // invisible to it, and the scrollbar visibly sits above true bottom until
    // some unrelated signal happens to change and the effect re-runs. The
    // clientHeight RO right above this one doesn't catch it either — it only
    // fires on VIEWPORT resizes (scrollRef's own border-box), not content
    // growth (scrollRef's box is fixed by the flex layout regardless of how
    // tall its overflowing content gets).
    //
    // A concrete, confirmed example: MarkdownBlock throttles streaming
    // re-parses and fires a bare `setTimeout` ~90ms after the last token to
    // commit a syntax-highlighted re-render — a code block routinely reflows
    // to a different height than its plain-text intermediate. That write is
    // completely outside the Solid signal graph the effect above depends on.
    //
    // Fix: observe the two elements whose box size IS "how tall the content
    // actually is" — the virtualized region and the streaming buffer —
    // directly, and re-pin from ANY resize, regardless of what caused it.
    // ResizeObserver's notification step runs after layout but before paint
    // (the same guarantee the WICG spec's own chat-scroll example and
    // libraries like use-stick-to-bottom rely on), so writing scrollTop
    // synchronously here never produces a visible flash. See
    // docs/specs/REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md.
    //
    // workingRowHeight is NOT covered by this observer — it drives
    // `.agent-document`'s own padding-bottom (scrollRef's box, not a child's),
    // which changes scrollHeight without resizing either observed element.
    // It stays covered by the effect above.
    onMount(() => {
        if (typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver(() => {
            if (!scrollRef) return;
            // Unconditional — see syncOverflowState's own doc comment.
            syncOverflowState();
            if (!props.viewState.stickToBottom()) return;
            scrollToTrueBottom();
        });
        if (virtualContainerRef) ro.observe(virtualContainerRef);
        if (streamingBufferRef) ro.observe(streamingBufferRef);
        onCleanup(() => ro.disconnect());
    });

    // jumpToBottom: forced scroll-to-bottom that re-engages stick.
    // Exposed to parent so AgentFooter can call it on keystroke.
    const jumpToBottom = (): void => {
        if (!scrollRef) return;
        props.viewState.engageStickToBottom();
        scrollToTrueBottom();
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
        const containerRect = scrollRef.getBoundingClientRect();
        // A hidden pane (inactive tab: display:none, same "0×0" signature the
        // ResizeObserver comment above already relies on) or a minimized window
        // reports a zero-size rect for every element, not just this container's
        // children — a real "scrolled off the top" collapse can never be
        // determined when nothing has size, so skip the scan rather than
        // mass-releasing every held-open tool the instant hidden-but-still-
        // streaming content arrives (user-reported: tool blocks were
        // collapsing on tab-switch/window-minimize with no actual scroll).
        if (containerRect.width === 0 && containerRect.height === 0) return;
        const containerTop = containerRect.top;
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

    const handleScrollNow = (): void => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;

        // Consume the programmatic-scroll flag for THIS batch — see
        // scrollToTrueBottom's own comment. Read once, up front: every
        // branch below that might disengage needs to know whether this
        // scroll event batch was (at least partly) caused by our own
        // auto-scroll, not just the disengage branch specifically.
        const wasProgrammatic = pendingProgrammaticScroll;
        pendingProgrammaticScroll = false;

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

        // This pane's content has overflowed its viewport for the first
        // time since it last didn't (see isOverflowing's own doc comment
        // for why this isn't a one-time latch). Capture the PRE-sync
        // reading for the transition check, then sync unconditionally —
        // every scroll event is an observation point regardless of
        // stickToBottom, same as syncOverflowState's other call sites.
        const wasOverflowing = props.viewState.isOverflowing();
        syncOverflowState();
        const isFirstOverflow = scrollHeight > clientHeight && !wasOverflowing;

        // Engage stick when user scrolls back near bottom; disengage
        // otherwise. Engaging clears any captured headAnchor (atomic).
        const nearBottom = isNearBottom(scrollTop, scrollHeight, clientHeight);
        if (isFirstOverflow && props.viewState.stickToBottom() && !nearBottom) {
            // A pane that was still following when it hit its first overflow
            // has no legitimate reason for THIS event's geometry to read as
            // "far from bottom" — nowhere existed to scroll away to before
            // this instant, so a native scrollbar-insertion side effect or a
            // same-frame race is the far likelier explanation. Force a fresh
            // pin instead of trusting this one reading.
            //
            // Gated on stickToBottom() already being true: this must never
            // force an engage from an ALREADY, legitimately disengaged state
            // — most notably a headAnchor captured moments earlier by
            // in-flight older-history pagination (its restore's own
            // scrollTo() doesn't route through scrollToTrueBottom, so
            // isOverflowing() can still be transitioning here too). Reading
            // stickToBottom() false in that case means the capture already
            // ran before this event, so this branch correctly does nothing,
            // leaving the just-restored reading position alone (reagent P1
            // on PR #2834).
            //
            // Also gated on !nearBottom: if the raw geometry already reads
            // near bottom, forcing is a redundant no-op — only log/act when
            // this is a genuine save (codex P2 on PR #2834).
            console.info(
                "[wave-scroll-first-overflow]",
                `pane=${props.blockId?.slice(0, 7) ?? "?"}`,
                `forced engage — scrollTop=${scrollTop} scrollHeight=${scrollHeight} clientHeight=${clientHeight}`,
            );
            scrollToTrueBottom();
            // Stop here — the pagination check below reads the scrollTop
            // captured at the top of this event, now stale (we just forced
            // true bottom). Letting it run could re-capture a head anchor
            // from that stale near-top reading and immediately undo the pin
            // (codex P1 on PR #2834).
            return;
        }

        if (nearBottom) {
            if (!props.viewState.stickToBottom()) {
                props.viewState.engageStickToBottom();
            }
        } else {
            if (props.viewState.stickToBottom()) {
                const gapPx = scrollHeight - clientHeight - scrollTop;
                if (wasProgrammatic) {
                    // This scroll event batch included our own scrollTo()
                    // call — isNearBottom() reading "not near bottom" here
                    // means either new content landed in the same tick
                    // (the pin effect's own reactive deps will re-fire and
                    // re-scroll) or a user scroll got coalesced into the
                    // same frame. Either way, disengaging on THIS event
                    // would be misattributing our own auto-scroll (or a
                    // same-frame race) as the user scrolling away.
                    console.info(
                        "[wave-scroll]",
                        `pane=${props.blockId?.slice(0, 7) ?? "?"}`,
                        `suppressed disengage — programmatic scroll in this batch, gap=${gapPx}px`,
                    );
                } else {
                    console.info(
                        "[wave-scroll]",
                        `pane=${props.blockId?.slice(0, 7) ?? "?"}`,
                        `disengage — scrollTop=${scrollTop} scrollHeight=${scrollHeight} clientHeight=${clientHeight} gap=${gapPx}px`,
                    );
                    props.viewState.disengageStickToBottom();
                }
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

    // Coalesce native `scroll` events to at most one `handleScrollNow` call
    // per animation frame. `onScroll` below wires this, not `handleScrollNow`
    // directly.
    const handleScroll = (): void => {
        if (scrollRafId != null) return;
        scrollRafId = requestAnimationFrame(() => {
            scrollRafId = null;
            handleScrollNow();
        });
    };
    onCleanup(() => {
        if (scrollRafId != null) {
            cancelAnimationFrame(scrollRafId);
            scrollRafId = null;
        }
    });

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

    // Diagnostic-only RO over STREAMING-BUFFER rows — deliberately separate
    // from measureRO above rather than an extra branch inside it. measureRO
    // dispatches `RowMeasured` into the agent-pane-layout slice, and the slice
    // models the VIRTUALIZED partition only; feeding it streaming-buffer rows
    // would be a real behavior change (rows it has no positions for), not
    // instrumentation. The streaming buffer is also exactly where the
    // interesting shrinks happen — a completing tool call is in the trailing
    // N nodes by definition — and it is currently measured by nothing at all,
    // which is the concrete reason the pane-level diagnostic could never name
    // a culprit. Feeds `shrinkTrace`; nothing reads it except the
    // `[wave-scroll-shrink]` log line.
    const shrinkElNode = new WeakMap<Element, { id: string; type: string }>();
    const shrinkRO = typeof ResizeObserver !== "undefined"
        ? new ResizeObserver((entries) => {
            const zoom = props.zoomFactor?.() ?? 1;
            const now = performance.now();
            for (const entry of entries) {
                const meta = shrinkElNode.get(entry.target);
                if (!meta) continue;
                // ÷zoom for the same reason measureRO does it: getBoundingClientRect
                // is the one read that IS scaled by an ancestor CSS `zoom`, and
                // mixing zoomed and unzoomed px would make every attribution
                // wrong by the zoom factor at non-100% pane zoom.
                shrinkTrace.record(
                    meta.id,
                    meta.type,
                    entry.target.getBoundingClientRect().height / (zoom || 1),
                    now,
                );
            }
        })
        : undefined;
    onCleanup(() => shrinkRO?.disconnect());
    const observeStreamingRow = (el: HTMLElement, node: DocumentNode): void => {
        shrinkElNode.set(el, { id: node.id, type: node.type });
        shrinkRO?.observe(el);
    };
    const unobserveStreamingRow = (el: HTMLElement, nodeId: string): void => {
        shrinkRO?.unobserve(el);
        shrinkElNode.delete(el);
        // Drop the baseline too — a cap-advance can retire a tall row and
        // later re-add the same node short, which would otherwise log as one
        // fabricated shrink spanning the gap.
        shrinkTrace.forget(nodeId);
    };

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
                                        highlightNodeId={props.highlightNodeId}
                                        onToggleCollapse={props.onToggleCollapse}
                                        onTogglePin={props.onTogglePin}
                                        onHoldToolOpen={props.onHoldToolOpen}
                                        onAgentErrorLogin={props.onAgentErrorLogin}
                                        onOpenHistory={props.onOpenHistory}
                                        dispatchMatches={props.dispatchMatches}
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
                    <div
                        class="agent-document-streaming-buffer"
                        data-animate={animateEnabled() || undefined}
                        ref={(el) => { streamingBufferRef = el; }}
                    >
                        <Key each={p().streamingNodes as DocumentNode[]} by={(n) => n.id}>
                            {(nodeAccessor) => {
                                // Diagnostic shrink-attribution only (see
                                // shrinkRO above). The <Key> slot's own
                                // onCleanup is what makes the unobserve
                                // reliable across a cap-advance retiring this
                                // slot — same lifecycle hook the virtualized
                                // rows use for measureRO.
                                let rowEl: HTMLElement | undefined;
                                const nodeId = nodeAccessor().id;
                                onCleanup(() => { if (rowEl) unobserveStreamingRow(rowEl, nodeId); });
                                return (
                                    <DocumentRow
                                        node={nodeAccessor}
                                        documentState={props.documentState}
                                        highlightNodeId={props.highlightNodeId}
                                        onToggleCollapse={props.onToggleCollapse}
                                        onTogglePin={props.onTogglePin}
                                        onHoldToolOpen={props.onHoldToolOpen}
                                        onAgentErrorLogin={props.onAgentErrorLogin}
                                        onOpenHistory={props.onOpenHistory}
                                        dispatchMatches={props.dispatchMatches}
                                        ref={(el) => { rowEl = el; observeStreamingRow(el, nodeAccessor()); }}
                                    />
                                );
                            }}
                        </Key>
                    </div>
                )}
            </Show>
        </div>
    );
}

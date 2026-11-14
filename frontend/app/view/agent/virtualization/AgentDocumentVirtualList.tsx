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

import { Button } from "@/app/element/ui";
import { batch, createComputed, createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, untrack, type Accessor, type JSX, type Setter } from "solid-js";
import { trail } from "@/log/render-trail";
import { shouldReduceMotion } from "@/app/motion/motion";
import { Key } from "@/util/keyed";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { AgentDispatch } from "../../swarm/swarm-model";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";
import { agentPerfStore, startAgentLayoutShiftObserver } from "../../../agent-model/perf-probe";
import {
    captureTopmostAnchor,
    restoreScrollFromAnchor,
} from "./anchor";
import { DocumentRow } from "./DocumentRow";
import { attachOneWayRecorder, noteOneWay, sampleOneWayFrame } from "../scroll/one-way-recorder";
import { OneWayFlow, scrollTranscriptTo } from "../scroll/one-way-flow";
import { createJumpSettler } from "./jump-settler";
import type { NodeSettler } from "../scroll/settle-on-node";
import { createFollowScroll, type FollowScroll } from "../../../scroll/follow-scroll";
import { createFollowLog, NO_SCROLL_INPUT, type ScrollInput } from "../../../scroll/follow-wiring";
import { estimateNode, estimateNodeForState, previewCapPx } from "./renderers";
import { currentExpansion } from "./expansion-source";
import type { AgentViewState } from "./state";
import {
    dispatchIfRegistered as dispatchLayoutIfRegistered,
    snapshot as snapshotLayout,
    type LayoutView,
    type RowPosition,
} from "@/app/store/agent-pane-layout-store";
import { computeLayoutView } from "@/app/store/agent-pane-layout/reducer";
import { captureDomAnchor, captureOnScreen, restoreOnScreen, type DomAnchor } from "./keep-on-screen";
import { agentViewControls } from "../agent-view-controls";
import { SWITCH_KEYS } from "./verbosity";
import { useSettleOnShow } from "../scroll/settle-on-show";
import { useScrolledOffRelease, type AwayArrivals } from "./held-release";
import { useScrollMemo } from "./scroll-memo";
import { useListZoomAnchor } from "./zoom-anchor-adapter";
import { inFlowState, ROW_GAP_PX, type ExpansionState } from "@/app/store/agent-pane-layout/types";
import { blockAgentName } from "@/app/block/block-agent-name";
import { liftedEarlyLine, logShowCover } from "@/app/block/pane-cover-log";
import { useSearchPaint } from "../search/search-paint";
import {
    initialStickyFrontierId,
    locateFrontierRow,
    nodeBytes,
    partitionAt,
    partitionForVirtualization,
    STREAMING_BUFFER_SIZE,
    TURN_TAIL_MAX_BYTES,
    TURN_TAIL_MAX_DEFERRED_BYTES,
    TURN_TAIL_MAX_DEFERRED_NODES,
    TURN_TAIL_MAX_NODES,
    turnScopedFrontier,
} from "../../../agent-model/streaming-buffer";

/** The three numbers the scroll logic works from, in unzoomed CSS px. */
interface ScrollGeometry {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
}

export interface AgentDocumentVirtualListProps {
    viewState: AgentViewState;
    documentState: Accessor<DocumentState>;
    onLoadOlder?: () => Promise<void>;
    loadingOlder?: Accessor<boolean>;
    /** False when there is no older history to page in (a new session, or
     *  already at the beginning). Pagination is skipped then — capturing its
     *  anchor would turn stick-to-bottom off for nothing. Absent = assume
     *  there may be. */
    hasOlderHistory?: Accessor<boolean>;
    highlightNodeId?: Accessor<string | null>;
    scrollCommand?: Accessor<ScrollCommand | null>;
    /** Wire-up callback so the parent (AgentFooter via AgentDocumentView)
     *  can invoke jumpToBottom on user keystroke. */
    scrollToBottomRef?: (fn: (reason?: string) => void) => void;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    /** Hold a tool expanded after it completes live on screen (ToolBlock calls
     *  this on the active→inactive transition). */
    onHoldNodeOpen?: (id: string) => void;
    /** Rows that finished or arrived while the pane was hidden: held if on screen once it is shown. */
    awayArrivals?: AwayArrivals;
    /** Release a held tool once its row has scrolled off the top (latched
     *  collapse). Invoked by the scroll-off scan in `handleScroll`. */
    onReleaseNodeHold?: (id: string) => void;
    /** Re-run the provider login flow — forwarded to each row so an inline
     *  auth-error node can offer a "Login Again" CTA (SPEC_REAUTH_FROM_AUTH_ERROR §7). */
    onAgentErrorLogin?: () => void;
    /** Open/focus the Agent History tab — forwarded to each row so a
     *  `history_link` synthetic node can act on click. See
     *  SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2. */
    onOpenHistory?: () => void;
    /**
     * Live per-pane zoom factor (the CSS `zoom` on `.agent-view`, agent-view.tsx); default 1. Only
     * the measure ResizeObserver's getBoundingClientRect is zoomed, so it is divided by this to
     * give unzoomed CSS px; scrollTop / clientHeight / offsetTop are already unzoomed and feed the
     * slice raw (SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01).
     */
    zoomFactor?: Accessor<number>;
    /**
     * blockId for the agent-pane-layout slice. When present, the list feeds the slice (nodes,
     * estimates, expansion, viewport, measurements) and the measure RO dispatches `RowMeasured`
     * keyed by the row's expansion state (INV-3). Required for the slice-driven render path.
     */
    blockId?: string;
    /**
     * Derived layout view from the agent-pane-layout slice: rows render from `layoutView().rows`
     * (prefix-sum positions).
     */
    layoutView?: Accessor<LayoutView | null>;
    /** Ordinal-matched tool_use_id -> live dispatch, for this pane's
     *  Agent/Task/Workflow tool nodes. See `activity/dispatch-correlation.ts`. */
    dispatchMatches?: Accessor<Map<string, AgentDispatch>>;
    /**
     * What the always-mounted streaming buffer holds, read once at mount. `"turn"` (default): the
     * turn in flight, bounded by TURN_TAIL_MAX_NODES / TURN_TAIL_MAX_BYTES
     * (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md Phase 3). `"count"`: the last
     * STREAMING_BUFFER_SIZE nodes, the kill switch (`agent:turnscopedtail` = false).
     */
    tailPolicy?: "turn" | "count";
}

/**
 * What the component keeps between its hooks: the elements, the flags and the
 * controllers that used to be `let`s and closures in one function. Fields
 * holding functions are assigned by the hook that owns them; each is read only
 * after that hook has run (at mount or on an event).
 */
interface ListState {
    scrollRef: HTMLDivElement;
    virtualContainerRef: HTMLDivElement | undefined;
    // The streaming buffer's own element, observed by the content-resize pin; set once the buffer
    // first mounts.
    streamingBufferRef: HTMLDivElement | undefined;
    // Guard against concurrent older-history fetches triggered by scroll.
    loadingOlderInFlight: boolean;
    // RAF-coalesced scroll handling (task #39): a drag or wheel gesture fires dozens of native
    // `scroll` events, and each would dispatch a layout `Scrolled` and re-derive the window. One
    // dispatch per animation frame, like the stream path's `scheduleFlush` (stream-flush-queue.ts).
    // handleScrollNow reads live values, so deferring it is safe.
    scrollRafId: number | null;
    // Set when OneWayFlow writes scrollTop (its `wrote` callback), consumed by the next
    // handleScrollNow. The scrollTo() fires a native `scroll` event that lands in the same
    // coalesced batch as any user scroll, so this flag lets the disengage branch tell our own
    // auto-scroll from a user's (the "input-gating race" of the scroll-follow bug class:
    // docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md,
    // docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md §3/§5).
    pendingProgrammaticScroll: boolean;
    // The geometry our own pin produced: scrollHeight and clientHeight read with layout clean (the
    // pin runs in a ResizeObserver callback), scrollTop read right after its scrollTo(). The scroll
    // event that scrollTo() fires is handled on the next frame before layout, so reading geometry
    // there would force a synchronous layout per pinned frame per pane. A batch that is purely our
    // own pin uses this instead; every handleScrollNow clears it
    // (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.1).
    pinnedGeometry: ScrollGeometry | null;
    paneTag: () => string;
    // The `[scroll-follow]` transition log (spec I4).
    followLog: ReturnType<typeof createFollowLog>;
    // User scroll input: the follow controller's (set on mount).
    input: ScrollInput;
    // `follow` alone decides whether the pane follows; stickToBottom is its
    // projection. Created on mount: it needs the scroller.
    follow: FollowScroll | null;
    oneWayFlow: OneWayFlow;
    // Sticky frontier id: set once when the document first crosses STREAMING_BUFFER_SIZE and
    // advanced when the buffer exceeds the cap (createPartition). A plain field, not a signal:
    // mutating it inside the memo must not trigger another run. Cleared when its node is truncated
    // (history reset, re-mount); the next partition run re-anchors to the new tail.
    stickyFrontierId: string | null;
    // Where the frontier was last time: streaming only appends, so this is
    // almost always still right and saves a full findIndex per flush.
    frontierIndexHint: number;
    frontierRow: DocumentNode | undefined; // the row itself, so a re-key keeps it (locateFrontierRow)
    lastScrollMarginPx: number;
    tailHeights: Map<string, { px: number; node: DocumentNode; state: ExpansionState }>;
    tailRowNode: WeakMap<Element, () => DocumentNode>;
    tailRO: ResizeObserver | undefined;
    partition: Accessor<ReturnType<typeof partitionAt>>;
    showCoveredNow: () => boolean;
    releaseScrolledOffHolds: () => void;
    observedGeometry: () => ScrollGeometry;
    syncOverflowState: (geo?: ScrollGeometry) => void;
    dispatchScrollMargin: () => void;
    loadOlderFrom: (scrollTop: number) => void;
    fillOlder: () => void;
    // Set when the list unmounts: a frame still pending from a fill must not ask for another page.
    disposed: boolean;
    // A fill pass gave up with the content still short: the reader is offered "Load earlier messages".
    fillStalled: Accessor<boolean>;
    setFillStalled: Setter<boolean>;
    jumpToBottom: (reason?: string) => void;
    scrollToNode: (nodeId: string, offsetPx?: number) => void;
    /** Lands a jump to a row the list has not measured yet (scroll/settle-on-node.ts); one at a time. */
    settleNode: NodeSettler | null;
}

/** What the render needs from the rows hook. */
interface RowRendering {
    windowedRows: Accessor<RowPosition[]>;
    nodeById: Accessor<Map<string, DocumentNode>>;
    observeRow: (el: HTMLElement, nodeId: string) => void;
    unobserveRow: (el: HTMLElement) => void;
    showCovered: Accessor<boolean>;
}

function createListState(props: AgentDocumentVirtualListProps): ListState {
    const [fillStalled, setFillStalled] = createSignal(false);
    const s = {
        fillStalled,
        setFillStalled,
        disposed: false,
        scrollRef: undefined as unknown as HTMLDivElement,
        virtualContainerRef: undefined,
        streamingBufferRef: undefined,
        loadingOlderInFlight: false,
        scrollRafId: null,
        pendingProgrammaticScroll: false,
        pinnedGeometry: null,
        input: NO_SCROLL_INPUT,
        settleNode: null,
        follow: null,
        stickyFrontierId: null,
        frontierIndexHint: -1,
        frontierRow: undefined,
        lastScrollMarginPx: -1,
        showCoveredNow: () => false,
    } as unknown as ListState;
    s.paneTag = (): string => props.blockId?.slice(0, 7) ?? "?";
    s.followLog = createFollowLog(s.paneTag, props.viewState);
    /** Geometry read in a ResizeObserver callback, where layout is clean. */
    s.observedGeometry = (): ScrollGeometry => ({
        scrollTop: s.scrollRef.scrollTop, // perf:allow-layout-read — ResizeObserver callback (layout clean)
        scrollHeight: s.scrollRef.scrollHeight, // perf:allow-layout-read — ResizeObserver callback (layout clean)
        clientHeight: s.scrollRef.clientHeight, // perf:allow-layout-read — ResizeObserver callback (layout clean)
    });

    // Keeps isOverflowing() current in both directions from live geometry. Runs unconditionally
    // from every observation point (handleScrollNow, both ResizeObservers, OneWayFlow's observer),
    // not only where stickToBottom() is true: a pane can collapse to non-overflowing (a `/clear`,
    // or the whole-pane scrollHeight->0px collapse) while the reader is scrolled away, and a gated
    // call would leave it stale until they re-engage (#2834).
    s.syncOverflowState = (geo?: ScrollGeometry): void => {
        if (!s.scrollRef) return;
        const overflowing = geo ? geo.scrollHeight > geo.clientHeight : s.scrollRef.scrollHeight > s.scrollRef.clientHeight; // perf:allow-layout-read — only without geometry: RO callbacks (layout clean) and the no-RO fallback
        if (overflowing !== props.viewState.isOverflowing()) {
            if (overflowing) {
                props.viewState.markOverflowing();
            } else {
                props.viewState.markNotOverflowing();
            }
        }
    };
    return s;
}

// ── Follow state machine ────────────────────────────────────────────
// The ResizeObservers below report each observation to it; OneWayFlow does the
// pinning.
function useFollowController(props: AgentDocumentVirtualListProps, s: ListState): void {
    onMount(() => {
        if (!s.scrollRef) return;
        s.input = s.follow = createFollowScroll(
            s.scrollRef,
            {
                pin: (cause) => {
                    noteOneWay(s.paneTag(), `pin:${cause}`);
                    // A send or a queued turn starting is a commit point; typing is not.
                    s.oneWayFlow.jumpToBottom(cause === "jump-to-bottom:sent" || cause === "jump-to-bottom:queued-turn");
                },
                loadOlder: (geo) => s.loadOlderFrom(geo.scrollTop), // perf:allow-layout-read — the batch's stored geometry, not a DOM read
                fillOlder: () => s.fillOlder(),
                fillStalled: (stalled) => s.setFillStalled(stalled),
                transition: s.followLog.project,
                hasOlderHistory: () => !!props.onLoadOlder && (props.hasOlderHistory?.() ?? true),
                loadingOlder: () => s.loadingOlderInFlight || !!props.loadingOlder?.(),
                released: () => s.oneWayFlow.recheckSoon(),
            },
        );
        onCleanup(() => {
            s.disposed = true;
            s.follow?.dispose();
        });
    });
    // Older history becoming available (the first screen has painted) or a page landing changes what a short
    // pane can do with no size change to say so: ask again.
    createEffect(() => {
        props.hasOlderHistory?.();
        props.loadingOlder?.();
        untrack(() => s.follow?.checkFill());
    });
}

function useTailRowHeights(props: AgentDocumentVirtualListProps, s: ListState): void {
    // Streaming-buffer row heights, for the height handoff when a node migrates into the
    // virtualized head (slice effect below). Observed, not read: a ResizeObserver callback runs
    // with layout clean. Divided by zoom, like the head's measure RO
    // (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.2).
    //
    // Each height is stored with what it measured: the node object (nodes are immutable, so a
    // content or status change is a new object) and its in-flow expansion state. The handoff uses
    // it only if both still match: a row whose content or expansion changes in the update that
    // migrates it is disposed before the RO reports its new size, and the stale height would stay
    // in the layout for good when the node is above the viewport (#3610).
    s.tailHeights = new Map<string, { px: number; node: DocumentNode; state: ExpansionState }>();
    s.tailRowNode = new WeakMap<Element, () => DocumentNode>();
    s.tailRO = typeof ResizeObserver !== "undefined"
        ? new ResizeObserver((entries) => {
            const zoom = props.zoomFactor?.() ?? 1;
            const docState = untrack(props.documentState);
            for (const entry of entries) {
                const node = untrack(() => s.tailRowNode.get(entry.target)?.());
                if (!node) continue;
                const cssPx = entry.target.getBoundingClientRect().height / (zoom || 1); // perf:allow-layout-read — tail ResizeObserver callback (layout clean)
                if (!Number.isFinite(cssPx) || cssPx <= 0) continue;
                s.tailHeights.set(node.id, { px: cssPx, node, state: inFlowState(currentExpansion(node, docState)) });
            }
        })
        : undefined;
    onCleanup(() => s.tailRO?.disconnect());
}

/**
 * How far the frontier may move right now, from `from` toward `to`: across
 * buffer rows that are wholly above the viewport, so no row the user can
 * see is remounted (a remount is jump-free since 3a, but re-renders
 * content — async markdown like mermaid would flash). Geometry comes from
 * values already known — the head's current size (layoutView), the buffer
 * rows' observed heights (tailHeights), the store's scrollTop — so nothing
 * here reads layout. A row with no observed height yet stops the advance;
 * the next partition run retries. Past twice either ceiling the tail is
 * trimmed regardless, so it stays bounded while the user reads mid-tail;
 * and a deferral longer than TURN_TAIL_MAX_DEFERRED_* moves anyway.
 */
const advanceFrontier = (props: AgentDocumentVirtualListProps, s: ListState, nodes: readonly DocumentNode[], from: number, to: number): number => {
    let tailBytes = 0;
    for (let i = from; i < nodes.length; i++) tailBytes += nodeBytes(nodes[i]);
    if (nodes.length - from > 2 * TURN_TAIL_MAX_NODES || tailBytes > 2 * TURN_TAIL_MAX_BYTES) return to;
    const snap = props.blockId ? snapshotLayout(props.blockId) : null;
    const view = untrack(() => props.layoutView?.());
    if (!snap || !view) return to; // no layout store: nothing measured to protect
    let rowTop = view.scrollMarginPx + view.totalSize;
    let k = from;
    while (k < to) {
        // Only a height observed for this exact node object counts; a stale
        // or missing one stops the advance (retried on the next run).
        const seen = s.tailHeights.get(nodes[k].id);
        if (!seen || seen.node !== nodes[k] || rowTop + seen.px > snap.scrollTop) break; // perf:allow-layout-read — snap is the layout store's state: a stored number, not a DOM read
        rowTop += seen.px + ROW_GAP_PX;
        k++;
    }
    // Deferral is for the few rows the user is looking at. The split is contiguous, so one visible
    // row holds back everything after it; past a small amount, move anyway (the visible rows
    // remount jump-free). Seen live: a 75-node history load kept 76 rows / 44k elements mounted.
    let deferredBytes = 0;
    for (let i = k; i < to; i++) deferredBytes += nodeBytes(nodes[i]);
    if (to - k > TURN_TAIL_MAX_DEFERRED_NODES || deferredBytes > TURN_TAIL_MAX_DEFERRED_BYTES) return to;
    return k;
};

const locateFrontier = (s: ListState, nodes: readonly DocumentNode[]): number =>
    s.stickyFrontierId == null ? -1 : locateFrontierRow(nodes, s.stickyFrontierId, s.frontierIndexHint, s.frontierRow);

/**
 * Turn-scoped partition (Phase 3, §6.2): the tail starts at the turn in
 * flight (turnScopedFrontier). The frontier only moves forward, only
 * inside this memo (invariant 1, single residency), and only as far as
 * advanceFrontier allows. With no frontier yet — first run, or its anchor
 * was cleared/truncated — nothing is mounted to protect, so it goes
 * straight to the target: restoring a long conversation mounts only its
 * last turn plus the visible head rows.
 */
const turnPartition = (props: AgentDocumentVirtualListProps, s: ListState, nodes: readonly DocumentNode[]) => {
    let at = locateFrontier(s, nodes);
    // `from: at`: the frontier never moves back, so the search never looks at the head and
    // per-flush cost follows the tail and the turn in flight, not the conversation (#3611).
    const target = turnScopedFrontier(nodes, { from: Math.max(0, at) });
    if (at < 0) at = target;
    else if (target > at) at = advanceFrontier(props, s, nodes, at, target);
    s.stickyFrontierId = nodes[at]?.id ?? null;
    s.frontierRow = nodes[at];
    s.frontierIndexHint = at;
    const result = partitionAt(nodes, at);
    trail("agent:virt:partition", {
        virtCount: result.virtualizedNodes.length,
        streamCount: result.streamingNodes.length,
        frontier: s.stickyFrontierId?.slice(0, 8) ?? null,
        policy: "turn",
    });
    return result;
};

// Memoized: partition is read by the slice-feeding effect, nodeById, scrollToNode / older-history
// and the streaming buffer <Key>. Without createMemo every read re-slices the document (O(n)) on
// every streamed token (#784).
function createPartition(props: AgentDocumentVirtualListProps, s: ListState) {
    // Tail policy, fixed for this mount: a settings toggle takes effect on the
    // pane's next mount and never re-partitions a live pane.
    const turnScopedTail = untrack(() => (props.tailPolicy ?? "turn") === "turn");
    return createMemo(() => {
        const nodes = props.viewState.nodes();
        if (turnScopedTail) return turnPartition(props, s, nodes);

        // Kill-switch path (`agent:turnscopedtail` off): the last STREAMING_BUFFER_SIZE nodes, as
        // before Phase 3. The frontier locks the first time the document exceeds the buffer; later
        // appends grow `streamingNodes`.
        if (s.stickyFrontierId == null) {
            s.stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
        }

        let result = partitionForVirtualization(
            nodes,
            STREAMING_BUFFER_SIZE,
            s.stickyFrontierId,
        );

        // Stale frontier (anchor node was truncated). Re-anchor and
        // recompute once.
        if (result.splitIndex === -1) {
            s.stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
            result = partitionForVirtualization(
                nodes,
                STREAMING_BUFFER_SIZE,
                s.stickyFrontierId,
            );
        }

        // Cap: the sticky frontier never advances on its own, so across multi-turn sessions the
        // buffer would grow without bound. Advance it to pin streamingNodes at
        // STREAMING_BUFFER_SIZE. This runs inside createMemo before any render effect reads the
        // result, so the array never grows under reconcileArrays, and growth is what crashes it
        // with "replaceChild: node not a child"
        // (SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §7.4; keying: see
        // StreamingBuffer) (#1302, #1317).
        if (result.streamingNodes.length > STREAMING_BUFFER_SIZE) {
            s.stickyFrontierId = initialStickyFrontierId(nodes, STREAMING_BUFFER_SIZE);
            result = partitionForVirtualization(nodes, STREAMING_BUFFER_SIZE, s.stickyFrontierId);
        }

        // Crash-trace: every partition recompute is a candidate trigger for the reconciler's
        // reconcile pass; the render-trail dump from the error boundary shows how many landed just
        // before a throw.
        trail("agent:virt:partition", {
            virtCount: result.virtualizedNodes.length,
            streamCount: result.streamingNodes.length,
            frontier: s.stickyFrontierId?.slice(0, 8) ?? null,
        });
        return result;
    });
}

function useCapPx(): Accessor<number> {
    // Capped rows' estimates follow the window height (renderers.ts previewCapPx). An estimate is
    // pushed once per row, so when the cap changes push them all again, or an unmeasured row keeps
    // the old window's estimate (#3934). Measured heights are unaffected. Debounced: dragging a
    // window edge fires resize every frame.
    const [capPx, setCapPx] = createSignal(Math.round(previewCapPx()));
    onMount(() => {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const onResize = (): void => {
            clearTimeout(timer);
            timer = setTimeout(() => setCapPx(Math.round(previewCapPx())), 150);
        };
        window.addEventListener("resize", onResize);
        onCleanup(() => {
            clearTimeout(timer);
            window.removeEventListener("resize", onResize);
        });
    });
    return capPx;
}

/** What the slice effect has already pushed, so unchanged inputs are skipped. */
interface PushedLayoutInputs {
    // nodeId → JSON(Expansion) last pushed (expansion diff cache).
    pushedExpansion: Map<string, string>;
    // nodeIds with EstimateSet pushed for both states; pruned in lockstep
    // with the slice so a removed-then-re-added node gets fresh values.
    estimatesPushed: Set<string>;
}

function pushLayoutInputs(
    s: ListState,
    blockId: string,
    vnodes: readonly DocumentNode[],
    docState: DocumentState,
    pushed: PushedLayoutInputs,
): void {
    const { pushedExpansion, estimatesPushed } = pushed;
    const ids = vnodes.map((n) => n.id);
    dispatchLayoutIfRegistered(blockId, { type: "NodesChanged", orderedIds: ids });
    const idSet = new Set(ids);
    for (const k of pushedExpansion.keys()) if (!idSet.has(k)) pushedExpansion.delete(k);
    for (const k of estimatesPushed) if (!idSet.has(k)) estimatesPushed.delete(k);
    // Nodes entering the slice this run that the streaming buffer had measured get their real
    // height below (height handoff, INV-3).
    const handoff: DocumentNode[] = [];
    for (const node of vnodes) {
        if (!estimatesPushed.has(node.id)) {
            estimatesPushed.add(node.id);
            if (s.tailHeights.has(node.id)) handoff.push(node);
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
    // Height handoff: a node leaving the streaming buffer for the head gets the height the browser
    // last laid it out at, in the same batch that adds it, so its first head frame is exact and
    // nothing below moves (ROW_GAP_PX matches the buffer's gap). Without it the first frame used an
    // estimate that the head's measure RO corrected a frame later: a visible jump for everything
    // below. Consumed once, so a later head measurement is never overwritten by a stale buffer
    // height.
    for (const node of handoff) {
        const measured = s.tailHeights.get(node.id)!;
        s.tailHeights.delete(node.id);
        const state = inFlowState(currentExpansion(node, docState));
        // Measured something else (older content, other expansion):
        // keep the estimate; the head's measure RO corrects it if
        // the row is ever mounted.
        if (measured.node !== node || measured.state !== state) continue;
        dispatchLayoutIfRegistered(blockId, {
            type: "RowMeasured",
            nodeId: node.id,
            state,
            cssPx: measured.px,
        });
    }
}

// Feed the layout slice from the VIRTUALIZED partition only (the streaming buffer is normal flow,
// out of scope): NodesChanged, per-node EstimateSet (both states, INV-3) and ExpansionResolved, so
// positions and effectiveHeight are authoritative. No-op without blockId.
function useLayoutSliceFeed(props: AgentDocumentVirtualListProps, s: ListState): void {
    if (!props.blockId) return;
    const pushed: PushedLayoutInputs = { pushedExpansion: new Map<string, string>(), estimatesPushed: new Set<string>() };
    const capPx = useCapPx();
    let lastCapPx = capPx();
    // A view change: the topmost row as drawn, read now, before the rows change. A computation runs before the
    // render effects that redraw the rows; the effect below uses it, and it is dropped if that effect does not run.
    let viewDom: DomAnchor | null = null;
    const viewKey = createMemo(() => {
        const view = agentViewControls(props.blockId)?.resolved().view;
        return view ? SWITCH_KEYS.map((k) => view[k]).join("|") : "";
    });
    createComputed(
        on(
            viewKey,
            () => {
                viewDom = untrack(props.viewState.stickToBottom) ? null : captureDomAnchor(s.scrollRef, untrack(() => props.zoomFactor?.()) ?? 1);
                queueMicrotask(() => (viewDom = null));
            },
            { defer: true }
        )
    );
    createEffect(() => {
        const blockId = props.blockId!;
        const vnodes = s.partition().virtualizedNodes;
        const docState = props.documentState();
        const cap = capPx();
        if (cap !== lastCapPx) {
            lastCapPx = cap;
            pushed.estimatesPushed.clear();
        }
        // `dispatchLayoutIfRegistered` synchronously calls the slot's `setLayoutView`, a real Solid
        // signal setter, so each dispatch below (NodesChanged, up to 2×N EstimateSet, up to N
        // ExpansionResolved) could fire an unbatched, different LayoutView and re-render
        // `windowedRows()` several times per update against a half-updated slice. A large batch
        // (every node changing identity at once, e.g. AgentHistoryView's reparse on loadOlder,
        // SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md) crashed `reconcileArrays`
        // ("replaceChild: node not a child"). `batch()` defers the writes, so `windowedRows()` sees
        // one consistent LayoutView per update.
        //
        // Rows removed or inserted above the screen (a roll-off, the history link) don't move the
        // rows on screen, and a view change keeps the topmost row, or the run that now holds it, in
        // place (keep-on-screen.ts).
        const onScreen = captureOnScreen(blockId, untrack(() => props.layoutView?.()));
        const dom = viewDom;
        viewDom = null;
        batch(() => pushLayoutInputs(s, blockId, vnodes, docState, pushed));
        restoreOnScreen({
            blockId,
            rows: onScreen,
            dom,
            latest: () => untrack(() => props.layoutView?.()),
            scroller: s.scrollRef,
            nodes: untrack(props.viewState.nodes),
            zoom: () => props.zoomFactor?.() ?? 1,
            following: props.viewState.stickToBottom,
        });
        // Heights of nodes that left the buffer any other way (removed,
        // cleared) are dropped; the buffer holds a few dozen at most.
        const inBuffer = new Set(s.partition().streamingNodes.map((n) => n.id));
        for (const id of s.tailHeights.keys()) if (!inBuffer.has(id)) s.tailHeights.delete(id);
    });
}

function useViewportFeed(props: AgentDocumentVirtualListProps, s: ListState): void {
    // Feed the slice's VIEWPORT inputs (scrollTop, viewport, scrollMargin, zoom), which the render
    // path reads back for windowing and prefix-sum positions. All blockId-gated and
    // reducer-deduped. Slice positions are unzoomed CSS px; under the ancestor CSS `zoom`,
    // scrollTop / clientHeight / offsetTop are unzoomed too (confirmed over CDP at zoom 0.5 and 2;
    // only getBoundingClientRect is zoomed), so they feed the slice raw and the lone ÷zoom is at
    // the measure RO.
    s.dispatchScrollMargin = (): void => {
        if (!props.blockId) return;
        const px = s.virtualContainerRef?.offsetTop ?? 0; // perf:allow-layout-read — ResizeObserver/MutationObserver callbacks and user scroll only
        if (px === s.lastScrollMarginPx) return; // skip redundant dispatches
        s.lastScrollMarginPx = px;
        dispatchLayoutIfRegistered(props.blockId, { type: "ScrollMarginChanged", px });
    };
    if (props.blockId) {
        // Mirror the live zoom into the slice (INV-2: stored, never relayouts).
        createEffect(() => {
            const blockId = props.blockId!;
            const zoom = props.zoomFactor?.() ?? 1;
            dispatchLayoutIfRegistered(blockId, { type: "ZoomChanged", zoom });
        });
        // scrollMargin = the virtualized container's offsetTop. Nothing renders above the container
        // any more (the auth box moved to AgentAuthPanel, the loading-older banner is gone), so the
        // header watchers are gone; re-read on every region reflow. It must stay in sync because
        // row starts include it while the render path, windowing and scroll-to-node subtract the
        // live offsetTop.
        onMount(() => {
            if (!s.virtualContainerRef) return;
            s.dispatchScrollMargin();
            const ro = new ResizeObserver(() => s.dispatchScrollMargin());
            ro.observe(s.virtualContainerRef);
            onCleanup(() => ro.disconnect());
        });
    }
}

function useAnimateEnabled(props: AgentDocumentVirtualListProps, s: ListState): Accessor<boolean> {
    // Gate for the new-message enter animation; starts false and flips true once history is ready
    // (effect below).
    const [animateEnabled, setAnimateEnabled] = createSignal(false);

    // Flip animateEnabled once history loading is done. `historyReady()` is set by
    // useHistoryPagination after HistoryLoaded/HistoryRestored (or at once for an empty document),
    // when the history rows are already in the DOM.
    //
    // Force a style resolution of those rows BEFORE adding [data-animate]: otherwise
    // @starting-style applies to them on their first resolution, which happens at paint time after
    // every pending microtask, and they all animate. Reading scrollRef.scrollTop flushes style and
    // layout synchronously, so they resolve without [data-animate] and only later mounts (new
    // streaming rows) animate (#1212). For an empty conversation the reflow is a no-op and the
    // first streaming row mounts with [data-animate] already present.
    createEffect(() => {
        if (animateEnabled() || !props.viewState.historyReady()) return;
        void s.scrollRef?.scrollTop; // forced synchronous style/layout flush perf:allow-layout-read — deliberate one-time flush when history becomes ready (see above)
        setAnimateEnabled(true);
    });
    return animateEnabled;
}

function useScrollObservers(props: AgentDocumentVirtualListProps, s: ListState): void {
    // Viewport resize: any clientHeight change to this scroll container, not just hidden → visible. A *sibling*
    // below it resizing (the retry bar, AgentDecisionPanel, AgentQuestionPanel or PendingMessagesPanel appearing
    // mid-turn, per SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md §3.3) shrinks the container by
    // CSS reflow alone: no scroll event fires and no signal changes. This observer feeds the overflow state and the
    // follow controller; OneWayFlow's own observer does the pinning.
    onMount(() => {
        const ro = new ResizeObserver(() => {
            const h = s.scrollRef.clientHeight; // perf:allow-layout-read — clientHeight ResizeObserver callback (layout clean)
            // Unconditional — see syncOverflowState's own doc comment.
            s.syncOverflowState();
            // The follow controller also sees hidden (0 px) and shown here. OneWayFlow pins.
            s.follow?.observed("viewport", s.observedGeometry());
            // The container resizing changes the viewport the slice windows against (hidden→visible
            // 0→N, pane resize); feed it. scrollTop and h are unzoomed CSS px.
            if (props.blockId && h > 0) {
                dispatchLayoutIfRegistered(props.blockId, {
                    type: "Scrolled",
                    scrollTop: s.scrollRef.scrollTop, // perf:allow-layout-read — clientHeight ResizeObserver callback (layout clean)
                    viewportPx: h,
                });
            }
        });
        ro.observe(s.scrollRef);
        onCleanup(() => ro.disconnect());
    });

    // Content resize: the virtualized region and the streaming buffer are the two elements whose box size is how
    // tall the content is. The viewport observer above does not see content growth (the scroll container's own box
    // is fixed by the flex layout), and some growth is outside the Solid signal graph: MarkdownBlock commits a
    // syntax-highlighted re-render from a bare `setTimeout` ~90ms after the last token, and a code block routinely
    // reflows to a different height. This observer feeds the overflow state and the follow controller; OneWayFlow
    // pins, from its own observer. See docs/specs/REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md.
    onMount(() => {
        if (typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver(() => {
            if (!s.scrollRef) return;
            const geo = s.follow ? s.observedGeometry() : undefined;
            // Unconditional — see syncOverflowState's own doc comment.
            s.syncOverflowState(geo);
            // The follow controller sees the content too. OneWayFlow pins, from its own observer.
            if (s.follow && geo) s.follow.observed("content", geo);
            if (geo) props.viewState.noteContentHeight(geo.scrollHeight); // perf:allow-layout-read — the value observedGeometry() read just above, not a second read
        });
        if (s.virtualContainerRef) ro.observe(s.virtualContainerRef);
        if (s.streamingBufferRef) ro.observe(s.streamingBufferRef);
        onCleanup(() => ro.disconnect());
    });
}

function useScrollCommands(props: AgentDocumentVirtualListProps, s: ListState): void {
    const settler = createJumpSettler(props, s);
    // jumpToBottom: forced scroll-to-bottom that re-engages stick.
    // Exposed to parent so AgentFooter can call it on keystroke. `reason`
    // (typing / sent / queued-turn) only feeds the transition log.
    s.jumpToBottom = (reason?: string): void => {
        if (!s.scrollRef) return;
        s.settleNode?.cancel();
        // "accepted" (a queued message the backend picked up, even mid-turn)
        // is not a jump: it only marks a commit point for the one-way path, and
        // only while following, so a reader scrolled up is never pulled down.
        if (reason === "accepted") {
            if (props.viewState.stickToBottom()) s.oneWayFlow.commit();
            return;
        }
        s.follow?.jumpToBottom(reason ?? "unspecified"); // FOLLOWING, then OneWayFlow pins
    };
    if (props.scrollToBottomRef) props.scrollToBottomRef(s.jumpToBottom);

    // scrollToNode: jump the named node into view. Disengages stick
    // (user wants to stay where they jumped). Uses the slice's row
    // position if the target is in the virtualized head, else the
    // streaming buffer's natural DOM offset.
    // With `offsetPx` (scroll-memo.ts, after a reload): the row at that offset, at once; else centered, smoothly.
    s.scrollToNode = (nodeId: string, offsetPx?: number): void => {
        if (!s.scrollRef) return;
        const idx = props.viewState.indexOf(nodeId);
        if (idx < 0) return;
        s.settleNode?.cancel(); // a newer jump replaces the one still landing
        // Detach first, so no pin can pull the pane back while it scrolls there.
        s.follow?.jumpToNode(nodeId);
        const p = s.partition();
        if (idx < p.splitIndex) {
            // Virtualized region: center the row from the slice's position. row.start / height and
            // the scroll element's clientHeight / scrollTop are all unzoomed CSS px under CSS
            // `zoom`, so no conversion.
            const v = props.layoutView?.();
            const row = v?.rows[idx];
            if (row) {
                const viewportPx = s.scrollRef.clientHeight; // perf:allow-layout-read — user-initiated scrollToNode
                const center = row.start - viewportPx / 2 + row.height / 2;
                const top = s.scrollRef.scrollTop; // perf:allow-layout-read — user-initiated scrollToNode
                const onScreen = row.start >= top && row.start + row.height <= top + viewportPx;
                if (offsetPx != null) scrollTranscriptTo(s.scrollRef, Math.max(0, row.start + offsetPx));
                else if (onScreen) scrollTranscriptTo(s.scrollRef, Math.max(0, center), "smooth");
                else settler.start(nodeId);
            }
        } else {
            // Streaming buffer — node is mounted; query DOM and scroll directly.
            const el = s.scrollRef.querySelector(`[data-node-id="${nodeId}"]`) as HTMLElement | null;
            if (!el) return;
            const elTop = el.offsetTop; // perf:allow-layout-read — user-initiated scrollToNode
            const center = elTop - s.scrollRef.clientHeight / 2 + el.clientHeight / 2; // perf:allow-layout-read — user-initiated scrollToNode
            const top = s.scrollRef.scrollTop; // perf:allow-layout-read — user-initiated scrollToNode
            const onScreen = elTop >= top && elTop + el.clientHeight <= top + s.scrollRef.clientHeight; // perf:allow-layout-read — user-initiated scrollToNode
            if (offsetPx != null) scrollTranscriptTo(s.scrollRef, Math.max(0, elTop + offsetPx));
            else if (onScreen) scrollTranscriptTo(s.scrollRef, Math.max(0, center), "smooth");
            // Off screen: the rows above are estimates until drawn, so where the tail sits moves as they are
            // measured; one aim from now would land short (clamped at the bottom), so it settles instead.
            else settler.start(nodeId);
        }
    };

    // Jump commands from the parent's useScrollToNode, untracked: scrollToNode reads layoutView /
    // partition, which change on every scroll, and the last command is held, so tracking them
    // would re-fire the jump on each scroll and pin the user on the old target.
    createEffect(() => {
        const cmd = props.scrollCommand?.();
        if (cmd) untrack(() => s.scrollToNode(cmd.nodeId));
    });
}

function useScrolledOffHolds(props: AgentDocumentVirtualListProps, s: ListState): void {
    // Collapse held-open tools once they scroll off the top while the reader
    // watches: never while hidden, nor on the catch-up drawn on return (`held-release.ts`).
    s.releaseScrolledOffHolds = useScrolledOffRelease({
        scroller: () => s.scrollRef,
        documentState: props.documentState,
        onRelease: (id) => props.onReleaseNodeHold?.(id),
        covered: () => s.showCoveredNow(),
        away: props.awayArrivals,
        onHold: (id) => props.onHoldNodeOpen?.(id),
    });
}

function createHandleScrollNow(props: AgentDocumentVirtualListProps, s: ListState): () => void {
    const handleScrollNow = (): void => {
        if (!s.scrollRef) return;

        // Consume the programmatic-scroll flag for THIS batch (OneWayFlow's `wrote` callback), read
        // once up front: every branch that might disengage needs to know whether our own
        // auto-scroll caused part of the batch.
        const wasProgrammatic = s.pendingProgrammaticScroll;
        s.pendingProgrammaticScroll = false;
        // A pointer held on the content counts too: a text-selection drag past
        // the pane's edge autoscrolls it, and that scroll is the user's (it must
        // be able to release follow).
        const hadUserInput = s.input.windowOpen() || s.input.pointerHeld();

        // A batch that is purely our own pin reuses the geometry the pin read
        // after layout (see pinnedGeometry) — no layout read here. Anything the
        // user may have caused reads live geometry, exactly as before.
        const trustPin = wasProgrammatic && !hadUserInput && s.pinnedGeometry !== null;
        const geo: ScrollGeometry = trustPin ? s.pinnedGeometry! : {
            scrollTop: s.scrollRef.scrollTop, // perf:allow-layout-read — user scroll batch only; our own pin reuses pinnedGeometry
            scrollHeight: s.scrollRef.scrollHeight, // perf:allow-layout-read — user scroll batch only; our own pin reuses pinnedGeometry
            clientHeight: s.scrollRef.clientHeight, // perf:allow-layout-read — user scroll batch only; our own pin reuses pinnedGeometry
        };
        s.pinnedGeometry = null;
        const { scrollTop, clientHeight } = geo;

        // scrollTop / clientHeight are already unzoomed CSS px under the ancestor CSS `zoom`, so
        // push them raw (the lone ÷zoom is at the measure RO). Reducer-deduped; blockId-gated.
        if (props.blockId) {
            dispatchLayoutIfRegistered(props.blockId, {
                type: "Scrolled",
                scrollTop,
                viewportPx: clientHeight,
            });
            // Reads offsetTop (a layout read). Our own pin cannot move the header above the rows
            // and the container's ResizeObserver tracks every other way the margin changes, so this
            // is the safety net for user scrolling; a trusted pin batch skips it.
            if (!trustPin) s.dispatchScrollMargin();
        }

        // The follow controller makes every follow decision for this batch
        // (detach, re-attach, paging).
        if (!s.follow) return;
        const wasFollowing = s.follow.following();
        s.follow.scrolled(geo, hadUserInput);
        // Re-baseline the rows. Not after a re-attach: its pin did it.
        if (hadUserInput && (wasFollowing || !s.follow.following())) {
            s.oneWayFlow.userScrolled(geo);
        }
        if (props.viewState.stickToBottom() && !trustPin) s.releaseScrolledOffHolds();
        s.syncOverflowState(geo);
    };
    return handleScrollNow;
}

function defineFillOlder(props: AgentDocumentVirtualListProps, s: ListState): void {
    /**
     * Page in older history for a pane too short to scroll. No anchor: the reader hasn't scrolled, and the
     * pane keeps following the bottom as the page lands above the rows (capturing an anchor would detach it).
     */
    s.fillOlder = (): void => {
        s.loadingOlderInFlight = true;
        const landed = (): void => {
            // Two frames: the page's rows are laid out and observed in the first, and the next check reads
            // that geometry, so a short pane takes its pages one at a time, not all at once.
            requestAnimationFrame(() =>
                requestAnimationFrame(() => {
                    s.loadingOlderInFlight = false;
                    if (!s.disposed) s.follow?.checkFill();
                })
            );
        };
        let page: Promise<void> | undefined;
        try {
            page = props.onLoadOlder?.();
        } catch {
            // The flag is cleared below all the same; the pass's page count bounds the retries.
        }
        (page ?? Promise.resolve()).then(landed, landed);
    };
}

function defineLoadOlder(props: AgentDocumentVirtualListProps, s: ListState): void {
    /** Capture the topmost visible row as an anchor, page in older history, restore the anchor. */
    s.loadOlderFrom = (scrollTop: number): void => {
        // Anchor on the topmost visible node: from the slice's window when the document is large
        // enough to virtualize (row.start already includes scrollMargin), else, when it fits
        // entirely in the streaming buffer and the window is empty, the buffer's first node via
        // DOM. The latter is the common initial-pagination case (#784).
        const v = props.layoutView?.();
        const topRow = v && v.window.endIndex >= v.window.startIndex
            ? v.rows[v.window.startIndex]
            : undefined;
        let anchorId: string | null = null;
        let anchorOffsetPx = 0;
        if (topRow) {
            anchorId = topRow.nodeId;
            anchorOffsetPx = topRow.start;
        } else if (s.partition().streamingNodes.length > 0) {
            anchorId = s.partition().streamingNodes[0].id;
            const el = s.scrollRef.querySelector(
                `[data-node-id="${anchorId}"]`,
            ) as HTMLElement | null;
            anchorOffsetPx = el?.offsetTop ?? 0; // perf:allow-layout-read — user scrolled near the top: older-history anchor capture
        }
        if (anchorId != null) {
            const anchor = captureTopmostAnchor(
                [{ id: anchorId, offsetPx: anchorOffsetPx }],
                scrollTop,
            );
            if (anchor) {
                s.followLog.transitionFollow("load-older", `scrollTop=${Math.round(scrollTop)}`, () => props.viewState.captureHeadAnchor(anchor));
            }
        }

        s.loadingOlderInFlight = true;
        props.onLoadOlder().then(() => {
            requestAnimationFrame(() => {
                // Restore from the anchor by id: the prepended nodes are in partition, so its new
                // index gives a fresh measured offset. Use the same `partition()` memo the DOM
                // renders from: the raw `partitionForVirtualization()` is a count-based split that
                // ignores the sticky frontier, so an anchor actually in the streaming buffer could
                // read a slice row past the virtualized region and the restore would silently fail
                // (#1101).
                const anchor = props.viewState.headAnchor();
                if (anchor != null && s.scrollRef) {
                    const newIdx = props.viewState.indexOf(anchor.nodeId);
                    if (newIdx >= 0) {
                        const newP = s.partition();
                        if (newIdx < newP.splitIndex) {
                            // Still virtualized: read the new offset from a FRESH slice snapshot
                            // (the prepend shifted every position); synchronous, so no
                            // projection-timing dependency. start includes scrollMargin, like the
                            // captured anchorOffsetPx.
                            const snap = props.blockId ? snapshotLayout(props.blockId) : null;
                            const offset = snap ? computeLayoutView(snap).rows[newIdx]?.start : undefined;
                            if (offset != null) {
                                const target = restoreScrollFromAnchor(anchor, offset);
                                scrollTranscriptTo(s.scrollRef, target);
                            }
                        } else {
                            // Now in streaming buffer — query DOM.
                            const el = s.scrollRef.querySelector(
                                `[data-node-id="${anchor.nodeId}"]`,
                            ) as HTMLElement | null;
                            if (el) {
                                const target = restoreScrollFromAnchor(anchor, el.offsetTop); // perf:allow-layout-read — older-history restore, one rAF after the page loaded
                                scrollTranscriptTo(s.scrollRef, target);
                            }
                        }
                    }
                }
                s.loadingOlderInFlight = false;
            });
        }).catch(() => {
            s.loadingOlderInFlight = false;
        });
    };
}

function useScrollHandling(props: AgentDocumentVirtualListProps, s: ListState): () => void {
    const handleScrollNow = createHandleScrollNow(props, s);
    defineLoadOlder(props, s);
    defineFillOlder(props, s);

    // One-way-flow recorder (opt-in, `__agentmuxOneWay.enable()`): checks every
    // painted frame for content moving down or blank space taken back while
    // following. Phase 0 of SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md.
    onMount(() => {
        if (s.scrollRef) onCleanup(attachOneWayRecorder(s.paneTag(), s.scrollRef));
    });

    // Coalesce native `scroll` events to at most one `handleScrollNow` call
    // per animation frame. `onScroll` below wires this, not `handleScrollNow`
    // directly.
    const handleScroll = (): void => {
        if (s.scrollRafId != null) return;
        s.scrollRafId = requestAnimationFrame(() => {
            s.scrollRafId = null;
            handleScrollNow();
        });
    };
    onCleanup(() => {
        if (s.scrollRafId != null) {
            cancelAnimationFrame(s.scrollRafId);
            s.scrollRafId = null;
        }
    });
    return handleScroll;
}

function createMeasureObserver(
    props: AgentDocumentVirtualListProps,
    nodeById: Accessor<Map<string, DocumentNode>>,
    elNodeId: WeakMap<Element, string>,
): ResizeObserver | undefined {
    // Every height is read first, then all of them go in one RowsMeasured: a RowMeasured after each
    // read re-laid out the rows, so the next read forced a layout, and each dispatch rebuilt the
    // whole prefix sum, per row, every frame of a window drag
    // (ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md §13).
    const measureRO = typeof ResizeObserver !== "undefined"
        ? new ResizeObserver((entries) => {
            const blockId = props.blockId;
            if (!blockId) return;
            const zoom = props.zoomFactor?.() ?? 1;
            const snap = snapshotLayout(blockId);
            const nodes = nodeById();
            const rows: { nodeId: string; state: ExpansionState; cssPx: number }[] = [];
            for (const entry of entries) {
                const nodeId = elNodeId.get(entry.target);
                if (!nodeId) continue;
                const cssPx = entry.target.getBoundingClientRect().height / (zoom || 1); // perf:allow-layout-read — measure ResizeObserver callback (layout clean)
                rows.push({ nodeId, state: inFlowState(snap?.expansion.get(nodeId)), cssPx });
            }
            if (rows.length === 0) return;
            for (const { nodeId, cssPx } of rows) {
                const node = nodes.get(nodeId);
                if (node) {
                    agentPerfStore.recordEstimatorMeasurement(
                        node.type,
                        estimateNode(node, props.documentState()),
                        cssPx,
                    );
                }
            }
            dispatchLayoutIfRegistered(blockId, { type: "RowsMeasured", rows });
        })
        : undefined;
    return measureRO;
}

/** The one-way path: pins the rows to the bottom while following; returns whether the show-cover is up. */
function useOneWayFlow(props: AgentDocumentVirtualListProps, s: ListState): Accessor<boolean> {
    const reducedMotion = () => shouldReduceMotion("transition");
    // The one-way path's single observer (SPEC_AGENT_PANE_ONE_WAY_FLOW R4). Created after measureRO
    // and tailRO on purpose: in a round where rows resized, its callback runs after theirs have
    // updated the slice, so the geometry it reads already includes them.
    s.oneWayFlow = new OneWayFlow({
        following: () => untrack(props.viewState.stickToBottom),
        userActive: () => s.input.pointerHeld() || s.input.windowOpen(),
        reducedMotion,
        wrote: (geo) => {
            s.pendingProgrammaticScroll = true;
            s.pinnedGeometry = geo;
            s.follow?.noteGeometry(geo);
        },
        // Collapsing held-open tools that scrolled off the top changes layout, so it
        // runs before the observation is measured and is compensated in the same frame.
        prepare: () => s.releaseScrolledOffHolds(),
        observed: (geo) => {
            s.syncOverflowState(geo);
            sampleOneWayFrame(s.paneTag());
        },
        note: (label) => noteOneWay(s.paneTag(), label),
    });
    onCleanup(() => s.oneWayFlow.dispose());
    const agentName = blockAgentName(props.blockId);
    const showCovered = useSettleOnShow(s.oneWayFlow, props.viewState.stickToBottom, {
        changes: props.viewState.nodes,
        observe: () => [s.virtualContainerRef, s.streamingBufferRef],
        onLift: (why, frames, ms) => {
            if (why !== "still" && props.blockId) console.info("[pane-cover]", liftedEarlyLine(props.blockId, why, frames, ms, agentName()));
        },
    });
    s.showCoveredNow = () => untrack(showCovered);
    logShowCover(props.blockId, showCovered, agentName);
    onMount(() => {
        if (s.scrollRef) {
            s.oneWayFlow.attach(s.scrollRef, [s.virtualContainerRef, s.streamingBufferRef]);
        }
    });
    // `/clear` or a fresh session: nothing to compare against, and the
    // pane follows the new conversation.
    createEffect(() => {
        if (props.viewState.nodes().length !== 0) return;
        s.oneWayFlow.reset();
        s.follow?.reset();
    });
    return showCovered;
}

function useRowRendering(props: AgentDocumentVirtualListProps, s: ListState): RowRendering {
    // Rows render from the slice's prefix-sum positions (the layoutView signal), keyed by stable
    // nodeId so the fresh RowPosition objects of each recompute don't remount rows. Heights come
    // from one ResizeObserver dispatching RowMeasured keyed by (nodeId, state).

    // The visible rows, sliced from the slice's full positions array. Empty
    // window (endIndex < startIndex) → render nothing.
    const windowedRows = createMemo<RowPosition[]>(() => {
        const v = props.layoutView?.();
        if (!v || v.window.endIndex < v.window.startIndex) return [];
        return v.rows.slice(v.window.startIndex, v.window.endIndex + 1);
    });

    // nodeId → live DocumentNode for the virtualized partition, so a row renders the CURRENT node
    // (streaming updates propagate without remount). A windowed row with no node this frame is
    // skipped: a recoverable drop.
    const nodeById = createMemo(() => {
        const m = new Map<string, DocumentNode>();
        for (const n of s.partition().virtualizedNodes) m.set(n.id, n);
        return m;
    });

    // One measure RO for all virtualized rows; el→nodeId lets the callback
    // dispatch RowMeasured keyed by the row's current in-flow state, ÷zoom to
    // stay in unzoomed CSS px (INV-2/3). Rows observe on mount, unobserve on
    // unmount (the Key child's onCleanup).
    const elNodeId = new WeakMap<Element, string>();
    const measureRO = createMeasureObserver(props, nodeById, elNodeId);
    onCleanup(() => measureRO?.disconnect());
    const showCovered = useOneWayFlow(props, s);

    const observeRow = (el: HTMLElement, nodeId: string): void => {
        elNodeId.set(el, nodeId);
        // Border box: the height dispatched is the rect's, so a padding or
        // border change must re-measure too.
        measureRO?.observe(el, { box: "border-box" });
        s.oneWayFlow.observeRow(el);
    };
    const unobserveRow = (el: HTMLElement): void => {
        measureRO?.unobserve(el);
        s.oneWayFlow.unobserveRow(el);
        elNodeId.delete(el);
    };
    return { windowedRows, nodeById, observeRow, unobserveRow, showCovered };
}

// Virtualized head, present only when the document exceeds the buffer. <Key by={r => r.nodeId}>
// ties each DOM slot to one nodeId for its lifetime; the position-keyed <Index> it replaced let a
// slot's nodeId go stale when the window shifted, so measureRO dispatched RowMeasured for the wrong
// node (#1319, #1326).
function VirtualizedHead(list: { props: AgentDocumentVirtualListProps; s: ListState; rows: RowRendering }): JSX.Element {
    const { props, s, rows } = list;
    return (
        <div
            ref={(el) => { s.virtualContainerRef = el; }}
            class="agent-document-virtualizer"
            style={{
                height: `${props.layoutView?.()?.totalSize ?? 0}px`,
                position: "relative",
                width: "100%",
            }}
        >
            <Key each={rows.windowedRows()} by={(r) => r.nodeId}>
                {(row) => {
                    let rowEl: HTMLElement | undefined;
                    // The live node at this row's id; skip a frame if it's
                    // momentarily absent from the partition (recoverable).
                    const node = (): DocumentNode | undefined => rows.nodeById().get(row().nodeId);
                    onCleanup(() => { if (rowEl) rows.unobserveRow(rowEl); });
                    return (
                        <Show when={node()}>
                            {(n) => (
                                <DocumentRow
                                    node={n}
                                    documentState={props.documentState}
                                    highlightNodeId={props.highlightNodeId}
                                    onToggleCollapse={props.onToggleCollapse}
                                    onTogglePin={props.onTogglePin}
                                    onHoldNodeOpen={props.onHoldNodeOpen}
                                    onAgentErrorLogin={props.onAgentErrorLogin}
                                    onOpenHistory={props.onOpenHistory}
                                    dispatchMatches={props.dispatchMatches}
                                    ref={(el) => { rowEl = el; rows.observeRow(el, row().nodeId); }}
                                    style={{
                                        position: "absolute",
                                        top: "0",
                                        left: "0",
                                        width: "100%",
                                        // start includes the stored scroll margin; subtract that
                                        // stored value, not a live offsetTop (which forced layout
                                        // for every row as it rendered).
                                        transform: `translateY(${row().start - (props.layoutView?.()?.scrollMarginPx ?? 0)}px)`,
                                    }}
                                />
                            )}
                        </Show>
                    );
                }}
            </Key>
        </div>
    );
}

// Streaming buffer: always-mounted trailing nodes, keyed by <Key by={n => n.id}>:
// - <For> keys by item reference, and each streamed token replaces the active node's object (same
//   id, new ref), so it would remount the row per token (#784).
// - <Index> is position-keyed: ToolBlock's slot state (prevStatus, expanded) leaks to whatever node
//   lands at that position after a cap-advance migration. It also crashes if the array grows
//   (reconcileArrays sentinel corruption): growth is the trigger, not the keying (#1326).
// - <Key> gives each slot a stable id: a token update fires the slot's accessor without a remount,
//   and a cap-advance disposes the departing slot cleanly. The partition memo caps streamingNodes
//   at STREAMING_BUFFER_SIZE before any render effect sees it, so the array never grows (#1302,
//   #1317, #1326).
//
// <Show when={partition()}> guards the secondary crash: a memo read after its scope is disposed
// (mid-error-boundary teardown) gives undefined, and "Cannot read properties of undefined (reading
// 'streamingNodes')". The narrowed `p()` puts <Key> in its own child scope, disposed before the
// outer memo can produce a stale read (SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md
// §3.3).
function StreamingBuffer(list: { props: AgentDocumentVirtualListProps; s: ListState; animateEnabled: Accessor<boolean> }): JSX.Element {
    const { props, s, animateEnabled } = list;
    return (
        <Show when={s.partition()}>
            {(p) => (
                <div
                    class="agent-document-streaming-buffer"
                    data-animate={animateEnabled() || undefined}
                    ref={(el) => { s.streamingBufferRef = el; }}
                >
                    <Key each={p().streamingNodes as DocumentNode[]} by={(n) => n.id}>
                        {(nodeAccessor) => {
                            let rowEl: HTMLElement | undefined;
                            // Unobserve only: the cached height must outlive the row, since a
                            // migrating node's buffer row is disposed before the slice effect hands
                            // its height to the head.
                            onCleanup(() => {
                                if (rowEl) {
                                    s.tailRO?.unobserve(rowEl);
                                    s.oneWayFlow.unobserveRow(rowEl);
                                }
                            });
                            return (
                                <DocumentRow
                                    node={nodeAccessor}
                                    documentState={props.documentState}
                                    highlightNodeId={props.highlightNodeId}
                                    onToggleCollapse={props.onToggleCollapse}
                                    onTogglePin={props.onTogglePin}
                                    onHoldNodeOpen={props.onHoldNodeOpen}
                                    onAgentErrorLogin={props.onAgentErrorLogin}
                                    onOpenHistory={props.onOpenHistory}
                                    dispatchMatches={props.dispatchMatches}
                                    ref={(el) => {
                                        rowEl = el;
                                        s.tailRowNode.set(el, nodeAccessor);
                                        s.tailRO?.observe(el, { box: "border-box" });
                                        s.oneWayFlow.observeRow(el);
                                    }}
                                />
                            );
                        }}
                    </Key>
                </div>
            )}
        </Show>
    );
}

export function AgentDocumentVirtualList(props: AgentDocumentVirtualListProps): JSX.Element {
    const s = createListState(props);
    useFollowController(props, s);
    useTailRowHeights(props, s);
    s.partition = createPartition(props, s);
    useLayoutSliceFeed(props, s);
    useViewportFeed(props, s);
    // Positions, windowing and measurement come from the agent-pane-layout slice (rendered by
    // VirtualizedHead from layoutView and the measure RO). Heights are keyed by (nodeId, state), so
    // a row that scrolls out and back keeps its measured height, and prefix-sum positions make
    // overlap impossible (INV-1).

    // Layout-shift observer scoped to .agent-document: idempotent, and a no-op in production.
    onMount(() => {
        startAgentLayoutShiftObserver();
    });
    const animateEnabled = useAnimateEnabled(props, s);
    useScrollObservers(props, s);
    useScrollCommands(props, s);
    useScrollMemo({ blockId: props.blockId, scroller: () => s.scrollRef, following: props.viewState.stickToBottom, zoom: props.zoomFactor, historyReady: props.viewState.historyReady, scrollToRow: s.scrollToNode });
    useListZoomAnchor(props, () => s.scrollRef, () => s.follow);
    useScrolledOffHolds(props, s);
    const handleScroll = useScrollHandling(props, s);
    const rows = useRowRendering(props, s);
    const searchPaint = useSearchPaint();

    // Nothing renders above the rows, so a loading banner cannot push them down when a scroll-up
    // page loads. The "Load earlier messages" button is a zero-height sticky box and takes no room.
    return (
        <div
            class="agent-document"
            ref={(el) => { s.scrollRef = el; }}
            onScroll={handleScroll}
            data-follow-state={props.viewState.stickToBottom() ? "following" : "detached"}
            data-show-cover={rows.showCovered() || undefined}
            data-search-dim={searchPaint?.dim() || undefined}
        >
            <Show when={s.fillStalled() && props.viewState.stickToBottom()}>
                <div class="agent-load-earlier">
                    <Button
                        tone="accent"
                        density="compact"
                        onClick={() => {
                            s.follow?.restartFill();
                            s.fillOlder();
                        }}
                    >
                        Load earlier messages
                    </Button>
                </div>
            </Show>
            <VirtualizedHead props={props} s={s} rows={rows} />
            <StreamingBuffer props={props} s={s} animateEnabled={animateEnabled} />
        </div>
    );
}

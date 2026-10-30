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
import { shouldReduceMotion } from "@/app/motion/motion";
import { Key } from "@/util/keyed";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { AgentDispatch } from "../../swarm/swarm-model";
import type { DocumentNode, DocumentState } from "../types";
import { agentPerfStore, startAgentLayoutShiftObserver } from "./perf-probe";
import {
    captureTopmostAnchor,
    isNearTop,
    restoreScrollFromAnchor,
} from "./anchor";
import { DocumentRow } from "./DocumentRow";
import { attachOneWayRecorder, noteOneWay, sampleOneWayFrame } from "../scroll/one-way-recorder";
import { OneWayFlow } from "../scroll/one-way-flow";
import { REATTACH_PX } from "../scroll/follow-controller";
import { createFollowScroll, type FollowScroll } from "../scroll/follow-scroll";
import { attachLegacyScrollInput, createFollowLog, NO_SCROLL_INPUT, type ScrollInput } from "../scroll/follow-wiring";
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
import { captureOnScreen, restoreOnScreen } from "./keep-on-screen";
import { useSettleOnShow } from "../scroll/settle-on-show";
import { useScrolledOffRelease, type AwayArrivals } from "./held-release";
import { useScrollMemo } from "./scroll-memo";
import { useListZoomAnchor } from "./zoom-anchor-adapter";
import { inFlowState, ROW_GAP_PX, type ExpansionState } from "@/app/store/agent-pane-layout/types";
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
} from "./streaming-buffer";

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
    /** Ordinal-matched tool_use_id -> live dispatch, for this pane's
     *  Agent/Task/Workflow tool nodes. See `activity/dispatch-correlation.ts`. */
    dispatchMatches?: Accessor<Map<string, AgentDispatch>>;
    /**
     * What the always-mounted streaming buffer holds. `"turn"` (default): the
     * turn in flight, bounded by TURN_TAIL_MAX_NODES / TURN_TAIL_MAX_BYTES —
     * Phase 3 of SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md.
     * `"count"`: the last STREAMING_BUFFER_SIZE nodes, the pre-Phase-3
     * behaviour and the kill switch (`agent:turnscopedtail` = false). Read
     * once at mount.
     */
    tailPolicy?: "turn" | "count";
    /**
     * The follow state machine (`agent:followcontroller`): one reducer decides
     * whether the pane follows, and only a user gesture can stop it. False keeps
     * the older engage/disengage code in `handleScrollNow`. Read once at mount.
     * SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §8 Phase 2.
     */
    followController?: boolean;
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
    // Same one-dispatch-per-animation-frame idea the stream path applies to
    // chunks (stream-flush-queue.ts's `scheduleFlush`), done here with a
    // nullable rAF handle as the guard.
    // `handleScrollNow` reads scrollRef's LIVE values (not anything captured
    // from the event), so it's safe to defer to the next frame: whichever
    // scroll position is current when the frame runs is the one applied.
    let scrollRafId: number | null = null;

    // Set whenever OneWayFlow writes scrollTop (its `wrote` callback below),
    // consumed by the very next handleScrollNow() call. A `scrollTo()` call itself
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
    // The geometry our own pin read and produced — scrollHeight and
    // clientHeight read with layout already clean (the pin runs from a
    // ResizeObserver callback), scrollTop read right after its scrollTo().
    // The scroll event that scrollTo() fires is handled by handleScrollNow on
    // the NEXT animation frame, before that frame's layout: reading geometry
    // there would force a synchronous layout, once per pinned frame per pane.
    // When the batch is purely our own pin (no user scroll input since), it
    // uses this instead. Cleared by every handleScrollNow. Phase 1 of
    // SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.1.
    let pinnedGeometry: ScrollGeometry | null = null;
    const paneTag = (): string => props.blockId?.slice(0, 7) ?? "?";
    // The `[scroll-follow]` transition log (spec I4).
    const followLog = createFollowLog(paneTag, props.viewState);
    const transitionFollow = followLog.transitionFollow;
    // User scroll input: the follow controller's, or the legacy listeners (set on mount).
    let input: ScrollInput = NO_SCROLL_INPUT;

    // ── Follow state machine (props.followController) ───────────────────
    // When on, `follow` alone decides whether the pane follows; stickToBottom
    // is its projection (written only in `transition` below), so every
    // existing reader keeps working. The pin writers stay where they are: the
    // ResizeObservers below (or OneWayFlow) report each observation to it and
    // pin when it says so. Created on mount: it needs the scroller.
    const useFollowController = untrack(() => props.followController === true);
    let follow: FollowScroll | null = null;
    onMount(() => {
        if (!scrollRef) return;
        if (!useFollowController) {
            const legacy = attachLegacyScrollInput(scrollRef, { released: () => oneWayFlow.recheckSoon() });
            input = legacy;
            onCleanup(() => legacy.dispose());
            return;
        }
        input = follow = createFollowScroll(
            scrollRef,
            {
                pin: (cause) => {
                    noteOneWay(paneTag(), `pin:${cause}`);
                    // A send or a queued turn starting is a commit point; typing is not.
                    oneWayFlow.jumpToBottom(cause === "jump-to-bottom:sent" || cause === "jump-to-bottom:queued-turn");
                },
                loadOlder: (geo) => loadOlderFrom(geo.scrollTop), // perf:allow-layout-read — the batch's stored geometry, not a DOM read
                transition: followLog.project,
                hasOlderHistory: () => !!props.onLoadOlder && (props.hasOlderHistory?.() ?? true),
                loadingOlder: () => loadingOlderInFlight || !!props.loadingOlder?.(),
                released: () => oneWayFlow.recheckSoon(),
            },
        );
        onCleanup(() => follow?.dispose());
    });
    /** Geometry read in a ResizeObserver callback, where layout is clean. */
    const observedGeometry = (): ScrollGeometry => ({
        scrollTop: scrollRef.scrollTop, // perf:allow-layout-read — ResizeObserver callback (layout clean)
        scrollHeight: scrollRef.scrollHeight, // perf:allow-layout-read — ResizeObserver callback (layout clean)
        clientHeight: scrollRef.clientHeight, // perf:allow-layout-read — ResizeObserver callback (layout clean)
    });
    // Keeps isOverflowing() current in both directions from live geometry.
    // MUST run unconditionally from every observation point (handleScrollNow,
    // both ResizeObservers, OneWayFlow's observer) — NOT only from paths gated
    // on stickToBottom() already being true. A pane can collapse to
    // non-overflowing (or grow past it) while the user is scrolled away
    // reading history — a `/clear` wiping the transcript, or the documented
    // whole-pane scrollHeight->0px collapse, mid-history-read — and a
    // stickToBottom()-gated call site would otherwise never observe it,
    // leaving isOverflowing() stale until the user manually re-engages
    // (reagent P1 on PR #2834).
    function syncOverflowState(geo?: ScrollGeometry): void {
        if (!scrollRef) return;
        const overflowing = geo ? geo.scrollHeight > geo.clientHeight : scrollRef.scrollHeight > scrollRef.clientHeight; // perf:allow-layout-read — only without geometry: RO callbacks (layout clean) and the no-RO fallback
        if (overflowing !== props.viewState.isOverflowing()) {
            if (overflowing) {
                props.viewState.markOverflowing();
            } else {
                props.viewState.markNotOverflowing();
            }
        }
    }

    const reducedMotion = () => shouldReduceMotion("transition");

    // Streaming-buffer row heights, for the height handoff when a node
    // migrates into the virtualized head (slice effect below). Observed, not
    // read: a ResizeObserver callback runs with layout already clean, so
    // reading the rect there forces nothing. ÷zoom to stay in unzoomed CSS px,
    // exactly like the head's measure RO. Phase 3 of
    // SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.2.
    //
    // Each height is stored with WHAT it measured: the node object the row
    // was rendering (nodes are immutable, so a content or status change is a
    // new object) and its in-flow expansion state. The handoff uses it only if
    // both still match — a row whose content or expansion changes in the same
    // update that migrates it is disposed before the RO can report its new
    // size, and that stale height would stay in the layout for good if the
    // node is above the viewport (never mounted in the head, never
    // re-measured). Codex P2 on #3610.
    const tailHeights = new Map<string, { px: number; node: DocumentNode; state: ExpansionState }>();
    const tailRowNode = new WeakMap<Element, () => DocumentNode>();
    const tailRO = typeof ResizeObserver !== "undefined"
        ? new ResizeObserver((entries) => {
            const zoom = props.zoomFactor?.() ?? 1;
            const docState = untrack(props.documentState);
            for (const entry of entries) {
                const node = untrack(() => tailRowNode.get(entry.target)?.());
                if (!node) continue;
                const cssPx = entry.target.getBoundingClientRect().height / (zoom || 1); // perf:allow-layout-read — tail ResizeObserver callback (layout clean)
                if (!Number.isFinite(cssPx) || cssPx <= 0) continue;
                tailHeights.set(node.id, { px: cssPx, node, state: inFlowState(currentExpansion(node, docState)) });
            }
        })
        : undefined;
    onCleanup(() => tailRO?.disconnect());

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
    // Tail policy, fixed for this mount: a settings toggle takes effect on the
    // pane's next mount and never re-partitions a live pane.
    const turnScopedTail = untrack(() => (props.tailPolicy ?? "turn") === "turn");

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
    const advanceFrontier = (nodes: readonly DocumentNode[], from: number, to: number): number => {
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
            const seen = tailHeights.get(nodes[k].id);
            if (!seen || seen.node !== nodes[k] || rowTop + seen.px > snap.scrollTop) break; // perf:allow-layout-read — snap is the layout store's state: a stored number, not a DOM read
            rowTop += seen.px + ROW_GAP_PX;
            k++;
        }
        // Deferral is for the few rows the user is looking at. The split is
        // contiguous, so a visible row holds back everything after it; past a
        // small amount, move anyway (the visible rows are remounted, jump-free
        // since 3a). Otherwise a big batch arriving while the reader is
        // scrolled up stays mounted behind one visible row — seen live: a
        // 75-node history load kept 76 rows / 44k elements mounted.
        let deferredBytes = 0;
        for (let i = k; i < to; i++) deferredBytes += nodeBytes(nodes[i]);
        if (to - k > TURN_TAIL_MAX_DEFERRED_NODES || deferredBytes > TURN_TAIL_MAX_DEFERRED_BYTES) return to;
        return k;
    };

    /**
     * Turn-scoped partition (Phase 3, §6.2): the tail starts at the turn in
     * flight (turnScopedFrontier). The frontier only moves forward, only
     * inside this memo (invariant 1, single residency), and only as far as
     * advanceFrontier allows. With no frontier yet — first run, or its anchor
     * was cleared/truncated — nothing is mounted to protect, so it goes
     * straight to the target: restoring a long conversation mounts only its
     * last turn plus the visible head rows.
     */
    // Where the frontier was last time: streaming only appends, so this is
    // almost always still right and saves a full findIndex per flush.
    let frontierIndexHint = -1;
    let frontierRow: DocumentNode | undefined; // the row itself, so a re-key keeps it (locateFrontierRow)
    const locateFrontier = (nodes: readonly DocumentNode[]): number =>
        stickyFrontierId == null ? -1 : locateFrontierRow(nodes, stickyFrontierId, frontierIndexHint, frontierRow);
    const turnPartition = (nodes: readonly DocumentNode[]) => {
        let at = locateFrontier(nodes);
        // `from: at` — the frontier never moves back, so the target search
        // never looks at the head: per-flush cost follows the tail and the
        // turn in flight, not the conversation (Codex P2, #3611).
        const target = turnScopedFrontier(nodes, { from: Math.max(0, at) });
        if (at < 0) at = target;
        else if (target > at) at = advanceFrontier(nodes, at, target);
        stickyFrontierId = nodes[at]?.id ?? null;
        frontierRow = nodes[at];
        frontierIndexHint = at;
        const result = partitionAt(nodes, at);
        trail("agent:virt:partition", {
            virtCount: result.virtualizedNodes.length,
            streamCount: result.streamingNodes.length,
            frontier: stickyFrontierId?.slice(0, 8) ?? null,
            policy: "turn",
        });
        return result;
    };

    const partition = createMemo(() => {
        const nodes = props.viewState.nodes();
        if (turnScopedTail) return turnPartition(nodes);

        // Kill-switch path (`agent:turnscopedtail` off): the last
        // STREAMING_BUFFER_SIZE nodes, as before Phase 3.
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
        // Capped rows' estimates follow the window height (renderers.ts
        // previewCapPx). An estimate is pushed once per row, so when the cap
        // changes (window resize), push them all again: an unmeasured row
        // would otherwise keep the old window's estimate (Codex P2 on #3934).
        // Measured heights are kept separately and are not affected.
        // Debounced: dragging a window edge fires resize every frame.
        const [capPx, setCapPx] = createSignal(Math.round(previewCapPx()));
        let lastCapPx = capPx();
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
        createEffect(() => {
            const blockId = props.blockId!;
            const vnodes = partition().virtualizedNodes;
            const docState = props.documentState();
            const cap = capPx();
            if (cap !== lastCapPx) {
                lastCapPx = cap;
                estimatesPushed.clear();
            }
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
            // incremental change (live streaming) nothing visibly breaks, but a
            // LARGE batch (every node changing identity at once, e.g.
            // AgentHistoryView's reparse-on-loadOlder,
            // SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md)
            // crashed `reconcileArrays` ("replaceChild: node not a child": a
            // stale intermediate row referencing a DOM node a later pass had
            // removed). `batch()` defers every signal write inside it, so
            // `windowedRows()` sees ONE consistent LayoutView per update.
            // Rows removed or inserted above the screen (a roll-off, the
            // history link) don't move the rows on screen (keep-on-screen.ts).
            const onScreen = captureOnScreen(blockId, untrack(() => props.layoutView?.()));
            batch(() => {
                dispatchLayoutIfRegistered(blockId, { type: "NodesChanged", orderedIds: ids });
                const idSet = new Set(ids);
                for (const k of pushedExpansion.keys()) if (!idSet.has(k)) pushedExpansion.delete(k);
                for (const k of estimatesPushed) if (!idSet.has(k)) estimatesPushed.delete(k);
                // Nodes entering the slice this run that the streaming buffer
                // had measured: handed their real height below (Phase 3 height
                // handoff, invariant 3).
                const handoff: DocumentNode[] = [];
                for (const node of vnodes) {
                    if (!estimatesPushed.has(node.id)) {
                        estimatesPushed.add(node.id);
                        if (tailHeights.has(node.id)) handoff.push(node);
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
                // Height handoff: a node leaving the streaming buffer for the
                // head gets the height the browser last laid it out at, in the
                // SAME batch that adds it — so its first head frame is exact
                // and nothing below it moves (with ROW_GAP_PX matching the
                // buffer's gap, the head occupies exactly the space the buffer
                // did). Without this the first frame used an estimate and the
                // head's measure RO corrected it a frame later: a visible jump
                // for everything below. Consumed once: a later head
                // measurement is never overwritten by a stale buffer height.
                for (const node of handoff) {
                    const measured = tailHeights.get(node.id)!;
                    tailHeights.delete(node.id);
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
            });
            restoreOnScreen(blockId, onScreen, untrack(() => props.layoutView?.()), scrollRef);
            // Heights of nodes that left the buffer any other way (removed,
            // cleared) are dropped; the buffer holds a few dozen at most.
            const inBuffer = new Set(partition().streamingNodes.map((n) => n.id));
            for (const id of tailHeights.keys()) if (!inBuffer.has(id)) tailHeights.delete(id);
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
        const px = virtualContainerRef?.offsetTop ?? 0; // perf:allow-layout-read — ResizeObserver/MutationObserver callbacks and user scroll only
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
        // scrollMargin = the virtualized container's offsetTop. Nothing renders
        // above the container any more (the auth box moved to AgentAuthPanel,
        // the loading-older banner was removed), so the header watchers that
        // tracked it are gone; re-read on every region reflow. Keeping
        // scrollMarginPx in sync matters because row starts include it while
        // the render path, windowing, and scroll-to-node subtract the live
        // offsetTop.
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
        void scrollRef?.scrollTop; // forced synchronous style/layout flush perf:allow-layout-read — deliberate one-time flush when history becomes ready (see above)
        setAnimateEnabled(true);
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
    // deps (nodes().length, layoutView().totalSize) change
    // either, so `stickToBottom` stays silently true while the view falls
    // short of the new, smaller max scroll position. Rather than adding a
    // tracked height signal per interposing panel (the whack-a-mole pattern
    // the 07-24 fix already had to extend once), just re-pin on every resize
    // this observer reports — idempotent when already at true bottom, and
    // still gated on `stickToBottom()` so a user who deliberately scrolled
    // away is never fought.
    onMount(() => {
        const ro = new ResizeObserver(() => {
            const h = scrollRef.clientHeight; // perf:allow-layout-read — clientHeight ResizeObserver callback (layout clean)
            // Unconditional — see syncOverflowState's own doc comment.
            syncOverflowState();
            // The follow controller also sees hidden (0 px) and shown here. OneWayFlow pins.
            follow?.observed("viewport", observedGeometry());
            // Phase 3: the scroll container resizing changes the viewport the
            // slice windows against — feed it (covers hidden→visible 0→N and
            // pane resize). blockId-gated, reducer-deduped. scrollTop / h are
            // unzoomed CSS px (Phase 4 — only the measure RO ÷zooms).
            if (props.blockId && h > 0) {
                dispatchLayoutIfRegistered(props.blockId, {
                    type: "Scrolled",
                    scrollTop: scrollRef.scrollTop, // perf:allow-layout-read — clientHeight ResizeObserver callback (layout clean)
                    viewportPx: h,
                });
            }
        });
        ro.observe(scrollRef);
        onCleanup(() => ro.disconnect());
    });

    // Content-resize-driven pin. The effect above only re-pins when one of
    // two hand-picked signals changes (nodes().length,
    // layoutView().totalSize) — any OTHER source of the content growing taller is
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
    onMount(() => {
        if (typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver(() => {
            if (!scrollRef) return;
            const geo = follow ? observedGeometry() : undefined;
            // Unconditional — see syncOverflowState's own doc comment.
            syncOverflowState(geo);
            // The follow controller sees the content too. OneWayFlow pins, from its own observer.
            if (follow && geo) follow.observed("content", geo);
        });
        if (virtualContainerRef) ro.observe(virtualContainerRef);
        if (streamingBufferRef) ro.observe(streamingBufferRef);
        onCleanup(() => ro.disconnect());
    });

    // jumpToBottom: forced scroll-to-bottom that re-engages stick.
    // Exposed to parent so AgentFooter can call it on keystroke. `reason`
    // (typing / sent / queued-turn) only feeds the transition log.
    const jumpToBottom = (reason?: string): void => {
        if (!scrollRef) return;
        // "accepted" (a queued message the backend picked up, even mid-turn)
        // is not a jump: it only marks a commit point for the one-way path, and
        // only while following, so a reader scrolled up is never pulled down.
        if (reason === "accepted") {
            if (props.viewState.stickToBottom()) oneWayFlow.commit();
            return;
        }
        if (follow) {
            follow.jumpToBottom(reason ?? "unspecified"); // FOLLOWING, then its pin effect
            return;
        }
        transitionFollow(`jump-to-bottom:${reason ?? "unspecified"}`, undefined, () => props.viewState.engageStickToBottom());
        noteOneWay(paneTag(), `pin:jump:${reason ?? "unspecified"}`);
        // A send or a queued turn starting is a commit point; each keystroke
        // while typing is not.
        oneWayFlow.jumpToBottom(reason === "sent" || reason === "queued-turn");
    };
    if (props.scrollToBottomRef) props.scrollToBottomRef(jumpToBottom);

    // scrollToNode: jump the named node into view. Disengages stick
    // (user wants to stay where they jumped). Uses the slice's row
    // position if the target is in the virtualized head, else the
    // streaming buffer's natural DOM offset.
    // With `offsetPx` (scroll-memo.ts, after a reload): the row at that offset, at once; else centered, smoothly.
    const scrollToNode = (nodeId: string, offsetPx?: number): void => {
        if (!scrollRef) return;
        const idx = props.viewState.indexOf(nodeId);
        if (idx < 0) return;
        // Detach first, so no pin can pull the pane back while it scrolls there.
        follow?.jumpToNode(nodeId);
        const p = partition();
        if (idx < p.splitIndex) {
            // Virtualized region — center the row from the slice's position
            // (Step 5). row.start/height and the scroll element's clientHeight /
            // scrollTop are all unzoomed CSS px (Phase 4 — CSS `zoom` leaves them
            // in layout px), so no zoom conversion is needed here.
            const v = props.layoutView?.();
            const row = v?.rows[idx];
            if (row) {
                const viewportPx = scrollRef.clientHeight; // perf:allow-layout-read — user-initiated scrollToNode
                const center = row.start - viewportPx / 2 + row.height / 2;
                scrollRef.scrollTo(offsetPx == null ? { top: Math.max(0, center), behavior: "smooth" } : { top: Math.max(0, row.start + offsetPx) });
            }
        } else {
            // Streaming buffer — node is mounted; query DOM and scroll directly.
            const el = scrollRef.querySelector(`[data-node-id="${nodeId}"]`) as HTMLElement | null;
            if (!el) return;
            const elTop = el.offsetTop; // perf:allow-layout-read — user-initiated scrollToNode
            const center = elTop - scrollRef.clientHeight / 2 + el.clientHeight / 2; // perf:allow-layout-read — user-initiated scrollToNode
            scrollRef.scrollTo(offsetPx == null ? { top: Math.max(0, center), behavior: "smooth" } : { top: Math.max(0, elTop + offsetPx) });
        }
        if (!follow) transitionFollow("jump-to-node", `node=${nodeId.slice(0, 8)}`, () => props.viewState.disengageStickToBottom());
    };

    // Jump commands from the parent's useScrollToNode, untracked: scrollToNode reads layoutView /
    // partition, which change on every scroll, and the last command is held, so tracking them
    // would re-fire the jump on each scroll and pin the user on the old target.
    createEffect(() => {
        const cmd = props.scrollCommand?.();
        if (cmd) untrack(() => scrollToNode(cmd.nodeId));
    });
    useScrollMemo({ blockId: props.blockId, scroller: () => scrollRef, following: props.viewState.stickToBottom, zoom: props.zoomFactor, historyReady: props.viewState.historyReady, scrollToRow: scrollToNode });
    useListZoomAnchor(props, () => scrollRef, () => follow);

    // Collapse held-open tools once they scroll off the top while the reader
    // watches: never while hidden, nor on the catch-up drawn on return (`held-release.ts`).
    let showCoveredNow: () => boolean = () => false;
    const releaseScrolledOffHolds = useScrolledOffRelease({
        scroller: () => scrollRef,
        documentState: props.documentState,
        onRelease: (id) => props.onReleaseNodeHold?.(id),
        covered: () => showCoveredNow(),
        away: props.awayArrivals,
        onHold: (id) => props.onHoldNodeOpen?.(id),
    });

    const handleScrollNow = (): void => {
        if (!scrollRef) return;

        // Consume the programmatic-scroll flag for THIS batch — see
        // OneWayFlow's `wrote` callback. Read once, up front: every
        // branch below that might disengage needs to know whether this
        // scroll event batch was (at least partly) caused by our own
        // auto-scroll, not just the disengage branch specifically.
        const wasProgrammatic = pendingProgrammaticScroll;
        pendingProgrammaticScroll = false;
        // A pointer held on the content counts too: a text-selection drag past
        // the pane's edge autoscrolls it, and that scroll is the user's (it must
        // be able to release follow).
        const hadUserInput = input.windowOpen() || input.pointerHeld();

        // A batch that is purely our own pin reuses the geometry the pin read
        // after layout (see pinnedGeometry) — no layout read here. Anything the
        // user may have caused reads live geometry, exactly as before.
        const trustPin = wasProgrammatic && !hadUserInput && pinnedGeometry !== null;
        const geo: ScrollGeometry = trustPin ? pinnedGeometry! : {
            scrollTop: scrollRef.scrollTop, // perf:allow-layout-read — user scroll batch only; our own pin reuses pinnedGeometry
            scrollHeight: scrollRef.scrollHeight, // perf:allow-layout-read — user scroll batch only; our own pin reuses pinnedGeometry
            clientHeight: scrollRef.clientHeight, // perf:allow-layout-read — user scroll batch only; our own pin reuses pinnedGeometry
        };
        pinnedGeometry = null;
        const { scrollTop, scrollHeight, clientHeight } = geo;

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
            // Reads offsetTop (a layout read). Our own pin cannot move the
            // header above the rows, and the container/header ResizeObservers
            // and the childList MutationObserver above already track every way
            // the margin changes — this call is the safety net for user
            // scrolling, so a trusted pin batch skips it.
            if (!trustPin) dispatchScrollMargin();
        }

        // Follow controller: the reducer makes every follow decision for this
        // batch (detach, re-attach, paging); nothing below runs.
        if (follow) {
            const wasFollowing = follow.following();
            follow.scrolled(geo, hadUserInput);
            // Re-baseline the rows. Not after a re-attach: its pin did it.
            if (hadUserInput && (wasFollowing || !follow.following())) {
                oneWayFlow.userScrolled(geo);
            }
            if (props.viewState.stickToBottom() && !trustPin) releaseScrolledOffHolds();
            syncOverflowState(geo);
            return;
        }

        // A user scroll re-baselines the rows it keeps in place; moving the
        // transcript up while following releases follow at once, however small
        // the move.
        if (hadUserInput && !trustPin && oneWayFlow.userScrolled(geo) && props.viewState.stickToBottom()) {
            transitionFollow(
                input.scrollbarHeld() ? "user-scroll:scrollbar" : "user-scroll-up",
                `gap=${Math.round(scrollHeight - clientHeight - scrollTop)}px`,
                () => props.viewState.disengageStickToBottom(),
            );
        }

        // Collapse held-open tools that have scrolled off the top (latched).
        // Gated on stick-to-bottom: collapsing a row above the fold shrinks
        // layout height above scrollTop, which is invisible while pinned to the
        // bottom (no jump) — the primary live-streaming case. The scrolled-up /
        // anchor-compensated case is a documented Phase 2 follow-up.
        // A trusted pin batch skips it: the pin pass that caused this scroll
        // already ran it with layout clean (content ResizeObserver).
        if (props.viewState.stickToBottom() && !trustPin) {
            releaseScrolledOffHolds();
        }

        syncOverflowState(geo);

        // Engage stick when the user scrolls back to the true bottom (24 px, spec 09-24 §5.5): a reader
        // 100 px up is reading, not following. Engaging clears any captured headAnchor (atomic).
        const nearBottom = scrollHeight - clientHeight - scrollTop <= REATTACH_PX;

        if (nearBottom) {
            if (!props.viewState.stickToBottom()) {
                transitionFollow(
                    hadUserInput ? "user-scroll-to-bottom" : "reached-bottom",
                    `gap=${Math.round(scrollHeight - clientHeight - scrollTop)}px`,
                    () => props.viewState.engageStickToBottom(),
                );
            }
        }
        // Moving down, or not at the bottom, never releases follow here: only an upward user scroll does
        // (OneWayFlow.userScrolled, above). A user scrolling DOWN while the follower is still catching up
        // is helping it, not leaving.

        // Older-history pagination — capture anchor, fetch, restore.
        //
        // Only from a USER scroll (inside the user-input window; a held
        // scrollbar drag keeps it open), and never when there is nothing
        // older to load. Capturing the anchor turns stick-to-bottom off, so
        // any other trigger silently stops the follow: our own pin (a new
        // session's first overflow is a line or two, so the pin lands at
        // scrollTop < NEAR_TOP_THRESHOLD_PX — the pane then loaded nothing and
        // stopped following until the user scrolled down by hand, spec §2.1),
        // or a browser-made scroll near the top (a clamp or anchoring
        // adjustment after the range collapses and regrows — /clear, the
        // whole-pane collapse — same class as §2.2 B2; ReAgent P1 on #3652).
        if (
            props.onLoadOlder &&
            hadUserInput &&
            (props.hasOlderHistory?.() ?? true) &&
            isNearTop(scrollTop) &&
            !loadingOlderInFlight &&
            !(props.loadingOlder?.())
        ) {
            loadOlderFrom(scrollTop);
        }
    };

    /** Capture the topmost visible row as an anchor, page in older history, restore the anchor. */
    const loadOlderFrom = (scrollTop: number): void => {
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
            anchorOffsetPx = el?.offsetTop ?? 0; // perf:allow-layout-read — user scrolled near the top: older-history anchor capture
        }
        if (anchorId != null) {
            const anchor = captureTopmostAnchor(
                [{ id: anchorId, offsetPx: anchorOffsetPx }],
                scrollTop,
            );
            if (anchor) {
                transitionFollow("load-older", `scrollTop=${Math.round(scrollTop)}`, () => props.viewState.captureHeadAnchor(anchor));
            }
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
                                const target = restoreScrollFromAnchor(anchor, el.offsetTop); // perf:allow-layout-read — older-history restore, one rAF after the page loaded
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
    };

    // One-way-flow recorder (opt-in, `__agentmuxOneWay.enable()`): checks every
    // painted frame for content moving down or blank space taken back while
    // following. Phase 0 of SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md.
    onMount(() => {
        if (scrollRef) onCleanup(attachOneWayRecorder(paneTag(), scrollRef));
    });

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
    // Every height is read first, then all of them go in one RowsMeasured.
    // Dispatching a RowMeasured after each read re-laid out the rows, so the
    // next row's read forced a layout, and each dispatch rebuilt the whole
    // prefix sum: per row that rewrapped, every frame of a window drag
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
    onCleanup(() => measureRO?.disconnect());
    // The one-way path's single observer (SPEC_AGENT_PANE_ONE_WAY_FLOW R4). Created after measureRO
    // and tailRO on purpose: in a round where rows resized, its callback runs after theirs have
    // updated the slice, so the geometry it reads already includes them.
    const oneWayFlow = new OneWayFlow({
        following: () => untrack(props.viewState.stickToBottom),
        userActive: () => input.pointerHeld() || input.windowOpen(),
        reducedMotion,
        wrote: (geo) => {
            pendingProgrammaticScroll = true;
            pinnedGeometry = geo;
            follow?.noteGeometry(geo);
        },
        // Collapsing held-open tools that scrolled off the top changes layout, so it
        // runs before the observation is measured and is compensated in the same frame.
        prepare: () => releaseScrolledOffHolds(),
        observed: (geo) => {
            syncOverflowState(geo);
            sampleOneWayFrame(paneTag());
        },
        note: (label) => noteOneWay(paneTag(), label),
    });
    onCleanup(() => oneWayFlow.dispose());
    const showCovered = useSettleOnShow(oneWayFlow, props.viewState.stickToBottom, { changes: props.viewState.nodes, observe: () => [virtualContainerRef, streamingBufferRef] });
    showCoveredNow = () => untrack(showCovered);
    onMount(() => {
        if (scrollRef) {
            oneWayFlow.attach(scrollRef, [virtualContainerRef, streamingBufferRef]);
        }
    });
    // `/clear` or a fresh session: nothing to compare against, and the
    // pane follows the new conversation.
    createEffect(() => {
        if (props.viewState.nodes().length !== 0) return;
        oneWayFlow.reset();
        follow?.reset();
    });

    const observeRow = (el: HTMLElement, nodeId: string): void => {
        elNodeId.set(el, nodeId);
        // Border box: the height dispatched is the rect's, so a padding or
        // border change must re-measure too.
        measureRO?.observe(el, { box: "border-box" });
        oneWayFlow.observeRow(el);
    };
    const unobserveRow = (el: HTMLElement): void => {
        measureRO?.unobserve(el);
        oneWayFlow.unobserveRow(el);
        elNodeId.delete(el);
    };

    // Render: scroll container holds the virtualized head and the streaming buffer. Nothing renders
    // above the rows: the "Loading older messages" banner that used to sit here pushed the rows down
    // by its height every time a scroll-up page loaded.
    return (
        <div
            class="agent-document"
            ref={scrollRef}
            onScroll={handleScroll}
            data-follow-state={props.viewState.stickToBottom() ? "following" : "detached"}
            data-show-cover={showCovered() || undefined}
            data-follow-controller={useFollowController || undefined}
        >
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
                                        onHoldNodeOpen={props.onHoldNodeOpen}
                                        onAgentErrorLogin={props.onAgentErrorLogin}
                                        onOpenHistory={props.onOpenHistory}
                                        dispatchMatches={props.dispatchMatches}
                                        ref={(el) => { rowEl = el; observeRow(el, row().nodeId); }}
                                        style={{
                                            position: "absolute",
                                            top: "0",
                                            left: "0",
                                            width: "100%",
                                            // start includes the stored scroll margin; subtracting
                                            // that same stored value (not a live offsetTop read,
                                            // which forced layout for every row as it rendered)
                                            // places the row exactly within the region.
                                            transform: `translateY(${row().start - (props.layoutView?.()?.scrollMarginPx ?? 0)}px)`,
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
                                let rowEl: HTMLElement | undefined;
                                // Unobserve only — the cached height must
                                // outlive the row: a migrating node's buffer
                                // row is disposed before the slice effect hands
                                // its height to the head.
                                onCleanup(() => {
                                    if (rowEl) {
                                        tailRO?.unobserve(rowEl);
                                        oneWayFlow.unobserveRow(rowEl);
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
                                            tailRowNode.set(el, nodeAccessor);
                                            tailRO?.observe(el, { box: "border-box" });
                                            oneWayFlow.observeRow(el);
                                        }}
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

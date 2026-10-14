// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolOverlayLog — log body of the tool overlay (Phase 3 of
 * SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4).
 *
 * Renders `ToolNode.log.chunks` if the streaming runner has populated
 * them. Falls back to the per-tool rich result renderer
 * (tool-renderers/, e.g. BashOutputViewer, DiffViewer) when no chunks are present — preserves today's UX
 * for tools that don't stream (yet) or that have already terminated
 * before Phase 2's backend wraps the runner.
 *
 * No ANSI parsing in this PR (Phase 3): chunks render as plain text.
 * ANSI parsing lands in Phase γ (perf + worker offload) per the spec.
 */

import { Match, Show, Switch, createComputed, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
// `Show` retained for fallback ToolOverlayResult sub-tree.
import type { ToolNode } from "../types";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { beginHeightContinuity, cancelHeightContinuity } from "../resize-contract";
import { ChunkPreview } from "./ChunkPreview";
import { dropBashwrapStartingChunk } from "./output-cap";
import { startsAtTop } from "../tool-meta/tool-descriptors";
import { createOverflowReporter } from "./overflow-report";
import { previewBox } from "./scroll-handoff";
import { createPreviewFollow } from "../scroll/preview-follow";
import { followControllerOn } from "../scroll/follow-setting";
import { renderCompactDefault } from "./tool-renderers/builtins";
import { registerToolRenderers } from "./tool-renderers";
import { resolveToolRenderer, type ToolRenderContext } from "./tool-renderers/registry";

// Every result renderer, from one explicit list (tool-renderers/index.ts).
registerToolRenderers();

interface ToolOverlayLogProps {
    node: ToolNode;
    /** Ordinal-matched live dispatch for an Agent/Task/Workflow tool call —
     *  see `activity/dispatch-correlation.ts`. Undefined when no confident
     *  match was found, or for any other tool kind. */
    dispatchMatch?: AgentDispatch;
    /** Whether this log can be seen now: its panel is open and its pane isn't
     *  dormant (ToolBlock's `bodyShowing`). While it can't, the log keeps
     *  showing what it showed: see `branch`. Undefined counts as open. */
    open?: boolean;
    /** Whether the pane is dormant (hidden). A dormant log whose tool finishes
     *  lets go of the chunk view it was holding: see `branch`. */
    dormant?: boolean;
    /** Told whether the preview scrolls vertically, once its size has settled (overflow-report.ts). */
    onOverflowChange?: (overflows: boolean) => void;
}

export const ToolOverlayLog = (props: ToolOverlayLogProps): JSX.Element => {
    let scrollRef: HTMLDivElement | undefined;

    // INLINE prop access pattern (matches MarkdownBlock lines 18-23) —
    // wrapping `props.node.log?.chunks` in createMemo broke reactivity
    // for in-place ToolNode updates (ToolChunkAppend only mutates log,
    // not the array length, and the memo's tracked dependencies did
    // not fire on those updates — verified via diag in PR #884/#885/#886
    // where the reducer appended 58+ chunks but the memo evaluated
    // exactly once per overlay mount with log=undefined). Solid's
    // JSX-expression auto-wrapping IS reactive end-to-end through
    // multi-layer prop chains; createMemo's manual tracking is not.
    // Read every gate inline at JSX time so each repaint sees the
    // latest log state.
    /**
     * Phase 3 fallback rules — refined after codex P1 on PR #803.
     *
     * - While the tool is still streaming (`log.open === true`):
     *   show the live chunk feed exclusively. This is the user's
     *   primary "what's happening" surface.
     * - Once the tool terminates (`log.open === false`): if a
     *   structured `result` is present (BashResult with exit code,
     *   EditResult diff, ReadResult content), show the rich
     *   `ToolOverlayResult`. Structured viewers carry information
     *   the raw chunk feed can't (exit code, diff syntax highlight,
     *   per-language code highlight).
     * - If terminated without a structured result, keep the chunks
     *   visible so the user can still see what happened.
     * - If neither chunks nor result is present (running tool that
     *   hasn't emitted yet, or a non-streaming tool with no result
     *   yet), defer to `ToolOverlayResult` which renders the
     *   "⏳ Running..." placeholder.
     *
     * The codex-reported bug was a naive `chunks.length > 0` gate
     * that permanently suppressed the structured result viewer for
     * every tool that streamed any output — exit codes, diffs, and
     * highlighted Read content were silently dropped post-completion.
     */
    const isStreaming = () => props.node.log?.open === true;
    // Filtered the same way `ChunkList` filters below — NOT the raw
    // `log.chunks.length`. A Bash tool's very first chunk is always
    // bashwrap's own `[bashwrap] starting: N chars` marker (see
    // output-cap.ts's `dropBashwrapStartingChunk`), which `ChunkList`
    // already hides from render. Before this fix, `hasChunks()` counted
    // that hidden chunk, so `isStreaming() && hasChunks()` matched and
    // routed to `ChunkList` — which then rendered nothing (the one chunk
    // it had was filtered out), instead of falling through to the
    // `!hasChunks() && !hasResult()` branch's "⏳ Running…" placeholder a
    // few lines down. The net effect: the tool panel auto-expanded (see
    // `rowDisclosure` in `virtualization/disclosure.ts`, gated on status
    // alone) straight
    // into a visibly blank body for however long real output took to
    // arrive, between the Working row's "Thinking…" and the first real
    // chunk — the exact gap user reports still exist after
    // dropBashwrapStartingChunk shipped (which only fixed what ChunkList
    // rendered once it WAS the active branch, not which branch got
    // chosen). Counting the post-filter length here means an all-
    // bashwrap-marker chunk list now correctly reads as "no visible
    // content yet" and shows the placeholder instead of an empty box.
    const hasChunks = () => dropBashwrapStartingChunk(props.node.log?.chunks ?? []).length > 0;
    const hasResult = () => props.node.result != null;
    const chunks = () => props.node.log?.chunks ?? [];

    // What the log shows, and so which `<Switch>` branch below renders (also
    // what the height FLIP further down watches for a change):
    // - "chunks": the streamed output, while the tool runs and after it when
    //   no structured result came. One branch whether or not the stream is
    //   still open: both render the same ChunkList, and as two branches the
    //   stream closing rebuilt the whole list (and measured a FLIP between
    //   two equal heights);
    // - "result": the structured result; "empty": nothing yet (the
    //   placeholder), both drawn by ToolOverlayResult;
    // - "none": the stream is open but shows nothing yet while a result
    //   already exists, which renders nothing.
    //
    // While it can't be seen (`props.open === false`: the panel is closed, or
    // the pane is dormant) it keeps showing what it showed. A finished tool's
    // panel closes as its result arrives, and its body is dropped a second
    // later (body-mount.ts): building the result view then, every line of its
    // output, was work for a panel nobody saw, and so was everything a hidden
    // pane built. With a dozen agents streaming, each line of output was
    // built three times (SPEC_AGENT_PANE_STREAM_FRAME_BUDGET_2026_10_13.md
    // §2.2, §2.3). Shown again, it shows the current output.
    //
    // A memo is safe here: the reducer replaces a node's `log` and chunk array
    // on every change (the in-place mutation that once made memos miss
    // updates, PR #884/#885/#886, is gone).
    type LogBranch = "chunks" | "result" | "empty" | "none";
    const currentBranch = (): LogBranch => {
        if (hasChunks() && (isStreaming() || !hasResult())) return "chunks";
        if (!isStreaming() && hasResult()) return "result";
        if (!hasChunks() && !hasResult()) return "empty";
        return "none";
    };
    // - A log that mounts while it can't be seen shows nothing until it can
    //   (a restored tool in an already-hidden tab, a row mounted hidden).
    // - A dormant log whose tool finished lets go of the chunk view it held:
    //   the reducer freed the node's chunks, and holding the stale array (and
    //   its lines) until the tab is shown would defeat that. It builds the
    //   result when shown. A closing panel (not dormant) keeps the view for
    //   its collapse animation.
    const branch = createMemo<LogBranch | undefined>((prev) => {
        if (props.open === false) {
            if (prev === undefined) return "none";
            if (prev === "chunks" && props.dormant && !isStreaming() && hasResult()) return "none";
            return prev;
        }
        return currentBranch();
    }) as () => LogBranch;
    // The node the branches draw from, held with the branch while the panel is
    // closed: the reducer frees a tool's chunks once its result arrives (a
    // closing panel still showing the chunk view must not empty out
    // mid-collapse), and a tool with no chunks would otherwise build its
    // result inside ToolOverlayResult as its status changes. Held for at most
    // the body's linger (body-mount.ts), then unmounted.
    const shownNode = createMemo<ToolNode | undefined>((prev) =>
        props.open === false && prev !== undefined && branch() !== "none" ? prev : props.node
    ) as () => ToolNode;
    const shownChunks = (): ReadonlyArray<LogChunk> => shownNode().log?.chunks ?? [];
    // The dispatch card's live data is held with the node: a correlated
    // dispatch update must not rebuild a result while its pane is dormant.
    const shownDispatch = createMemo<AgentDispatch | undefined>((prev) =>
        props.open === false && branch() !== "none" ? prev : props.dispatchMatch
    );

    // Scroll hand-off to the outer pane at this box's edges, with the skid, as
    // every preview box has (scroll-handoff.ts `previewBox`; this box carries
    // `overscroll-behavior: contain` in _tool-overlay-portal.scss).
    const handoff = previewBox();
    onMount(() => {
        if (scrollRef) handoff(scrollRef);
    });

    // Track whether the overlay panel is collapsed (content-visibility: hidden).
    // Accessing layout-forcing properties (scrollHeight, scrollTop) on an element
    // inside a content-visibility:hidden subtree forces a synchronous subtree
    // render and emits "Rendering was performed in a subtree hidden by
    // content-visibility" warnings in the console. MutationObserver is
    // layout-free and correctly tracks the .agent-tool-panel--hidden class flip
    // that applies content-visibility:hidden to the panel containing this log.
    //
    // panelHidden is a SolidJS signal so that createEffect below tracks it as a
    // reactive dependency. If it were a plain `let`, the effect would have no
    // dependency on it: when streaming completes while the panel is collapsed,
    // `chunks()` stops changing and the effect never re-fires on expand, leaving
    // `scrollTop` frozen at the pre-collapse position. Using a signal ensures
    // the effect re-runs when the panel is expanded.
    const [panelHidden, setPanelHidden] = createSignal(false);
    onMount(() => {
        const panel = scrollRef?.closest(".agent-tool-panel");
        if (!panel) return;
        setPanelHidden(panel.classList.contains("agent-tool-panel--hidden"));
        const mo = new MutationObserver(() => {
            setPanelHidden(panel.classList.contains("agent-tool-panel--hidden"));
        });
        mo.observe(panel, { attributes: true, attributeFilter: ["class"] });
        onCleanup(() => mo.disconnect());
    });

    // Follow the latest output (scroll/preview-follow.ts): pin while
    // following, only the user detaches, documents start at the top.
    const initialFollow = (): boolean =>
        !(startsAtTop(props.node) && dropBashwrapStartingChunk(props.node.log?.chunks ?? []).length === 0);
    const follow = createPreviewFollow(
        { el: () => scrollRef, hidden: panelHidden, tag: () => props.node.id.slice(-7), following: initialFollow() },
        followControllerOn(),
    );
    onMount(() => {
        if (scrollRef) onCleanup(follow.attach(scrollRef));
    });

    // Tell the tool block whether this box scrolls (it shows its taller toggle
    // only then), on the same resizes the pin below follows.
    const overflow = createOverflowReporter({
        el: () => scrollRef,
        hidden: panelHidden,
        onChange: () => props.onOverflowChange,
    });

    // Pin on every resize of the box (FLIP, max-height transition, pane or
    // window resize, un-hiding) and of the content (new chunks, the result
    // swap, late renders such as syntax highlighting).
    let contentRef: HTMLDivElement | undefined;
    const hasResizeObserver = typeof ResizeObserver !== "undefined";
    onMount(() => {
        const el = scrollRef;
        if (!el || !hasResizeObserver) return;
        const ro = new ResizeObserver(() => {
            follow.resized();
            overflow.check();
        });
        ro.observe(el);
        if (contentRef) ro.observe(contentRef);
        onCleanup(() => ro.disconnect());
    });

    createEffect(() => {
        // Tracked: chunks, the rendered branch, and panelHidden — so this
        // re-runs when the panel expands after streaming has ended.
        chunks();
        const b = branch();
        panelHidden();
        // A document preview that starts streaming follows like any other.
        if (b === "chunks" && isStreaming() && follow.detachedByDefault()) follow.streamingStarted();
        // Without ResizeObserver (jsdom), pin one frame after the DOM flush.
        if (!hasResizeObserver) {
            if (follow.following()) requestAnimationFrame(follow.pin);
            requestAnimationFrame(overflow.check);
        }
    });

    // A different tool node reusing this slot (streaming-buffer cap advance,
    // see `lastNodeId` below) starts from its own initial state.
    let followNodeId = props.node.id;
    createEffect(() => {
        const id = props.node.id;
        if (id === followNodeId) return;
        followNodeId = id;
        follow.restart(initialFollow());
        if (!follow.following() && scrollRef) scrollRef.scrollTop = 0;
    });

    // FLIP-style height transition when the rendered `<Switch>` branch below
    // changes (running -> terminal, most commonly): `ChunkList` and
    // `ToolOverlayResult` are different component trees with different
    // natural heights, and today they swap with zero transition — the
    // "jerk" in ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md.
    // Migrated to the shared contract (step 3 of
    // SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md) — see resize-contract.ts
    // for the FLIP mechanics themselves (reduced motion, magnitude cap,
    // cancellation, and the content-visibility check that fixes this file's
    // own former heightStale/panelHidden lag, §3a of that spec).
    //
    // `.agent-tool-overlay-log` scrolls its own overflow
    // (`overflow-y: auto`, `_tool-overlay-portal.scss`) inside
    // `.agent-tool-panel`'s `max-height` cap (a third of 50vh) — the module's default
    // measurement (`offsetHeight`, the rendered box) clamps at whatever's
    // left of that budget and stops changing once content exceeds it,
    // while `scrollHeight` keeps reflecting the true content height.
    // That's exactly the large-shrink case (a long raw chunk log
    // collapsing to a short compact result) this FLIP exists to smooth, so
    // it must measure `scrollHeight`, not the default.
    const measureHeight = (el: HTMLElement): number => el.scrollHeight;

    // BUT the magnitude cap (resize-contract.ts's MAX_ANIMATED_DELTA_PX)
    // must be gated on the RENDERED delta, not this scrollHeight one
    // (codex P2, PR #2962): a raw chunk log anywhere near
    // MAX_TOOL_OUTPUT_LINES (1,000) has a scrollHeight delta easily in the
    // tens of thousands of px against a short terminal result, which would
    // blow the cap and skip animating — even though the box's actual
    // VISIBLE shrink is bounded by the same vh cap to at most a few
    // hundred px. The cap exists for an unrelated phenomenon (a whole
    // pane's scrollHeight collapsing by 20,000+px — see that constant's
    // own doc comment); measured against scrollHeight here, it would fire
    // for exactly the long-output transitions this FLIP most needs to
    // smooth. offsetHeight is what the user actually sees change size.
    const measureRenderedHeight = (el: HTMLElement): number => el.offsetHeight;

    // Measure only when the rendered branch changes, and capture the "from"
    // height BEFORE this update's DOM patch rather than carrying a baseline
    // from the previous update.
    //
    // A `createComputed` is a pure computation: Solid runs those before any
    // render effect in the same update, so when it sees the branch change,
    // the DOM still shows the OLD branch — exactly the height to FLIP from.
    // The user effect below runs after the patch and commits (measures the
    // new branch, starts the FLIP).
    //
    // This used to re-baseline on EVERY run of an effect that tracked
    // `props.node` — and every stream flush hands each mounted tool log a
    // new node object, changed or not. Each baseline was `getComputedStyle`
    // up the whole ancestor chain plus `scrollHeight`/`offsetHeight`: forced
    // style and layout inside the flush, per tool log in the streaming
    // buffer, per flush (~5 % of main-thread time with three panes
    // streaming — TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md
    // §3.4). Same-branch updates now read nothing.
    //
    // It also means a same-branch update no longer cancels a FLIP in flight
    // (the old re-baseline did, as a side effect): a per-flush identity
    // change used to cut most 150 ms transitions short. The next branch
    // change still cancels a running one (resize-contract.ts's
    // cancelInFlight), before measuring.
    let lastBranch: LogBranch | undefined;
    // Guards the same `<Index>` slot-position hazard `ToolBlock.tsx` guards
    // via `prevNodeId` (PR #1317; see the "WHY <Index> NOT <Key>" note in
    // `AgentDocumentVirtualList.tsx`): a streaming-buffer cap-advance can swap
    // a different tool node into this component instance without it ever
    // unmounting. Without this guard a pending commit captured for the
    // OUTGOING node would fire against the INCOMING node's first render, FLIPping from the old tool's height to
    // the new tool's height (reagent P1 round 2 on PR #1975).
    let lastNodeId: string = props.node.id;
    let pendingCommit: (() => void) | undefined;

    createComputed(() => {
        const b = branch();
        const nodeId = props.node.id;
        if (nodeId !== lastNodeId) {
            // Different node reused this slot — never animate across the
            // swap: drop anything pending, and stop a FLIP still running for
            // the outgoing node, or its pinned height keeps animating
            // against the incoming node's content until transitionend
            // (ReAgent P1, #3607). No measuring: cancelling reads nothing.
            lastNodeId = nodeId;
            lastBranch = b;
            pendingCommit = undefined;
            if (scrollRef) cancelHeightContinuity(scrollRef);
            return;
        }
        const el = scrollRef;
        // `el` is unset only on the very first run, during setup.
        // Not while the log can't be seen: its branch changes when a dormant
        // tool finishes, and measuring (computed style up the ancestors,
        // scrollHeight, offsetHeight) and easing a height nobody sees would
        // force the layout the dormancy gate exists to avoid.
        if (el && lastBranch !== undefined && b !== lastBranch && props.open !== false) {
            pendingCommit = beginHeightContinuity(el, measureHeight, measureRenderedHeight);
        }
        lastBranch = b;
    });

    createEffect(() => {
        branch(); // re-run after the DOM patch for a branch change
        const commit = pendingCommit;
        pendingCommit = undefined;
        commit?.();
    });
    // No onCleanup here (the old code had one, cancelling any in-flight
    // flip on unmount) — resize-contract.ts doesn't expose a per-element
    // cancel to callers, only the two entry points above. Unmounting mid-
    // flip leaves a harmless, self-resolving remainder: the pending rAF
    // fires once against a now-detached `el` (a no-op — no error, no
    // visible effect), its `transitionend` listener never fires on a
    // detached, non-animating element, and the module's own `inFlight`
    // WeakMap entry for `el` is reclaimed once nothing else references
    // `el`, i.e. as part of ordinary GC after this component's own
    // teardown — not a real leak.

    /**
     * Render decision — exhaustive, mutually exclusive branches via
     * `<Switch>` rather than the prior 4-way `<Show>` cascade that
     * rendered `ToolOverlayResult` from TWO different branches. SolidJS's
     * reconciler saw the same component type in two sibling slots and
     * (during the running → success state transition) tried to re-parent
     * the DOM node from one Show slot to the other, calling
     * `replaceChild` on a node that was no longer a child of the
     * expected parent. `<Switch>` exits all other branches before
     * rendering the matched one — no shared DOM between branches.
     */
    return (
        <div
            class="agent-tool-overlay-log"
            ref={scrollRef}
            onScroll={() => follow.scrolled()}
        >
            <div class="agent-tool-overlay-log-content" ref={contentRef}>
            <Switch>
                <Match when={branch() === "chunks"}>
                    <ChunkList chunks={shownChunks()} />
                </Match>
                {/* One branch for the placeholder and the result: ToolOverlayResult
                    swaps them itself, without remounting. */}
                <Match when={branch() === "result" || branch() === "empty"}>
                    <ToolOverlayResult node={shownNode()} dispatchMatch={shownDispatch()} />
                </Match>
            </Switch>
            </div>
        </div>
    );
};

type LogChunk = { kind: string; content: string; timestamp: number };

/** A running tool's streamed output: joined into lines and drawn exactly as
 *  its finished output will be (ChunkPreview.tsx). */
function ChunkList(props: { chunks: ReadonlyArray<LogChunk> }): JSX.Element {
    return <ChunkPreview chunks={props.chunks} class="agent-tool-log-feed" />;
}

ToolOverlayLog.displayName = "ToolOverlayLog";

/**
 * Per-tool rich result fallback — same content the old portal overlay
 * rendered. Used when there are no streaming chunks yet (or the tool
 * doesn't stream).
 */
function ToolOverlayResult(props: {
    node: ToolNode;
    dispatchMatch?: AgentDispatch;
}): JSX.Element {
    // NEVER destructure `const node = props.node`. The streaming
    // buffer keeps this component mounted across reducer updates;
    // the reducer's ToolChunkAppend replaces the ToolNode reference
    // for each chunk. A destructured `node` would capture the very
    // first reference (status="running", no log) and freeze — every
    // subsequent JSX evaluation would see the stale snapshot and
    // keep rendering "⏳ Running..." even after chunks landed and
    // status flipped. (Same pattern MarkdownBlock at lines 18-23
    // warns against; bit us on PR #887.)
    //
    // But not every new reference is a change: a stream flush hands each
    // mounted tool a new node object whether or not anything about it changed,
    // and the body below is built from the node as a whole, so reading
    // props.node directly rebuilt a finished tool's entire result (every line
    // of its output) on every flush: 2 to 6 times per tool in the lab, most of
    // the pane's DOM churn with several agents streaming
    // (SPEC_AGENT_PANE_STREAM_FRAME_BUDGET_2026_10_13.md §3 R3). This memo
    // passes a new node on only when one of its own fields changed.
    const node = createMemo(() => props.node, undefined, { equals: sameToolNode });
    return (
        <Show
            when={node().status !== "running"}
            fallback={
                <div class="agent-tool-loading">
                    <span class="agent-tool-spinner">⏳</span> Thinking...
                </div>
            }
        >
            {renderToolResultBody(node(), { dispatchMatch: props.dispatchMatch })}
        </Show>
    );
}

/** Same node for rendering: the same object, or one whose own fields are all
 *  the same values (nested objects compared by reference, as the reducer
 *  keeps the ones it didn't change). */
export function sameToolNode(a: ToolNode, b: ToolNode): boolean {
    if (a === b) return true;
    const ka = Object.keys(a) as (keyof ToolNode)[];
    if (ka.length !== Object.keys(b).length) return false;
    for (const k of ka) if (a[k] !== b[k]) return false;
    return true;
}

function renderToolResultBody(node: ToolNode, ctx?: ToolRenderContext): JSX.Element {
    // Hard fallback to the default renderer if (somehow) nothing is registered.
    const render = resolveToolRenderer(node) ?? renderCompactDefault;
    return render(node, ctx);
}

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

import { For, Match, Show, Switch, createComputed, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
// `Show` retained for fallback ToolOverlayResult sub-tree.
import type { ToolNode } from "../types";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { beginHeightContinuity, cancelHeightContinuity } from "../resize-contract";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { BashCommandView } from "./shell-highlight/ShellCommand";
import { capChars, createChunkCapper, createSpinnerCollapser, dropBashwrapStartingChunk } from "./output-cap";
import { startsAtTop } from "../tool-meta/tool-descriptors";
import { attachScrollHandoff } from "./scroll-handoff";
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
}

/** Same window as the pane's own (`AgentDocumentVirtualList.tsx`'s
 *  USER_INPUT_WINDOW_MS): how long after the last user scroll input a scroll
 *  event still counts as the user's. Not imported, to keep this component
 *  free of the virtual list's module graph. */
const USER_INPUT_WINDOW_MS = 250;
/** Re-attach when a user scroll ends this close to the bottom — the pane's
 *  REATTACH_PX (pane spec §5.5); not 1 px, for fractional positions at
 *  non-100% zoom. */
const REATTACH_PX = 24;
const SCROLL_KEYS = new Set(["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown", " "]);

const KIND_CLASS: Record<string, string> = {
    stdout: "agent-tool-log-line--stdout",
    stderr: "agent-tool-log-line--stderr",
    system: "agent-tool-log-line--system",
    "diff-hunk": "agent-tool-log-line--diff",
};

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
    // The streaming log has output only; a Bash call also shows its command, as
    // the finished viewer does, so it is visible from the first frame.
    const bashCommand = (): string | undefined =>
        props.node.tool === "Bash" ? (props.node.params as { command?: string }).command : undefined;

    // Mirrors the `<Switch>` branches below exactly — used only to detect
    // when the RENDERED branch changes (for the height-FLIP effect further
    // down), so it must stay a plain function, not a `createMemo`: see the
    // "INLINE prop access pattern" note above `isStreaming` — memoizing
    // anything derived from `props.node.log?.chunks` has previously broken
    // reactivity for in-place chunk-array mutations (PR #884/#885/#886).
    type LogBranch = "streaming" | "result" | "chunks-final" | "empty";
    const branch = (): LogBranch => {
        if (isStreaming() && hasChunks()) return "streaming";
        if (!isStreaming() && hasResult()) return "result";
        if (!isStreaming() && !hasResult() && hasChunks()) return "chunks-final";
        return "empty";
    };

    // Scroll hand-off to the outer pane once this box can't scroll further
    // (scroll-handoff.ts; this box carries `overscroll-behavior: contain` in
    // _tool-overlay-portal.scss). A tool preview skids at its edge even when its
    // content fits, so a short preview under the pointer still holds the wheel.
    onMount(() => {
        if (scrollRef) onCleanup(attachScrollHandoff(scrollRef, { skidWhenFits: true }));
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

    // Follow the latest output: the agent pane's FOLLOWING / DETACHED model
    // (SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §5), local to
    // this scroller until that spec's shared follow controller exists.
    // SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md §3.
    //
    // - FOLLOWING pins to the bottom on every content or box resize (the
    //   ResizeObserver below). The old code pinned once, one frame after each
    //   node update, and never again: a result that kept growing after that
    //   frame (async syntax highlighting, late renderers, the height FLIP)
    //   was left with its bottom out of view once updates stopped — the
    //   "wanders to the middle" report.
    // - Only the user can detach. A scroll with no wheel / touch / scrollbar /
    //   scroll-key input in the last USER_INPUT_WINDOW_MS never does, whatever
    //   its geometry — a browser clamp, scroll anchoring, the output cap
    //   trimming head lines. The old rule (any scroll > 40px from the bottom)
    //   detached on all of those.
    // - A user scroll that ends within REATTACH_PX of the bottom re-attaches.
    // - Previews whose descriptor says `scroll: "top"` (documents: Read /
    //   Write / Edit; content-first answers: WebSearch) that haven't streamed
    //   start DETACHED at the top: that's where reading begins. If one does
    //   start streaming, it follows.
    const initialFollow = (): boolean =>
        !(startsAtTop(props.node) && dropBashwrapStartingChunk(props.node.log?.chunks ?? []).length === 0);
    let following = initialFollow();
    // Detached only because of the document-preview default, not by the user.
    let detachedByDefault = !following;
    let lastScrollTop = 0;
    let lastUserScrollInputAt = Number.NEGATIVE_INFINITY;
    let scrollbarPointerHeld = false;
    const hasRecentUserScrollInput = (): boolean =>
        scrollbarPointerHeld || performance.now() - lastUserScrollInputAt <= USER_INPUT_WINDOW_MS;

    // One line per state change — `muxlog fe grep scroll-follow`, same prefix
    // as the pane's own follow log (I4 of the pane spec).
    const setFollowing = (next: boolean, cause: string, detail?: string): void => {
        detachedByDefault = false;
        if (next === following) return;
        following = next;
        console.info(
            "[scroll-follow]",
            `tool=${props.node.id.slice(-7)}`,
            `${next ? "detached→following" : "following→detached"} cause=${cause}${detail ? ` ${detail}` : ""}`,
        );
    };

    // The only writer of scrollTop-to-bottom. Called from ResizeObserver
    // callbacks (layout is already clean there) or, without RO, from a rAF.
    // Re-checks isConnected: a Switch-branch flip can detach the element, and
    // writing scrollTop on a detached node raised the `replaceChild`
    // reconciliation race that crashed v0.33.799. Never touches a
    // content-visibility:hidden subtree.
    const pinToBottom = (): void => {
        const el = scrollRef;
        if (!el || !following || !el.isConnected || panelHidden()) return;
        el.scrollTop = el.scrollHeight;
        lastScrollTop = el.scrollTop;
    };

    const onScroll = (): void => {
        const el = scrollRef;
        if (!el || panelHidden()) return;
        const top = el.scrollTop;
        const gap = el.scrollHeight - el.clientHeight - top;
        const movedUp = top < lastScrollTop;
        lastScrollTop = top;
        if (!hasRecentUserScrollInput()) return; // not the user: never changes follow
        if (following && movedUp && gap > REATTACH_PX) {
            setFollowing(false, scrollbarPointerHeld ? "user-scroll:scrollbar" : "user-scroll", `gap=${Math.round(gap)}px`);
        } else if (!following && gap <= REATTACH_PX) {
            setFollowing(true, "user-scroll-to-bottom");
        }
    };

    // Record user scroll input. Wheel and touch on the scroller; a pointer
    // only when pressed on the scroller element itself (its scrollbar —
    // content clicks target a child), held open until release; scroll keys
    // while focus is inside. Target identity, not a hit-test: no layout read
    // in an input handler.
    onMount(() => {
        const el = scrollRef;
        if (!el) return;
        const mark = (): void => {
            lastUserScrollInputAt = performance.now();
        };
        const onPointerDown = (e: PointerEvent): void => {
            if (e.target !== el) return;
            scrollbarPointerHeld = true;
            mark();
        };
        const onPointerUp = (): void => {
            if (!scrollbarPointerHeld) return;
            scrollbarPointerHeld = false;
            mark(); // the drag's trailing scroll events are still the user's
        };
        const onKey = (e: KeyboardEvent): void => {
            if (SCROLL_KEYS.has(e.key)) mark();
        };
        const opts: AddEventListenerOptions = { passive: true, capture: true };
        const types = ["wheel", "touchstart", "touchmove"] as const;
        for (const type of types) el.addEventListener(type, mark, opts);
        el.addEventListener("pointerdown", onPointerDown, opts);
        el.addEventListener("keydown", onKey, opts);
        window.addEventListener("pointerup", onPointerUp, opts);
        window.addEventListener("pointercancel", onPointerUp, opts);
        onCleanup(() => {
            for (const type of types) el.removeEventListener(type, mark, opts);
            el.removeEventListener("pointerdown", onPointerDown, opts);
            el.removeEventListener("keydown", onKey, opts);
            window.removeEventListener("pointerup", onPointerUp, opts);
            window.removeEventListener("pointercancel", onPointerUp, opts);
        });
    });

    // Pin on every resize of the box (FLIP, max-height transition, pane or
    // window resize, un-hiding) and of the content (new chunks, the result
    // swap, late renders such as syntax highlighting).
    let contentRef: HTMLDivElement | undefined;
    const hasResizeObserver = typeof ResizeObserver !== "undefined";
    onMount(() => {
        const el = scrollRef;
        if (!el || !hasResizeObserver) return;
        const ro = new ResizeObserver(() => pinToBottom());
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
        if (b === "streaming" && detachedByDefault) setFollowing(true, "streaming-started");
        // Without ResizeObserver (jsdom), pin one frame after the DOM flush.
        if (!hasResizeObserver && following) requestAnimationFrame(pinToBottom);
    });

    // A different tool node reusing this slot (streaming-buffer cap advance,
    // see `lastNodeId` below) starts from its own initial state.
    let followNodeId = props.node.id;
    createEffect(() => {
        const id = props.node.id;
        if (id === followNodeId) return;
        followNodeId = id;
        following = initialFollow();
        detachedByDefault = !following;
        lastScrollTop = 0;
        if (!following && scrollRef) scrollRef.scrollTop = 0;
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
        if (el && lastBranch !== undefined && b !== lastBranch) {
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
            onScroll={onScroll}
        >
            <div class="agent-tool-overlay-log-content" ref={contentRef}>
            <Switch>
                <Match when={isStreaming() && hasChunks()}>
                    <ChunkList chunks={chunks()} command={bashCommand()} />
                </Match>
                <Match when={!isStreaming() && hasResult()}>
                    <ToolOverlayResult node={props.node} dispatchMatch={props.dispatchMatch} />
                </Match>
                <Match when={!isStreaming() && !hasResult() && hasChunks()}>
                    <ChunkList chunks={chunks()} command={bashCommand()} />
                </Match>
                <Match when={!hasChunks() && !hasResult()}>
                    <ToolOverlayResult node={props.node} dispatchMatch={props.dispatchMatch} command={bashCommand()} />
                </Match>
            </Switch>
            </div>
        </div>
    );
};

type LogChunk = { kind: string; content: string; timestamp: number };

interface ChunkListProps {
    chunks: ReadonlyArray<LogChunk>;
    /** Bash calls: the command, shown above the output. */
    command?: string;
}
function ChunkList(props: ChunkListProps): JSX.Element {
    // Collapse first (raw, incremental — see PersistentShellBlock.tsx for
    // why this order matters both for correctness under a long stream and
    // for createSpinnerCollapser's append-only identity tracking), then cap
    // the deduplicated result to the line budget.
    const spinnerCollapse = createSpinnerCollapser<LogChunk>();
    const cap = createChunkCapper();

    const view = createMemo(() => {
        // dropBashwrapStartingChunk BEFORE collapse/cap, not after: filtering
        // downstream would still burn one line of the cap budget per system
        // chunk while hiding the rendered row, silently evicting real
        // output. See output-cap.ts's doc comment for why this is also safe
        // to call fresh every render despite the stateful collapse/cap
        // functions' append-only identity tracking.
        const { display: collapsed, spinnerSlot } = spinnerCollapse(dropBashwrapStartingChunk(props.chunks));
        const { chunks: display, hiddenLines } = cap(collapsed);
        return { display, spinnerSlot, hiddenLines };
    });

    return (
        <>
            <Show when={props.command}>
                <div class="agent-bash">
                    <BashCommandView command={props.command!} />
                </div>
            </Show>
            <Show when={view().hiddenLines > 0}>
                <OutputHiddenMarker hidden={view().hiddenLines} noun="line" from="tail" />
            </Show>
            <For each={view().display}>
                {(chunk) => (
                    <pre class={`agent-tool-log-line ${KIND_CLASS[chunk.kind] ?? ""}`}>
                        {capChars(chunk.content)}
                    </pre>
                )}
            </For>
            <Show when={view().spinnerSlot !== null}>
                <pre class={`agent-tool-log-line ${KIND_CLASS[view().spinnerSlot?.kind ?? ""] ?? ""}`}>
                    {view().spinnerSlot?.content}
                </pre>
            </Show>
        </>
    );
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
    /** Bash calls still waiting for output: the command, shown above the spinner. */
    command?: string;
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
    return (
        <Show
            when={props.node.status !== "running"}
            fallback={
                <>
                    <Show when={props.command}>
                        <div class="agent-bash">
                            <BashCommandView command={props.command!} />
                        </div>
                    </Show>
                    <div class="agent-tool-loading">
                        <span class="agent-tool-spinner">⏳</span> Thinking...
                    </div>
                </>
            }
        >
            {renderToolResultBody(props.node, { dispatchMatch: props.dispatchMatch })}
        </Show>
    );
}

function renderToolResultBody(node: ToolNode, ctx?: ToolRenderContext): JSX.Element {
    // Hard fallback to the default renderer if (somehow) nothing is registered.
    const render = resolveToolRenderer(node) ?? renderCompactDefault;
    return render(node, ctx);
}

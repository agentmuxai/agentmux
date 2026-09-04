// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolOverlayLog — log body of the tool overlay (Phase 3 of
 * SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4).
 *
 * Renders `ToolNode.log.chunks` if the streaming runner has populated
 * them. Falls back to per-tool rich result content (BashOutputViewer,
 * DiffViewer, ...) when no chunks are present — preserves today's UX
 * for tools that don't stream (yet) or that have already terminated
 * before Phase 2's backend wraps the runner.
 *
 * No ANSI parsing in this PR (Phase 3): chunks render as plain text.
 * ANSI parsing lands in Phase γ (perf + worker offload) per the spec.
 */

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
// `Show` retained for fallback ToolOverlayResult sub-tree.
import type { ToolNode } from "../types";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { beginHeightContinuity } from "../resize-contract";
import { Markdown } from "@/app/element/markdown";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";
import { HighlightedCode } from "./HighlightedCode";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capChars, createChunkCapper, createSpinnerCollapser, capText, dropBashwrapStartingChunk, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { stripCommonIndent, stripCommonIndentNumbered } from "./dedent";
import { detectLanguage } from "./detectLanguage";
import {
    registerToolRenderer,
    resolveToolRenderer,
    byKind,
    anyTool,
    type ToolRenderContext,
} from "./tool-renderers/registry";
// Side-effect: registers the rich renderers (WebSearch cards, WebFetch view, record tables, …).
import "./tool-renderers/SearchResults";
import "./tool-renderers/WebFetchResult";
import "./tool-renderers/RecordTable";
import "./tool-renderers/DispatchCard";

interface ToolOverlayLogProps {
    node: ToolNode;
    fontScale?: () => number;
    /** Ordinal-matched live dispatch for an Agent/Task/Workflow tool call —
     *  see `activity/dispatch-correlation.ts`. Undefined when no confident
     *  match was found, or for any other tool kind. */
    dispatchMatch?: AgentDispatch;
}

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
    const hasChunks = () => (props.node.log?.chunks?.length ?? 0) > 0;
    const hasResult = () => props.node.result != null;
    const chunks = () => props.node.log?.chunks ?? [];

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

    // Auto-stick to bottom while the user hasn't scrolled away. The
    // threshold is forgiving — within 40px of the bottom counts as
    // "still at bottom" so a single mousewheel tick doesn't unstick.
    let stickToBottom = true;
    const onScroll = () => {
        if (!scrollRef) return;
        const dist = scrollRef.scrollHeight - scrollRef.scrollTop - scrollRef.clientHeight;
        stickToBottom = dist < 40;
    };

    // Scroll-chaining handoff to the outer pane (SPEC_TOOL_PREVIEW_SCROLL_
    // CHAINING_2026_07_03.md Phase 2). This box carries `overscroll-behavior:
    // contain` (_tool-overlay-portal.scss) so the browser's native chaining —
    // which would otherwise hand excess wheel delta to `.agent-document` once
    // this box hits its scroll limit — never fires; `contain` blocks that
    // relay unconditionally, regardless of any JS listener. Verified live via
    // CDP: with only the CSS (Phase 1), reaching either boundary of this box
    // hard-dead-ends further wheel ticks — `.agent-document`'s scrollTop never
    // moves. So the handoff has to be done by hand: once this box can't
    // consume more scroll in the wheel's direction, forward the same delta to
    // the outer pane directly (not "let it bubble" — bubbling the event
    // doesn't help, `contain` isn't an event-propagation setting).
    onMount(() => {
        const el = scrollRef;
        if (!el) return;
        const onWheel = (e: WheelEvent) => {
            // Ctrl+wheel is the preview font-zoom gesture (ToolBlock.tsx) —
            // leave it alone.
            if (e.ctrlKey) return;
            const atTop = el.scrollTop <= 0;
            const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
            const scrollingUp = e.deltaY < 0;
            const scrollingDown = e.deltaY > 0;
            if (!((atTop && scrollingUp) || (atBottom && scrollingDown))) return;
            const outerPane = el.closest<HTMLElement>(".agent-document");
            if (!outerPane) return;
            e.preventDefault();
            outerPane.scrollTop += e.deltaY;
        };
        el.addEventListener("wheel", onWheel, { passive: false });
        onCleanup(() => el.removeEventListener("wheel", onWheel));
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

    createEffect(() => {
        // Re-read chunks and panelHidden to register both as reactive deps.
        // panelHidden must be read here (not inside the RAF callback) so the
        // effect re-fires when the panel expands even after streaming has ended
        // and chunks() is no longer changing.
        chunks();
        const hidden = panelHidden();
        if (stickToBottom && scrollRef) {
            // Wait one frame for the DOM to flush before measuring.
            // Re-check scrollRef + isConnected because a Show-branch
            // flip during the same RAF window can detach the element
            // out from under us. Mutating scrollTop on a detached node
            // raised the `replaceChild` reconciliation race that
            // crashed v0.33.799. Guard panelHidden to avoid forcing
            // layout on a content-visibility:hidden subtree.
            requestAnimationFrame(() => {
                if (scrollRef && scrollRef.isConnected && !hidden) {
                    scrollRef.scrollTop = scrollRef.scrollHeight;
                }
            });
        }
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
    // `.agent-tool-panel`'s `max-height: 50vh` cap — the module's default
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
    // VISIBLE shrink is bounded by the same 50vh cap to at most a few
    // hundred px. The cap exists for an unrelated phenomenon (a whole
    // pane's scrollHeight collapsing by 20,000+px — see that constant's
    // own doc comment); measured against scrollHeight here, it would fire
    // for exactly the long-output transitions this FLIP most needs to
    // smooth. offsetHeight is what the user actually sees change size.
    const measureRenderedHeight = (el: HTMLElement): number => el.offsetHeight;

    // `lastBranch`/`pendingCommit` hold what the PREVIOUS run of this
    // effect captured — Solid effects always run after the DOM has already
    // been patched for the change that triggered them, so a run can only
    // ever see its OWN "after" state; the "before" has to come from
    // whatever the last run left behind. `beginHeightContinuity` is called
    // on every invocation (not just branch changes) so the eventually-
    // committed "from" height is always the most recent one, not whatever
    // it was when the current branch started — matching the old
    // `lastMeasuredHeight` field's unconditional per-tick update.
    let lastBranch: LogBranch | undefined;
    // Guards the same `<Index>` slot-position hazard `ToolBlock.tsx` guards
    // via `prevNodeId` (PR #1317, `AgentDocumentVirtualList.tsx:193-194`): a
    // streaming-buffer cap-advance can swap a different tool node into this
    // component instance without it ever unmounting. Without this guard a
    // pending commit captured for the OUTGOING node would fire against the
    // INCOMING node's first render, FLIPping from the old tool's height to
    // the new tool's height (reagent P1 round 2 on PR #1975).
    let lastNodeId: string = props.node.id;
    let pendingCommit: (() => void) | undefined;

    createEffect(() => {
        const b = branch();
        chunks(); // also re-measure as chunks stream in, not just on branch changes
        const el = scrollRef;
        if (!el) return;

        const nodeId = props.node.id;
        if (nodeId !== lastNodeId) {
            // Different node reused this slot — never animate across the
            // swap; discard whatever was pending and start fresh for the
            // incoming node.
            lastNodeId = nodeId;
            lastBranch = b;
            pendingCommit = beginHeightContinuity(el, measureHeight, measureRenderedHeight);
            return;
        }

        const branchChanged = lastBranch !== undefined && b !== lastBranch;
        lastBranch = b;

        // Re-baseline for the NEXT invocation BEFORE committing this one.
        // beginHeightContinuity cancels any transition already in flight as
        // part of capturing a trustworthy "from" height (resize-contract.ts's
        // cancelInFlight) — doing that AFTER committing would immediately
        // cancel the flip this same tick is about to start. Capturing first
        // means the only thing it can cancel is a PRIOR, now-stale
        // transition, which is exactly what should happen once content has
        // moved on again.
        const commit = pendingCommit;
        pendingCommit = beginHeightContinuity(el, measureHeight, measureRenderedHeight);
        if (branchChanged) commit?.();
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
            style={props.fontScale ? { "font-size": `${props.fontScale() * 100}%` } : undefined}
        >
            <Switch>
                <Match when={isStreaming() && hasChunks()}>
                    <ChunkList chunks={chunks()} />
                </Match>
                <Match when={!isStreaming() && hasResult()}>
                    <ToolOverlayResult node={props.node} dispatchMatch={props.dispatchMatch} />
                </Match>
                <Match when={!isStreaming() && !hasResult() && hasChunks()}>
                    <ChunkList chunks={chunks()} />
                </Match>
                <Match when={!hasChunks() && !hasResult()}>
                    <ToolOverlayResult node={props.node} dispatchMatch={props.dispatchMatch} />
                </Match>
            </Switch>
        </div>
    );
};

type LogChunk = { kind: string; content: string; timestamp: number };

interface ChunkListProps {
    chunks: ReadonlyArray<LogChunk>;
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
function ToolOverlayResult(props: { node: ToolNode; dispatchMatch?: AgentDispatch }): JSX.Element {
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
                <div class="agent-tool-loading">
                    <span class="agent-tool-spinner">⏳</span> Thinking...
                </div>
            }
        >
            {renderToolResultBody(props.node, { dispatchMatch: props.dispatchMatch })}
        </Show>
    );
}

// Per-tool result renderers. Each is registered with the tool-renderer registry
// (below) so the open-ended tool universe can be routed by name/shape rather than
// a closed switch — these are the built-in (coarse-kind) entries. The bodies are
// the former `switch` arms verbatim; behavior is unchanged. See
// SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md (Phase 1).

function renderEdit(node: ToolNode): JSX.Element {
    return <DiffViewer params={node.params as any} result={node.result as any} status={node.status} />;
}

function renderBash(node: ToolNode): JSX.Element {
    return <BashOutputViewer params={node.params as any} result={node.result as any} />;
}

function renderRead(node: ToolNode): JSX.Element {
    const filePath = (node.params as any).file_path ?? "";
    const content: string | undefined = (node.result as any)?.content;
    // Head-cap file content (read top-down) so a huge Read can't bloat
    // the conversation DOM; HighlightedCode stays simple (it injects
    // innerHTML, so capping here is cleaner than inside it).
    const capped = content ? capText(content, MAX_TOOL_OUTPUT_LINES, "head") : null;
    // Strip indentation common to every VISIBLE line — computed after
    // capping so a deeper hidden tail can't reduce the dedent of what's
    // actually shown (SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md §3.2.1). Claude
    // Code's Read result lines are "<N>\t<code>"; the numbered variant
    // splits off that prefix before dedenting so the line-number gutter
    // itself is untouched.
    const dedentedText = capped ? stripCommonIndentNumbered(capped.text) : "";
    // Render markdown files as formatted markdown, matching renderWrite. Without
    // this a .md Read shows raw source instead of a rendered preview.
    const isMarkdown = filePath.endsWith(".md") || filePath.endsWith(".mdx");
    return (
        <div class="agent-tool-read">
            <div class="agent-tool-file-path">{filePath}</div>
            <Show
                when={capped}
                fallback={
                    <Show when={node.result}>
                        <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                    </Show>
                }
            >
                <Show
                    when={isMarkdown}
                    fallback={
                        <HighlightedCode
                            code={dedentedText}
                            // Language detection reads the RAW (non-dedented) first
                            // line — shebang/content sniffing should see the file
                            // as-is; only the displayed text is dedented.
                            lang={detectLanguage(filePath, capped!.text.split("\n")[0])}
                            class="agent-tool-read-content"
                        />
                    }
                >
                    <div class="agent-tool-read-content agent-tool-read-md">
                        <Markdown text={dedentedText} />
                    </div>
                </Show>
                <Show when={capped!.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={capped!.hiddenLines} noun="line" from="head" />
                </Show>
            </Show>
        </div>
    );
}

function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function renderWrite(node: ToolNode): JSX.Element {
    const filePath = (node.params as any).file_path ?? "";
    const content: string | undefined = (node.params as any).content;
    const bytes: number | undefined = (node.result as any)?.bytesWritten;
    const capped = content ? capText(content, MAX_TOOL_OUTPUT_LINES, "head") : null;
    // Plain dedent, NOT the Read-specific numbered variant
    // (SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md §3.2.3) — Write content has no
    // CLI-added "<N>\t" line-number prefix, so stripCommonIndentNumbered's
    // heuristic would misfire on a genuine tab-delimited file (TSV/BED/GTF)
    // whose every non-blank line happens to start with digits+tab, silently
    // dropping that real leading column. stripCommonIndent has no such
    // ambiguity and is a no-op for the common already-flush case anyway.
    const dedentedText = capped ? stripCommonIndent(capped.text) : "";
    const isMarkdown = filePath.endsWith(".md") || filePath.endsWith(".mdx");
    return (
        <div class="agent-tool-write">
            <div class="agent-tool-file-path-row">
                <span class="agent-tool-file-path">{filePath}</span>
                <Show when={bytes != null}>
                    <span class="agent-tool-write-bytes">{formatBytes(bytes!)}</span>
                </Show>
            </div>
            <Show when={capped} fallback={<div class="agent-tool-write-info">No content written.</div>}>
                <Show
                    when={isMarkdown}
                    fallback={
                        <HighlightedCode
                            code={dedentedText}
                            lang={detectLanguage(filePath, capped!.text.split("\n")[0])}
                            class="agent-tool-write-content"
                        />
                    }
                >
                    <div class="agent-tool-write-content agent-tool-write-md">
                        <Markdown text={dedentedText} />
                    </div>
                </Show>
                <Show when={capped!.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={capped!.hiddenLines} noun="line" from="head" />
                </Show>
            </Show>
        </div>
    );
}

function renderSearch(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-search">
            <div class="agent-tool-pattern">Pattern: {(node.params as any).pattern}</div>
            <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
        </div>
    );
}

// Exported (not just module-local) so `DispatchCard.tsx`'s no-match fallback
// can delegate to the SAME per-kind rendering these built-ins already do
// (description-while-running, no-result gating) instead of a bare
// `CompactResult` call that loses both — reagent/codex P1 on PR #2676: a
// still-running unmatched Agent/Task call was showing raw "No output" and a
// completed one was losing its description entirely, guaranteed to trigger
// on the Agent History tab (which always falls back to CompactResult there).
export function renderAgent(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-agent">
            <Show when={(node.params as any).description}>
                <div class="agent-tool-agent-desc">{(node.params as any).description}</div>
            </Show>
            <Show when={node.result}>
                <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
            </Show>
        </div>
    );
}

export function renderTask(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-task">
            <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
        </div>
    );
}

export function renderWorkflow(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-workflow">
            <Show when={(node.params as any).description ?? (node.params as any).title}>
                <div class="agent-tool-agent-desc">
                    {(node.params as any).description ?? (node.params as any).title}
                </div>
            </Show>
            <Show when={node.result}>
                <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
            </Show>
        </div>
    );
}

function renderCompactDefault(node: ToolNode): JSX.Element {
    return <CompactResult tool={node.tool} params={node.params as any} result={node.result} />;
}

// Register the built-ins (priority 0; the catch-all sits below everything). Rich,
// name/shape-matched renderers (WebSearch cards, mcp__* tools, DispatchCard, …)
// register from their own modules at a higher priority.
registerToolRenderer({ priority: 0, label: "builtin:Edit", match: byKind("Edit"), render: renderEdit });
registerToolRenderer({ priority: 0, label: "builtin:Bash", match: byKind("Bash"), render: renderBash });
registerToolRenderer({ priority: 0, label: "builtin:Read", match: byKind("Read"), render: renderRead });
registerToolRenderer({ priority: 0, label: "builtin:Write", match: byKind("Write"), render: renderWrite });
registerToolRenderer({ priority: 0, label: "builtin:Search", match: byKind("Grep", "Glob"), render: renderSearch });
registerToolRenderer({ priority: 0, label: "builtin:Agent", match: byKind("Agent"), render: renderAgent });
registerToolRenderer({ priority: 0, label: "builtin:Task", match: byKind("Task"), render: renderTask });
registerToolRenderer({ priority: 0, label: "builtin:Workflow", match: byKind("Workflow"), render: renderWorkflow });
registerToolRenderer({ priority: -Infinity, label: "builtin:default", match: anyTool, render: renderCompactDefault });

function renderToolResultBody(node: ToolNode, ctx?: ToolRenderContext): JSX.Element {
    // Hard fallback to the default renderer if (somehow) nothing is registered.
    const render = resolveToolRenderer(node) ?? renderCompactDefault;
    return render(node, ctx);
}

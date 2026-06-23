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

import { For, Match, Show, Switch, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
// `Show` retained for fallback ToolOverlayResult sub-tree.
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";
import { HighlightedCode } from "./HighlightedCode";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { capChars, createChunkCapper, capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { detectLanguage } from "./detectLanguage";
import {
    registerToolRenderer,
    resolveToolRenderer,
    byKind,
    anyTool,
} from "./tool-renderers/registry";
// Side-effect: registers the rich renderers (WebSearch cards, WebFetch view, record tables, …).
import "./tool-renderers/SearchResults";
import "./tool-renderers/WebFetchResult";
import "./tool-renderers/RecordTable";

interface ToolOverlayLogProps {
    node: ToolNode;
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

    // Auto-stick to bottom while the user hasn't scrolled away. The
    // threshold is forgiving — within 40px of the bottom counts as
    // "still at bottom" so a single mousewheel tick doesn't unstick.
    let stickToBottom = true;
    const onScroll = () => {
        if (!scrollRef) return;
        const dist = scrollRef.scrollHeight - scrollRef.scrollTop - scrollRef.clientHeight;
        stickToBottom = dist < 40;
    };

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
        <div class="agent-tool-overlay-log" ref={scrollRef} onScroll={onScroll}>
            <Switch>
                <Match when={isStreaming() && hasChunks()}>
                    <ChunkList chunks={chunks()} />
                </Match>
                <Match when={!isStreaming() && hasResult()}>
                    <ToolOverlayResult node={props.node} />
                </Match>
                <Match when={!isStreaming() && !hasResult() && hasChunks()}>
                    <ChunkList chunks={chunks()} />
                </Match>
                <Match when={!hasChunks() && !hasResult()}>
                    <ToolOverlayResult node={props.node} />
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
    // All CR/spinner handling is in the Rust layer: pending_cr_override slots
    // in pty_reader_loop and stream_reader collapse throttled spinner frames
    // before they become LineEvents; spawn_publisher_loop strips any leading \r
    // before publishing. No \r reaches the frontend. The cap is the only
    // transform needed here.
    const cap = createChunkCapper();
    const capped = () => cap(props.chunks);
    return (
        <>
            <Show when={capped().hiddenLines > 0}>
                <OutputHiddenMarker hidden={capped().hiddenLines} noun="line" from="tail" />
            </Show>
            <For each={capped().chunks}>
                {(chunk) => (
                    <pre class={`agent-tool-log-line ${KIND_CLASS[chunk.kind] ?? ""}`}>
                        {capChars(chunk.content)}
                    </pre>
                )}
            </For>
        </>
    );
}

ToolOverlayLog.displayName = "ToolOverlayLog";

/**
 * Per-tool rich result fallback — same content the old portal overlay
 * rendered. Used when there are no streaming chunks yet (or the tool
 * doesn't stream).
 */
function ToolOverlayResult(props: { node: ToolNode }): JSX.Element {
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
            {renderToolResultBody(props.node)}
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
                <HighlightedCode
                    code={capped!.text}
                    lang={detectLanguage(filePath, capped!.text.split("\n")[0])}
                    class="agent-tool-read-content"
                />
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
    return (
        <div class="agent-tool-write">
            <div class="agent-tool-file-path-row">
                <span class="agent-tool-file-path">{filePath}</span>
                <Show when={bytes != null}>
                    <span class="agent-tool-write-bytes">{formatBytes(bytes!)}</span>
                </Show>
            </div>
            <Show when={capped} fallback={<div class="agent-tool-write-info">No content written.</div>}>
                <HighlightedCode
                    code={capped!.text}
                    lang={detectLanguage(filePath, capped!.text.split("\n")[0])}
                    class="agent-tool-write-content"
                />
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

function renderAgent(node: ToolNode): JSX.Element {
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

function renderTask(node: ToolNode): JSX.Element {
    return (
        <div class="agent-tool-task">
            <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
        </div>
    );
}

function renderCompactDefault(node: ToolNode): JSX.Element {
    return <CompactResult tool={node.tool} params={node.params as any} result={node.result} />;
}

// Register the built-ins (priority 0; the catch-all sits below everything). Rich,
// name/shape-matched renderers (WebSearch cards, mcp__* tools, …) register from
// their own modules at a higher priority.
registerToolRenderer({ priority: 0, label: "builtin:Edit", match: byKind("Edit"), render: renderEdit });
registerToolRenderer({ priority: 0, label: "builtin:Bash", match: byKind("Bash"), render: renderBash });
registerToolRenderer({ priority: 0, label: "builtin:Read", match: byKind("Read"), render: renderRead });
registerToolRenderer({ priority: 0, label: "builtin:Write", match: byKind("Write"), render: renderWrite });
registerToolRenderer({ priority: 0, label: "builtin:Search", match: byKind("Grep", "Glob"), render: renderSearch });
registerToolRenderer({ priority: 0, label: "builtin:Agent", match: byKind("Agent"), render: renderAgent });
registerToolRenderer({ priority: 0, label: "builtin:Task", match: byKind("Task"), render: renderTask });
registerToolRenderer({ priority: -Infinity, label: "builtin:default", match: anyTool, render: renderCompactDefault });

function renderToolResultBody(node: ToolNode): JSX.Element {
    // Hard fallback to the default renderer if (somehow) nothing is registered.
    const render = resolveToolRenderer(node) ?? renderCompactDefault;
    return render(node);
}

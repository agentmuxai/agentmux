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

import { For, Match, Show, Switch, createEffect, type JSX } from "solid-js";
// `Show` retained for fallback ToolOverlayResult sub-tree.
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";
import { HighlightedCode } from "./HighlightedCode";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { createChunkCapper, capText, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { detectLanguage } from "./detectLanguage";

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

    createEffect(() => {
        // Re-read chunks to register the dep, then schedule a scroll-down.
        chunks();
        if (stickToBottom && scrollRef) {
            // Wait one frame for the DOM to flush before measuring.
            // Re-check scrollRef + isConnected because a Show-branch
            // flip during the same RAF window can detach the element
            // out from under us. Mutating scrollTop on a detached node
            // raised the `replaceChild` reconciliation race that
            // crashed v0.33.799.
            requestAnimationFrame(() => {
                if (scrollRef && scrollRef.isConnected) {
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

interface ChunkListProps {
    chunks: ReadonlyArray<{ kind: string; content: string; timestamp: number }>;
}
function ChunkList(props: ChunkListProps): JSX.Element {
    // Cap the inline render to ~MAX_TOOL_OUTPUT_LINES worth of trailing
    // chunks. Inline (not memoized) per this file's reactivity discipline.
    // The capper is stateful (per-stream running line total) so each streamed
    // append only scans the new chunk, never the growing dropped prefix.
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
                        {chunk.content}
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

function renderToolResultBody(node: ToolNode): JSX.Element {
    switch (node.tool) {
        case "Edit":
            return <DiffViewer params={node.params as any} result={node.result as any} />;

        case "Bash":
            return <BashOutputViewer params={node.params as any} result={node.result as any} />;

        case "Read": {
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

        case "Write":
            return (
                <div class="agent-tool-write">
                    <div class="agent-tool-file-path">{(node.params as any).file_path}</div>
                    <div class="agent-tool-write-info">
                        {node.result && `Wrote ${(node.result as any).bytesWritten || 0} bytes`}
                    </div>
                </div>
            );

        case "Grep":
        case "Glob":
            return (
                <div class="agent-tool-search">
                    <div class="agent-tool-pattern">Pattern: {(node.params as any).pattern}</div>
                    <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                </div>
            );

        case "Agent":
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

        case "Task":
            return (
                <div class="agent-tool-task">
                    <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                </div>
            );

        default:
            return <CompactResult tool={node.tool} params={node.params as any} result={node.result} />;
    }
}

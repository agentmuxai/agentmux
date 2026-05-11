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

import { For, Show, createEffect, createMemo, type JSX } from "solid-js";
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";
import { HighlightedCode } from "./HighlightedCode";
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

    const chunks = createMemo(() => props.node.log?.chunks ?? []);
    const hasLog = createMemo(() => chunks().length > 0);

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
            requestAnimationFrame(() => {
                if (scrollRef) scrollRef.scrollTop = scrollRef.scrollHeight;
            });
        }
    });

    return (
        <div class="agent-tool-overlay-log" ref={scrollRef} onScroll={onScroll}>
            <Show when={hasLog()} fallback={<ToolOverlayResult node={props.node} />}>
                <For each={chunks()}>
                    {(chunk) => (
                        <pre
                            class={`agent-tool-log-line ${KIND_CLASS[chunk.kind] ?? ""}`}
                        >
                            {chunk.content}
                        </pre>
                    )}
                </For>
            </Show>
        </div>
    );
};

ToolOverlayLog.displayName = "ToolOverlayLog";

/**
 * Per-tool rich result fallback — same content the old portal overlay
 * rendered. Used when there are no streaming chunks yet (or the tool
 * doesn't stream).
 */
function ToolOverlayResult(props: { node: ToolNode }): JSX.Element {
    const node = props.node;
    if (node.status === "running") {
        return (
            <div class="agent-tool-loading">
                <span class="agent-tool-spinner">⏳</span> Running...
            </div>
        );
    }

    switch (node.tool) {
        case "Edit":
            return <DiffViewer params={node.params as any} result={node.result as any} />;

        case "Bash":
            return <BashOutputViewer params={node.params as any} result={node.result as any} />;

        case "Read": {
            const filePath = (node.params as any).file_path ?? "";
            const content: string | undefined = (node.result as any)?.content;
            return (
                <div class="agent-tool-read">
                    <div class="agent-tool-file-path">{filePath}</div>
                    <Show
                        when={content}
                        fallback={
                            <Show when={node.result}>
                                <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                            </Show>
                        }
                    >
                        <HighlightedCode
                            code={content!}
                            lang={detectLanguage(filePath, content!.split("\n")[0])}
                            class="agent-tool-read-content"
                        />
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

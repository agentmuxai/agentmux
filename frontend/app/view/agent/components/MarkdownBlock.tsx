// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Markdown } from "@/app/element/markdown";
import clsx from "clsx";
import { createMemo, createSignal, Index, Show, type JSX } from "solid-js";
import type { MarkdownNode } from "../types";

/**
 * Split markdown into top-level blocks at paragraph breaks that are NOT
 * inside a ``` fence (so a code block is never cut). Lets the streaming
 * render parse only the last/growing block per frame — completed blocks
 * keep their stable string so the inner Markdown memo skips re-parsing
 * them. O(n) char scan, negligible vs a parse.
 * See docs/analysis/ANALYSIS_AGENT_PANE_TYPING_LATENCY_2026_05_30.md.
 */
function splitTopLevelBlocks(content: string): string[] {
    if (!content) return [];
    const blocks: string[] = [];
    let fenceOpen = false;
    let start = 0;
    let i = 0;
    while (i < content.length) {
        if (content.startsWith("```", i)) {
            fenceOpen = !fenceOpen;
            i += 3;
            continue;
        }
        if (!fenceOpen && content.startsWith("\n\n", i)) {
            blocks.push(content.slice(start, i));
            i += 2;
            start = i;
            continue;
        }
        i++;
    }
    if (start < content.length) blocks.push(content.slice(start));
    return blocks;
}

interface MarkdownBlockProps {
    node: MarkdownNode;
}

export const MarkdownBlock = (props: MarkdownBlockProps): JSX.Element => {
    // Don't destructure `node` — the streaming buffer keeps this row
    // mounted across token deltas, and useAgentStream replaces the
    // node reference for each chunk. A destructured `node` would
    // capture the first reference and freeze. Access props.node.X at
    // each site so Solid's reactivity tracks the read. (codex P1 on
    // PR #786 / virt redesign.)

    // Canceled thinking — orphan-scrub flipped this on at the last
    // SessionEnd or HistoryRestored. Render collapsed by default
    // with a "⏹ Canceled" label; click to expand the partial
    // content. Spec:
    // `docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md`.
    const isCanceled = (): boolean => props.node.metadata?.canceled === true;
    const [expanded, setExpanded] = createSignal(false);

    // Incremental streaming render: split into top-level blocks so only the
    // last (growing) block re-parses each frame. Completed blocks keep their
    // stable string, so the inner Markdown memo skips them — fixes the O(n²)
    // full-message re-parse that was starving keystrokes during streaming.
    const blocks = createMemo(() => splitTopLevelBlocks(props.node.content));

    return (
        <Show
            when={isCanceled()}
            fallback={
                <div
                    class={clsx("agent-markdown-block", {
                        "thinking-block": props.node.metadata?.thinking,
                    })}
                >
                    <Index each={blocks()}>
                        {(block) => <Markdown text={block()} scrollable={false} />}
                    </Index>
                </div>
            }
        >
            <div class="agent-markdown-block markdown-canceled">
                <button
                    type="button"
                    class="markdown-canceled-header"
                    onClick={() => setExpanded((v) => !v)}
                    aria-expanded={expanded()}
                >
                    <span class="markdown-canceled-icon" aria-hidden="true">⏹</span>
                    <span class="markdown-canceled-label">
                        Canceled — partial thought
                    </span>
                    <span class="markdown-canceled-chevron" aria-hidden="true">
                        {expanded() ? "▾" : "▸"}
                    </span>
                </button>
                <Show when={expanded()}>
                    <div class="markdown-canceled-body">
                        <Markdown text={props.node.content} />
                    </div>
                </Show>
            </div>
        </Show>
    );
};

MarkdownBlock.displayName = "MarkdownBlock";

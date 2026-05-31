// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Markdown } from "@/app/element/markdown";
import clsx from "clsx";
import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import type { MarkdownNode } from "../types";

interface MarkdownBlockProps {
    node: MarkdownNode;
}

// During streaming the message content grows ~60x/s. Re-parsing the whole
// document (including syntax highlighting) on every frame is O(n^2) and
// starves keystrokes (see ANALYSIS_AGENT_PANE_TYPING_LATENCY_2026_05_30.md).
// Coalesce: commit at most one cheap (un-highlighted) intermediate render per
// window while content keeps arriving, then one full highlighted render once
// it settles. The whole message stays a SINGLE parse, so lists / reference
// definitions / paragraph spacing are unaffected. This is a perf rate-limit
// on an expensive render, not a timer papering over a race.
const STREAM_RENDER_MS = 90;

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

    // Throttled view of the streaming content + whether to syntax-highlight.
    // A settled/static message renders fully (highlighted) immediately; a
    // fast stream renders cheap intermediates and a full final.
    const [view, setView] = createSignal<{ text: string; highlight: boolean }>({
        text: props.node.content,
        highlight: true,
    });
    let lastCommitAt = 0;
    let streaming = false;
    let trailing: ReturnType<typeof setTimeout> | undefined;
    createEffect(() => {
        const text = props.node.content; // dep: re-runs on each streamed update
        const now = performance.now();
        if (trailing) clearTimeout(trailing);
        if (now - lastCommitAt >= STREAM_RENDER_MS) {
            // Leading edge: cheap intermediate (skip highlight mid-stream).
            lastCommitAt = now;
            setView({ text, highlight: !streaming });
        }
        streaming = true;
        // Trailing edge: once updates stop for a window, render full + highlight.
        trailing = setTimeout(() => {
            streaming = false;
            lastCommitAt = performance.now();
            setView({ text: props.node.content, highlight: true });
        }, STREAM_RENDER_MS);
    });
    onCleanup(() => {
        if (trailing) clearTimeout(trailing);
    });

    return (
        <Show
            when={isCanceled()}
            fallback={
                <div
                    class={clsx("agent-markdown-block", {
                        "thinking-block": props.node.metadata?.thinking,
                    })}
                >
                    <Markdown text={view().text} highlight={view().highlight} />
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

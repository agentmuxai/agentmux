// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Markdown } from "@/app/element/markdown";
import clsx from "clsx";
import { createSignal, Show, type JSX } from "solid-js";
import type { MarkdownNode } from "../types";

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

    return (
        <Show
            when={isCanceled()}
            fallback={
                <div
                    class={clsx("agent-markdown-block", {
                        "thinking-block": props.node.metadata?.thinking,
                    })}
                >
                    <Markdown text={props.node.content} />
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

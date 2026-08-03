// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock - Renders markdown content from agent output
 */

import { Markdown } from "@/app/element/markdown";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import { createEffect, createMemo, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import type { MarkdownNode } from "../types";
import { PeekOverlay } from "./PeekOverlay";

// 150ms enter-delay matches UserMessageBlock's hover-to-peek — prevents
// accidental expansions during fast scroll-throughs.
const PEEK_ENTER_DELAY_MS = 150;

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
    // Value-based equality: `Markdown` subscribes to this signal via the
    // `highlight` prop, so a fresh-but-equal object would needlessly re-parse
    // static / history blocks on mount (and on the trailing no-op write).
    // With this, a same-value setView is a no-op.
    const [view, setView] = createSignal<{ text: string; highlight: boolean }>(
        { text: props.node.content, highlight: true },
        { equals: (a, b) => a.text === b.text && a.highlight === b.highlight },
    );
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

    // Thinking-clump peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md
    // §2.4). Mirrors ToolBlock.tsx's time + estimate pattern exactly. No
    // duration line — deriving one from the next node's timestamp needs a new
    // prop threaded down through the virtualization list's windowed-rows/
    // streaming-buffer machinery (AgentDocumentVirtualList.tsx), which is
    // performance-critical and carefully tuned; deferred as a follow-up
    // rather than risked here for a nice-to-have (§4 resolution 1 of that
    // spec still calls for it eventually).
    // reagent P2 on PR #2392 (1st round): short-circuit BEFORE reading
    // `peekTick()` for every non-thinking block (the far more common node
    // kind) — a memo only subscribes to what it actually reads during a
    // given run, so returning early here means regular assistant text never
    // subscribes to the shared 1s ticker at all, instead of silently
    // recomputing a value nothing ever renders.
    //
    // reagent P2 on PR #2392 (3rd round): the thinking-kind check alone
    // isn't enough — every MOUNTED thinking clump still called peekTick()
    // regardless of hover state, the same defect ToolBlock.tsx fixed via an
    // explicit isPeeking signal. Mirror that fix here: only read peekTick()
    // while this block is actually being hovered.
    const peekTick = useTick(1000);
    const [isPeeking, setIsPeeking] = createSignal(false);
    const peekTimeText = createMemo(() => {
        if (!props.node.metadata?.thinking) return null;
        if (!isPeeking()) return null;
        peekTick();
        const ts = props.node.timestamp;
        if (ts == null) return null;
        return `${formatExactTime(ts)} · ${formatTimeAgo(ts)}`;
    });
    const peekEstimateText = createMemo(() => {
        if (!props.node.metadata?.thinking) return null;
        const count = estimateTokenCount(props.node.content);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });

    // Peek overlay — Portal-rendered (see PeekOverlay.tsx's doc comment for
    // why: each virtualized row is its own CSS stacking context, so a plain
    // `position: absolute` child can never paint above a LATER row no
    // matter its z-index). Styled and positioned like UserMessageBlock.tsx's
    // "Session context" hover-to-peek, reusing the same `hover-anchor.ts`
    // direction logic — just rendered outside the row's subtree.
    let peekEnterTimer: ReturnType<typeof setTimeout> | undefined;
    let rowEl: HTMLDivElement | undefined;

    const handlePeekEnter = () => {
        clearTimeout(peekEnterTimer);
        peekEnterTimer = setTimeout(() => setIsPeeking(true), PEEK_ENTER_DELAY_MS);
    };
    const handlePeekLeave = () => {
        clearTimeout(peekEnterTimer);
        setIsPeeking(false);
    };
    onCleanup(() => clearTimeout(peekEnterTimer));

    return (
        <Show
            when={isCanceled()}
            fallback={
                <Show
                    when={props.node.metadata?.thinking}
                    fallback={
                        <div class="agent-markdown-block">
                            {/* scrollable={false}: agent markdown streams (reactive). With
                                scrollable, OverlayScrollbars relocates SolidJS's children into
                                its viewport, so the next streaming reconcile calls replaceChild
                                on a node it has moved → the long-standing replaceChild crash
                                (#1326). Per-block scroll is also wrong inside the virtualized
                                document, which owns the scroll. */}
                            <Markdown text={view().text} highlight={view().highlight} scrollable={false} />
                        </div>
                    }
                >
                    <div
                        ref={(el) => (rowEl = el)}
                        class="agent-thinking-peek-anchor"
                        onMouseEnter={handlePeekEnter}
                        onMouseLeave={handlePeekLeave}
                    >
                        <div class="agent-markdown-block thinking-block">
                            <Markdown text={view().text} highlight={view().highlight} scrollable={false} />
                        </div>
                        {/* Peek overlay — see ToolBlock.tsx's identical pattern
                            and PeekOverlay.tsx. Not gated on an `expanded()`
                            check here — thinking clumps have no pin/expand
                            state to collide with. */}
                        <PeekOverlay
                            show={isPeeking() && (peekTimeText() != null || peekEstimateText() != null)}
                            rowEl={() => rowEl}
                        >
                            <Show when={peekTimeText()}>
                                <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div>
                            </Show>
                            <Show when={peekEstimateText()}>
                                <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div>
                            </Show>
                        </PeekOverlay>
                    </div>
                </Show>
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
                        <Markdown text={props.node.content} scrollable={false} />
                    </div>
                </Show>
            </div>
        </Show>
    );
};

MarkdownBlock.displayName = "MarkdownBlock";

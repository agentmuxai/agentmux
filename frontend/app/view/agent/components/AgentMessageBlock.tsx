// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMessageBlock - Displays agent-to-agent communication (mux/ject)
 */

import clsx from "clsx";
import { Show, createMemo, type JSX } from "solid-js";
import type { AgentMessageNode } from "../types";
import { LinkifiedText } from "@/app/element/linkified-text";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import { useTick } from "@/app/hook/useTick";
import { useNodePeek } from "../hooks/useNodePeek";
import { PeekOverlay } from "./PeekOverlay";

interface AgentMessageBlockProps {
    node: AgentMessageNode;
    collapsed: boolean;
    onToggle: () => void;
}

export const AgentMessageBlock = (props: AgentMessageBlockProps): JSX.Element => {
    // Don't destructure — the streaming buffer keeps this row mounted
    // across token deltas; useAgentStream replaces props.node ref on
    // each chunk. Destructured `node` would freeze at first ref.
    // Access props.X reactively at each site. (codex P1 on PR #786 +
    // family of issues on virt redesign — also fixed in MarkdownBlock.)

    // Peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25) —
    // this node type never surfaces its own timestamp anywhere, collapsed
    // or expanded, so the peek isn't gated on `props.collapsed` (unlike
    // ToolBlock's panel, expanding this block wouldn't make the peek
    // redundant — the expanded view has no time at all).
    const peekTick = useTick(1000);
    const { isPeeking, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } = useNodePeek();
    const peekTimeText = createMemo(() => {
        if (!isPeeking()) return null;
        peekTick();
        return `${formatExactTime(props.node.timestamp)} · ${formatTimeAgo(props.node.timestamp)}`;
    });
    const peekEstimateText = createMemo(() => {
        const count = estimateTokenCount(props.node.message);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });

    return (
        <div
            ref={setPeekRowEl}
            class={clsx("agent-message-block", {
                incoming: props.node.direction === "incoming",
                outgoing: props.node.direction !== "incoming",
                collapsed: props.collapsed,
                mux: props.node.method === "mux",
                ject: props.node.method === "ject",
            })}
            onClick={props.onToggle}
            onMouseEnter={handlePeekEnter}
            onMouseLeave={handlePeekLeave}
        >
            <div class="agent-message-summary">
                <span class="agent-message-chevron">{props.collapsed ? "▸" : "▾"}</span>
                <span class="agent-message-icon">{props.node.summary}</span>
            </div>
            <Show when={!props.collapsed}>
                <div class="agent-message-content" onClick={(e) => e.stopPropagation()}>
                    <div class="agent-message-meta">
                        <span class="agent-message-from">From: {props.node.from}</span>
                        <span class="agent-message-to">To: {props.node.to}</span>
                        <span class="agent-message-method">Method: {props.node.method}</span>
                    </div>
                    <pre class="agent-message-body">
                        <LinkifiedText text={props.node.message} />
                    </pre>
                </div>
            </Show>
            <PeekOverlay show={isPeeking()} rowEl={peekRowEl}>
                <Show when={peekTimeText()}>
                    <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div>
                </Show>
                <Show when={peekEstimateText()}>
                    <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div>
                </Show>
            </PeekOverlay>
        </div>
    );
};

AgentMessageBlock.displayName = "AgentMessageBlock";

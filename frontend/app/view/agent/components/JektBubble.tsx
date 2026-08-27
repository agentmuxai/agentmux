// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * JektBubble — agent-pane rendering of a muxbus jekt message.
 *
 * Replaces the raw `[JEKT:FROM=... TIER=... ...]` marker text with a
 * distinct, labeled block so the human operator can always tell a jekt
 * apart from a typed user message or agent output (design goals G1/G2).
 *
 * Collapsed by default, like AgentMessageBlock: a one-line summary showing
 * direction, sender/recipient, and the tier + delivery badges. Clicking
 * expands to show the message body plus metadata (full MSGID, timestamp,
 * raw marker payload) — spec §3.3's "click the bubble shows metadata".
 *
 * Spec: docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §3.3.
 */

import clsx from "clsx";
import { Show, createMemo, type JSX } from "solid-js";
import type { JektMessageNode } from "../types";
import { JEKT_DELIVERY_ICONS, JEKT_TIER_ICONS } from "../types";
import { LinkifiedText } from "@/app/element/linkified-text";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import { useTick } from "@/app/hook/useTick";
import { useNodePeek } from "../hooks/useNodePeek";
import { PeekOverlay } from "./PeekOverlay";

interface JektBubbleProps {
    node: JektMessageNode;
    collapsed: boolean;
    onToggle: () => void;
}

function formatTimestamp(ts: number): string {
    return new Date(ts).toLocaleString();
}

export const JektBubble = (props: JektBubbleProps): JSX.Element => {
    // Don't destructure props — see AgentMessageBlock/MarkdownBlock
    // for why (codex P1 on PR #786 + family of virt-redesign issues).
    // Same reactivity discipline applies here.

    // Peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25).
    // Not gated on `props.collapsed` — the expanded view's own timestamp
    // line is plain `toLocaleString()`, no relative "ago"; the peek adds
    // that even when already expanded.
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
            class={clsx("agent-jekt-bubble", {
                incoming: props.node.direction === "incoming",
                outgoing: props.node.direction === "outgoing",
                collapsed: props.collapsed,
                [`tier-${props.node.tier}`]: true,
            })}
            onClick={props.onToggle}
            onMouseEnter={handlePeekEnter}
            onMouseLeave={handlePeekLeave}
        >
            <div class="agent-jekt-summary">
                <span class="agent-jekt-chevron">{props.collapsed ? "▸" : "▾"}</span>
                <span class="agent-jekt-direction-icon">
                    {props.node.direction === "incoming" ? "📥" : "📤"}
                </span>
                <span class="agent-jekt-peer">
                    {props.node.direction === "incoming"
                        ? `From ${props.node.from}`
                        : `To ${props.node.to}`}
                </span>
                <span class="agent-jekt-tier-badge" title={`Tier: ${props.node.tier}`}>
                    {JEKT_TIER_ICONS[props.node.tier]} {props.node.tier}
                </span>
                <span
                    class="agent-jekt-delivery-badge"
                    title={`Delivery: ${props.node.deliveryTier} (${props.node.trust})`}
                >
                    {JEKT_DELIVERY_ICONS[props.node.deliveryTier]} {props.node.deliveryTier}
                </span>
            </div>
            <Show when={!props.collapsed}>
                <div class="agent-jekt-content" onClick={(e) => e.stopPropagation()}>
                    <pre class="agent-jekt-body">
                        <LinkifiedText text={props.node.message} />
                    </pre>
                    <div class="agent-jekt-meta">
                        <span class="agent-jekt-meta-item">From: {props.node.from}</span>
                        <span class="agent-jekt-meta-item">To: {props.node.to}</span>
                        <span class="agent-jekt-meta-item">MSGID: {props.node.msgId || "—"}</span>
                        <span class="agent-jekt-meta-item">Trust: {props.node.trust}</span>
                        <span class="agent-jekt-meta-item">Priority: {props.node.priority}</span>
                        <span class="agent-jekt-meta-item">{formatTimestamp(props.node.timestamp)}</span>
                    </div>
                    <details class="agent-jekt-raw">
                        <summary>Raw payload</summary>
                        <pre>{props.node.raw}</pre>
                    </details>
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

JektBubble.displayName = "JektBubble";

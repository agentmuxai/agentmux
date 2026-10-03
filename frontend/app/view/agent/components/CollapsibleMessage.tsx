// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CollapsibleMessage — the shell AgentMessageBlock and JektBubble share
 * (SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §1): a row that toggles on
 * click, a summary line led by a ▸/▾ chevron, a body shown when open (clicks
 * inside it don't toggle), and the time + token peek on hover.
 *
 * Class names come from the caller (`rootClass`, and `classPrefix` for
 * `-summary` / `-chevron` / `-content`) so each message kind keeps its own
 * styling hooks.
 *
 * Don't destructure props: the streaming buffer keeps these rows mounted
 * across updates and replaces `props.node`-derived values in place; a
 * destructured prop freezes at mount (codex P1 on PR #786).
 */

import clsx from "clsx";
import { Show, createMemo, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import { useNodePeek } from "../hooks/useNodePeek";
import { PeekOverlay } from "./PeekOverlay";
import { PeekMetaRow } from "./PeekMetaRow";

interface CollapsibleMessageProps {
    /** The root's own class, e.g. "agent-jekt-bubble". */
    rootClass: string;
    /** Prefix for the `-summary`, `-chevron` and `-content` elements. */
    classPrefix: string;
    /** Extra root classes (direction, tier, method, …). */
    classes?: Record<string, boolean>;
    collapsed: boolean;
    onToggle: () => void;
    /** The summary line after the chevron. */
    summary: JSX.Element;
    /** The body, shown when open. */
    body: JSX.Element;
    /** What the peek's token estimate counts (the message text). */
    peekText: string;
    timestamp: number;
}

export const CollapsibleMessage = (props: CollapsibleMessageProps): JSX.Element => {
    // Peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25). Not
    // gated on `collapsed`: neither message kind shows a relative time when
    // expanded, so the peek adds it either way.
    const peekTick = useTick(1000);
    const { isPeeking, panelVisible: peekPanelVisible, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } = useNodePeek();
    const peekTimeText = createMemo(() => {
        if (!peekPanelVisible()) return null;
        peekTick();
        return `${formatExactTime(props.timestamp)} · ${formatTimeAgo(props.timestamp)}`;
    });
    const peekEstimateText = createMemo(() => {
        const count = estimateTokenCount(props.peekText);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });

    return (
        <div
            ref={setPeekRowEl}
            class={clsx(props.rootClass, props.classes, { collapsed: props.collapsed })}
            onClick={props.onToggle}
            onMouseEnter={handlePeekEnter}
            onMouseLeave={handlePeekLeave}
        >
            <div class={`${props.classPrefix}-summary`}>
                <span class={`${props.classPrefix}-chevron`}>{props.collapsed ? "▸" : "▾"}</span>
                {props.summary}
            </div>
            <Show when={!props.collapsed}>
                <div class={`${props.classPrefix}-content`} onClick={(e) => e.stopPropagation()}>
                    {props.body}
                </div>
            </Show>
            <PeekOverlay show={isPeeking()} rowEl={peekRowEl}>
                <PeekMetaRow time={peekTimeText()} tokens={peekEstimateText()} />
            </PeekOverlay>
        </div>
    );
};

CollapsibleMessage.displayName = "CollapsibleMessage";

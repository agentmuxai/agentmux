// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UserMessageBlock — agent-pane row for the human's own input: the inverted turn style
 * (`styles/_human-turn.scss`), always expanded, `<pre>` content word-wrapped, highest-contrast
 * colour, and a hover-to-peek time/estimate overlay (PeekOverlay), same as tool calls and
 * thinking clumps (2026-08-03 user request). A Swarm broadcast, a Supervisor continuation and
 * a message from a paired device carry a one-line label above the words.
 *
 * AgentMux's own setup prompts (the start-up document, the opening update) are not the person's
 * words and do not come here: they are a SetupNote (`node.isStartup`).
 */

import { LinkifiedText } from "@/app/element/linkified-text";
import { useTick } from "@/app/hook/useTick";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import clsx from "clsx";
import { Show, createMemo, type JSX } from "solid-js";
import { AttachmentStrip } from "../attachments/AttachmentStrip";
import { useNodePeek } from "../hooks/useNodePeek";
import type { UserMessageNode } from "../types";
import { PeekMetaRow } from "./PeekMetaRow";
import { PeekOverlay } from "./PeekOverlay";

interface UserMessageBlockProps {
    node: UserMessageNode;
}

/** "Broadcast · 7 agents", or just "Broadcast" when the count is unknown or 1. */
function broadcastNoteLabel(broadcast: { recipients?: number }): string {
    const n = broadcast.recipients;
    return n != null && n > 1 ? `Broadcast · ${n} agents` : "Broadcast";
}

export const UserMessageBlock = (props: UserMessageBlockProps): JSX.Element => {
    // Peek metadata (time + token estimate), same treatment ToolBlock.tsx / MarkdownBlock.tsx have.
    const peekTick = useTick(1000);
    // One element reference, owned by the hook, so its drag-release hover resync can fire (reagentx P1 on #3470).
    const {
        isPeeking,
        panelVisible: peekPanelVisible,
        rowEl: peekRowEl,
        setRowEl: setPeekRowEl,
        handlePeekEnter,
        handlePeekLeave,
    } = useNodePeek();

    const peekTimeText = createMemo(() => {
        if (!peekPanelVisible()) return null;
        peekTick(); // re-run every second so "ago" stays live while hovered
        const ts = props.node.timestamp;
        if (ts == null) return null;
        return `${formatExactTime(ts)} · ${formatTimeAgo(ts)}`;
    });
    const peekEstimateText = createMemo(() => {
        const count = estimateTokenCount(props.node.message);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });

    return (
        <div
            ref={setPeekRowEl}
            class={clsx("agent-user-message")}
            onMouseEnter={handlePeekEnter}
            onMouseLeave={handlePeekLeave}
        >
            <div class="agent-user-message-content agent-user-message-content--flow">
                <Show when={props.node.broadcast}>
                    <div
                        class="agent-user-message-broadcast-note"
                        title="Typed once in Swarm and sent to several agents"
                    >
                        <i class="fa-solid fa-bullhorn" aria-hidden="true" />{" "}
                        <span>{broadcastNoteLabel(props.node.broadcast!)}</span>
                    </div>
                </Show>
                <Show when={props.node.supervisor}>
                    {(s) => (
                        <div
                            class="agent-user-message-broadcast-note"
                            title="Sent by the Swarm Supervisor to keep this agent going"
                        >
                            <i class="fa-solid fa-user-shield" aria-hidden="true" />{" "}
                            <span>
                                Supervisor · {s().mode === "drive" ? "Drive" : "Steady"} · {s().n} of {s().of}
                            </span>
                        </div>
                    )}
                </Show>
                <Show when={props.node.fromDevice}>
                    <div class="agent-user-message-broadcast-note" title="Sent from a paired device">
                        <i class="fa-solid fa-mobile-screen" aria-hidden="true" />{" "}
                        <span>From {props.node.fromDevice}</span>
                    </div>
                </Show>
                <Show when={props.node.attachments?.length}>
                    <AttachmentStrip attachments={props.node.attachments!} />
                </Show>
                <Show when={props.node.message}>
                    <pre>
                        <LinkifiedText text={props.node.message} />
                    </pre>
                </Show>
            </div>
            {/* Time + token estimate on hover; the message itself is always in flow above. */}
            <PeekOverlay show={isPeeking() && (peekTimeText() != null || peekEstimateText() != null)} rowEl={peekRowEl}>
                <PeekMetaRow time={peekTimeText()} tokens={peekEstimateText()} />
            </PeekOverlay>
        </div>
    );
};

UserMessageBlock.displayName = "UserMessageBlock";

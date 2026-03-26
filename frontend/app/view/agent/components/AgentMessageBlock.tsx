// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMessageBlock - Displays agent-to-agent communication (mux/ject)
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import type { AgentMessageNode } from "../types";

interface AgentMessageBlockProps {
    node: AgentMessageNode;
    collapsed: boolean;
    onToggle: () => void;
}

export const AgentMessageBlock = ({ node, collapsed, onToggle }: AgentMessageBlockProps): JSX.Element => {
    const isIncoming = node.direction === "incoming";
    const timestamp = new Date(node.timestamp).toLocaleTimeString();

    return (
        <div
            class={clsx("agent-message-block", {
                incoming: isIncoming,
                outgoing: !isIncoming,
                collapsed,
                mux: node.method === "mux",
                ject: node.method === "ject",
            })}
            onClick={onToggle}
        >
            <div class="agent-message-summary">
                <span class="agent-message-chevron">{collapsed ? "▸" : "▾"}</span>
                <span class="agent-message-icon">{node.summary}</span>
                <span class="agent-message-time">{timestamp}</span>
            </div>
            <Show when={!collapsed}>
                <div class="agent-message-content" onClick={(e) => e.stopPropagation()}>
                    <div class="agent-message-meta">
                        <span class="agent-message-from">From: {node.from}</span>
                        <span class="agent-message-to">To: {node.to}</span>
                        <span class="agent-message-method">Method: {node.method}</span>
                    </div>
                    <pre class="agent-message-body">{node.message}</pre>
                </div>
            </Show>
        </div>
    );
};

AgentMessageBlock.displayName = "AgentMessageBlock";

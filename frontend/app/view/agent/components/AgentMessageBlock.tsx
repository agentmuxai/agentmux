// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMessageBlock - Displays agent-to-agent communication (mux/ject)
 */

import clsx from "clsx";
import React, { memo } from "react";
import type { AgentMessageNode } from "../types";

interface AgentMessageBlockProps {
    node: AgentMessageNode;
    collapsed: boolean;
    onToggle: () => void;
}

export const AgentMessageBlock: React.FC<AgentMessageBlockProps> = memo(({ node, collapsed, onToggle }) => {
    const isIncoming = node.direction === "incoming";
    const timestamp = new Date(node.timestamp).toLocaleTimeString();

    return (
        <div
            className={clsx("agent-message-block", {
                incoming: isIncoming,
                outgoing: !isIncoming,
                collapsed,
                mux: node.method === "mux",
                ject: node.method === "ject",
            })}
            onClick={onToggle}
        >
            <div className="agent-message-summary">
                <span className="agent-message-chevron">{collapsed ? "▸" : "▾"}</span>
                <span className="agent-message-icon">{node.summary}</span>
                <span className="agent-message-time">{timestamp}</span>
            </div>
            {!collapsed && (
                <div className="agent-message-content" onClick={(e) => e.stopPropagation()}>
                    <div className="agent-message-meta">
                        <span className="agent-message-from">From: {node.from}</span>
                        <span className="agent-message-to">To: {node.to}</span>
                        <span className="agent-message-method">Method: {node.method}</span>
                    </div>
                    <pre className="agent-message-body">{node.message}</pre>
                </div>
            )}
        </div>
    );
});

AgentMessageBlock.displayName = "AgentMessageBlock";

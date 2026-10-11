// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMessageBlock - Displays agent-to-agent communication (mux/ject).
 * The row, toggle, chevron and peek come from CollapsibleMessage.
 */

import { type JSX } from "solid-js";
import type { AgentMessageNode } from "../types";
import { LinkifiedText } from "@/app/element/linkified-text";
import { CollapsibleMessage } from "./CollapsibleMessage";

interface AgentMessageBlockProps {
    node: AgentMessageNode;
    collapsed: boolean;
    onToggle: () => void;
}

export const AgentMessageBlock = (props: AgentMessageBlockProps): JSX.Element => (
    // Don't destructure — the streaming buffer keeps this row mounted across
    // token deltas; useAgentStream replaces props.node on each chunk.
    <CollapsibleMessage
        rootClass="agent-message-block"
        classPrefix="agent-message"
        classes={{
            incoming: props.node.direction === "incoming",
            outgoing: props.node.direction !== "incoming",
            mux: props.node.method === "mux",
            ject: props.node.method === "ject",
        }}
        collapsed={props.collapsed}
        onToggle={props.onToggle}
        peekText={props.node.message}
        timestamp={props.node.timestamp}
        summary={<span class="agent-message-icon">{props.node.summary}</span>}
        body={
            <>
                <div class="agent-message-meta">
                    <span class="agent-message-from">From: {props.node.from}</span>
                    <span class="agent-message-to">To: {props.node.to}</span>
                    <span class="agent-message-method">Method: {props.node.method}</span>
                </div>
                <pre class="agent-message-body">
                    <LinkifiedText text={props.node.message} />
                </pre>
            </>
        }
    />
);

AgentMessageBlock.displayName = "AgentMessageBlock";

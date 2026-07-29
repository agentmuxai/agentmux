// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMessageBlock - Displays agent-to-agent communication (mux/ject)
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import type { AgentMessageNode } from "../types";
import { LinkifiedText } from "@/app/element/linkified-text";

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
    return (
        <div
            class={clsx("agent-message-block", {
                incoming: props.node.direction === "incoming",
                outgoing: props.node.direction !== "incoming",
                collapsed: props.collapsed,
                mux: props.node.method === "mux",
                ject: props.node.method === "ject",
            })}
            onClick={props.onToggle}
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
        </div>
    );
};

AgentMessageBlock.displayName = "AgentMessageBlock";

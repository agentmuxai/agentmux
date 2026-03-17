// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SubagentLinkBlock — Renders a clickable subagent link in the agent pane.
 * When clicked, opens a subagent activity pane split from the parent block.
 */

import clsx from "clsx";
import type { JSX } from "solid-js";
import type { SubagentLinkNode } from "../types";

interface SubagentLinkBlockProps {
    node: SubagentLinkNode;
    onClick: (node: SubagentLinkNode) => void;
}

export const SubagentLinkBlock = ({ node, onClick }: SubagentLinkBlockProps): JSX.Element => {
    const isActive = () => node.status === "active";

    return (
        <div
            class={clsx("agent-subagent-link", {
                active: isActive(),
                completed: !isActive(),
            })}
            onClick={() => onClick(node)}
        >
            <span class="agent-subagent-link-icon">{isActive() ? "\u{26A1}" : "\u2714"}</span>
            <span class="agent-subagent-link-info">
                <span class="agent-subagent-link-slug">{node.slug || node.subagentId.substring(0, 7)}</span>
                <span class="agent-subagent-link-id">{node.subagentId.substring(0, 7)}</span>
            </span>
            <span class="agent-subagent-link-action">{"\u2192"}</span>
        </div>
    );
};

SubagentLinkBlock.displayName = "SubagentLinkBlock";

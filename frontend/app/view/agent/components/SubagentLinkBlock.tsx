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

export const SubagentLinkBlock = (props: SubagentLinkBlockProps): JSX.Element => {
    // Don't destructure -- see family of fixes on virt redesign:
    // AgentMessageBlock, MarkdownBlock have the same change. The
    // streaming buffer's Index keeps the row mounted across status
    // transitions (active to completed); a destructured node would
    // freeze the active class. (codex P2 on PR #786.)
    const isActive = () => props.node.status === "active";

    return (
        <div
            class={clsx("agent-subagent-link", {
                active: isActive(),
                completed: !isActive(),
            })}
            onClick={() => props.onClick(props.node)}
        >
            <span class="agent-subagent-link-icon">{isActive() ? "\u{26A1}" : "\u2714"}</span>
            <span class="agent-subagent-link-info">
                <span class="agent-subagent-link-slug">{props.node.slug || props.node.subagentId.substring(0, 7)}</span>
                <span class="agent-subagent-link-id">{props.node.subagentId.substring(0, 7)}</span>
            </span>
            <span class="agent-subagent-link-action">{"\u2192"}</span>
        </div>
    );
};

SubagentLinkBlock.displayName = "SubagentLinkBlock";

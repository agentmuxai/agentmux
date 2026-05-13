// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolOverlayActions — bottom action bar of the tool overlay
 * (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4 + §4).
 *
 * Hosts the branching actions that used to live in `NodeHoverStrip`:
 * bookmark, open-in-pane, open-in-window, new-agent-here. Branching
 * actions belong at the bottom of the log so the user sees output
 * first, options second — matches every native log viewer (terminal,
 * VSCode output panel, browser DevTools console).
 *
 * Phase 3 wires bookmark + open-in-pane to real handlers; open-in-window
 * and new-agent-here remain stubs with explanatory tooltips until the
 * host APIs land (open-in-window) and a backend RPC exists
 * (new-agent-here).
 */

import { Show, type JSX } from "solid-js";
import type { ToolNode } from "../types";

interface ToolOverlayActionsProps {
    node: ToolNode;
    isBookmarked?: boolean;
    onBookmark?: () => void;
    onOpenInPane?: () => void;
    onOpenInWindow?: () => void;
    onNewAgentHere?: () => void;
}

interface ActionButtonProps {
    icon: string;
    label: string;
    title?: string;
    disabled?: boolean;
    active?: boolean;
    onClick?: () => void;
}

const ActionButton = (props: ActionButtonProps): JSX.Element => (
    <button
        type="button"
        class="agent-tool-overlay-action"
        classList={{
            "agent-tool-overlay-action--active": props.active === true,
            "agent-tool-overlay-action--disabled": props.disabled === true,
        }}
        disabled={props.disabled}
        title={props.title ?? props.label}
        aria-label={props.label}
        onClick={(e) => {
            e.stopPropagation();
            props.onClick?.();
        }}
    >
        <span class="agent-tool-overlay-action-icon">{props.icon}</span>
        <span class="agent-tool-overlay-action-label">{props.label}</span>
    </button>
);

export const ToolOverlayActions = (props: ToolOverlayActionsProps): JSX.Element => (
    <div class="agent-tool-overlay-actions" data-node-id={props.node.id}>
        <Show when={props.onBookmark}>
            <ActionButton
                icon="🔖"
                label={props.isBookmarked ? "Bookmarked" : "Bookmark"}
                active={props.isBookmarked === true}
                onClick={props.onBookmark}
            />
        </Show>
        <Show when={props.onOpenInPane}>
            <ActionButton
                icon="⧉"
                label="Open in pane"
                onClick={props.onOpenInPane}
            />
        </Show>
        <ActionButton
            icon="⊡"
            label="Open in window"
            title={
                props.onOpenInWindow
                    ? "Open in new window"
                    : "Coming soon — host API not yet shipped"
            }
            disabled={props.onOpenInWindow == null}
            onClick={props.onOpenInWindow}
        />
        <ActionButton
            icon="✦"
            label="New agent here"
            title={
                props.onNewAgentHere
                    ? "Spawn a sibling agent seeded with this context"
                    : "Coming soon — backend RPC not yet shipped"
            }
            disabled={props.onNewAgentHere == null}
            onClick={props.onNewAgentHere}
        />
    </div>
);

ToolOverlayActions.displayName = "ToolOverlayActions";

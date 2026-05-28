// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlockOverlay — three-slot tool overlay
 * (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4):
 *
 *   ┌────────────────────────────────────┐
 *   │ header   (sticky)                  │
 *   ├────────────────────────────────────┤
 *   │ log body (scrollable, virtualized) │
 *   │   ...                              │
 *   ├────────────────────────────────────┤
 *   │ action bar (fixed)                 │
 *   └────────────────────────────────────┘
 *
 * Per SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md the header was
 * simplified to two slots: timestamp on the left, status label on the
 * right. The status icon, tool name, summary, and duration were dropped
 * because the collapsed row above already displays all four — the
 * earlier header was pure duplication. The "small time popup" the user
 * was seeing on hover (a browser-native `title=` tooltip whose contents
 * included `(N.Ns)`) is also gone with this change: the time now lives
 * here, persistent, at the top of the unified panel.
 */

import { type JSX } from "solid-js";
import type { ToolNode } from "../types";
import { ToolOverlayActions } from "./ToolOverlayActions";
import { ToolOverlayLog } from "./ToolOverlayLog";

export interface ToolBlockOverlayProps {
    node: ToolNode;
    isBookmarked?: boolean;
    onBookmark?: () => void;
    onOpenInPane?: () => void;
    onOpenInWindow?: () => void;
    onNewAgentHere?: () => void;
}

const STATUS_LABEL: Record<ToolNode["status"], string> = {
    running: "running",
    pending_approval: "awaiting approval",
    success: "ok",
    failed: "failed",
    denied: "denied",
    canceled: "canceled",
};

/**
 * Local-time `HH:MM:SS` for the header timestamp. Minutes precision is
 * too coarse for distinguishing rapid tool sequences in a Bash chain;
 * seconds gives the user enough to correlate against their own clock
 * without dominating the visual layout.
 */
function formatToolTime(ms: number | undefined): string {
    if (ms == null) return "";
    return new Date(ms).toLocaleTimeString(undefined, { hour12: false });
}

export const ToolBlockOverlay = (props: ToolBlockOverlayProps): JSX.Element => (
    <div class="agent-tool-overlay" data-node-id={props.node.id}>
        <div class="agent-tool-overlay-header">
            <span class="agent-tool-overlay-time">
                {formatToolTime(props.node.timestamp)}
            </span>
            <span class="agent-tool-overlay-status-label">
                {STATUS_LABEL[props.node.status]}
            </span>
        </div>
        <ToolOverlayLog node={props.node} />
        <ToolOverlayActions
            node={props.node}
            isBookmarked={props.isBookmarked}
            onBookmark={props.onBookmark}
            onOpenInPane={props.onOpenInPane}
            onOpenInWindow={props.onOpenInWindow}
            onNewAgentHere={props.onNewAgentHere}
        />
    </div>
);

ToolBlockOverlay.displayName = "ToolBlockOverlay";

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
 * Replaces the prior inline `renderToolContent()` switch in `ToolBlock`.
 * The log slot uses `ToolOverlayLog` which falls back to per-tool rich
 * result content when no streaming chunks are present (graceful for the
 * Phase 3 ship — chunks start flowing in Phase 2's backend wrap).
 */

import { Show, type JSX } from "solid-js";
import type { ToolNode } from "../types";
import { STATUS_ICONS } from "../types";
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
};

export const ToolBlockOverlay = (props: ToolBlockOverlayProps): JSX.Element => (
    <div class="agent-tool-overlay" data-node-id={props.node.id}>
        <div class="agent-tool-overlay-header">
            <span class="agent-tool-overlay-status">
                {STATUS_ICONS[props.node.status] ?? "•"}
            </span>
            <span class="agent-tool-overlay-tool">{props.node.tool}</span>
            <span class="agent-tool-overlay-summary" title={props.node.summary}>
                {props.node.summary}
            </span>
            <span class="agent-tool-overlay-meta">
                <Show when={props.node.duration}>
                    <span class="agent-tool-overlay-duration">
                        {props.node.duration!.toFixed(1)}s
                    </span>
                </Show>
                <span class="agent-tool-overlay-status-label">
                    {STATUS_LABEL[props.node.status]}
                </span>
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

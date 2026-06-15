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
 * simplified: the status icon, tool name, summary, and duration are on
 * the collapsed row above, so the header carries only the status label.
 * The on-hover timestamp slot was later removed. The header is omitted
 * while running (the body's "Thinking…" spinner conveys that) and on
 * success (the collapsed row already shows the outcome) — so it appears
 * only for failed / denied / canceled / awaiting-approval statuses.
 */

import { type JSX } from "solid-js";
import type { ToolNode } from "../types";
import { ToolOverlayActions } from "./ToolOverlayActions";
import { ToolOverlayLog } from "./ToolOverlayLog";

export interface ToolBlockOverlayProps {
    node: ToolNode;
    onOpenInPane?: () => void;
    onOpenInWindow?: () => void;
    onNewAgentHere?: () => void;
}

const STATUS_LABEL: Record<ToolNode["status"], string> = {
    running: "running",
    pending_approval: "awaiting approval",
    awaiting_answer: "awaiting answer",
    success: "ok",
    failed: "failed",
    denied: "denied",
    canceled: "canceled",
};

export const ToolBlockOverlay = (props: ToolBlockOverlayProps): JSX.Element => (
    <div class="agent-tool-overlay" data-node-id={props.node.id}>
        <div
            class="agent-tool-overlay-header"
            style={{
                display:
                    props.node.status === "running" || props.node.status === "success"
                        ? "none"
                        : "",
            }}
        >
            <span class="agent-tool-overlay-status-label">
                {STATUS_LABEL[props.node.status]}
            </span>
        </div>
        <ToolOverlayLog node={props.node} />
        <ToolOverlayActions
            node={props.node}
            onOpenInPane={props.onOpenInPane}
            onOpenInWindow={props.onOpenInWindow}
            onNewAgentHere={props.onNewAgentHere}
        />
    </div>
);

ToolBlockOverlay.displayName = "ToolBlockOverlay";

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlockOverlay — two-slot tool overlay
 * (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4):
 *
 *   ┌────────────────────────────────────┐
 *   │ header   (sticky)                  │
 *   ├────────────────────────────────────┤
 *   │ log body (scrollable, virtualized) │
 *   │   ...                              │
 *   └────────────────────────────────────┘
 *
 * The action bar (open-in-pane / open-in-window / new-agent-here) was
 * removed — all three were non-functional stubs (open-in-pane just
 * console.warn'd, the other two were permanently disabled pending
 * host APIs / backend RPCs that never shipped).
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
import type { AgentDispatch } from "../../swarm/swarm-model";
import type { ToolNode } from "../types";
import { TOOL_STATUS } from "../tool-meta/tool-status";
import { ToolOverlayLog } from "./ToolOverlayLog";

export interface ToolBlockOverlayProps {
    node: ToolNode;
    /** Ordinal-matched live dispatch for an Agent/Task/Workflow tool call —
     *  see `activity/dispatch-correlation.ts`. */
    dispatchMatch?: AgentDispatch;
}

export const ToolBlockOverlay = (props: ToolBlockOverlayProps): JSX.Element => (
    <div class="agent-tool-overlay" data-node-id={props.node.id}>
        <div
            class="agent-tool-overlay-header agent-tool-summary-fade-in"
            style={{
                display:
                    props.node.status === "running" || props.node.status === "success"
                        ? "none"
                        : "",
            }}
        >
            <span class="agent-tool-overlay-status-label">
                {TOOL_STATUS[props.node.status]?.label}
            </span>
        </div>
        <ToolOverlayLog node={props.node} dispatchMatch={props.dispatchMatch} />
    </div>
);

ToolBlockOverlay.displayName = "ToolBlockOverlay";

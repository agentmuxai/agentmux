// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The result pill on a ToolBlock row: a compact inline summary shown at medium and wider pane
 * widths (hidden by a CSS container query below that; always rendered so the DOM is stable when
 * the pane is resized through the breakpoint).
 */

import type { AgentDispatch } from "../../swarm/swarm-model";
import type { ToolNode } from "../../../agent-model/types";
import { TOOL_STATUS } from "../../../agent-model/tool-status";
import { toolPill } from "../../../agent-model/tool-descriptors";

export interface ResultPill {
    label: string;
    variant: string;
}

/** A result unloaded from memory still counts as present; opening the row reads it back from the transcript. */
export const hasToolResult = (node: ToolNode): boolean => node.result != null || node.resultUnloaded != null;

/** The live dispatch's pill for an Agent, Task or Workflow call. */
function dispatchPill(node: ToolNode, d: AgentDispatch | undefined): ResultPill | null {
    if (d) {
        if (d.status === "running") {
            return d.kind === "workflow"
                ? { label: `${d.members_done}/${d.member_count}`, variant: "agent-running" }
                : { label: "running", variant: "agent-running" };
        }
        return { label: "done", variant: "agent" };
    }
    // No confident match — fall back to today's static, result-gated behavior.
    const s = node.status;
    if (s === "running" || s === "pending_approval" || !hasToolResult(node)) return null;
    return s === "success" ? { label: "done", variant: "agent" } : null;
}

export function toolResultPill(node: ToolNode, dispatchMatch: AgentDispatch | undefined): ResultPill | null {
    // Agent/Task/Workflow: prefer the live ordinal-matched dispatch status (see
    // activity/dispatch-correlation.ts) over the tool call's own status. Deliberately bypasses the
    // running/no-result gate below — the whole point is showing progress WHILE the
    // subagent/workflow is still running, long before the parent's own tool_use resolves.
    if (node.tool === "Agent" || node.tool === "Task" || node.tool === "Workflow") {
        return dispatchPill(node, dispatchMatch);
    }
    if (TOOL_STATUS[node.status].active || !hasToolResult(node)) return null;
    // An unloaded result keeps the pill it had (tool-result-unload.ts).
    if (node.resultUnloaded) return node.resultUnloaded.pill;
    return toolPill(node);
}

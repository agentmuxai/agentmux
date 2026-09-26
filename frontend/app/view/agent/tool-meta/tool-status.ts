// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Everything the agent pane derives from a tool's status, in one table
 * (SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md §2.4; report item A3).
 * Replaces ToolBlock's STATUS_ICON / isActive / isFailTerminal,
 * ToolBlockOverlay's STATUS_LABEL and tool-adapter's toolActivityStatus.
 */

import type { ActivityStatus } from "../activity/types";
import type { ToolNode } from "../types";

export interface ToolStatusInfo {
    /** The row's status glyph. */
    icon: string;
    /** The overlay header's word for it. */
    label: string;
    /** The Activity Dock's bucket. */
    activity: ActivityStatus;
    /** Still in progress: the panel auto-expands. */
    active: boolean;
    /** The user dismissed it (denied / canceled): no post-completion hold. */
    dismissed: boolean;
}

export const TOOL_STATUS: Record<ToolNode["status"], ToolStatusInfo> = {
    running: { icon: "⏳", label: "running", activity: "running", active: true, dismissed: false },
    // Cut off or never actually ran long: not a failure signal, the same
    // bucket as subagent-adapter.ts's "abandoned" → "stopped".
    pending_approval: { icon: "⚠", label: "awaiting approval", activity: "stopped", active: true, dismissed: false },
    awaiting_answer: { icon: "❓", label: "awaiting answer", activity: "stopped", active: false, dismissed: false },
    success: { icon: "✓", label: "ok", activity: "done", active: false, dismissed: false },
    failed: { icon: "✗", label: "failed", activity: "error", active: false, dismissed: false },
    denied: { icon: "⊘", label: "denied", activity: "stopped", active: false, dismissed: true },
    canceled: { icon: "⏹", label: "canceled", activity: "stopped", active: false, dismissed: true },
};

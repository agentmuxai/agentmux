// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { TOOL_STATUS } from "./tool-status";

// One table for what used to be four: ToolBlock's STATUS_ICON, isActive and
// isFailTerminal, ToolBlockOverlay's STATUS_LABEL, and tool-adapter's
// toolActivityStatus. Values captured from those before the move.
describe("TOOL_STATUS", () => {
    it("matches the previous per-module maps", () => {
        expect(TOOL_STATUS).toEqual({
            running: { icon: "⏳", label: "running", activity: "running", active: true, dismissed: false },
            pending_approval: { icon: "⚠", label: "awaiting approval", activity: "stopped", active: true, dismissed: false },
            awaiting_answer: { icon: "❓", label: "awaiting answer", activity: "stopped", active: false, dismissed: false },
            success: { icon: "✓", label: "ok", activity: "done", active: false, dismissed: false },
            failed: { icon: "✗", label: "failed", activity: "error", active: false, dismissed: false },
            denied: { icon: "⊘", label: "denied", activity: "stopped", active: false, dismissed: true },
            canceled: { icon: "⏹", label: "canceled", activity: "stopped", active: false, dismissed: true },
        });
    });
});

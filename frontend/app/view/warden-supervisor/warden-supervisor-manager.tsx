// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Supervisor section. Placeholder pending its backend support
// (per-agent auto_continue_enabled field, the transcript-read + nudge
// routes/MCP tools, and the consecutive-nudge-ceiling guardrail — see
// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md).
// Once that lands, this becomes a per-agent auto-continue toggle list +
// recent-decisions feed (reusing warden-audit-shared.ts, filtered to
// entry.outcome != null).

import type { JSX } from "solid-js";

import "@/app/view/warden-shared/warden-manager-chrome.scss";

export const WardenSupervisorManager = (): JSX.Element => (
    <div class="warden-manager-body">
        <p class="warden-manager-summary">Opt-in continuation nudging for stalled agents</p>
        <div class="warden-section-stub">
            Coming soon — per-agent auto-continue configuration and a
            recent-decisions feed. Backend support (transcript access,
            nudge delivery, and the consecutive-nudge ceiling guardrail)
            ships in a follow-up PR.
        </div>
    </div>
);

WardenSupervisorManager.displayName = "WardenSupervisorManager";

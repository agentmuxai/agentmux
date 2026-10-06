// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `agent-view.tsx` and every module split out of it, for the guard tests that
 * scan its source text: `agent-view-dispatch-via-pane-model.test.ts` (A9) and
 * `agent-pane-view.test.ts` (A6). Code moved out of `agent-view.tsx` into a
 * file NOT listed here is no longer scanned, so each extraction adds its new
 * files here in the same PR, and `agent-view-size.test.ts` checks every entry
 * exists.
 *
 * Paths are relative to this directory. The split plan is
 * docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.4.
 */
export const AGENT_VIEW_SOURCES: readonly string[] = [
    "agent-view.tsx",
    "agent-block-content.tsx",
    "agent-pane-chrome-model.ts",
    "hooks/useShellLogBridge.ts",
    "hooks/useWorkingIndicator.ts",
    "components/AgentProgressBar.tsx",
    "activity/promotion-clock.ts",
    "startup/sendStartupSequence.ts",
    "activity/useAttachedTaskAxis.ts",
    "components/AgentStashDrawer.tsx",
    "components/AgentShellDrawer.tsx",
    "components/AgentBottomPanels.tsx",
    "failure/useAccountBinding.ts",
    "failure/useAuthHealth.ts",
    "agent-media.tsx",
    "hooks/usePaneReveal.ts",
    "hooks/useLiveFeedRollOff.ts",
    "hooks/useTurnReconciliation.ts",
    "hooks/turn-confirmation.ts",
    "hooks/useContextReading.ts",
];

/**
 * Line cap for `agent-view.tsx` itself (`agent-view-size.test.ts`). The file
 * grew from 1,959 lines (2026-07-31) to 3,001 after an earlier split, because
 * nothing stopped it. Lower this whenever an extraction shrinks the file;
 * raising it needs a reason in the PR.
 */
export const AGENT_VIEW_MAX_LINES = 1502;

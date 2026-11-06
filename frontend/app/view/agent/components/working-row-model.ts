// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's props, and the two small facts about them that more than one part of the row reads. */

import type { ActivityState, CompactionState, ResumeRetryState, TurnCarry } from "@/app/store/agent-pane-state/types";
import { turnOpen, type TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import type { SessionStats, TurnTokens } from "../../../agent-model/types";
import type { LaunchPhase } from "../../../agent-model/launch-phase";
import { TRIGGER_LEAD_IN_MS, triggerLeadIn } from "../turn-trigger-text";

export interface AgentWorkingRowProps {
    loading: boolean;
    stopping?: boolean;
    /** The pane's ambient (Haiku-written) session summary, or the CLI's own
     *  window-title topic when there isn't one yet — `readSwarmSummary` of the
     *  block meta, the same line the Swarm row shows. Shown in the left zone
     *  in place of the old "tool · arg" text; the phrase cycle covers the gap
     *  until one exists. */
    activitySummary?: string | null;
    sessionStats?: SessionStats | null;
    /** Live token counts for the CLI pass in progress; the right zone shows
     *  the turn's output so far, with ↑/↓ for the request phase, next to the
     *  elapsed time. */
    turnTokens?: TurnTokens | null;
    /** srv's turn ledger (`agentturn`): the turn as the user sees it, over
     *  however many CLI passes. While it is open the row times and counts the
     *  whole turn, and stays up (dimmed) between two of its passes; once it
     *  ends, the Worked line reports it. Absent (older srv): per pass, as before.
     *  SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4. */
    turnLedger?: TurnLedger | null;
    /** Each pass's figures summed over the turn, for providers srv doesn't
     *  count (`AgentPaneState.turnCarry`). */
    turnCarry?: TurnCarry | null;
    /** What the agent is doing right now (`AgentPaneState.activity`): the
     *  live status's facts. SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6. */
    activity?: ActivityState | null;
    /** A question or approval waiting on the user, in words: it outranks
     *  everything else the row could say. */
    needsYou?: string | null;
    /** Set when the provider is rate-limited; shows "Rate limited…" in place of thinking phrase. */
    waitingReason?: "rate_limited" | null;
    /** When the provider's next retry is due (epoch ms, from Retry-After). The
     *  row counts down to it while waitingReason is set. */
    retryAtMs?: number | null;
    /** What the launch/login flow is doing right now — shown in place of the
     *  generic thinking phrase so a timed wait (login link, login completion)
     *  never renders as bare "Working…". See launch-phase.ts. */
    launchPhase?: LaunchPhase | null;
    /** Cancel the in-flight login. Shown as a small button while launchPhase
     *  is a pre-authUrl wait (opening-login-terminal /
     *  waiting-for-login-completion) — AuthUrlBox has its own cancel button
     *  once a URL exists, but before that this is the only affordance. */
    onCancelLogin?: () => void;
    /** True while AuthUrlBox is already showing its own Cancel button (a URL
     *  was captured: tier 1's "opened" outcome sets launchPhase to
     *  "waiting-for-login-completion" too, so without this the row would render
     *  a second, redundant Cancel button alongside AuthUrlBox's (#2300). */
    hasAuthUrl?: boolean;
    /** Live "compaction in progress" state, or null — relocated here from
     *  AgentComposerStrip (2026-08-27, Part 2 of
     *  SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) so a user has
     *  exactly one place to check "is something happening right now." Takes
     *  priority over the normal Working/tool display. */
    compacting?: CompactionState | null;
    /** Tokens in the pane's context (`state.context`, validated). While
     *  `compacting`, the row estimates how long this compaction will take from
     *  earlier ones (`compaction-estimate.ts`) scaled by this, and shows an
     *  ESTIMATE bar — Claude Code reports no real compaction progress, so the
     *  bar is labeled as an estimate. Read once when the compaction starts, so
     *  the estimate doesn't move mid-run. No earlier compactions → the built-in
     *  default (`DEFAULT_COMPACTION_MS`). */
    compactionContextTokens?: number | null;
    /** The pane's model key (`compactionModelKey`), to estimate from that model's past compactions. */
    compactionModel?: string | null;
    /** Live "recovering from a stale --resume session id" state, or null —
     *  same relocation as `compacting` above. Fires ONLY after the
     *  underlying process has already crashed/exited, so this can be set
     *  while `loading` is false — the row must render for this alone. Takes
     *  priority over `compacting` (mirrors the old composer-strip priority:
     *  a silent-gap recovery is more easily mistaken for a hang). */
    reconnecting?: ResumeRetryState | null;
}

// "waiting-for-login-link" (tier 1's own up-to-15s countdown) must be here:
// AuthUrlBox can't show a cancel button yet (no URL captured), so the row's own
// button is the only way out of that timed wait (#2304).
export const CANCELLABLE_LAUNCH_PHASES = new Set([
    "first-login",
    "auth-expired",
    "waiting-for-login-link",
    "opening-login-terminal",
    "waiting-for-login-completion",
]);

/** A turn something other than the user started opens by saying so. */
export function leadInText(props: AgentWorkingRowProps, nowMs: number): string | null {
    const l = props.turnLedger;
    if (!l || !turnOpen(l, nowMs) || nowMs - l.startedAtMs >= TRIGGER_LEAD_IN_MS) return null;
    return triggerLeadIn(l.trigger);
}

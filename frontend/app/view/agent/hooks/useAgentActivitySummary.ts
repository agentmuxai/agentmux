// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — drives the session-goal title in the agent pane header.
 *
 * Fires when a turn ENTERS `Submitting` — i.e. right when the user submits a
 * new message — not on turn completion. The goal a session is working toward
 * can only change at the point the user says something new; the agent's own
 * tool calls afterward never change it, so there's no reason to wait for a
 * (possibly long, multi-tool-call) turn to finish before re-evaluating the
 * title. This also means the backend no longer needs to read a FileStore
 * output tail for this call — `TurnPhase.Submitting.pendingContent` already
 * carries the literal text just submitted (threaded through via
 * `TurnStart.content`, see agent-pane-state/reducer.ts).
 *
 * Sends that text, plus the CURRENTLY DISPLAYED title (already in block
 * meta), to claude-haiku-4-5-20251001 and asks it to maintain a stable,
 * PR-title-style summary of the session's OVERALL GOAL — repeating the
 * current title back unchanged unless the new message represents a genuinely
 * new or expanded goal. This replaces the previous "what is currently being
 * worked on" per-turn micro-activity phrasing, which regenerated from a
 * blank slate every call and had no way to recognize "this is still the same
 * task." See docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
 *
 * The result is written to the `term:ambient_summary` block meta key, which
 * agent-model.ts and swarm-model.ts read (preferring it over the free
 * `term:osc_title` signal — see
 * docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §3.4).
 *
 * Word target is derived from pane width so narrow panes get ~5 words and wide
 * panes can accommodate up to 12.
 *
 * This call is routed through the backend's Ambient Model Call gateway
 * (`crate::ambient`), keyed by block_id: the gateway persists its
 * per-block generation state across pane remounts (tab-switch), but this
 * hook's own state does not (a fresh `activeTurnId` starts at 0 on every
 * mount — confirmed by useBlockActivity.ts's own comment on remount
 * behavior). Sending the local counter as `generation` would mean a remount
 * right after a high-generation turn could send a *lower* number than the
 * gateway already has recorded for this block, getting rejected as
 * stale-on-arrival for up to 15s (until the still-in-flight prior call's
 * guard drops) even though it's a legitimately new request. `Date.now()` is
 * used for the wire `generation` instead — always increasing regardless of
 * remounts, since real time never goes backwards for this purpose.
 *
 * The summary is never cleared on our own — it persists across turns so the
 * header always shows the last known title. It's cleared elsewhere
 * (useBlockActivity.ts) when the underlying session ends.
 */

import { createEffect, on, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/wos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { recordTurn } from "@/app/store/token-usage";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

export interface UseAgentActivitySummaryOptions {
    blockId: string;
    turnPhase: Accessor<TurnPhase>;
    getRootWidth: () => number | undefined;
}

export function useAgentActivitySummary(opts: UseAgentActivitySummaryOptions): void {
    const { blockId, turnPhase, getRootWidth } = opts;

    // Monotonically increasing local counter, scoped to this mount — the
    // write-boundary staleness guard (a fast second submission before the
    // first call returns must discard the first call's result). Independent
    // of the wire `generation` sent to the backend gateway — see the module
    // doc comment for why those are deliberately different counters.
    let activeTurnId = 0;

    // `defer: true` — skip the run at mount. A freshly-opened pane whose
    // live turnPhase happens to already be Submitting (e.g. reattaching
    // mid-turn) shouldn't immediately fire; the next genuine submission will.
    createEffect(on(turnPhase, (phase) => {
        if (phase.kind !== "Submitting") return;
        activeTurnId++;
        const myTurnId = activeTurnId;
        const rootWidth = getRootWidth() ?? 400;
        const textWidth = Math.max(0, rootWidth - 280);
        const wordTarget = Math.max(5, Math.min(12, Math.floor(textWidth / 48)));

        RpcApi.AgentActivitySummaryCommand(
            TabRpcClient,
            {
                block_id: blockId,
                word_target: wordTarget,
                generation: Date.now(),
                user_message: phase.pendingContent,
            },
            { timeout: 20_000 },
        ).then((result) => {
            if (activeTurnId !== myTurnId) return; // superseded by a newer turn
            if (result.tokens) {
                recordTurn("ambient:activity_summary", result.tokens);
            }
            if (result.summary) {
                fireAndForget(() =>
                    ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                        "term:ambient_summary": result.summary,
                    } as any)
                );
            }
        }).catch(() => {
            // Silently ignore — the header just stays on its last title.
        });
    }, { defer: true }));
}

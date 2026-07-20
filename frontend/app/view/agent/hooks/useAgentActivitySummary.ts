// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — drives the live mini-summary in the agent pane header.
 *
 * Fires exactly once per genuine, backend-confirmed turn completion — see
 * `turnJustEndedAtom` below, NOT on `TurnPhase.kind === "Done"` (a previous
 * version triggered off that; `Done` also fires on several non-terminal
 * transitions — a premature per-round `session_end` the Claude Code
 * translator synthesizes between tool-call rounds, and the bounded-timeout
 * force-transitions for a stop/submit that never got acked — none of which
 * mean the agent actually finished responding. See
 * docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md for the full
 * diagnosis). Sends recent session output to claude-haiku-4-5-20251001 and
 * asks for a short phrase (~word_target words) describing what was just
 * done. The result is written to the `term:ambient_summary` block meta key,
 * which agent-model.ts and swarm-model.ts read (preferring it over the free
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
 * remounts, since real time never goes backwards for this purpose. The
 * local `activeTurnId !== myTurnId` check below is unaffected by this (it's
 * scoped to a single mount's closures) and remains a second, independent
 * guard at the write boundary on top of the gateway's own cancellation.
 *
 * The summary is never cleared on our own — it persists across turns so the
 * header always shows the last known activity. It's cleared elsewhere
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
    /**
     * Bumped exactly once per genuine, backend-confirmed turn completion —
     * the `turn_active: true -> false` edge derived from the live
     * `controllerstatus` event (see agent-view.tsx's `reconcileTurnActive`),
     * which the backend only flips on the CLI's own real "result" line
     * (`agentmux-srv/src/backend/blockcontroller/persistent.rs`), not per
     * tool-call round. This is the trigger — see the module doc comment for
     * why `TurnPhase.kind === "Done"` isn't used instead.
     */
    turnJustEndedAtom: Accessor<number>;
    getRootWidth: () => number | undefined;
}

export function useAgentActivitySummary(opts: UseAgentActivitySummaryOptions): void {
    const { blockId, turnPhase, turnJustEndedAtom, getRootWidth } = opts;

    // Monotonically increasing turn ID, scoped to this mount. Bumped on every
    // Submitting transition and re-checked locally when the response lands
    // (NOT sent to the backend — see the module doc comment above for why).
    let activeTurnId = 0;

    createEffect(on(turnPhase, (phase) => {
        if (phase.kind === "Submitting") {
            activeTurnId++;
        }
    }));

    // `defer: true` — skip the run at mount (turnJustEndedAtom starts at 0;
    // a freshly-opened pane onto an already-idle agent must not fire).
    createEffect(on(turnJustEndedAtom, () => {
        const myTurnId = activeTurnId;
        const rootWidth = getRootWidth() ?? 400;
        const textWidth = Math.max(0, rootWidth - 280);
        const wordTarget = Math.max(5, Math.min(12, Math.floor(textWidth / 48)));

        RpcApi.AgentActivitySummaryCommand(
            TabRpcClient,
            { block_id: blockId, word_target: wordTarget, generation: Date.now() },
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
            // Silently ignore — the header just stays blank.
        });
    }, { defer: true }));
}

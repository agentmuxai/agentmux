// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — drives the live mini-summary in the agent pane header.
 *
 * On every completed agent turn, sends recent session output to
 * claude-haiku-4-5-20251001 and asks for a short phrase (~word_target words)
 * describing what was just done. The result is written to the
 * `term:ambient_summary` block meta key, which agent-model.ts and
 * swarm-model.ts read (preferring it over the free `term:osc_title` signal —
 * see docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §3.4).
 *
 * Word target is derived from pane width so narrow panes get ~5 words and wide
 * panes can accommodate up to 12.
 *
 * This call is routed through the backend's Ambient Model Call gateway
 * (`crate::ambient`): the turn counter sent as `generation` lets the gateway
 * cancel a still-running Haiku call from a superseded turn (killing the
 * subprocess, not just discarding its result) and reject an out-of-order
 * request before any work happens. The `activeTurnId !== myTurnId` check
 * below is belt-and-suspenders on top of that — the gateway is the primary
 * guard now, this is a second, independent check at the write boundary.
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
    getRootWidth: () => number | undefined;
}

export function useAgentActivitySummary(opts: UseAgentActivitySummaryOptions): void {
    const { blockId, turnPhase, getRootWidth } = opts;

    // Monotonically increasing turn ID. Bumped on every Submitting transition;
    // sent to the backend as `generation` (ambient-gateway cancellation key)
    // and re-checked locally when the response lands.
    let activeTurnId = 0;

    createEffect(on(turnPhase, (phase) => {
        if (phase.kind === "Submitting") {
            activeTurnId++;
            return;
        }

        if (phase.kind === "Done") {
            const myTurnId = activeTurnId;
            const rootWidth = getRootWidth() ?? 400;
            const textWidth = Math.max(0, rootWidth - 280);
            const wordTarget = Math.max(5, Math.min(12, Math.floor(textWidth / 48)));

            RpcApi.AgentActivitySummaryCommand(
                TabRpcClient,
                { block_id: blockId, word_target: wordTarget, generation: myTurnId },
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
        }
    }));
}

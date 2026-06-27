// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — drives the live mini-summary in the agent pane header.
 *
 * On every completed agent turn, sends recent session output to
 * claude-haiku-4-5-20251001 and asks for a short phrase (~word_target words)
 * describing what was just done. The result is written to the `term:activity`
 * block meta key, which agent-model.ts surfaces in the pane header.
 *
 * Word target is derived from pane width so narrow panes get ~5 words and wide
 * panes can accommodate up to 12.
 *
 * The summary is never cleared — it persists across turns so the header always
 * shows the last known activity. A turn counter ensures a slow Haiku response
 * from a superseded turn cannot overwrite a newer summary.
 */

import { createEffect, on, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/wos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

export interface UseAgentActivitySummaryOptions {
    blockId: string;
    turnPhase: Accessor<TurnPhase>;
    getRootWidth: () => number | undefined;
}

export function useAgentActivitySummary(opts: UseAgentActivitySummaryOptions): void {
    const { blockId, turnPhase, getRootWidth } = opts;

    // Monotonically increasing turn ID. Bumped on every Submitting transition so
    // any in-flight Haiku response from a prior turn can detect it is stale and
    // skip the meta write.
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
                { block_id: blockId, word_target: wordTarget },
                { timeout: 20_000 },
            ).then((result) => {
                if (activeTurnId !== myTurnId) return; // superseded by a newer turn
                if (result.summary) {
                    fireAndForget(() =>
                        ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                            "term:activity": result.summary,
                        } as any)
                    );
                }
            }).catch(() => {
                // Silently ignore — the header just stays blank.
            });
        }
    }));
}

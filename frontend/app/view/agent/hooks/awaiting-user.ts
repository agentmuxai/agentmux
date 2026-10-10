// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's `term:awaiting_user` meta: true while the agent itself waits on
 * the user here, for a question (useAgentQuestions) or a tool permission
 * (useAgentDecisions). srv's `AgentState::Waiting`, the Swarm's "Waiting for
 * you" line and the LAN and viewer feeds read it, so it's kept in block meta,
 * where they can see it for panes that aren't mounted
 * (REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §2).
 */

import { createEffect, on } from "solid-js";
import { isAgentWaitingForYou } from "@/app/notification/waiting-for-you";
import { getPaneModel } from "@/app/store/agent-pane-registration";
import { isInitReady } from "@/app/store/agent-pane-state/types";
import { makeORef } from "@/app/store/mos";
import { ObjectService } from "@/app/store/services";
import { META_AWAITING_USER } from "@/app/store/swarm-line";
import { fireAndForget } from "@/util/util";

/** Write the meta from what waits in the pane now: call after starting or
 *  ending a question or permission wait. `null` removes the key. */
export function syncAwaitingUser(blockId: string): void {
    const waiting = isAgentWaitingForYou(blockId);
    fireAndForget(() =>
        ObjectService.UpdateObjectMeta(makeORef("block", blockId), { [META_AWAITING_USER]: waiting ? true : null } as any)
    );
}

/**
 * Run `reconcile` once the pane's history has loaded: a wait left from
 * before this mount (a question cancelled while the pane was unmounted)
 * ends then. Not at mount itself: the document is empty until history
 * replays, and ending then would resolve the notification for a question
 * that's still there. Call from a hook inside the pane.
 */
export function reconcileWhenHistoryLoads(blockId: string, reconcile: () => void): void {
    const model = getPaneModel(blockId);
    if (!model) return;
    createEffect(
        on(
            () => isInitReady(model.state),
            (ready) => {
                if (!ready) return;
                reconcile();
                // The flag follows what's left waiting, after the reconcile.
                syncAwaitingUser(blockId);
            }
        )
    );
}

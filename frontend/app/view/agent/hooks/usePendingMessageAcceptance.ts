// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * usePendingMessageAcceptance — subscribes to `AgentMessageAccepted` and
 * promotes the matching pending-queue entry into a real `user_message`
 * document node. Pushes into the shared `StreamFlushQueue` rather than
 * dispatching/flushing independently — a second independent RAF/`batch()`
 * here would reintroduce the reconcileArrays/replaceChild crash documented
 * in RETRO_REPLACECHILD_CRASH_2026-06-06.md; see stream-flush-queue.ts's
 * module doc for the full rationale.
 *
 * Called directly from inside the caller's `onMount`, matching the
 * original inline placement. No-ops when `pendingMessagesAtom` is not
 * supplied.
 */

import { onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import * as WOS from "@/app/store/wos";
import { trail } from "@/log/render-trail";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { PendingMessage, SignalPair } from "../state";
import type { UserMessageNode } from "../types";
import { STARTUP_HEADING_RE } from "../stream-parser";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UsePendingMessageAcceptanceOptions {
    blockId: string;
    model: AgentPaneModel;
    pendingMessagesAtom?: SignalPair<PendingMessage[]>;
    queue: StreamFlushQueue;
    hasNodeId: (id: string) => boolean;
    addNodeId: (id: string) => void;
    /**
     * Called whenever this hook starts a new turn from the queue-drain path
     * (below). Wired to the message-list's `jumpToBottom` (re-engages
     * `stickToBottom` + scrolls to true bottom) so a turn that begins here —
     * a queued message the backend just picked up, not something typed into
     * THIS pane's own composer — still auto-follows. Without this, a user
     * who scrolled away earlier (even during a prior, unrelated turn) sees a
     * stale `stickToBottom=false` carry into a brand-new turn whose start
     * had nothing to do with their scroll action. `handleSendMessage`'s own
     * `onSent` callback already covers the composer-driven case; this covers
     * the one TurnStart site that didn't. See
     * REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md's
     * scroll-follow-drift investigation, root cause #3.
     */
    onTurnStartFromQueue?: () => void;
}

export function usePendingMessageAcceptance(opts: UsePendingMessageAcceptanceOptions): void {
    if (!opts.pendingMessagesAtom) return;
    const [getPending] = opts.pendingMessagesAtom;

    // Subscribe to `agent-message-accepted`: when the backend picks
    // up a queued message, promote the matching entry out of the
    // pending zone into a real `user_message` document node. That
    // color shift (amber → accent blue) is the user's visible
    // "accepted" transition for the user.
    const acceptedUnsub = waveEventSubscribe({
        eventType: WpsEvent.AgentMessageAccepted,
        scope: WOS.makeORef("block", opts.blockId),
        handler: (event) => {
            const data = (event as any)?.data;
            if (!data) return;
            const messageId: string | undefined = data.message_id;
            if (!messageId) return;
            const pending = getPending().find((m) => m.id === messageId);
            if (!pending) {
                // Accepted event for an id we don't know about.
                // Can legitimately happen if the entry was already
                // promoted or the pane was re-mounted mid-queue.
                return;
            }
            // Reducer removes the entry + emits pending-accepted.
            opts.model.dispatchPane({
                type: "PendingMessageAccepted",
                id: messageId,
            });
            // Queue-drain case: the prior turn ended (phase Done/Idle/
            // Disconnected) and the backend is now picking up the next
            // queued message. Re-enter Submitting so the working
            // animation re-activates.
            //
            // For idle sends (phase Submitting/Streaming/Interrupting),
            // TurnStart was already dispatched in handleSendMessage —
            // a second fire here regresses Streaming → Submitting and
            // re-arms the 30 s submit timeout unnecessarily.
            // See docs/analysis/ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
            const currentPhase = paneSnapshot(opts.blockId)?.turnPhase;
            const needsTurnStart =
                currentPhase?.kind === "Done" ||
                currentPhase?.kind === "Idle" ||
                currentPhase?.kind === "Disconnected" ||
                currentPhase == null;
            if (needsTurnStart) {
                trail("agent:dispatch:TurnStart", { messageId });
                opts.model.dispatchPane({ type: "TurnStart", at: Date.now(), content: pending.text });
                trail("agent:dispatch:TurnStart:done", { messageId });
                opts.onTurnStartFromQueue?.();
            }
            // Append as a normal user_message so it joins the
            // conversation stream. Keeps the same id so the new
            // node ties back to the pending entry 1:1.
            // The optimistic-acceptance path goes through the
            // same `handleSendMessage` pipeline as the startup
            // injection (see agent-view.tsx `onReadyFn`). Apply
            // the same heuristic here as in the stream-parser
            // so the startup payload is flagged on first
            // render — otherwise UserMessageBlock would render
            // it as a regular user message (the full Markdown
            // wall, not the collapsed summary).
            // Codex P1 round 2 on PR #1020.
            const node: UserMessageNode = {
                type: "user_message",
                id: pending.id,
                message: pending.text,
                timestamp: Date.now(),
                isStartup: STARTUP_HEADING_RE.test(pending.text),
            };
            if (!opts.hasNodeId(node.id)) {
                opts.addNodeId(node.id);
                opts.queue.pushNewNode(node);
                opts.queue.scheduleFlush();
            }
        },
    });
    onCleanup(() => acceptedUnsub());
}
